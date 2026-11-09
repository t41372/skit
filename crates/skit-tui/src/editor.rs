//! One editing model for every text box.
//!
//! A single-line box and the multi-line box use the same state, the same key map, the same caret
//! and selection rules, the same undo history, and the same paste policy. A screen gives a key to
//! [`TextEditor::handle_key`] first. The editor ignores the keys that are not editing keys (Enter
//! in a single-line box, Tab, Esc, the function keys, and every screen chord), so the screen keeps
//! its own commands.
//!
//! The editor counts positions in characters, and a position is always on a grapheme boundary. A
//! wide character, an emoji sequence, or a letter with a combining mark is one step for every move
//! and every delete.

use std::borrow::Cow;

use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation as _;
use unicode_width::UnicodeWidthStr as _;

/// The number of edits that undo can take back in one box.
const HISTORY_LIMIT: usize = 100;

/// The cells from one tab stop to the next in the multi-line box.
pub(crate) const TAB_WIDTH: usize = 4;

/// One caret position: a line, and the characters before the caret on that line.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct TextPosition {
    /// The zero-based line.
    pub(crate) line: usize,
    /// The characters before the caret on its line.
    pub(crate) column: usize,
}

impl TextPosition {
    pub(crate) const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

/// One caret move.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Motion {
    CharLeft,
    CharRight,
    WordLeft,
    WordRight,
    LineStart,
    LineEnd,
    LineUp,
    LineDown,
    TextStart,
    TextEnd,
}

/// One editing operation. [`edit_command`] maps a key to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditCommand {
    /// Move the caret. `select` keeps the selection anchor, so the move selects text.
    Move {
        motion: Motion,
        select: bool,
    },
    Insert(char),
    NewLine,
    DeleteBackward,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    DeleteToLineStart,
    DeleteToLineEnd,
    /// Remove the selection. The session copies it to the clipboard first.
    Cut,
    Undo,
    Redo,
}

/// What the editor did with one event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EditResponse {
    /// The event is not an editing event. The screen can use it.
    Ignored,
    /// The editor owned the event, and the text did not change.
    Consumed,
    /// The text changed.
    Changed,
    /// Up or Down reached the first or the last line of a multi-line box, with no selection.
    Boundary,
}

/// The editing keys that Help lists, each with what it does.
///
/// Every chord here has a positive test in this module, and the end-to-end cases drive them in a
/// real terminal. The row after the Backspace row names the one box that gives an editing chord to
/// its screen: the Search box of the file picker keeps Ctrl+H for the hidden-files chip of the
/// picker.
pub(crate) const EDITING_HELP: &[(&str, &str)] = &[
    ("Ctrl+A, Home", "Go to the start of the line"),
    ("Ctrl+E, End", "Go to the end of the line"),
    ("Alt+B, Alt+←, Ctrl+←", "Go to the previous word"),
    ("Alt+F, Alt+→, Ctrl+→", "Go to the next word"),
    ("Shift+←/→/Home/End", "Select text"),
    ("Ctrl+Shift+←/→, Alt+Shift+←/→", "Select by word"),
    ("Drag, double-click", "Select with the mouse"),
    (
        "Backspace, Ctrl+H",
        "Delete the character before the cursor",
    ),
    ("Ctrl+H", "Show or hide hidden files in the file picker"),
    ("Delete, Ctrl+D", "Delete the character after the cursor"),
    ("Ctrl+W, Alt+Backspace", "Delete the word before the cursor"),
    ("Alt+D, Ctrl+Delete", "Delete the word after the cursor"),
    ("Ctrl+U", "Delete to the start of the line"),
    ("Ctrl+K", "Delete to the end of the line"),
    ("Ctrl+C", "Copy the selected text"),
    ("Ctrl+X", "Cut the selected text"),
    ("Ctrl+V", "Paste"),
    ("Ctrl+Z", "Undo"),
    ("Ctrl+Y", "Redo"),
];

/// Map one key to its editing operation.
///
/// This is the only key map of the text boxes. `multiline` gives Enter, Up, and Down to the box;
/// a single-line box leaves them to the screen.
pub(crate) fn edit_command(key: &KeyEvent, multiline: bool) -> Option<EditCommand> {
    if key.kind == KeyEventKind::Release
        || key
            .modifiers
            .intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META)
    {
        return None;
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let select = key.modifiers.contains(KeyModifiers::SHIFT);
    let move_by = |motion| Some(EditCommand::Move { motion, select });
    match key.code {
        KeyCode::Char(character) if !control && !alt => Some(EditCommand::Insert(character)),
        // Windows reports a character typed with AltGr as Ctrl+Alt and the character.
        KeyCode::Char(character) if control && alt && !character.is_ascii_alphabetic() => {
            Some(EditCommand::Insert(character))
        }
        KeyCode::Char(character) if control && !alt => match character.to_ascii_lowercase() {
            'a' => move_by(Motion::LineStart),
            'e' => move_by(Motion::LineEnd),
            'h' => Some(EditCommand::DeleteBackward),
            'd' => Some(EditCommand::DeleteForward),
            'w' => Some(EditCommand::DeleteWordBackward),
            'u' => Some(EditCommand::DeleteToLineStart),
            'k' => Some(EditCommand::DeleteToLineEnd),
            'x' => Some(EditCommand::Cut),
            'z' if select || character == 'Z' => Some(EditCommand::Redo),
            'z' => Some(EditCommand::Undo),
            'y' => Some(EditCommand::Redo),
            _ => None,
        },
        KeyCode::Char(character) if alt && !control => match character.to_ascii_lowercase() {
            'b' => move_by(Motion::WordLeft),
            'f' => move_by(Motion::WordRight),
            'd' => Some(EditCommand::DeleteWordForward),
            _ => None,
        },
        KeyCode::Left if control || alt => move_by(Motion::WordLeft),
        KeyCode::Left => move_by(Motion::CharLeft),
        KeyCode::Right if control || alt => move_by(Motion::WordRight),
        KeyCode::Right => move_by(Motion::CharRight),
        KeyCode::Home if control => move_by(Motion::TextStart),
        KeyCode::Home => move_by(Motion::LineStart),
        KeyCode::End if control => move_by(Motion::TextEnd),
        KeyCode::End => move_by(Motion::LineEnd),
        KeyCode::Up if multiline && !control && !alt => move_by(Motion::LineUp),
        KeyCode::Down if multiline && !control && !alt => move_by(Motion::LineDown),
        KeyCode::Backspace if control || alt => Some(EditCommand::DeleteWordBackward),
        KeyCode::Backspace => Some(EditCommand::DeleteBackward),
        KeyCode::Delete if control || alt => Some(EditCommand::DeleteWordForward),
        KeyCode::Delete => Some(EditCommand::DeleteForward),
        KeyCode::Enter if multiline && !control && !alt => Some(EditCommand::NewLine),
        _ => None,
    }
}

/// Report whether `key` is the copy chord: Ctrl+C.
pub(crate) fn is_copy_key(key: &KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('c' | 'C'))
}

/// Report whether `key` is the paste chord: Ctrl+V.
pub(crate) fn is_paste_key(key: &KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
        && key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('v' | 'V'))
}

/// Report whether `key` is the cut chord: Ctrl+X.
pub(crate) fn is_cut_key(key: &KeyEvent) -> bool {
    edit_command(key, false) == Some(EditCommand::Cut)
}

/// Return the text that a paste inserts into a box.
///
/// A terminal can send a line break as CR, LF, or CRLF, so every form becomes LF. A single-line
/// box keeps only the first line, as version 0.4 did. A Tab in that line stays a Tab: the box
/// shows it as one blank cell, and the value keeps it.
pub(crate) fn paste_text(raw: &str, multiline: bool) -> Cow<'_, str> {
    if multiline {
        if raw.contains('\r') {
            Cow::Owned(raw.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            Cow::Borrowed(raw)
        }
    } else {
        let end = raw.find(['\r', '\n']).unwrap_or(raw.len());
        Cow::Borrowed(&raw[..end])
    }
}

/// The complete text, caret, and selection of one box.
#[derive(Clone, Debug, Eq, PartialEq)]
struct EditorState {
    lines: Vec<String>,
    cursor: TextPosition,
    anchor: Option<TextPosition>,
}

/// The run of edits that the next edit can join, so one undo takes the whole run back.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EditGroup {
    /// Typed characters. A word and the spaces after it are one undo step.
    Typing {
        after_space: bool,
    },
    DeleteBackward,
    DeleteForward,
}

/// Editing state of one text box.
#[derive(Clone, Debug)]
pub(crate) struct TextEditor {
    state: EditorState,
    multiline: bool,
    undo: Vec<EditorState>,
    redo: Vec<EditorState>,
    group: Option<EditGroup>,
    /// The display column that consecutive Up and Down moves keep.
    goal_column: Option<usize>,
}

impl Default for TextEditor {
    fn default() -> Self {
        Self::single_line("")
    }
}

impl TextEditor {
    /// A single-line box with `value`, and the caret after its last character.
    pub(crate) fn single_line(value: &str) -> Self {
        Self::with_lines(vec![value.to_owned()], false)
    }

    /// A multi-line box with `text`, and the caret after its last character.
    pub(crate) fn multi_line(text: &str) -> Self {
        Self::with_lines(text.split('\n').map(str::to_owned).collect(), true)
    }

    /// A box with `lines`, and the caret after the last character.
    ///
    /// `lines` always holds one line or more: a single-line box gives its value, and splitting a
    /// text at each LF gives one line even for an empty text.
    fn with_lines(lines: Vec<String>, multiline: bool) -> Self {
        let last = lines.len() - 1;
        let cursor = TextPosition::new(last, lines[last].chars().count());
        Self {
            state: EditorState {
                lines,
                cursor,
                anchor: None,
            },
            multiline,
            undo: Vec::new(),
            redo: Vec::new(),
            group: None,
            goal_column: None,
        }
    }

    /// Replace the whole text with a value that another owner set, and forget the history.
    ///
    /// A screen calls this when the model holds a value that the box did not make. The caret goes
    /// after the last character, as it does in a new box.
    pub(crate) fn replace_text(&mut self, text: &str) {
        *self = if self.multiline {
            Self::multi_line(text)
        } else {
            Self::single_line(text)
        };
    }

    /// The lines of the text. A single-line box has exactly one.
    pub(crate) fn lines(&self) -> &[String] {
        &self.state.lines
    }

    /// The first line, which is the whole value of a single-line box.
    pub(crate) fn value(&self) -> &str {
        &self.state.lines[0]
    }

    /// The whole text, with LF between lines.
    pub(crate) fn text(&self) -> Cow<'_, str> {
        if self.state.lines.len() == 1 {
            Cow::Borrowed(&self.state.lines[0])
        } else {
            Cow::Owned(self.state.lines.join("\n"))
        }
    }

    pub(crate) const fn cursor(&self) -> TextPosition {
        self.state.cursor
    }

    /// The selected span, start first, when the selection is not empty.
    pub(crate) fn selection(&self) -> Option<(TextPosition, TextPosition)> {
        let anchor = self.state.anchor?;
        let cursor = self.state.cursor;
        match anchor.cmp(&cursor) {
            std::cmp::Ordering::Less => Some((anchor, cursor)),
            std::cmp::Ordering::Greater => Some((cursor, anchor)),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// The selected text, when the selection is not empty.
    pub(crate) fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        Some(self.slice(start, end))
    }

    /// Give one key to the box.
    pub(crate) fn handle_key(&mut self, key: &KeyEvent) -> EditResponse {
        edit_command(key, self.multiline)
            .map_or(EditResponse::Ignored, |command| self.apply(command))
    }

    /// Insert pasted text at the caret, in place of the selection, as one undo step.
    pub(crate) fn paste(&mut self, raw: &str) -> EditResponse {
        let text = paste_text(raw, self.multiline);
        if text.is_empty() {
            return EditResponse::Consumed;
        }
        let text = text.into_owned();
        self.insert(&text, None)
    }

    /// Replace the whole text as one edit that undo can take back.
    pub(crate) fn set_text(&mut self, text: &str) -> EditResponse {
        if self.text() == text {
            return EditResponse::Consumed;
        }
        let last = self.state.lines.len() - 1;
        let end = TextPosition::new(last, self.state.lines[last].chars().count());
        let text = if self.multiline {
            text.to_owned()
        } else {
            paste_text(text, false).into_owned()
        };
        self.replace(TextPosition::default(), end, &text, None);
        EditResponse::Changed
    }

    /// Put the caret at `position` with no selection, for a pointer press.
    ///
    /// A later [`Self::drag_to`] selects from this position.
    pub(crate) fn press_at(&mut self, position: TextPosition) {
        let position = self.clamp(position);
        self.place(position, position);
    }

    /// Move the caret to `position` and keep the anchor, for a pointer drag.
    pub(crate) fn drag_to(&mut self, position: TextPosition) {
        let position = self.clamp(position);
        let anchor = self.state.anchor.unwrap_or(self.state.cursor);
        self.place(position, anchor);
    }

    /// Select the word, or the run of other characters, at `position`, for a double click.
    pub(crate) fn select_word_at(&mut self, position: TextPosition) {
        let position = self.clamp(position);
        let line = &self.state.lines[position.line];
        let (start, end) = word_span_at(line, position.column);
        self.place(
            TextPosition::new(position.line, end),
            TextPosition::new(position.line, start),
        );
    }

    /// Put the caret and the selection anchor where the pointer put them.
    ///
    /// The pointer chooses a new column, so the next Up or Down starts from the caret, not from
    /// the column of an earlier vertical move.
    fn place(&mut self, cursor: TextPosition, anchor: TextPosition) {
        self.state.cursor = cursor;
        self.state.anchor = Some(anchor);
        self.goal_column = None;
        self.end_group();
    }

    /// Apply one editing operation.
    pub(crate) fn apply(&mut self, command: EditCommand) -> EditResponse {
        match command {
            EditCommand::Move { motion, select } => self.move_caret(motion, select),
            EditCommand::Insert(character) => {
                let mut buffer = [0_u8; 4];
                let text = character.encode_utf8(&mut buffer);
                let after_space = character.is_whitespace();
                let joins = matches!(
                    self.group,
                    Some(EditGroup::Typing { after_space: previous }) if !previous || after_space
                ) && self.selection().is_none();
                self.insert(text, Some((EditGroup::Typing { after_space }, joins)))
            }
            EditCommand::NewLine => self.insert("\n", None),
            EditCommand::DeleteBackward => self.delete_backward(),
            EditCommand::DeleteForward => self.delete_forward(),
            EditCommand::DeleteWordBackward => self.delete_to(|editor| editor.word_left()),
            EditCommand::DeleteWordForward => self.delete_to(|editor| editor.word_right()),
            EditCommand::DeleteToLineStart => {
                self.delete_to(|editor| TextPosition::new(editor.state.cursor.line, 0))
            }
            EditCommand::DeleteToLineEnd => self.delete_to(|editor| {
                let line = editor.state.cursor.line;
                TextPosition::new(line, editor.state.lines[line].chars().count())
            }),
            EditCommand::Cut => match self.selection() {
                Some((start, end)) => {
                    self.replace(start, end, "", None);
                    EditResponse::Changed
                }
                None => EditResponse::Consumed,
            },
            EditCommand::Undo => self.step_history(true),
            EditCommand::Redo => self.step_history(false),
        }
    }

    fn move_caret(&mut self, motion: Motion, select: bool) -> EditResponse {
        let cursor = self.state.cursor;
        let selection = self.selection();
        let vertical = matches!(motion, Motion::LineUp | Motion::LineDown);
        if !vertical {
            self.goal_column = None;
        }
        self.end_group();
        if !select && let Some((start, end)) = selection {
            // An arrow without Shift ends the selection at its own side, as a text field does.
            match motion {
                Motion::CharLeft => {
                    self.state.cursor = start;
                    self.state.anchor = None;
                    return EditResponse::Consumed;
                }
                Motion::CharRight => {
                    self.state.cursor = end;
                    self.state.anchor = None;
                    return EditResponse::Consumed;
                }
                _ => {}
            }
        }
        let target = match motion {
            Motion::CharLeft => self.char_left(),
            Motion::CharRight => self.char_right(),
            Motion::WordLeft => self.word_left(),
            Motion::WordRight => self.word_right(),
            Motion::LineStart => TextPosition::new(cursor.line, 0),
            Motion::LineEnd => {
                TextPosition::new(cursor.line, self.state.lines[cursor.line].chars().count())
            }
            Motion::TextStart => TextPosition::default(),
            Motion::TextEnd => {
                let last = self.state.lines.len() - 1;
                TextPosition::new(last, self.state.lines[last].chars().count())
            }
            Motion::LineUp | Motion::LineDown => {
                let up = motion == Motion::LineUp;
                let edge = if up {
                    cursor.line == 0
                } else {
                    cursor.line + 1 == self.state.lines.len()
                };
                if edge {
                    if selection.is_none() && !select {
                        return EditResponse::Boundary;
                    }
                    if !select {
                        self.state.anchor = None;
                        return EditResponse::Consumed;
                    }
                    if up {
                        TextPosition::default()
                    } else {
                        TextPosition::new(
                            cursor.line,
                            self.state.lines[cursor.line].chars().count(),
                        )
                    }
                } else {
                    let goal = *self.goal_column.get_or_insert_with(|| {
                        display_column(&self.state.lines[cursor.line], cursor.column)
                    });
                    let line = if up { cursor.line - 1 } else { cursor.line + 1 };
                    TextPosition::new(line, column_at_cells(&self.state.lines[line], goal))
                }
            }
        };
        if select {
            if self.state.anchor.is_none() {
                self.state.anchor = Some(cursor);
            }
        } else {
            self.state.anchor = None;
        }
        self.state.cursor = target;
        EditResponse::Consumed
    }

    fn char_left(&self) -> TextPosition {
        let cursor = self.state.cursor;
        if cursor.column > 0 {
            let line = &self.state.lines[cursor.line];
            TextPosition::new(cursor.line, previous_boundary(line, cursor.column))
        } else if cursor.line > 0 {
            let line = cursor.line - 1;
            TextPosition::new(line, self.state.lines[line].chars().count())
        } else {
            cursor
        }
    }

    fn char_right(&self) -> TextPosition {
        let cursor = self.state.cursor;
        let line = &self.state.lines[cursor.line];
        if cursor.column < line.chars().count() {
            TextPosition::new(cursor.line, next_boundary(line, cursor.column))
        } else if cursor.line + 1 < self.state.lines.len() {
            TextPosition::new(cursor.line + 1, 0)
        } else {
            cursor
        }
    }

    fn word_left(&self) -> TextPosition {
        let cursor = self.state.cursor;
        if cursor.column == 0 {
            return if cursor.line > 0 {
                let line = cursor.line - 1;
                TextPosition::new(line, self.state.lines[line].chars().count())
            } else {
                cursor
            };
        }
        let line = &self.state.lines[cursor.line];
        TextPosition::new(
            cursor.line,
            word_start_before(line, cursor.column).unwrap_or(0),
        )
    }

    fn word_right(&self) -> TextPosition {
        let cursor = self.state.cursor;
        let line = &self.state.lines[cursor.line];
        let length = line.chars().count();
        if cursor.column == length {
            return if cursor.line + 1 < self.state.lines.len() {
                TextPosition::new(cursor.line + 1, 0)
            } else {
                cursor
            };
        }
        TextPosition::new(
            cursor.line,
            word_start_after(line, cursor.column).unwrap_or(length),
        )
    }

    fn delete_backward(&mut self) -> EditResponse {
        if let Some((start, end)) = self.selection() {
            self.replace(start, end, "", None);
            return EditResponse::Changed;
        }
        let target = self.char_left();
        if target == self.state.cursor {
            self.end_group();
            return EditResponse::Consumed;
        }
        let joins = self.group == Some(EditGroup::DeleteBackward);
        self.replace(
            target,
            self.state.cursor,
            "",
            Some((EditGroup::DeleteBackward, joins)),
        );
        EditResponse::Changed
    }

    fn delete_forward(&mut self) -> EditResponse {
        if let Some((start, end)) = self.selection() {
            self.replace(start, end, "", None);
            return EditResponse::Changed;
        }
        let target = self.char_right();
        if target == self.state.cursor {
            self.end_group();
            return EditResponse::Consumed;
        }
        let joins = self.group == Some(EditGroup::DeleteForward);
        self.replace(
            self.state.cursor,
            target,
            "",
            Some((EditGroup::DeleteForward, joins)),
        );
        EditResponse::Changed
    }

    /// Delete the selection, or the span from the caret to the position that `target` names.
    fn delete_to(&mut self, target: impl FnOnce(&Self) -> TextPosition) -> EditResponse {
        if let Some((start, end)) = self.selection() {
            self.replace(start, end, "", None);
            return EditResponse::Changed;
        }
        let cursor = self.state.cursor;
        let target = target(self);
        if target == cursor {
            self.end_group();
            return EditResponse::Consumed;
        }
        let (start, end) = if target < cursor {
            (target, cursor)
        } else {
            (cursor, target)
        };
        self.replace(start, end, "", None);
        EditResponse::Changed
    }

    fn insert(&mut self, text: &str, group: Option<(EditGroup, bool)>) -> EditResponse {
        let (start, end) = self
            .selection()
            .unwrap_or((self.state.cursor, self.state.cursor));
        let text = if self.multiline {
            Cow::Borrowed(text)
        } else {
            Cow::Owned(text.replace('\n', ""))
        };
        if text.is_empty() && start == end {
            self.end_group();
            return EditResponse::Consumed;
        }
        if start != end && self.slice(start, end) == *text {
            // The text replaces a selection that holds the same text, so the text stays as it
            // was. That is no edit: no undo step records it, and the caret goes after the text.
            self.state.cursor = end;
            self.state.anchor = None;
            self.goal_column = None;
            self.end_group();
            return EditResponse::Consumed;
        }
        self.replace(start, end, &text, group);
        EditResponse::Changed
    }

    /// Replace the span `start..end` with `text`, record the undo step, and put the caret after
    /// the new text.
    ///
    /// The text before and after the span can join into one grapheme, for example an emoji, a
    /// zero-width joiner, and a second emoji, or two regional indicators. The caret then goes after
    /// that grapheme, where the box draws it, so it stays on a grapheme boundary.
    fn replace(
        &mut self,
        start: TextPosition,
        end: TextPosition,
        text: &str,
        group: Option<(EditGroup, bool)>,
    ) {
        let joins = group.is_some_and(|(_, joins)| joins);
        if !joins {
            self.undo.push(self.state.clone());
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.group = group.map(|(group, _)| group);
        self.goal_column = None;

        let lines = &mut self.state.lines;
        let tail = {
            let end_line = &lines[end.line];
            end_line[byte_index(end_line, end.column)..].to_owned()
        };
        let head = {
            let start_line = &lines[start.line];
            start_line[..byte_index(start_line, start.column)].to_owned()
        };
        let mut inserted = text.split('\n');
        let first = inserted.next().unwrap_or_default();
        let mut replacement = vec![format!("{head}{first}")];
        replacement.extend(inserted.map(str::to_owned));
        let last = replacement.len() - 1;
        let cursor_column = replacement[last].chars().count();
        replacement[last].push_str(&tail);
        let cursor_column = boundary_at_or_after(&replacement[last], cursor_column);
        lines.splice(start.line..=end.line, replacement);
        self.state.cursor = TextPosition::new(start.line + last, cursor_column);
        self.state.anchor = None;
    }

    fn step_history(&mut self, undo: bool) -> EditResponse {
        let (from, to) = if undo {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some(state) = from.pop() else {
            self.group = None;
            return EditResponse::Consumed;
        };
        to.push(std::mem::replace(&mut self.state, state));
        self.group = None;
        self.goal_column = None;
        EditResponse::Changed
    }

    fn end_group(&mut self) {
        self.group = None;
    }

    fn slice(&self, start: TextPosition, end: TextPosition) -> String {
        let lines = &self.state.lines;
        if start.line == end.line {
            let line = &lines[start.line];
            return line[byte_index(line, start.column)..byte_index(line, end.column)].to_owned();
        }
        let mut text = {
            let line = &lines[start.line];
            line[byte_index(line, start.column)..].to_owned()
        };
        for line in &lines[start.line + 1..end.line] {
            text.push('\n');
            text.push_str(line);
        }
        let line = &lines[end.line];
        text.push('\n');
        text.push_str(&line[..byte_index(line, end.column)]);
        text
    }

    /// Put `position` inside the text and on the nearest grapheme boundary at or before it.
    fn clamp(&self, position: TextPosition) -> TextPosition {
        let line = position.line.min(self.state.lines.len() - 1);
        let text = &self.state.lines[line];
        TextPosition::new(line, boundary_at_or_before(text, position.column))
    }

    /// Put the caret at `position` with no selection.
    #[cfg(test)]
    pub(crate) fn set_cursor(&mut self, position: TextPosition) {
        self.state.cursor = self.clamp(position);
        self.state.anchor = None;
    }

    /// A deterministic description of every part of the state, for the agent-review snapshot.
    pub(crate) fn agent_review_snapshot(&self) -> serde_json::Value {
        let position = |position: TextPosition| {
            if self.multiline {
                serde_json::json!({"line": position.line, "column": position.column})
            } else {
                serde_json::json!(position.column)
            }
        };
        let state = |state: &EditorState| {
            let text = if self.multiline {
                serde_json::json!(state.lines)
            } else {
                serde_json::json!(state.lines[0])
            };
            serde_json::json!({
                "value": text,
                "cursor": position(state.cursor),
                "anchor": state.anchor.map(position),
            })
        };
        let group = match self.group {
            None => serde_json::Value::Null,
            Some(EditGroup::Typing { after_space }) => {
                serde_json::json!({"typing": {"after_space": after_space}})
            }
            Some(EditGroup::DeleteBackward) => serde_json::json!("delete_backward"),
            Some(EditGroup::DeleteForward) => serde_json::json!("delete_forward"),
        };
        let mut snapshot = state(&self.state);
        let object = snapshot
            .as_object_mut()
            .expect("an editor state snapshot is an object");
        object.insert(
            "undo".to_owned(),
            serde_json::Value::Array(self.undo.iter().map(state).collect()),
        );
        object.insert(
            "redo".to_owned(),
            serde_json::Value::Array(self.redo.iter().map(state).collect()),
        );
        object.insert("group".to_owned(), group);
        if self.multiline {
            object.insert(
                "goal_column".to_owned(),
                serde_json::json!(self.goal_column),
            );
        }
        snapshot
    }
}

/// The byte offset of the character at `column` in `line`.
fn byte_index(line: &str, column: usize) -> usize {
    line.char_indices()
        .nth(column)
        .map_or(line.len(), |(index, _)| index)
}

/// The character offset of every grapheme boundary of `line`, the end included.
fn boundaries(line: &str) -> impl Iterator<Item = usize> + '_ {
    let mut column = 0;
    std::iter::once(0).chain(line.graphemes(true).map(move |grapheme| {
        column += grapheme.chars().count();
        column
    }))
}

fn previous_boundary(line: &str, column: usize) -> usize {
    boundaries(line)
        .take_while(|boundary| *boundary < column)
        .last()
        .unwrap_or(0)
}

fn next_boundary(line: &str, column: usize) -> usize {
    boundaries(line)
        .find(|boundary| *boundary > column)
        .unwrap_or_else(|| line.chars().count())
}

/// The largest grapheme boundary of `line` at or before `column`.
pub(crate) fn boundary_at_or_before(line: &str, column: usize) -> usize {
    boundaries(line)
        .take_while(|boundary| *boundary <= column)
        .last()
        .unwrap_or(0)
}

/// The smallest grapheme boundary of `line` at or after `column`.
fn boundary_at_or_after(line: &str, column: usize) -> usize {
    boundaries(line)
        .find(|boundary| *boundary >= column)
        .unwrap_or_else(|| line.chars().count())
}

/// Report whether one word segment is a word, not a run of spaces or ASCII punctuation.
fn is_word(segment: &str) -> bool {
    segment
        .chars()
        .any(|character| !character.is_whitespace() && !character.is_ascii_punctuation())
}

/// The character offset of the start of every word in `line`.
fn word_starts(line: &str) -> impl Iterator<Item = usize> + '_ {
    let mut column = 0;
    line.split_word_bounds().filter_map(move |segment| {
        let start = column;
        column += segment.chars().count();
        is_word(segment).then_some(start)
    })
}

/// The start of the last word that starts before `column`.
fn word_start_before(line: &str, column: usize) -> Option<usize> {
    word_starts(line).take_while(|start| *start < column).last()
}

/// The start of the first word that starts after `column`.
fn word_start_after(line: &str, column: usize) -> Option<usize> {
    word_starts(line).find(|start| *start > column)
}

/// The span of the word segment at `column`, for a double click.
///
/// A click on a space or a punctuation mark selects that run of segments, so a double click
/// always selects something next to the pointer.
fn word_span_at(line: &str, column: usize) -> (usize, usize) {
    let mut start = 0;
    let mut spans = Vec::new();
    for segment in line.split_word_bounds() {
        let end = start + segment.chars().count();
        spans.push((start, end, is_word(segment)));
        start = end;
    }
    let Some(index) = spans
        .iter()
        .position(|(start, end, _)| *start <= column && column < *end)
        .or_else(|| spans.len().checked_sub(1))
    else {
        return (0, 0);
    };
    let (mut first, mut last) = (index, index);
    let word = spans[index].2;
    if !word {
        while first > 0 && !spans[first - 1].2 {
            first -= 1;
        }
        while last + 1 < spans.len() && !spans[last + 1].2 {
            last += 1;
        }
    }
    (spans[first].0, spans[last].1)
}

/// The cells from the start of `line` to the character at `column`, with tab stops.
pub(crate) fn display_column(line: &str, column: usize) -> usize {
    let mut cells = 0_usize;
    let mut characters = 0_usize;
    for grapheme in line.graphemes(true) {
        if characters >= column {
            break;
        }
        characters += grapheme.chars().count();
        cells += grapheme_cells(grapheme, cells);
    }
    cells
}

/// The cells one grapheme takes when it starts at `cells`: a Tab goes to the next tab stop.
pub(crate) fn grapheme_cells(grapheme: &str, cells: usize) -> usize {
    if grapheme == "\t" {
        TAB_WIDTH - cells % TAB_WIDTH
    } else {
        grapheme.width()
    }
}

/// The largest grapheme boundary of `line` whose display column is at most `cells`.
fn column_at_cells(line: &str, cells: usize) -> usize {
    let mut used = 0_usize;
    let mut column = 0_usize;
    for grapheme in line.graphemes(true) {
        let next = used + grapheme_cells(grapheme, used);
        if next > cells {
            break;
        }
        used = next;
        column += grapheme.chars().count();
    }
    column
}

/// The internal clipboard of one terminal session.
///
/// Ctrl+C and Ctrl+X put the selected text here, and Ctrl+V pastes it back. Each copy also asks
/// the terminal to put the text on the system clipboard with OSC 52, so a paste in another
/// program gets it too.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Clipboard {
    text: String,
    /// The text that the terminal has not received yet.
    pending_export: Option<String>,
}

impl Clipboard {
    pub(crate) fn copy(&mut self, text: String) {
        self.pending_export = Some(text.clone());
        self.text = text;
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Take the copy that the terminal must still send to the system clipboard.
    pub(crate) fn take_export(&mut self) -> Option<String> {
        self.pending_export.take()
    }

    pub(crate) fn agent_review_snapshot(&self) -> serde_json::Value {
        serde_json::json!({"text": self.text, "pending_export": self.pending_export})
    }
}

/// The OSC 52 sequence that asks a terminal to put `text` on the system clipboard.
pub(crate) fn osc52_copy(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        let triple = (u32::from(first) << 16)
            | (u32::from(second.unwrap_or(0)) << 8)
            | u32::from(third.unwrap_or(0));
        for (index, present) in [true, true, second.is_some(), third.is_some()]
            .into_iter()
            .enumerate()
        {
            if present {
                let sextet = (triple >> (18 - 6 * index)) & 0x3f;
                encoded.push(char::from(ALPHABET[sextet as usize]));
            } else {
                encoded.push('=');
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    fn typed(editor: &mut TextEditor, text: &str) {
        for character in text.chars() {
            assert_eq!(
                editor.apply(EditCommand::Insert(character)),
                EditResponse::Changed
            );
        }
    }

    fn press(editor: &mut TextEditor, code: KeyCode, modifiers: KeyModifiers) -> EditResponse {
        editor.handle_key(&key(code, modifiers))
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
    const CONTROL: KeyModifiers = KeyModifiers::CONTROL;
    const ALT: KeyModifiers = KeyModifiers::ALT;

    // Ways this can fail: one advertised chord maps to another operation, or to nothing; a chord
    // that a screen owns reaches the editor; Shift does not turn a move into a selection.
    #[test]
    fn every_advertised_chord_maps_to_its_one_operation() {
        let moved = |motion| {
            Some(EditCommand::Move {
                motion,
                select: false,
            })
        };
        let selected = |motion| {
            Some(EditCommand::Move {
                motion,
                select: true,
            })
        };
        for (code, modifiers, expected) in [
            (KeyCode::Left, NONE, moved(Motion::CharLeft)),
            (KeyCode::Right, NONE, moved(Motion::CharRight)),
            (KeyCode::Home, NONE, moved(Motion::LineStart)),
            (KeyCode::End, NONE, moved(Motion::LineEnd)),
            (KeyCode::Char('a'), CONTROL, moved(Motion::LineStart)),
            (KeyCode::Char('e'), CONTROL, moved(Motion::LineEnd)),
            (KeyCode::Left, CONTROL, moved(Motion::WordLeft)),
            (KeyCode::Right, CONTROL, moved(Motion::WordRight)),
            (KeyCode::Left, ALT, moved(Motion::WordLeft)),
            (KeyCode::Right, ALT, moved(Motion::WordRight)),
            (KeyCode::Char('b'), ALT, moved(Motion::WordLeft)),
            (KeyCode::Char('f'), ALT, moved(Motion::WordRight)),
            (KeyCode::Home, CONTROL, moved(Motion::TextStart)),
            (KeyCode::End, CONTROL, moved(Motion::TextEnd)),
            (KeyCode::Left, SHIFT, selected(Motion::CharLeft)),
            (KeyCode::Right, SHIFT, selected(Motion::CharRight)),
            (KeyCode::Home, SHIFT, selected(Motion::LineStart)),
            (KeyCode::End, SHIFT, selected(Motion::LineEnd)),
            (KeyCode::Left, CONTROL | SHIFT, selected(Motion::WordLeft)),
            (KeyCode::Right, CONTROL | SHIFT, selected(Motion::WordRight)),
            (KeyCode::Left, ALT | SHIFT, selected(Motion::WordLeft)),
            (KeyCode::Right, ALT | SHIFT, selected(Motion::WordRight)),
            (KeyCode::Char('B'), ALT | SHIFT, selected(Motion::WordLeft)),
            (KeyCode::Char('F'), ALT | SHIFT, selected(Motion::WordRight)),
            (KeyCode::Backspace, NONE, Some(EditCommand::DeleteBackward)),
            (
                KeyCode::Char('h'),
                CONTROL,
                Some(EditCommand::DeleteBackward),
            ),
            (KeyCode::Delete, NONE, Some(EditCommand::DeleteForward)),
            (
                KeyCode::Char('d'),
                CONTROL,
                Some(EditCommand::DeleteForward),
            ),
            (
                KeyCode::Char('w'),
                CONTROL,
                Some(EditCommand::DeleteWordBackward),
            ),
            (
                KeyCode::Backspace,
                ALT,
                Some(EditCommand::DeleteWordBackward),
            ),
            (
                KeyCode::Backspace,
                CONTROL,
                Some(EditCommand::DeleteWordBackward),
            ),
            (
                KeyCode::Char('d'),
                ALT,
                Some(EditCommand::DeleteWordForward),
            ),
            (
                KeyCode::Delete,
                CONTROL,
                Some(EditCommand::DeleteWordForward),
            ),
            (KeyCode::Delete, ALT, Some(EditCommand::DeleteWordForward)),
            (
                KeyCode::Char('u'),
                CONTROL,
                Some(EditCommand::DeleteToLineStart),
            ),
            (
                KeyCode::Char('k'),
                CONTROL,
                Some(EditCommand::DeleteToLineEnd),
            ),
            (KeyCode::Char('x'), CONTROL, Some(EditCommand::Cut)),
            (KeyCode::Char('z'), CONTROL, Some(EditCommand::Undo)),
            (KeyCode::Char('y'), CONTROL, Some(EditCommand::Redo)),
            (KeyCode::Char('z'), CONTROL | SHIFT, Some(EditCommand::Redo)),
            (KeyCode::Char('Z'), CONTROL | SHIFT, Some(EditCommand::Redo)),
            (KeyCode::Char('x'), NONE, Some(EditCommand::Insert('x'))),
            (KeyCode::Char('X'), SHIFT, Some(EditCommand::Insert('X'))),
            (KeyCode::Char('界'), NONE, Some(EditCommand::Insert('界'))),
            (
                KeyCode::Char('@'),
                CONTROL | ALT,
                Some(EditCommand::Insert('@')),
            ),
        ] {
            assert_eq!(
                edit_command(&key(code, modifiers), false),
                expected,
                "{code:?} {modifiers:?}"
            );
            assert_eq!(
                edit_command(&key(code, modifiers), true),
                expected,
                "multi-line {code:?} {modifiers:?}"
            );
        }
        for (code, modifiers) in [
            (KeyCode::Enter, NONE),
            (KeyCode::Tab, NONE),
            (KeyCode::BackTab, SHIFT),
            (KeyCode::Esc, NONE),
            (KeyCode::F(2), NONE),
            (KeyCode::PageUp, NONE),
            (KeyCode::Up, NONE),
            (KeyCode::Down, NONE),
            (KeyCode::Char('c'), CONTROL),
            (KeyCode::Char('v'), CONTROL),
            (KeyCode::Char('s'), CONTROL),
            (KeyCode::Char('r'), CONTROL),
            (KeyCode::Char('o'), CONTROL),
            (KeyCode::Char('n'), CONTROL),
            (KeyCode::Char('t'), CONTROL),
            (KeyCode::Char('p'), CONTROL),
            (KeyCode::Char('x'), ALT),
            (KeyCode::Char('b'), CONTROL | ALT),
            (KeyCode::Char('a'), KeyModifiers::SUPER),
        ] {
            assert_eq!(
                edit_command(&key(code, modifiers), false),
                None,
                "a single-line box took {code:?} {modifiers:?}"
            );
        }
        assert_eq!(
            edit_command(&key(KeyCode::Enter, NONE), true),
            Some(EditCommand::NewLine)
        );
        assert_eq!(
            edit_command(&key(KeyCode::Up, NONE), true),
            moved(Motion::LineUp)
        );
        assert_eq!(
            edit_command(&key(KeyCode::Down, SHIFT), true),
            selected(Motion::LineDown)
        );
        let mut released = key(KeyCode::Char('x'), NONE);
        released.kind = KeyEventKind::Release;
        assert_eq!(edit_command(&released, false), None);
        assert!(is_copy_key(&key(KeyCode::Char('c'), CONTROL)));
        assert!(!is_copy_key(&key(KeyCode::Char('c'), NONE)));
        assert!(is_paste_key(&key(KeyCode::Char('v'), CONTROL)));
        assert!(!is_paste_key(&key(KeyCode::Char('v'), CONTROL | ALT)));
        assert!(is_cut_key(&key(KeyCode::Char('x'), CONTROL)));
        assert!(!is_cut_key(&key(KeyCode::Char('x'), ALT)));
    }

    // Ways this can fail: a word move stops inside a word or skips one; the line moves leave the
    // line; a move with no room reports a change.
    #[test]
    fn moves_follow_words_lines_and_graphemes() {
        let mut editor = TextEditor::single_line("one two three");
        let at = |editor: &mut TextEditor, code, modifiers, column| {
            assert_eq!(press(editor, code, modifiers), EditResponse::Consumed);
            assert_eq!(
                editor.cursor(),
                TextPosition::new(0, column),
                "{code:?} {modifiers:?}"
            );
        };
        at(&mut editor, KeyCode::Char('b'), ALT, 8);
        at(&mut editor, KeyCode::Char('b'), ALT, 4);
        at(&mut editor, KeyCode::Char('b'), ALT, 0);
        at(&mut editor, KeyCode::Char('b'), ALT, 0);
        at(&mut editor, KeyCode::Char('f'), ALT, 4);
        at(&mut editor, KeyCode::Char('f'), ALT, 8);
        at(&mut editor, KeyCode::Char('f'), ALT, 13);
        at(&mut editor, KeyCode::Char('f'), ALT, 13);
        at(&mut editor, KeyCode::Char('a'), CONTROL, 0);
        at(&mut editor, KeyCode::Left, NONE, 0);
        at(&mut editor, KeyCode::Right, NONE, 1);
        at(&mut editor, KeyCode::Char('e'), CONTROL, 13);

        // Words follow Unicode word boundaries: a slash separates two words, and a dot between
        // two letters joins them, so `two.py` is one word.
        let mut path = TextEditor::single_line("/tmp/one/two.py");
        at(&mut path, KeyCode::Left, CONTROL, 9);
        at(&mut path, KeyCode::Left, CONTROL, 5);
        at(&mut path, KeyCode::Home, NONE, 0);
        at(&mut path, KeyCode::Right, CONTROL, 1);

        let mut wide = TextEditor::single_line("日本🙂e\u{301}👨\u{200d}👩\u{200d}👧z");
        for column in [10, 5, 3, 2, 1, 0] {
            at(&mut wide, KeyCode::Left, NONE, column);
        }
        for column in [1, 2, 3, 5, 10, 11] {
            at(&mut wide, KeyCode::Right, NONE, column);
        }
    }

    // Ways this can fail: a delete removes half a grapheme, the wrong word, or the whole line for
    // Ctrl+U; a delete with no room reports a change.
    #[test]
    fn deletes_follow_characters_words_and_lines() {
        let check = |text: &str, cursor: usize, code, modifiers, expected: &str| {
            let mut editor = TextEditor::single_line(text);
            editor.state.cursor = TextPosition::new(0, cursor);
            let response = press(&mut editor, code, modifiers);
            assert_eq!(
                editor.value(),
                expected,
                "{text:?}@{cursor} {code:?} {modifiers:?}"
            );
            assert_eq!(
                response,
                if text == expected {
                    EditResponse::Consumed
                } else {
                    EditResponse::Changed
                }
            );
        };
        check("alpha beta", 10, KeyCode::Char('w'), CONTROL, "alpha ");
        check("alpha beta", 6, KeyCode::Backspace, ALT, "beta");
        check("alpha beta", 0, KeyCode::Char('d'), ALT, "beta");
        check("alpha beta", 0, KeyCode::Delete, CONTROL, "beta");
        check("alpha beta", 6, KeyCode::Delete, ALT, "alpha ");
        check("two x y z", 7, KeyCode::Char('u'), CONTROL, " z");
        check("keep z", 4, KeyCode::Char('k'), CONTROL, "keep");
        check("keeps", 4, KeyCode::Char('d'), CONTROL, "keep");
        check("keep", 4, KeyCode::Char('h'), CONTROL, "kee");
        check("keep", 4, KeyCode::Backspace, NONE, "kee");
        check("keep", 4, KeyCode::Delete, NONE, "keep");
        check("keep", 0, KeyCode::Backspace, NONE, "keep");
        check("keep", 0, KeyCode::Char('u'), CONTROL, "keep");
        check("keep", 4, KeyCode::Char('k'), CONTROL, "keep");
        check("xe\u{301}y", 3, KeyCode::Backspace, NONE, "xy");
        check("x👨\u{200d}👩\u{200d}👧y", 1, KeyCode::Delete, NONE, "xy");
        check("x日本", 3, KeyCode::Backspace, NONE, "x日");
    }

    // Ways this can fail: Shift does not keep the anchor; typing inserts beside the selection; a
    // delete chord with a selection removes something else; a plain arrow does not end the
    // selection at its own side.
    #[test]
    fn a_selection_is_replaced_by_typing_and_removed_by_every_delete() {
        let mut editor = TextEditor::single_line("hello world");
        for _ in 0..5 {
            press(&mut editor, KeyCode::Left, SHIFT);
        }
        assert_eq!(editor.selected_text().as_deref(), Some("world"));
        typed(&mut editor, "there");
        assert_eq!(editor.value(), "hello there");
        assert_eq!(editor.selection(), None);

        for (code, modifiers) in [
            (KeyCode::Backspace, NONE),
            (KeyCode::Delete, NONE),
            (KeyCode::Char('w'), CONTROL),
            (KeyCode::Char('d'), ALT),
            (KeyCode::Char('u'), CONTROL),
            (KeyCode::Char('k'), CONTROL),
            (KeyCode::Char('h'), CONTROL),
            (KeyCode::Char('d'), CONTROL),
            (KeyCode::Char('x'), CONTROL),
        ] {
            let mut editor = TextEditor::single_line("abc def ghi");
            editor.state.cursor = TextPosition::new(0, 4);
            press(&mut editor, KeyCode::Right, SHIFT | CONTROL);
            assert_eq!(editor.selected_text().as_deref(), Some("def "));
            assert_eq!(press(&mut editor, code, modifiers), EditResponse::Changed);
            assert_eq!(editor.value(), "abc ghi", "{code:?} {modifiers:?}");
        }

        let mut collapse = TextEditor::single_line("abcd");
        press(&mut collapse, KeyCode::Left, SHIFT);
        press(&mut collapse, KeyCode::Left, SHIFT);
        assert_eq!(
            press(&mut collapse, KeyCode::Right, NONE),
            EditResponse::Consumed
        );
        assert_eq!(collapse.cursor().column, 4);
        assert_eq!(collapse.selection(), None);
        press(&mut collapse, KeyCode::Left, SHIFT);
        press(&mut collapse, KeyCode::Left, NONE);
        assert_eq!(collapse.cursor().column, 3);
        press(&mut collapse, KeyCode::Home, SHIFT);
        assert_eq!(collapse.selected_text().as_deref(), Some("abc"));
        press(&mut collapse, KeyCode::End, SHIFT);
        assert_eq!(collapse.selected_text().as_deref(), Some("d"));
        assert_eq!(
            press(&mut collapse, KeyCode::Char('x'), CONTROL),
            EditResponse::Changed
        );
        assert_eq!(collapse.value(), "abc");
        assert_eq!(
            press(&mut collapse, KeyCode::Char('x'), CONTROL),
            EditResponse::Consumed
        );
    }

    // Ways this can fail: one undo takes back one character of a typed word; undo skips the
    // selection it replaced; redo survives a new edit; a history step with nothing to take back
    // reports a change.
    #[test]
    fn undo_takes_back_one_word_or_one_command_and_redo_puts_it_back() {
        let mut editor = TextEditor::single_line("");
        typed(&mut editor, "copy me");
        assert_eq!(
            press(&mut editor, KeyCode::Char('z'), CONTROL),
            EditResponse::Changed
        );
        assert_eq!(editor.value(), "copy ");
        assert_eq!(
            press(&mut editor, KeyCode::Char('z'), CONTROL),
            EditResponse::Changed
        );
        assert_eq!(editor.value(), "");
        assert_eq!(
            press(&mut editor, KeyCode::Char('z'), CONTROL),
            EditResponse::Consumed
        );
        assert_eq!(
            press(&mut editor, KeyCode::Char('y'), CONTROL),
            EditResponse::Changed
        );
        assert_eq!(editor.value(), "copy ");
        assert_eq!(
            press(&mut editor, KeyCode::Char('y'), CONTROL),
            EditResponse::Changed
        );
        assert_eq!(editor.value(), "copy me");
        assert_eq!(
            press(&mut editor, KeyCode::Char('y'), CONTROL),
            EditResponse::Consumed
        );

        press(&mut editor, KeyCode::Home, SHIFT);
        typed(&mut editor, "X");
        assert_eq!(editor.value(), "X");
        press(&mut editor, KeyCode::Char('z'), CONTROL);
        assert_eq!(editor.value(), "copy me");
        assert_eq!(editor.selected_text().as_deref(), Some("copy me"));
        press(&mut editor, KeyCode::End, NONE);
        typed(&mut editor, "!");
        assert_eq!(
            press(&mut editor, KeyCode::Char('y'), CONTROL),
            EditResponse::Consumed
        );
        assert_eq!(editor.value(), "copy me!");

        let mut deleted = TextEditor::single_line("abc");
        press(&mut deleted, KeyCode::Backspace, NONE);
        press(&mut deleted, KeyCode::Backspace, NONE);
        press(&mut deleted, KeyCode::Char('z'), CONTROL);
        assert_eq!(deleted.value(), "abc");

        let mut limited = TextEditor::single_line("");
        for index in 0..HISTORY_LIMIT + 5 {
            limited.set_text(&index.to_string());
        }
        assert_eq!(limited.undo.len(), HISTORY_LIMIT);
    }

    // Ways this can fail: a paste keeps a later line in a single-line box; a CRLF keeps its CR; a
    // paste does not replace the selection or is more than one undo step.
    #[test]
    fn a_paste_follows_the_box_shape_and_is_one_undo_step() {
        assert_eq!(paste_text("Bob\tX\nCarol", false), "Bob\tX");
        assert_eq!(paste_text("Bob\r\nCarol", false), "Bob");
        assert_eq!(paste_text("Bob\rCarol", false), "Bob");
        assert_eq!(paste_text("a\r\nb\rc\nd", true), "a\nb\nc\nd");
        assert!(matches!(paste_text("plain", true), Cow::Borrowed(_)));

        let mut single = TextEditor::single_line("ab");
        assert_eq!(single.paste("X\nY"), EditResponse::Changed);
        assert_eq!(single.value(), "abX");
        assert_eq!(single.paste("\nY"), EditResponse::Consumed);
        press(&mut single, KeyCode::Home, SHIFT);
        single.paste("Z");
        assert_eq!(single.value(), "Z");
        press(&mut single, KeyCode::Char('z'), CONTROL);
        assert_eq!(single.value(), "abX");

        let mut multi = TextEditor::multi_line("one");
        multi.paste("\r\ntwo\r\nthree");
        assert_eq!(multi.text(), "one\ntwo\nthree");
        assert_eq!(multi.cursor(), TextPosition::new(2, 5));
        press(&mut multi, KeyCode::Char('z'), CONTROL);
        assert_eq!(multi.text(), "one");

        let large = "a".repeat(3_000);
        let mut long = TextEditor::single_line("");
        long.paste(&large);
        assert_eq!(long.value().len(), 3_000);

        // A typed line feed never enters a single-line box, and it leaves no undo step.
        let mut typed_break = TextEditor::single_line("ab");
        assert_eq!(
            press(&mut typed_break, KeyCode::Char('\n'), NONE),
            EditResponse::Consumed
        );
        assert_eq!(typed_break.value(), "ab");
        assert_eq!(
            press(&mut typed_break, KeyCode::Char('z'), CONTROL),
            EditResponse::Consumed
        );
    }

    // Ways this can fail: the multi-line box ignores Enter or the vertical arrows; a vertical move
    // loses its column on a short line; the first and last lines do not report the boundary;
    // word moves and deletes stop at a line break instead of crossing it.
    #[test]
    fn the_multi_line_box_moves_and_deletes_across_lines() {
        let mut editor = TextEditor::multi_line("first line\nab\nthird line");
        assert_eq!(editor.cursor(), TextPosition::new(2, 10));
        assert_eq!(
            press(&mut editor, KeyCode::Down, NONE),
            EditResponse::Boundary
        );
        press(&mut editor, KeyCode::Up, NONE);
        assert_eq!(editor.cursor(), TextPosition::new(1, 2));
        press(&mut editor, KeyCode::Up, NONE);
        assert_eq!(editor.cursor(), TextPosition::new(0, 10));
        assert_eq!(
            press(&mut editor, KeyCode::Up, NONE),
            EditResponse::Boundary
        );
        press(&mut editor, KeyCode::Up, SHIFT);
        assert_eq!(editor.cursor(), TextPosition::new(0, 0));
        assert_eq!(editor.selected_text().as_deref(), Some("first line"));
        assert_eq!(
            press(&mut editor, KeyCode::Up, NONE),
            EditResponse::Consumed
        );
        assert_eq!(editor.selection(), None);

        press(&mut editor, KeyCode::End, CONTROL);
        assert_eq!(editor.cursor(), TextPosition::new(2, 10));
        press(&mut editor, KeyCode::Home, NONE);
        press(&mut editor, KeyCode::Left, CONTROL);
        assert_eq!(editor.cursor(), TextPosition::new(1, 2));
        press(&mut editor, KeyCode::Right, CONTROL);
        assert_eq!(editor.cursor(), TextPosition::new(2, 0));
        press(&mut editor, KeyCode::Backspace, NONE);
        assert_eq!(editor.text(), "first line\nabthird line");
        press(&mut editor, KeyCode::Enter, NONE);
        assert_eq!(editor.text(), "first line\nab\nthird line");
        press(&mut editor, KeyCode::Home, NONE);
        press(&mut editor, KeyCode::Char('w'), CONTROL);
        assert_eq!(editor.text(), "first line\nabthird line");
        press(&mut editor, KeyCode::Up, NONE);
        press(&mut editor, KeyCode::End, NONE);
        press(&mut editor, KeyCode::Delete, CONTROL);
        assert_eq!(editor.text(), "first lineabthird line");
        press(&mut editor, KeyCode::Home, CONTROL);
        press(&mut editor, KeyCode::End, SHIFT | CONTROL);
        assert_eq!(
            editor.selected_text().as_deref(),
            Some("first lineabthird line")
        );

        let mut wide = TextEditor::multi_line("日本語x\nabcdef");
        wide.state.cursor = TextPosition::new(1, 5);
        press(&mut wide, KeyCode::Up, NONE);
        assert_eq!(wide.cursor(), TextPosition::new(0, 2));
        press(&mut wide, KeyCode::Down, NONE);
        assert_eq!(wide.cursor(), TextPosition::new(1, 5));

        let mut cut = TextEditor::multi_line("a\nbc\nd");
        cut.state.cursor = TextPosition::new(0, 1);
        press(&mut cut, KeyCode::Down, SHIFT);
        press(&mut cut, KeyCode::Down, SHIFT);
        assert_eq!(cut.selected_text().as_deref(), Some("\nbc\nd"));
        press(&mut cut, KeyCode::Char('x'), CONTROL);
        assert_eq!(cut.text(), "a");

        // Shift+Down on the last line selects to the end of that line, and Right at the end of a
        // line goes on to the start of the next line.
        let mut last = TextEditor::multi_line("ab\ncd");
        last.state.cursor = TextPosition::new(1, 1);
        press(&mut last, KeyCode::Down, SHIFT);
        assert_eq!(last.cursor(), TextPosition::new(1, 2));
        assert_eq!(last.selected_text().as_deref(), Some("d"));
        last.set_cursor(TextPosition::new(0, 2));
        press(&mut last, KeyCode::Right, NONE);
        assert_eq!(last.cursor(), TextPosition::new(1, 0));
    }

    // Ways this can fail: an edit joins the text on its two sides into one grapheme (an emoji, a
    // zero-width joiner, and a second emoji, or two regional indicators) and leaves the caret
    // inside that grapheme, so the next typed character splits it; undo or redo brings back a
    // caret inside a grapheme.
    #[test]
    fn an_edit_that_joins_two_graphemes_puts_the_caret_after_the_joined_one() {
        let woman = "\u{1f469}";
        let laptop = "\u{1f4bb}";
        let mut editor = TextEditor::single_line("");
        editor.paste(&format!("{woman}\u{200d} {laptop}"));
        press(&mut editor, KeyCode::Home, NONE);
        press(&mut editor, KeyCode::Right, NONE);
        assert_eq!(editor.cursor(), TextPosition::new(0, 2));
        assert_eq!(
            press(&mut editor, KeyCode::Delete, NONE),
            EditResponse::Changed
        );
        assert_eq!(editor.value(), format!("{woman}\u{200d}{laptop}"));
        assert_eq!(editor.cursor(), TextPosition::new(0, 3));
        typed(&mut editor, "x");
        assert_eq!(editor.value(), format!("{woman}\u{200d}{laptop}x"));
        press(&mut editor, KeyCode::Char('z'), CONTROL);
        assert_eq!(editor.cursor(), TextPosition::new(0, 3));
        press(&mut editor, KeyCode::Char('z'), CONTROL);
        assert_eq!(editor.value(), format!("{woman}\u{200d} {laptop}"));
        assert_eq!(editor.cursor(), TextPosition::new(0, 2));
        press(&mut editor, KeyCode::Char('y'), CONTROL);
        assert_eq!(editor.cursor(), TextPosition::new(0, 3));

        // Backspace joins two regional indicators into one flag.
        let mut flag = TextEditor::single_line("\u{1f1fa}x\u{1f1f8}");
        flag.set_cursor(TextPosition::new(0, 2));
        press(&mut flag, KeyCode::Backspace, NONE);
        assert_eq!(flag.value(), "\u{1f1fa}\u{1f1f8}");
        assert_eq!(flag.cursor(), TextPosition::new(0, 2));

        // A typed character joins the text after the caret.
        let mut typed_flag = TextEditor::single_line("\u{1f1f8}");
        typed_flag.set_cursor(TextPosition::new(0, 0));
        typed(&mut typed_flag, "\u{1f1fa}");
        assert_eq!(typed_flag.cursor(), TextPosition::new(0, 2));

        // Delete at the end of a line joins it with the next line.
        let mut lines = TextEditor::multi_line(&format!("{woman}\u{200d}\n{laptop}"));
        lines.set_cursor(TextPosition::new(0, 2));
        press(&mut lines, KeyCode::Delete, NONE);
        assert_eq!(lines.text(), format!("{woman}\u{200d}{laptop}"));
        assert_eq!(lines.cursor(), TextPosition::new(0, 3));
    }

    // Ways this can fail: typing, pasting, or Enter over a selection that holds the same text
    // records an undo step and a typing group although the text did not change, so the screen
    // sends an unchanged value and the next undo takes back nothing.
    #[test]
    fn the_same_text_over_a_selection_is_no_edit() {
        let unchanged = |editor: &TextEditor, text: &str, cursor: TextPosition| {
            assert_eq!(editor.text(), text);
            assert_eq!(editor.cursor(), cursor);
            assert_eq!(editor.selection(), None);
            let snapshot = editor.agent_review_snapshot();
            assert_eq!(snapshot["undo"], serde_json::json!([]), "{text:?}");
            assert_eq!(snapshot["group"], serde_json::Value::Null, "{text:?}");
        };
        let mut pasted = TextEditor::single_line("skit-new-000000");
        press(&mut pasted, KeyCode::Left, SHIFT);
        assert_eq!(pasted.selected_text().as_deref(), Some("0"));
        assert_eq!(pasted.paste("0"), EditResponse::Consumed);
        unchanged(&pasted, "skit-new-000000", TextPosition::new(0, 15));

        let mut typed_over = TextEditor::single_line("external");
        typed_over.set_cursor(TextPosition::new(0, 1));
        press(&mut typed_over, KeyCode::Left, SHIFT);
        assert_eq!(typed_over.selected_text().as_deref(), Some("e"));
        assert_eq!(
            press(&mut typed_over, KeyCode::Char('e'), NONE),
            EditResponse::Consumed
        );
        unchanged(&typed_over, "external", TextPosition::new(0, 1));
        typed(&mut typed_over, "x");
        assert_eq!(typed_over.value(), "exxternal");
        press(&mut typed_over, KeyCode::Char('z'), CONTROL);
        assert_eq!(typed_over.value(), "external");

        let mut line_break = TextEditor::multi_line("a\nb");
        line_break.set_cursor(TextPosition::new(0, 1));
        press(&mut line_break, KeyCode::Right, SHIFT);
        assert_eq!(line_break.selected_text().as_deref(), Some("\n"));
        assert_eq!(
            press(&mut line_break, KeyCode::Enter, NONE),
            EditResponse::Consumed
        );
        unchanged(&line_break, "a\nb", TextPosition::new(1, 0));
    }

    // Ways this can fail: a pointer press, a drag, or a double click keeps the column of an
    // earlier Up or Down, so the next vertical move goes to that old column instead of the
    // column the pointer chose.
    #[test]
    fn a_pointer_placement_resets_the_column_of_vertical_moves() {
        let mut pressed = TextEditor::multi_line("abcdef\nabcdef\nabcdef");
        press(&mut pressed, KeyCode::Up, NONE);
        assert_eq!(pressed.cursor(), TextPosition::new(1, 6));
        pressed.press_at(TextPosition::new(2, 2));
        press(&mut pressed, KeyCode::Up, NONE);
        assert_eq!(pressed.cursor(), TextPosition::new(1, 2));
        typed(&mut pressed, "X");
        assert_eq!(pressed.text(), "abcdef\nabXcdef\nabcdef");

        let mut dragged = TextEditor::multi_line("abcdef\nabcdef\nabcdef");
        press(&mut dragged, KeyCode::Up, NONE);
        dragged.drag_to(TextPosition::new(2, 2));
        press(&mut dragged, KeyCode::Up, NONE);
        assert_eq!(dragged.cursor(), TextPosition::new(1, 2));

        let mut clicked_twice = TextEditor::multi_line("abcdef\nabcdef\nab cdef");
        press(&mut clicked_twice, KeyCode::Up, NONE);
        assert_eq!(clicked_twice.cursor(), TextPosition::new(1, 6));
        clicked_twice.select_word_at(TextPosition::new(2, 1));
        assert_eq!(clicked_twice.selected_text().as_deref(), Some("ab"));
        press(&mut clicked_twice, KeyCode::Up, NONE);
        assert_eq!(clicked_twice.cursor(), TextPosition::new(1, 2));
    }

    // Ways this can fail: a press leaves an old selection; a drag selects from the wrong side; a
    // double click selects half a word or nothing on a space; a pointer position outside the text
    // or inside a grapheme is used as it is.
    #[test]
    fn pointer_presses_drags_and_double_clicks_select_whole_graphemes() {
        let mut editor = TextEditor::single_line("alpha beta  gamma");
        editor.press_at(TextPosition::new(0, 7));
        assert_eq!(editor.selection(), None);
        editor.drag_to(TextPosition::new(0, 2));
        assert_eq!(editor.selected_text().as_deref(), Some("pha b"));
        editor.select_word_at(TextPosition::new(0, 7));
        assert_eq!(editor.selected_text().as_deref(), Some("beta"));
        editor.select_word_at(TextPosition::new(0, 11));
        assert_eq!(editor.selected_text().as_deref(), Some("  "));
        editor.select_word_at(TextPosition::new(4, 99));
        assert_eq!(editor.selected_text().as_deref(), Some("gamma"));
        editor.press_at(TextPosition::new(3, 99));
        assert_eq!(editor.cursor(), TextPosition::new(0, 17));
        editor.drag_to(TextPosition::new(0, 99));
        assert_eq!(editor.selection(), None);

        let mut wide = TextEditor::single_line("e\u{301}x");
        wide.press_at(TextPosition::new(0, 1));
        assert_eq!(wide.cursor(), TextPosition::new(0, 0));

        let mut empty = TextEditor::single_line("");
        empty.select_word_at(TextPosition::new(0, 0));
        assert_eq!(empty.selection(), None);

        let mut dragged = TextEditor::single_line("abc");
        dragged.state.anchor = None;
        dragged.drag_to(TextPosition::new(0, 1));
        assert_eq!(dragged.selected_text().as_deref(), Some("bc"));

        // A double click on a punctuation mark or on the space after it selects the whole run of
        // punctuation and spaces between the two words.
        let mut punctuation = TextEditor::single_line("alpha, beta");
        punctuation.select_word_at(TextPosition::new(0, 5));
        assert_eq!(punctuation.selected_text().as_deref(), Some(", "));
        punctuation.select_word_at(TextPosition::new(0, 6));
        assert_eq!(punctuation.selected_text().as_deref(), Some(", "));
    }

    // Ways this can fail: the snapshot omits the history, the anchor, or the group, so two boxes
    // that behave differently snapshot the same; a sync keeps an old history.
    #[test]
    fn the_snapshot_covers_every_part_of_the_state() {
        let mut editor = TextEditor::single_line("ab");
        let base = editor.agent_review_snapshot();
        assert_eq!(
            base,
            serde_json::json!({
                "value": "ab", "cursor": 2, "anchor": null,
                "undo": [], "redo": [], "group": null,
            })
        );
        typed(&mut editor, "c");
        let typed_snapshot = editor.agent_review_snapshot();
        assert_eq!(typed_snapshot["undo"][0]["value"], "ab");
        assert_eq!(typed_snapshot["group"]["typing"]["after_space"], false);
        press(&mut editor, KeyCode::Left, SHIFT);
        assert_eq!(editor.agent_review_snapshot()["anchor"], 3);
        press(&mut editor, KeyCode::Backspace, NONE);
        press(&mut editor, KeyCode::Backspace, NONE);
        assert_eq!(editor.agent_review_snapshot()["group"], "delete_backward");
        press(&mut editor, KeyCode::Delete, NONE);
        press(&mut editor, KeyCode::Home, NONE);
        press(&mut editor, KeyCode::Delete, NONE);
        assert_eq!(editor.agent_review_snapshot()["group"], "delete_forward");
        press(&mut editor, KeyCode::Char('z'), CONTROL);
        assert_eq!(
            editor.agent_review_snapshot()["redo"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        editor.replace_text("new");
        assert_eq!(
            editor.agent_review_snapshot(),
            serde_json::json!({
                "value": "new", "cursor": 3, "anchor": null,
                "undo": [], "redo": [], "group": null,
            })
        );

        let mut multi = TextEditor::multi_line("a\nb");
        press(&mut multi, KeyCode::Up, NONE);
        let snapshot = multi.agent_review_snapshot();
        assert_eq!(snapshot["value"], serde_json::json!(["a", "b"]));
        assert_eq!(
            snapshot["cursor"],
            serde_json::json!({"line": 0, "column": 1})
        );
        assert_eq!(snapshot["goal_column"], 1);
        multi.replace_text("x\ny");
        assert_eq!(multi.text(), "x\ny");
        assert_eq!(
            multi.agent_review_snapshot()["value"],
            serde_json::json!(["x", "y"])
        );
    }

    // Ways this can fail: a whole-text replacement cannot be undone, or keeps a line break in a
    // single-line box.
    #[test]
    fn a_whole_text_replacement_is_one_undo_step() {
        let mut editor = TextEditor::single_line("old");
        assert_eq!(editor.set_text("old"), EditResponse::Consumed);
        assert_eq!(editor.set_text("new\nline"), EditResponse::Changed);
        assert_eq!(editor.value(), "new");
        press(&mut editor, KeyCode::Char('z'), CONTROL);
        assert_eq!(editor.value(), "old");
        let mut multi = TextEditor::multi_line("a");
        multi.set_text("b\nc");
        assert_eq!(multi.text(), "b\nc");
        assert_eq!(multi.cursor(), TextPosition::new(1, 1));
    }

    // Ways this can fail: the encoding drops padding or a byte; the copy is lost before the
    // terminal takes it; a second take repeats it.
    #[test]
    fn a_copy_reaches_the_clipboard_and_the_osc_52_sequence_once() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("copy me".as_bytes()), "Y29weSBtZQ==");
        assert_eq!(osc52_copy("日"), "\x1b]52;c;5pel\x07");

        let mut clipboard = Clipboard::default();
        assert_eq!(clipboard.take_export(), None);
        clipboard.copy("text".to_owned());
        assert_eq!(clipboard.text(), "text");
        assert_eq!(
            clipboard.agent_review_snapshot(),
            serde_json::json!({"text": "text", "pending_export": "text"})
        );
        assert_eq!(clipboard.take_export().as_deref(), Some("text"));
        assert_eq!(clipboard.take_export(), None);
        assert_eq!(clipboard.text(), "text");
    }

    // Ways this can fail: a tab stop or a wide character moves the display column; a column on a
    // wide character rounds up into the next cell.
    #[test]
    fn display_columns_count_cells_and_tab_stops() {
        assert_eq!(display_column("a\tb", 2), 4);
        assert_eq!(display_column("日本", 1), 2);
        assert_eq!(display_column("ab", 9), 2);
        assert_eq!(column_at_cells("日本", 3), 1);
        assert_eq!(column_at_cells("a\tb", 3), 1);
        assert_eq!(grapheme_cells("\t", 1), 3);
        assert_eq!(boundary_at_or_before("e\u{301}x", 1), 0);
        assert_eq!(word_span_at("", 3), (0, 0));
    }
}
