//! Shared terminal pointer geometry.

use ratatui_core::layout::Rect;
use ratatui_crossterm::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui_textarea::{CursorMove, TextArea as RichTextArea};
use tui_input::{Input, InputRequest};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// A typed hit map. The last rendered region is the topmost region.
#[derive(Debug)]
pub(crate) struct HitMap<T> {
    regions: Vec<(Rect, T)>,
}

/// Result of one pointer event sent through a press-release tracker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClickOutcome<T> {
    /// A primary press armed a semantic target.
    Armed,
    /// A primary release matched the semantic target that was pressed.
    Activated(T),
    /// The event did not complete a click.
    Ignored,
}

/// Whether one pointer event belonged to this tracker or can continue to a lower owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ClickDispatch<T> {
    /// The tracker or one of its rendered targets owned the event.
    Captured(ClickOutcome<T>),
    /// No target and no active press owned the event.
    Unowned,
}

/// Keep one semantic target stable across a primary press and release.
#[derive(Debug)]
pub(crate) struct ClickTracker<T> {
    pressed: Option<T>,
}

impl<T> Default for ClickTracker<T> {
    fn default() -> Self {
        Self { pressed: None }
    }
}

impl<T: Clone + Eq> ClickTracker<T> {
    pub(crate) fn cancel(&mut self) {
        self.pressed = None;
    }

    /// Update the lifecycle and report whether a lower pointer owner can see this event.
    pub(crate) fn dispatch(&mut self, mouse: &MouseEvent, target: Option<&T>) -> ClickDispatch<T> {
        let had_press = self.pressed.is_some();
        let captured = target.is_some()
            || (had_press && !matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)));
        let outcome = self.update(mouse, target);
        if captured {
            ClickDispatch::Captured(outcome)
        } else {
            ClickDispatch::Unowned
        }
    }

    pub(crate) fn update(&mut self, mouse: &MouseEvent, target: Option<&T>) -> ClickOutcome<T> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.pressed = target.cloned();
                if self.pressed.is_some() {
                    ClickOutcome::Armed
                } else {
                    ClickOutcome::Ignored
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let pressed = self.pressed.take();
                match (pressed, target) {
                    (Some(pressed), Some(released)) if &pressed == released => {
                        ClickOutcome::Activated(pressed)
                    }
                    _ => ClickOutcome::Ignored,
                }
            }
            MouseEventKind::Down(MouseButton::Right | MouseButton::Middle)
            | MouseEventKind::Up(MouseButton::Right | MouseButton::Middle)
            | MouseEventKind::Drag(_) => {
                self.pressed = None;
                ClickOutcome::Ignored
            }
            _ => ClickOutcome::Ignored,
        }
    }
}

impl<T> Default for HitMap<T> {
    fn default() -> Self {
        Self {
            regions: Vec::new(),
        }
    }
}

impl<T> HitMap<T> {
    pub(crate) fn clear(&mut self) {
        self.regions.clear();
    }

    pub(crate) fn register(&mut self, rect: Rect, target: T) {
        self.regions.push((rect, target));
    }

    pub(crate) fn topmost(&self, column: u16, row: u16) -> Option<&T> {
        self.regions
            .iter()
            .rev()
            .find(|(rect, _)| contains(*rect, column, row))
            .map(|(_, target)| target)
    }
}

/// Exact rendered cells that can place a cursor in one single-line input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EditableGeometry {
    content: Rect,
    visual_scroll: usize,
    secret: bool,
}

impl EditableGeometry {
    pub(crate) const fn new(content: Rect, visual_scroll: usize, secret: bool) -> Self {
        Self {
            content,
            visual_scroll,
            secret,
        }
    }

    /// Move the input cursor to the nearest grapheme boundary at the clicked cell.
    pub(crate) fn place_cursor(&self, input: &mut Input, column: u16, row: u16) -> bool {
        if !contains(self.content, column, row) {
            return false;
        }
        let visual_cell = self
            .visual_scroll
            .saturating_add(usize::from(column.saturating_sub(self.content.x)));
        let cursor = cursor_for_visual_cell(input.value(), visual_cell, self.secret);
        let _ = input.handle(InputRequest::SetCursor(cursor));
        true
    }
}

/// Persistent viewport coordinates for one multiline editor.
///
/// `ratatui-textarea` keeps these coordinates private. Keep the same alignment rule here so a
/// terminal cell can map back to the logical line and character that the editor rendered.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TextAreaViewport {
    top_row: usize,
    left_cell: usize,
}

/// Exact rendered cells that can place a cursor in one multiline editor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TextAreaGeometry {
    content: Rect,
    first_row: usize,
    left_cell: usize,
    tab_length: usize,
}

impl TextAreaViewport {
    pub(crate) fn align(&mut self, state: &RichTextArea<'_>, width: usize, height: usize) {
        let cursor = state.cursor();
        let (row, column) = (cursor.0, cursor.1);
        let visual_column =
            textarea_display_column(&state.lines()[row], column, usize::from(state.tab_length()));
        self.top_row = aligned_scroll(self.top_row, row, height);
        self.left_cell = textarea_scroll_boundary(
            &state.lines()[row],
            aligned_scroll(self.left_cell, visual_column, width),
            usize::from(state.tab_length()),
        );
    }

    pub(crate) const fn top_row(self) -> usize {
        self.top_row
    }

    pub(crate) const fn left_cell(self) -> usize {
        self.left_cell
    }

    pub(crate) const fn geometry(
        self,
        content: Rect,
        skipped_rows: usize,
        tab_length: usize,
    ) -> TextAreaGeometry {
        TextAreaGeometry {
            content,
            first_row: self.top_row.saturating_add(skipped_rows),
            left_cell: self.left_cell,
            tab_length,
        }
    }
}

impl TextAreaGeometry {
    /// Move the editor cursor to the nearest character boundary at the clicked cell.
    pub(crate) fn place_cursor(self, state: &mut RichTextArea<'_>, column: u16, row: u16) -> bool {
        if !contains(self.content, column, row) {
            return false;
        }
        let logical_row = self
            .first_row
            .saturating_add(usize::from(row.saturating_sub(self.content.y)))
            .min(state.lines().len().saturating_sub(1));
        let visual_cell = self
            .left_cell
            .saturating_add(usize::from(column.saturating_sub(self.content.x)));
        let logical_column = cursor_for_textarea_visual_cell(
            &state.lines()[logical_row],
            visual_cell,
            self.tab_length,
        );
        let first_row = logical_row.min(usize::from(u16::MAX));
        state.move_cursor(CursorMove::Jump(
            u16::try_from(first_row).expect("the clamped row fits"),
            0,
        ));
        for _ in first_row..logical_row {
            state.move_cursor(CursorMove::Down);
        }
        for _ in 0..logical_column {
            state.move_cursor(CursorMove::Forward);
        }
        true
    }
}

pub(crate) fn textarea_display_column(
    line: &str,
    logical_column: usize,
    tab_length: usize,
) -> usize {
    let mut display_column = 0_usize;
    let mut codepoints = 0_usize;
    for grapheme in line.graphemes(true) {
        if codepoints >= logical_column {
            break;
        }
        let next_codepoints = codepoints.saturating_add(grapheme.chars().count());
        if logical_column < next_codepoints {
            break;
        }
        codepoints = next_codepoints;
        display_column = if grapheme == "\t" && tab_length > 0 {
            display_column.saturating_add(tab_length - (display_column % tab_length))
        } else {
            display_column.saturating_add(grapheme.width())
        };
    }
    display_column
}

fn textarea_scroll_boundary(line: &str, wanted: usize, tab_length: usize) -> usize {
    let mut boundary = 0_usize;
    for grapheme in line.graphemes(true) {
        let width = if grapheme == "\t" && tab_length > 0 {
            tab_length - (boundary % tab_length)
        } else {
            grapheme.width()
        };
        let next = boundary.saturating_add(width);
        if next > wanted {
            break;
        }
        boundary = next;
    }
    boundary
}

fn cursor_for_textarea_visual_cell(value: &str, visual_cell: usize, tab_length: usize) -> usize {
    let mut cells = 0_usize;
    let mut codepoints = 0_usize;
    for grapheme in value.graphemes(true) {
        let width = if grapheme == "\t" && tab_length > 0 {
            tab_length - (cells % tab_length)
        } else {
            grapheme.width()
        };
        let grapheme_codepoints = grapheme.chars().count();
        let next = cells.saturating_add(width);
        if grapheme == "\t" {
            if visual_cell < next {
                return codepoints;
            }
        } else if width > 0 && visual_cell <= next {
            return if visual_cell.saturating_sub(cells) * 2 >= width {
                codepoints.saturating_add(grapheme_codepoints)
            } else {
                codepoints
            };
        }
        cells = next;
        codepoints = codepoints.saturating_add(grapheme_codepoints);
    }
    codepoints
}

fn aligned_scroll(previous: usize, cursor: usize, length: usize) -> usize {
    if length == 0 {
        return cursor;
    }
    if previous.saturating_add(length) <= cursor {
        cursor.saturating_add(1).saturating_sub(length)
    } else {
        previous.min(cursor)
    }
}

pub(crate) const fn is_primary_down(mouse: &MouseEvent) -> bool {
    matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
}

pub(crate) const fn contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x && column < rect.right() && row >= rect.y && row < rect.bottom()
}

pub(crate) fn display_cursor(value: &str, cursor: usize, secret: bool) -> usize {
    let prefix: String = value.chars().take(cursor).collect();
    if secret {
        prefix.graphemes(true).count()
    } else {
        prefix.width()
    }
}

pub(crate) fn display_scroll(value: &str, cursor: usize, width: usize, secret: bool) -> usize {
    let wanted = display_cursor(value, cursor, secret).saturating_sub(width);
    let mut scroll = 0_usize;
    for grapheme in value.graphemes(true) {
        if scroll >= wanted {
            break;
        }
        scroll = scroll.saturating_add(if secret { 1 } else { grapheme.width() });
    }
    scroll
}

pub(crate) fn secret_display(value: &str) -> String {
    "•".repeat(value.graphemes(true).count())
}

fn cursor_for_visual_cell(value: &str, visual_cell: usize, secret: bool) -> usize {
    let mut cells = 0_usize;
    let mut codepoints = 0_usize;
    for grapheme in value.graphemes(true) {
        let width = if secret { 1 } else { grapheme.width() };
        let grapheme_codepoints = grapheme.chars().count();
        if width == 0 {
            codepoints = codepoints.saturating_add(grapheme_codepoints);
            continue;
        }
        let next = cells.saturating_add(width);
        if visual_cell <= next {
            return if visual_cell.saturating_sub(cells) * 2 >= width {
                codepoints.saturating_add(grapheme_codepoints)
            } else {
                codepoints
            };
        }
        cells = next;
        codepoints = codepoints.saturating_add(grapheme_codepoints);
    }
    codepoints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_scroll_matches_tui_input_without_splitting_a_grapheme() {
        for (value, width, secret) in [
            ("abcdef", 3, false),
            ("a界b", 2, false),
            ("界a", 2, false),
            ("e\u{301}xyz", 2, false),
            ("👨‍👩‍👧‍👦xy", 2, false),
            ("界a", 1, true),
        ] {
            let input = Input::new(value.to_owned());
            let expected = if secret {
                secret_display(value).width().saturating_sub(width)
            } else {
                input.visual_scroll(width)
            };
            assert_eq!(
                display_scroll(value, input.cursor(), width, secret),
                expected,
                "value={value:?} width={width} secret={secret}"
            );
        }
    }

    #[test]
    fn visual_cell_matrix_returns_only_complete_grapheme_boundaries() {
        for (value, secret, expected) in [
            ("abc", false, vec![0, 1, 2, 3]),
            ("a界b", false, vec![0, 1, 2, 2, 3]),
            ("e\u{301}x", false, vec![0, 2, 3]),
            ("👨‍👩‍👧‍👦x", false, vec![0, 7, 7, 8]),
            ("a界b", true, vec![0, 1, 2, 3]),
        ] {
            let boundaries = value
                .grapheme_indices(true)
                .map(|(byte, _)| value[..byte].chars().count())
                .chain(std::iter::once(value.chars().count()))
                .collect::<Vec<_>>();
            for (cell, expected_cursor) in expected.into_iter().enumerate() {
                let cursor = cursor_for_visual_cell(value, cell, secret);
                assert_eq!(cursor, expected_cursor, "value={value:?} cell={cell}");
                assert!(
                    boundaries.contains(&cursor),
                    "value={value:?} cell={cell} split at codepoint {cursor}"
                );
            }
        }
    }

    #[test]
    fn cursor_mapping_never_splits_a_wide_or_combining_grapheme() {
        assert_eq!(cursor_for_visual_cell("ab", 1, false), 1);
        assert_eq!(cursor_for_visual_cell("a界b", 1, false), 1);
        assert_eq!(cursor_for_visual_cell("a界b", 2, false), 2);
        assert_eq!(cursor_for_visual_cell("a界b", 3, false), 2);
        assert_eq!(cursor_for_visual_cell("e\u{301}x", 0, false), 0);
        assert_eq!(cursor_for_visual_cell("e\u{301}x", 1, false), 2);
        assert_eq!(cursor_for_visual_cell("\u{301}x", 0, false), 1);
    }

    #[test]
    fn textarea_alignment_follows_the_cursor_across_both_viewport_edges() {
        assert_eq!(aligned_scroll(4, 2, 3), 2);
        assert_eq!(aligned_scroll(2, 2, 3), 2);
        assert_eq!(aligned_scroll(2, 5, 3), 3);
        assert_eq!(aligned_scroll(1, 4, 3), 2);
        assert_eq!(aligned_scroll(1, 2, 3), 1);
        assert_eq!(aligned_scroll(4, 9, 0), 9);
    }

    #[test]
    fn topmost_hit_wins() {
        let mut hits = HitMap::default();
        hits.register(Rect::new(0, 0, 2, 1), 1);
        hits.register(Rect::new(1, 0, 2, 1), 2);
        assert_eq!(hits.topmost(1, 0), Some(&2));
    }

    #[test]
    fn hit_map_drops_the_previous_frame() {
        let mut hits = HitMap::default();
        hits.register(Rect::new(1, 1, 2, 2), 1);
        hits.clear();

        assert_eq!(hits.topmost(1, 1), None);
    }

    #[test]
    fn hit_testing_excludes_the_right_and_bottom_edges() {
        let rect = Rect::new(2, 3, 4, 5);
        assert!(contains(rect, 5, 7));
        assert!(!contains(rect, rect.right(), 7));
        assert!(!contains(rect, 5, rect.bottom()));
    }

    #[test]
    fn textarea_viewport_tracks_a_cursor_beyond_both_visible_edges() {
        let mut state = RichTextArea::new(vec![
            "zero".to_owned(),
            "one".to_owned(),
            "two".to_owned(),
            "0123456789".to_owned(),
        ]);
        state.move_cursor(CursorMove::Jump(3, 8));
        let mut viewport = TextAreaViewport::default();

        viewport.align(&state, 4, 2);

        assert_eq!(viewport.geometry(Rect::new(4, 5, 4, 2), 1, 4).first_row, 3);
        assert_eq!(viewport.left_cell, 5);
    }

    #[test]
    fn click_tracker_cancels_drag_nonprimary_and_semantic_mismatch() {
        let target = 1;
        let other = 2;
        let event = |kind| MouseEvent {
            kind,
            column: 1,
            row: 1,
            modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
        };
        let mut tracker = ClickTracker::default();

        assert_eq!(
            tracker.update(
                &event(MouseEventKind::Down(MouseButton::Left)),
                Some(&target)
            ),
            ClickOutcome::Armed
        );
        assert_eq!(
            tracker.update(
                &event(MouseEventKind::Drag(MouseButton::Left)),
                Some(&target)
            ),
            ClickOutcome::Ignored
        );
        assert_eq!(
            tracker.update(&event(MouseEventKind::Up(MouseButton::Left)), Some(&target)),
            ClickOutcome::Ignored,
            "a drag must cancel the armed click"
        );

        for button in [MouseButton::Right, MouseButton::Middle] {
            assert_eq!(
                tracker.update(
                    &event(MouseEventKind::Down(MouseButton::Left)),
                    Some(&target)
                ),
                ClickOutcome::Armed
            );
            assert_eq!(
                tracker.update(&event(MouseEventKind::Up(button)), Some(&target)),
                ClickOutcome::Ignored
            );
            assert_eq!(
                tracker.update(&event(MouseEventKind::Up(MouseButton::Left)), Some(&target)),
                ClickOutcome::Ignored,
                "{button:?} Up retained the primary arm"
            );
        }

        assert_eq!(
            tracker.update(
                &event(MouseEventKind::Down(MouseButton::Left)),
                Some(&target)
            ),
            ClickOutcome::Armed
        );
        assert_eq!(
            tracker.update(
                &event(MouseEventKind::Down(MouseButton::Middle)),
                Some(&target)
            ),
            ClickOutcome::Ignored
        );
        assert_eq!(
            tracker.update(&event(MouseEventKind::Up(MouseButton::Left)), Some(&target)),
            ClickOutcome::Ignored
        );

        assert_eq!(
            tracker.update(
                &event(MouseEventKind::Down(MouseButton::Left)),
                Some(&target)
            ),
            ClickOutcome::Armed
        );
        assert_eq!(
            tracker.update(&event(MouseEventKind::Up(MouseButton::Left)), Some(&other)),
            ClickOutcome::Ignored
        );
        assert_eq!(
            tracker.update(&event(MouseEventKind::Up(MouseButton::Left)), Some(&target)),
            ClickOutcome::Ignored,
            "a semantic mismatch retained the primary arm"
        );
    }

    #[test]
    fn click_dispatch_captures_an_outside_release_but_not_an_unowned_press() {
        let target = 1;
        let event = |kind| MouseEvent {
            kind,
            column: 1,
            row: 1,
            modifiers: ratatui_crossterm::crossterm::event::KeyModifiers::NONE,
        };
        let mut tracker = ClickTracker::default();

        assert_eq!(
            tracker.dispatch(&event(MouseEventKind::Down(MouseButton::Left)), None),
            ClickDispatch::Unowned,
            "a blank primary press must remain available to a lower owner"
        );
        assert_eq!(
            tracker.dispatch(
                &event(MouseEventKind::Down(MouseButton::Left)),
                Some(&target)
            ),
            ClickDispatch::Captured(ClickOutcome::Armed)
        );
        assert_eq!(
            tracker.dispatch(&event(MouseEventKind::Up(MouseButton::Left)), None),
            ClickDispatch::Captured(ClickOutcome::Ignored),
            "the tracker must consume the release that cancels its active press"
        );
        assert_eq!(
            tracker.dispatch(&event(MouseEventKind::Up(MouseButton::Left)), Some(&target)),
            ClickDispatch::Captured(ClickOutcome::Ignored),
            "a late release resurrected a target cancelled outside"
        );
    }
}
