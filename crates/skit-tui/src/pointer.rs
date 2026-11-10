//! Shared terminal pointer geometry.

use ratatui_core::layout::Rect;
use ratatui_crossterm::crossterm::event::{
    Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

use crate::editor::{TextEditor, TextPosition, display_column, grapheme_cells};

/// The modifier that marks the second press of a double click.
///
/// A terminal reports no click count. The session counts the presses itself and marks the second
/// press with a modifier that no terminal reports for a pointer event, so every screen can give the
/// press to its box unchanged.
pub(crate) const DOUBLE_PRESS: KeyModifiers = KeyModifiers::HYPER;

/// Two primary presses on one cell of a text box, with only the release between them, make a
/// double click.
///
/// Only a text box uses a double click, so a press anywhere else leaves no state: a click on a
/// button ends where the same command by key ends. The terminal loop also ends a sequence when the
/// second press comes too late. A walker or a test that drives the session directly gets the same
/// answer for the same events every time.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PressSequence {
    /// The cell of the last primary press and whether that press was already the second one.
    last: Option<(u16, u16, bool)>,
}

impl PressSequence {
    /// Follow one event, and mark the second press of a double click with [`DOUBLE_PRESS`].
    ///
    /// `on_text_box` tells whether a primary press lands on a text box.
    pub(crate) fn follow(&mut self, event: &mut Event, on_text_box: bool) {
        let Event::Mouse(mouse) = event else {
            self.last = None;
            return;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) if !on_text_box => self.last = None,
            MouseEventKind::Down(MouseButton::Left) => {
                let double = matches!(
                    self.last,
                    Some((column, row, false)) if column == mouse.column && row == mouse.row
                );
                if double {
                    mouse.modifiers |= DOUBLE_PRESS;
                }
                self.last = Some((mouse.column, mouse.row, double));
            }
            MouseEventKind::Up(MouseButton::Left) => {}
            _ => self.last = None,
        }
    }

    /// Forget the last press, so the next press is a first press.
    pub(crate) fn end(&mut self) {
        self.last = None;
    }

    pub(crate) fn snapshot(self) -> serde_json::Value {
        self.last.map_or(serde_json::Value::Null, |(column, row, second)| {
            serde_json::json!({"column": column, "row": row, "second": second})
        })
    }
}

/// The last cell that the pointer reported, for the hover look of chips and buttons.
///
/// Mouse capture reports every move, so the cell follows the pointer. The terminal reports no
/// leave event: a lost focus clears the cell instead.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PointerPosition {
    cell: Option<(u16, u16)>,
}

impl PointerPosition {
    /// Follow one terminal event.
    pub(crate) fn follow(&mut self, event: &Event) {
        match event {
            Event::Mouse(mouse) => self.cell = Some((mouse.column, mouse.row)),
            Event::FocusLost => self.cell = None,
            Event::FocusGained | Event::Key(_) | Event::Paste(_) | Event::Resize(_, _) => {}
        }
    }

    /// Report whether the pointer rests on `rect`.
    pub(crate) fn is_over(self, rect: Rect) -> bool {
        self.cell
            .is_some_and(|(column, row)| contains(rect, column, row))
    }

    /// The cell, or null, for the canonical session snapshot.
    pub(crate) fn snapshot(self) -> serde_json::Value {
        self.cell.map_or(
            serde_json::Value::Null,
            |(column, row)| serde_json::json!({"column": column, "row": row}),
        )
    }
}

/// A typed hit map. The last rendered region is the topmost region.
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
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

impl<T> ClickTracker<T> {
    /// Return the semantic target held between a primary press and release.
    pub(crate) const fn pressed(&self) -> Option<&T> {
        self.pressed.as_ref()
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
    /// Return regions in registration order. The last matching region is topmost.
    pub(crate) fn regions(&self) -> &[(Rect, T)] {
        &self.regions
    }

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

    pub(crate) const fn content(self) -> Rect {
        self.content
    }

    pub(crate) const fn visual_scroll(self) -> usize {
        self.visual_scroll
    }

    pub(crate) const fn secret(self) -> bool {
        self.secret
    }

    /// Report whether the cell shows the text of this box.
    pub(crate) const fn position_contains(self, column: u16, row: u16) -> bool {
        contains(self.content, column, row)
    }

    /// The caret position at one cell, or nothing when the cell is outside the box.
    pub(crate) fn position_at(
        &self,
        editor: &TextEditor,
        column: u16,
        row: u16,
    ) -> Option<TextPosition> {
        contains(self.content, column, row).then(|| self.clamped_position(editor, column))
    }

    /// The caret position at the nearest cell of the box, for a drag that leaves the box.
    fn clamped_position(&self, editor: &TextEditor, column: u16) -> TextPosition {
        let column = column.clamp(
            self.content.x,
            self.content.right().saturating_sub(1).max(self.content.x),
        );
        let visual_cell = self
            .visual_scroll
            .saturating_add(usize::from(column.saturating_sub(self.content.x)));
        TextPosition::new(
            0,
            cursor_for_visual_cell(editor.value(), visual_cell, self.secret),
        )
    }

    /// Give one primary press or drag to `editor`, and report whether the box took it.
    ///
    /// A press puts the caret at the cell, and the second press of a double click selects the
    /// word there. A drag moves the caret and keeps the anchor of the press, so it selects.
    pub(crate) fn pointer(&self, editor: &mut TextEditor, mouse: &MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(position) = self.position_at(editor, mouse.column, mouse.row) else {
                    return false;
                };
                press(editor, position, mouse);
                true
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                editor.drag_to(self.clamped_position(editor, mouse.column));
                true
            }
            _ => false,
        }
    }
}

fn press(editor: &mut TextEditor, position: TextPosition, mouse: &MouseEvent) {
    if mouse.modifiers.contains(DOUBLE_PRESS) {
        editor.select_word_at(position);
    } else {
        editor.press_at(position);
    }
}

/// Report whether `mouse` is a primary drag.
pub(crate) const fn is_primary_drag(mouse: &MouseEvent) -> bool {
    matches!(mouse.kind, MouseEventKind::Drag(MouseButton::Left))
}

/// Persistent viewport coordinates for one multiline editor.
///
/// The renderer and the pointer use these coordinates, so a terminal cell can map back to the
/// logical line and character that the renderer drew.
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
}

impl TextAreaViewport {
    pub(crate) fn align(&mut self, state: &TextEditor, width: usize, height: usize) {
        let cursor = state.cursor();
        let line = &state.lines()[cursor.line];
        let visual_column = display_column(line, cursor.column);
        self.top_row = aligned_scroll(self.top_row, cursor.line, height);
        self.left_cell =
            textarea_scroll_boundary(line, aligned_scroll(self.left_cell, visual_column, width));
    }

    pub(crate) const fn top_row(self) -> usize {
        self.top_row
    }

    pub(crate) const fn left_cell(self) -> usize {
        self.left_cell
    }

    pub(crate) const fn geometry(self, content: Rect, skipped_rows: usize) -> TextAreaGeometry {
        TextAreaGeometry {
            content,
            first_row: self.top_row.saturating_add(skipped_rows),
            left_cell: self.left_cell,
        }
    }
}

impl TextAreaGeometry {
    pub(crate) const fn content(self) -> Rect {
        self.content
    }

    pub(crate) const fn first_row(self) -> usize {
        self.first_row
    }

    pub(crate) const fn left_cell(self) -> usize {
        self.left_cell
    }

    /// Report whether the cell shows the text of this box.
    pub(crate) const fn position_contains(self, column: u16, row: u16) -> bool {
        contains(self.content, column, row)
    }

    /// The caret position at one cell, or nothing when the cell is outside the box.
    pub(crate) fn position_at(
        self,
        editor: &TextEditor,
        column: u16,
        row: u16,
    ) -> Option<TextPosition> {
        contains(self.content, column, row).then(|| self.clamped_position(editor, column, row))
    }

    /// The caret position at the nearest cell of the box, for a drag that leaves the box.
    fn clamped_position(self, editor: &TextEditor, column: u16, row: u16) -> TextPosition {
        let column = column.clamp(
            self.content.x,
            self.content.right().saturating_sub(1).max(self.content.x),
        );
        let line = if row < self.content.y {
            self.first_row
                .saturating_sub(usize::from(self.content.y - row))
        } else {
            self.first_row
                .saturating_add(usize::from(row - self.content.y))
        }
        .min(editor.lines().len().saturating_sub(1));
        let visual_cell = self
            .left_cell
            .saturating_add(usize::from(column.saturating_sub(self.content.x)));
        TextPosition::new(
            line,
            cursor_for_textarea_visual_cell(&editor.lines()[line], visual_cell),
        )
    }

    /// Give one primary press or drag to `editor`, and report whether the box took it.
    pub(crate) fn pointer(self, editor: &mut TextEditor, mouse: &MouseEvent) -> bool {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let Some(position) = self.position_at(editor, mouse.column, mouse.row) else {
                    return false;
                };
                press(editor, position, mouse);
                true
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                editor.drag_to(self.clamped_position(editor, mouse.column, mouse.row));
                true
            }
            _ => false,
        }
    }
}

fn textarea_scroll_boundary(line: &str, wanted: usize) -> usize {
    let mut boundary = 0_usize;
    for grapheme in line.graphemes(true) {
        let next = boundary.saturating_add(grapheme_cells(grapheme, boundary));
        if next > wanted {
            break;
        }
        boundary = next;
    }
    boundary
}

fn cursor_for_textarea_visual_cell(value: &str, visual_cell: usize) -> usize {
    let mut cells = 0_usize;
    let mut codepoints = 0_usize;
    for grapheme in value.graphemes(true) {
        let width = grapheme_cells(grapheme, cells);
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

/// The first visible cell of a single-line box that shows the caret.
///
/// The caret always keeps a free cell of its own. At the end of a long value it stands after the
/// last character, not on it, as version 0.4 showed it.
pub(crate) fn display_scroll(value: &str, cursor: usize, width: usize, secret: bool) -> usize {
    let wanted = display_cursor(value, cursor, secret)
        .saturating_add(1)
        .saturating_sub(width);
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

    // Ways this can fail: the caret at the end of a long value stands on the last character
    // instead of a free cell; the scroll starts inside a wide or combined grapheme.
    #[test]
    fn display_scroll_keeps_a_free_caret_cell_without_splitting_a_grapheme() {
        for (value, width, secret, expected) in [
            ("abcdef", 3, false, 4),
            ("abc", 3, false, 1),
            ("ab", 3, false, 0),
            ("a界b", 2, false, 3),
            ("界a", 2, false, 2),
            ("e\u{301}xyz", 2, false, 3),
            ("👨‍👩‍👧‍👦xy", 2, false, 3),
            ("界a", 1, true, 2),
            ("", 1, false, 0),
        ] {
            let cursor = value.chars().count();
            let scroll = display_scroll(value, cursor, width, secret);
            assert_eq!(
                scroll, expected,
                "value={value:?} width={width} secret={secret}"
            );
            assert!(
                display_cursor(value, cursor, secret) - scroll < width,
                "the caret has no free cell: value={value:?} width={width}"
            );
        }
    }

    // Ways this can fail: the second press on one cell is not marked; a press on another cell,
    // a third press, or any other event in between still counts as a double click.
    #[test]
    fn a_second_press_on_the_same_cell_is_marked_as_a_double_click() {
        let mouse = |kind, column| {
            Event::Mouse(MouseEvent {
                kind,
                column,
                row: 2,
                modifiers: KeyModifiers::NONE,
            })
        };
        let marked = |event: &Event| matches!(event, Event::Mouse(mouse) if mouse.modifiers.contains(DOUBLE_PRESS));
        let mut sequence = PressSequence::default();
        let mut follow = |event: Event| {
            let mut event = event;
            sequence.follow(&mut event, true);
            marked(&event)
        };
        let down = MouseEventKind::Down(MouseButton::Left);
        let up = MouseEventKind::Up(MouseButton::Left);
        assert!(!follow(mouse(down, 4)));
        assert!(!follow(mouse(up, 4)));
        assert!(follow(mouse(down, 4)));
        assert!(!follow(mouse(up, 4)));
        assert!(
            !follow(mouse(down, 4)),
            "a third press is a first press again"
        );
        assert!(!follow(mouse(down, 5)), "another cell is a first press");
        assert!(!follow(mouse(MouseEventKind::Moved, 5)));
        assert!(!follow(mouse(down, 5)), "a move ends the sequence");
        assert!(!follow(Event::FocusLost));
        assert!(!follow(mouse(down, 5)), "another event ends the sequence");
        let mut outside = PressSequence::default();
        let mut first = mouse(down, 1);
        outside.follow(&mut first, true);
        let mut button = mouse(down, 1);
        outside.follow(&mut button, false);
        assert!(
            !marked(&button),
            "a press outside a text box is no second press"
        );
        assert_eq!(outside.snapshot(), serde_json::Value::Null);
        let mut ended = PressSequence::default();
        let mut first = mouse(down, 1);
        ended.follow(&mut first, true);
        assert_eq!(
            ended.snapshot(),
            serde_json::json!({"column": 1, "row": 2, "second": false})
        );
        ended.end();
        assert_eq!(ended.snapshot(), serde_json::Value::Null);
        let mut second = mouse(down, 1);
        ended.follow(&mut second, true);
        assert!(!marked(&second), "an ended sequence starts again");
    }

    // Ways this can fail: a press or a drag lands outside the text or inside a grapheme; a drag
    // outside the box does not reach the nearest cell; a double press does not select a word.
    #[test]
    fn line_and_area_geometry_turn_presses_and_drags_into_editor_selections() {
        let event = |kind, column, row, modifiers| MouseEvent {
            kind,
            column,
            row,
            modifiers,
        };
        let down = MouseEventKind::Down(MouseButton::Left);
        let drag = MouseEventKind::Drag(MouseButton::Left);
        let line = EditableGeometry::new(Rect::new(10, 3, 5, 1), 2, false);
        let mut editor = TextEditor::single_line("ab界cdef");
        assert!(!line.pointer(&mut editor, &event(down, 9, 3, KeyModifiers::NONE)));
        assert!(!line.pointer(
            &mut editor,
            &event(MouseEventKind::Moved, 10, 3, KeyModifiers::NONE)
        ));
        assert!(line.pointer(&mut editor, &event(down, 10, 3, KeyModifiers::NONE)));
        assert_eq!(editor.cursor(), TextPosition::new(0, 2));
        assert!(line.pointer(&mut editor, &event(drag, 40, 9, KeyModifiers::NONE)));
        assert_eq!(editor.selected_text().as_deref(), Some("界cd"));
        assert!(line.pointer(&mut editor, &event(down, 13, 3, DOUBLE_PRESS)));
        assert_eq!(editor.selected_text().as_deref(), Some("cdef"));
        assert_eq!(line.position_at(&editor, 15, 3), None);

        let area = TextAreaGeometry {
            content: Rect::new(2, 5, 4, 2),
            first_row: 1,
            left_cell: 0,
        };
        let mut text = TextEditor::multi_line(
            "zero
one two
three
	x",
        );
        assert!(area.pointer(&mut text, &event(down, 3, 6, KeyModifiers::NONE)));
        assert_eq!(text.cursor(), TextPosition::new(2, 1));
        assert!(area.pointer(&mut text, &event(drag, 0, 2, KeyModifiers::NONE)));
        assert_eq!(text.cursor(), TextPosition::new(0, 0));
        assert!(area.pointer(&mut text, &event(drag, 9, 9, KeyModifiers::NONE)));
        assert_eq!(text.cursor(), TextPosition::new(3, 0));
        assert!(area.pointer(&mut text, &event(down, 2, 5, DOUBLE_PRESS)));
        assert_eq!(text.selected_text().as_deref(), Some("one"));
        assert!(!area.pointer(&mut text, &event(down, 2, 7, KeyModifiers::NONE)));
        assert!(!area.pointer(
            &mut text,
            &event(MouseEventKind::Moved, 2, 5, KeyModifiers::NONE)
        ));
        assert_eq!(cursor_for_textarea_visual_cell("\tx", 2), 0);
        assert_eq!(cursor_for_textarea_visual_cell("\tx", 4), 1);
        assert!(is_primary_drag(&event(drag, 0, 0, KeyModifiers::NONE)));
        assert!(!is_primary_drag(&event(down, 0, 0, KeyModifiers::NONE)));
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
        let mut state = TextEditor::multi_line("zero\none\ntwo\n0123456789");
        let _ = state.apply(crate::editor::EditCommand::Move {
            motion: crate::editor::Motion::CharLeft,
            select: false,
        });
        let _ = state.apply(crate::editor::EditCommand::Move {
            motion: crate::editor::Motion::CharLeft,
            select: false,
        });
        let mut viewport = TextAreaViewport::default();

        viewport.align(&state, 4, 2);

        assert_eq!(viewport.geometry(Rect::new(4, 5, 4, 2), 1).first_row, 3);
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
