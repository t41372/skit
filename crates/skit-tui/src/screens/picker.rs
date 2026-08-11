//! Shared list, candidate, and filesystem picker widgets.

use std::path::PathBuf;

use ratatui_core::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui_interact::components::{
    EntryType, FileEntry, FileExplorerState, ListPicker, ListPickerState, ListPickerStyle,
};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    paragraph::{Paragraph, Wrap},
};
use skit_i18n::{Locale, text};
use skit_ui::{ChoicePicker, PathPickerState, PathSelectionMode, PickerPurpose, PickerResult};
use tui_input::{Input as LineInput, InputRequest};
use unicode_width::UnicodeWidthStr as _;

use crate::theme::{ACCENT, BOX_INDIGO, SELECT_BG, SELECT_FG, panel_block};

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
}

impl PromptCandidatePickerSession {
    /// Open one isolated working selection from `ReviewState::prompt_picker`.
    #[must_use]
    pub fn new(picker: ChoicePicker<String>) -> Self {
        let total = picker.visible_items().len();
        Self {
            picker,
            query: LineInput::default(),
            list: ListPickerState::new(total),
            visible_height: 1,
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
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_choice_key(key),
            Event::Paste(value) => {
                for character in value.chars() {
                    self.query.handle(InputRequest::InsertChar(character));
                }
                self.sync_choice_filter();
                Some(PromptCandidatePickerEvent::Changed)
            }
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                let target = geometry
                    .hits
                    .iter()
                    .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                    .map(|hit| hit.target.clone())?;
                self.handle_choice_hit(target)
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
    let compact = area.height < 12 || area.width < 52;
    let outer = panel_block(
        text(locale, "Choose prompt variables").into_owned(),
        BOX_INDIGO,
    );
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
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
    let search_block = if compact {
        Block::default()
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title(text(locale, "type to filter…"))
            .border_style(Style::default().fg(ACCENT))
    };
    frame.render_widget(
        Paragraph::new(session.query.value()).block(search_block),
        search,
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
    let done = format!("[Ctrl+S] {}", text(locale, "Done"));
    let cancel = format!("[Esc] {}", text(locale, "Cancel"));
    frame.render_widget(
        Paragraph::new(format!("{done}  {cancel}"))
            .style(Style::default().add_modifier(Modifier::DIM)),
        footer,
    );
    let done_width = u16::try_from(done.width())
        .unwrap_or(u16::MAX)
        .min(footer.width);
    let cancel_width = u16::try_from(cancel.width())
        .unwrap_or(u16::MAX)
        .min(footer.width);
    let mut hits = vec![
        ChoicePickerHitRegion {
            area: search,
            target: ChoicePickerHit::Search,
        },
        ChoicePickerHitRegion {
            area: all,
            target: ChoicePickerHit::SelectAll,
        },
        ChoicePickerHitRegion {
            area: Rect::new(footer.x, footer.y, done_width, 1),
            target: ChoicePickerHit::Done,
        },
        ChoicePickerHitRegion {
            area: Rect::new(
                footer.right().saturating_sub(cancel_width),
                footer.y,
                cancel_width,
                1,
            ),
            target: ChoicePickerHit::Cancel,
        },
    ];
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
    visible_height: usize,
    io_error: Option<String>,
}

impl FilePickerSession {
    /// Open the nearest readable ancestor of the requested start directory.
    #[must_use]
    pub fn new(contract: PathPickerState) -> Self {
        let start = nearest_directory(contract.start_dir().to_path_buf());
        let mut explorer = FileExplorerState::new(start);
        explorer.show_hidden = contract.show_hidden() || contract.query().starts_with('.');
        let io_error = explorer.load_entries().err().map(|error| error.to_string());
        select_first_real_entry(&mut explorer);
        let query = LineInput::new(contract.query().to_owned());
        if !contract.query().is_empty() {
            explorer.search_query = contract.query().to_owned();
            apply_filter(&mut explorer);
        }
        Self {
            contract,
            explorer,
            query,
            visible_height: 1,
            io_error,
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
            Event::Key(key) if key.kind != KeyEventKind::Release => self.handle_key(key),
            Event::Mouse(mouse) if matches!(mouse.kind, MouseEventKind::Down(_)) => {
                let target = geometry
                    .hits
                    .iter()
                    .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                    .map(|hit| hit.target.clone())?;
                self.handle_hit(target)
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
                self.explorer.cursor_up();
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Down => {
                self.explorer.cursor_down();
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Home => {
                self.explorer.cursor_index = 0;
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::End => {
                self.explorer.cursor_index = self.explorer.visible_count().saturating_sub(1);
                self.explorer.ensure_visible(self.visible_height);
                Some(FilePickerEvent::Changed)
            }
            KeyCode::Enter => self.activate_current(),
            KeyCode::Char(' ') if self.contract.allow_multiple() => {
                self.explorer.toggle_selection();
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
            FilePickerHit::CurrentDirectory => self.accept_current_directory(),
            FilePickerHit::Entry(index) => {
                if index >= self.explorer.visible_count() {
                    return None;
                }
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
            FilePickerHit::Accept => self.accept_selection(),
            FilePickerHit::Cancel => Some(FilePickerEvent::Cancelled),
        }
    }

    fn activate_current(&mut self) -> Option<FilePickerEvent> {
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
        matches!(
            self.contract.selection(),
            PathSelectionMode::Directory | PathSelectionMode::FileOrDirectory
        )
        .then(|| {
            FilePickerEvent::Accepted(vec![self.contract.output_path(&self.explorer.current_dir)])
        })
    }

    fn accept_selection(&self) -> Option<FilePickerEvent> {
        if !self.contract.allow_multiple() {
            return self.accept_current_directory();
        }
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
        let query = self.query.value().to_owned();
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
            select_first_real_entry(&mut self.explorer);
        }
    }

    fn reload_entries(&mut self) {
        self.explorer.show_hidden = self.contract.show_hidden();
        self.io_error = self
            .explorer
            .load_entries()
            .err()
            .map(|error| error.to_string());
        select_first_real_entry(&mut self.explorer);
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
    let search_block = if compact {
        Block::default()
    } else {
        Block::default()
            .borders(Borders::ALL)
            .title(text(locale, "Search"))
            .border_style(Style::default().fg(ACCENT))
    };
    frame.render_widget(
        Paragraph::new(session.query.value()).block(search_block),
        search,
    );
    if !compact {
        frame.render_widget(
            Paragraph::new(session.explorer.current_dir.display().to_string())
                .style(Style::default().add_modifier(Modifier::DIM)),
            path_row,
        );
    }
    let accepts_dir = matches!(
        session.contract.selection(),
        PathSelectionMode::Directory | PathSelectionMode::FileOrDirectory
    );
    let mut lines = Vec::new();
    let mut hits = vec![FilePickerHitRegion {
        area: search,
        target: FilePickerHit::Search,
    }];
    if accepts_dir {
        lines.push(Line::from(vec![
            Span::styled("▶ ", Style::default().fg(ACCENT)),
            Span::styled(
                session.explorer.current_dir.display().to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
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
        let cursor = display_index == session.explorer.cursor_index;
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
    hits.extend(render_file_footer(frame, footer, locale, compact));
    FilePickerGeometry { search, rows, hits }
}

fn render_file_footer(
    frame: &mut Frame,
    area: Rect,
    locale: Locale,
    compact: bool,
) -> Vec<FilePickerHitRegion> {
    let mut chips = vec![
        (
            format!("[Enter] {}", text(locale, "Select")),
            FilePickerHit::Accept,
        ),
        (
            format!("[Esc] {}", text(locale, "Cancel")),
            FilePickerHit::Cancel,
        ),
    ];
    if !compact {
        chips.extend([
            (
                format!("[Backspace] {}", text(locale, "Back")),
                FilePickerHit::Up,
            ),
            ("[Ctrl+H] .".to_owned(), FilePickerHit::Hidden),
        ]);
    }
    let mut x = 0_u16;
    let mut hits = Vec::new();
    for (label, target) in chips {
        let width = u16::try_from(label.width().saturating_add(1))
            .unwrap_or(u16::MAX)
            .min(area.width.saturating_sub(x));
        if width == 0 {
            break;
        }
        let chip_area = Rect::new(area.x.saturating_add(x), area.y, width, 1);
        frame.render_widget(
            Paragraph::new(label).style(Style::default().add_modifier(Modifier::DIM)),
            chip_area,
        );
        hits.push(FilePickerHitRegion {
            area: chip_area,
            target,
        });
        x = x.saturating_add(width).saturating_add(1);
    }
    hits
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

fn select_first_real_entry(explorer: &mut FileExplorerState) {
    if let Some(index) = explorer
        .entries
        .iter()
        .position(|entry| !matches!(entry.entry_type, EntryType::ParentDir))
    {
        explorer.cursor_index = index;
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use ratatui_core::{backend::TestBackend, terminal::Terminal};
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
        let mouse = Event::Mouse(ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
            column: row.area.x,
            row: row.area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            session.handle_event(mouse, &geometry),
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
            let mouse = Event::Mouse(ratatui_crossterm::crossterm::event::MouseEvent {
                kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
                column: hit.area.x,
                row: hit.area.y,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(session.handle_event(mouse, &geometry), Some(expected));
        }

        let cancel = geometry
            .hits
            .iter()
            .find(|hit| hit.target == ChoicePickerHit::Cancel)
            .unwrap();
        let mouse = Event::Mouse(ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
            column: cancel.area.x,
            row: cancel.area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            session.handle_event(mouse, &geometry),
            Some(PromptCandidatePickerEvent::Cancelled)
        );
    }
}
