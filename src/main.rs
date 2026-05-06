use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseButton, MouseEvent, MouseEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect, Position},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarState, ScrollbarOrientation},
    Frame, Terminal,
};
use std::{
    error::Error,
    io::{self, Write, Read},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use sysinfo::System;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use vt100::{Parser, MouseProtocolEncoding, MouseProtocolMode};

fn log_debug(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("debug.log") {
        let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg);
    }
}

struct App {
    cpu_usage: f32,
    mem_usage: (u64, u64),
    git_info: Option<String>,
    sidebar_state: ListState,
    is_dragging_scrollbar: bool,
    is_dragging_sidebar_scrollbar: bool,
    is_selecting: bool,
    selection_start: Option<(u16, u16)>,
    selection_end: Option<(u16, u16)>,
    sidebar_items: Vec<String>,
    sidebar_commands: Vec<String>,
    sidebar_infos: Vec<String>,
    show_menu: bool,
    mouse_pos: Option<(u16, u16)>,
    parser: Arc<Mutex<Parser>>,
    pty_write: Box<dyn Write + Send>,
    master: Box<dyn portable_pty::MasterPty + Send>,
    should_quit: bool,
    last_activity: Instant,
    start_time: Instant,
    search_query: String,
    is_search_focused: bool,
}


fn load_commands(exe_dir: &std::path::Path) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut labels = Vec::new();
    let mut commands = Vec::new();
    let mut infos = Vec::new();

    let cmd_path = exe_dir.join("commands.txt");
    let mut label_counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    if let Ok(content) = std::fs::read_to_string(cmd_path) {
        let mut last_label_base: Option<String> = None;
        let mut last_info = String::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('#') {
                let parts: Vec<&str> = trimmed[1..].trim().split_whitespace().collect();
                if let Some(first) = parts.first() {
                    last_label_base = Some(first.to_string());
                    last_info = parts[1..].join(" ");
                }
            } else {
                let base_name = last_label_base.clone().unwrap_or_else(|| {
                    trimmed.split_whitespace().next().unwrap_or("cmd").to_string()
                });
                
                let count = label_counts.entry(base_name.clone()).or_insert(0);
                *count += 1;
                
                let final_label = if *count > 1 {
                    format!("{}{}", base_name, *count)
                } else {
                    base_name
                };

                labels.push(final_label);
                commands.push(trimmed.to_string());
                infos.push(last_info.clone());
            }
        }
    }
    (labels, commands, infos)
}

impl App {
    fn new(
        pty_write: Box<dyn Write + Send>, 
        master: Box<dyn portable_pty::MasterPty + Send>,
        parser: Arc<Mutex<Parser>>,
        sidebar_items: Vec<String>,
        sidebar_commands: Vec<String>,
        sidebar_infos: Vec<String>,
    ) -> App {
        let mut state = ListState::default();
        state.select(Some(0));
        App {
            cpu_usage: 0.0,
            mem_usage: (0, 0),
            git_info: None,
            sidebar_state: state,
            sidebar_items,
            sidebar_commands,
            sidebar_infos,
            show_menu: false,
            mouse_pos: None,
            parser,
            pty_write,
            master,
            should_quit: false,
            last_activity: Instant::now(),
            start_time: Instant::now(),
            is_dragging_scrollbar: false,
            is_dragging_sidebar_scrollbar: false,
            is_selecting: false,
            selection_start: None,
            selection_end: None,
            search_query: String::new(),
            is_search_focused: false,
        }
    }

    fn update_stats(&mut self, sys: &mut System) {
        sys.refresh_cpu();
        sys.refresh_memory();
        self.cpu_usage = sys.global_cpu_info().cpu_usage();
        self.mem_usage = (sys.used_memory(), sys.total_memory());

        // Only refresh git info every ~2 seconds (40 * 50ms) to save CPU
        static mut GIT_COUNTER: u32 = 0;
        unsafe {
            GIT_COUNTER += 1;
            if GIT_COUNTER < 4 && self.git_info.is_some() {
                return;
            }
            GIT_COUNTER = 0;
        }

        let branch = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .ok()
            .and_then(|out| if out.status.success() { 
                Some(String::from_utf8_lossy(&out.stdout).trim().to_string()) 
            } else { None });

        if let Some(b) = branch {
            let status = Command::new("git")
                .args(["status", "--porcelain"])
                .output()
                .ok()
                .and_then(|out| if out.status.success() {
                    let s = String::from_utf8_lossy(&out.stdout);
                    Some(if s.trim().is_empty() { "clean" } else { "modified" })
                } else { None })
                .unwrap_or("unknown");
            self.git_info = Some(format!("GIT: {} ({})", b, status));
        } else {
            self.git_info = None;
        }
    }

    fn copy_selection(&mut self) {
        log_debug("Copy selection triggered");
        if let (Some(start), Some(end)) = (self.selection_start, self.selection_end) {
            log_debug(&format!("Selection range: {:?} to {:?}", start, end));
            let parser = self.parser.lock().unwrap();
            let screen = parser.screen();
            let (_, cols) = screen.size();
            
            let (s_row, s_col) = start;
            let (e_row, e_col) = end;
            let (min_row, min_col, max_row, max_col) = if s_row < e_row || (s_row == e_row && s_col <= e_col) {
                (s_row, s_col, e_row, e_col)
            } else {
                (e_row, e_col, s_row, s_col)
            };

            let mut text = String::new();
            for r in min_row..=max_row {
                let start_c = if r == min_row { min_col } else { 0 };
                let end_c = if r == max_row { max_col } else { (cols as u16).saturating_sub(1) };
                
                let mut line = String::new();
                for c in start_c..=end_c {
                    if let Some(cell) = screen.cell(r, c) {
                        line.push_str(cell.contents());
                    } else {
                        line.push(' ');
                    }
                }
                text.push_str(line.trim_end());
                if r < max_row {
                    text.push('\n');
                }
            }
            
            log_debug(&format!("Extracted text ({} chars): {:?}", text.len(), text));
            match arboard::Clipboard::new() {
                Ok(mut clipboard) => {
                    if let Err(e) = clipboard.set_text(text) {
                        log_debug(&format!("Clipboard set_text error: {:?}", e));
                    } else {
                        log_debug("Clipboard set_text success");
                    }
                }
                Err(e) => {
                    log_debug(&format!("Clipboard init error: {:?}", e));
                }
            }
        } else {
            log_debug("No selection to copy");
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let exe_path = std::env::current_exe()?;
    let exe_dir = exe_path.parent().ok_or("Could not find executable directory")?;
    let args: Vec<String> = std::env::args().collect();
    let (sidebar_items, sidebar_commands, sidebar_infos) = load_commands(exe_dir);

    if args.len() > 1 {
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        let (labels, sidebar_commands, _) = load_commands(&exe_dir);
        let search_label = args[1].clone();
        
        if let Some(i) = labels.iter().position(|l| l == &search_label) {
            if let Some(cmd) = sidebar_commands.get(i) {
                if cmd.contains("<prompt:") {
                    let mut final_cmd = cmd.clone();
                    while let Some(start) = final_cmd.find("<prompt:") {
                        if let Some(end) = final_cmd[start..].find(">") {
                            let tag = &final_cmd[start + 8..start + end];
                            let parts: Vec<&str> = tag.split(':').collect();
                            let label = parts.get(0).unwrap_or(&"Value");
                            let default = parts.get(1).unwrap_or(&"");
                            
                            print!("{}: (Default: {}) ", label, default);
                            std::io::stdout().flush()?;
                            
                            let mut input = String::new();
                            std::io::stdin().read_line(&mut input)?;
                            let input = input.trim();
                            
                            let val = if input.is_empty() { default.to_string() } else { input.to_string() };
                            final_cmd.replace_range(start..start + end + 1, &val);
                        } else {
                            break;
                        }
                    }
                    println!("\x1b[90mExecuting: {}\x1b[0m", final_cmd);
                    std::process::Command::new("bash").arg("-c").arg(&final_cmd).status()?;
                } else {
                    println!("\x1b[90mExecuting: {}\x1b[0m", cmd);
                    std::process::Command::new("bash").arg("-c").arg(cmd).status()?;
                }
                return Ok(());
            }
        }
        return Ok(());
    }

    log_debug("--- Starting Rust Dashboard ---");

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    
    let chunks = if size.height < 26 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(size)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(2)])
            .split(size)
    };
    let top_chunks = if size.width < 106 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(chunks[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(25)])
            .split(chunks[0])
    };
        
    let term_area = top_chunks[0];
    let rows = term_area.height;
    let cols = term_area.width;

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut cmd = CommandBuilder::new("bash");
    cmd.cwd(exe_dir);
    cmd.env("TERM", "xterm-256color");
    cmd.env("LANG", "C.UTF-8");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLUMNS", cols.to_string());
    cmd.env("LINES", rows.to_string());
    let mut child = pair.slave.spawn_command(cmd)?;
    
    let mut reader = pair.master.try_clone_reader()?;
    let pty_write = pair.master.take_writer()?;
    let master = pair.master;
    
    let parser = Arc::new(Mutex::new(Parser::new(rows, cols, 1000)));
    let parser_clone = Arc::clone(&parser);

    std::thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(n) if n > 0 => {
                    let data = &buffer[..n];
                    log_debug(&format!("PTY Read: {} bytes", n));
                    let mut p = parser_clone.lock().unwrap();
                    p.process(data);
                }
                Ok(_) => {
                    log_debug("PTY Read: 0 bytes (EOF)");
                    break;
                }
                Err(e) => {
                    log_debug(&format!("PTY Read Error: {}", e));
                    break;
                }
            }
        }
    });

    let _ = master.resize(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    });

    let mut app = App::new(pty_write, master, parser, sidebar_items, sidebar_commands, sidebar_infos);
    
    let mut sys = System::new_all();
    let tick_rate = Duration::from_millis(500);
    let mut last_tick = Instant::now();

    loop {
        if app.should_quit { break; }
        
        if let Ok(Some(_)) = child.try_wait() {
            log_debug("Child process exited.");
            break;
        }

        terminal.draw(|f| {
            ui(f, &mut app);
        })?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    app.last_activity = Instant::now();
                    log_debug(&format!("Raw Key Event: {:?}", key));
                    
                    if (key.code == KeyCode::Char('C') && key.modifiers.contains(KeyModifiers::CONTROL)) ||
                       (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) && key.modifiers.contains(KeyModifiers::SHIFT)) {
                        app.copy_selection();
                        continue;
                    }

                    if key.modifiers.contains(KeyModifiers::SHIFT) {
                        match key.code {
                            KeyCode::PageUp => {
                                let mut p = app.parser.lock().unwrap();
                                let current = p.screen().scrollback();
                                p.screen_mut().set_scrollback(current + 20);
                                continue;
                            }
                            KeyCode::PageDown => {
                                let mut p = app.parser.lock().unwrap();
                                let current = p.screen().scrollback();
                                p.screen_mut().set_scrollback(current.saturating_sub(20));
                                continue;
                            }
                            _ => {}
                        }
                    }

                    {
                        let mut p = app.parser.lock().unwrap();
                        p.screen_mut().set_scrollback(0);
                    }

                    if app.is_search_focused {
                        match key.code {
                            KeyCode::Esc => { app.is_search_focused = false; }
                            KeyCode::Backspace => { 
                                app.search_query.pop(); 
                                app.sidebar_state.select(Some(0));
                            }
                            KeyCode::Char(c) => { 
                                app.search_query.push(c); 
                                app.sidebar_state.select(Some(0));
                            }
                            KeyCode::Enter => { app.is_search_focused = false; }
                            _ => {}
                        }
                        continue;
                    }

                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        let _ = app.pty_write.write_all(b"\x03");
                        let _ = app.pty_write.flush();
                        continue;
                    }

                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match key.code {
                            KeyCode::Char(c) => {
                                let n = c as u8;
                                if (97..=122).contains(&n) {
                                    let _ = app.pty_write.write_all(&[n - 96]);
                                } else if (65..=90).contains(&n) {
                                    let _ = app.pty_write.write_all(&[n - 64]);
                                } else if c == '[' { let _ = app.pty_write.write_all(&[27]); }
                                else if c == '\\' { let _ = app.pty_write.write_all(&[28]); }
                                else if c == ']' { let _ = app.pty_write.write_all(&[29]); }
                                else if c == '^' { let _ = app.pty_write.write_all(&[30]); }
                                else if c == '_' { let _ = app.pty_write.write_all(&[31]); }
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Esc => { let _ = write!(app.pty_write, "\x1b"); }
                            KeyCode::Char(c) => { 
                                if key.modifiers.contains(KeyModifiers::CONTROL) && c == 'q' {
                                    app.should_quit = true;
                                } else {
                                    let _ = write!(app.pty_write, "{}", c); 
                                }
                            }
                            KeyCode::Enter => { let _ = write!(app.pty_write, "\r"); }
                            KeyCode::Backspace => { let _ = write!(app.pty_write, "\x7f"); }
                            KeyCode::Tab => { let _ = write!(app.pty_write, "	"); }
                            KeyCode::Up => { let _ = write!(app.pty_write, "\x1b[A"); }
                            KeyCode::Down => { let _ = write!(app.pty_write, "\x1b[B"); }
                            KeyCode::Right => { let _ = write!(app.pty_write, "\x1b[C"); }
                            KeyCode::Left => { let _ = write!(app.pty_write, "\x1b[D"); }
                            KeyCode::Home => { let _ = write!(app.pty_write, "\x1b[H"); }
                            KeyCode::End => { let _ = write!(app.pty_write, "\x1b[F"); }
                            KeyCode::PageUp => { let _ = write!(app.pty_write, "\x1b[5~"); }
                            KeyCode::PageDown => { let _ = write!(app.pty_write, "\x1b[6~"); }
                            KeyCode::Delete => { let _ = write!(app.pty_write, "\x1b[3~"); }
                            KeyCode::F(n) => {
                                let seq = match n {
                                    1 => "\x1bOP", 2 => "\x1bOQ", 3 => "\x1bOR", 4 => "\x1bOS",
                                    5 => "\x1b[15~", 6 => "\x1b[17~", 7 => "\x1b[18~", 8 => "\x1b[19~",
                                    9 => "\x1b[20~", 10 => "\x1b[21~", 11 => "\x1b[23~", 12 => "\x1b[24~",
                                    _ => ""
                                };
                                let _ = write!(app.pty_write, "{}", seq);
                            }
                            _ => {}
                        }
                    }
                    let _ = app.pty_write.flush();
                }
                Event::Mouse(mouse) => {
                    app.last_activity = Instant::now();
                    app.mouse_pos = Some((mouse.column, mouse.row));
                    handle_click(&mut app, mouse, terminal.size()?);
                }
                _ => {}
            }
        }


        if last_tick.elapsed() >= tick_rate {
            app.update_stats(&mut sys);
            last_tick = Instant::now();
        }
    }

    let _ = child.kill();
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    
    while event::poll(Duration::from_millis(10))? {
        let _ = event::read()?;
    }

    disable_raw_mode()?;
    terminal.show_cursor()?;
    log_debug("App exited cleanly.");
    Ok(())
}

fn handle_click(app: &mut App, mouse: MouseEvent, size: Rect) {
    let chunks = if size.height < 26 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(size)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(2)])
            .split(size)
    };

    let top_chunks = if size.width < 106 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(chunks[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(25)])
            .split(chunks[0])
    };

    let term_area = top_chunks[0];
    let right_pane = top_chunks[1];
    
    let right_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(0),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(right_pane);

    let scrollbar_area = right_layout[0];
    let sidebar_area = right_layout[2];
    let sidebar_scrollbar_area = right_layout[3];

    let status_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(chunks[1]);
    
    let has_selection = app.selection_start.is_some() && app.selection_end.is_some();
    let copy_width = if has_selection { 6 } else { 0 };

    let bar_chunks_upper = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(8), // MENU
            Constraint::Length(copy_width), // COPY
            Constraint::Min(0),
            Constraint::Length(30),
        ])
        .split(status_chunks[0]);

    let bar_chunks_lower = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(10),
        ])
        .split(status_chunks[1]);

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        if bar_chunks_upper[0].contains(Position::new(mouse.column, mouse.row)) {
            app.show_menu = !app.show_menu;
            return;
        }
        if bar_chunks_upper[1].contains(Position::new(mouse.column, mouse.row)) {
            app.copy_selection();
            return;
        }
        if bar_chunks_lower[1].contains(Position::new(mouse.column, mouse.row)) {
            app.should_quit = true;
        }
    }
    
    let is_app_active = {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        screen.alternate_screen() || screen.hide_cursor() || screen.application_cursor()
    };

    let sidebar_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(sidebar_area);
    let search_area = sidebar_layout[0];
    let list_area = sidebar_layout[1];

    if search_area.contains(Position::new(mouse.column, mouse.row)) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            app.is_search_focused = true;
        }
        return;
    } else if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        app.is_search_focused = false;
    }

    if list_area.contains(Position::new(mouse.column, mouse.row)) && !is_app_active {
        let filtered: Vec<(usize, &String)> = app.sidebar_items.iter().enumerate()
            .filter(|(i, label)| {
                let info = &app.sidebar_infos[*i];
                label.to_lowercase().contains(&app.search_query.to_lowercase()) ||
                info.to_lowercase().contains(&app.search_query.to_lowercase())
            })
            .collect();

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let current = app.sidebar_state.selected().unwrap_or(0);
                if current > 0 {
                    app.sidebar_state.select(Some(current - 1));
                }
            }
            MouseEventKind::ScrollDown => {
                let current = app.sidebar_state.selected().unwrap_or(0);
                if current + 1 < filtered.len() {
                    app.sidebar_state.select(Some(current + 1));
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let sidebar_y = mouse.row.saturating_sub(list_area.y);
                let offset = app.sidebar_state.offset();
                let index = (sidebar_y as usize).saturating_add(offset);

                if index < filtered.len() {
                    app.sidebar_state.select(Some(index));
                    
                    let sidebar_x = mouse.column.saturating_sub(list_area.x);
                    if sidebar_x <= 1 {
                        // Clicked the indicator part: RUN command (termbookman <label>)
                        let (original_index, _) = filtered[index];
                        let label = &app.sidebar_items[original_index];
                        let exe = std::env::current_exe().unwrap_or_default();
                        let _ = write!(app.pty_write, "{} {}\r", exe.display(), label);
                        let _ = app.pty_write.flush();
                    }
                }
            }
            _ => {}
        }
    } else if sidebar_scrollbar_area.contains(Position::new(mouse.column, mouse.row)) || app.is_dragging_sidebar_scrollbar {
        let filtered: Vec<(usize, &String)> = app.sidebar_items.iter().enumerate()
            .filter(|(i, label)| {
                let info = &app.sidebar_infos[*i];
                label.to_lowercase().contains(&app.search_query.to_lowercase()) ||
                info.to_lowercase().contains(&app.search_query.to_lowercase())
            })
            .collect();

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = true;
                let track_height = sidebar_scrollbar_area.height as f32;
                let click_pos = (mouse.row.saturating_sub(sidebar_scrollbar_area.y)) as f32;
                if track_height > 0.0 && !filtered.is_empty() {
                    let visual_idx = ((filtered.len() as f32 * (click_pos / track_height)) as usize).min(filtered.len() - 1);
                    let (original_index, _) = filtered[visual_idx];
                    app.sidebar_state.select(Some(original_index));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_dragging_sidebar_scrollbar => {
                let track_height = sidebar_scrollbar_area.height as f32;
                let mouse_y = mouse.row.clamp(sidebar_scrollbar_area.y, sidebar_scrollbar_area.bottom().saturating_sub(1));
                let click_pos = (mouse_y.saturating_sub(sidebar_scrollbar_area.y)) as f32;
                if track_height > 0.0 && !filtered.is_empty() {
                    let visual_idx = ((filtered.len() as f32 * (click_pos / track_height)) as usize).min(filtered.len() - 1);
                    let (original_index, _) = filtered[visual_idx];
                    app.sidebar_state.select(Some(original_index));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = false;
            }
            _ => {}
        }
    } else if scrollbar_area.contains(Position::new(mouse.column, mouse.row)) || app.is_dragging_scrollbar {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_scrollbar = true;
                let mut parser = app.parser.lock().unwrap();
                let screen = parser.screen_mut();
                let current = screen.scrollback();
                
                if mouse.row == scrollbar_area.y {
                    screen.set_scrollback(current + 3);
                } else if mouse.row == scrollbar_area.bottom() - 1 {
                    screen.set_scrollback(current.saturating_sub(3));
                } else {
                    let track_height = scrollbar_area.height.saturating_sub(2) as f32;
                    let click_pos = (mouse.row.saturating_sub(scrollbar_area.y + 1)) as f32;
                    let history_len = screen.scrollback_len() as f32;
                    if track_height > 0.0 {
                        let target_scroll = (history_len * (1.0 - (click_pos / track_height))) as usize;
                        screen.set_scrollback(target_scroll);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_dragging_scrollbar => {
                let mut parser = app.parser.lock().unwrap();
                let screen = parser.screen_mut();
                let track_height = scrollbar_area.height.saturating_sub(2) as f32;
                let mouse_y = mouse.row.clamp(scrollbar_area.y + 1, scrollbar_area.bottom().saturating_sub(2));
                let click_pos = (mouse_y.saturating_sub(scrollbar_area.y + 1)) as f32;
                let history_len = screen.scrollback_len() as f32;
                
                if track_height > 0.0 {
                    let target_scroll = (history_len * (1.0 - (click_pos / track_height))) as usize;
                    screen.set_scrollback(target_scroll);
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.is_dragging_scrollbar = false;
            }
            _ => {}
        }
    } else if term_area.contains(Position::new(mouse.column, mouse.row)) || app.is_selecting {
        let (parser, writer) = (&app.parser, &mut app.pty_write);
        let mut parser = parser.lock().unwrap();
        let screen = parser.screen_mut();

        let tx = mouse.column.saturating_sub(term_area.x);
        let ty = mouse.row.saturating_sub(term_area.y);

        if screen.mouse_protocol_mode() == MouseProtocolMode::None {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    app.is_selecting = true;
                    app.selection_start = Some((ty, tx));
                    app.selection_end = Some((ty, tx));
                    return;
                }
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_selecting => {
                    app.selection_end = Some((ty, tx));
                    return;
                }
                MouseEventKind::Up(MouseButton::Left) if app.is_selecting => {
                    app.is_selecting = false;
                    if app.selection_start == app.selection_end {
                        app.selection_start = None;
                        app.selection_end = None;
                    }
                    return;
                }
                _ => {}
            }
        }

        if screen.mouse_protocol_encoding() == MouseProtocolEncoding::Sgr {
            let x = mouse.column.saturating_sub(term_area.x) + 1;
            let y = mouse.row.saturating_sub(term_area.y) + 1;
            
            let (button, event_char) = match mouse.kind {
                MouseEventKind::Down(btn) => (btn, 'M'),
                MouseEventKind::Up(btn) => (btn, 'm'),
                MouseEventKind::ScrollUp => (MouseButton::Left, 'M'),
                MouseEventKind::ScrollDown => (MouseButton::Left, 'M'),
                MouseEventKind::Drag(btn) => (btn, 'M'),
                _ => return, 
            };
            
            let button_code = match mouse.kind {
                MouseEventKind::ScrollUp => 64,
                MouseEventKind::ScrollDown => 65,
                MouseEventKind::Drag(MouseButton::Left) => 32,
                MouseEventKind::Drag(MouseButton::Middle) => 33,
                MouseEventKind::Drag(MouseButton::Right) => 34,
                _ => match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                }
            };

            let seq = format!("\x1b[<{};{};{}{}", button_code, x, y, event_char);
            let _ = writer.write_all(seq.as_bytes());
            let _ = writer.flush();
        } else {
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    let current = screen.scrollback();
                    screen.set_scrollback(current + 3);
                }
                MouseEventKind::ScrollDown => {
                    let current = screen.scrollback();
                    screen.set_scrollback(current.saturating_sub(3));
                }
                _ => {}
            }
        }
    }
}

fn ui(f: &mut Frame, app: &mut App) {
    let size = f.size();

    let chunks = if size.height < 26 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(size)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(2)])
            .split(size)
    };

    let top_chunks = if size.width < 106 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(0)])
            .split(chunks[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Fill(1), Constraint::Length(25)])
            .split(chunks[0])
    };

    let term_area = top_chunks[0];
    let right_pane = top_chunks[1];
    f.render_widget(Block::default().style(Style::default().bg(Color::Black)), right_pane);
    
    let right_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(0),
            Constraint::Fill(1),
            Constraint::Length(1),
        ])
        .split(right_pane);

    let scrollbar_area = right_layout[0];
    let sidebar_area = right_layout[2];
    let sidebar_scrollbar_area = right_layout[3];

    {
        let mut parser = app.parser.lock().unwrap();
        let (p_rows, p_cols) = parser.screen().size();
        if term_area.height != p_rows || term_area.width != p_cols {
            let _ = app.master.resize(PtySize {
                rows: term_area.height,
                cols: term_area.width,
                pixel_width: 0,
                pixel_height: 0,
            });
            parser.screen_mut().set_size(term_area.height, term_area.width);
        }
    }

    for y in term_area.top()..term_area.bottom() {
        for x in term_area.left()..term_area.right() {
            f.buffer_mut().get_mut(x, y).reset();
        }
    }

    {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        
        for row in 0..term_area.height {
            for col in 0..term_area.width {
                let x = term_area.x + col;
                let y = term_area.y + row;
                
                if let Some(cell) = screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }

                    let mut style = Style::default();
                    match cell.fgcolor() {
                        vt100::Color::Rgb(r, g, b) => { style = style.fg(Color::Rgb(r, g, b)); }
                        vt100::Color::Idx(i) => { style = style.fg(Color::Indexed(i)); }
                        _ => {} 
                    }
                    match cell.bgcolor() {
                        vt100::Color::Rgb(r, g, b) => { style = style.bg(Color::Rgb(r, g, b)); }
                        vt100::Color::Idx(i) => { style = style.bg(Color::Indexed(i)); }
                        _ => {} 
                    }
                    
                    if cell.bold() { style = style.add_modifier(Modifier::BOLD); }
                    if cell.italic() { style = style.add_modifier(Modifier::ITALIC); }
                    if cell.inverse() { style = style.add_modifier(Modifier::REVERSED); }
                    if cell.underline() { style = style.add_modifier(Modifier::UNDERLINED); }
                    
                    if let (Some(start), Some(end)) = (app.selection_start, app.selection_end) {
                        let (s_row, s_col) = start;
                        let (e_row, e_col) = end;
                        let (min_row, min_col, max_row, max_col) = if s_row < e_row || (s_row == e_row && s_col <= e_col) {
                            (s_row, s_col, e_row, e_col)
                        } else {
                            (e_row, e_col, s_row, s_col)
                        };
                        let is_selected = if row > min_row && row < max_row {
                            true
                        } else if row == min_row && row == max_row {
                            col >= min_col && col <= max_col
                        } else if row == min_row {
                            col >= min_col
                        } else if row == max_row {
                            col <= max_col
                        } else {
                            false
                        };
                        if is_selected {
                            style = style.bg(Color::Rgb(80, 80, 80)).fg(Color::White);
                        }
                    }

                    let symbol = cell.contents();
                    let draw_sym = if symbol.is_empty() { " " } else { symbol };
                    f.buffer_mut().set_string(x, y, draw_sym, style);
                } else {
                    f.buffer_mut().get_mut(x, y).set_symbol(" ").set_style(Style::default());
                }
            }
        }

        let (cursor_row, cursor_col) = screen.cursor_position();
        let cx = (term_area.x + cursor_col).min(term_area.right().saturating_sub(1));
        let cy = (term_area.y + cursor_row).min(term_area.bottom().saturating_sub(1));
        f.set_cursor(cx, cy);

        let is_app_active = screen.alternate_screen() || screen.hide_cursor() || screen.application_cursor();
        
        let sidebar_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(sidebar_area);
        let _search_area = sidebar_layout[0];
        let list_area = sidebar_layout[1];
        
        let search_style = if app.is_search_focused {
            Style::default().fg(Color::Yellow).bg(Color::Rgb(30, 30, 30))
        } else {
            Style::default().fg(Color::DarkGray).bg(Color::Black)
        };
 
        let now = Instant::now().duration_since(app.start_time).as_millis();
        let cursor_char = if app.is_search_focused && (now / 500) % 2 == 0 { "_" } else { " " };
        
        let search_text = if app.search_query.is_empty() && !app.is_search_focused {
            format!(" Search {} commands .. ", app.sidebar_items.len())
        } else {
            format!(" {}{} ", app.search_query, cursor_char)
        };
        f.render_widget(Paragraph::new(search_text).style(search_style), sidebar_layout[0]);

        let filtered_items: Vec<(usize, &String)> = app.sidebar_items.iter().enumerate()
            .filter(|(i, label)| {
                let info = &app.sidebar_infos[*i];
                let query = app.search_query.to_lowercase();
                let matched = label.to_lowercase().contains(&query) || info.to_lowercase().contains(&query);
                matched
            })
            .collect();

        let sidebar_list_items: Vec<ListItem> = filtered_items.iter().enumerate().map(|(idx, (i, item))| {
            let color = if app.sidebar_commands[*i].contains("<prompt") {
                Color::Green
            } else if app.sidebar_commands[*i].contains("sudo") {
                Color::Red
            } else {
                Color::White
            };

            let item_bg = if is_app_active {
                Color::Black
            } else if app.sidebar_state.selected() == Some(idx) {
                Color::Rgb(60, 60, 60)
            } else {
                Color::Black
            };

            let style = if is_app_active {
                Style::default().fg(Color::DarkGray).bg(item_bg)
            } else if app.sidebar_state.selected() == Some(idx) {
                Style::default().fg(color).bg(item_bg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(color).bg(item_bg)
            };
            
            let offset = app.sidebar_state.offset();
            let is_hovered = if let Some((mx, my)) = app.mouse_pos {
                mx == list_area.x + 1 && my == list_area.y + (idx as u16).saturating_sub(offset as u16) && idx >= offset && idx < offset + list_area.height as usize
            } else {
                false
            };

            let indicator_style = if is_hovered {
                Style::default().fg(Color::Black).bg(Color::White).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg)
            };

            let symbol = "▸";
            
            let info = &app.sidebar_infos[*i];
            let line = Line::from(vec![
                Span::styled(" ", Style::default().bg(item_bg)),
                Span::styled(symbol, indicator_style),
                Span::styled(item.as_str(), style),
                Span::styled(format!(" {}", info), Style::default().fg(Color::DarkGray).bg(item_bg)),
            ]);
            ListItem::new(line).style(Style::default().bg(item_bg))
        }).collect();

        let sidebar_list = List::new(sidebar_list_items)
            .style(Style::default().bg(Color::Black))
            .highlight_style(if is_app_active { 
                Style::default()
            } else { 
                Style::default().add_modifier(Modifier::BOLD)
            })
            .highlight_symbol("");
        f.render_stateful_widget(sidebar_list, sidebar_layout[1], &mut app.sidebar_state);

        let sidebar_scroll_pos = app.sidebar_state.selected().unwrap_or(0);
        let mut sidebar_scrollbar_state = ScrollbarState::new(filtered_items.len())
            .position(sidebar_scroll_pos);

        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some(" "))
                .thumb_symbol("┃")
                .track_style(Style::default().bg(Color::Black).fg(Color::Rgb(20, 20, 20)))
                .thumb_style(Style::default().fg(Color::Rgb(50, 50, 50))),
            sidebar_scrollbar_area,
            &mut sidebar_scrollbar_state,
        );
        f.render_widget(Block::default().style(Style::default().bg(Color::Black)), chunks[1]);
        
        let status_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(chunks[1]);
            
        let has_selection = app.selection_start.is_some() && app.selection_end.is_some();
        let copy_width = if has_selection { 6 } else { 0 };

        let bar_chunks_upper = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(8), // MENU
                Constraint::Length(copy_width), // COPY
                Constraint::Min(0),
                Constraint::Length(30),
            ])
            .split(status_chunks[0]);

        let bar_chunks_lower = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(0),
                Constraint::Length(10),
            ])
            .split(status_chunks[1]);
            
        let is_menu_hovered = if let Some((mx, my)) = app.mouse_pos {
            bar_chunks_upper[0].contains(Position::new(mx, my))
        } else { false };
        
        let is_copy_hovered = if has_selection {
            if let Some((mx, my)) = app.mouse_pos {
                bar_chunks_upper[1].contains(Position::new(mx, my))
            } else { false }
        } else { false };

        let is_exit_hovered = if let Some((mx, my)) = app.mouse_pos {
            bar_chunks_lower[1].contains(Position::new(mx, my))
        } else { false };

        let menu_bg = if is_menu_hovered { Color::Rgb(150, 255, 150) } else { Color::Green };
        let copy_bg = if is_copy_hovered { Color::Rgb(150, 255, 255) } else { Color::Cyan };
        let exit_bg = if is_exit_hovered { Color::Rgb(255, 100, 100) } else { Color::Red };

        let menu_span = Span::styled(" ≡ MENU ", Style::default().bg(menu_bg).fg(Color::Black).add_modifier(Modifier::BOLD));
        let copy_span = Span::styled(" COPY ", Style::default().bg(copy_bg).fg(Color::Black).add_modifier(Modifier::BOLD));
        let exit_span = Span::styled(" EXIT ", Style::default().bg(exit_bg).fg(Color::White).add_modifier(Modifier::BOLD));
        
        f.render_widget(Paragraph::new(menu_span).style(Style::default().bg(Color::Black)), bar_chunks_upper[0]);
        if has_selection {
            f.render_widget(Paragraph::new(copy_span).style(Style::default().bg(Color::Black)), bar_chunks_upper[1]);
        }
        
        if let Some(idx) = app.sidebar_state.selected() {
            if idx < app.sidebar_items.len() {
                let label = &app.sidebar_items[idx];
                let info = &app.sidebar_infos[idx];
                let cmd = &app.sidebar_commands[idx];
                let status_line = Line::from(vec![
                    Span::styled(" # ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{} {} ", label, info), Style::default().fg(Color::White)),
                    Span::styled(cmd.to_string(), Style::default().fg(Color::DarkGray)),
                ]);
                f.render_widget(
                    Paragraph::new(status_line).style(Style::default().bg(Color::Black)),
                    bar_chunks_upper[2]
                );
            }
        }
        
        let (rows, _) = screen.size();
        let scroll_offset = screen.scrollback();
        let history_len = screen.scrollback_len();
        let total_lines = rows as usize + history_len; 
        let line_info = format!("{}/{}", scroll_offset, total_lines);
        
        let clock_str = format!("{} | {}", line_info, chrono::Local::now().format("%H:%M:%S"));
        f.render_widget(
            Paragraph::new(Span::styled(clock_str, Style::default().fg(Color::White)))
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().bg(Color::Black)), 
            bar_chunks_upper[3]
        );

        let cpu_color = if app.cpu_usage > 80.0 { Color::Red } else { Color::Yellow };
        let git_str = app.git_info.as_deref().unwrap_or("");
        let line2 = Line::from(vec![
            Span::styled(format!("CPU: {:.0}%", app.cpu_usage.round()), Style::default().fg(cpu_color)),
            Span::raw(" | "),
            Span::styled(format!("MEM: {}GB/{}GB", (app.mem_usage.0 as f64 / 1_073_741_824.0).round() as u64, (app.mem_usage.1 as f64 / 1_073_741_824.0).round() as u64), Style::default().fg(Color::Cyan)),
            Span::raw(" | "),
            Span::styled(git_str, Style::default().fg(Color::Magenta)),
        ]);
        f.render_widget(Paragraph::new(line2).style(Style::default().bg(Color::Black)), bar_chunks_lower[0]);
        f.render_widget(
            Paragraph::new(exit_span)
                .alignment(ratatui::layout::Alignment::Right)
                .style(Style::default().bg(Color::Black)), 
            bar_chunks_lower[1]
        );

        f.render_widget(Block::default().style(Style::default().bg(Color::Black)), scrollbar_area);
        let history_len = screen.scrollback_len();
        let scroll_pos = history_len.saturating_sub(scroll_offset);

        let mut scrollbar_state = ScrollbarState::new(history_len)
            .position(scroll_pos);

        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(Style::default().fg(Color::Rgb(30, 30, 30)))
                .thumb_style(Style::default().fg(Color::Rgb(60, 60, 60)))
                .begin_style(Style::default().fg(Color::Rgb(60, 60, 60)))
                .end_style(Style::default().fg(Color::Rgb(60, 60, 60))),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }

    if app.show_menu {
        let area = centered_rect(40, 30, size);
        f.render_widget(Clear, area);
        let menu_block = Block::default()
            .title(" Commands ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .style(Style::default().bg(Color::Black));
        let menu_items = vec![
            ListItem::new("b: Run Btop").style(Style::default().bg(Color::Black)), 
            ListItem::new("s: Check Space").style(Style::default().bg(Color::Black)), 
            ListItem::new("q: Close").style(Style::default().bg(Color::Black))
        ];
        f.render_widget(List::new(menu_items).block(menu_block).style(Style::default().bg(Color::Black)), area);
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)].as_ref())
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)].as_ref())
        .split(popup_layout[1])[1]
}
