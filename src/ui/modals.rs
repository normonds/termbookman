use crate::app::App;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsRects {
    pub modal_area: Rect,
    pub inner_area: Rect,
    pub device_flow_header: Rect,
    pub device_flow_or_login: Rect,
    pub pat_header: Rect,
    pub pat_input: Rect,
    pub status: Rect,
    pub clear_token_button: Rect,
    pub update_url_header: Rect,
    pub update_url_input: Rect,
    pub update_warning: Rect,
    pub update_button: Rect,
    pub editor_header: Rect,
    pub editor_input: Rect,
}

pub fn settings_layout(area: Rect) -> SettingsRects {
    let modal_area = centered_rect_fixed(100, 26, area);
    let inner_area = Block::default().borders(Borders::ALL).inner(modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(1), // 0: Header
            Constraint::Length(2), // 1: Device Flow / Login Button
            Constraint::Length(1), // 2: PAT Header
            Constraint::Length(3), // 3: PAT Input
            Constraint::Length(1), // 4: Status / Clear Token
            Constraint::Length(1), // 5: Spacer
            Constraint::Length(1), // 6: Repo URL Header
            Constraint::Length(3), // 7: Repo URL Input
            Constraint::Length(1), // 8: Update Warning
            Constraint::Length(1), // 9: Update Button
            Constraint::Length(1), // 10: Spacer
            Constraint::Length(1), // 11: Editor Header
            Constraint::Length(3), // 12: Editor Input
            Constraint::Min(0),
        ])
        .split(inner_area);

    let status_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(chunks[4]);

    SettingsRects {
        modal_area,
        inner_area,
        device_flow_header: chunks[0],
        device_flow_or_login: chunks[1],
        pat_header: chunks[2],
        pat_input: chunks[3],
        status: status_chunks[0],
        clear_token_button: status_chunks[1],
        update_url_header: chunks[6],
        update_url_input: chunks[7],
        update_warning: chunks[8],
        update_button: chunks[9],
        editor_header: chunks[11],
        editor_input: chunks[12],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteConfirmRects {
    pub modal_area: Rect,
    pub inner_area: Rect,
    pub warning: Rect,
    pub gist_name: Rect,
    pub yes_button: Rect,
    pub no_button: Rect,
}

pub fn delete_confirm_layout(area: Rect) -> DeleteConfirmRects {
    let modal_area = centered_rect_fixed(50, 10, area);
    let inner_area = Block::default().borders(Borders::ALL).inner(modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Warning
            Constraint::Length(2), // Gist name
            Constraint::Length(1), // Spacer
            Constraint::Length(1), // Buttons
            Constraint::Min(0),
        ])
        .split(inner_area);

    let button_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[3]);

    DeleteConfirmRects {
        modal_area,
        inner_area,
        warning: chunks[0],
        gist_name: chunks[1],
        yes_button: button_chunks[0],
        no_button: button_chunks[1],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewGistRects {
    pub modal_area: Rect,
    pub inner_area: Rect,
    pub question: Rect,
    pub input: Rect,
    pub help: Rect,
}

pub fn new_gist_layout(area: Rect) -> NewGistRects {
    let modal_area = centered_rect_fixed(50, 10, area);
    let inner_area = Block::default().borders(Borders::ALL).inner(modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Question
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Help
            Constraint::Min(0),
        ])
        .split(inner_area);

    NewGistRects {
        modal_area,
        inner_area,
        question: chunks[0],
        input: chunks[1],
        help: chunks[2],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptRects {
    pub modal_area: Rect,
    pub inner_area: Rect,
    pub question: Rect,
    pub input: Rect,
    pub help: Rect,
}

pub fn prompt_layout(area: Rect) -> PromptRects {
    let modal_area = centered_rect_fixed(60, 10, area);
    let inner_area = Block::default().borders(Borders::ALL).inner(modal_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // Question
            Constraint::Length(3), // Input field
            Constraint::Length(1), // Help
            Constraint::Min(0),
        ])
        .split(inner_area);

    PromptRects {
        modal_area,
        inner_area,
        question: chunks[0],
        input: chunks[1],
        help: chunks[2],
    }
}

pub fn render_modals(f: &mut Frame, app: &mut App, area: Rect) {
    if app.show_menu {
        let height = (app.config.menu.len() as u16 + 4).clamp(6, 20);
        let area = centered_rect_fixed(45, height, area);
        f.render_widget(Clear, area);
        let menu_block = Block::default()
            .title(" Commands ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Green))
            .style(Style::default().bg(Color::Black));
        let mut menu_items: Vec<ListItem> = app
            .config
            .menu
            .iter()
            .map(|item| {
                ListItem::new(format!("{}: {}", item.key, item.label))
                    .style(Style::default().bg(Color::Black).fg(Color::White))
            })
            .collect();
        menu_items.push(
            ListItem::new("q/Esc: Close")
                .style(Style::default().bg(Color::Black).fg(Color::DarkGray)),
        );
        f.render_widget(
            List::new(menu_items)
                .block(menu_block)
                .style(Style::default().bg(Color::Black)),
            area,
        );
    }

    if app.show_settings_modal {
        let rects = settings_layout(area);
        f.render_widget(Clear, rects.modal_area);

        let block = Block::default()
            .title(Span::styled(
                " Settings & GitHub Auth ",
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .style(Style::default().bg(Color::Black));

        f.render_widget(block, rects.modal_area);

        // --- Device Flow Section ---
        f.render_widget(
            Paragraph::new("GITHUB AUTHENTICATION:").style(Style::default().fg(Color::DarkGray)),
            rects.device_flow_header,
        );

        if let (Some(uri), Some(code)) = (&app.github_verification_uri, &app.github_user_code) {
            let device_text = format!("1. Open {}   2. Enter: {}", uri, code);
            f.render_widget(
                Paragraph::new(Span::styled(
                    device_text,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )),
                rects.device_flow_or_login,
            );
        } else if app.auth_token.is_none() {
            let is_login_hovered = if let Some((mx, my)) = app.mouse_pos {
                rects.device_flow_or_login.contains(Position::new(mx, my))
            } else {
                false
            };
            let style = if is_login_hovered {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(Color::Cyan)
                    .bg(Color::Rgb(20, 40, 50))
                    .add_modifier(Modifier::BOLD)
            };
            f.render_widget(
                Paragraph::new(" [ Log in with GitHub (Device Flow) ] ")
                    .style(style)
                    .alignment(ratatui::layout::Alignment::Center),
                rects.device_flow_or_login,
            );
        } else {
            f.render_widget(
                Paragraph::new("✓ Authenticated with GitHub")
                    .style(Style::default().fg(Color::Green)),
                rects.device_flow_or_login,
            );
        }

        // --- PAT Section ---
        f.render_widget(
            Paragraph::new("GITHUB PERSONAL ACCESS TOKEN (PAT):")
                .style(Style::default().fg(Color::DarkGray)),
            rects.pat_header,
        );

        let pat_style = if app.is_pat_focused {
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let pat_block =
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.is_pat_focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                });

        let now = Instant::now().duration_since(app.start_time).as_millis();
        let pat_cursor = if app.is_pat_focused && (now / 500).is_multiple_of(2) {
            "_"
        } else {
            " "
        };
        let pat_value = if app.is_pat_focused {
            app.pat_input.clone()
        } else if app.pat_input.len() > 8 {
            let last4 = &app.pat_input[app.pat_input.len() - 4..];
            let prefix = if app.pat_input.starts_with("ghp_") {
                "ghp_"
            } else {
                "••••"
            };
            format!("{}••••{}", prefix, last4)
        } else if !app.pat_input.is_empty() {
            "••••••••".to_string()
        } else {
            String::new()
        };
        let pat_display = format!(" {}{} ", pat_value, pat_cursor);
        f.render_widget(
            Paragraph::new(pat_display)
                .style(pat_style)
                .block(pat_block),
            rects.pat_input,
        );

        // --- Status/Error Section and Clear Token Button ---
        if let Some(err) = &app.login_error {
            f.render_widget(
                Paragraph::new(Span::styled(err, Style::default().fg(Color::Red))),
                rects.status,
            );
        } else if app.auth_token.is_some() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "✓ Authenticated",
                    Style::default().fg(Color::Green),
                )),
                rects.status,
            );
        } else {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "Enter PAT or click Log in above",
                    Style::default().fg(Color::DarkGray),
                )),
                rects.status,
            );
        }

        if app.auth_token.is_some() || !app.pat_input.is_empty() {
            let is_clear_hovered = if let Some((mx, my)) = app.mouse_pos {
                rects.clear_token_button.contains(Position::new(mx, my))
            } else {
                false
            };
            let clear_style = if is_clear_hovered {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Red).bg(Color::Rgb(40, 20, 20))
            };
            f.render_widget(
                Paragraph::new(" [ Clear token / Log out ] ")
                    .style(clear_style)
                    .alignment(ratatui::layout::Alignment::Right),
                rects.clear_token_button,
            );
        }

        // --- Update URL Section ---
        f.render_widget(
            Paragraph::new("UPDATE URL:").style(Style::default().fg(Color::DarkGray)),
            rects.update_url_header,
        );

        let update_url_style = if app.is_update_url_focused {
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let update_url_block =
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.is_update_url_focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                });

        let update_url_cursor = if app.is_update_url_focused && (now / 500).is_multiple_of(2) {
            "_"
        } else {
            " "
        };
        let update_url_display = format!(" {}{} ", app.update_url_input, update_url_cursor);
        f.render_widget(
            Paragraph::new(update_url_display)
                .style(update_url_style)
                .block(update_url_block),
            rects.update_url_input,
        );

        // --- Self Update Button & Warning ---
        let update_area = rects.update_button;
        let is_update_hovered = if let Some((mx, my)) = app.mouse_pos {
            update_area.contains(Position::new(mx, my))
        } else {
            false
        };

        let disabled_reason = app.update_disabled_reason.as_deref();
        let (update_btn_style, update_btn_text) = match disabled_reason {
            Some(reason) => (
                Style::default()
                    .fg(Color::DarkGray)
                    .bg(Color::Rgb(30, 30, 30)),
                format!(" [ UPDATE DISABLED: {} ] ", reason),
            ),
            None if app.is_updating => (
                Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(40, 40, 20))
                    .add_modifier(Modifier::BOLD),
                " [ UPDATING... ] ".to_string(),
            ),
            None => {
                let style = if is_update_hovered {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Green)
                        .bg(Color::Rgb(20, 40, 20))
                        .add_modifier(Modifier::BOLD)
                };
                (style, " [ CHECK & INSTALL UPDATE ] ".to_string())
            }
        };

        f.render_widget(
            Paragraph::new(update_btn_text)
                .style(update_btn_style)
                .alignment(ratatui::layout::Alignment::Center),
            update_area,
        );

        // --- Editor Section ---
        f.render_widget(
            Paragraph::new("EXTERNAL EDITOR:").style(Style::default().fg(Color::DarkGray)),
            rects.editor_header,
        );

        let editor_style = if app.is_editor_focused {
            Style::default()
                .fg(Color::Yellow)
                .bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let editor_block =
            Block::default()
                .borders(Borders::ALL)
                .border_style(if app.is_editor_focused {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                });

        let editor_cursor = if app.is_editor_focused && (now / 500).is_multiple_of(2) {
            "_"
        } else {
            " "
        };
        let editor_display = format!(" {}{} ", app.editor_input, editor_cursor);
        f.render_widget(
            Paragraph::new(editor_display)
                .style(editor_style)
                .block(editor_block),
            rects.editor_input,
        );
    }

    if app.show_new_gist_dialog {
        let rects = new_gist_layout(area);
        f.render_widget(Clear, rects.modal_area);
        let block = Block::default()
            .title(Span::styled(
                " New Gist Script ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block, rects.modal_area);

        f.render_widget(
            Paragraph::new("Enter a name for the new Gist script:")
                .alignment(ratatui::layout::Alignment::Center),
            rects.question,
        );

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let now = Instant::now().duration_since(app.start_time).as_millis();
        let cursor = if (now / 500).is_multiple_of(2) {
            "_"
        } else {
            " "
        };
        let input_display = format!(" {}{} ", app.new_gist_name_input, cursor);
        f.render_widget(
            Paragraph::new(input_display)
                .style(Style::default().fg(Color::Cyan))
                .block(input_block),
            rects.input,
        );

        f.render_widget(
            Paragraph::new("ENTER to confirm, ESC to cancel")
                .style(Style::default().fg(Color::DarkGray)),
            rects.help,
        );
    }

    if app.show_delete_confirm {
        let rects = delete_confirm_layout(area);
        f.render_widget(Clear, rects.modal_area);
        let block = Block::default()
            .title(Span::styled(
                " Confirm Deletion ",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block, rects.modal_area);

        let target = app
            .gist_to_delete
            .as_ref()
            .and_then(|p| app.gist_entries.iter().find(|e| e.path.as_ref() == Some(p)));
        let gist_name = target
            .map(|e| e.label.clone())
            .unwrap_or_else(|| "Unknown Script".to_string());
        let file_name = target
            .and_then(|e| e.remote_name.clone())
            .unwrap_or_default();

        f.render_widget(
            Paragraph::new("Are you sure you want to delete this script?")
                .alignment(ratatui::layout::Alignment::Center),
            rects.warning,
        );
        f.render_widget(
            Paragraph::new(format!(
                "'{}' ({})\nThe file is removed locally and from its Gist.",
                gist_name, file_name
            ))
            .alignment(ratatui::layout::Alignment::Center),
            rects.gist_name,
        );

        let is_yes_hovered = if let Some((mx, my)) = app.mouse_pos {
            rects.yes_button.contains(Position::new(mx, my))
        } else {
            false
        };
        let is_no_hovered = if let Some((mx, my)) = app.mouse_pos {
            rects.no_button.contains(Position::new(mx, my))
        } else {
            false
        };

        let yes_style = if is_yes_hovered {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red).bg(Color::Rgb(40, 20, 20))
        };
        let no_style = if is_no_hovered {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray).bg(Color::Rgb(30, 30, 30))
        };

        f.render_widget(
            Paragraph::new(" [Y] Yes, Delete ")
                .style(yes_style)
                .alignment(ratatui::layout::Alignment::Center),
            rects.yes_button,
        );
        f.render_widget(
            Paragraph::new(" [N] No, Cancel ")
                .style(no_style)
                .alignment(ratatui::layout::Alignment::Center),
            rects.no_button,
        );
    }

    if let Some(prompt) = &app.prompt_state {
        let rects = prompt_layout(area);
        f.render_widget(Clear, rects.modal_area);
        let title_str = format!(
            " Prompt ({}/{}) ",
            prompt.current_idx + 1,
            prompt.prompts.len()
        );
        let block = Block::default()
            .title(Span::styled(
                title_str,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block, rects.modal_area);

        let label = &prompt.prompts[prompt.current_idx].0;
        f.render_widget(
            Paragraph::new(format!("Enter value for '{}':", label))
                .alignment(ratatui::layout::Alignment::Center),
            rects.question,
        );

        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let now = Instant::now().duration_since(app.start_time).as_millis();
        let cursor = if (now / 500).is_multiple_of(2) {
            "_"
        } else {
            " "
        };
        let input_display = format!(" {}{} ", prompt.current_input, cursor);
        f.render_widget(
            Paragraph::new(input_display)
                .style(Style::default().fg(Color::Yellow))
                .block(input_block),
            rects.input,
        );

        f.render_widget(
            Paragraph::new("ENTER to submit, ESC to cancel")
                .style(Style::default().fg(Color::DarkGray)),
            rects.help,
        );
    }
}

pub fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
    let width = width.min(r.width.saturating_sub(4));
    let height = height.min(r.height.saturating_sub(4));

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(r.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(r.width.saturating_sub(width) / 2),
            Constraint::Length(width),
            Constraint::Min(0),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_contains(parent: Rect, child: Rect) -> bool {
        child.x >= parent.x
            && child.y >= parent.y
            && child.x + child.width <= parent.x + parent.width
            && child.y + child.height <= parent.y + parent.height
    }

    #[test]
    fn settings_layout_rects_are_contained_within_modal() {
        let area = Rect::new(0, 0, 120, 40);
        let rects = settings_layout(area);

        assert!(rect_contains(area, rects.modal_area));
        assert!(rect_contains(rects.modal_area, rects.inner_area));
        assert!(rect_contains(rects.inner_area, rects.pat_input));
        assert!(rect_contains(rects.inner_area, rects.clear_token_button));
        assert!(rect_contains(rects.inner_area, rects.update_url_input));
        assert!(rect_contains(rects.inner_area, rects.update_button));
        assert!(rect_contains(rects.inner_area, rects.editor_input));

        // Status and clear token button must be on same row without overlap
        assert_eq!(rects.status.y, rects.clear_token_button.y);
        assert!(rects.status.x + rects.status.width <= rects.clear_token_button.x);
    }

    #[test]
    fn delete_confirm_and_new_gist_layouts_are_contained() {
        let area = Rect::new(0, 0, 100, 30);
        let del = delete_confirm_layout(area);
        assert!(rect_contains(area, del.modal_area));
        assert!(rect_contains(del.modal_area, del.inner_area));
        assert!(rect_contains(del.inner_area, del.yes_button));
        assert!(rect_contains(del.inner_area, del.no_button));
        assert!(del.yes_button.x + del.yes_button.width <= del.no_button.x);

        let gist = new_gist_layout(area);
        assert!(rect_contains(area, gist.modal_area));
        assert!(rect_contains(gist.modal_area, gist.inner_area));
        assert!(rect_contains(gist.inner_area, gist.input));
        assert!(rect_contains(gist.inner_area, gist.help));
    }

    #[test]
    fn prompt_layout_bounds_and_stacking() {
        let area = Rect::new(0, 0, 80, 24);
        let p = prompt_layout(area);
        assert!(rect_contains(area, p.modal_area));
        assert!(rect_contains(p.modal_area, p.inner_area));
        assert!(rect_contains(p.inner_area, p.question));
        assert!(rect_contains(p.inner_area, p.input));
        assert!(rect_contains(p.inner_area, p.help));
        assert!(p.question.y + p.question.height <= p.input.y);
        assert!(p.input.y + p.input.height <= p.help.y);
    }

    #[test]
    fn test_backend_renders_settings_modal() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;
        use std::sync::{Arc, Mutex};
        use vt100::Parser;

        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(portable_pty::PtySize {
                rows: 30,
                cols: 110,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();

        let mut app = App::new(
            Box::new(std::io::sink()),
            pair.master,
            Arc::new(Mutex::new(Parser::new(30, 110, 0))),
            Vec::new(),
            std::process::id(),
        );

        app.show_settings_modal = true;
        app.auth_token = None;
        app.pat_input = "ghp_1234567890abcdef".to_string();

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::render(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer();
        let rendered_text: String = buffer.content().iter().map(|c| c.symbol()).collect();

        assert!(
            rendered_text.contains("SETTINGS"),
            "Expected SETTINGS modal header in rendered output"
        );
        assert!(
            rendered_text.contains("PERSONAL ACCESS TOKEN"),
            "Expected PERSONAL ACCESS TOKEN header in rendered output"
        );
        assert!(
            rendered_text.contains("Log in with GitHub"),
            "Expected Login button in rendered output"
        );

        // When authenticated, it shows authenticated status
        app.auth_token = Some("ghp_test".to_string());
        terminal.draw(|f| crate::ui::render(f, &mut app)).unwrap();
        let auth_text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(auth_text.contains("Authenticated with GitHub"));
    }
}
