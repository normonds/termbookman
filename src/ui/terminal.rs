use crate::app::App;
use crate::utils::parse_color;
use portable_pty::PtySize;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Scrollbar, ScrollbarOrientation, ScrollbarState};
use ratatui::Frame;

pub fn render_terminal(f: &mut Frame, app: &mut App, area: Rect, scrollbar_area: Rect) {
    {
        let mut parser = app.parser.lock().unwrap();
        let (p_rows, p_cols) = parser.screen().size();
        if area.height != p_rows || area.width != p_cols {
            let _ = app.master.resize(PtySize {
                rows: area.height,
                cols: area.width,
                pixel_width: 0,
                pixel_height: 0,
            });
            parser.screen_mut().set_size(area.height, area.width);
        }
    }

    let configured_bg = parse_color(&app.config.ui.terminal_bg);
    let term_bg = if app.is_search_focused {
        Color::Rgb(30, 30, 30)
    } else {
        configured_bg
    };
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = f.buffer_mut().get_mut(x, y);
            cell.reset();
            cell.set_bg(term_bg);
        }
    }

    {
        let parser = app.parser.lock().unwrap();
        let screen = parser.screen();

        for row in 0..area.height {
            for col in 0..area.width {
                let x = area.x + col;
                let y = area.y + row;

                if let Some(cell) = screen.cell(row, col) {
                    if cell.is_wide_continuation() {
                        continue;
                    }

                    let mut style = Style::default().bg(term_bg);
                    match cell.fgcolor() {
                        vt100::Color::Rgb(r, g, b) => {
                            style = style.fg(Color::Rgb(r, g, b));
                        }
                        vt100::Color::Idx(i) => {
                            style = style.fg(Color::Indexed(i));
                        }
                        _ => {}
                    }
                    match cell.bgcolor() {
                        vt100::Color::Rgb(r, g, b) => {
                            style = style.bg(Color::Rgb(r, g, b));
                        }
                        vt100::Color::Idx(i) => {
                            style = style.bg(Color::Indexed(i));
                        }
                        _ => {}
                    }

                    if cell.bold() {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if cell.italic() {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    if cell.inverse() {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    if cell.underline() {
                        style = style.add_modifier(Modifier::UNDERLINED);
                    }

                    if let (Some(start), Some(end)) = (app.selection_start, app.selection_end) {
                        let (s_row, s_col) = start;
                        let (e_row, e_col) = end;
                        let (min_row, min_col, max_row, max_col) =
                            if s_row < e_row || (s_row == e_row && s_col <= e_col) {
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
                    f.buffer_mut()
                        .get_mut(x, y)
                        .set_symbol(" ")
                        .set_style(Style::default());
                }
            }
        }

        let (cursor_row, cursor_col) = screen.cursor_position();
        let cx = (area.x + cursor_col).min(area.right().saturating_sub(1));
        let cy = (area.y + cursor_row).min(area.bottom().saturating_sub(1));
        f.set_cursor(cx, cy);

        // Render scrollbar
        let scrollbar_bg = if std::env::var("SUDO_USER").is_ok() {
            Color::Red
        } else {
            Color::Black
        };
        f.render_widget(
            Block::default().style(Style::default().bg(scrollbar_bg)),
            scrollbar_area,
        );
        let scroll_offset = screen.scrollback();
        let history_len = screen.scrollback_len();
        let scroll_pos = history_len.saturating_sub(scroll_offset);

        let mut scrollbar_state = ScrollbarState::new(history_len).position(scroll_pos);

        let is_term_scrollbar_hovered = if let Some((mx, my)) = app.mouse_pos {
            scrollbar_area.contains(ratatui::layout::Position::new(mx, my))
        } else {
            false
        };

        let term_scrollbar_color = if app.is_dragging_sidebar {
            Color::Yellow
        } else if is_term_scrollbar_hovered || app.is_dragging_term_scrollbar {
            Color::White
        } else {
            Color::Rgb(60, 60, 60)
        };

        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(Some("▲"))
                .end_symbol(Some("▼"))
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(Style::default().fg(Color::Rgb(30, 30, 30)))
                .thumb_style(Style::default().fg(term_scrollbar_color))
                .begin_style(Style::default().fg(term_scrollbar_color))
                .end_style(Style::default().fg(term_scrollbar_color)),
            scrollbar_area,
            &mut scrollbar_state,
        );
    }
}
