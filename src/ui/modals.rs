use ratatui::Frame;
use ratatui::layout::{Layout, Constraint, Direction, Rect};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use std::time::Instant;
use crate::app::App;

pub fn render_modals(f: &mut Frame, app: &mut App, area: Rect) {
    if app.show_menu {
        let area = centered_rect(40, 30, area);
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

    if app.show_settings_modal {
        let modal_area = centered_rect_fixed(50, 20, area);
        f.render_widget(Clear, modal_area);
        
        let block = Block::default()
            .title(Span::styled(" Settings & GitHub Auth ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .style(Style::default().bg(Color::Black));
            
        f.render_widget(block.clone(), modal_area);
        let inner_area = block.inner(modal_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1), // Header
                Constraint::Length(2), // Device Flow
                Constraint::Length(1), // PAT Header
                Constraint::Length(3), // PAT Input
                Constraint::Length(1), // Editor Header
                Constraint::Length(3), // Editor Input
                Constraint::Min(0),    // Status
            ])
            .split(inner_area);

        // --- Device Flow Section ---
        if let (Some(uri), Some(code)) = (&app.github_verification_uri, &app.github_user_code) {
            let device_text = format!("1. Open {}   2. Enter: {}", uri, code);
            f.render_widget(Paragraph::new("GITHUB DEVICE FLOW:").style(Style::default().fg(Color::DarkGray)), chunks[0]);
            f.render_widget(Paragraph::new(Span::styled(device_text, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))), chunks[1]);
        } else {
            f.render_widget(Paragraph::new("GITHUB DEVICE FLOW:").style(Style::default().fg(Color::DarkGray)), chunks[0]);
            f.render_widget(Paragraph::new("Enter the code on GitHub to authenticate...").style(Style::default().fg(Color::DarkGray)), chunks[1]);
        }

        // --- PAT Section ---
        f.render_widget(Paragraph::new("GITHUB PERSONAL ACCESS TOKEN (PAT):").style(Style::default().fg(Color::DarkGray)), chunks[2]);
        
        let pat_style = if app.is_pat_focused {
            Style::default().fg(Color::Yellow).bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let pat_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if app.is_pat_focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) });

        let now = Instant::now().duration_since(app.start_time).as_millis();
        let pat_cursor = if app.is_pat_focused && (now / 500) % 2 == 0 { "_" } else { " " };
        let pat_display = format!(" {}{} ", app.pat_input, pat_cursor);
        f.render_widget(Paragraph::new(pat_display).style(pat_style).block(pat_block), chunks[3]);

        // --- Editor Section ---
        f.render_widget(Paragraph::new("EXTERNAL EDITOR:").style(Style::default().fg(Color::DarkGray)), chunks[4]);
        
        let editor_style = if app.is_editor_focused {
            Style::default().fg(Color::Yellow).bg(Color::Rgb(20, 20, 20))
        } else {
            Style::default().fg(Color::Gray).bg(Color::Black)
        };

        let editor_block = Block::default()
            .borders(Borders::ALL)
            .border_style(if app.is_editor_focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) });

        let editor_cursor = if app.is_editor_focused && (now / 500) % 2 == 0 { "_" } else { " " };
        let editor_display = format!(" {}{} ", app.editor_input, editor_cursor);
        f.render_widget(Paragraph::new(editor_display).style(editor_style).block(editor_block), chunks[5]);

        // --- Status/Error Section ---
        if let Some(err) = &app.login_error {
            f.render_widget(Paragraph::new(Span::styled(err, Style::default().fg(Color::Red))), chunks[6]);
        } else if app.auth_token.is_some() {
            f.render_widget(Paragraph::new(Span::styled("✓ Authenticated", Style::default().fg(Color::Green))), chunks[6]);
        } else {
            f.render_widget(Paragraph::new(Span::styled("Waiting for input...", Style::default().fg(Color::DarkGray))), chunks[6]);
        }
    }

    if app.show_upload_confirm {
        let modal_area = centered_rect_fixed(50, 10, area);
        f.render_widget(Clear, modal_area);
        let block = Block::default()
            .title(Span::styled(" Upload Modified Gist? ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block.clone(), modal_area);
        let inner = block.inner(modal_area);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Filename
                Constraint::Length(2), // Question
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Buttons
                Constraint::Min(0),
            ])
            .split(inner);

        let filename = app.pending_gist_file.as_ref().and_then(|p| p.file_name()).map(|f| f.to_string_lossy()).unwrap_or_default();
        f.render_widget(Paragraph::new(format!("File '{}' was modified locally.", filename)).alignment(ratatui::layout::Alignment::Center), chunks[0]);
        f.render_widget(Paragraph::new("\nUpload changes to GitHub Gist?").alignment(ratatui::layout::Alignment::Center), chunks[1]);

        let button_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(chunks[3]);

        let is_yes_hovered = if let Some((mx, my)) = app.mouse_pos {
            button_chunks[0].contains(ratatui::layout::Position::new(mx, my))
        } else { false };
        let is_no_hovered = if let Some((mx, my)) = app.mouse_pos {
            button_chunks[1].contains(ratatui::layout::Position::new(mx, my))
        } else { false };

        let yes_style = if is_yes_hovered {
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green).bg(Color::Rgb(20, 40, 20))
        };
        let no_style = if is_no_hovered {
            Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red).bg(Color::Rgb(40, 20, 20))
        };

        f.render_widget(Paragraph::new(" [Y] Yes, Upload ").style(yes_style).alignment(ratatui::layout::Alignment::Center), button_chunks[0]);
        f.render_widget(Paragraph::new(" [N] No, Keep Local ").style(no_style).alignment(ratatui::layout::Alignment::Center), button_chunks[1]);
    }

    if app.show_new_gist_dialog {
        let modal_area = centered_rect_fixed(50, 10, area);
        f.render_widget(Clear, modal_area);
        let block = Block::default()
            .title(Span::styled(" New Gist Script ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block.clone(), modal_area);
        let inner = block.inner(modal_area);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2), // Question
                Constraint::Length(3), // Input field
                Constraint::Length(1), // Help
                Constraint::Min(0),
            ])
            .split(inner);

        f.render_widget(Paragraph::new("Enter a name for the new Gist script:").alignment(ratatui::layout::Alignment::Center), chunks[0]);
        
        let input_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        
        let now = Instant::now().duration_since(app.start_time).as_millis();
        let cursor = if (now / 500) % 2 == 0 { "_" } else { " " };
        let input_display = format!(" {}{} ", app.new_gist_name_input, cursor);
        f.render_widget(Paragraph::new(input_display).style(Style::default().fg(Color::Cyan)).block(input_block), chunks[1]);
        
        f.render_widget(Paragraph::new("ENTER to confirm, ESC to cancel").style(Style::default().fg(Color::DarkGray)), chunks[2]);
    }

    if app.show_delete_confirm {
        let modal_area = centered_rect_fixed(50, 10, area);
        f.render_widget(Clear, modal_area);
        let block = Block::default()
            .title(Span::styled(" Confirm Deletion ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red))
            .style(Style::default().bg(Color::Black));
        f.render_widget(block.clone(), modal_area);
        let inner = block.inner(modal_area);
        
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Warning
                Constraint::Length(2), // Gist name
                Constraint::Length(1), // Spacer
                Constraint::Length(1), // Buttons
                Constraint::Min(0),
            ])
            .split(inner);

        let gist_name = app.gist_index_to_delete
            .and_then(|idx| app.gist_items.get(idx))
            .cloned()
            .unwrap_or_else(|| "Unknown Script".to_string());

        f.render_widget(Paragraph::new("Are you sure you want to delete this script?").alignment(ratatui::layout::Alignment::Center), chunks[0]);
        f.render_widget(Paragraph::new(format!("'{}'\nIt will be removed locally and from GitHub.", gist_name)).alignment(ratatui::layout::Alignment::Center), chunks[1]);

        let button_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(50),
                Constraint::Percentage(50),
            ])
            .split(chunks[3]);

        let is_yes_hovered = if let Some((mx, my)) = app.mouse_pos {
            button_chunks[0].contains(ratatui::layout::Position::new(mx, my))
        } else { false };
        let is_no_hovered = if let Some((mx, my)) = app.mouse_pos {
            button_chunks[1].contains(ratatui::layout::Position::new(mx, my))
        } else { false };

        let yes_style = if is_yes_hovered {
            Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Red).bg(Color::Rgb(40, 20, 20))
        };
        let no_style = if is_no_hovered {
            Style::default().fg(Color::Black).bg(Color::Gray).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray).bg(Color::Rgb(30, 30, 30))
        };

        f.render_widget(Paragraph::new(" [Y] Yes, Delete ").style(yes_style).alignment(ratatui::layout::Alignment::Center), button_chunks[0]);
        f.render_widget(Paragraph::new(" [N] No, Cancel ").style(no_style).alignment(ratatui::layout::Alignment::Center), button_chunks[1]);
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage((100 - percent_y) / 2), Constraint::Percentage(percent_y), Constraint::Percentage((100 - percent_y) / 2)].as_ref())
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage((100 - percent_x) / 2), Constraint::Percentage(percent_x), Constraint::Percentage((100 - percent_x) / 2)].as_ref())
        .split(popup_layout[1])[1]
}

pub fn centered_rect_fixed(width: u16, height: u16, r: Rect) -> Rect {
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
