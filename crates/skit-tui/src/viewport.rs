//! Shared viewport invariants for clipped and scrollable surfaces.

use ratatui_core::layout::Rect;
use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, MouseEvent, MouseEventKind};
use ratatui_interact::components::ScrollableContentState;

/// Inputs that decide whether a scroll owner must realign its focused item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AlignmentSignature<Focus, Reflow> {
    focus: Focus,
    viewport_width: u16,
    viewport_height: u16,
    reflow: Reflow,
}

impl<Focus: Eq, Reflow: Eq> AlignmentSignature<Focus, Reflow> {
    /// Store the current inputs and report whether focus or layout changed.
    pub(crate) fn update(
        previous: &mut Option<Self>,
        focus: Focus,
        viewport: Rect,
        reflow: Reflow,
    ) -> bool {
        let current = Self {
            focus,
            viewport_width: viewport.width,
            viewport_height: viewport.height,
            reflow,
        };
        let changed = previous.as_ref() != Some(&current);
        *previous = Some(current);
        changed
    }
}

/// One measured viewport and the virtual rows that it displays.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Viewport {
    area: Rect,
    content_height: usize,
}

/// Scroll state for virtual rows that do not have one stored string per row.
///
/// Forms and settings already own their text in typed controls. A second `Vec<String>` of empty
/// rows makes render memory grow with a multiline value even though only a terminal-sized band is
/// visible. This state stores only the row count and the reader's position.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct VirtualScrollState {
    line_count: usize,
    scroll_offset: usize,
}

impl VirtualScrollState {
    #[cfg(test)]
    pub(crate) const fn line_count(&self) -> usize {
        self.line_count
    }

    pub(crate) const fn scroll_offset(&self) -> usize {
        self.scroll_offset
    }

    pub(crate) fn set_line_count(&mut self, line_count: usize) {
        self.line_count = line_count;
        self.scroll_offset = if line_count == 0 {
            0
        } else {
            self.scroll_offset.min(line_count - 1)
        };
    }

    #[cfg(test)]
    pub(crate) fn set_lines(&mut self, lines: Vec<String>) {
        self.set_line_count(lines.len());
    }

    pub(crate) fn set_scroll_offset(&mut self, offset: usize) {
        self.scroll_offset = if self.line_count == 0 {
            0
        } else {
            offset.min(self.line_count - 1)
        };
    }

    fn scroll_up(&mut self, rows: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(rows);
    }

    fn scroll_down(&mut self, rows: usize, visible_height: usize) {
        let maximum = self.line_count.saturating_sub(visible_height);
        self.scroll_offset = self.scroll_offset.saturating_add(rows).min(maximum);
    }

    /// Apply the same reader keys as `ratatui-interact` without materializing virtual rows.
    pub(crate) fn handle_key(&mut self, key: &KeyEvent, visible_height: usize) -> bool {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1, visible_height),
            KeyCode::PageUp => self.scroll_up(visible_height.saturating_sub(1)),
            KeyCode::PageDown => {
                self.scroll_down(visible_height.saturating_sub(1), visible_height);
            }
            KeyCode::Home => self.scroll_offset = 0,
            KeyCode::End => {
                self.scroll_offset = self.line_count.saturating_sub(visible_height);
            }
            _ => return false,
        }
        true
    }

    /// Apply a wheel event only when this virtual viewport owns its terminal cell.
    pub(crate) fn handle_mouse(
        &mut self,
        mouse: &MouseEvent,
        area: Rect,
        visible_height: usize,
    ) -> bool {
        if !area.contains((mouse.column, mouse.row).into()) {
            return false;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(3),
            MouseEventKind::ScrollDown => self.scroll_down(3, visible_height),
            _ => return false,
        }
        true
    }
}

impl Viewport {
    pub(crate) const fn new(area: Rect, content_height: usize) -> Self {
        Self {
            area,
            content_height,
        }
    }

    pub(crate) const fn visible_height(self) -> usize {
        self.area.height as usize
    }

    pub(crate) const fn maximum_scroll_offset(self) -> usize {
        self.content_height.saturating_sub(self.visible_height())
    }

    /// Clamp persistent scroll state after content or viewport geometry changes.
    pub(crate) fn clamp_scroll(self, scroll: &mut ScrollableContentState) {
        scroll.set_scroll_offset(scroll.scroll_offset().min(self.maximum_scroll_offset()));
    }

    /// Split a local surface without fabricating a row in an empty viewport.
    pub(crate) const fn split_footer(area: Rect, preferred_footer_height: u16) -> [Rect; 2] {
        let footer_height = match area.height.checked_sub(preferred_footer_height) {
            Some(_) => preferred_footer_height,
            None => area.height,
        };
        let body_height = area.height.saturating_sub(footer_height);
        [
            Rect::new(area.x, area.y, area.width, body_height),
            Rect::new(
                area.x,
                area.y.saturating_add(body_height),
                area.width,
                footer_height,
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_clamp_preserves_every_offset_through_the_exact_maximum() {
        let viewport = Viewport::new(Rect::new(2, 3, 20, 4), 9);
        assert_eq!(viewport.maximum_scroll_offset(), 5);
        for offset in [0, 4, 5] {
            let mut scroll = ScrollableContentState::default();
            scroll.set_lines(vec![String::new(); 9]);
            scroll.set_scroll_offset(offset);
            viewport.clamp_scroll(&mut scroll);
            assert_eq!(scroll.scroll_offset(), offset);
        }

        let mut scroll = ScrollableContentState::default();
        scroll.set_lines(vec![String::new(); 9]);
        scroll.set_scroll_offset(8);
        viewport.clamp_scroll(&mut scroll);
        assert_eq!(scroll.scroll_offset(), 5);
        Viewport::new(Rect::new(2, 3, 20, 10), 9).clamp_scroll(&mut scroll);
        assert_eq!(scroll.scroll_offset(), 0);
    }

    #[test]
    fn footer_split_tiles_the_viewport_at_zero_exact_and_overflow_boundaries() {
        for height in 0..=4 {
            let area = Rect::new(7, 11, 20, height);
            for preferred in 0..=5 {
                let [body, footer] = Viewport::split_footer(area, preferred);
                assert_eq!(body, Rect::new(7, 11, 20, height.saturating_sub(preferred)));
                assert_eq!(footer.x, area.x);
                assert_eq!(footer.y, body.bottom());
                assert_eq!(footer.width, area.width);
                assert_eq!(footer.height, preferred.min(height));
                assert_eq!(footer.bottom(), area.bottom());
            }
        }
    }

    #[test]
    fn virtual_scroll_keeps_large_content_as_a_count_and_reaches_its_tail() {
        let mut scroll = VirtualScrollState::default();
        scroll.set_line_count(usize::from(u16::MAX) + 10_000);
        assert_eq!(scroll.line_count(), usize::from(u16::MAX) + 10_000);
        assert!(scroll.handle_key(&KeyEvent::from(KeyCode::End), 5));
        assert_eq!(scroll.scroll_offset(), scroll.line_count() - 5);

        let down = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
        };
        assert!(!scroll.handle_mouse(&down, Rect::new(6, 6, 4, 4), 5));
        assert!(scroll.handle_mouse(&down, Rect::new(4, 4, 4, 4), 5));
        assert_eq!(scroll.scroll_offset(), scroll.line_count() - 5);
    }

    #[test]
    fn virtual_scroll_owns_the_documented_keys_and_only_vertical_wheel_events() {
        let mut scroll = VirtualScrollState::default();
        scroll.set_line_count(20);
        scroll.set_scroll_offset(5);

        for (code, expected) in [
            (KeyCode::Up, 4),
            (KeyCode::Char('k'), 3),
            (KeyCode::Down, 4),
            (KeyCode::Char('j'), 5),
            (KeyCode::Home, 0),
        ] {
            assert!(scroll.handle_key(&KeyEvent::from(code), 4));
            assert_eq!(scroll.scroll_offset(), expected, "key {code:?}");
        }

        scroll.set_scroll_offset(5);
        for code in [KeyCode::F(10), KeyCode::Enter] {
            assert!(!scroll.handle_key(&KeyEvent::from(code), 4));
            assert_eq!(scroll.scroll_offset(), 5, "key {code:?}");
        }

        let moved = MouseEvent {
            kind: MouseEventKind::Moved,
            column: 5,
            row: 5,
            modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
        };
        assert!(!scroll.handle_mouse(&moved, Rect::new(4, 4, 4, 4), 4));
        assert_eq!(scroll.scroll_offset(), 5);
    }
}
