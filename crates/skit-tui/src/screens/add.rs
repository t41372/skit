//! Add, source-review, executable-review, and prompt-review widgets.

use std::{collections::BTreeMap, hash::Hash, path::PathBuf};

use ratatui_core::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui_interact::{
    components::{
        CheckBox, CheckBoxState, ListPickerState, ScrollableContentState, Select, SelectAction,
        SelectState, handle_scrollable_content_key, handle_scrollable_content_mouse,
        handle_select_key,
    },
    state::FocusManager,
};
use ratatui_widgets::paragraph::Paragraph;
use skit_domain::StorageMode;
use skit_i18n::{Locale, format_text, kind_label, text};
use skit_ui::{
    AddAction, AddNotice, AddProblem, AddStage, AddWorkflowState, DependencySurface, DraftKind,
    KnownEntryKind, PROMPT_LIST_PREVIEW_LIMIT, PathOutputPolicy, PathPickerState,
    PathSelectionMode, PickerPurpose, ReviewLane,
};
use tui_input::{Input as LineInput, backend::crossterm::EventHandler as _};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    session::render_line_input,
    theme::{ACCENT, BOX_MAROON, SELECT_BG, SELECT_FG, panel_block},
};

/// Typed editable control identity.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AddTextField {
    /// Add source path.
    SourcePath,
    /// Command template.
    CommandTemplate,
    /// Command display name.
    CommandName,
    /// Command description.
    CommandDescription,
    /// Review display name.
    ReviewName,
    /// Review description.
    ReviewDescription,
    /// Package dependency list.
    Dependencies,
    /// Python version constraint.
    PythonConstraint,
}

/// Typed focus and mouse identity. User-visible English never selects behavior.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AddControlId {
    /// Mature single-line input.
    Text(AddTextField),
    /// Open source path browser.
    BrowseSource,
    /// Kept draft row.
    Draft(usize),
    /// Start script authoring.
    NewScript,
    /// Start prompt authoring.
    NewPrompt,
    /// Confirm draft deletion.
    DeleteDraft,
    /// Continue source intake.
    Continue,
    /// Ambiguous kind row.
    Kind(usize),
    /// Accept the mature kind-list cursor from the footer.
    PickFocusedKind,
    /// Copy/reference select.
    Storage,
    /// One copy/reference dropdown option.
    StorageOption(usize),
    /// Parser-backed source candidate.
    Candidate(String),
    /// Prompt interpolation master switch.
    Interpolate,
    /// Prompt placeholder candidate.
    PromptCandidate(String),
    /// Prompt runner select.
    Runner,
    /// One prompt-runner dropdown option. Zero means ask at run time.
    RunnerOption(usize),
    /// Open runner editor.
    NewRunner,
    /// Open source editor and rescan.
    EditSource,
    /// Validate and commit.
    Save,
    /// Toggle the currently focused checkbox from the footer.
    ToggleFocused,
    /// Move focus to the next field from the footer.
    NextField,
    /// Cancel.
    Cancel,
}

/// One clickable screen region.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddHitRegion {
    /// Terminal rectangle.
    pub area: Rect,
    /// Semantic target.
    pub target: AddControlId,
}

/// Responsive add geometry.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AddScreenGeometry {
    /// Scrollable body viewport.
    pub body: Rect,
    /// First visible virtual row.
    pub first_visible: usize,
    /// Keyboard-equivalent mouse regions.
    pub hits: Vec<AddHitRegion>,
}

/// Terminal-only result. The host dispatches `Action` through the reducer.
#[derive(Clone, Debug, PartialEq)]
pub enum AddScreenEvent {
    /// Frontend-neutral reducer action.
    Action(AddAction),
    /// Open the mature filesystem picker.
    OpenPathPicker(PathPickerState),
    /// Open the complete prompt-candidate picker.
    OpenPromptCandidates,
    /// Open the shared runner editor.
    OpenRunnerEditor,
    /// Ephemeral focus, cursor, select, or scroll state changed.
    Changed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AddSignature {
    stage: AddStage,
    kind: Option<KnownEntryKind>,
    storage: Option<StorageMode>,
    dependency_surface: Option<DependencySurface>,
    interpolate: Option<bool>,
    drafts: usize,
    candidates: Vec<String>,
    prompt_candidates: Vec<String>,
    runners: Vec<String>,
}

/// Ephemeral mature-widget state for all add surfaces.
#[derive(Debug, Default)]
pub struct AddScreenSession {
    signature: Option<AddSignature>,
    focus: FocusManager<AddControlId>,
    inputs: BTreeMap<AddTextField, LineInput>,
    checks: BTreeMap<AddControlId, CheckBoxState>,
    kind_picker: ListPickerState,
    draft_picker: ListPickerState,
    storage: SelectState,
    runner: SelectState,
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    row_starts: BTreeMap<AddControlId, usize>,
}

impl AddScreenSession {
    /// Current typed focus.
    #[must_use]
    pub fn focused(&self) -> Option<&AddControlId> {
        self.focus.current()
    }

    /// Synchronize widgets from durable reducer state only when the control shape changes.
    pub fn sync(&mut self, state: &AddWorkflowState) {
        let signature = signature(state);
        if self.signature.as_ref() == Some(&signature) {
            self.sync_values(state);
            return;
        }
        self.signature = Some(signature);
        self.focus.clear();
        self.inputs.clear();
        self.checks.clear();
        self.row_starts.clear();
        match state.stage() {
            AddStage::Source => {
                let source = state.source();
                self.insert_input(AddTextField::SourcePath, &source.path);
                self.focus.register(AddControlId::BrowseSource);
                self.insert_input(AddTextField::CommandTemplate, &source.command_template);
                self.insert_input(AddTextField::CommandName, &source.command_name);
                self.insert_input(
                    AddTextField::CommandDescription,
                    &source.command_description,
                );
                for index in 0..source.listed_drafts().len() {
                    self.focus.register(AddControlId::Draft(index));
                }
                self.draft_picker = ListPickerState::new(source.listed_drafts().len());
                if let Some(index) = source.selected_draft {
                    self.draft_picker.select(index);
                }
                self.focus
                    .register_all([AddControlId::NewScript, AddControlId::NewPrompt]);
                if !source.listed_drafts().is_empty() {
                    self.focus.register(AddControlId::DeleteDraft);
                }
                self.focus
                    .register_all([AddControlId::Continue, AddControlId::Cancel]);
            }
            AddStage::Kind => {
                if let Some(picker) = state.kind_picker() {
                    self.kind_picker = ListPickerState::new(picker.choices().len());
                    if let Some(suggested) = picker.suggested()
                        && let Some(index) = picker
                            .choices()
                            .iter()
                            .position(|choice| *choice == suggested)
                    {
                        self.kind_picker.select(index);
                    }
                    for index in 0..picker.choices().len() {
                        self.focus.register(AddControlId::Kind(index));
                    }
                }
                self.focus.register(AddControlId::Cancel);
                self.focus
                    .set(AddControlId::Kind(self.kind_picker.selected_index));
            }
            AddStage::Review => {
                let Some(review) = state.review() else {
                    return;
                };
                self.insert_input(AddTextField::ReviewName, review.name());
                self.insert_input(AddTextField::ReviewDescription, review.description());
                if !review.is_fresh() && review.lane() != ReviewLane::Executable {
                    self.focus.register(AddControlId::Storage);
                    self.storage = SelectState::with_selected(
                        2,
                        usize::from(review.storage() == StorageMode::Reference),
                    );
                }
                match review.dependency_surface() {
                    DependencySurface::Python => {
                        self.insert_input(AddTextField::Dependencies, review.dependencies_text());
                        self.insert_input(AddTextField::PythonConstraint, review.requires_python());
                    }
                    DependencySurface::Npm if review.storage() == StorageMode::Copy => {
                        self.insert_input(AddTextField::Dependencies, review.dependencies_text());
                    }
                    DependencySurface::None
                    | DependencySurface::Npm
                    | DependencySurface::PythonOwned(_) => {}
                }
                if review.storage() == StorageMode::Copy {
                    for candidate in review.candidates() {
                        let id = AddControlId::Candidate(candidate.declaration.name.clone());
                        self.focus.register(id.clone());
                        self.checks
                            .insert(id, CheckBoxState::new(candidate.selected));
                    }
                }
                if review.lane() == ReviewLane::Prompt {
                    self.focus.register(AddControlId::Interpolate);
                    self.checks.insert(
                        AddControlId::Interpolate,
                        CheckBoxState::new(review.interpolate()),
                    );
                    if review.interpolate() {
                        for candidate in review.prompt_preview() {
                            let id = AddControlId::PromptCandidate(candidate.name.clone());
                            self.focus.register(id.clone());
                            self.checks
                                .insert(id, CheckBoxState::new(candidate.selected));
                        }
                    }
                    self.focus.register(AddControlId::Runner);
                    let runner_index = review
                        .runner_names()
                        .iter()
                        .position(|runner| runner == review.runner())
                        .map_or(0, |index| index.saturating_add(1));
                    self.runner = SelectState::with_selected(
                        review.runner_names().len().saturating_add(1),
                        runner_index,
                    );
                    self.focus.register(AddControlId::NewRunner);
                }
                if review.interpolate()
                    && review.prompt_candidates().len() > review.prompt_preview().len()
                {
                    self.focus.register(AddControlId::Continue);
                }
                self.focus.register_all([
                    AddControlId::EditSource,
                    AddControlId::Save,
                    AddControlId::Cancel,
                ]);
            }
            AddStage::ConfirmDraftDelete => self
                .focus
                .register_all([AddControlId::DeleteDraft, AddControlId::Cancel]),
            AddStage::Complete | AddStage::Cancelled => {}
        }
        self.sync_values(state);
    }

    /// Dispatch one key or mouse event through active mature state.
    #[must_use]
    pub fn handle_event(
        &mut self,
        event: Event,
        state: &AddWorkflowState,
        geometry: &AddScreenGeometry,
    ) -> Option<AddScreenEvent> {
        self.sync(state);
        if let Event::Mouse(mouse) = &event {
            if matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            ) && handle_scrollable_content_mouse(
                &mut self.scroll,
                mouse,
                self.viewport,
                self.visible_height,
            )
            .is_some()
            {
                return Some(AddScreenEvent::Changed);
            }
            if matches!(mouse.kind, MouseEventKind::Down(_)) {
                let target = geometry
                    .hits
                    .iter()
                    .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                    .map(|hit| hit.target.clone())?;
                if !matches!(
                    target,
                    AddControlId::PickFocusedKind
                        | AddControlId::ToggleFocused
                        | AddControlId::NextField
                ) {
                    self.focus.set(target.clone());
                    self.ensure_focus_visible();
                }
                return self.activate(target, state);
            }
            return None;
        }
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            // Ctrl+E belongs to a focused text Input as its end-of-line motion (the
            // oracle's Ctrl+A rule); it opens $EDITOR only when no Input owns focus.
            // So while a review text field is focused, do not consume Ctrl+E here —
            // let it fall through to the Input below, which maps it to end-of-line.
            let text_focused = matches!(self.focus.current(), Some(AddControlId::Text(_)));
            if !(text_focused
                && key.code == KeyCode::Char('e')
                && state.stage() == AddStage::Review)
            {
                return match (key.code, state.stage()) {
                    (KeyCode::Char('n'), AddStage::Source) => Some(AddScreenEvent::Action(
                        AddAction::NewDraft(DraftKind::Script),
                    )),
                    (KeyCode::Char('p'), AddStage::Source) => Some(AddScreenEvent::Action(
                        AddAction::NewDraft(DraftKind::Prompt),
                    )),
                    (KeyCode::Char('d'), AddStage::Source) => {
                        Some(AddScreenEvent::Action(AddAction::DeleteSelectedDraft))
                    }
                    (KeyCode::Char('e'), AddStage::Review) => {
                        Some(AddScreenEvent::Action(AddAction::EditSource))
                    }
                    (KeyCode::Char('s'), AddStage::Review) => {
                        Some(AddScreenEvent::Action(AddAction::Save))
                    }
                    // Ctrl+O opens the searchable candidate picker only when the
                    // detected list is capped (more placeholders than the inline
                    // preview shows). A short prompt lists every candidate inline, so
                    // Ctrl+O is a no-op.
                    (KeyCode::Char('o'), AddStage::Review)
                        if state.review().is_some_and(|review| {
                            review.prompt_candidates().len() > PROMPT_LIST_PREVIEW_LIMIT
                        }) =>
                    {
                        Some(AddScreenEvent::OpenPromptCandidates)
                    }
                    _ => None,
                };
            }
        }
        if key.code == KeyCode::Tab {
            self.focus.next();
            self.ensure_focus_visible();
            return Some(AddScreenEvent::Changed);
        }
        if key.code == KeyCode::BackTab {
            self.focus.prev();
            self.ensure_focus_visible();
            return Some(AddScreenEvent::Changed);
        }
        if key.code == KeyCode::Esc {
            return Some(AddScreenEvent::Action(match state.stage() {
                AddStage::Kind => AddAction::PickKind(None),
                AddStage::ConfirmDraftDelete => AddAction::ConfirmDraftDelete(false),
                _ => AddAction::Cancel,
            }));
        }
        if state.stage() == AddStage::Kind {
            match key.code {
                KeyCode::Up => self.kind_picker.select_prev(),
                KeyCode::Down => self.kind_picker.select_next(),
                KeyCode::Home => self.kind_picker.select_first(),
                KeyCode::End => self.kind_picker.select_last(),
                KeyCode::Enter => {
                    let index = self.kind_picker.selected_index;
                    return self.activate(AddControlId::Kind(index), state);
                }
                _ => return None,
            }
            self.focus
                .set(AddControlId::Kind(self.kind_picker.selected_index));
            return Some(AddScreenEvent::Changed);
        }
        if let Some(id) = self.focus.current().cloned() {
            if let AddControlId::Text(field) = id {
                if key.code == KeyCode::Enter && state.stage() == AddStage::Source {
                    return Some(AddScreenEvent::Action(AddAction::Continue));
                }
                let input = self.inputs.get_mut(&field)?;
                if input.handle_event(&Event::Key(key)).is_some() {
                    return Some(AddScreenEvent::Action(text_action(field, input.value())));
                }
            }
            if id == AddControlId::Storage {
                let action = handle_select_key(&key, &mut self.storage);
                if let Some(SelectAction::Select(index)) = action {
                    return Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                        if index == 0 {
                            StorageMode::Copy
                        } else {
                            StorageMode::Reference
                        },
                    )));
                }
                if action.is_some() || self.storage.is_open {
                    return Some(AddScreenEvent::Changed);
                }
            }
            if id == AddControlId::Runner {
                let action = handle_select_key(&key, &mut self.runner);
                if let Some(SelectAction::Select(index)) = action {
                    let name = state
                        .review()
                        .and_then(|review| {
                            index
                                .checked_sub(1)
                                .and_then(|i| review.runner_names().get(i))
                        })
                        .cloned()
                        .unwrap_or_default();
                    return Some(AddScreenEvent::Action(AddAction::SetPromptRunner {
                        name,
                        picked: true,
                    }));
                }
                if action.is_some() || self.runner.is_open {
                    return Some(AddScreenEvent::Changed);
                }
            }
            if key.code == KeyCode::Char(' ')
                && let Some(check) = self.checks.get_mut(&id)
            {
                check.toggle();
                return checkbox_action(&id, check.checked).map(AddScreenEvent::Action);
            }
            if key.code == KeyCode::Enter {
                return self.activate(id, state);
            }
        }
        if handle_scrollable_content_key(&mut self.scroll, &key, self.visible_height).is_some() {
            return Some(AddScreenEvent::Changed);
        }
        None
    }

    fn activate(
        &mut self,
        target: AddControlId,
        state: &AddWorkflowState,
    ) -> Option<AddScreenEvent> {
        match target {
            AddControlId::Text(_) => Some(AddScreenEvent::Changed),
            AddControlId::BrowseSource => {
                Some(AddScreenEvent::OpenPathPicker(PathPickerState::new(
                    PickerPurpose::Source,
                    std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                    PathSelectionMode::FileOrDirectory,
                    PathOutputPolicy::Absolute,
                    false,
                )))
            }
            AddControlId::Draft(index) => {
                Some(AddScreenEvent::Action(AddAction::SelectDraft(index)))
            }
            AddControlId::NewScript => Some(AddScreenEvent::Action(AddAction::NewDraft(
                DraftKind::Script,
            ))),
            AddControlId::NewPrompt => Some(AddScreenEvent::Action(AddAction::NewDraft(
                DraftKind::Prompt,
            ))),
            AddControlId::DeleteDraft => Some(AddScreenEvent::Action(
                if state.stage() == AddStage::ConfirmDraftDelete {
                    AddAction::ConfirmDraftDelete(true)
                } else {
                    AddAction::DeleteSelectedDraft
                },
            )),
            AddControlId::Continue => Some(if state.stage() == AddStage::Review {
                AddScreenEvent::OpenPromptCandidates
            } else {
                AddScreenEvent::Action(AddAction::Continue)
            }),
            AddControlId::Kind(index) => state
                .kind_picker()
                .and_then(|picker| picker.choices().get(index))
                .copied()
                .map(|kind| AddScreenEvent::Action(AddAction::PickKind(Some(kind)))),
            AddControlId::PickFocusedKind => {
                self.activate(AddControlId::Kind(self.kind_picker.selected_index), state)
            }
            AddControlId::Storage => {
                self.storage.toggle();
                Some(AddScreenEvent::Changed)
            }
            AddControlId::StorageOption(index) => {
                self.storage.select(index);
                Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                    if index == 0 {
                        StorageMode::Copy
                    } else {
                        StorageMode::Reference
                    },
                )))
            }
            AddControlId::Candidate(_)
            | AddControlId::Interpolate
            | AddControlId::PromptCandidate(_) => {
                let check = self.checks.get_mut(&target)?;
                check.toggle();
                checkbox_action(&target, check.checked).map(AddScreenEvent::Action)
            }
            AddControlId::Runner => {
                self.runner.toggle();
                Some(AddScreenEvent::Changed)
            }
            AddControlId::RunnerOption(index) => {
                self.runner.select(index);
                let name = state
                    .review()
                    .and_then(|review| {
                        index
                            .checked_sub(1)
                            .and_then(|runner| review.runner_names().get(runner))
                    })
                    .cloned()
                    .unwrap_or_default();
                Some(AddScreenEvent::Action(AddAction::SetPromptRunner {
                    name,
                    picked: true,
                }))
            }
            AddControlId::NewRunner => Some(AddScreenEvent::OpenRunnerEditor),
            AddControlId::EditSource => Some(AddScreenEvent::Action(AddAction::EditSource)),
            AddControlId::Save => Some(AddScreenEvent::Action(AddAction::Save)),
            AddControlId::ToggleFocused => {
                let focused = self.focus.current()?.clone();
                let check = self.checks.get_mut(&focused)?;
                check.toggle();
                checkbox_action(&focused, check.checked).map(AddScreenEvent::Action)
            }
            AddControlId::NextField => {
                self.focus.next();
                self.ensure_focus_visible();
                Some(AddScreenEvent::Changed)
            }
            AddControlId::Cancel => Some(AddScreenEvent::Action(match state.stage() {
                AddStage::Kind => AddAction::PickKind(None),
                AddStage::ConfirmDraftDelete => AddAction::ConfirmDraftDelete(false),
                _ => AddAction::Cancel,
            })),
        }
    }

    fn insert_input(&mut self, field: AddTextField, value: &str) {
        self.inputs.insert(field, LineInput::new(value.to_owned()));
        self.focus.register(AddControlId::Text(field));
    }

    fn sync_values(&mut self, state: &AddWorkflowState) {
        if let Some(review) = state.review() {
            for candidate in review.candidates() {
                if let Some(check) = self
                    .checks
                    .get_mut(&AddControlId::Candidate(candidate.declaration.name.clone()))
                {
                    check.checked = candidate.selected;
                }
            }
            if let Some(check) = self.checks.get_mut(&AddControlId::Interpolate) {
                check.checked = review.interpolate();
            }
            for candidate in review.prompt_preview() {
                if let Some(check) = self
                    .checks
                    .get_mut(&AddControlId::PromptCandidate(candidate.name.clone()))
                {
                    check.checked = candidate.selected;
                }
            }
        }
        for (id, check) in &mut self.checks {
            check.focused = self.focus.is_focused(id);
        }
        self.storage.focused = self.focus.is_focused(&AddControlId::Storage);
        self.runner.focused = self.focus.is_focused(&AddControlId::Runner);
    }

    fn ensure_focus_visible(&mut self) {
        let Some(id) = self.focus.current() else {
            return;
        };
        let Some(row) = self.row_starts.get(id).copied() else {
            return;
        };
        if row < self.scroll.scroll_offset() {
            self.scroll.set_scroll_offset(row);
        } else if row
            >= self
                .scroll
                .scroll_offset()
                .saturating_add(self.visible_height)
        {
            self.scroll
                .set_scroll_offset(row.saturating_sub(self.visible_height.saturating_sub(1)));
        }
    }
}

/// Render AddSource, kind ASK, executable/script review, prompt review, and draft confirmation.
pub fn render_add(
    frame: &mut Frame,
    area: Rect,
    state: &AddWorkflowState,
    session: &mut AddScreenSession,
    locale: Locale,
) -> AddScreenGeometry {
    session.sync(state);
    let short = area.height < 14;
    let footer_height = if short { 1 } else { 2 }.min(area.height);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(footer_height)])
        .split(area);
    let title = match state.stage() {
        AddStage::Source => text(locale, "Add an entry").into_owned(),
        AddStage::Kind => state.kind_picker().map_or_else(
            || text(locale, "Kind").into_owned(),
            |picker| picker.filename().to_owned(),
        ),
        AddStage::Review => state.review().map_or_else(
            || text(locale, "Add").into_owned(),
            |review| {
                let filename = review.source().path.file_name().map_or_else(
                    || review.name().to_owned(),
                    |name| name.to_string_lossy().into_owned(),
                );
                format!("{} {filename}", text(locale, "Add"))
            },
        ),
        AddStage::ConfirmDraftDelete => text(locale, "Confirm removal").into_owned(),
        AddStage::Complete | AddStage::Cancelled => text(locale, "Add").into_owned(),
    };
    let body_block = panel_block(title, BOX_MAROON);
    let body = body_block.inner(chunks[0]);
    frame.render_widget(body_block, chunks[0]);
    let rows = build_rows(state, locale);
    let total_height = rows.iter().map(RenderRow::height).sum::<usize>();
    session
        .scroll
        .set_lines(vec![String::new(); total_height.max(1)]);
    session.viewport = body;
    session.visible_height = usize::from(body.height).max(1);
    session.row_starts.clear();
    let mut logical = 0;
    for row in &rows {
        if let Some(id) = row.id() {
            session.row_starts.insert(id.clone(), logical);
        }
        logical = logical.saturating_add(row.height());
    }
    session.ensure_focus_visible();
    let maximum = total_height.saturating_sub(session.visible_height);
    if session.scroll.scroll_offset() > maximum {
        session.scroll.set_scroll_offset(maximum);
    }
    let offset = session.scroll.scroll_offset();
    let mut hits = Vec::new();
    let mut row_start = 0_usize;
    let mut select_overlays = Vec::new();
    for row in &rows {
        let height = row.height();
        let row_end = row_start.saturating_add(height);
        if row_end > offset && row_start < offset.saturating_add(session.visible_height) {
            let clipped_top = offset.saturating_sub(row_start);
            let y = body
                .y
                .saturating_add(u16::try_from(row_start.saturating_sub(offset)).unwrap_or(0));
            let visible_height = height.saturating_sub(clipped_top).min(
                session
                    .visible_height
                    .saturating_sub(row_start.saturating_sub(offset)),
            );
            let rect = Rect::new(
                body.x,
                y,
                body.width,
                u16::try_from(visible_height).unwrap_or(u16::MAX),
            );
            render_row(
                frame,
                rect,
                row,
                state,
                session,
                locale,
                &mut select_overlays,
            );
            if let Some(id) = row.id() {
                hits.push(AddHitRegion {
                    area: rect,
                    target: id.clone(),
                });
            }
        }
        row_start = row_end;
    }
    for overlay in select_overlays {
        hits.extend(overlay);
    }
    hits.extend(render_footer(frame, chunks[1], state, locale));
    AddScreenGeometry {
        body,
        first_visible: offset,
        hits,
    }
}

#[derive(Clone, Debug)]
enum RenderRow {
    Input(AddTextField, String),
    Button(AddControlId, String),
    Check(AddControlId, String),
    Select(AddControlId, String),
    Note(String, Style),
}

impl RenderRow {
    const fn height(&self) -> usize {
        match self {
            Self::Input(..) | Self::Select(..) => 3,
            Self::Button(..) | Self::Check(..) | Self::Note(..) => 1,
        }
    }

    const fn id(&self) -> Option<&AddControlId> {
        match self {
            Self::Input(field, _) => Some(match field {
                AddTextField::SourcePath => &SOURCE_PATH_ID,
                AddTextField::CommandTemplate => &COMMAND_TEMPLATE_ID,
                AddTextField::CommandName => &COMMAND_NAME_ID,
                AddTextField::CommandDescription => &COMMAND_DESCRIPTION_ID,
                AddTextField::ReviewName => &REVIEW_NAME_ID,
                AddTextField::ReviewDescription => &REVIEW_DESCRIPTION_ID,
                AddTextField::Dependencies => &DEPENDENCIES_ID,
                AddTextField::PythonConstraint => &PYTHON_ID,
            }),
            Self::Button(id, _) | Self::Check(id, _) | Self::Select(id, _) => Some(id),
            Self::Note(..) => None,
        }
    }
}

const SOURCE_PATH_ID: AddControlId = AddControlId::Text(AddTextField::SourcePath);
const COMMAND_TEMPLATE_ID: AddControlId = AddControlId::Text(AddTextField::CommandTemplate);
const COMMAND_NAME_ID: AddControlId = AddControlId::Text(AddTextField::CommandName);
const COMMAND_DESCRIPTION_ID: AddControlId = AddControlId::Text(AddTextField::CommandDescription);
const REVIEW_NAME_ID: AddControlId = AddControlId::Text(AddTextField::ReviewName);
const REVIEW_DESCRIPTION_ID: AddControlId = AddControlId::Text(AddTextField::ReviewDescription);
const DEPENDENCIES_ID: AddControlId = AddControlId::Text(AddTextField::Dependencies);
const PYTHON_ID: AddControlId = AddControlId::Text(AddTextField::PythonConstraint);

fn build_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let mut rows = match state.stage() {
        AddStage::Source => source_rows(state, locale),
        AddStage::Kind => kind_rows(state, locale),
        AddStage::Review => review_rows(state, locale),
        AddStage::ConfirmDraftDelete => vec![
            RenderRow::Note(
                text(locale, "Remove this entry:").into_owned(),
                Style::default().fg(Color::Red),
            ),
            RenderRow::Button(
                AddControlId::DeleteDraft,
                text(locale, "Remove").into_owned(),
            ),
            RenderRow::Button(AddControlId::Cancel, text(locale, "Cancel").into_owned()),
        ],
        AddStage::Complete | AddStage::Cancelled => Vec::new(),
    };
    if let Some(problem) = state.problem() {
        rows.insert(
            0,
            RenderRow::Note(
                problem_text(problem, locale),
                Style::default().fg(Color::Red),
            ),
        );
    }
    if let Some(notice) = state.notice() {
        rows.insert(
            usize::from(state.problem().is_some()),
            RenderRow::Note(notice_text(notice, locale), Style::default().fg(ACCENT)),
        );
    }
    rows
}

fn source_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let source = state.source();
    let mut rows = vec![
        RenderRow::Note(
            text(locale, "Path to a script, executable, or prompt:").into_owned(),
            Style::default().add_modifier(Modifier::DIM),
        ),
        RenderRow::Input(
            AddTextField::SourcePath,
            text(locale, "Source path").into_owned(),
        ),
        RenderRow::Button(
            AddControlId::BrowseSource,
            text(locale, "Select").into_owned(),
        ),
        RenderRow::Input(
            AddTextField::CommandTemplate,
            text(locale, "Command template").into_owned(),
        ),
        RenderRow::Input(AddTextField::CommandName, text(locale, "Name").into_owned()),
        RenderRow::Input(
            AddTextField::CommandDescription,
            text(locale, "Description").into_owned(),
        ),
    ];
    for (index, draft) in source.listed_drafts().iter().enumerate() {
        rows.push(RenderRow::Button(
            AddControlId::Draft(index),
            draft
                .path
                .file_name()
                .map_or_else(String::new, |value| value.to_string_lossy().into_owned()),
        ));
    }
    if source.draft_overflow() > 0 {
        rows.push(RenderRow::Note(
            format_text(locale, "…and {} more", &[&source.draft_overflow()]),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    rows.extend([
        RenderRow::Button(
            AddControlId::NewScript,
            text(locale, "Write a script…").into_owned(),
        ),
        RenderRow::Button(
            AddControlId::NewPrompt,
            text(locale, "Draft a prompt…").into_owned(),
        ),
    ]);
    if !source.listed_drafts().is_empty() {
        rows.push(RenderRow::Button(
            AddControlId::DeleteDraft,
            text(locale, "Delete draft…").into_owned(),
        ));
    }
    rows.extend([
        RenderRow::Button(
            AddControlId::Continue,
            text(locale, "Continue").into_owned(),
        ),
        RenderRow::Button(AddControlId::Cancel, text(locale, "Cancel").into_owned()),
    ]);
    rows
}

fn kind_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let Some(picker) = state.kind_picker() else {
        return Vec::new();
    };
    let mut rows = vec![RenderRow::Note(
        format_text(
            locale,
            if picker.has_shebang() {
                "The #! in {} names no interpreter skit knows. What is it?"
            } else {
                "What is {}? skit can't tell from the name."
            },
            &[&picker.filename()],
        ),
        Style::default().fg(ACCENT),
    )];
    rows.extend(picker.choices().iter().enumerate().map(|(index, kind)| {
        RenderRow::Button(
            AddControlId::Kind(index),
            kind_label(locale, kind.as_str()).into_owned(),
        )
    }));
    rows.push(RenderRow::Button(
        AddControlId::Cancel,
        text(locale, "Cancel").into_owned(),
    ));
    rows
}

fn review_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let Some(review) = state.review() else {
        return Vec::new();
    };
    let mut rows = vec![
        RenderRow::Input(AddTextField::ReviewName, text(locale, "Name").into_owned()),
        RenderRow::Input(
            AddTextField::ReviewDescription,
            text(locale, "Description").into_owned(),
        ),
    ];
    if !review.is_fresh() && review.lane() != ReviewLane::Executable {
        rows.push(RenderRow::Select(
            AddControlId::Storage,
            text(locale, "Storage mode").into_owned(),
        ));
    }
    match review.dependency_surface() {
        DependencySurface::Python => {
            rows.push(RenderRow::Input(
                AddTextField::Dependencies,
                text(locale, "Package dependencies").into_owned(),
            ));
            rows.push(RenderRow::Input(
                AddTextField::PythonConstraint,
                text(locale, "Python constraint").into_owned(),
            ));
        }
        DependencySurface::PythonOwned(metadata) => {
            rows.push(RenderRow::Note(
                format_text(
                    locale,
                    "The script declares its own dependencies (PEP 723): {}",
                    &[&metadata.dependencies.join(", ")],
                ),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
        DependencySurface::Npm if review.storage() == StorageMode::Copy => {
            rows.push(RenderRow::Input(
                AddTextField::Dependencies,
                text(locale, "Package dependencies").into_owned(),
            ));
        }
        DependencySurface::Npm => {}
        DependencySurface::None => {}
    }
    if let Some(count) = review.modeled_cli_field_count()
        && count > 0
    {
        rows.push(RenderRow::Note(
            format_text(
                locale,
                if count == 1 {
                    "✓ skit read this script's own arguments ({} field). Running it opens a form — nothing to memorize."
                } else {
                    "✓ skit read this script's own arguments ({} fields). Running it opens a form — nothing to memorize."
                },
                &[&count],
            ),
            Style::default().fg(Color::Green),
        ));
    }
    if review.modeled_cli_field_count().is_none() && review.onboarding().uses_cli_framework() {
        rows.push(RenderRow::Note(
            format_text(
                locale,
                "This script parses its own arguments ({}); skit couldn't model them statically, so the run form offers an extra-arguments field.",
                &[&review.onboarding().frameworks.join(", ")],
            ),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if review.storage() == StorageMode::Copy && !review.candidates().is_empty() {
        rows.push(RenderRow::Note(
            text(locale, "Tick the ones the run form should ask for:").into_owned(),
            Style::default().add_modifier(Modifier::DIM),
        ));
        for candidate in review.candidates() {
            let mut label = candidate.declaration.name.clone();
            if candidate.declaration.binding == skit_domain::parameters::ParameterBinding::Input {
                label = format!(
                    "input() #{}: {}",
                    candidate.declaration.order + 1,
                    candidate.declaration.prompt
                );
            }
            if candidate.demoted {
                label.push_str(" ⚠");
            }
            rows.push(RenderRow::Check(
                AddControlId::Candidate(candidate.declaration.name.clone()),
                label,
            ));
            if candidate.demoted {
                rows.push(RenderRow::Note(
                    text(
                        locale,
                        "⚠ looks like a loop accumulator — probably not a parameter",
                    )
                    .into_owned(),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }
    }
    if !review.onboarding().filename_literals.is_empty() {
        let literals = review
            .onboarding()
            .filename_literals
            .iter()
            .map(|literal| format!("'{literal}'"))
            .collect::<Vec<_>>()
            .join(", ");
        rows.push(RenderRow::Note(
            format_text(
                locale,
                "💡 {} are written directly inside the code, so skit can't turn them into form fields. To manage one, first give it a name at the top of the script, e.g. OUTPUT = '…' (Ctrl+E edits it now).",
                &[&literals],
            ),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if review.storage() == StorageMode::Reference && review.lane() == ReviewLane::Script {
        rows.push(RenderRow::Note(
            text(
                locale,
                if review
                    .modeled_cli_field_count()
                    .is_some_and(|count| count > 0)
                {
                    "Link the original: skit never writes to the file."
                } else {
                    "Link the original: parameter setup is skipped — skit never writes to the file."
                },
            )
            .into_owned(),
            Style::default().add_modifier(Modifier::DIM),
        ));
        if matches!(review.dependency_surface(), DependencySurface::Npm) {
            rows.push(RenderRow::Note(
                text(
                    locale,
                    "npm dependencies apply to stored copies only, so none are recorded.",
                )
                .into_owned(),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }
    }
    if review.onboarding().uses_argv && !review.onboarding().uses_cli_framework() {
        rows.push(RenderRow::Note(
            text(
                locale,
                "This script reads command-line arguments; the run form has an extra-arguments field for them.",
            )
            .into_owned(),
            Style::default().add_modifier(Modifier::DIM),
        ));
    }
    if review.lane() == ReviewLane::Prompt {
        rows.push(RenderRow::Check(
            AddControlId::Interpolate,
            text(locale, "Prompt interpolation (true or false)").into_owned(),
        ));
        if review.interpolate() {
            if review.prompt_candidates().is_empty() {
                rows.push(RenderRow::Note(
                    text(
                        locale,
                        "No {{name}} placeholders detected — the body travels to the agent as written.",
                    )
                    .into_owned(),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            } else if review.prompt_is_flooded() {
                rows.push(RenderRow::Note(
                    format_text(
                        locale,
                        "Detected {} placeholders — probably not written for insertion. Tick only the ones you need, or untick the switch above.",
                        &[&review.prompt_candidates().len()],
                    ),
                    Style::default().fg(Color::Yellow),
                ));
            } else {
                rows.push(RenderRow::Note(
                    text(locale, "Tick the ones the run form should ask for:").into_owned(),
                    Style::default().add_modifier(Modifier::DIM),
                ));
            }
            for candidate in review.prompt_preview() {
                rows.push(RenderRow::Check(
                    AddControlId::PromptCandidate(candidate.name.clone()),
                    if candidate.secret {
                        format!("{}{}", candidate.name, text(locale, " (secret)"))
                    } else {
                        candidate.name.clone()
                    },
                ));
            }
            if review.prompt_candidates().len() > review.prompt_preview().len() {
                rows.push(RenderRow::Button(
                    AddControlId::Continue,
                    text(locale, "Choose variables…").into_owned(),
                ));
            }
        }
        rows.push(RenderRow::Select(
            AddControlId::Runner,
            text(locale, "Prompt runner").into_owned(),
        ));
        rows.push(RenderRow::Button(
            AddControlId::NewRunner,
            format!("{} {}", text(locale, "Add"), text(locale, "Runner")),
        ));
    }
    if review.lane() != ReviewLane::Executable {
        rows.push(RenderRow::Button(
            AddControlId::EditSource,
            text(locale, "Edit").into_owned(),
        ));
    }
    rows.extend([
        RenderRow::Button(AddControlId::Save, text(locale, "Save").into_owned()),
        RenderRow::Button(AddControlId::Cancel, text(locale, "Cancel").into_owned()),
    ]);
    rows
}

fn render_row(
    frame: &mut Frame,
    area: Rect,
    row: &RenderRow,
    state: &AddWorkflowState,
    session: &mut AddScreenSession,
    locale: Locale,
    overlays: &mut Vec<Vec<AddHitRegion>>,
) {
    match row {
        RenderRow::Input(field, label) => {
            if let Some(input) = session.inputs.get(field) {
                render_line_input(
                    frame,
                    area,
                    input,
                    false,
                    session.focus.is_focused(&AddControlId::Text(*field)),
                    label,
                );
            }
        }
        RenderRow::Button(id, label) => {
            let focused = session.focus.is_focused(id);
            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        if focused { "▶ " } else { "  " },
                        Style::default().fg(ACCENT),
                    ),
                    Span::styled(
                        label,
                        if focused {
                            Style::default()
                                .fg(SELECT_FG)
                                .bg(SELECT_BG)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        },
                    ),
                ])),
                area,
            );
        }
        RenderRow::Check(id, label) => {
            if let Some(check) = session.checks.get(id) {
                frame.render_widget(CheckBox::new(label, check), area);
            }
        }
        RenderRow::Select(id, label) => {
            let (options, select_state) = if *id == AddControlId::Storage {
                (storage_options(state, locale), &session.storage)
            } else {
                let mut options = vec![text(locale, "ask on the run form").into_owned()];
                options.extend(
                    state
                        .review()
                        .into_iter()
                        .flat_map(|review| review.runner_names().iter().cloned()),
                );
                (options, &session.runner)
            };
            let select = Select::new(&options, select_state).label(label);
            select.render_stateful(frame, area);
            if select_state.is_open {
                let select = Select::new(&options, select_state).label(label);
                let regions = select.render_dropdown(frame, area, frame.area());
                overlays.push(
                    regions
                        .into_iter()
                        .filter_map(|region| match region.data {
                            SelectAction::Select(index) => Some(AddHitRegion {
                                area: region.area,
                                target: if *id == AddControlId::Storage {
                                    AddControlId::StorageOption(index)
                                } else {
                                    AddControlId::RunnerOption(index)
                                },
                            }),
                            SelectAction::Focus | SelectAction::Open | SelectAction::Close => None,
                        })
                        .collect(),
                );
            }
        }
        RenderRow::Note(message, style) => {
            frame.render_widget(Paragraph::new(message.as_str()).style(*style), area);
        }
    }
}

fn signature(state: &AddWorkflowState) -> AddSignature {
    AddSignature {
        stage: state.stage(),
        kind: state.review().map(|review| review.kind()),
        storage: state.review().map(|review| review.storage()),
        dependency_surface: state
            .review()
            .map(|review| review.dependency_surface().clone()),
        interpolate: state.review().map(|review| review.interpolate()),
        drafts: state.source().listed_drafts().len(),
        candidates: state.review().map_or_else(Vec::new, |review| {
            review
                .candidates()
                .iter()
                .map(|candidate| candidate.declaration.name.clone())
                .collect()
        }),
        prompt_candidates: state.review().map_or_else(Vec::new, |review| {
            review
                .prompt_preview()
                .iter()
                .map(|candidate| candidate.name.clone())
                .collect()
        }),
        runners: state
            .review()
            .map_or_else(Vec::new, |review| review.runner_names().to_vec()),
    }
}

fn text_action(field: AddTextField, value: &str) -> AddAction {
    match field {
        AddTextField::SourcePath => AddAction::SetSourcePath(value.to_owned()),
        AddTextField::CommandTemplate => AddAction::SetCommandTemplate(value.to_owned()),
        AddTextField::CommandName => AddAction::SetCommandName(value.to_owned()),
        AddTextField::CommandDescription => AddAction::SetCommandDescription(value.to_owned()),
        AddTextField::ReviewName => AddAction::SetReviewName(value.to_owned()),
        AddTextField::ReviewDescription => AddAction::SetReviewDescription(value.to_owned()),
        AddTextField::Dependencies => AddAction::SetReviewDependencies(value.to_owned()),
        AddTextField::PythonConstraint => AddAction::SetReviewPython(value.to_owned()),
    }
}

fn checkbox_action(id: &AddControlId, selected: bool) -> Option<AddAction> {
    match id {
        AddControlId::Candidate(name) => Some(AddAction::SetReviewCandidate {
            name: name.clone(),
            selected,
        }),
        AddControlId::Interpolate => Some(AddAction::SetPromptInterpolation(selected)),
        AddControlId::PromptCandidate(name) => Some(AddAction::SetPromptCandidate {
            name: name.clone(),
            selected,
        }),
        _ => None,
    }
}

#[derive(Debug)]
struct AddFooterChip {
    key: &'static str,
    label: String,
    target: AddControlId,
}

fn footer_chips(state: &AddWorkflowState, locale: Locale) -> Vec<AddFooterChip> {
    let chip = |key, label: String, target| AddFooterChip { key, label, target };
    match state.stage() {
        AddStage::Source => {
            let mut chips = vec![
                chip(
                    "Enter",
                    text(locale, "Continue").into_owned(),
                    AddControlId::Continue,
                ),
                chip(
                    "Esc",
                    text(locale, "Cancel").into_owned(),
                    AddControlId::Cancel,
                ),
                chip(
                    "Ctrl+N",
                    text(locale, "Write a script…").into_owned(),
                    AddControlId::NewScript,
                ),
                chip(
                    "Ctrl+P",
                    text(locale, "Draft a prompt…").into_owned(),
                    AddControlId::NewPrompt,
                ),
            ];
            if !state.source().listed_drafts().is_empty() {
                chips.push(chip(
                    "Ctrl+D",
                    text(locale, "Delete draft…").into_owned(),
                    AddControlId::DeleteDraft,
                ));
            }
            chips.push(chip(
                "Tab",
                text(locale, "Next field").into_owned(),
                AddControlId::NextField,
            ));
            chips
        }
        AddStage::Review => {
            let mut chips = vec![
                chip(
                    "Ctrl+S",
                    text(locale, "Add").into_owned(),
                    AddControlId::Save,
                ),
                chip(
                    "Esc",
                    text(locale, "Cancel").into_owned(),
                    AddControlId::Cancel,
                ),
            ];
            if state.review().is_some_and(|review| {
                review.lane() == ReviewLane::Prompt
                    || (review.storage() == StorageMode::Copy && !review.candidates().is_empty())
            }) {
                chips.push(chip(
                    "Space",
                    text(locale, "Toggle").into_owned(),
                    AddControlId::ToggleFocused,
                ));
            }
            if state
                .review()
                .is_some_and(|review| review.lane() != ReviewLane::Executable)
            {
                chips.push(chip(
                    "Ctrl+E",
                    text(
                        locale,
                        if state
                            .review()
                            .is_some_and(|review| review.lane() == ReviewLane::Prompt)
                        {
                            "Edit prompt"
                        } else {
                            "Edit script"
                        },
                    )
                    .into_owned(),
                    AddControlId::EditSource,
                ));
            }
            chips.push(chip(
                "Tab",
                text(locale, "Next field").into_owned(),
                AddControlId::NextField,
            ));
            chips
        }
        AddStage::Kind => vec![
            chip(
                "Enter",
                text(locale, "Select").into_owned(),
                AddControlId::PickFocusedKind,
            ),
            chip(
                "Esc",
                text(locale, "Cancel").into_owned(),
                AddControlId::Cancel,
            ),
        ],
        AddStage::ConfirmDraftDelete => vec![
            chip(
                "Enter",
                text(locale, "Remove").into_owned(),
                AddControlId::DeleteDraft,
            ),
            chip(
                "Esc",
                text(locale, "Cancel").into_owned(),
                AddControlId::Cancel,
            ),
        ],
        AddStage::Complete | AddStage::Cancelled => Vec::new(),
    }
}

fn render_footer(
    frame: &mut Frame,
    area: Rect,
    state: &AddWorkflowState,
    locale: Locale,
) -> Vec<AddHitRegion> {
    if area.is_empty() {
        return Vec::new();
    }
    let mut row = 0_u16;
    let mut x = 0_u16;
    let mut hits = Vec::new();
    for chip in footer_chips(state, locale) {
        let label = format!("[{}] {}", chip.key, chip.label);
        let desired = u16::try_from(label.width().saturating_add(1))
            .unwrap_or(u16::MAX)
            .min(area.width);
        if x > 0 && x.saturating_add(desired) > area.width {
            row = row.saturating_add(1);
            x = 0;
        }
        if row >= area.height {
            break;
        }
        let width = desired.min(area.width.saturating_sub(x));
        if width == 0 {
            continue;
        }
        let chip_area = Rect::new(area.x.saturating_add(x), area.y + row, width, 1);
        frame.render_widget(
            Paragraph::new(label).style(Style::default().add_modifier(Modifier::DIM)),
            chip_area,
        );
        hits.push(AddHitRegion {
            area: chip_area,
            target: chip.target,
        });
        x = x.saturating_add(width).saturating_add(1);
    }
    hits
}

fn storage_options(state: &AddWorkflowState, locale: Locale) -> Vec<String> {
    let reference = if state
        .review()
        .is_some_and(|review| review.lane() == ReviewLane::Prompt)
    {
        "Link the original — edits take effect immediately; skit never writes to the file"
    } else {
        "Link the original — edits take effect immediately, but skit won't write to the file, so parameter definitions are yours to maintain"
    };
    vec![
        text(
            locale,
            "Keep a copy — skit stores it; your original file is never modified",
        )
        .into_owned(),
        text(locale, reference).into_owned(),
    ]
}

fn problem_text(problem: &AddProblem, locale: Locale) -> String {
    match problem {
        AddProblem::SourceUnavailable { path, reason } => {
            format!(
                "{}: {} ({reason})",
                text(locale, "Source path"),
                path.display()
            )
        }
        AddProblem::MissingCommandName => {
            format!("{}: {}", text(locale, "error"), text(locale, "Name"))
        }
        AddProblem::InvalidKind => {
            format!("{}: {}", text(locale, "error"), text(locale, "Kind"))
        }
        AddProblem::InvalidPromptEncoding => text(
            locale,
            "invalid UTF-8 was detected in one or more arguments",
        )
        .into_owned(),
        AddProblem::InvalidDependency { value } => {
            format!("{}: {}", text(locale, "Package dependencies"), value)
        }
        AddProblem::InvalidPythonConstraint { value } => {
            format!("{}: {}", text(locale, "Python constraint"), value)
        }
        AddProblem::SourceEdit { reason }
        | AddProblem::CommitFailed { reason }
        | AddProblem::EditFailed { reason }
        | AddProblem::DraftDeleteFailed { reason } => reason.clone(),
    }
}

fn notice_text(notice: &AddNotice, locale: Locale) -> String {
    match notice {
        AddNotice::NothingWritten => {
            text(locale, "Nothing was written, so nothing was added.").into_owned()
        }
        AddNotice::DraftKept(path) => {
            format_text(locale, "Your draft was kept at {}", &[&path.display()])
        }
        AddNotice::DraftDeleted(path) => {
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            format_text(locale, "Deleted the draft {}.", &[&name])
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::KeyEvent;
    use skit_application::SourcePermissions;
    use skit_ui::{AddEffect, ReviewDefaults, ReviewState, SourceSnapshot};

    use super::*;

    fn source(path: &str, bytes: &[u8], kind: KnownEntryKind) -> AddWorkflowState {
        AddWorkflowState::from_review(ReviewState::from_source(
            SourceSnapshot {
                path: PathBuf::from(path),
                source_record: path.to_owned(),
                bytes: bytes.to_vec(),
                permissions: SourcePermissions::default(),
                is_regular: true,
                is_directory: false,
                is_draft: false,
            },
            kind,
            ReviewDefaults::default(),
        ))
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn draw(
        state: &AddWorkflowState,
        session: &mut AddScreenSession,
        width: u16,
        height: u16,
    ) -> (Terminal<TestBackend>, AddScreenGeometry) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut geometry = AddScreenGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_add(frame, frame.area(), state, session, Locale::En);
            })
            .unwrap();
        (terminal, geometry)
    }

    fn text_of(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn source_keyboard_and_browse_mouse_have_positive_typed_paths() {
        let mut state = AddWorkflowState::new(Vec::new());
        let mut session = AddScreenSession::default();
        let (terminal, geometry) = draw(&state, &mut session, 80, 24);
        assert!(text_of(&terminal).contains("Command template"));
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry,),
            Some(AddScreenEvent::Action(AddAction::Continue)),
            "the advertised Enter chip must continue directly from the source input",
        );
        let browse = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::BrowseSource)
            .unwrap();
        let mouse = Event::Mouse(ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
            column: browse.area.x,
            row: browse.area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(matches!(
            session.handle_event(mouse, &state, &geometry),
            Some(AddScreenEvent::OpenPathPicker(_))
        ));
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char('n'), KeyModifiers::CONTROL),
                &state,
                &geometry
            ),
            Some(AddScreenEvent::Action(AddAction::NewDraft(
                DraftKind::Script
            )))
        );
        let write_script = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::NewScript)
            .unwrap();
        let mouse = Event::Mouse(ratatui_crossterm::crossterm::event::MouseEvent {
            kind: MouseEventKind::Down(ratatui_crossterm::crossterm::event::MouseButton::Left),
            column: write_script.area.x,
            row: write_script.area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(
            session.handle_event(mouse, &state, &geometry),
            Some(AddScreenEvent::Action(AddAction::NewDraft(
                DraftKind::Script
            )))
        );

        let effects = state.reduce(AddAction::NewDraft(DraftKind::Script));
        let [AddEffect::AuthorDraft { request, .. }] = effects.as_slice() else {
            panic!("new draft must open the editor");
        };
        let request = *request;
        let _ = state.reduce(AddAction::DraftEdited {
            request,
            result: Ok(None),
        });
        let (terminal, _) = draw(&state, &mut session, 80, 24);
        assert!(text_of(&terminal).contains("Nothing was written, so nothing was added."));
        assert_eq!(
            notice_text(
                &AddNotice::DraftKept(PathBuf::from("skit-new-task.py")),
                Locale::En,
            ),
            "Your draft was kept at skit-new-task.py"
        );
        assert_eq!(
            notice_text(
                &AddNotice::DraftDeleted(PathBuf::from("skit-new-task.py")),
                Locale::En,
            ),
            "Deleted the draft skit-new-task.py."
        );
    }

    #[test]
    fn review_candidate_space_and_save_shortcut_emit_reducer_actions() {
        let state = source(
            "tool.py",
            b"VALUE = 1\nprint(VALUE)\n",
            KnownEntryKind::Python,
        );
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw(&state, &mut session, 80, 20);
        while !matches!(session.focused(), Some(AddControlId::Candidate(_))) {
            assert_eq!(
                session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &state, &geometry),
                Some(AddScreenEvent::Changed)
            );
        }
        assert!(matches!(
            session.handle_event(
                key(KeyCode::Char(' '), KeyModifiers::NONE),
                &state,
                &geometry
            ),
            Some(AddScreenEvent::Action(AddAction::SetReviewCandidate {
                selected: false,
                ..
            }))
        ));
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char('s'), KeyModifiers::CONTROL),
                &state,
                &geometry
            ),
            Some(AddScreenEvent::Action(AddAction::Save))
        );
        while session.focused() != Some(&AddControlId::Save) {
            let _ = session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &state, &geometry);
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry),
            Some(AddScreenEvent::Action(AddAction::Save)),
            "Enter activates the focused mature Save button",
        );
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char('e'), KeyModifiers::CONTROL),
                &state,
                &geometry
            ),
            Some(AddScreenEvent::Action(AddAction::EditSource))
        );
    }

    #[test]
    fn reference_review_folds_unavailable_controls_and_explains_the_result() {
        let mut state = source(
            "tool.js",
            b"const OUTPUT = 'x';\nimport chalk from 'chalk';\nconsole.log(OUTPUT, chalk);\n",
            KnownEntryKind::JavaScript,
        );
        let _ = state.reduce(AddAction::SetReviewStorage(StorageMode::Reference));
        let mut session = AddScreenSession::default();
        let (terminal, _) = draw(&state, &mut session, 120, 30);
        let rendered = text_of(&terminal);

        assert!(!rendered.contains("Package dependencies"));
        assert!(
            !session
                .focus
                .elements()
                .iter()
                .any(|id| matches!(id, AddControlId::Candidate(_)))
        );
        assert!(rendered.contains("parameter setup is skipped"));
        assert!(rendered.contains("npm dependencies apply to stored copies only"));
    }

    #[test]
    fn modeled_reference_keeps_the_reader_notice_without_claiming_the_form_was_lost() {
        let mut state = source(
            "tool.py",
            b"import argparse\np = argparse.ArgumentParser()\np.add_argument('--name')\np.parse_args()\n",
            KnownEntryKind::Python,
        );
        let _ = state.reduce(AddAction::SetReviewStorage(StorageMode::Reference));
        let mut session = AddScreenSession::default();
        let (terminal, _) = draw(&state, &mut session, 120, 30);
        let rendered = text_of(&terminal);

        assert!(rendered.contains("skit read this script's own arguments"));
        assert!(rendered.contains("Link the original: skit never writes to the file."));
        assert!(!rendered.contains("parameter setup is skipped"));
    }

    #[test]
    fn disabling_prompt_interpolation_folds_the_placeholder_controls_without_losing_state() {
        let mut state = source(
            "task.prompt.md",
            b"Review {{topic}} with {{api_key}}.",
            KnownEntryKind::Prompt,
        );
        let _ = state.reduce(AddAction::SetPromptInterpolation(false));
        let mut session = AddScreenSession::default();
        let (terminal, _) = draw(&state, &mut session, 90, 22);

        assert!(text_of(&terminal).contains("Prompt interpolation"));
        assert!(
            !session
                .focus
                .elements()
                .contains(&AddControlId::PromptCandidate("topic".to_owned()))
        );
        assert_eq!(
            state.review().unwrap().selected_prompt_names(),
            vec!["topic", "api_key"]
        );
    }

    #[test]
    fn every_visible_primary_footer_action_has_a_mouse_twin() {
        let state = source(
            "tool.py",
            b"VALUE = 1\nprint(VALUE)\n",
            KnownEntryKind::Python,
        );
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw(&state, &mut session, 100, 20);
        for (target, expected) in [
            (AddControlId::Save, AddScreenEvent::Action(AddAction::Save)),
            (
                AddControlId::EditSource,
                AddScreenEvent::Action(AddAction::EditSource),
            ),
            (
                AddControlId::Cancel,
                AddScreenEvent::Action(AddAction::Cancel),
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
            assert_eq!(
                session.handle_event(mouse, &state, &geometry),
                Some(expected)
            );
        }
    }

    #[test]
    fn prompt_review_exposes_interpolation_runner_and_complete_picker() {
        let prompt = (0..25)
            .map(|index| format!("{{{{h{index}}}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        let state = source("task.prompt.md", prompt.as_bytes(), KnownEntryKind::Prompt);
        let mut session = AddScreenSession::default();
        let (terminal, geometry) = draw(&state, &mut session, 90, 18);
        let rendered = text_of(&terminal);
        assert!(rendered.contains("Prompt interpolation"));
        assert!(session.focus.elements().contains(&AddControlId::Runner));
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char('o'), KeyModifiers::CONTROL),
                &state,
                &geometry
            ),
            Some(AddScreenEvent::OpenPromptCandidates)
        );
    }

    #[test]
    fn tiny_test_backend_scrolls_focus_and_keeps_cancel_mouse_target() {
        let state = source(
            "tool.py",
            b"A = 1\nB = 2\nC = 3\nD = 4\nprint(A, B, C, D)\n",
            KnownEntryKind::Python,
        );
        let mut session = AddScreenSession::default();
        let (_, first) = draw(&state, &mut session, 36, 8);
        for _ in 0..session.focus.elements().len() {
            if session.focused() == Some(&AddControlId::Save) {
                break;
            }
            let _ = session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &state, &first);
        }
        let (_, second) = draw(&state, &mut session, 36, 8);
        assert!(second.first_visible > 0);
        assert!(
            second
                .hits
                .iter()
                .any(|hit| hit.target == AddControlId::Cancel)
        );
    }
}
