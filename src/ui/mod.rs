use ratatui::Frame;
use ratatui::layout::{Layout, Constraint, Direction, Rect};
use crate::app::App;

pub mod terminal;
pub mod sidebar;
pub mod statusbar;
pub mod modals;

pub struct LayoutResults {
    pub term_area: Rect,
    pub scrollbar_area: Rect,
    pub sidebar_area: Rect,
    pub sidebar_scrollbar_area: Rect,
    pub status_chunks: Vec<Rect>,
}

pub fn calculate_layout(size: Rect, sidebar_width: u16) -> LayoutResults {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(2)])
        .split(size);

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(sidebar_width)])
        .split(chunks[0]);

    let term_area = top_chunks[0];
    let right_pane = top_chunks[1];
    
    let right_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(1), // Terminal Scrollbar / Resize Handle
            Constraint::Length(0), 
            Constraint::Fill(1),   // Sidebar Items
            Constraint::Length(1), // Sidebar Scrollbar
        ])
        .split(right_pane);

    let status_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(chunks[1]);

    LayoutResults {
        term_area,
        scrollbar_area: right_layout[0],
        sidebar_area: right_layout[2],
        sidebar_scrollbar_area: right_layout[3],
        status_chunks: status_chunks.to_vec(),
    }
}

pub fn render(f: &mut Frame, app: &mut App) {
    let layout = calculate_layout(f.size(), app.sidebar_width);
    
    terminal::render_terminal(f, app, layout.term_area, layout.scrollbar_area);
    sidebar::render_sidebar(f, app, layout.sidebar_area, layout.sidebar_scrollbar_area);
    statusbar::render_statusbars(f, app, &layout.status_chunks);
    modals::render_modals(f, app, f.size());
}
