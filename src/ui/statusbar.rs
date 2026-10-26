use crate::app::App;
use crate::config::{ItemType, StatusBarItem};
use crate::utils::{display_width, parse_color};
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

/// Visible items of one status bar and the area each occupies. Shared by rendering and
/// click handling so both always agree.
pub fn bar_layout(app: &App, bar_items: &[StatusBarItem], area: Rect) -> Vec<(StatusBarItem, Rect)> {
    let visible: Vec<&StatusBarItem> = bar_items
        .iter()
        .filter(|item| app.is_item_visible(item))
        .collect();
    let constraints: Vec<Constraint> = visible
        .iter()
        .map(|item| {
            if let Some(w) = item.width {
                Constraint::Length(w)
            } else if matches!(
                item.type_,
                ItemType::Spacer
                    | ItemType::GitInfo
                    | ItemType::SelectedCommandInfo
                    | ItemType::GithubStatus
            ) {
                Constraint::Fill(1)
            } else if let Some(label) = &item.label {
                Constraint::Length(display_width(label) as u16)
            } else {
                Constraint::Min(0)
            }
        })
        .collect();
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);
    visible
        .into_iter()
        .cloned()
        .zip(chunks.iter().copied())
        .collect()
}

pub fn render_statusbars(f: &mut Frame, app: &mut App, chunks: &[Rect]) {
    let upper_bg = parse_color(&app.config.ui.upper_statusbar_bg);
    let lower_bg = parse_color(&app.config.ui.lower_statusbar_bg);

    let mut build_and_render_bar = |bar_items: &Vec<StatusBarItem>,
                                    bar_chunk: Rect,
                                    bar_bg: Color| {
        for (item, chunk) in bar_layout(app, bar_items, bar_chunk) {
            let item = &item;
            if chunk.width == 0 {
                continue;
            }

            match item.type_ {
                ItemType::Button => {
                    if let Some(label) = &item.label {
                        let is_hovered = if let Some((mx, my)) = app.mouse_pos {
                            chunk.contains(Position::new(mx, my))
                        } else {
                            false
                        };

                        let bg_color_str = if is_hovered {
                            item.hover_color
                                .as_deref()
                                .unwrap_or(item.color.as_deref().unwrap_or("white"))
                        } else {
                            item.color.as_deref().unwrap_or("white")
                        };

                        let bg_color = parse_color(bg_color_str);
                        let fg_color = if bg_color_str == "green"
                            || bg_color_str == "cyan"
                            || bg_color_str.starts_with("light_")
                        {
                            Color::Black
                        } else {
                            Color::White
                        };

                        let span = Span::styled(
                            label,
                            Style::default()
                                .bg(bg_color)
                                .fg(fg_color)
                                .add_modifier(Modifier::BOLD),
                        );
                        f.render_widget(
                            Paragraph::new(span).style(Style::default().bg(bar_bg)),
                            chunk,
                        );
                    }
                }
                ItemType::SystemStats => {
                    let cpu_color = if app.cpu_usage > 80.0 {
                        Color::Red
                    } else {
                        Color::Yellow
                    };
                    let cpu_mem_str = format!(
                        "{:.0}% {}/{}GB",
                        app.cpu_usage.round(),
                        (app.mem_usage.0 as f64 / 1_073_741_824.0).round() as u64,
                        (app.mem_usage.1 as f64 / 1_073_741_824.0).round() as u64
                    );
                    f.render_widget(
                        Paragraph::new(Span::styled(cpu_mem_str, Style::default().fg(cpu_color)))
                            .alignment(ratatui::layout::Alignment::Right)
                            .style(Style::default().bg(bar_bg)),
                        chunk,
                    );
                }
                ItemType::GitInfo => {
                    let git_str = app.git_info.as_deref().unwrap_or("");
                    f.render_widget(
                        Paragraph::new(Span::styled(
                            format!("  {}", git_str),
                            Style::default().fg(Color::Magenta),
                        ))
                        .style(Style::default().bg(bar_bg)),
                        chunk,
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
                    let stats_line = Line::from(vec![Span::styled(
                        format!(
                            "{} | {}",
                            line_info,
                            chrono::Local::now().format("%H:%M:%S")
                        ),
                        Style::default().fg(Color::White),
                    )]);
                    f.render_widget(
                        Paragraph::new(stats_line)
                            .alignment(ratatui::layout::Alignment::Right)
                            .style(Style::default().bg(bar_bg)),
                        chunk,
                    );
                }
                ItemType::SelectedCommandInfo => {
                    if let Some(entry) = app.selected_entry() {
                        let status_line = Line::from(vec![
                            Span::styled(" # ", Style::default().fg(Color::DarkGray)),
                            Span::styled(
                                format!("{} {} ", entry.label, entry.info),
                                Style::default().fg(Color::White),
                            ),
                            Span::styled(
                                entry.command.clone(),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]);
                        f.render_widget(
                            Paragraph::new(status_line).style(Style::default().bg(bar_bg)),
                            chunk,
                        );
                    }
                }
                ItemType::Spacer => {
                    f.render_widget(Paragraph::new("").style(Style::default().bg(bar_bg)), chunk);
                }
                ItemType::GithubStatus => {
                    if let Some(ref msg) = app.github_status_msg {
                        let color = if app.github_status_is_error {
                            Color::Red
                        } else {
                            Color::Cyan
                        };
                        f.render_widget(
                            Paragraph::new(Span::styled(
                                format!(" {}", msg),
                                Style::default().fg(color).add_modifier(Modifier::BOLD),
                            ))
                            .style(Style::default().bg(bar_bg)),
                            chunk,
                        );
                    } else {
                        f.render_widget(
                            Paragraph::new("").style(Style::default().bg(bar_bg)),
                            chunk,
                        );
                    }
                }
            }
        }
    };

    if chunks.len() >= 2 {
        build_and_render_bar(&app.config.statusbar.upper, chunks[0], upper_bg);
        build_and_render_bar(&app.config.statusbar.lower, chunks[1], lower_bg);
    }
}
