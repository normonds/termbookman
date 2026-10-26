use crate::app::{is_deletable, App, Message, SidebarMode};
use crate::config::ActionType;
use crate::entry::{Entry, EntryKind};
use crate::input::{encode_mouse, key_to_bytes};
use crate::ui::{calculate_layout, modals, statusbar};
use crate::utils::{log_debug, shell_quote};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vt100::MouseProtocolMode;

fn write_pty(app: &mut App, bytes: &[u8]) {
    let _ = app.pty_write.write_all(bytes);
    let _ = app.pty_write.flush();
}

fn exe_word() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    shell_quote(&exe.to_string_lossy())
}

/// The shell line that runs a sidebar entry. Entries from files are referenced by file and key,
/// so the command run is exactly the one shown, whatever labels other sources produce.
pub fn run_line(entry: &Entry) -> String {
    match (&entry.kind, &entry.path) {
        (EntryKind::History, _) => format!("{}\r", entry.command),
        (_, Some(path)) => format!(
            "{} run {} {}\r",
            exe_word(),
            shell_quote(&path.to_string_lossy()),
            shell_quote(&entry.key)
        ),
        (_, None) => format!("{} {}\r", exe_word(), shell_quote(&entry.label)),
    }
}

/// The shell line for a status bar `send_command`. Commands with `<prompt:...>` tags go through
/// `termbookman exec` so the prompts are asked interactively inside the terminal.
pub fn send_command_line(cmd: &str) -> String {
    if !cmd.contains("<prompt:") {
        return cmd.to_string();
    }
    let body = cmd.trim_end_matches(['\r', '\n']);
    format!("{} exec {}\r", exe_word(), shell_quote(body))
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
    std::thread::spawn(move || match crate::github::request_device_code(&client_id, &scope) {
        Ok(code) => {
            let _ = tx.send(Message::DeviceCodeSuccess(code));
        }
        Err(e) => {
            log_debug(&format!("GitHub auth error: {}", e));
            let _ = tx.send(Message::AuthError(e));
            let _ = tx.send(Message::DeviceFlowFinished);
        }
    });
}

/// Keys pressed while the menu modal is open.
pub fn handle_menu_key(app: &mut App, key: &KeyEvent) {
    match key.code {
        KeyCode::Char('b') => write_pty(app, b"btop\r"),
        KeyCode::Char('s') => write_pty(app, b"df -h\r"),
        _ => {}
    }
    app.show_menu = false;
}

/// Forwards a key press to the PTY.
pub fn forward_key(app: &mut App, key: &KeyEvent) {
    let app_cursor = {
        let mut p = app.parser.lock().unwrap();
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
    let mut p = app.parser.lock().unwrap();
    let history_len = p.screen().scrollback_len() as f32;
    let pos = (row - inner_y) as f32 / inner_h.saturating_sub(1).max(1) as f32;
    p.screen_mut()
        .set_scrollback((history_len * (1.0 - pos)).round() as usize);
}

fn scroll_terminal(app: &mut App, lines: isize) {
    let mut p = app.parser.lock().unwrap();
    let current = p.screen().scrollback() as isize;
    p.screen_mut()
        .set_scrollback((current + lines).max(0) as usize);
}

pub fn handle_click(app: &mut App, mouse: MouseEvent, size: Rect, tx: &mpsc::Sender<Message>) {
    let layout = calculate_layout(size, app.sidebar_width);
    let pos = Position::new(mouse.column, mouse.row);
    let left_down = mouse.kind == MouseEventKind::Down(MouseButton::Left);

    if app.show_delete_confirm {
        let area = modals::centered_rect_fixed(50, 10, size);
        if area.contains(pos) && left_down {
            let inner = ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .inner(area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // warning
                    Constraint::Length(2), // name
                    Constraint::Length(1), // spacer
                    Constraint::Length(1), // buttons
                    Constraint::Min(0),
                ])
                .split(inner);

            if chunks[3].contains(pos) {
                let button_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(chunks[3]);

                if button_chunks[0].contains(pos) {
                    if let Some(path) = app.gist_to_delete.take() {
                        let _ = tx.send(Message::DeleteGist(path));
                    }
                    app.show_delete_confirm = false;
                } else if button_chunks[1].contains(pos) {
                    app.show_delete_confirm = false;
                    app.gist_to_delete = None;
                }
            }
        }
        return;
    }

    if app.show_settings_modal {
        let area = modals::centered_rect_fixed(100, 26, size);
        if area.contains(pos) {
            let inner_area = ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .inner(area);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(1), // Header
                    Constraint::Length(2), // Device Flow
                    Constraint::Length(1), // PAT Header
                    Constraint::Length(3), // PAT Input
                    Constraint::Length(1), // Status
                    Constraint::Length(1), // Spacer
                    Constraint::Length(1), // Update URL Header
                    Constraint::Length(3), // Update URL Input
                    Constraint::Length(1), // Update Warning
                    Constraint::Length(1), // Update Button
                    Constraint::Length(1), // Spacer
                    Constraint::Length(1), // Editor Header
                    Constraint::Length(3), // Editor Input
                    Constraint::Min(0),
                ])
                .split(inner_area);

            let focus = |app: &mut App, pat: bool, url: bool, editor: bool| {
                app.is_pat_focused = pat;
                app.is_update_url_focused = url;
                app.is_editor_focused = editor;
            };
            if chunks[3].contains(pos) {
                if left_down {
                    focus(app, true, false, false);
                }
                return;
            }
            if chunks[7].contains(pos) {
                if left_down {
                    focus(app, false, true, false);
                }
                return;
            }
            if chunks[9].contains(pos) {
                if left_down && app.update_disabled_reason.is_none() {
                    let _ = tx.send(Message::UpdateBinary);
                }
                return;
            }
            if chunks[12].contains(pos) {
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
            if !area.contains(pos) {
                app.show_settings_modal = false;
            }
        }
        return;
    }

    let (mouse_mode, mouse_enc, alt_screen, app_cursor) = {
        let parser = app.parser.lock().unwrap();
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
                let Some(action) = item.action else {
                    continue;
                };
                match action {
                    ActionType::ToggleMenu => app.show_menu = !app.show_menu,
                    ActionType::CopySelection => app.copy_selection(),
                    ActionType::SendCommand => {
                        if let Some(cmd) = item.command {
                            let line = send_command_line(&cmd);
                            write_pty(app, line.as_bytes());
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
                                start_device_flow(app, tx);
                            }
                        }
                    }
                    ActionType::FetchGists => {
                        let _ = tx.send(Message::FetchGists);
                    }
                }
                return;
            }
        }
    }

    let is_app_active = alt_screen || app_cursor || {
        let parser = app.parser.lock().unwrap();
        parser.screen().hide_cursor()
    };
    let is_sidebar_locked = is_app_active || app.is_pty_busy;

    let sa = layout.sidebar_area;
    let tabs_area = Rect::new(sa.x, sa.y, sa.width, 1);
    let search_area = Rect::new(sa.x, sa.y + 1, sa.width, 1);
    let show_new_button = app.sidebar_mode == SidebarMode::Gists && !is_sidebar_locked;
    let list_area = Rect::new(
        sa.x,
        sa.y + 2,
        sa.width,
        sa.height.saturating_sub(if show_new_button { 3 } else { 2 }),
    );
    let new_button_area = show_new_button.then(|| {
        Rect::new(sa.x, sa.y + sa.height.saturating_sub(1), sa.width, 1)
    });

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
                app.is_selecting = true;
                app.selection_start = Some(cell);
                app.selection_end = Some(cell);
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved
                if app.is_selecting =>
            {
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
                if let Some(path) = entry.path.as_ref().filter(|_| entry.kind != EntryKind::History)
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
            } else if x <= 1 {
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
    fn run_line_references_file_and_key_with_quoting() {
        let line = run_line(&entry(EntryKind::Command, Some("/home/a b/commands.txt"), "it's"));
        assert!(line.ends_with(" run '/home/a b/commands.txt' 'it'\\''s'\r"), "{}", line);
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
        assert!(wrapped.ends_with(" exec 'ssh <prompt:Host>'\r"), "{}", wrapped);
    }
}
