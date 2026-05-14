use ratatui::Frame;
use ratatui::layout::{Layout, Constraint, Direction, Rect, Position};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use std::time::Instant;
use crate::app::{App, SidebarMode};
use crate::utils::parse_color;

pub fn render_sidebar(f: &mut Frame, app: &mut App, area: Rect, scrollbar_area: Rect) {
    let sidebar_bg = parse_color(&app.config.ui.sidebar_bg);
    let parser = app.parser.lock().unwrap();
    let screen = parser.screen();
    let is_app_active = screen.alternate_screen() || screen.hide_cursor() || screen.application_cursor();
    let is_sidebar_locked = is_app_active || app.is_pty_busy;
    drop(parser); // Release lock before further rendering if needed, though usually Frame uses it. Actually keep it if we need more info.

    let sidebar_layout = if app.sidebar_mode == SidebarMode::Gists && !is_sidebar_locked {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Tabs
                Constraint::Length(1), // Search Bar
                Constraint::Min(0),    // List
                Constraint::Length(1), // New Script Button
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Tabs
                Constraint::Length(1), // Search Bar
                Constraint::Min(0)     // List
            ])
            .split(area)
    };

    let tabs_area = sidebar_layout[0];
    let (new_button_area, search_area, list_area) = if app.sidebar_mode == SidebarMode::Gists && !is_sidebar_locked {
        (Some(sidebar_layout[3]), sidebar_layout[1], sidebar_layout[2])
    } else {
        (None, sidebar_layout[1], sidebar_layout[2])
    };

    if let Some(area) = new_button_area {
        let is_hovered = if let Some((mx, my)) = app.mouse_pos {
            area.contains(Position::new(mx, my))
        } else { false };

        let style = if is_hovered {
            Style::default().fg(Color::Black).bg(Color::Green).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Green).bg(sidebar_bg)
        };

        f.render_widget(
            Paragraph::new(Span::styled(" [+] NEW SCRIPT ", style))
                .alignment(ratatui::layout::Alignment::Center),
            area
        );
    }
    
    if is_sidebar_locked {
         f.render_widget(
            Paragraph::new(Span::styled("  SIDEBAR DISABLED  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)))
                .style(Style::default().bg(sidebar_bg))
                .alignment(ratatui::layout::Alignment::Center),
            tabs_area
        );
    } else {
        let tabs_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(1, 3), Constraint::Ratio(1, 3)])
            .split(tabs_area);
        
        let modes = [
            (SidebarMode::Commands, " COMMANDS ", Color::Green),
            (SidebarMode::History, " HISTORY ", Color::Rgb(255, 180, 100)),
            (SidebarMode::Gists, " GISTS ", Color::Yellow),
        ];

        for (i, (mode, label, color)) in modes.iter().enumerate() {
            let is_active = app.sidebar_mode == *mode;
            let is_hovered = if let Some((mx, my)) = app.mouse_pos {
                tabs_chunks[i].contains(Position::new(mx, my))
            } else { false };

            let style = if is_active {
                Style::default().fg(sidebar_bg).bg(*color).add_modifier(Modifier::BOLD)
            } else if is_hovered {
                Style::default().fg(*color).bg(Color::Rgb(50, 50, 50))
            } else {
                Style::default().fg(*color).bg(sidebar_bg)
            };

            f.render_widget(
                Paragraph::new(Span::styled(*label, style))
                    .alignment(ratatui::layout::Alignment::Center),
                tabs_chunks[i]
            );
        }
    }
    
    let search_style = if is_sidebar_locked {
        Style::default().fg(Color::DarkGray).bg(sidebar_bg)
    } else if app.is_search_focused {
        Style::default().fg(Color::Yellow).bg(Color::Rgb(30, 30, 30))
    } else {
        Style::default().fg(Color::DarkGray).bg(sidebar_bg)
    };

    let now = Instant::now().duration_since(app.start_time).as_millis();
    let cursor_char = if app.is_search_focused && (now / 500) % 2 == 0 { "_" } else { " " };
    
    let (items, infos, commands) = match app.sidebar_mode {
        SidebarMode::Commands => (&app.sidebar_items, Some(&app.sidebar_infos), &app.sidebar_commands),
        SidebarMode::History => (&app.history_items, None, &app.history_commands),
        SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
    };

    let search_text = if app.search_query.is_empty() && !app.is_search_focused {
        format!(" Search {} items .. ", items.len())
    } else {
        format!(" {}{} ", app.search_query, cursor_char)
    };
    f.render_widget(Paragraph::new(search_text).style(search_style), search_area);

    let filtered_items: Vec<(usize, &String)> = items.iter().enumerate()
        .filter(|(idx, label)| {
            let matches_label = label.to_lowercase().contains(&app.search_query.to_lowercase());
            let matches_info = if let Some(infos) = infos {
                infos[*idx].to_lowercase().contains(&app.search_query.to_lowercase())
            } else {
                false
            };
            matches_label || matches_info
        })
        .collect();

    let sidebar_list_items: Vec<ListItem> = filtered_items.iter().enumerate().map(|(idx, (i, item))| {
        let color = match app.sidebar_mode {
            SidebarMode::History => Color::Rgb(200, 200, 200),
            SidebarMode::Gists => Color::Yellow,
            SidebarMode::Commands => {
                if commands[*i].contains("<prompt") {
                    Color::Green
                } else if commands[*i].contains("sudo") || (infos.is_some() && infos.unwrap()[*i].to_lowercase().contains("sudo")) {
                    Color::Red
                } else {
                    Color::White
                }
            }
        };

        let item_bg = if is_sidebar_locked {
            sidebar_bg
        } else if app.sidebar_state.selected() == Some(idx) {
            Color::Rgb(60, 60, 60)
        } else {
            sidebar_bg
        };

        let offset = app.sidebar_state.offset();
        let is_row_hovered = if let Some((mx, my)) = app.mouse_pos {
            list_area.contains(Position::new(mx, my)) && 
            my == list_area.y + (idx as u16).saturating_sub(offset as u16) && 
            idx >= offset && idx < offset + list_area.height as usize
        } else {
            false
        };

        let is_button_hovered = is_row_hovered && if let Some((mx, _)) = app.mouse_pos {
            mx == list_area.x + 1
        } else { false };

        let is_delete_hovered = is_row_hovered && if let Some((mx, _)) = app.mouse_pos {
            mx >= list_area.x + list_area.width.saturating_sub(4) && mx < list_area.x + list_area.width.saturating_sub(1)
        } else { false };

        let style = if is_sidebar_locked {
            Style::default().fg(Color::DarkGray).bg(item_bg)
        } else if app.sidebar_state.selected() == Some(idx) || is_row_hovered {
            Style::default().fg(color).bg(item_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(color).bg(item_bg)
        };
        
        let indicator_style = if is_sidebar_locked {
            Style::default().fg(Color::Rgb(25, 25, 25)).bg(item_bg)
        } else if is_button_hovered {
            Style::default().fg(sidebar_bg).bg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg)
        };

        let delete_style = if is_sidebar_locked {
            Style::default().fg(Color::Rgb(25, 25, 25)).bg(item_bg)
        } else if is_delete_hovered {
            Style::default().fg(Color::White).bg(Color::Red).add_modifier(Modifier::BOLD)
        } else if is_row_hovered {
            Style::default().fg(Color::Rgb(100, 50, 50)).bg(item_bg)
        } else {
            Style::default().fg(sidebar_bg).bg(item_bg)
        };

        let symbol = "▸";
        
        let (info, is_fallback) = if let Some(infos) = infos {
            let info_text = &infos[*i];
            if info_text.trim().is_empty() || info_text.len() < 3 {
                (format!(" {}", commands[*i]), true)
            } else {
                (format!(" {}", info_text), false)
            }
        } else {
            (String::new(), false)
        };

        let label_text = if app.sidebar_mode == SidebarMode::Gists && app.loading_gist && idx == 0 {
            let now = Instant::now().duration_since(app.start_time).as_millis();
            let spinner = ["|", "/", "-", "\\"][(now / 150 % 4) as usize];
            format!("{} LOADING ...", spinner)
        } else {
            item.to_string()
        };

        // Detect script items via __SCRIPT__ tag in info field
        let is_script = if let Some(infos) = infos {
            infos[*i].starts_with("__SCRIPT__")
        } else {
            false
        };

        // Strip __SCRIPT__ prefix from label if present (gists tab)
        let label_text = if label_text.starts_with("__SCRIPT__") {
            label_text[10..].to_string()
        } else {
            label_text
        };

        // Strip __SCRIPT__ from info display text
        let info = if info.trim_start().starts_with("__SCRIPT__") {
            let s = info.replacen("__SCRIPT__", "", 1);
            if s.starts_with("  ") {
                s[1..].to_string()
            } else {
                s
            }
        } else {
            info
        };

        let mut command_text = String::new();
        let mut hide_info = false;
        if label_text.len() < 20 && app.sidebar_mode != SidebarMode::History {
            command_text = format!(" {}", commands[*i]);
            if is_fallback {
                hide_info = true;
            }
        }

        let info_style = if is_fallback {
            Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg)
        } else if is_row_hovered || app.sidebar_state.selected() == Some(idx) {
            Style::default().fg(Color::Rgb(90, 90, 90)).bg(item_bg).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Rgb(90, 90, 90)).bg(item_bg)
        };

        let command_style = Style::default().fg(Color::Rgb(50, 50, 50)).bg(item_bg);
        let script_badge_style = Style::default().fg(Color::Rgb(80, 80, 80)).bg(item_bg);

        let mut spans = vec![
            Span::styled(" ", Style::default().bg(item_bg)),
            Span::styled(symbol, indicator_style),
        ];

        let mut avail = (list_area.width as usize).saturating_sub(2);
        if app.sidebar_mode == SidebarMode::Gists {
            avail = avail.saturating_sub(3);
        }

        // Add label
        let label_to_draw = if label_text.len() > avail {
            format!("{}..", &label_text[..avail.saturating_sub(2)])
        } else {
            label_text.clone()
        };
        avail = avail.saturating_sub(label_to_draw.len());
        spans.push(Span::styled(label_to_draw, style));

        // Add SCRIPT badge
        if is_script && avail >= 7 {
            spans.push(Span::styled(" SCRIPT", script_badge_style));
            avail = avail.saturating_sub(7);
        }

        // Add info
        if !hide_info && !info.is_empty() && avail > 3 {
            let info_to_draw = if info.len() > avail {
                format!("{}..", &info[..avail.saturating_sub(2)])
            } else {
                info.clone()
            };
            avail = avail.saturating_sub(info_to_draw.len());
            spans.push(Span::styled(info_to_draw, info_style));
        }

        // Add command preview
        if !command_text.is_empty() && avail > 3 {
            let cmd_to_draw = if command_text.len() > avail {
                format!("{}..", &command_text[..avail.saturating_sub(2)])
            } else {
                command_text.clone()
            };
            avail = avail.saturating_sub(cmd_to_draw.len());
            spans.push(Span::styled(cmd_to_draw, command_style));
        }

        if app.sidebar_mode == SidebarMode::Gists && is_script {
            if avail > 0 {
                spans.push(Span::styled(" ".repeat(avail), Style::default().bg(item_bg)));
            }
            spans.push(Span::styled(" x ", delete_style));
        }

        let line = Line::from(spans);
        ListItem::new(line).style(Style::default().bg(item_bg))
    }).collect();

    let sidebar_list = List::new(sidebar_list_items)
        .style(Style::default().bg(sidebar_bg))
        .highlight_style(if is_app_active { 
            Style::default()
        } else { 
            Style::default().add_modifier(Modifier::BOLD)
        })
        .highlight_symbol("");
    f.render_stateful_widget(sidebar_list, list_area, &mut app.sidebar_state);

    let sidebar_scroll_pos = app.sidebar_state.selected().unwrap_or(0);
    let mut sidebar_scrollbar_state = ScrollbarState::new(filtered_items.len())
        .position(sidebar_scroll_pos);

    let is_sidebar_scrollbar_hovered = if let Some((mx, my)) = app.mouse_pos {
        scrollbar_area.contains(Position::new(mx, my))
    } else { false };

    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(Some(" "))
            .thumb_symbol("┃")
            .track_style(Style::default().bg(sidebar_bg).fg(Color::Rgb(20, 20, 20)))
            .thumb_style(if is_sidebar_scrollbar_hovered || app.is_dragging_sidebar_scrollbar {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::Rgb(50, 50, 50))
            }),
        scrollbar_area,
        &mut sidebar_scrollbar_state,
    );
}
