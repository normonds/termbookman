use crate::app::{is_deletable, App, Message, SidebarMode};
use crate::config::{ActionType, ItemType};
use crate::entry::{Entry, EntryKind};
use crate::input::{encode_mouse, key_to_bytes};
use crate::ui::{calculate_layout, modals, statusbar};
use crate::utils::{log_debug, shell_quote};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vt100::MouseProtocolMode;

fn write_pty(app: &mut App, bytes: &[u8]) {
    let _ = app.pty_write.write_all(bytes);
    let _ = app.pty_write.flush();
}

pub fn sanitize_exe_path(exe: &Path) -> PathBuf {
    let s = exe.to_string_lossy();
    if let Some(stripped) = s.strip_suffix(" (deleted)") {
        PathBuf::from(stripped)
    } else {
        exe.to_path_buf()
    }
}

#[allow(dead_code)]
fn exe_word() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    shell_quote(&sanitize_exe_path(&exe).to_string_lossy())
}

/// The shell line that runs a sidebar entry. Entries from files are referenced by file and key,
/// so the command run is exactly the one shown, whatever labels other sources produce.
pub fn run_line(entry: &Entry) -> String {
    run_line_with_exe(entry, &std::env::current_exe().unwrap_or_default())
}

pub fn run_line_with_exe(entry: &Entry, exe: &Path) -> String {
    let exe_str = shell_quote(&sanitize_exe_path(exe).to_string_lossy());
    match (&entry.kind, &entry.path) {
        (EntryKind::History, _) => format!("{}\r", entry.command),
        (_, Some(path)) => format!(
            "{} run {} {}\r",
            exe_str,
            shell_quote(&path.to_string_lossy()),
            shell_quote(&entry.key)
        ),
        (_, None) => format!("{} {}\r", exe_str, shell_quote(&entry.label)),
    }
}

/// The shell line for a status bar `send_command`. Commands with `<prompt:...>` tags go through
/// `termbookman exec` so the prompts are asked interactively inside the terminal.
pub fn send_command_line(cmd: &str) -> String {
    send_command_line_with_exe(cmd, &std::env::current_exe().unwrap_or_default())
}

pub fn send_command_line_with_exe(cmd: &str, exe: &Path) -> String {
    if !cmd.contains("<prompt:") {
        return cmd.to_string();
    }
    let body = cmd.trim_end_matches(['\r', '\n']);
    let exe_str = shell_quote(&sanitize_exe_path(exe).to_string_lossy());
    format!("{} exec {}\r", exe_str, shell_quote(body))
}

pub fn open_in_editor(app: &mut App, path: &std::path::Path) {
    let cmd = format!(
        "{} {}\r",
        app.config.external_editor,
        shell_quote(&path.to_string_lossy())
    );
    write_pty(app, cmd.as_bytes());
}

/// Starts the GitHub device login unless one is already running.
pub fn start_device_flow(app: &mut App, tx: &mpsc::Sender<Message>) {
    let Some(client_id) = app.config.auth.github_client_id.clone() else {
        app.login_error = Some("No github_client_id found in config.".to_string());
        return;
    };
    if app.device_flow_running {
        return;
    }
    app.device_flow_running = true;
    app.device_flow_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    app.login_error = Some("Fetching GitHub code...".to_string());
    let scope = app.config.auth.scope.clone();
    let tx = tx.clone();
    std::thread::spawn(
        move || match crate::github::request_device_code(&client_id, &scope) {
            Ok(code) => {
                let _ = tx.send(Message::DeviceCodeSuccess(code));
            }
            Err(e) => {
                log_debug(&format!("GitHub auth error: {}", e));
                let _ = tx.send(Message::AuthError(e));
                let _ = tx.send(Message::DeviceFlowFinished);
            }
        },
    );
}

/// Keys pressed while the menu modal is open.
pub fn handle_menu_key(app: &mut App, key: &KeyEvent) {
    if let KeyCode::Char(c) = key.code {
        if let Some(item) = app.config.menu.iter().find(|m| m.key == c) {
            let cmd = format!("{}\r", item.command);
            write_pty(app, cmd.as_bytes());
        }
    }
    app.show_menu = false;
}

/// Forwards a key press to the PTY.
pub fn forward_key(app: &mut App, key: &KeyEvent) {
    let app_cursor = {
        let mut p = app.lock_parser();
        p.screen_mut().set_scrollback(0);
        p.screen().application_cursor()
    };
    if let Some(bytes) = key_to_bytes(key, app_cursor) {
        write_pty(app, &bytes);
    }
}

fn select_by_scrollbar(app: &mut App, row: u16, track: Rect) {
    let count = app.filtered_indices().len();
    if track.height == 0 || count == 0 {
        return;
    }
    let row = row.clamp(track.y, track.bottom().saturating_sub(1));
    let pos = (row - track.y) as f32 / track.height as f32;
    let visual_idx = ((count as f32 * pos) as usize).min(count - 1);
    app.sidebar_state.select(Some(visual_idx));
}

fn scrollback_from_track(app: &mut App, row: u16, track: Rect) {
    // The first and last rows of the scrollbar are the arrow buttons.
    let inner_y = track.y + 1;
    let inner_h = track.height.saturating_sub(2);
    if inner_h == 0 {
        return;
    }
    let row = row.clamp(inner_y, inner_y + inner_h - 1);
    let mut p = app.lock_parser();
    let history_len = p.screen().scrollback_len() as f32;
    let pos = (row - inner_y) as f32 / inner_h.saturating_sub(1).max(1) as f32;
    p.screen_mut()
        .set_scrollback((history_len * (1.0 - pos)).round() as usize);
}

fn scroll_terminal(app: &mut App, lines: isize) {
    let mut p = app.lock_parser();
    let current = p.screen().scrollback() as isize;
    p.screen_mut()
        .set_scrollback((current + lines).max(0) as usize);
}

pub fn handle_click(app: &mut App, mouse: MouseEvent, size: Rect, tx: &mpsc::Sender<Message>) {
    let layout = calculate_layout(size, app.sidebar_width);
    let pos = Position::new(mouse.column, mouse.row);
    let left_down = mouse.kind == MouseEventKind::Down(MouseButton::Left);

    if app.show_delete_confirm {
        let rects = modals::delete_confirm_layout(size);
        if rects.modal_area.contains(pos) && left_down {
            if rects.yes_button.contains(pos) {
                if let Some(path) = app.gist_to_delete.take() {
                    let _ = tx.send(Message::DeleteGist(path));
                }
                app.show_delete_confirm = false;
            } else if rects.no_button.contains(pos) {
                app.show_delete_confirm = false;
                app.gist_to_delete = None;
            }
        }
        return;
    }

    if app.show_settings_modal {
        let rects = modals::settings_layout(size);
        if rects.modal_area.contains(pos) {
            let focus = |app: &mut App, pat: bool, url: bool, editor: bool| {
                app.is_pat_focused = pat;
                app.is_update_url_focused = url;
                app.is_editor_focused = editor;
            };
            if rects.device_flow_or_login.contains(pos) && left_down {
                if !app.device_flow_running && app.auth_token.is_none() {
                    start_device_flow(app, tx);
                }
                return;
            }
            if rects.clear_token_button.contains(pos) && left_down {
                if app.auth_token.is_some() || !app.pat_input.is_empty() {
                    app.auth_token = None;
                    app.pat_input.clear();
                    app.config.auth.personal_access_token = None;
                    crate::try_save_config(app);
                    app.set_github_status("✓ Token cleared / Logged out", false, 4);
                }
                return;
            }
            if rects.pat_input.contains(pos) {
                if left_down {
                    focus(app, true, false, false);
                }
                return;
            }
            if rects.update_url_input.contains(pos) {
                if left_down {
                    focus(app, false, true, false);
                }
                return;
            }
            if rects.update_button.contains(pos) {
                if left_down && app.update_disabled_reason.is_none() {
                    let _ = tx.send(Message::UpdateBinary);
                }
                return;
            }
            if rects.editor_input.contains(pos) {
                if left_down {
                    focus(app, false, false, true);
                }
                return;
            }
        }
        if left_down {
            app.is_pat_focused = false;
            app.is_update_url_focused = false;
            app.is_editor_focused = false;
            if !rects.modal_area.contains(pos) {
                app.show_settings_modal = false;
            }
        }
        return;
    }

    let (mouse_mode, mouse_enc, alt_screen, app_cursor) = {
        let parser = app.lock_parser();
        let screen = parser.screen();
        (
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
            screen.alternate_screen(),
            screen.application_cursor(),
        )
    };

    // Programs that enabled mouse reporting get the events; Shift bypasses this so text can
    // still be selected locally, as in xterm.
    let in_term = layout.term_area.contains(pos);
    if in_term
        && mouse_mode != MouseProtocolMode::None
        && !mouse.modifiers.contains(KeyModifiers::SHIFT)
        && !app.is_selecting
    {
        let col = mouse.column - layout.term_area.x + 1;
        let row = mouse.row - layout.term_area.y + 1;
        if let Some(bytes) =
            encode_mouse(mouse.kind, mouse.modifiers, col, row, mouse_mode, mouse_enc)
        {
            write_pty(app, &bytes);
        }
        return;
    }

    if let MouseEventKind::Up(MouseButton::Left) = mouse.kind {
        app.is_dragging_sidebar = false;
        app.is_dragging_term_scrollbar = false;
        app.is_dragging_sidebar_scrollbar = false;
    }

    if app.is_dragging_term_scrollbar {
        scrollback_from_track(app, mouse.row, layout.scrollbar_area);
        return;
    }

    if app.is_dragging_sidebar {
        let new_width = size.width.saturating_sub(mouse.column);
        app.sidebar_width = new_width.clamp(10, size.width.saturating_sub(20).max(10));
        return;
    }

    if left_down && layout.scrollbar_area.contains(pos) {
        let track = layout.scrollbar_area;
        if mouse.modifiers.contains(KeyModifiers::CONTROL) {
            app.is_dragging_sidebar = true;
        } else if mouse.row == track.y {
            scroll_terminal(app, 3);
        } else if mouse.row + 1 == track.bottom() {
            scroll_terminal(app, -3);
        } else {
            app.is_dragging_term_scrollbar = true;
            scrollback_from_track(app, mouse.row, track);
        }
        return;
    }

    if left_down {
        let bars = [
            (app.config.statusbar.upper.clone(), layout.status_chunks[0]),
            (app.config.statusbar.lower.clone(), layout.status_chunks[1]),
        ];
        for (bar_items, bar_chunk) in bars {
            for (item, chunk) in statusbar::bar_layout(app, &bar_items, bar_chunk) {
                if !chunk.contains(pos) {
                    continue;
                }
                if item.type_ == ItemType::GithubStatus && !app.unsynced_files.is_empty() {
                    let _ = tx.send(Message::UploadUnsynced);
                    return;
                }
                let Some(action) = item.action else {
                    continue;
                };
                match action {
                    ActionType::ToggleMenu => app.show_menu = !app.show_menu,
                    ActionType::CopySelection => app.copy_selection(),
                    ActionType::SendCommand => {
                        if let Some(cmd) = item.command {
                            if cmd.contains("<prompt:") {
                                app.open_prompt_dialog(&cmd);
                            } else {
                                let line = send_command_line(&cmd);
                                write_pty(app, line.as_bytes());
                            }
                        }
                    }
                    ActionType::Quit => app.should_quit = true,
                    ActionType::OpenLog => {
                        let log_path = crate::utils::log_path();
                        if !log_path.exists() {
                            log_debug("Log opened");
                        }
                        open_in_editor(app, &log_path);
                    }
                    ActionType::ShowSettingsModal => {
                        app.show_settings_modal = !app.show_settings_modal;
                        if app.show_settings_modal {
                            app.editor_input = app.config.external_editor.clone();
                            if let Some(token) = &app.auth_token {
                                app.pat_input = token.clone();
                            } else {
                                app.pat_input = String::new();
                            }
                        }
                    }
                    ActionType::FetchGists => {
                        let _ = tx.send(Message::FetchGists { switch_tab: true });
                    }
                }
                return;
            }
        }
    }

    let is_sidebar_locked = crate::ui::sidebar::is_sidebar_locked(app);
    let sa = layout.sidebar_area;
    let s_rects = crate::ui::sidebar::sidebar_layout(
        sa,
        app.sidebar_mode == SidebarMode::Gists,
        is_sidebar_locked,
    );
    let tabs_area = s_rects.tabs;
    let search_area = s_rects.search;
    let list_area = s_rects.list;
    let new_button_area = s_rects.new_button;

    if sa.contains(pos) || layout.sidebar_scrollbar_area.contains(pos) {
        if is_sidebar_locked {
            return;
        }

        if let Some(area) = new_button_area {
            if area.contains(pos) {
                if left_down {
                    let _ = tx.send(Message::CreateNewGist);
                }
                return;
            }
        }

        if tabs_area.contains(pos) {
            if left_down {
                let tabs_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Ratio(1, 3),
                        Constraint::Ratio(1, 3),
                        Constraint::Ratio(1, 3),
                    ])
                    .split(tabs_area);

                if tabs_chunks[0].contains(pos) {
                    app.sidebar_mode = SidebarMode::Commands;
                } else if tabs_chunks[1].contains(pos) {
                    app.sidebar_mode = SidebarMode::History;
                    app.refresh_history();
                } else if tabs_chunks[2].contains(pos) {
                    app.sidebar_mode = SidebarMode::Gists;
                }
                app.sidebar_state.select(Some(0));
                app.recompute_filtered_indices();
            }
            return;
        }

        if search_area.contains(pos) {
            if left_down {
                app.is_search_focused = true;
            }
            return;
        }
    }
    if left_down {
        app.is_search_focused = false;
    }

    if list_area.contains(pos) && !is_sidebar_locked {
        handle_list_mouse(app, mouse, list_area);
        return;
    }

    if !is_sidebar_locked
        && (layout.sidebar_scrollbar_area.contains(pos) || app.is_dragging_sidebar_scrollbar)
    {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = true;
                select_by_scrollbar(app, mouse.row, layout.sidebar_scrollbar_area);
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved
                if app.is_dragging_sidebar_scrollbar =>
            {
                select_by_scrollbar(app, mouse.row, layout.sidebar_scrollbar_area);
            }
            _ => {}
        }
        return;
    }

    if in_term || app.is_selecting {
        let tx_ = mouse.column.saturating_sub(layout.term_area.x);
        let ty = mouse.row.saturating_sub(layout.term_area.y);
        let max_col = layout.term_area.width.saturating_sub(1);
        let max_row = layout.term_area.height.saturating_sub(1);
        let cell = (ty.min(max_row), tx_.min(max_col));
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let now = Instant::now();
                let is_repeat = app.last_term_click_cell == Some(cell)
                    && app
                        .last_term_click_time
                        .is_some_and(|t| now.duration_since(t) < Duration::from_millis(350));

                let count = if is_repeat {
                    app.term_click_count = (app.term_click_count % 3) + 1;
                    app.term_click_count
                } else {
                    app.term_click_count = 1;
                    1
                };
                app.last_term_click_time = Some(now);
                app.last_term_click_cell = Some(cell);

                if count == 2 {
                    // Double click: select word
                    let mut start_c = cell.1;
                    let mut end_c = cell.1;
                    {
                        let parser = app.lock_parser();
                        let screen = parser.screen();
                        let is_word_char = |c: u16| {
                            screen.cell(cell.0, c).is_some_and(|cell| {
                                cell.has_contents()
                                    && !cell.contents().chars().any(|ch| ch.is_whitespace())
                            })
                        };
                        while start_c > 0 && is_word_char(start_c - 1) {
                            start_c -= 1;
                        }
                        while end_c < max_col && is_word_char(end_c + 1) {
                            end_c += 1;
                        }
                    }
                    app.is_selecting = false;
                    app.selection_start = Some((cell.0, start_c));
                    app.selection_end = Some((cell.0, end_c));
                } else if count == 3 {
                    // Triple click: select full line
                    app.is_selecting = false;
                    app.selection_start = Some((cell.0, 0));
                    app.selection_end = Some((cell.0, max_col));
                } else {
                    app.is_selecting = true;
                    app.selection_start = Some(cell);
                    app.selection_end = Some(cell);
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved if app.is_selecting => {
                app.selection_end = Some(cell);
            }
            MouseEventKind::Up(MouseButton::Left) if app.is_selecting => {
                app.is_selecting = false;
                if app.selection_start == app.selection_end {
                    app.selection_start = None;
                    app.selection_end = None;
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = mouse.kind == MouseEventKind::ScrollUp;
                if alt_screen {
                    // Full-screen programs without mouse support (less, man) scroll with arrows.
                    let key = KeyEvent::new(
                        if up { KeyCode::Up } else { KeyCode::Down },
                        KeyModifiers::NONE,
                    );
                    if let Some(bytes) = key_to_bytes(&key, app_cursor) {
                        write_pty(app, &bytes.repeat(3));
                    }
                } else {
                    scroll_terminal(app, if up { 3 } else { -3 });
                }
            }
            _ => {}
        }
    }
}

fn handle_list_mouse(app: &mut App, mouse: MouseEvent, list_area: Rect) {
    let filtered = app.filtered_indices();
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            let current = app.sidebar_state.selected().unwrap_or(0);
            app.sidebar_state.select(Some(current.saturating_sub(1)));
        }
        MouseEventKind::ScrollDown => {
            let current = app.sidebar_state.selected().unwrap_or(0);
            if current + 1 < filtered.len() {
                app.sidebar_state.select(Some(current + 1));
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            let index = (mouse.row - list_area.y) as usize + app.sidebar_state.offset();
            let Some(&original_index) = filtered.get(index) else {
                return;
            };
            let entry = app.entries()[original_index].clone();
            let now = Instant::now();
            let is_double_click = app.last_clicked_index == Some(index)
                && app
                    .last_click_time
                    .is_some_and(|t| now.duration_since(t) < Duration::from_millis(300));

            if is_double_click {
                app.last_click_time = None;
                app.last_clicked_index = None;
                if let Some(path) = entry
                    .path
                    .as_ref()
                    .filter(|_| entry.kind != EntryKind::History)
                {
                    if let Ok(mtime) = std::fs::metadata(path).and_then(|m| m.modified()) {
                        app.editing_file = Some((path.clone(), mtime));
                    }
                    open_in_editor(app, path);
                }
                return;
            }

            app.sidebar_state.select(Some(index));
            app.last_click_time = Some(now);
            app.last_clicked_index = Some(index);

            let x = mouse.column - list_area.x;
            let is_delete_click = app.sidebar_mode == SidebarMode::Gists
                && is_deletable(&entry)
                && x >= list_area.width.saturating_sub(4)
                && x < list_area.width.saturating_sub(1);

            if is_delete_click {
                app.show_delete_confirm = true;
                app.gist_to_delete = entry.path.clone();
            } else if entry.command.contains("<prompt:") {
                app.open_prompt_dialog(&entry.command);
            } else {
                let line = run_line(&entry);
                write_pty(app, line.as_bytes());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(kind: EntryKind, path: Option<&str>, key: &str) -> Entry {
        Entry {
            label: "label2".into(),
            key: key.into(),
            command: "echo hi".into(),
            info: String::new(),
            kind,
            path: path.map(PathBuf::from),
            mtime: None,
            remote_name: None,
        }
    }

    #[test]
    fn run_line_keeps_working_when_current_exe_ends_in_deleted() {
        let deleted_exe = Path::new("/usr/local/bin/termbookman (deleted)");
        assert_eq!(
            sanitize_exe_path(deleted_exe),
            PathBuf::from("/usr/local/bin/termbookman")
        );
        let line = run_line_with_exe(
            &entry(EntryKind::Command, Some("/commands.txt"), "key"),
            deleted_exe,
        );
        assert!(
            line.starts_with("'/usr/local/bin/termbookman' run")
                || line.starts_with("/usr/local/bin/termbookman run")
        );
        assert!(!line.contains("(deleted)"));
    }

    #[test]
    fn run_line_references_file_and_key_with_quoting() {
        let line = run_line(&entry(
            EntryKind::Command,
            Some("/home/a b/commands.txt"),
            "it's",
        ));
        assert!(
            line.ends_with(" run '/home/a b/commands.txt' 'it'\\''s'\r"),
            "{}",
            line
        );
        assert!(line.starts_with(&exe_word()));
    }

    #[test]
    fn run_line_types_history_directly() {
        assert_eq!(run_line(&entry(EntryKind::History, None, "")), "echo hi\r");
    }

    #[test]
    fn send_command_line_only_wraps_prompt_commands() {
        assert_eq!(send_command_line("git status\r"), "git status\r");
        let wrapped = send_command_line("ssh <prompt:Host>\r");
        assert!(
            wrapped.ends_with(" exec 'ssh <prompt:Host>'\r"),
            "{}",
            wrapped
        );
    }

    #[test]
    fn terminal_double_click_selects_word_and_triple_click_selects_line() {
        use std::sync::{Arc, Mutex};
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let parser = Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0)));
        {
            let mut p = parser.lock().unwrap();
            p.process(b"echo 'hello-world' 123\r\n");
        }

        let mut app = App::new(
            Box::new(std::io::sink()),
            pair.master,
            parser,
            Vec::new(),
            std::process::id(),
        );

        let (tx, _rx) = std::sync::mpsc::channel();
        let size = Rect::new(0, 0, 100, 24);

        // First click on row 0, col 7 ('h' in 'hello-world')
        let click_event = crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 7,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        handle_click(&mut app, click_event, size, &tx);
        assert_eq!(app.term_click_count, 1);
        assert_eq!(app.selection_start, Some((0, 7)));

        // Double click at same cell within 350ms
        handle_click(&mut app, click_event, size, &tx);
        assert_eq!(app.term_click_count, 2);
        // The word "'hello-world'" should be selected (from col 5 to col 17)
        assert_eq!(app.selection_start, Some((0, 5)));
        assert_eq!(app.selection_end, Some((0, 17)));

        // Triple click at same cell
        handle_click(&mut app, click_event, size, &tx);
        assert_eq!(app.term_click_count, 3);
        // Full line should be selected (col 0 to max_col of term_area)
        let max_col = crate::ui::calculate_layout(size, crate::DEFAULT_SIDEBAR_WIDTH)
            .term_area
            .width
            .saturating_sub(1);
        assert_eq!(app.selection_start, Some((0, 0)));
        assert_eq!(app.selection_end, Some((0, max_col)));
    }
}
