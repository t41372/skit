//! Responsive top-level terminal geometry.

use ratatui_core::layout::{Constraint, Direction, Layout, Rect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewAreas {
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) footer: Rect,
}

pub(crate) fn split(area: Rect, footer_height: u16) -> ViewAreas {
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3.min(area.height)),
            Constraint::Min(0),
            Constraint::Length(footer_height.min(area.height.saturating_sub(3))),
        ])
        .split(area);
    ViewAreas {
        header: areas[0],
        body: areas[1],
        footer: areas[2],
    }
}
