use crate::app::{App, Message, SidebarMode};
use crate::config::{ActionType, ItemType};
use crate::github::upload_gist;
use crate::ui::{calculate_layout, modals};
use crate::utils::log_debug;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

pub fn handle_click(app: &mut App, mouse: MouseEvent, size: Rect, tx: &mpsc::Sender<Message>) {
    let layout = calculate_layout(size, app.sidebar_width);

    if app.show_delete_confirm {
        let area = modals::centered_rect_fixed(50, 10, size);
        if area.contains(Position::new(mouse.column, mouse.row)) {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let block =
                    ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
                let inner = block.inner(area);

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

                if chunks[3].contains(Position::new(mouse.column, mouse.row)) {
                    let button_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(chunks[3]);

                    if button_chunks[0].contains(Position::new(mouse.column, mouse.row)) {
                        // Clicked [Y]
                        if let Some(idx) = app.gist_index_to_delete {
                            let _ = tx.send(Message::DeleteGist(idx));
                        }
                        app.show_delete_confirm = false;
                        app.gist_index_to_delete = None;
                    } else if button_chunks[1].contains(Position::new(mouse.column, mouse.row)) {
                        // Clicked [N]
                        app.show_delete_confirm = false;
                        app.gist_index_to_delete = None;
                    }
                }
            }
        }
        return;
    }

    if app.show_upload_confirm {
        let area = modals::centered_rect_fixed(50, 10, size);
        if area.contains(Position::new(mouse.column, mouse.row)) {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let block =
                    ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
                let inner = block.inner(area);

                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1), // filename
                        Constraint::Length(2), // question
                        Constraint::Length(1), // spacer
                        Constraint::Length(1), // choices
                        Constraint::Min(0),
                    ])
                    .split(inner);

                if chunks[3].contains(Position::new(mouse.column, mouse.row)) {
                    let choices_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                        .split(chunks[3]);

                    if choices_chunks[0].contains(Position::new(mouse.column, mouse.row)) {
                        // Clicked [Y]
                        if let (Some(path), Some(token)) =
                            (app.pending_gist_file.take(), app.auth_token.clone())
                        {
                            let mut remote_name = None;
                            for (i, p) in app.gist_paths.iter().enumerate() {
                                if let Some(p) = p {
                                    if p == &path {
                                        remote_name = Some(app.gist_remote_names[i].clone());
                                        break;
                                    }
                                }
                            }
                            if let Some(remote_name) = remote_name {
                                let tx = tx.clone();
                                let display_name = remote_name.clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(Message::GistUploadStatus(
                                        format!("[Uploading Gist: {}]", display_name),
                                        true,
                                    ));
                                    if let Err(e) = upload_gist(&token, &path, &remote_name) {
                                        let _ = tx.send(Message::GistUploadStatus(
                                            format!("✗ Gist upload failed: {}", e),
                                            false,
                                        ));
                                    } else {
                                        let _ = tx.send(Message::GistUploadStatus(
                                            format!("✓ Gist updated: {}", display_name),
                                            true,
                                        ));
                                    }
                                });
                            }
                        }
                        app.show_upload_confirm = false;
                    } else if choices_chunks[1].contains(Position::new(mouse.column, mouse.row)) {
                        // Clicked [N]
                        app.show_upload_confirm = false;
                        app.pending_gist_file = None;
                    }
                }
            }
        }
        return;
    }

    if app.show_settings_modal {
        let area = modals::centered_rect_fixed(100, 26, size);
        if area.contains(Position::new(mouse.column, mouse.row)) {
            let block = ratatui::widgets::Block::default().borders(ratatui::widgets::Borders::ALL);
            let inner_area = block.inner(area);
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
                    Constraint::Length(1), // Update Warning (new)
                    Constraint::Length(1), // Update Button
                    Constraint::Length(1), // Spacer
                    Constraint::Length(1), // Editor Header
                    Constraint::Length(3), // Editor Input
                    Constraint::Min(0),
                ])
                .split(inner_area);

            if chunks[3].contains(Position::new(mouse.column, mouse.row)) {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    app.is_pat_focused = true;
                    app.is_update_url_focused = false;
                    app.is_editor_focused = false;
                }
                return;
            }
            if chunks[7].contains(Position::new(mouse.column, mouse.row)) {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    app.is_update_url_focused = true;
                    app.is_pat_focused = false;
                    app.is_editor_focused = false;
                }
                return;
            }
            if chunks[9].contains(Position::new(mouse.column, mouse.row)) {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    // Only allow update if not disabled
                    if app.update_disabled_reason.is_none() {
                        let _ = tx.send(Message::UpdateBinary);
                    }
                }
                return;
            }
            if chunks[12].contains(Position::new(mouse.column, mouse.row)) {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    app.is_editor_focused = true;
                    app.is_pat_focused = false;
                    app.is_update_url_focused = false;
                }
                return;
            }
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            app.is_pat_focused = false;
            app.is_update_url_focused = false;
            app.is_editor_focused = false;
            if !area.contains(Position::new(mouse.column, mouse.row)) {
                app.show_settings_modal = false;
            }
        }
        return;
    }

    let (mouse_mode, mouse_enc) = {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        (
            screen.mouse_protocol_mode(),
            screen.mouse_protocol_encoding(),
        )
    };

    if layout
        .term_area
        .contains(Position::new(mouse.column, mouse.row))
        && mouse_mode != MouseProtocolMode::None
    {
        let tx = (mouse.column.saturating_sub(layout.term_area.x) + 1) as i32;
        let ty = (mouse.row.saturating_sub(layout.term_area.y) + 1) as i32;

        let btn = match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => 0,
            MouseEventKind::Down(MouseButton::Middle) => 1,
            MouseEventKind::Down(MouseButton::Right) => 2,
            MouseEventKind::Up(MouseButton::Left) => 0,
            MouseEventKind::Up(MouseButton::Middle) => 1,
            MouseEventKind::Up(MouseButton::Right) => 2,
            MouseEventKind::Drag(MouseButton::Left) => 32,
            MouseEventKind::Drag(MouseButton::Middle) => 33,
            MouseEventKind::Drag(MouseButton::Right) => 34,
            MouseEventKind::Moved => 35,
            MouseEventKind::ScrollUp => 64,
            MouseEventKind::ScrollDown => 65,
            _ => 0,
        };

        let is_up = matches!(mouse.kind, MouseEventKind::Up(_));
        let suffix = if is_up { 'm' } else { 'M' };

        if mouse_enc == MouseProtocolEncoding::Sgr {
            let seq = format!("\x1b[<{};{};{}{}", btn, tx, ty, suffix);
            let _ = app.pty_write.write_all(seq.as_bytes());
            let _ = app.pty_write.flush();
            return;
        }
    }

    if let MouseEventKind::Up(MouseButton::Left) = mouse.kind {
        app.is_dragging_sidebar = false;
        app.is_dragging_term_scrollbar = false;
        app.is_dragging_sidebar_scrollbar = false;
    }

    if app.is_dragging_term_scrollbar {
        let mut p = app.parser.lock().unwrap();
        let screen = p.screen();
        let history_len = screen.scrollback_len();
        if history_len > 0 {
            let relative_y = mouse.row.saturating_sub(layout.scrollbar_area.y) as f32;
            let percent = relative_y / layout.scrollbar_area.height as f32;
            let new_pos = (percent * history_len as f32) as usize;
            let target_offset = history_len.saturating_sub(new_pos);
            p.screen_mut().set_scrollback(target_offset);
        }
        return;
    }

    if app.is_dragging_sidebar {
        let new_width = size.width.saturating_sub(mouse.column);
        app.sidebar_width = new_width.clamp(10, size.width.saturating_sub(20));
        return;
    }

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        if layout
            .scrollbar_area
            .contains(Position::new(mouse.column, mouse.row))
        {
            if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                app.is_dragging_sidebar = true;
            } else {
                app.is_dragging_term_scrollbar = true;
            }
            return;
        }
    }

    if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        let bars = vec![
            (app.config.statusbar.upper.clone(), layout.status_chunks[0]),
            (app.config.statusbar.lower.clone(), layout.status_chunks[1]),
        ];

        for (bar_items, bar_chunk) in bars {
            let mut constraints = Vec::new();
            let mut visible_items = Vec::new();

            for item in bar_items {
                if app.is_item_visible(&item) {
                    let width = if let Some(w) = item.width {
                        Constraint::Length(w)
                    } else if item.type_ == ItemType::Spacer
                        || item.type_ == ItemType::GitInfo
                        || item.type_ == ItemType::SelectedCommandInfo
                    {
                        Constraint::Fill(1)
                    } else if let Some(label) = &item.label {
                        Constraint::Length(label.len() as u16)
                    } else {
                        Constraint::Min(0)
                    };
                    constraints.push(width);
                    visible_items.push(item);
                }
            }

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(constraints)
                .split(bar_chunk);

            for (i, item) in visible_items.into_iter().enumerate() {
                let chunk = chunks[i];
                if chunk.contains(Position::new(mouse.column, mouse.row)) {
                    if let Some(action) = item.action {
                        match action {
                            ActionType::ToggleMenu => {
                                app.show_menu = !app.show_menu;
                            }
                            ActionType::CopySelection => {
                                app.copy_selection();
                            }
                            ActionType::SendCommand => {
                                if let Some(cmd) = item.command {
                                    let mut final_cmd = cmd.clone();
                                    app.process_prompt(&mut final_cmd);
                                    let _ = app.pty_write.write_all(final_cmd.as_bytes());
                                    let _ = app.pty_write.flush();
                                }
                            }
                            ActionType::Quit => {
                                app.should_quit = true;
                            }
                            ActionType::ShowSettingsModal => {
                                app.show_settings_modal = !app.show_settings_modal;
                                if app.show_settings_modal {
                                    app.editor_input = app.config.external_editor.clone();
                                    if let Some(token) = &app.auth_token {
                                        app.pat_input = token.clone();
                                    } else if let Some(client_id) =
                                        &app.config.auth.github_client_id
                                    {
                                        app.login_error =
                                            Some("Fetching GitHub code...".to_string());
                                        let client_id = client_id.clone();
                                        let scope = app.config.auth.scope.clone();
                                        let tx = tx.clone();
                                        std::thread::spawn(move || {
                                            let url = "https://github.com/login/device/code";
                                            let client = reqwest::blocking::Client::new();
                                            let mut params =
                                                vec![("client_id", client_id.as_str())];
                                            if !scope.is_empty() {
                                                params.push(("scope", scope.as_str()));
                                            }
                                            match client
                                                .post(url)
                                                .header("Accept", "application/json")
                                                .query(&params)
                                                .send()
                                            {
                                                Ok(res) => {
                                                    let status = res.status();
                                                    if let Ok(json) =
                                                        res.json::<serde_json::Value>()
                                                    {
                                                        if let (
                                                            Some(device),
                                                            Some(user),
                                                            Some(uri),
                                                        ) = (
                                                            json["device_code"].as_str(),
                                                            json["user_code"].as_str(),
                                                            json["verification_uri"].as_str(),
                                                        ) {
                                                            let _ = tx.send(
                                                                Message::DeviceCodeSuccess(
                                                                    device.to_string(),
                                                                    user.to_string(),
                                                                    uri.to_string(),
                                                                ),
                                                            );
                                                            return;
                                                        }
                                                        if let Some(error) = json["error"].as_str()
                                                        {
                                                            let desc = json["error_description"]
                                                                .as_str()
                                                                .unwrap_or(error);
                                                            let _ = tx.send(Message::AuthError(
                                                                format!("GitHub: {}", desc),
                                                            ));
                                                            log_debug(&format!(
                                                                "GitHub auth error: {} - {}",
                                                                error, desc
                                                            ));
                                                            return;
                                                        }
                                                        log_debug(&format!("Failed to parse GitHub JSON (status {}): {}", status, json));
                                                    } else {
                                                        log_debug(&format!("Failed to parse GitHub response as JSON (status {})", status));
                                                    }
                                                    let _ = tx.send(Message::AuthError(format!(
                                                        "Failed to parse device code (Status: {})",
                                                        status
                                                    )));
                                                }
                                                Err(_) => {
                                                    let _ = tx.send(Message::AuthError(
                                                        "Network error fetching code.".to_string(),
                                                    ));
                                                }
                                            }
                                        });
                                    } else {
                                        app.login_error = Some(
                                            "No github_client_id found in config.".to_string(),
                                        );
                                    }
                                }
                            }
                            ActionType::FetchGists => {
                                let _ = tx.send(Message::FetchGists);
                            }
                        }
                    }
                }
            }
        }
    }

    let is_app_active = {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();
        screen.alternate_screen() || screen.hide_cursor() || screen.application_cursor()
    };
    let is_sidebar_locked = is_app_active || app.is_pty_busy;

    let tabs_area = Rect::new(
        layout.sidebar_area.x,
        layout.sidebar_area.y,
        layout.sidebar_area.width,
        1,
    );
    let (new_button_area, search_area, list_area) =
        if app.sidebar_mode == SidebarMode::Gists && !is_sidebar_locked {
            (
                Some(Rect::new(
                    layout.sidebar_area.x,
                    layout.sidebar_area.y + layout.sidebar_area.height.saturating_sub(1),
                    layout.sidebar_area.width,
                    1,
                )),
                Rect::new(
                    layout.sidebar_area.x,
                    layout.sidebar_area.y + 1,
                    layout.sidebar_area.width,
                    1,
                ),
                Rect::new(
                    layout.sidebar_area.x,
                    layout.sidebar_area.y + 2,
                    layout.sidebar_area.width,
                    layout.sidebar_area.height.saturating_sub(3),
                ),
            )
        } else {
            (
                None,
                Rect::new(
                    layout.sidebar_area.x,
                    layout.sidebar_area.y + 1,
                    layout.sidebar_area.width,
                    1,
                ),
                Rect::new(
                    layout.sidebar_area.x,
                    layout.sidebar_area.y + 2,
                    layout.sidebar_area.width,
                    layout.sidebar_area.height.saturating_sub(2),
                ),
            )
        };

    if is_sidebar_locked {
        return;
    }

    if let Some(area) = new_button_area {
        if area.contains(Position::new(mouse.column, mouse.row)) {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                let _ = tx.send(Message::CreateNewGist);
            }
            return;
        }
    }

    if tabs_area.contains(Position::new(mouse.column, mouse.row)) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            let tabs_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                ])
                .split(tabs_area);

            if tabs_chunks[0].contains(Position::new(mouse.column, mouse.row)) {
                app.sidebar_mode = SidebarMode::Commands;
            } else if tabs_chunks[1].contains(Position::new(mouse.column, mouse.row)) {
                app.sidebar_mode = SidebarMode::History;
                app.refresh_history();
            } else if tabs_chunks[2].contains(Position::new(mouse.column, mouse.row)) {
                app.sidebar_mode = SidebarMode::Gists;
            }
            app.sidebar_state.select(Some(0));
        }
        return;
    }

    if search_area.contains(Position::new(mouse.column, mouse.row)) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            app.is_search_focused = true;
        }
        return;
    } else if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
        app.is_search_focused = false;
    }

    if list_area.contains(Position::new(mouse.column, mouse.row)) && !is_app_active {
        let (items, infos, _commands) = match app.sidebar_mode {
            SidebarMode::Commands => (
                &app.sidebar_items,
                Some(&app.sidebar_infos),
                &app.sidebar_commands,
            ),
            SidebarMode::History => (&app.history_items, None, &app.history_commands),
            SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
        };

        let filtered: Vec<(usize, &String)> = items
            .iter()
            .enumerate()
            .filter(|(idx, label)| {
                let matches_label = label
                    .to_lowercase()
                    .contains(&app.search_query.to_lowercase());
                let matches_info = if let Some(infos) = infos {
                    infos[*idx]
                        .to_lowercase()
                        .contains(&app.search_query.to_lowercase())
                } else {
                    false
                };
                matches_label || matches_info
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
                    let (original_index, label) = filtered[index];
                    let now = Instant::now();
                    let is_double_click = if let (Some(last_time), Some(last_idx)) =
                        (app.last_click_time, app.last_clicked_index)
                    {
                        now.duration_since(last_time) < Duration::from_millis(300)
                            && last_idx == index
                    } else {
                        false
                    };

                    if is_double_click {
                        app.last_click_time = None;
                        app.last_clicked_index = None;

                        let (path, cmd_str) = match app.sidebar_mode {
                            SidebarMode::Commands => {
                                let p = &app.sidebar_paths[original_index];
                                (
                                    p.clone(),
                                    p.as_ref()
                                        .map(|path| {
                                            let p_str =
                                                path.to_string_lossy().replace("'", "'\\''");
                                            format!("{} '{}'\r", app.config.external_editor, p_str)
                                        })
                                        .unwrap_or_default(),
                                )
                            }
                            SidebarMode::Gists => {
                                let p = &app.gist_paths[original_index];
                                (
                                    p.clone(),
                                    p.as_ref()
                                        .map(|path| {
                                            let p_str =
                                                path.to_string_lossy().replace("'", "'\\''");
                                            format!("{} '{}'\r", app.config.external_editor, p_str)
                                        })
                                        .unwrap_or_default(),
                                )
                            }
                            SidebarMode::History => (None, String::new()),
                        };

                        if let (Some(p), _) = (&path, &cmd_str) {
                            if let Ok(metadata) = std::fs::metadata(p) {
                                if let Ok(mtime) = metadata.modified() {
                                    app.editing_file = Some((p.clone(), mtime));
                                }
                            }
                        }

                        if !cmd_str.is_empty() {
                            let _ = app.pty_write.write_all(cmd_str.as_bytes());
                            let _ = app.pty_write.flush();
                        }
                    } else {
                        app.sidebar_state.select(Some(index));
                        app.last_click_time = Some(now);
                        app.last_clicked_index = Some(index);

                        let sidebar_x = mouse.column.saturating_sub(list_area.x);

                        let is_script = if app.sidebar_mode == SidebarMode::Gists {
                            app.gist_infos
                                .get(original_index)
                                .map(|inf| inf.starts_with("__SCRIPT__"))
                                .unwrap_or(false)
                        } else {
                            app.sidebar_infos
                                .get(original_index)
                                .map(|inf| inf.starts_with("__SCRIPT__"))
                                .unwrap_or(false)
                        };

                        let is_delete_click = app.sidebar_mode == SidebarMode::Gists
                            && is_script
                            && sidebar_x >= list_area.width.saturating_sub(4)
                            && sidebar_x < list_area.width.saturating_sub(1);

                        if is_delete_click {
                            app.show_delete_confirm = true;
                            app.gist_index_to_delete = Some(original_index);
                        } else if sidebar_x <= 1 {
                            if label.starts_with("GIST: ") && app.sidebar_mode == SidebarMode::Gists
                            {
                                let _ = tx.send(Message::FetchGists);
                            } else {
                                let cmd_str = match app.sidebar_mode {
                                    SidebarMode::History => {
                                        format!("{}\r", app.history_commands[original_index])
                                    }
                                    SidebarMode::Gists | SidebarMode::Commands => {
                                        let (items, _, _) = match app.sidebar_mode {
                                            SidebarMode::Commands => (
                                                &app.sidebar_items,
                                                Some(&app.sidebar_infos),
                                                &app.sidebar_commands,
                                            ),
                                            SidebarMode::History => {
                                                (&app.history_items, None, &app.history_commands)
                                            }
                                            SidebarMode::Gists => (
                                                &app.gist_items,
                                                Some(&app.gist_infos),
                                                &app.gist_commands,
                                            ),
                                        };
                                        let label = &items[original_index];
                                        let exe = std::env::current_exe().unwrap_or_default();
                                        if is_script {
                                            format!("{} script {}\r", exe.display(), label)
                                        } else {
                                            format!("{} {}\r", exe.display(), label)
                                        }
                                    }
                                };
                                let _ = app.pty_write.write_all(cmd_str.as_bytes());
                                let _ = app.pty_write.flush();
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    } else if layout
        .sidebar_scrollbar_area
        .contains(Position::new(mouse.column, mouse.row))
        || app.is_dragging_sidebar_scrollbar
    {
        let (items, infos, _) = match app.sidebar_mode {
            SidebarMode::Commands => (
                &app.sidebar_items,
                Some(&app.sidebar_infos),
                &app.sidebar_commands,
            ),
            SidebarMode::History => (&app.history_items, None, &app.history_commands),
            SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
        };

        let filtered: Vec<(usize, &String)> = items
            .iter()
            .enumerate()
            .filter(|(idx, label)| {
                let matches_label = label
                    .to_lowercase()
                    .contains(&app.search_query.to_lowercase());
                let matches_info = if let Some(infos) = infos {
                    infos[*idx]
                        .to_lowercase()
                        .contains(&app.search_query.to_lowercase())
                } else {
                    false
                };
                matches_label || matches_info
            })
            .collect();

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = true;
                let track_height = layout.sidebar_scrollbar_area.height as f32;
                let click_pos = (mouse.row.saturating_sub(layout.sidebar_scrollbar_area.y)) as f32;
                if track_height > 0.0 && !filtered.is_empty() {
                    let visual_idx = ((filtered.len() as f32 * (click_pos / track_height))
                        as usize)
                        .min(filtered.len() - 1);
                    let (original_index, _) = filtered[visual_idx];
                    app.sidebar_state.select(Some(original_index));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved
                if app.is_dragging_sidebar_scrollbar =>
            {
                let track_height = layout.sidebar_scrollbar_area.height as f32;
                let mouse_y = mouse.row.clamp(
                    layout.sidebar_scrollbar_area.y,
                    layout.sidebar_scrollbar_area.bottom().saturating_sub(1),
                );
                let click_pos = (mouse_y.saturating_sub(layout.sidebar_scrollbar_area.y)) as f32;
                if track_height > 0.0 && !filtered.is_empty() {
                    let visual_idx = ((filtered.len() as f32 * (click_pos / track_height))
                        as usize)
                        .min(filtered.len() - 1);
                    let (original_index, _) = filtered[visual_idx];
                    app.sidebar_state.select(Some(original_index));
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                app.is_dragging_sidebar_scrollbar = false;
            }
            _ => {}
        }
    } else if layout
        .scrollbar_area
        .contains(Position::new(mouse.column, mouse.row))
        || app.is_dragging_scrollbar
    {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                app.is_dragging_scrollbar = true;
                let mut parser = app.parser.lock().unwrap();
                let screen = parser.screen_mut();
                let current = screen.scrollback();

                if mouse.row == layout.scrollbar_area.y {
                    screen.set_scrollback(current + 3);
                } else if mouse.row == layout.scrollbar_area.bottom() - 1 {
                    screen.set_scrollback(current.saturating_sub(3));
                } else {
                    let track_height = layout.scrollbar_area.height.saturating_sub(2) as f32;
                    let click_pos = (mouse.row.saturating_sub(layout.scrollbar_area.y + 1)) as f32;
                    let history_len = screen.scrollback_len() as f32;
                    if track_height > 0.0 {
                        let target_scroll =
                            (history_len * (1.0 - (click_pos / track_height))) as usize;
                        screen.set_scrollback(target_scroll);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved
                if app.is_dragging_scrollbar =>
            {
                let mut parser = app.parser.lock().unwrap();
                let screen = parser.screen_mut();
                let track_height = layout.scrollbar_area.height.saturating_sub(2) as f32;
                let mouse_y = mouse.row.clamp(
                    layout.scrollbar_area.y + 1,
                    layout.scrollbar_area.bottom().saturating_sub(2),
                );
                let click_pos = (mouse_y.saturating_sub(layout.scrollbar_area.y + 1)) as f32;
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
    } else if layout
        .term_area
        .contains(Position::new(mouse.column, mouse.row))
        || app.is_selecting
    {
        let (parser, writer) = (&app.parser, &mut app.pty_write);
        let mut parser = parser.lock().unwrap();
        let screen = parser.screen_mut();

        let tx = mouse.column.saturating_sub(layout.term_area.x);
        let ty = mouse.row.saturating_sub(layout.term_area.y);

        if screen.mouse_protocol_mode() == MouseProtocolMode::None {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    app.is_selecting = true;
                    app.selection_start = Some((ty, tx));
                    app.selection_end = Some((ty, tx));
                    return;
                }
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Moved
                    if app.is_selecting =>
                {
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
            let x = mouse.column.saturating_sub(layout.term_area.x) + 1;
            let y = mouse.row.saturating_sub(layout.term_area.y) + 1;

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
                },
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
