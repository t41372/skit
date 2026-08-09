//! Responsive top-level terminal geometry.

use ratatui_core::layout::{Constraint, Direction, Layout, Rect};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewAreas {
    pub(crate) header: Rect,
    pub(crate) body: Rect,
    pub(crate) footer: Rect,
}

/// Split the terminal with an explicit header budget.
///
/// A screen that already titles itself takes no header. Version 0.4 gives the run form the whole
/// area above the footer and writes `Run <name>` on the panel border
/// (`src/skit/tui_form.py:606-611`), so a header row there would print the same title twice and
/// cost three rows of form.
pub(crate) fn split_with_header(area: Rect, footer_height: u16, header_height: u16) -> ViewAreas {
    let header = header_height.min(area.height);
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header),
            Constraint::Min(0),
            Constraint::Length(footer_height.min(area.height.saturating_sub(header))),
        ])
        .split(area);
    ViewAreas {
        header: areas[0],
        body: areas[1],
        footer: areas[2],
    }
}
