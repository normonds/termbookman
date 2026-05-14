use ratatui::Frame;
use ratatui::layout::{Layout, Constraint, Direction, Rect, Position};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use crate::app::App;
use crate::config::{StatusBarItem, ItemType};
use crate::utils::parse_color;

pub fn render_statusbars(f: &mut Frame, app: &mut App, chunks: &[Rect]) {
    let upper_bg = parse_color(&app.config.ui.upper_statusbar_bg);
    let lower_bg = parse_color(&app.config.ui.lower_statusbar_bg);

    let mut build_and_render_bar = |bar_items: &Vec<StatusBarItem>, bar_chunk: Rect, bar_bg: Color| {
        let mut constraints = Vec::new();
        let mut visible_items = Vec::new();

        for item in bar_items {
            if app.is_item_visible(item) {
                visible_items.push(item);
                let width = if let Some(w) = item.width {
                    Constraint::Length(w)
                } else if item.type_ == ItemType::Spacer || item.type_ == ItemType::GitInfo || item.type_ == ItemType::SelectedCommandInfo {
                    Constraint::Fill(1)
                } else if let Some(label) = &item.label {
                    Constraint::Length(label.len() as u16)
                } else {
                    Constraint::Min(0)
                };
                constraints.push(width);
            }
        }

        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(bar_chunk);

        for (i, item) in visible_items.iter().enumerate() {
            let chunk = chunks[i];
            if chunk.width == 0 { continue; }

            match item.type_ {
                ItemType::Button => {
                    if let Some(label) = &item.label {
                        let is_hovered = if let Some((mx, my)) = app.mouse_pos {
                            chunk.contains(Position::new(mx, my))
                        } else { false };

                        let bg_color_str = if is_hovered {
                            item.hover_color.as_deref().unwrap_or(item.color.as_deref().unwrap_or("white"))
                        } else {
                            item.color.as_deref().unwrap_or("white")
                        };
                        
                        let bg_color = parse_color(bg_color_str);
                        let fg_color = if bg_color_str == "green" || bg_color_str == "cyan" || bg_color_str.starts_with("light_") { Color::Black } else { Color::White };
                        
                        let span = Span::styled(label, Style::default().bg(bg_color).fg(fg_color).add_modifier(Modifier::BOLD));
                        f.render_widget(Paragraph::new(span).style(Style::default().bg(bar_bg)), chunk);
                    }
                }
                ItemType::SystemStats => {
                    let cpu_color = if app.cpu_usage > 80.0 { Color::Red } else { Color::Yellow };
                    let cpu_mem_str = format!("{:.0}% {}/{}GB", 
                        app.cpu_usage.round(), 
                        (app.mem_usage.0 as f64 / 1_073_741_824.0).round() as u64, 
                        (app.mem_usage.1 as f64 / 1_073_741_824.0).round() as u64
                    );
                    f.render_widget(
                        Paragraph::new(Span::styled(cpu_mem_str, Style::default().fg(cpu_color)))
                            .alignment(ratatui::layout::Alignment::Right)
                            .style(Style::default().bg(bar_bg)), 
                        chunk
                    );
                }
                ItemType::GitInfo => {
                    let git_str = app.git_info.as_deref().unwrap_or("");
                    f.render_widget(
                        Paragraph::new(Span::styled(format!("  {}", git_str), Style::default().fg(Color::Magenta)))
                            .style(Style::default().bg(bar_bg)), 
                        chunk
                    );
                }
                ItemType::TimeAndScroll => {
                    let parser = app.parser.lock().unwrap();
                    let screen = parser.screen();
                    let (rows, _) = screen.size();
                    let scroll_offset = screen.scrollback();
                    let history_len = screen.scrollback_len();
                    let total_lines = rows as usize + history_len; 
                    let line_info = format!("{}/{}", scroll_offset, total_lines);
                    let stats_line = Line::from(vec![
                        Span::styled(format!("{} | {}", line_info, chrono::Local::now().format("%H:%M:%S")), Style::default().fg(Color::White)),
                    ]);
                    f.render_widget(
                        Paragraph::new(stats_line)
                            .alignment(ratatui::layout::Alignment::Right)
                            .style(Style::default().bg(bar_bg)), 
                        chunk
                    );
                }
                ItemType::SelectedCommandInfo => {
                    if let Some(idx) = app.sidebar_state.selected() {
                        let (items, infos, commands) = match app.sidebar_mode {
                            crate::app::SidebarMode::Commands => (&app.sidebar_items, Some(&app.sidebar_infos), &app.sidebar_commands),
                            crate::app::SidebarMode::History => (&app.history_items, None, &app.history_commands),
                            crate::app::SidebarMode::Gists => (&app.gist_items, Some(&app.gist_infos), &app.gist_commands),
                        };

                        let filtered: Vec<usize> = items.iter().enumerate()
                            .filter(|(idx, label)| {
                                let matches_label = label.to_lowercase().contains(&app.search_query.to_lowercase());
                                let matches_info = if let Some(infos) = infos {
                                    infos[*idx].to_lowercase().contains(&app.search_query.to_lowercase())
                                } else {
                                    false
                                };
                                matches_label || matches_info
                            })
                            .map(|(idx, _)| idx)
                            .collect();

                        if idx < filtered.len() {
                            let original_index = filtered[idx];
                            let label = &items[original_index];
                            let info = if let Some(infos) = infos { &infos[original_index] } else { "" };
                            let cmd = &commands[original_index];
                            let status_line = Line::from(vec![
                                Span::styled(" # ", Style::default().fg(Color::DarkGray)),
                                Span::styled(format!("{} {} ", label, info), Style::default().fg(Color::White)),
                                Span::styled(cmd.to_string(), Style::default().fg(Color::DarkGray)),
                            ]);
                            f.render_widget(
                                Paragraph::new(status_line).style(Style::default().bg(bar_bg)),
                                chunk
                            );
                        }
                    }
                }
                ItemType::Spacer => {
                    f.render_widget(Paragraph::new("").style(Style::default().bg(bar_bg)), chunk);
                }
            }
        }
    };

    if chunks.len() >= 2 {
        build_and_render_bar(&app.config.statusbar.upper, chunks[0], upper_bg);
        build_and_render_bar(&app.config.statusbar.lower, chunks[1], lower_bg);
    }
}
