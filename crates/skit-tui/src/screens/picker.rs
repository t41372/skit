//! Shared list, candidate, and filesystem picker widgets.

use std::path::PathBuf;

use ratatui_core::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui_interact::components::{
    EntryType, FileEntry, FileExplorerState, ListPicker, ListPickerState, ListPickerStyle,
    ScrollableContentState,
};
use ratatui_widgets::paragraph::{Paragraph, Wrap};
use skit_i18n::{Locale, text};
use skit_ui::{ChoicePicker, PathPickerState, PathSelectionMode, PickerPurpose, PickerResult};
use tui_input::{Input as LineInput, InputRequest};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    footer::handle_footer_scroll,
    pointer::{ClickOutcome, ClickTracker, EditableGeometry},
    session::render_search_line_input,
    theme::{BOX_INDIGO, SELECT_BG, SELECT_FG, panel_block},
};

/// Mouse target in the complete prompt-variable picker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChoicePickerHit {
    /// Search input.
    Search,
    /// Toggle the complete selection, not only filtered rows.
    SelectAll,
    /// One filtered choice row.
    Row(usize),
    /// Publish the complete working set.
    Done,
    /// Discard the working set.
    Cancel,
}

/// One clickable choice-picker region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoicePickerHitRegion {
    /// Terminal rectangle.
    pub area: Rect,
    /// Typed target.
    pub target: ChoicePickerHit,
}

/// Responsive complete-choice geometry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChoicePickerGeometry {
    /// Search input.
    pub search: Rect,
    /// Filtered list viewport.
    pub rows: Rect,
    /// Keyboard-equivalent mouse targets.
    pub hits: Vec<ChoicePickerHitRegion>,
}

/// Result of one complete prompt-variable picker event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptCandidatePickerEvent {
    /// Ephemeral query, cursor, or selection state changed.
    Changed,
    /// Publish selected names in original source order.
    Accepted(Vec<String>),
    /// Close without publishing the isolated working set.
    Cancelled,
}

/// Complete searchable prompt-variable picker backed by mature input and list state.
#[derive(Debug)]
pub struct PromptCandidatePickerSession {
    picker: ChoicePicker<String>,
    query: LineInput,
    list: ListPickerState,
    visible_height: usize,
    footer_scroll: ScrollableContentState,
    footer_viewport: Rect,
    footer_visible_height: usize,
    click: ClickTracker<ChoicePickerHit>,
    query_editable: Option<EditableGeometry>,
}

impl PromptCandidatePickerSession {
    pub(crate) fn cancel_click(&mut self) {
        self.click.cancel();
    }

    /// Open one isolated working selection from `ReviewState::prompt_picker`.
    #[must_use]
    pub fn new(picker: ChoicePicker<String>) -> Self {
        let total = picker.visible_items().len();
        Self {
            picker,
            query: LineInput::default(),
            list: ListPickerState::new(total),
            visible_height: 1,
            footer_scroll: ScrollableContentState::default(),
            footer_viewport: Rect::default(),
            footer_visible_height: 0,
            click: ClickTracker::default(),
            query_editable: None,
        }
    }

    /// Current visible names in fuzzy rank order.
    #[must_use]
    pub fn visible_names(&self) -> Vec<&str> {
        self.picker
            .visible_items()
            .into_iter()
            .map(|item| item.id.as_str())
            .collect()
    }

    /// Dispatch keyboard, paste, or mouse through the mature widget state.
    #[must_use]
    pub fn handle_event(
        &mut self,
        event: Event,
        geometry: &ChoicePickerGeometry,
    ) -> Option<PromptCandidatePickerEvent> {
        match event {
            Event::Key(key) => {
                self.click.cancel();
                if key.kind == KeyEventKind::Release {
                    None
                } else {
                    self.handle_choice_key(key)
                }
            }
            Event::Paste(value) => {
                self.click.cancel();
                for character in value.chars() {
                    self.query.handle(InputRequest::InsertChar(character));
                }
                self.sync_choice_filter();
                Some(PromptCandidatePickerEvent::Changed)
            }
            Event::FocusGained | Event::FocusLost => {
                self.click.cancel();
                None
            }
            Event::Mouse(mouse)
                if geometry.rows.contains((mouse.column, mouse.row).into())
                    && matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    ) =>
            {
                self.click.cancel();
                if mouse.kind == MouseEventKind::ScrollUp {
                    self.list.select_prev();
                } else {
                    self.list.select_next();
                }
                self.list.ensure_visible(self.visible_height);
                Some(PromptCandidatePickerEvent::Changed)
            }
            Event::Mouse(mouse)
                if handle_footer_scroll(
                    &mut self.footer_scroll,
                    &mouse,
                    self.footer_viewport,
                    self.footer_visible_height,
                ) =>
            {
                Some(PromptCandidatePickerEvent::Changed)
            }
            Event::Mouse(mouse) => {
                let target = geometry
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                    .map(|hit| &hit.target);
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && matches!(target, Some(ChoicePickerHit::Search))
                    && let Some(editable) = self.query_editable
                {
                    let _ = editable.place_cursor(&mut self.query, mouse.column, mouse.row);
                }
                match self.click.update(&mouse, target) {
                    ClickOutcome::Armed => Some(PromptCandidatePickerEvent::Changed),
                    ClickOutcome::Activated(target) => self.handle_choice_hit(target),
                    ClickOutcome::Ignored => None,
                }
            }
            _ => None,
        }
    }

    fn handle_choice_key(&mut self, key: KeyEvent) -> Option<PromptCandidatePickerEvent> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('s') => Some(self.accept_choice_picker()),
                KeyCode::Char('a') => {
                    self.picker.select_all(true);
                    Some(PromptCandidatePickerEvent::Changed)
                }
                KeyCode::Char('n') => {
                    self.picker.select_all(false);
                    Some(PromptCandidatePickerEvent::Changed)
                }
                _ => None,
            };
        }
        match key.code {
            KeyCode::Up => {
                self.list.select_prev();
                self.list.ensure_visible(self.visible_height);
                Some(PromptCandidatePickerEvent::Changed)
            }
            KeyCode::Down => {
                self.list.select_next();
                self.list.ensure_visible(self.visible_height);
                Some(PromptCandidatePickerEvent::Changed)
            }
            KeyCode::Home => {
                self.list.select_first();
                self.list.ensure_visible(self.visible_height);
                Some(PromptCandidatePickerEvent::Changed)
            }
            KeyCode::End => {
                self.list.select_last();
                self.list.ensure_visible(self.visible_height);
                Some(PromptCandidatePickerEvent::Changed)
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::BackTab => {
                Some(PromptCandidatePickerEvent::Changed)
            }
            KeyCode::Char(' ') => self.toggle_current_choice(),
            KeyCode::Esc => Some(PromptCandidatePickerEvent::Cancelled),
            KeyCode::Backspace => {
                self.query.handle(InputRequest::DeletePrevChar);
                self.sync_choice_filter();
                Some(PromptCandidatePickerEvent::Changed)
            }
            KeyCode::Char(character) => {
                self.query.handle(InputRequest::InsertChar(character));
                self.sync_choice_filter();
                Some(PromptCandidatePickerEvent::Changed)
            }
            _ => None,
        }
    }

    fn handle_choice_hit(&mut self, target: ChoicePickerHit) -> Option<PromptCandidatePickerEvent> {
        match target {
            ChoicePickerHit::Search => Some(PromptCandidatePickerEvent::Changed),
            ChoicePickerHit::SelectAll => {
                self.picker.select_all(!self.picker.all_selected());
                Some(PromptCandidatePickerEvent::Changed)
            }
            ChoicePickerHit::Row(index) => {
                self.list.select(index);
                self.toggle_current_choice()
            }
            ChoicePickerHit::Done => Some(self.accept_choice_picker()),
            ChoicePickerHit::Cancel => Some(PromptCandidatePickerEvent::Cancelled),
        }
    }

    fn toggle_current_choice(&mut self) -> Option<PromptCandidatePickerEvent> {
        let id = self
            .picker
            .visible_items()
            .get(self.list.selected_index)?
            .id
            .clone();
        self.picker.toggle(&id);
        Some(PromptCandidatePickerEvent::Changed)
    }

    fn accept_choice_picker(&self) -> PromptCandidatePickerEvent {
        match self.picker.accept() {
            PickerResult::Many(selected) => PromptCandidatePickerEvent::Accepted(selected),
            PickerResult::One(selected) => PromptCandidatePickerEvent::Accepted(vec![selected]),
            PickerResult::Cancelled => PromptCandidatePickerEvent::Cancelled,
        }
    }

    fn sync_choice_filter(&mut self) {
        self.picker.set_query(self.query.value().trim());
        self.list.set_total(self.picker.visible_items().len());
        self.list.select_first();
        self.list.scroll = 0;
    }
}

/// Render the complete searchable prompt-variable picker.
pub fn render_prompt_candidate_picker(
    frame: &mut Frame,
    area: Rect,
    session: &mut PromptCandidatePickerSession,
    locale: Locale,
) -> ChoicePickerGeometry {
    session.query_editable = None;
    if area.is_empty() {
        session.visible_height = 0;
        session.footer_viewport = area;
        session.footer_visible_height = 0;
        return ChoicePickerGeometry {
            search: area,
            rows: area,
            hits: Vec::new(),
        };
    }
    let compact = area.height < 12 || area.width < 52;
    let outer = panel_block(
        text(locale, "Choose prompt variables").into_owned(),
        BOX_INDIGO,
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if inner.is_empty() {
        session.visible_height = 0;
        session.footer_viewport = inner;
        session.footer_visible_height = 0;
        return ChoicePickerGeometry {
            search: inner,
            rows: inner,
            hits: Vec::new(),
        };
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if compact {
            vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(2),
                Constraint::Length(1),
            ]
        })
        .split(inner);
    let search = chunks[0];
    let all = chunks[1];
    let rows = chunks[2];
    let footer = chunks[3];
    session.query_editable = render_search_line_input(
        frame,
        search,
        &session.query,
        &text(locale, "type to filter…"),
    );
    frame.render_widget(
        Paragraph::new(format!(
            "{} {}",
            if session.picker.all_selected() {
                "☑"
            } else {
                "☐"
            },
            text(locale, "Select all variables")
        )),
        all,
    );
    let visible = session.picker.visible_items();
    let labels = visible
        .iter()
        .map(|item| {
            format!(
                "{} {}",
                if session.picker.is_selected(&item.id) {
                    "☑"
                } else {
                    "☐"
                },
                item.search_text
            )
        })
        .collect::<Vec<_>>();
    session.list.set_total(labels.len());
    session.visible_height = usize::from(rows.height).max(1);
    session.list.ensure_visible(session.visible_height);
    if labels.is_empty() {
        frame.render_widget(
            Paragraph::new(text(locale, "No matching entries"))
                .style(Style::default().add_modifier(Modifier::DIM)),
            rows,
        );
    } else {
        frame.render_widget(
            ListPicker::new(&labels, &session.list).style(ListPickerStyle::arrow().bordered(false)),
            rows,
        );
    }
    let mut hits = vec![
        ChoicePickerHitRegion {
            area: search,
            target: ChoicePickerHit::Search,
        },
        ChoicePickerHitRegion {
            area: all,
            target: ChoicePickerHit::SelectAll,
        },
    ];
    hits.extend(render_choice_footer(frame, footer, locale, session));
    for index in session.list.scroll as usize
        ..labels
            .len()
            .min(session.list.scroll as usize + session.visible_height)
    {
        hits.push(ChoicePickerHitRegion {
            area: Rect::new(
                rows.x,
                rows.y.saturating_add(
                    u16::try_from(index.saturating_sub(session.list.scroll as usize))
                        .unwrap_or(u16::MAX),
                ),
                rows.width,
                1,
            ),
            target: ChoicePickerHit::Row(index),
        });
    }
    ChoicePickerGeometry { search, rows, hits }
}

fn render_choice_footer(
    frame: &mut Frame,
    area: Rect,
    locale: Locale,
    session: &mut PromptCandidatePickerSession,
) -> Vec<ChoicePickerHitRegion> {
    let done = format!("[Ctrl+S] {}", text(locale, "Done"));
    let cancel = format!("[Esc] {}", text(locale, "Cancel"));
    let items = if done
        .width()
        .saturating_add(cancel.width())
        .saturating_add(2)
        > usize::from(area.width)
    {
        vec![
            (cancel, ChoicePickerHit::Cancel),
            (done, ChoicePickerHit::Done),
        ]
    } else {
        vec![
            (done, ChoicePickerHit::Done),
            (cancel, ChoicePickerHit::Cancel),
        ]
    };
    let (positioned, rows, content_width) =
        scrollable_picker_footer(items, area, &mut session.footer_scroll);
    session.footer_visible_height = usize::from(area.height);
    session.footer_viewport = Rect::new(area.x, area.y, content_width, area.height);
    render_picker_footer_items(frame, area, positioned, rows, &session.footer_scroll)
        .into_iter()
        .map(|(area, target)| ChoicePickerHitRegion { area, target })
        .collect()
}

/// Mouse target returned by the filesystem renderer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilePickerHit {
    /// Search input.
    Search,
    /// Choose the current directory without hand-entering its path.
    CurrentDirectory,
    /// One mature-explorer row in visible order.
    Entry(usize),
    /// Navigate to the parent directory.
    Up,
    /// Toggle dotfiles.
    Hidden,
    /// Accept selected paths.
    Accept,
    /// Cancel.
    Cancel,
}

/// One clickable filesystem-picker region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilePickerHitRegion {
    /// Screen rectangle.
    pub area: Rect,
    /// Semantic target.
    pub target: FilePickerHit,
}

/// Geometry needed to route mouse events without re-deriving layout.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FilePickerGeometry {
    /// Search input rectangle.
    pub search: Rect,
    /// File-list viewport.
    pub rows: Rect,
    /// Clickable regions.
    pub hits: Vec<FilePickerHitRegion>,
}

/// Result of one terminal event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilePickerEvent {
    /// Widget state changed and needs a redraw.
    Changed,
    /// Accepted output paths after the frontend-neutral output policy.
    Accepted(Vec<PathBuf>),
    /// User cancelled.
    Cancelled,
}

/// Ephemeral session backed by `ratatui-interact::FileExplorerState`.
#[derive(Debug)]
pub struct FilePickerSession {
    contract: PathPickerState,
    explorer: FileExplorerState,
    query: LineInput,
    current_directory_focused: bool,
    visible_height: usize,
    io_error: Option<String>,
    footer_scroll: ScrollableContentState,
    footer_viewport: Rect,
    footer_visible_height: usize,
    click: ClickTracker<FilePickerHit>,
    query_editable: Option<EditableGeometry>,
}

impl FilePickerSession {
    pub(crate) fn cancel_click(&mut self) {
        self.click.cancel();
    }

    /// Open the nearest readable ancestor of the requested start directory.
    #[must_use]
    pub fn new(contract: PathPickerState) -> Self {
        let start = nearest_directory(contract.start_dir().to_path_buf());
        let mut explorer = FileExplorerState::new(start);
        explorer.show_hidden = contract.show_hidden() || contract.query().starts_with('.');
        let io_error = explorer.load_entries().err().map(|error| error.to_string());
        let has_real_entry = select_first_real_entry(&mut explorer);
        let query = LineInput::new(contract.query().to_owned());
        explorer.search_query = contract.query().trim().to_owned();
        if !explorer.search_query.is_empty() {
            apply_filter(&mut explorer);
        }
        let current_directory_focused = explorer.search_query.is_empty()
            && accepts_current_directory(contract.selection())
            && !has_real_entry;
        Self {
            contract,
            explorer,
            query,
            current_directory_focused,
            visible_height: 1,
            io_error,
            footer_scroll: ScrollableContentState::default(),
            footer_viewport: Rect::default(),
            footer_visible_height: 0,
            click: ClickTracker::default(),
            query_editable: None,
        }
    }

    /// Current directory.
    #[must_use]
    pub const fn current_dir(&self) -> &PathBuf {
        &self.explorer.current_dir
    }

    /// Mature explorer state for adapter-level inspection and tests.
    #[must_use]
    pub const fn explorer(&self) -> &FileExplorerState {
        &self.explorer
    }

    /// Last directory-read error, if any.
    #[must_use]
    pub fn io_error(&self) -> Option<&str> {
        self.io_error.as_deref()
    }

    /// Dispatch keyboard or mouse through the mature explorer state.
    #[must_use]
    pub fn handle_event(
        &mut self,
        event: Event,
        geometry: &FilePickerGeometry,
    ) -> Option<FilePickerEvent> {
        match event {
            Event::Key(key) => {
                self.click.cancel();
                if key.kind == KeyEventKind::Release {
                    None
                } else {
                    self.handle_key(key)
                }
            }
            Event::FocusGained | Event::FocusLost => {
                self.click.cancel();
                None
            }
            Event::Mouse(mouse)
                if geometry.rows.contains((mouse.column, mouse.row).into())
                    && matches!(
                        mouse.kind,
                        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    ) =>
            {
                self.click.cancel();
                self.handle_key(KeyEvent::new(
                    if mouse.kind == MouseEventKind::ScrollUp {
                        KeyCode::Up
                    } else {
                        KeyCode::Down
                    },
                    KeyModifiers::NONE,
                ))
            }
            Event::Mouse(mouse)
                if handle_footer_scroll(
                    &mut self.footer_scroll,
                    &mouse,
                    self.footer_viewport,
                    self.footer_visible_height,
                ) =>
            {
                Some(FilePickerEvent::Changed)
            }
            Event::Mouse(mouse) => {
                let target = geometry
                    .hits
                    .iter()
                    .rev()
                    .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                    .map(|hit| &hit.target);
                if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                    && matches!(target, Some(FilePickerHit::Search))
                    && let Some(editable) = self.query_editable
                {
                    let _ = editable.place_cursor(&mut self.query, mouse.column, mouse.row);
                }
                match self.click.update(&mouse, target) {
                    ClickOutcome::Armed => Some(FilePickerEvent::Changed),
                    ClickOutcome::Activated(target) => self.handle_hit(target),
                    ClickOutcome::Ignored => None,
                }
            }
            _ => None,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> Option<FilePickerEvent> {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('a') if self.contract.allow_multiple() => {
                    self.explorer.select_all();
                    Some(FilePickerEvent::Changed)
                }
                KeyCode::Char('n') if self.contract.allow_multiple() => {
                    self.explorer.select_none();
                    Some(FilePickerEvent::Changed)
                }
                KeyCode::Char('h') => self.handle_hit(FilePickerHit::Hidden),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Up => {
                if self.current_directory_available()
                    && (self.current_directory_focused
                        || first_real_visible_index(&self.explorer)
                            == Some(self.explorer.cursor_index))
                {
                    self.current_directory_focused = true;
                    self.explorer.scroll = 0;
                    return Some(FilePickerEvent::Changed);
                }
                self.explorer.cursor_up();
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Down => {
                if self.current_directory_focused {
                    if let Some(index) = first_real_visible_index(&self.explorer) {
                        self.current_directory_focused = false;
                        self.explorer.cursor_index = index;
                        self.explorer.ensure_visible(self.visible_height);
                    }
                    return Some(FilePickerEvent::Changed);
                }
                self.explorer.cursor_down();
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Home | KeyCode::PageUp => {
                if self.current_directory_available() {
                    self.current_directory_focused = true;
                    self.explorer.scroll = 0;
                    return Some(FilePickerEvent::Changed);
                }
                self.current_directory_focused = false;
                self.explorer.cursor_index = 0;
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::End | KeyCode::PageDown => {
                if let Some(index) = last_real_visible_index(&self.explorer) {
                    self.current_directory_focused = false;
                    self.explorer.cursor_index = index;
                } else if self.current_directory_available() {
                    self.current_directory_focused = true;
                    self.explorer.scroll = 0;
                    return Some(FilePickerEvent::Changed);
                } else {
                    self.current_directory_focused = false;
                    self.explorer.cursor_index = self.explorer.visible_count().saturating_sub(1);
                }
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Enter => self.activate_current(),
            KeyCode::Char(' ') if self.contract.allow_multiple() => {
                if !self.current_directory_focused {
                    self.explorer.toggle_selection();
                }
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Backspace if self.query.value().is_empty() => {
                self.go_up();
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Esc if self.query.value().is_empty() => Some(FilePickerEvent::Cancelled),
            KeyCode::Esc => {
                self.query.reset();
                self.sync_filter();
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Char(character) => {
                self.query.handle(InputRequest::InsertChar(character));
                self.sync_filter();
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Backspace => {
                self.query.handle(InputRequest::DeletePrevChar);
                self.sync_filter();
                Some(FilePickerEvent::Changed)
            }
            _ => None,
        }
    }

    fn handle_hit(&mut self, target: FilePickerHit) -> Option<FilePickerEvent> {
        match target {
            FilePickerHit::Search => Some(FilePickerEvent::Changed),
            FilePickerHit::CurrentDirectory if self.current_directory_available() => {
                self.accept_current_directory()
            }
            FilePickerHit::CurrentDirectory => None,
            FilePickerHit::Entry(index) => {
                if index >= self.explorer.visible_count() {
                    return None;
                }
                self.current_directory_focused = false;
                self.explorer.cursor_index = index;
                self.activate_current()
            }
            FilePickerHit::Up => {
                self.go_up();
                Some(FilePickerEvent::Changed)
            }
            FilePickerHit::Hidden => {
                self.contract.toggle_hidden();
                self.sync_filter();
                Some(FilePickerEvent::Changed)
            }
            FilePickerHit::Accept if self.contract.allow_multiple() => self.accept_selection(),
            FilePickerHit::Accept => self.activate_current(),
            FilePickerHit::Cancel => Some(FilePickerEvent::Cancelled),
        }
    }

    fn activate_current(&mut self) -> Option<FilePickerEvent> {
        if self.current_directory_focused && self.current_directory_available() {
            return self.accept_current_directory();
        }
        let entry = self.explorer.current_entry()?.clone();
        if entry.is_dir() {
            self.enter(entry.path);
            return Some(FilePickerEvent::Changed);
        }
        if !accepts_entry(self.contract.selection(), &entry) {
            return None;
        }
        if self.contract.allow_multiple() {
            self.explorer.toggle_selection();
            Some(FilePickerEvent::Changed)
        } else {
            Some(FilePickerEvent::Accepted(vec![
                self.contract.output_path(&entry.path),
            ]))
        }
    }

    fn accept_current_directory(&self) -> Option<FilePickerEvent> {
        accepts_current_directory(self.contract.selection()).then(|| {
            FilePickerEvent::Accepted(vec![self.contract.output_path(&self.explorer.current_dir)])
        })
    }

    fn current_directory_available(&self) -> bool {
        accepts_current_directory(self.contract.selection())
            && self.explorer.search_query.is_empty()
    }

    fn reset_empty_filter_focus(&mut self) {
        let has_real_entry = select_first_real_entry(&mut self.explorer);
        self.current_directory_focused = self.current_directory_available() && !has_real_entry;
        if self.current_directory_focused {
            self.explorer.scroll = 0;
        }
    }

    fn accept_selection(&self) -> Option<FilePickerEvent> {
        let mut selected = self
            .explorer
            .selected_files
            .iter()
            .map(|path| self.contract.output_path(path))
            .collect::<Vec<_>>();
        selected.sort();
        (!selected.is_empty()).then_some(FilePickerEvent::Accepted(selected))
    }

    fn enter(&mut self, path: PathBuf) {
        self.explorer.current_dir = path;
        self.query.reset();
        self.contract.set_query("");
        self.reload_entries();
    }

    fn go_up(&mut self) {
        let Some(parent) = self.explorer.current_dir.parent().map(PathBuf::from) else {
            return;
        };
        self.explorer.current_dir = parent;
        self.query.reset();
        self.contract.set_query("");
        self.reload_entries();
    }

    fn sync_filter(&mut self) {
        self.contract.set_query(self.query.value());
        let query = self.query.value().trim().to_owned();
        let show_hidden = self.contract.show_hidden() || query.starts_with('.');
        if self.explorer.show_hidden != show_hidden {
            self.explorer.show_hidden = show_hidden;
            self.io_error = self
                .explorer
                .load_entries()
                .err()
                .map(|error| error.to_string());
        }
        self.explorer.search_query = query;
        apply_filter(&mut self.explorer);
        if self.explorer.search_query.is_empty() {
            self.reset_empty_filter_focus();
        } else {
            self.current_directory_focused = false;
        }
    }

    fn reload_entries(&mut self) {
        self.explorer.show_hidden = self.contract.show_hidden();
        self.io_error = self
            .explorer
            .load_entries()
            .err()
            .map(|error| error.to_string());
        self.explorer.search_query.clear();
        self.reset_empty_filter_focus();
    }
}

/// Render one localized filesystem browser. The dependency owns traversal and cursor state;
/// this adapter owns labels because the dependency's built-in footer is English-only.
pub fn render_file_picker(
    frame: &mut Frame,
    area: Rect,
    session: &mut FilePickerSession,
    locale: Locale,
) -> FilePickerGeometry {
    session.query_editable = None;
    if area.is_empty() {
        session.visible_height = 0;
        session.footer_viewport = area;
        session.footer_visible_height = 0;
        return FilePickerGeometry {
            search: area,
            rows: area,
            hits: Vec::new(),
        };
    }
    let compact = area.height < 12 || area.width < 52;
    let outer = panel_block(
        match session.contract.purpose() {
            PickerPurpose::Source => text(locale, "Source path").into_owned(),
            PickerPurpose::Argument => text(locale, "Arguments").into_owned(),
            PickerPurpose::WorkingDirectory => text(locale, "Working directory").into_owned(),
            PickerPurpose::Configuration => text(locale, "Settings").into_owned(),
        },
        BOX_INDIGO,
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    if inner.is_empty() {
        session.visible_height = 0;
        session.footer_viewport = inner;
        session.footer_visible_height = 0;
        return FilePickerGeometry {
            search: inner,
            rows: inner,
            hits: Vec::new(),
        };
    }
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if compact {
            vec![
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ]
        } else {
            vec![
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(2),
                Constraint::Length(2),
            ]
        })
        .split(inner);
    let search = chunks[0];
    let path_row = if compact { Rect::default() } else { chunks[1] };
    let rows = if compact { chunks[1] } else { chunks[2] };
    let footer = *chunks.last().unwrap_or(&Rect::default());
    session.query_editable =
        render_search_line_input(frame, search, &session.query, &text(locale, "Search"));
    if !compact {
        frame.render_widget(
            Paragraph::new(session.explorer.current_dir.display().to_string())
                .style(Style::default().add_modifier(Modifier::DIM)),
            path_row,
        );
    }
    let mut lines = Vec::new();
    let mut hits = vec![FilePickerHitRegion {
        area: search,
        target: FilePickerHit::Search,
    }];
    if session.current_directory_available() {
        let style = if session.current_directory_focused {
            Style::default()
                .fg(SELECT_FG)
                .bg(SELECT_BG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(
            Line::from(vec![
                Span::styled("📂 ", Style::default().add_modifier(Modifier::DIM)),
                Span::raw(text(locale, "(use this directory)").into_owned()),
            ])
            .style(style),
        );
        hits.push(FilePickerHitRegion {
            area: Rect::new(rows.x, rows.y, rows.width, 1),
            target: FilePickerHit::CurrentDirectory,
        });
    }
    let offset = usize::from(session.explorer.scroll);
    let row_capacity = usize::from(rows.height).saturating_sub(lines.len());
    session.visible_height = row_capacity.max(1);
    let visible = visible_entries(&session.explorer);
    for (display_index, entry) in visible.iter().enumerate().skip(offset).take(row_capacity) {
        let cursor =
            !session.current_directory_focused && display_index == session.explorer.cursor_index;
        let selected = session.explorer.selected_files.contains(&entry.path);
        let icon = match entry.entry_type {
            EntryType::Directory | EntryType::ParentDir => "▸",
            EntryType::Symlink { .. } => "↗",
            EntryType::File { .. } => "·",
        };
        let mark = if session.contract.allow_multiple() && entry.is_selectable() {
            if selected { "☑" } else { "☐" }
        } else {
            " "
        };
        lines.push(
            Line::from(format!("{mark} {icon} {}", entry.name)).style(if cursor {
                Style::default()
                    .fg(SELECT_FG)
                    .bg(SELECT_BG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            }),
        );
        let y = rows
            .y
            .saturating_add(u16::try_from(lines.len().saturating_sub(1)).unwrap_or(u16::MAX));
        hits.push(FilePickerHitRegion {
            area: Rect::new(rows.x, y, rows.width, 1),
            target: FilePickerHit::Entry(display_index),
        });
    }
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            text(locale, "No matching entries"),
            Style::default().add_modifier(Modifier::DIM),
        )));
    }
    if let Some(error) = &session.io_error {
        lines.push(Line::from(Span::styled(
            error,
            Style::default().fg(Color::Red),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows);
    hits.extend(render_file_footer(frame, footer, locale, session));
    FilePickerGeometry { search, rows, hits }
}

#[derive(Debug)]
struct PositionedPickerFooterItem<T> {
    label: String,
    target: T,
    row: usize,
    x: u16,
    width: u16,
}

fn render_file_footer(
    frame: &mut Frame,
    area: Rect,
    locale: Locale,
    session: &mut FilePickerSession,
) -> Vec<FilePickerHitRegion> {
    let items = vec![
        (
            format!("[Enter] {}", text(locale, "Select")),
            FilePickerHit::Accept,
        ),
        (
            format!("[Esc] {}", text(locale, "Cancel")),
            FilePickerHit::Cancel,
        ),
        (
            format!("[Backspace] {}", text(locale, "Back")),
            FilePickerHit::Up,
        ),
        ("[Ctrl+H] .".to_owned(), FilePickerHit::Hidden),
    ];
    let (positioned, rows, content_width) =
        scrollable_picker_footer(items, area, &mut session.footer_scroll);
    session.footer_visible_height = usize::from(area.height);
    session.footer_viewport = Rect::new(area.x, area.y, content_width, area.height);
    render_picker_footer_items(frame, area, positioned, rows, &session.footer_scroll)
        .into_iter()
        .map(|(area, target)| FilePickerHitRegion { area, target })
        .collect()
}

fn scrollable_picker_footer<T>(
    items: Vec<(String, T)>,
    area: Rect,
    scroll: &mut ScrollableContentState,
) -> (Vec<PositionedPickerFooterItem<T>>, usize, u16) {
    let (mut positioned, mut rows) = position_picker_footer_items(items, area.width);
    let mut content_width = area.width;
    if rows > usize::from(area.height) && area.width > 1 {
        content_width = area.width.saturating_sub(1);
        let items = positioned
            .into_iter()
            .map(|item| (item.label, item.target))
            .collect();
        (positioned, rows) = position_picker_footer_items(items, content_width);
    }
    scroll.set_lines(vec![String::new(); rows]);
    crate::viewport::Viewport::new(Rect::new(area.x, area.y, content_width, area.height), rows)
        .clamp_scroll(scroll);
    (positioned, rows, content_width)
}

fn render_picker_footer_items<T>(
    frame: &mut Frame,
    area: Rect,
    positioned: Vec<PositionedPickerFooterItem<T>>,
    rows: usize,
    scroll: &ScrollableContentState,
) -> Vec<(Rect, T)> {
    let visible_height = usize::from(area.height);
    let offset = scroll.scroll_offset();
    let end = offset.saturating_add(visible_height);
    let mut hits = Vec::new();
    for item in positioned
        .into_iter()
        .filter(|item| item.row >= offset && item.row < end)
    {
        let y = area
            .y
            .saturating_add(u16::try_from(item.row.saturating_sub(offset)).unwrap_or(u16::MAX));
        let chip_area = Rect::new(area.x.saturating_add(item.x), y, item.width, 1);
        frame.render_widget(
            Paragraph::new(item.label).style(Style::default().add_modifier(Modifier::DIM)),
            chip_area,
        );
        hits.push((chip_area, item.target));
    }
    if rows > visible_height {
        let indicator = if scroll.is_at_top() {
            "↓"
        } else if scroll.is_at_bottom(visible_height) {
            "↑"
        } else {
            "↕"
        };
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().add_modifier(Modifier::DIM)),
            Rect::new(area.right().saturating_sub(1), area.y, 1, 1),
        );
    }
    hits
}

fn position_picker_footer_items<T>(
    items: Vec<(String, T)>,
    width: u16,
) -> (Vec<PositionedPickerFooterItem<T>>, usize) {
    if items.is_empty() || width == 0 {
        return (Vec::new(), 0);
    }
    let mut row = 0_usize;
    let mut x = 0_u16;
    let mut positioned = Vec::with_capacity(items.len());
    for (label, target) in items {
        let desired = u16::try_from(label.width()).unwrap_or(u16::MAX).min(width);
        if x.saturating_add(desired) > width {
            row = row.saturating_add(1);
            x = 0;
        }
        positioned.push(PositionedPickerFooterItem {
            label,
            target,
            row,
            x,
            width: desired.min(width.saturating_sub(x)),
        });
        x = x.saturating_add(desired).saturating_add(1);
    }
    (positioned, row.saturating_add(1))
}

fn visible_entries(explorer: &FileExplorerState) -> Vec<&FileEntry> {
    explorer.filtered_indices.as_ref().map_or_else(
        || explorer.entries.iter().collect(),
        |indices| {
            indices
                .iter()
                .filter_map(|index| explorer.entries.get(*index))
                .collect()
        },
    )
}

fn accepts_entry(mode: PathSelectionMode, entry: &FileEntry) -> bool {
    match mode {
        PathSelectionMode::File => entry.is_selectable(),
        PathSelectionMode::Directory => entry.is_dir(),
        PathSelectionMode::FileOrDirectory => entry.is_selectable() || entry.is_dir(),
    }
}

fn accepts_current_directory(mode: PathSelectionMode) -> bool {
    matches!(
        mode,
        PathSelectionMode::Directory | PathSelectionMode::FileOrDirectory
    )
}

fn nearest_directory(mut path: PathBuf) -> PathBuf {
    while !path.is_dir() {
        if !path.pop() {
            return std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        }
    }
    path
}

fn apply_filter(explorer: &mut FileExplorerState) {
    explorer.update_filter();
    let Some(indices) = &mut explorer.filtered_indices else {
        return;
    };
    let query = explorer.search_query.to_lowercase();
    indices.sort_by_key(|index| {
        let entry = &explorer.entries[*index];
        let name = entry.name.to_lowercase();
        (!name.starts_with(&query), !entry.is_dir(), name)
    });
}

fn first_real_visible_index(explorer: &FileExplorerState) -> Option<usize> {
    visible_entries(explorer)
        .iter()
        .position(|entry| !matches!(entry.entry_type, EntryType::ParentDir))
}

fn last_real_visible_index(explorer: &FileExplorerState) -> Option<usize> {
    visible_entries(explorer)
        .iter()
        .rposition(|entry| !matches!(entry.entry_type, EntryType::ParentDir))
}

fn select_first_real_entry(explorer: &mut FileExplorerState) -> bool {
    if let Some(index) = first_real_visible_index(explorer) {
        explorer.cursor_index = index;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{MouseButton, MouseEvent};
    use skit_ui::{ChoicePicker, PickerItem, PickerMode};
    use tempfile::tempdir;

    use super::*;

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>()
    }

    fn control(character: char) -> Event {
        Event::Key(KeyEvent::new(
            KeyCode::Char(character),
            KeyModifiers::CONTROL,
        ))
    }

    fn mouse(area: Rect, kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn prompt_click(
        session: &mut PromptCandidatePickerSession,
        geometry: &ChoicePickerGeometry,
        area: Rect,
    ) -> Option<PromptCandidatePickerEvent> {
        assert_eq!(
            session.handle_event(
                mouse(area, MouseEventKind::Down(MouseButton::Left)),
                geometry,
            ),
            Some(PromptCandidatePickerEvent::Changed),
        );
        session.handle_event(mouse(area, MouseEventKind::Up(MouseButton::Left)), geometry)
    }

    fn file_click(
        session: &mut FilePickerSession,
        geometry: &FilePickerGeometry,
        area: Rect,
    ) -> Option<FilePickerEvent> {
        assert_eq!(
            session.handle_event(
                mouse(area, MouseEventKind::Down(MouseButton::Left)),
                geometry,
            ),
            Some(FilePickerEvent::Changed),
        );
        session.handle_event(mouse(area, MouseEventKind::Up(MouseButton::Left)), geometry)
    }

    #[test]
    fn prompt_picker_cancels_an_armed_row_when_filtering_changes_its_identity() {
        let picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![
                PickerItem::new("alpha".to_owned(), "alpha"),
                PickerItem::new("beta".to_owned(), "beta"),
            ],
            Vec::new(),
        );
        let mut session = PromptCandidatePickerSession::new(picker);
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut geometry = ChoicePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let old = geometry
            .hits
            .iter()
            .find(|hit| hit.target == ChoicePickerHit::Row(0))
            .expect("alpha row is visible")
            .area;
        assert_eq!(session.visible_names(), ["alpha", "beta"]);
        assert_eq!(
            session.handle_event(
                mouse(old, MouseEventKind::Down(MouseButton::Left)),
                &geometry,
            ),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('b')), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let replacement = geometry
            .hits
            .iter()
            .find(|hit| hit.target == ChoicePickerHit::Row(0))
            .expect("beta replaced alpha on the first row")
            .area;
        assert_eq!(replacement, old);
        assert_eq!(session.visible_names(), ["beta"]);
        assert_eq!(
            session.handle_event(
                mouse(replacement, MouseEventKind::Up(MouseButton::Left)),
                &geometry,
            ),
            None,
            "a release must not toggle a new item that reused the old visual row",
        );
        assert_eq!(
            session.handle_event(
                mouse(replacement, MouseEventKind::Down(MouseButton::Left)),
                &geometry,
            ),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(session.handle_event(Event::FocusLost, &geometry), None);
        assert_eq!(
            session.handle_event(
                mouse(replacement, MouseEventKind::Up(MouseButton::Left)),
                &geometry,
            ),
            None,
            "focus loss must cancel an armed picker row",
        );
    }

    #[test]
    fn file_picker_cancels_an_armed_entry_when_filtering_changes_its_identity() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), b"").unwrap();
        fs::write(dir.path().join("beta.txt"), b"").unwrap();
        let contract = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        let mut session = FilePickerSession::new(contract);
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        let mut geometry = FilePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let parent = geometry
            .hits
            .iter()
            .find(|hit| {
                matches!(hit.target, FilePickerHit::Entry(index) if visible_entries(session.explorer()).get(index).is_some_and(|entry| entry.name == ".."))
            })
            .expect("parent row is visible")
            .area;
        assert_eq!(
            session.handle_event(
                mouse(parent, MouseEventKind::Down(MouseButton::Left)),
                &geometry,
            ),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('a')), &geometry),
            Some(FilePickerEvent::Changed)
        );
        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let alpha = geometry
            .hits
            .iter()
            .find(|hit| {
                matches!(hit.target, FilePickerHit::Entry(index) if visible_entries(session.explorer()).get(index).is_some_and(|entry| entry.name == "alpha.txt"))
            })
            .expect("alpha file row replaced the parent")
            .area;
        assert_eq!(alpha, parent);
        assert_eq!(
            session.handle_event(
                mouse(alpha, MouseEventKind::Up(MouseButton::Left)),
                &geometry,
            ),
            None,
            "a release must not accept a new file that reused the old visual row",
        );

        for focus_event in [Event::FocusLost, Event::FocusGained] {
            assert_eq!(
                session.handle_event(
                    mouse(alpha, MouseEventKind::Down(MouseButton::Left)),
                    &geometry,
                ),
                Some(FilePickerEvent::Changed)
            );
            assert_eq!(session.handle_event(focus_event, &geometry), None);
            assert_eq!(
                session.handle_event(
                    mouse(alpha, MouseEventKind::Up(MouseButton::Left)),
                    &geometry,
                ),
                None,
                "a focus transition left a stale file row armed",
            );
        }
        assert_eq!(
            session.handle_event(Event::Resize(70, 14), &geometry),
            None,
            "the picker leaves terminal resize ownership to the root",
        );
    }

    #[test]
    fn empty_or_zero_width_picker_footers_have_no_positions() {
        let (positioned, rows) = position_picker_footer_items::<()>(Vec::new(), 8);
        assert!(positioned.is_empty());
        assert_eq!(rows, 0);

        let (positioned, rows) = position_picker_footer_items(vec![("Save".to_owned(), ())], 0);
        assert!(positioned.is_empty());
        assert_eq!(rows, 0);
    }

    #[test]
    fn keyboard_filter_keeps_arrows_live_and_enter_descends_before_picking() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("reports")).unwrap();
        fs::write(dir.path().join("reports/tool.py"), b"print(1)\n").unwrap();
        fs::write(dir.path().join("alpha.py"), b"").unwrap();
        let contract = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::FileOrDirectory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        let mut session = FilePickerSession::new(contract);
        let geometry = FilePickerGeometry::default();
        for character in "repo".chars() {
            assert_eq!(
                session.handle_event(key(KeyCode::Char(character)), &geometry),
                Some(FilePickerEvent::Changed)
            );
        }
        assert_eq!(session.explorer().visible_count(), 1);
        assert_eq!(
            session.handle_event(key(KeyCode::Down), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(session.current_dir(), &dir.path().join("reports"));
        assert_eq!(
            session.handle_event(key(KeyCode::Down), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &geometry),
            Some(FilePickerEvent::Accepted(vec![
                dir.path().join("reports/tool.py")
            ]))
        );
    }

    #[test]
    fn file_filter_matches_latest_main_hidden_and_directory_ranking_rules() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), b"").unwrap();
        fs::create_dir(dir.path().join("xz")).unwrap();
        fs::write(dir.path().join("xa"), b"").unwrap();
        let contract = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::FileOrDirectory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        let mut session = FilePickerSession::new(contract);
        assert_ne!(
            session
                .explorer()
                .current_entry()
                .map(|entry| entry.name.as_str()),
            Some("..")
        );

        let geometry = FilePickerGeometry::default();
        assert_eq!(
            session.handle_event(key(KeyCode::Char('x')), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            visible_entries(session.explorer())
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["xz", "xa"]
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('.')), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('e')), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            visible_entries(session.explorer())
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec![".env"]
        );
    }

    #[test]
    fn row_and_footer_mouse_targets_are_positive_paths() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("tool.py"), b"").unwrap();
        let contract = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        let mut session = FilePickerSession::new(contract);
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        let mut geometry = FilePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let row = geometry
            .hits
            .iter()
            .find(|hit| {
                let FilePickerHit::Entry(index) = &hit.target else {
                    return false;
                };
                visible_entries(session.explorer())
                    .get(*index)
                    .is_some_and(|entry| entry.name == "tool.py")
            })
            .unwrap();
        assert_eq!(
            file_click(&mut session, &geometry, row.area),
            Some(FilePickerEvent::Accepted(vec![dir.path().join("tool.py")]))
        );
    }

    #[test]
    fn tiny_and_wide_test_backends_keep_cancel_discoverable() {
        let dir = tempdir().unwrap();
        let contract = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::FileOrDirectory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        for (width, height) in [(30, 6), (100, 24)] {
            let mut session = FilePickerSession::new(contract.clone());
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut geometry = FilePickerGeometry::default();
            terminal
                .draw(|frame| {
                    geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
                })
                .unwrap();
            assert!(buffer_text(&terminal).contains("Cancel"));
            assert!(
                geometry
                    .hits
                    .iter()
                    .any(|hit| hit.target == FilePickerHit::Accept)
            );
            assert!(
                geometry
                    .hits
                    .iter()
                    .any(|hit| hit.target == FilePickerHit::Cancel)
            );
            if width == 100 {
                assert!(
                    geometry
                        .hits
                        .iter()
                        .any(|hit| hit.target == FilePickerHit::Up)
                );
                assert!(
                    geometry
                        .hits
                        .iter()
                        .any(|hit| hit.target == FilePickerHit::Hidden)
                );
            }
        }
    }

    #[test]
    fn prompt_picker_filters_toggles_and_returns_the_complete_working_set() {
        let picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![
                PickerItem::new("topic".to_owned(), "topic"),
                PickerItem::new("api_key".to_owned(), "api_key"),
                PickerItem::new("format".to_owned(), "format"),
            ],
            vec!["topic".to_owned(), "format".to_owned()],
        );
        let mut session = PromptCandidatePickerSession::new(picker);
        let geometry = ChoicePickerGeometry::default();
        for character in "key".chars() {
            assert_eq!(
                session.handle_event(key(KeyCode::Char(character)), &geometry),
                Some(PromptCandidatePickerEvent::Changed)
            );
        }
        assert_eq!(session.visible_names(), vec!["api_key"]);
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char(' ')), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
                &geometry,
            ),
            Some(PromptCandidatePickerEvent::Accepted(vec![
                "topic".to_owned(),
                "api_key".to_owned(),
                "format".to_owned(),
            ]))
        );
    }

    #[test]
    fn prompt_picker_select_all_done_and_cancel_have_mouse_and_tiny_backend_paths() {
        let picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![
                PickerItem::new("a".to_owned(), "a"),
                PickerItem::new("b".to_owned(), "b"),
            ],
            Vec::new(),
        );
        let mut session = PromptCandidatePickerSession::new(picker);
        let mut terminal = Terminal::new(TestBackend::new(42, 8)).unwrap();
        let mut geometry = ChoicePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        for (target, expected) in [
            (
                ChoicePickerHit::SelectAll,
                PromptCandidatePickerEvent::Changed,
            ),
            (
                ChoicePickerHit::Done,
                PromptCandidatePickerEvent::Accepted(vec!["a".to_owned(), "b".to_owned()]),
            ),
        ] {
            let hit = geometry
                .hits
                .iter()
                .find(|hit| hit.target == target)
                .unwrap();
            assert_eq!(
                prompt_click(&mut session, &geometry, hit.area),
                Some(expected)
            );
        }

        let cancel = geometry
            .hits
            .iter()
            .find(|hit| hit.target == ChoicePickerHit::Cancel)
            .unwrap();
        assert_eq!(
            prompt_click(&mut session, &geometry, cancel.area),
            Some(PromptCandidatePickerEvent::Cancelled)
        );
    }

    #[test]
    fn every_prompt_picker_footer_action_has_a_key_and_mouse_twin_at_every_size_tier() {
        let picker = || {
            ChoicePicker::new(
                PickerMode::Multiple,
                vec![PickerItem::new("name".to_owned(), "name")],
                Vec::new(),
            )
        };
        let is_footer = |target: &ChoicePickerHit| {
            matches!(target, ChoicePickerHit::Done | ChoicePickerHit::Cancel)
        };
        let key_for = |target: &ChoicePickerHit| {
            if *target == ChoicePickerHit::Done {
                control('s')
            } else {
                key(KeyCode::Esc)
            }
        };

        let mut inventory_session = PromptCandidatePickerSession::new(picker());
        let mut inventory_terminal = Terminal::new(TestBackend::new(200, 30)).unwrap();
        let mut inventory_geometry = ChoicePickerGeometry::default();
        inventory_terminal
            .draw(|frame| {
                inventory_geometry = render_prompt_candidate_picker(
                    frame,
                    frame.area(),
                    &mut inventory_session,
                    Locale::En,
                );
            })
            .unwrap();
        let expected = inventory_geometry
            .hits
            .iter()
            .filter(|hit| is_footer(&hit.target))
            .map(|hit| hit.target.clone())
            .collect::<Vec<_>>();
        assert_eq!(expected.len(), 2, "the production footer inventory changed");

        for (width, height) in [(120, 30), (46, 12), (24, 6)] {
            let mut seen = Vec::new();
            for page in 0..8 {
                let mut page_session = PromptCandidatePickerSession::new(picker());
                let mut page_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let mut page_geometry = ChoicePickerGeometry::default();
                for step in 0..=page {
                    page_terminal
                        .draw(|frame| {
                            page_geometry = render_prompt_candidate_picker(
                                frame,
                                frame.area(),
                                &mut page_session,
                                Locale::En,
                            );
                        })
                        .unwrap();
                    if step < page {
                        let footer = page_geometry
                            .hits
                            .iter()
                            .find(|hit| is_footer(&hit.target))
                            .unwrap();
                        assert_eq!(
                            page_session.handle_event(
                                mouse(footer.area, MouseEventKind::ScrollDown),
                                &page_geometry,
                            ),
                            Some(PromptCandidatePickerEvent::Changed)
                        );
                    }
                }
                let unseen = page_geometry
                    .hits
                    .iter()
                    .filter(|hit| is_footer(&hit.target))
                    .filter(|hit| !seen.contains(&hit.target))
                    .cloned()
                    .collect::<Vec<_>>();
                for hit in unseen {
                    seen.push(hit.target.clone());

                    let mut key_session = PromptCandidatePickerSession::new(picker());
                    let mut key_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                    let mut key_geometry = ChoicePickerGeometry::default();
                    key_terminal
                        .draw(|frame| {
                            key_geometry = render_prompt_candidate_picker(
                                frame,
                                frame.area(),
                                &mut key_session,
                                Locale::En,
                            );
                        })
                        .unwrap();
                    let key_result = key_session.handle_event(key_for(&hit.target), &key_geometry);

                    let mut mouse_session = PromptCandidatePickerSession::new(picker());
                    let mut mouse_terminal =
                        Terminal::new(TestBackend::new(width, height)).unwrap();
                    let mut mouse_geometry = ChoicePickerGeometry::default();
                    for step in 0..=page {
                        mouse_terminal
                            .draw(|frame| {
                                mouse_geometry = render_prompt_candidate_picker(
                                    frame,
                                    frame.area(),
                                    &mut mouse_session,
                                    Locale::En,
                                );
                            })
                            .unwrap();
                        if step < page {
                            let footer = mouse_geometry
                                .hits
                                .iter()
                                .find(|candidate| is_footer(&candidate.target))
                                .unwrap();
                            let _ = mouse_session.handle_event(
                                mouse(footer.area, MouseEventKind::ScrollDown),
                                &mouse_geometry,
                            );
                        }
                    }
                    let mouse_hit = mouse_geometry
                        .hits
                        .iter()
                        .find(|candidate| candidate.target == hit.target)
                        .unwrap();
                    assert_eq!(
                        mouse_session.handle_event(
                            mouse(mouse_hit.area, MouseEventKind::Down(MouseButton::Left)),
                            &mouse_geometry,
                        ),
                        Some(PromptCandidatePickerEvent::Changed)
                    );
                    let mouse_result = mouse_session.handle_event(
                        mouse(mouse_hit.area, MouseEventKind::Up(MouseButton::Left)),
                        &mouse_geometry,
                    );
                    assert_eq!(
                        mouse_result, key_result,
                        "prompt-picker {:?} key and mouse diverged at {width}x{height}",
                        hit.target
                    );
                }
                if seen.len() == expected.len() {
                    break;
                }
            }
            assert!(
                seen.len() == expected.len() && expected.iter().all(|item| seen.contains(item)),
                "prompt-picker footer dropped actions at {width}x{height}: expected={expected:?} seen={seen:?}"
            );
        }
    }

    #[test]
    fn picker_footer_clamps_after_growth_and_single_directory_accept_has_mouse_twin() {
        let picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![PickerItem::new("name".to_owned(), "name")],
            Vec::new(),
        );
        let mut prompt = PromptCandidatePickerSession::new(picker);
        let mut narrow = Terminal::new(TestBackend::new(24, 6)).unwrap();
        let mut prompt_geometry = ChoicePickerGeometry::default();
        narrow
            .draw(|frame| {
                prompt_geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut prompt, Locale::En);
            })
            .unwrap();
        let footer = prompt_geometry
            .hits
            .iter()
            .find(|hit| matches!(hit.target, ChoicePickerHit::Done | ChoicePickerHit::Cancel))
            .unwrap();
        assert_eq!(
            prompt.handle_event(
                mouse(footer.area, MouseEventKind::ScrollDown),
                &prompt_geometry,
            ),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert!(prompt.footer_scroll.scroll_offset() > 0);
        let mut wide = Terminal::new(TestBackend::new(120, 30)).unwrap();
        wide.draw(|frame| {
            let geometry =
                render_prompt_candidate_picker(frame, frame.area(), &mut prompt, Locale::En);
            assert!(
                geometry
                    .hits
                    .iter()
                    .any(|hit| hit.target == ChoicePickerHit::Done)
            );
            assert!(
                geometry
                    .hits
                    .iter()
                    .any(|hit| hit.target == ChoicePickerHit::Cancel)
            );
        })
        .unwrap();
        assert_eq!(prompt.footer_scroll.scroll_offset(), 0);

        let empty = tempdir().unwrap();
        let contract = PathPickerState::new(
            PickerPurpose::Argument,
            empty.path().to_path_buf(),
            PathSelectionMode::FileOrDirectory,
            skit_ui::PathOutputPolicy::RelativeTo(empty.path().to_path_buf()),
            false,
        );
        let expected = Some(FilePickerEvent::Accepted(vec![PathBuf::new()]));
        let mut by_key = FilePickerSession::new(contract.clone());
        assert_eq!(
            by_key.handle_event(key(KeyCode::Enter), &FilePickerGeometry::default()),
            expected
        );

        let mut by_mouse = FilePickerSession::new(contract);
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut file_geometry = FilePickerGeometry::default();
        terminal
            .draw(|frame| {
                file_geometry = render_file_picker(frame, frame.area(), &mut by_mouse, Locale::En);
            })
            .unwrap();
        let accept = file_geometry
            .hits
            .iter()
            .find(|hit| hit.target == FilePickerHit::Accept)
            .unwrap();
        assert_eq!(
            by_mouse.handle_event(
                mouse(accept.area, MouseEventKind::Down(MouseButton::Left)),
                &file_geometry,
            ),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            by_mouse.handle_event(
                mouse(accept.area, MouseEventKind::Up(MouseButton::Left)),
                &file_geometry,
            ),
            expected
        );
    }

    #[test]
    fn prompt_picker_wheel_owns_the_list_and_reaches_the_last_clickable_row() {
        let items = (0..12)
            .map(|index| {
                let name = format!("item-{index:02}");
                PickerItem::new(name.clone(), name)
            })
            .collect();
        let picker = ChoicePicker::new(PickerMode::Multiple, items, Vec::new());
        let mut session = PromptCandidatePickerSession::new(picker);
        let mut terminal = Terminal::new(TestBackend::new(46, 8)).unwrap();
        let mut geometry = ChoicePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();

        assert_eq!(session.list.selected_index, 0);
        assert_eq!(
            session.handle_event(mouse(geometry.rows, MouseEventKind::ScrollDown), &geometry,),
            Some(PromptCandidatePickerEvent::Changed),
        );
        assert_eq!(session.list.selected_index, 1);
        assert_eq!(
            session.handle_event(mouse(geometry.rows, MouseEventKind::ScrollUp), &geometry,),
            Some(PromptCandidatePickerEvent::Changed),
        );
        assert_eq!(session.list.selected_index, 0);
        for _ in 0..20 {
            assert_eq!(
                session.handle_event(mouse(geometry.rows, MouseEventKind::ScrollDown), &geometry,),
                Some(PromptCandidatePickerEvent::Changed),
            );
        }
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        assert!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.target == ChoicePickerHit::Row(11)),
            "wheel scrolling must make the final semantic row visible and clickable",
        );
    }

    #[test]
    fn prompt_picker_search_click_places_the_caret_before_typing() {
        let picker = ChoicePicker::new(PickerMode::Multiple, Vec::new(), Vec::new());
        let mut session = PromptCandidatePickerSession::new(picker);
        session.query = LineInput::new("abcdef".to_owned());
        let mut terminal = Terminal::new(TestBackend::new(46, 8)).unwrap();
        let mut geometry = ChoicePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let _ = prompt_click(
            &mut session,
            &geometry,
            Rect::new(geometry.search.x.saturating_add(3), geometry.search.y, 1, 1),
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('X')), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(session.query.value(), "abXcdef");

        assert_eq!(
            session.handle_event(
                mouse(geometry.search, MouseEventKind::Down(MouseButton::Left)),
                &geometry,
            ),
            Some(PromptCandidatePickerEvent::Changed)
        );
        session.cancel_click();
        assert_eq!(
            session.handle_event(
                mouse(geometry.search, MouseEventKind::Up(MouseButton::Left)),
                &geometry,
            ),
            None
        );
    }

    #[test]
    fn file_picker_wheel_reaches_the_last_clickable_entry() {
        let dir = tempdir().unwrap();
        for index in 0..20 {
            fs::write(dir.path().join(format!("item-{index:02}")), b"").unwrap();
        }
        let contract = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        let mut session = FilePickerSession::new(contract);
        let mut terminal = Terminal::new(TestBackend::new(46, 8)).unwrap();
        let mut geometry = FilePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let initial = session.explorer.cursor_index;
        assert_eq!(
            session.handle_event(mouse(geometry.rows, MouseEventKind::ScrollDown), &geometry,),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(session.explorer.cursor_index, initial.saturating_add(1));
        assert_eq!(
            session.handle_event(mouse(geometry.rows, MouseEventKind::ScrollUp), &geometry,),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(session.explorer.cursor_index, initial);
        for _ in 0..40 {
            assert_eq!(
                session.handle_event(mouse(geometry.rows, MouseEventKind::ScrollDown), &geometry,),
                Some(FilePickerEvent::Changed)
            );
        }
        let last = visible_entries(session.explorer()).len().saturating_sub(1);
        assert_eq!(session.explorer.cursor_index, last);
        assert!(session.explorer.scroll > 0);
    }

    #[test]
    fn prompt_picker_routes_every_real_key_paste_mouse_and_empty_result() {
        let picker = ChoicePicker::new(
            PickerMode::Single,
            vec![
                PickerItem::new("alpha".to_owned(), "alpha"),
                PickerItem::new("beta".to_owned(), "beta"),
                PickerItem::new("gamma".to_owned(), "gamma"),
            ],
            Vec::new(),
        );
        let mut session = PromptCandidatePickerSession::new(picker);
        let mut terminal = Terminal::new(TestBackend::new(52, 10)).unwrap();
        let mut geometry = ChoicePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();

        for code in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::End,
            KeyCode::Home,
            KeyCode::Tab,
            KeyCode::BackTab,
        ] {
            assert_eq!(
                session.handle_event(key(code), &geometry),
                Some(PromptCandidatePickerEvent::Changed)
            );
        }
        assert_eq!(session.handle_event(control('x'), &geometry), None);
        assert_eq!(session.handle_event(key(KeyCode::F(2)), &geometry), None);
        assert_eq!(
            session.handle_event(Event::Paste("be".to_owned()), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(session.visible_names(), ["beta"]);
        assert_eq!(
            session.handle_event(key(KeyCode::Backspace), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(control('s'), &geometry),
            Some(PromptCandidatePickerEvent::Accepted(vec![
                "beta".to_owned()
            ]))
        );

        terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        for target in [ChoicePickerHit::Search, ChoicePickerHit::Row(0)] {
            let hit = geometry
                .hits
                .iter()
                .find(|hit| hit.target == target)
                .expect("the rendered choice target must be clickable");
            assert_eq!(
                prompt_click(&mut session, &geometry, hit.area),
                Some(PromptCandidatePickerEvent::Changed)
            );
        }
        let search = geometry.search;
        assert_eq!(
            session.handle_event(mouse(search, MouseEventKind::Moved), &geometry),
            None
        );
        assert_eq!(
            session.handle_event(
                mouse(search, MouseEventKind::Up(MouseButton::Left)),
                &geometry,
            ),
            None
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &geometry),
            Some(PromptCandidatePickerEvent::Cancelled)
        );

        let empty = ChoicePicker::new(PickerMode::Single, Vec::new(), Vec::new());
        let mut empty = PromptCandidatePickerSession::new(empty);
        let mut empty_terminal = Terminal::new(TestBackend::new(42, 8)).unwrap();
        empty_terminal
            .draw(|frame| {
                geometry =
                    render_prompt_candidate_picker(frame, frame.area(), &mut empty, Locale::ZhCn);
            })
            .unwrap();
        assert!(!buffer_text(&empty_terminal).trim().is_empty());
        assert_eq!(
            empty.handle_event(control('s'), &geometry),
            Some(PromptCandidatePickerEvent::Cancelled)
        );
        assert_eq!(
            empty.handle_event(control('n'), &geometry),
            Some(PromptCandidatePickerEvent::Changed)
        );
    }

    #[test]
    fn file_picker_routes_multi_directory_error_locale_and_reverse_mouse_paths() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), b"a").unwrap();
        fs::write(dir.path().join("beta.txt"), b"b").unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        fs::write(dir.path().join("folder/nested.txt"), b"n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("alpha.txt"), dir.path().join("alias")).unwrap();

        let contract = PathPickerState::new(
            PickerPurpose::Argument,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::RelativeTo(dir.path().to_path_buf()),
            true,
        );
        let mut session = FilePickerSession::new(contract);
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let mut geometry = FilePickerGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::ZhTw);
            })
            .unwrap();
        #[cfg(unix)]
        assert!(buffer_text(&terminal).contains('↗'));

        assert_eq!(session.handle_event(control('x'), &geometry), None);
        assert_eq!(session.handle_event(key(KeyCode::F(2)), &geometry), None);
        assert_eq!(
            session.handle_event(control('a'), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert!(session.explorer().selected_files.len() >= 2);
        assert_eq!(
            session.handle_event(control('n'), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert!(session.explorer().selected_files.is_empty());
        assert_eq!(session.handle_hit(FilePickerHit::Accept), None);
        assert_eq!(
            session.handle_event(control('h'), &geometry),
            Some(FilePickerEvent::Changed)
        );

        for code in [
            KeyCode::Home,
            KeyCode::PageUp,
            KeyCode::End,
            KeyCode::PageDown,
        ] {
            assert_eq!(
                session.handle_event(key(code), &geometry),
                Some(FilePickerEvent::Changed)
            );
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Char(' ')), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Char('z')), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Backspace), &geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Esc), &geometry),
            Some(FilePickerEvent::Cancelled)
        );

        terminal
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut session, Locale::En);
            })
            .unwrap();
        let search = geometry
            .hits
            .iter()
            .find(|hit| hit.target == FilePickerHit::Search)
            .expect("search must be a rendered mouse target")
            .area;
        assert_eq!(
            session.handle_event(mouse(search, MouseEventKind::Moved), &geometry),
            None
        );
        assert_eq!(
            session.handle_event(
                mouse(search, MouseEventKind::Up(MouseButton::Left)),
                &geometry
            ),
            None
        );
        assert_eq!(
            file_click(&mut session, &geometry, search),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            session.handle_event(
                mouse(search, MouseEventKind::Down(MouseButton::Left)),
                &geometry
            ),
            Some(FilePickerEvent::Changed)
        );
        session.cancel_click();
        assert_eq!(
            session.handle_event(
                mouse(search, MouseEventKind::Up(MouseButton::Left)),
                &geometry
            ),
            None
        );
        assert_eq!(session.handle_hit(FilePickerHit::Entry(usize::MAX)), None);
        assert_eq!(session.handle_hit(FilePickerHit::CurrentDirectory), None);
        assert_eq!(
            session.handle_event(control('n'), &geometry),
            Some(FilePickerEvent::Changed)
        );

        let row_indices = geometry
            .hits
            .iter()
            .filter_map(|hit| match hit.target {
                FilePickerHit::Entry(index)
                    if visible_entries(session.explorer())
                        .get(index)
                        .is_some_and(|entry| entry.is_selectable()) =>
                {
                    Some((index, hit.area))
                }
                _ => None,
            })
            .take(2)
            .collect::<Vec<_>>();
        assert_eq!(row_indices.len(), 2);
        for (_, area) in row_indices {
            assert_eq!(
                file_click(&mut session, &geometry, area),
                Some(FilePickerEvent::Changed)
            );
        }
        let accepted = session
            .handle_hit(FilePickerHit::Accept)
            .expect("two selected files must be accepted");
        assert!(matches!(
            accepted,
            FilePickerEvent::Accepted(paths)
                if paths.len() == 2 && paths.windows(2).all(|pair| pair[0] <= pair[1])
        ));

        let mut initial_filter = PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        initial_filter.set_query("alpha");
        let initial_filter = FilePickerSession::new(initial_filter);
        assert_eq!(
            visible_entries(initial_filter.explorer())
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            ["alpha.txt"]
        );
        assert_eq!(initial_filter.io_error(), None);

        let mut current_with_rows = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::FileOrDirectory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        assert_eq!(
            current_with_rows.handle_event(key(KeyCode::Home), &FilePickerGeometry::default()),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            current_with_rows.handle_event(key(KeyCode::Down), &FilePickerGeometry::default()),
            Some(FilePickerEvent::Changed)
        );

        let empty = tempdir().unwrap();
        let directory_contract = PathPickerState::new(
            PickerPurpose::WorkingDirectory,
            empty.path().to_path_buf(),
            PathSelectionMode::Directory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        );
        let mut directory = FilePickerSession::new(directory_contract);
        let default_geometry = FilePickerGeometry::default();
        assert_eq!(
            directory.handle_event(key(KeyCode::Up), &default_geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            directory.handle_event(key(KeyCode::Down), &default_geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            directory.handle_event(key(KeyCode::End), &default_geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            directory.handle_hit(FilePickerHit::Accept),
            Some(FilePickerEvent::Accepted(vec![empty.path().to_path_buf()]))
        );
        assert_eq!(
            directory.handle_event(key(KeyCode::Enter), &default_geometry),
            Some(FilePickerEvent::Accepted(vec![empty.path().to_path_buf()]))
        );
        assert_eq!(
            directory.handle_event(key(KeyCode::Char('x')), &default_geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            directory.handle_event(key(KeyCode::Esc), &default_geometry),
            Some(FilePickerEvent::Changed)
        );

        let mut empty_file = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Source,
            empty.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        assert_eq!(
            empty_file.handle_event(key(KeyCode::End), &default_geometry),
            Some(FilePickerEvent::Changed)
        );
        assert_eq!(
            empty_file.handle_hit(FilePickerHit::Accept),
            Some(FilePickerEvent::Changed),
            "the visible Select chip follows Enter into the highlighted parent row"
        );
        empty_file = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Source,
            empty.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        assert_eq!(
            empty_file.handle_event(key(KeyCode::Char('z')), &default_geometry),
            Some(FilePickerEvent::Changed)
        );
        let mut no_match = Terminal::new(TestBackend::new(40, 8)).unwrap();
        no_match
            .draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut empty_file, Locale::En);
            })
            .unwrap();
        assert!(buffer_text(&no_match).contains("No matching entries"));

        let mut directory_only = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::WorkingDirectory,
            dir.path().to_path_buf(),
            PathSelectionMode::Directory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        let file_index = visible_entries(directory_only.explorer())
            .iter()
            .position(|entry| entry.name == "alpha.txt")
            .expect("the real file row must be visible");
        assert_eq!(
            directory_only.handle_hit(FilePickerHit::Entry(file_index)),
            None
        );

        let nearest = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Configuration,
            PathBuf::from("a-path-that-does-not-exist"),
            PathSelectionMode::FileOrDirectory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        assert!(nearest.current_dir().is_dir());

        for purpose in [
            PickerPurpose::Source,
            PickerPurpose::WorkingDirectory,
            PickerPurpose::Configuration,
        ] {
            let mut localized = FilePickerSession::new(PathPickerState::new(
                purpose,
                dir.path().to_path_buf(),
                PathSelectionMode::FileOrDirectory,
                skit_ui::PathOutputPolicy::Absolute,
                false,
            ));
            let mut tiny = Terminal::new(TestBackend::new(18, 5)).unwrap();
            tiny.draw(|frame| {
                geometry = render_file_picker(frame, frame.area(), &mut localized, Locale::ZhCn);
            })
            .unwrap();
            assert!(!buffer_text(&tiny).trim().is_empty());
        }

        let mut one_column = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        let mut one_column_terminal = Terminal::new(TestBackend::new(1, 4)).unwrap();
        one_column_terminal
            .draw(|frame| {
                let _ = render_file_picker(frame, frame.area(), &mut one_column, Locale::En);
            })
            .unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let unreadable = tempdir().unwrap();
            let original = fs::metadata(unreadable.path()).unwrap().permissions();
            fs::set_permissions(unreadable.path(), fs::Permissions::from_mode(0o000)).unwrap();
            let mut failed = FilePickerSession::new(PathPickerState::new(
                PickerPurpose::Source,
                unreadable.path().to_path_buf(),
                PathSelectionMode::File,
                skit_ui::PathOutputPolicy::Absolute,
                false,
            ));
            fs::set_permissions(unreadable.path(), original).unwrap();
            assert!(failed.io_error().is_some());
            let mut failed_terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
            failed_terminal
                .draw(|frame| {
                    let _ = render_file_picker(frame, frame.area(), &mut failed, Locale::En);
                })
                .unwrap();
            assert!(!buffer_text(&failed_terminal).trim().is_empty());
        }
    }

    #[test]
    fn picker_releases_and_multiplicity_keep_distinct_keyboard_owners() {
        let picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![PickerItem::new("name".to_owned(), "name")],
            Vec::new(),
        );
        let mut prompt = PromptCandidatePickerSession::new(picker);
        let released_escape = Event::Key(KeyEvent::new_with_kind(
            KeyCode::Esc,
            KeyModifiers::NONE,
            KeyEventKind::Release,
        ));
        assert_eq!(
            prompt.handle_event(released_escape, &ChoicePickerGeometry::default()),
            None,
        );
        assert_eq!(
            prompt.handle_event(key(KeyCode::Esc), &ChoicePickerGeometry::default()),
            Some(PromptCandidatePickerEvent::Cancelled),
        );

        let dir = tempdir().unwrap();
        let file = dir.path().join("alpha.txt");
        fs::write(&file, b"").unwrap();
        let contract = |multiple| {
            PathPickerState::new(
                PickerPurpose::Source,
                dir.path().to_path_buf(),
                PathSelectionMode::File,
                skit_ui::PathOutputPolicy::Absolute,
                multiple,
            )
        };
        let geometry = FilePickerGeometry::default();
        let mut single = FilePickerSession::new(contract(false));
        assert_eq!(
            single.handle_event(
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Esc,
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                &geometry,
            ),
            None,
        );
        assert_eq!(single.handle_event(control('a'), &geometry), None);
        assert_eq!(single.handle_event(control('n'), &geometry), None);
        assert_eq!(
            single.handle_event(key(KeyCode::Char(' ')), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert!(single.explorer().selected_files.is_empty());

        let mut multiple = FilePickerSession::new(contract(true));
        assert_eq!(
            multiple.handle_event(key(KeyCode::Char(' ')), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert!(multiple.explorer().selected_files.contains(&file));
        assert_eq!(
            multiple.handle_event(control('n'), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert!(multiple.explorer().selected_files.is_empty());
        assert_eq!(
            multiple.handle_event(control('a'), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert!(multiple.explorer().selected_files.contains(&file));

        let mut current_directory = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::Directory,
            skit_ui::PathOutputPolicy::Absolute,
            true,
        ));
        assert_eq!(
            current_directory.handle_event(key(KeyCode::Up), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert_eq!(
            current_directory.handle_event(key(KeyCode::Char(' ')), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert!(current_directory.explorer().selected_files.is_empty());
    }

    #[test]
    fn file_picker_never_creates_an_unavailable_or_stale_current_directory_owner() {
        let dir = tempdir().unwrap();
        let alpha = dir.path().join("alpha.txt");
        fs::write(&alpha, b"").unwrap();
        let mut files = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::Source,
            dir.path().to_path_buf(),
            PathSelectionMode::File,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        let default_geometry = FilePickerGeometry::default();
        assert_eq!(
            files.handle_event(key(KeyCode::Up), &default_geometry),
            Some(FilePickerEvent::Changed),
        );
        assert_eq!(
            files.handle_event(key(KeyCode::Enter), &default_geometry),
            Some(FilePickerEvent::Changed),
        );
        assert_eq!(files.current_dir(), &dir.path().parent().unwrap());

        let mut directories = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::WorkingDirectory,
            dir.path().to_path_buf(),
            PathSelectionMode::Directory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
        let mut stale = FilePickerGeometry::default();
        terminal
            .draw(|frame| {
                stale = render_file_picker(frame, frame.area(), &mut directories, Locale::En);
            })
            .unwrap();
        let current = stale
            .hits
            .iter()
            .find(|hit| hit.target == FilePickerHit::CurrentDirectory)
            .expect("the empty filter exposes the current-directory row")
            .area;
        assert_eq!(
            directories.handle_event(key(KeyCode::Char('z')), &stale),
            Some(FilePickerEvent::Changed),
        );
        assert_eq!(file_click(&mut directories, &stale, current), None);
    }

    #[test]
    fn picker_breakpoints_change_only_on_the_documented_height_and_width_edges() {
        let picker = || {
            ChoicePicker::new(
                PickerMode::Multiple,
                vec![PickerItem::new("name".to_owned(), "name")],
                Vec::new(),
            )
        };
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), b"").unwrap();
        for (width, height, search_height, compact) in
            [(52, 11, 1, true), (52, 12, 3, false), (51, 12, 1, true)]
        {
            let mut prompt = PromptCandidatePickerSession::new(picker());
            let mut prompt_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut prompt_geometry = ChoicePickerGeometry::default();
            prompt_terminal
                .draw(|frame| {
                    prompt_geometry = render_prompt_candidate_picker(
                        frame,
                        frame.area(),
                        &mut prompt,
                        Locale::En,
                    );
                })
                .unwrap();
            assert_eq!(prompt_geometry.search.height, search_height);
            assert!(prompt_geometry.hits.iter().any(|hit| {
                hit.target == ChoicePickerHit::Search && hit.area == prompt_geometry.search
            }));

            let mut file = FilePickerSession::new(PathPickerState::new(
                PickerPurpose::Source,
                dir.path().to_path_buf(),
                PathSelectionMode::File,
                skit_ui::PathOutputPolicy::Absolute,
                false,
            ));
            let mut file_terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut file_geometry = FilePickerGeometry::default();
            file_terminal
                .draw(|frame| {
                    file_geometry = render_file_picker(frame, frame.area(), &mut file, Locale::En);
                })
                .unwrap();
            assert_eq!(file_geometry.search.height, search_height);
            assert_eq!(
                buffer_text(&file_terminal).contains(&dir.path().display().to_string()),
                !compact,
            );
        }
    }

    #[test]
    fn file_rows_keep_cursor_and_selection_marks_with_their_exact_owners() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("alpha.txt"), b"").unwrap();
        fs::write(dir.path().join("beta.txt"), b"").unwrap();
        fs::create_dir(dir.path().join("folder")).unwrap();
        let contract = |multiple| {
            PathPickerState::new(
                PickerPurpose::Source,
                dir.path().to_path_buf(),
                PathSelectionMode::FileOrDirectory,
                skit_ui::PathOutputPolicy::Absolute,
                multiple,
            )
        };
        let render = |session: &mut FilePickerSession| {
            let mut terminal = Terminal::new(TestBackend::new(70, 14)).unwrap();
            let mut geometry = FilePickerGeometry::default();
            terminal
                .draw(|frame| {
                    geometry = render_file_picker(frame, frame.area(), session, Locale::En);
                })
                .unwrap();
            (terminal, geometry)
        };
        let row_for = |session: &FilePickerSession, geometry: &FilePickerGeometry, name: &str| {
            geometry
                .hits
                .iter()
                .find(|hit| {
                    let FilePickerHit::Entry(index) = hit.target else {
                        return false;
                    };
                    visible_entries(session.explorer())
                        .get(index)
                        .is_some_and(|entry| entry.name == name)
                })
                .expect("the named entry has a visible row")
                .area
        };

        let mut single = FilePickerSession::new(contract(false));
        let (single_terminal, single_geometry) = render(&mut single);
        let cursor = single.explorer().cursor_index;
        let focused = single_geometry
            .hits
            .iter()
            .find(|hit| hit.target == FilePickerHit::Entry(cursor))
            .expect("the cursor entry has a visible hit")
            .area;
        let sibling = single_geometry
            .hits
            .iter()
            .find(|hit| matches!(hit.target, FilePickerHit::Entry(index) if index != cursor))
            .expect("a sibling entry is visible")
            .area;
        assert_eq!(
            single_terminal.backend().buffer()[(focused.x, focused.y)].bg,
            SELECT_BG,
        );
        assert_ne!(
            single_terminal.backend().buffer()[(sibling.x, sibling.y)].bg,
            SELECT_BG,
        );
        let alpha = row_for(&single, &single_geometry, "alpha.txt");
        assert_eq!(
            single_terminal.backend().buffer()[(alpha.x, alpha.y)].symbol(),
            " "
        );

        let mut multiple = FilePickerSession::new(contract(true));
        let (multiple_terminal, multiple_geometry) = render(&mut multiple);
        let alpha = row_for(&multiple, &multiple_geometry, "alpha.txt");
        let folder = row_for(&multiple, &multiple_geometry, "folder");
        assert_eq!(
            multiple_terminal.backend().buffer()[(alpha.x, alpha.y)].symbol(),
            "☐"
        );
        assert_eq!(
            multiple_terminal.backend().buffer()[(folder.x, folder.y)].symbol(),
            " "
        );

        assert_eq!(
            multiple.handle_event(key(KeyCode::Up), &multiple_geometry),
            Some(FilePickerEvent::Changed),
        );
        let (focused_terminal, focused_geometry) = render(&mut multiple);
        let current = focused_geometry
            .hits
            .iter()
            .find(|hit| hit.target == FilePickerHit::CurrentDirectory)
            .expect("directory selection exposes its current-directory row")
            .area;
        assert_eq!(
            focused_terminal.backend().buffer()[(current.x, current.y)].bg,
            SELECT_BG,
        );
        for hit in focused_geometry
            .hits
            .iter()
            .filter(|hit| matches!(hit.target, FilePickerHit::Entry(_)))
        {
            assert_ne!(
                focused_terminal.backend().buffer()[(hit.area.x, hit.area.y)].bg,
                SELECT_BG,
            );
        }
    }

    #[test]
    fn choice_footer_preserves_exact_fit_order_and_strict_overflow_order() {
        let done = format!("[Ctrl+S] {}", text(Locale::En, "Done"));
        let cancel = format!("[Esc] {}", text(Locale::En, "Cancel"));
        let required = u16::try_from(
            done.width()
                .saturating_add(cancel.width())
                .saturating_add(2),
        )
        .unwrap();
        for (width, first, same_row) in [
            (required - 1, ChoicePickerHit::Cancel, true),
            (required, ChoicePickerHit::Done, true),
            (required + 1, ChoicePickerHit::Done, true),
        ] {
            let picker = ChoicePicker::new(
                PickerMode::Multiple,
                vec![PickerItem::new("name".to_owned(), "name")],
                Vec::new(),
            );
            let mut session = PromptCandidatePickerSession::new(picker);
            let mut terminal = Terminal::new(TestBackend::new(required + 2, 3)).unwrap();
            let mut hits = Vec::new();
            terminal
                .draw(|frame| {
                    hits = render_choice_footer(
                        frame,
                        Rect::new(1, 1, width, 1),
                        Locale::En,
                        &mut session,
                    );
                })
                .unwrap();
            assert_eq!(hits.first().map(|hit| &hit.target), Some(&first));
            assert_eq!(
                hits.len() == 2 && hits[0].area.y == hits[1].area.y,
                same_row
            );
        }
    }

    #[test]
    fn picker_footer_uses_half_open_rows_and_exact_overflow_indicators() {
        let positioned = |rows: usize| {
            (0..rows)
                .map(|row| PositionedPickerFooterItem {
                    label: char::from(b'A' + u8::try_from(row).unwrap()).to_string(),
                    target: row,
                    row,
                    x: 0,
                    width: 1,
                })
                .collect::<Vec<_>>()
        };
        let area = Rect::new(1, 1, 6, 1);
        let mut terminal = Terminal::new(TestBackend::new(10, 4)).unwrap();
        let mut hits = Vec::new();
        terminal
            .draw(|frame| {
                frame.render_widget(Paragraph::new("SENTINEL"), Rect::new(1, 2, 8, 1));
                let scroll = ScrollableContentState::default();
                hits = render_picker_footer_items(frame, area, positioned(2), 2, &scroll);
            })
            .unwrap();
        assert_eq!(hits, vec![(Rect::new(1, 1, 1, 1), 0)]);
        assert_eq!(terminal.backend().buffer()[(1, 2)].symbol(), "S");
        assert_eq!(
            terminal.backend().buffer()[(area.right() - 1, area.y)].symbol(),
            "↓"
        );

        let mut exact = Terminal::new(TestBackend::new(10, 4)).unwrap();
        exact
            .draw(|frame| {
                let scroll = ScrollableContentState::default();
                let item = PositionedPickerFooterItem {
                    label: "Z".to_owned(),
                    target: 0,
                    row: 0,
                    x: area.width - 1,
                    width: 1,
                };
                let exact_hits = render_picker_footer_items(frame, area, vec![item], 1, &scroll);
                assert_eq!(exact_hits.len(), 1);
            })
            .unwrap();
        assert_eq!(
            exact.backend().buffer()[(area.right() - 1, area.y)].symbol(),
            "Z"
        );

        for (offset, indicator, target) in [(1, "↕", 1), (2, "↑", 2)] {
            let mut terminal = Terminal::new(TestBackend::new(10, 4)).unwrap();
            terminal
                .draw(|frame| {
                    let mut scroll = ScrollableContentState::default();
                    scroll.set_lines(vec![String::new(); 3]);
                    scroll.set_scroll_offset(offset);
                    let visible =
                        render_picker_footer_items(frame, area, positioned(3), 3, &scroll);
                    assert_eq!(visible[0].1, target);
                })
                .unwrap();
            assert_eq!(
                terminal.backend().buffer()[(area.right() - 1, area.y)].symbol(),
                indicator,
            );
        }
    }

    #[test]
    fn scrollable_picker_footer_reserves_only_a_real_indicator_column() {
        for (width, expected_hit_width) in [(7, 3), (2, 1), (1, 1)] {
            let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
            let mut hits = Vec::new();
            terminal
                .draw(|frame| {
                    let area = Rect::new(1, 1, width, 1);
                    let labels = if width == 7 {
                        vec![("AAA".to_owned(), 0), ("BBB".to_owned(), 1)]
                    } else {
                        vec![("AA".to_owned(), 0), ("BB".to_owned(), 1)]
                    };
                    let mut scroll = ScrollableContentState::default();
                    let (positioned, rows, _) = scrollable_picker_footer(labels, area, &mut scroll);
                    hits = render_picker_footer_items(frame, area, positioned, rows, &scroll);
                })
                .unwrap();
            assert_eq!(hits[0].0.width, expected_hit_width);
            if width == 7 {
                assert_eq!(hits.len(), 2, "an exact-fit footer wrapped at equality");
                assert_eq!(hits[0].0.y, hits[1].0.y);
            }
        }
    }

    #[test]
    fn file_picker_end_targets_the_last_real_directory_not_the_parent_or_current_row() {
        let dir = tempdir().unwrap();
        let child = dir.path().join("child");
        fs::create_dir(&child).unwrap();
        let mut session = FilePickerSession::new(PathPickerState::new(
            PickerPurpose::WorkingDirectory,
            dir.path().to_path_buf(),
            PathSelectionMode::Directory,
            skit_ui::PathOutputPolicy::Absolute,
            false,
        ));
        let geometry = FilePickerGeometry::default();
        assert_eq!(
            session.handle_event(key(KeyCode::End), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter), &geometry),
            Some(FilePickerEvent::Changed),
        );
        assert_eq!(session.current_dir(), &child);
    }
}
