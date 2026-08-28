//! Add, source-review, executable-review, and prompt-review widgets.

use std::{
    cmp::Ordering,
    collections::BTreeMap,
    hash::Hash,
    num::{NonZeroU16, NonZeroUsize},
    path::PathBuf,
};

use ratatui_core::{
    layout::Rect,
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui_interact::{
    components::{
        CheckBox, CheckBoxState, ListPickerState, ScrollableContentState, Select, SelectAction,
        SelectState, SelectStyle, handle_scrollable_content_key, handle_scrollable_content_mouse,
        handle_select_key,
    },
    state::FocusManager,
};
use ratatui_widgets::paragraph::{Paragraph, Wrap};
use skit_domain::StorageMode;
use skit_i18n::{Locale, format_text, kind_choice_label, text};
use skit_ui::{
    AddAction, AddNotice, AddProblem, AddStage, AddWorkflowState, DependencySurface, DraftKind,
    KnownEntryKind, PathOutputPolicy, PathPickerState, PathSelectionMode, PickerPurpose,
    ReviewLane,
};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    footer::handle_footer_scroll,
    pointer::{ClickDispatch, ClickOutcome, ClickTracker, EditableGeometry},
    rowclip::RowClip,
    session::render_line_input_band,
    theme::{ACCENT, BOX_MAROON, SELECT_BG, SELECT_FG, panel_block},
    viewport::AlignmentSignature,
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
    /// Move focus to the previous field from the footer.
    PreviousField,
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
    footer_scroll: ScrollableContentState,
    footer_viewport: Rect,
    footer_visible_height: usize,
    row_spans: BTreeMap<AddControlId, (usize, usize)>,
    editables: BTreeMap<AddTextField, EditableGeometry>,
    alignment: Option<AlignmentSignature<AddControlId, (usize, usize, usize)>>,
    click: ClickTracker<AddControlId>,
}

impl AddScreenSession {
    pub(crate) fn cancel_click(&mut self) {
        self.click.cancel();
    }

    /// Current typed focus.
    #[must_use]
    pub fn focused(&self) -> Option<&AddControlId> {
        self.focus.current()
    }

    /// Report a focus landing to the reducer when it is product state.
    ///
    /// Version 0.4 deletes the highlighted draft, and the highlight follows the
    /// keyboard into the list with no activation step (`OptionList.highlighted`,
    /// `src/skit/tui_add.py:481-490`). Landing on a draft row therefore names
    /// the row to the reducer; every other landing only repaints.
    fn focus_event(&self) -> AddScreenEvent {
        match self.focus.current() {
            Some(AddControlId::Draft(index)) => {
                AddScreenEvent::Action(AddAction::HighlightDraft(*index))
            }
            _ => AddScreenEvent::Changed,
        }
    }

    /// Synchronize widgets from durable reducer state only when the control shape changes.
    pub fn sync(&mut self, state: &AddWorkflowState) {
        let signature = signature(state);
        if self.signature.as_ref() == Some(&signature) {
            self.sync_values(state);
            return;
        }
        self.click.cancel();
        self.signature = Some(signature);
        self.focus.clear();
        self.inputs.clear();
        self.checks.clear();
        self.row_spans.clear();
        self.footer_scroll = ScrollableContentState::default();
        match state.stage() {
            AddStage::Source => {
                let source = state.source();
                self.insert_input(AddTextField::SourcePath, &source.path);
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
                match source.listed_drafts() {
                    [] => {}
                    [_first, ..] => self.focus.register(AddControlId::DeleteDraft),
                }
                self.focus
                    .register_all([AddControlId::Continue, AddControlId::Cancel]);
            }
            AddStage::Kind => {
                let picker = state
                    .kind_picker()
                    .expect("the typed Kind stage owns its picker");
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
                self.focus.register(AddControlId::Cancel);
                self.focus
                    .set(AddControlId::Kind(self.kind_picker.selected_index));
            }
            AddStage::Review => {
                let review = state
                    .review()
                    .expect("the typed Review stage owns its review state");
                self.insert_input(AddTextField::ReviewName, review.name());
                self.insert_input(AddTextField::ReviewDescription, review.description());
                if let (false, ReviewLane::Script | ReviewLane::Prompt) =
                    (review.is_fresh(), review.lane())
                {
                    self.focus.register(AddControlId::Storage);
                    self.storage = SelectState::with_selected(
                        2,
                        usize::from(matches!(review.storage(), StorageMode::Reference)),
                    );
                }
                match (review.dependency_surface(), review.storage()) {
                    (DependencySurface::Python, _) => {
                        self.insert_input(AddTextField::Dependencies, review.dependencies_text());
                        self.insert_input(AddTextField::PythonConstraint, review.requires_python());
                    }
                    (DependencySurface::Npm, StorageMode::Copy) => {
                        self.insert_input(AddTextField::Dependencies, review.dependencies_text());
                    }
                    (
                        DependencySurface::None
                        | DependencySurface::Npm
                        | DependencySurface::PythonOwned(_),
                        _,
                    ) => {}
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
                        .position(|runner| runner.as_str().eq(review.runner()))
                        .map_or(0, |index| index.saturating_add(1));
                    self.runner = SelectState::with_selected(
                        review.runner_names().len().saturating_add(1),
                        runner_index,
                    );
                    self.focus.register(AddControlId::NewRunner);
                }
                if let (true, true) = (review.interpolate(), has_more_prompt_candidates(review)) {
                    self.focus.register(AddControlId::Continue);
                }
                if matches!(review.lane(), ReviewLane::Script | ReviewLane::Prompt) {
                    self.focus.register(AddControlId::EditSource);
                }
                self.focus
                    .register_all([AddControlId::Save, AddControlId::Cancel]);
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
            if handle_footer_scroll(
                &mut self.footer_scroll,
                mouse,
                self.footer_viewport,
                self.footer_visible_height,
            ) {
                return Some(AddScreenEvent::Changed);
            }
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
            let target = geometry
                .hits
                .iter()
                .rev()
                .find(|hit| hit.area.contains((mouse.column, mouse.row).into()))
                .map(|hit| &hit.target);
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(AddControlId::Text(field)) = target
                && let Some(editable) = self.editables.get(field).copied()
                && let Some(input) = self.inputs.get_mut(field)
            {
                let _ = editable.place_cursor(input, mouse.column, mouse.row);
            }
            let ClickDispatch::Captured(outcome) = self.click.dispatch(mouse, target) else {
                return None;
            };
            let target = match outcome {
                ClickOutcome::Activated(target) => target,
                ClickOutcome::Armed => return Some(AddScreenEvent::Changed),
                ClickOutcome::Ignored => return None,
            };
            if !matches!(
                target,
                AddControlId::PickFocusedKind
                    | AddControlId::ToggleFocused
                    | AddControlId::NextField
                    | AddControlId::PreviousField
            ) {
                self.focus.set(target.clone());
                self.ensure_focus_visible();
            }
            return self.activate(target, state);
        }
        let Event::Key(key) = event else {
            return None;
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let text_focused = matches!(self.focus.current(), Some(AddControlId::Text(_)));
            // Editing keys belong to the focused mature Input. Ctrl+E moves to the end in a
            // review field. Ctrl+D deletes the next character in a source field.
            let input_owns_key = text_focused
                && matches!(
                    (key.code, state.stage()),
                    (KeyCode::Char('e'), AddStage::Review) | (KeyCode::Char('d'), AddStage::Source)
                );
            if !input_owns_key {
                return match (key.code, state.stage()) {
                    (KeyCode::Char('o'), AddStage::Source) => {
                        self.activate(AddControlId::BrowseSource, state)
                    }
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
                            review.interpolate() && has_more_prompt_candidates(review)
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
            return Some(self.focus_event());
        }
        if key.code == KeyCode::BackTab {
            self.focus.prev();
            self.ensure_focus_visible();
            return Some(self.focus_event());
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
                KeyCode::Esc => {
                    return Some(AddScreenEvent::Action(AddAction::PickKind(None)));
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
                if let (true, (KeyCode::Char('d'), AddStage::Source)) = (
                    key.modifiers.contains(KeyModifiers::CONTROL),
                    (key.code, state.stage()),
                ) {
                    let _ = input.handle(InputRequest::DeleteNextChar);
                    return Some(AddScreenEvent::Action(text_action(field, input.value())));
                }
                if input.handle_event(&Event::Key(key)).is_some() {
                    return Some(AddScreenEvent::Action(text_action(field, input.value())));
                }
            }
            if id == AddControlId::Storage {
                let action = handle_select_key(&key, &mut self.storage);
                if let Some(SelectAction::Select(index)) = action {
                    return Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                        match index {
                            0 => StorageMode::Copy,
                            _ => StorageMode::Reference,
                        },
                    )));
                }
                if action.is_some() {
                    return Some(AddScreenEvent::Changed);
                }
                if self.storage.is_open {
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
                if action.is_some() {
                    return Some(AddScreenEvent::Changed);
                }
                if self.runner.is_open {
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
        if key.code == KeyCode::Esc {
            return Some(AddScreenEvent::Action(match state.stage() {
                AddStage::ConfirmDraftDelete => AddAction::ConfirmDraftDelete(false),
                _ => AddAction::Cancel,
            }));
        }
        if matches!(state.stage(), AddStage::Source | AddStage::Review) {
            match key.code {
                KeyCode::Down => {
                    self.focus.next();
                    self.ensure_focus_visible();
                    return Some(self.focus_event());
                }
                KeyCode::Up => {
                    self.focus.prev();
                    self.ensure_focus_visible();
                    return Some(self.focus_event());
                }
                _ => {}
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
                    match index {
                        0 => StorageMode::Copy,
                        _ => StorageMode::Reference,
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
                Some(self.focus_event())
            }
            AddControlId::PreviousField => {
                self.focus.prev();
                self.ensure_focus_visible();
                Some(self.focus_event())
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

    fn sync_input_value(&mut self, field: AddTextField, value: &str) {
        let Some(input) = self.inputs.get_mut(&field) else {
            return;
        };
        if input.value() != value {
            *input = LineInput::new(value.to_owned());
        }
    }

    fn sync_values(&mut self, state: &AddWorkflowState) {
        match state.stage() {
            AddStage::Source => {
                let source = state.source();
                self.sync_input_value(AddTextField::SourcePath, &source.path);
                self.sync_input_value(AddTextField::CommandTemplate, &source.command_template);
                self.sync_input_value(AddTextField::CommandName, &source.command_name);
                self.sync_input_value(
                    AddTextField::CommandDescription,
                    &source.command_description,
                );
            }
            AddStage::Review => {
                let review = state
                    .review()
                    .expect("the typed Review stage owns its review state");
                self.sync_input_value(AddTextField::ReviewName, review.name());
                self.sync_input_value(AddTextField::ReviewDescription, review.description());
                self.sync_input_value(AddTextField::Dependencies, review.dependencies_text());
                self.sync_input_value(AddTextField::PythonConstraint, review.requires_python());
            }
            AddStage::Kind
            | AddStage::ConfirmDraftDelete
            | AddStage::Complete
            | AddStage::Cancelled => {}
        }
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
        let Some((row_start, row_height)) = self.row_spans.get(id).copied() else {
            return;
        };
        let row_end = row_start.saturating_add(row_height);
        let offset = self.scroll.scroll_offset();
        let aligned = match row_start.cmp(&offset) {
            Ordering::Less => row_start,
            Ordering::Equal | Ordering::Greater => {
                offset.max(row_end.saturating_sub(self.visible_height))
            }
        };
        self.scroll.set_scroll_offset(aligned);
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
    session.editables.clear();
    if area.is_empty() {
        session.viewport = area;
        session.visible_height = 0;
        session.footer_viewport = area;
        session.footer_visible_height = 0;
        return AddScreenGeometry {
            body: area,
            first_visible: session.scroll.scroll_offset(),
            hits: Vec::new(),
        };
    }
    let short = area.height < 14;
    let footer_height = if short { 1 } else { 2 }.min(area.height);
    let chunks = crate::viewport::Viewport::split_footer(area, footer_height);
    let title = match state.stage() {
        AddStage::Source => text(locale, "Add an entry").into_owned(),
        AddStage::Kind => state
            .kind_picker()
            .expect("the typed Kind stage owns its picker")
            .filename()
            .to_owned(),
        AddStage::Review => {
            let review = state
                .review()
                .expect("the typed Review stage owns its review state");
            let filename = review.source().path.file_name().map_or_else(
                || review.name().to_owned(),
                |name| name.to_string_lossy().into_owned(),
            );
            format!("{} {filename}", text(locale, "Add"))
        }
        AddStage::ConfirmDraftDelete => text(locale, "Confirm removal").into_owned(),
        AddStage::Complete | AddStage::Cancelled => text(locale, "Add").into_owned(),
    };
    let body_block = panel_block(title, BOX_MAROON);
    let body = body_block.inner(chunks[0]);
    frame.render_widget(body_block, chunks[0]);
    let rows = build_rows(state, locale);
    let total_height = rows.iter().map(|row| row.height(body.width)).sum::<usize>();
    session
        .scroll
        .set_lines(vec![String::new(); total_height.max(1)]);
    session.viewport = body;
    session.visible_height = usize::from(body.height);
    session.row_spans.clear();
    let mut logical = 0;
    for row in &rows {
        if let Some(id) = row.id() {
            session
                .row_spans
                .insert(id.clone(), (logical, row.height(body.width)));
        }
        logical = logical.saturating_add(row.height(body.width));
    }
    let focused = session.focus.current().cloned();
    let target = focused
        .as_ref()
        .and_then(|focused| session.row_spans.get(focused).copied());
    let reflow = target.map_or((total_height, 0, 0), |(start, height)| {
        (total_height, start, height)
    });
    let alignment_changed = focused.is_some_and(|focused| {
        AlignmentSignature::update(&mut session.alignment, focused, body, reflow)
    });
    if alignment_changed {
        session.ensure_focus_visible();
    }
    let maximum = total_height.saturating_sub(session.visible_height);
    session
        .scroll
        .set_scroll_offset(session.scroll.scroll_offset().min(maximum));
    let offset = session.scroll.scroll_offset();
    let mut hits = Vec::new();
    let mut row_start = 0_usize;
    for row in &rows {
        let height = row.height(body.width);
        let row_end = row_start.saturating_add(height);
        let visible_start = row_start.max(offset);
        let visible_end = row_end.min(offset.saturating_add(session.visible_height));
        if let Some(visible_height) = NonZeroUsize::new(visible_end.saturating_sub(visible_start)) {
            let clipped_top = visible_start.saturating_sub(row_start);
            let y = body.y.saturating_add(
                u16::try_from(visible_start.saturating_sub(offset))
                    .expect("the Add row starts inside its viewport"),
            );
            let rect = Rect::new(
                body.x,
                y,
                body.width,
                u16::try_from(visible_height.get()).expect("the Add row band fits its viewport"),
            );
            let clip = RowClip::new(height, clipped_top, rect);
            render_row(frame, clip, row, state, session, locale);
            if let Some(id) = row.id() {
                hits.push(AddHitRegion {
                    area: clip.area(),
                    target: id.clone(),
                });
            }
        }
        row_start = row_end;
    }
    hits.extend(render_footer(frame, chunks[1], state, session, locale));
    for select in [AddSelectControl::Storage, AddSelectControl::Runner] {
        let Some(area) = hits
            .iter()
            .find(|hit| &hit.target == select.control_id())
            .map(|hit| hit.area)
        else {
            continue;
        };
        hits.extend(render_select_overlay(
            frame, area, select, state, session, locale,
        ));
    }
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
    Select(AddSelectControl, String),
    Note(String, Style),
}

#[derive(Clone, Copy, Debug)]
enum AddSelectControl {
    Storage,
    Runner,
}

impl AddSelectControl {
    const fn control_id(self) -> &'static AddControlId {
        match self {
            Self::Storage => &STORAGE_ID,
            Self::Runner => &RUNNER_ID,
        }
    }

    const fn option_id(self, index: usize) -> AddControlId {
        match self {
            Self::Storage => AddControlId::StorageOption(index),
            Self::Runner => AddControlId::RunnerOption(index),
        }
    }
}

impl RenderRow {
    fn height(&self, width: u16) -> usize {
        match self {
            Self::Input(..) | Self::Select(..) => 3,
            Self::Button(..) | Self::Check(..) => 1,
            Self::Note(message, _) => Paragraph::new(message.as_str())
                .wrap(Wrap { trim: false })
                .line_count(width.max(1))
                .max(1),
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
            Self::Button(id, _) | Self::Check(id, _) => Some(id),
            Self::Select(select, _) => Some(select.control_id()),
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
const STORAGE_ID: AddControlId = AddControlId::Storage;
const RUNNER_ID: AddControlId = AddControlId::Runner;

fn build_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let mut rows = match state.stage() {
        AddStage::Source => source_rows(state, locale),
        AddStage::Kind => kind_rows(state, locale),
        AddStage::Review => review_rows(state, locale),
        AddStage::ConfirmDraftDelete => confirm_draft_delete_rows(state, locale),
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
            format!("[Ctrl+O] {}", text(locale, "Select")),
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
    if let Some(overflow) = NonZeroUsize::new(source.draft_overflow()) {
        rows.push(RenderRow::Note(
            format_text(locale, "…and {} more", &[&overflow.get()]),
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
    let picker = state
        .kind_picker()
        .expect("the typed Kind stage owns its picker");
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
            kind_choice_label(locale, kind.as_str()).into_owned(),
        )
    }));
    rows.push(RenderRow::Button(
        AddControlId::Cancel,
        text(locale, "Cancel").into_owned(),
    ));
    rows
}

/// Name one kept draft the way its row in the list names it.
fn draft_display_name(draft: &skit_ui::DraftSummary) -> String {
    draft
        .path
        .file_name()
        .map_or_else(String::new, |value| value.to_string_lossy().into_owned())
}

/// One dim guidance line under the control it explains.
///
/// Version 0.4 writes these as `hint`-classed statics beside the field they describe
/// (`src/skit/tui_add.py:925`, `:963`), so the reader learns what the field takes without leaving
/// the screen.
fn hint(body: String) -> RenderRow {
    RenderRow::Note(body, Style::default().add_modifier(Modifier::DIM))
}

/// Rows for the kept-draft delete confirmation.
///
/// The reducer keeps the candidate for the whole stage, so the confirmation always names its
/// draft. A kept draft is a file, not a library entry, and deleting it is not undoable: version
/// 0.4 says both (`src/skit/tui_add.py:176`).
fn confirm_draft_delete_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let mut rows = Vec::new();
    if let Some(draft) = state.delete_candidate() {
        rows.push(RenderRow::Note(
            format_text(
                locale,
                "Delete the draft \"{}\"? It is the only copy.",
                &[&draft_display_name(draft)],
            ),
            Style::default().fg(Color::Red),
        ));
        rows.push(RenderRow::Button(
            AddControlId::DeleteDraft,
            text(locale, "Remove").into_owned(),
        ));
        rows.push(RenderRow::Button(
            AddControlId::Cancel,
            text(locale, "Cancel").into_owned(),
        ));
    }
    rows
}

fn review_rows(state: &AddWorkflowState, locale: Locale) -> Vec<RenderRow> {
    let review = state
        .review()
        .expect("the typed Review stage owns its review state");
    let mut rows = vec![
        RenderRow::Input(AddTextField::ReviewName, text(locale, "Name").into_owned()),
        RenderRow::Input(
            AddTextField::ReviewDescription,
            text(locale, "Description").into_owned(),
        ),
    ];
    if let (false, ReviewLane::Script | ReviewLane::Prompt) = (review.is_fresh(), review.lane()) {
        rows.push(RenderRow::Select(
            AddSelectControl::Storage,
            text(locale, "Storage mode").into_owned(),
        ));
    }
    match review.dependency_surface() {
        DependencySurface::Python => {
            rows.push(RenderRow::Input(
                AddTextField::Dependencies,
                text(locale, "Package dependencies").into_owned(),
            ));
            rows.push(hint(
                text(locale, "detected from the script's imports — edit freely").into_owned(),
            ));
            rows.push(RenderRow::Input(
                AddTextField::PythonConstraint,
                text(locale, "Python constraint").into_owned(),
            ));
            rows.push(hint(
                text(
                    locale,
                    "Python version (requires-python) — prefilled from the #! line when it pins one; empty means automatic",
                )
                .into_owned(),
            ));
        }
        DependencySurface::PythonOwned(metadata) => {
            rows.push(RenderRow::Note(
                text(
                    locale,
                    "The script declares its own dependencies (PEP 723):",
                )
                .into_owned(),
                Style::default().add_modifier(Modifier::DIM),
            ));
            if !metadata.requires_python.is_empty() {
                rows.push(hint(format!(
                    "· {}",
                    format_text(locale, "needs Python {}", &[&metadata.requires_python])
                )));
            }
            for dependency in &metadata.dependencies {
                rows.push(hint(format!(
                    "· {}",
                    format_text(locale, "installs {}", &[dependency])
                )));
            }
            if let (true, true) = (
                metadata.requires_python.is_empty(),
                metadata.dependencies.is_empty(),
            ) {
                rows.push(hint(text(locale, "(none declared)").into_owned()));
            }
        }
        DependencySurface::Npm if review.storage() == StorageMode::Copy => {
            rows.push(RenderRow::Input(
                AddTextField::Dependencies,
                text(locale, "Package dependencies").into_owned(),
            ));
            rows.push(hint(
                text(locale, "detected from the script's imports — edit freely").into_owned(),
            ));
        }
        DependencySurface::Npm => {}
        DependencySurface::None => {}
    }
    if let Some(count) = review.modeled_cli_field_count().and_then(NonZeroUsize::new) {
        rows.push(RenderRow::Note(
            format_text(
                locale,
                if count.get() == 1 {
                    "✓ skit read this script's own arguments ({} field). Running it opens a form — nothing to memorize."
                } else {
                    "✓ skit read this script's own arguments ({} fields). Running it opens a form — nothing to memorize."
                },
                &[&count.get()],
            ),
            Style::default().fg(Color::Green),
        ));
    }
    if let (None, true) = (
        review.modeled_cli_field_count(),
        review.onboarding().uses_cli_framework(),
    ) {
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
            if let skit_domain::parameters::ParameterBinding::Input = candidate.declaration.binding
            {
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
                    .and_then(NonZeroUsize::new)
                    .is_some()
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
            if has_more_prompt_candidates(review) {
                rows.push(RenderRow::Button(
                    AddControlId::Continue,
                    text(locale, "Choose variables…").into_owned(),
                ));
            }
        }
        rows.push(RenderRow::Select(
            AddSelectControl::Runner,
            text(locale, "Prompt runner").into_owned(),
        ));
        rows.push(RenderRow::Button(
            AddControlId::NewRunner,
            format!("{} {}", text(locale, "Add"), text(locale, "Runner")),
        ));
    }
    if matches!(review.lane(), ReviewLane::Script | ReviewLane::Prompt) {
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
    clip: RowClip,
    row: &RenderRow,
    state: &AddWorkflowState,
    session: &mut AddScreenSession,
    locale: Locale,
) {
    let area = clip.area();
    match row {
        RenderRow::Input(field, label) => {
            if let Some(input) = session.inputs.get(field)
                && let Some(editable) = render_line_input_band(
                    frame,
                    clip,
                    input,
                    false,
                    session.focus.is_focused(&AddControlId::Text(*field)),
                    label,
                    None,
                )
            {
                session.editables.insert(*field, editable);
            }
        }
        RenderRow::Button(id, label) => {
            let focused = session.focus.is_focused(id);
            clip.paint_paragraph(
                frame.buffer_mut(),
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
            );
        }
        RenderRow::Check(id, label) => {
            if let Some(check) = session.checks.get(id) {
                frame.render_widget(CheckBox::new(label, check), area);
            }
        }
        RenderRow::Select(select, label) => {
            let (options, select_state) = match select {
                AddSelectControl::Storage => (storage_options(state, locale), &session.storage),
                AddSelectControl::Runner => {
                    let mut options = vec![text(locale, "ask on the run form").into_owned()];
                    options.extend(
                        state
                            .review()
                            .into_iter()
                            .flat_map(|review| review.runner_names().iter().cloned()),
                    );
                    (options, &session.runner)
                }
            };
            if clip.is_full() {
                let select = Select::new(&options, select_state).label(label);
                select.render_stateful(frame, area);
            } else {
                let style = SelectStyle::default();
                let display = &options[select_state.selected_index.unwrap()];
                let border = if select_state.focused {
                    style.focused_border
                } else {
                    style.unfocused_border
                };
                clip.paint_bordered_paragraph(
                    frame.buffer_mut(),
                    Paragraph::new(Line::from(vec![
                        Span::styled(display, Style::default().fg(style.text_fg)),
                        Span::styled(
                            format!(" {}", style.dropdown_indicator),
                            Style::default().fg(border),
                        ),
                    ])),
                    Line::from(format!(" {label} ")),
                    Style::default().fg(border),
                    0,
                );
            }
        }
        RenderRow::Note(message, style) => {
            clip.paint_paragraph(
                frame.buffer_mut(),
                Paragraph::new(message.as_str())
                    .wrap(Wrap { trim: false })
                    .style(*style),
            );
        }
    }
}

fn render_select_overlay(
    frame: &mut Frame,
    area: Rect,
    select: AddSelectControl,
    state: &AddWorkflowState,
    session: &AddScreenSession,
    locale: Locale,
) -> Vec<AddHitRegion> {
    let (options, select_state, label) = match select {
        AddSelectControl::Storage => (
            storage_options(state, locale),
            &session.storage,
            text(locale, "Storage mode").into_owned(),
        ),
        AddSelectControl::Runner => {
            let mut options = vec![text(locale, "ask on the run form").into_owned()];
            options.extend(
                state
                    .review()
                    .into_iter()
                    .flat_map(|review| review.runner_names().iter().cloned()),
            );
            (
                options,
                &session.runner,
                text(locale, "Prompt runner").into_owned(),
            )
        }
    };
    if !select_state.is_open {
        return Vec::new();
    }
    let first_visible = usize::from(select_state.scroll_offset);
    Select::new(&options, select_state)
        .label(&label)
        .render_dropdown(frame, area, frame.area())
        .into_iter()
        .zip(first_visible..)
        .map(|(region, index)| AddHitRegion {
            area: region.area,
            target: select.option_id(index),
        })
        .collect()
}

fn has_more_prompt_candidates(review: &skit_ui::ReviewState) -> bool {
    review
        .prompt_candidates()
        .get(review.prompt_preview().len())
        .is_some()
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

#[derive(Debug)]
struct PositionedAddFooterChip {
    chip: AddFooterChip,
    row: usize,
    x: u16,
    width: u16,
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
            chips.extend([
                chip("Tab/↓", String::new(), AddControlId::NextField),
                chip("Shift+Tab/↑", String::new(), AddControlId::PreviousField),
            ]);
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
                        match state.review().map(|review| review.lane()) {
                            Some(ReviewLane::Prompt) => "Edit prompt",
                            Some(ReviewLane::Script | ReviewLane::Executable) | None => {
                                "Edit script"
                            }
                        },
                    )
                    .into_owned(),
                    AddControlId::EditSource,
                ));
            }
            chips.extend([
                chip("Tab/↓", String::new(), AddControlId::NextField),
                chip("Shift+Tab/↑", String::new(), AddControlId::PreviousField),
            ]);
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
    session: &mut AddScreenSession,
    locale: Locale,
) -> Vec<AddHitRegion> {
    if area.is_empty() {
        session.footer_viewport = Rect::default();
        session.footer_visible_height = 0;
        return Vec::new();
    }
    let chips = footer_chips(state, locale);
    let (mut positioned, mut rows) = position_footer_chips(chips, area.width);
    let mut content_width = area.width;
    let mut indicator_column = None;
    if let (Some(_overflow), Some(narrower)) = (
        NonZeroUsize::new(rows.saturating_sub(usize::from(area.height))),
        NonZeroU16::new(area.width.saturating_sub(1)),
    ) {
        content_width = narrower.get();
        indicator_column = Some(area.right().saturating_sub(1));
        (positioned, rows) = position_footer_chips(footer_chips(state, locale), content_width);
    }
    session.footer_visible_height = usize::from(area.height);
    session.footer_viewport = Rect::new(area.x, area.y, content_width, area.height);
    session.footer_scroll.set_lines(vec![String::new(); rows]);
    crate::viewport::Viewport::new(session.footer_viewport, rows)
        .clamp_scroll(&mut session.footer_scroll);
    let offset = session.footer_scroll.scroll_offset();
    let end = offset.saturating_add(session.footer_visible_height);
    let mut hits = Vec::new();
    for item in positioned
        .into_iter()
        .filter(|item| item.row >= offset && item.row < end)
    {
        let label = format!("[{}] {}", item.chip.key, item.chip.label);
        let y = area
            .y
            .saturating_add(u16::try_from(item.row.saturating_sub(offset)).unwrap_or(u16::MAX));
        let chip_area = Rect::new(area.x.saturating_add(item.x), y, item.width, 1);
        frame.render_widget(
            Paragraph::new(label).style(Style::default().add_modifier(Modifier::DIM)),
            chip_area,
        );
        hits.push(AddHitRegion {
            area: chip_area,
            target: item.chip.target,
        });
    }
    if let (Some(indicator_x), Some(_overflow)) = (
        indicator_column,
        NonZeroUsize::new(rows.saturating_sub(session.footer_visible_height)),
    ) {
        let indicator = if session.footer_scroll.is_at_top() {
            "↓"
        } else if session
            .footer_scroll
            .is_at_bottom(session.footer_visible_height)
        {
            "↑"
        } else {
            "↕"
        };
        frame.render_widget(
            Paragraph::new(indicator).style(Style::default().add_modifier(Modifier::DIM)),
            Rect::new(indicator_x, area.y, 1, 1),
        );
    }
    hits
}

fn position_footer_chips(
    chips: Vec<AddFooterChip>,
    width: u16,
) -> (Vec<PositionedAddFooterChip>, usize) {
    if chips.is_empty() || width == 0 {
        return (Vec::new(), 0);
    }
    let mut row = 0_usize;
    let mut x = 0_u16;
    let mut positioned = Vec::with_capacity(chips.len());
    for chip in chips {
        let desired = u16::try_from(
            chip.key
                .width()
                .saturating_add(chip.label.width())
                .saturating_add(4),
        )
        .unwrap_or(u16::MAX)
        .min(width);
        if NonZeroU16::new(x.saturating_add(desired).saturating_sub(width)).is_some() {
            row = row.saturating_add(1);
            x = 0;
        }
        positioned.push(PositionedAddFooterChip {
            chip,
            row,
            x,
            width: desired.min(width.saturating_sub(x)),
        });
        x = x.saturating_add(desired).saturating_add(1);
    }
    (positioned, row.saturating_add(1))
}

fn storage_options(state: &AddWorkflowState, locale: Locale) -> Vec<String> {
    let reference = match state.review().map(|review| review.lane()) {
        Some(ReviewLane::Prompt) => {
            "Link the original — edits take effect immediately; skit never writes to the file"
        }
        Some(ReviewLane::Script | ReviewLane::Executable) | None => {
            "Link the original — edits take effect immediately, but skit won't write to the file, so parameter definitions are yours to maintain"
        }
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
        AddProblem::DraftChanged { path } => format_text(
            locale,
            "The kept draft changed before cleanup. skit kept it at {}.",
            &[&path.display()],
        ),
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

    use ratatui_core::{backend::TestBackend, layout::Rect, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::{KeyEvent, KeyEventKind, MouseButton, MouseEvent};
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
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            kind,
            ReviewDefaults::default(),
        ))
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn tab_until(
        session: &mut AddScreenSession,
        state: &AddWorkflowState,
        geometry: &AddScreenGeometry,
        description: &str,
        reached: impl Fn(Option<&AddControlId>) -> bool,
    ) {
        let attempts = session.focus.elements().len().saturating_add(1);
        let mut found = reached(session.focused());
        for _ in 0..attempts {
            if found {
                break;
            }
            let _ = session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), state, geometry);
            found = reached(session.focused());
        }
        assert!(
            found,
            "focus did not reach {description} in {attempts} Tab presses"
        );
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    }

    fn click_control(
        session: &mut AddScreenSession,
        state: &AddWorkflowState,
        geometry: &AddScreenGeometry,
        column: u16,
        row: u16,
    ) -> Option<AddScreenEvent> {
        assert_eq!(
            session.handle_event(
                mouse(MouseEventKind::Down(MouseButton::Left), column, row),
                state,
                geometry,
            ),
            Some(AddScreenEvent::Changed),
        );
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            state,
            geometry,
        )
    }

    fn ambiguous(path: &str, bytes: &[u8]) -> AddWorkflowState {
        let mut state = AddWorkflowState::new(Vec::new());
        let _ = state.reduce(AddAction::SetSourcePath(path.to_owned()));
        let effects = state.reduce(AddAction::Continue);
        let mut request = None;
        for effect in effects {
            if let AddEffect::InspectSource {
                request: current, ..
            } = effect
            {
                request = Some(current);
            }
        }
        let request = request.unwrap();
        let _ = state.reduce(AddAction::SourceInspected {
            request,
            result: Ok(SourceSnapshot {
                path: PathBuf::from(path),
                source_record: path.to_owned(),
                bytes: bytes.to_vec(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            }),
        });
        state
    }

    fn draw(
        state: &AddWorkflowState,
        session: &mut AddScreenSession,
        width: u16,
        height: u16,
    ) -> (Terminal<TestBackend>, AddScreenGeometry) {
        draw_locale(state, session, width, height, Locale::En)
    }

    fn draw_locale(
        state: &AddWorkflowState,
        session: &mut AddScreenSession,
        width: u16,
        height: u16,
        locale: Locale,
    ) -> (Terminal<TestBackend>, AddScreenGeometry) {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut geometry = AddScreenGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_add(frame, frame.area(), state, session, locale);
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

    fn hit_text(terminal: &Terminal<TestBackend>, area: Rect) -> String {
        let buffer = terminal.backend().buffer();
        (area.y..area.bottom())
            .flat_map(|row| {
                (area.x..area.right()).map(move |column| buffer[(column, row)].symbol())
            })
            .collect()
    }

    #[test]
    fn focus_alignment_distinguishes_above_equal_and_below_row_start() {
        let id = AddControlId::Cancel;
        let mut session = AddScreenSession::default();
        session.focus.register(id.clone());
        session.focus.set(id.clone());
        session.row_spans.insert(id, (5, 3));
        session.visible_height = 2;
        session.scroll.set_lines(vec![String::new(); 12]);

        session.scroll.set_scroll_offset(6);
        session.ensure_focus_visible();
        assert_eq!(
            session.scroll.scroll_offset(),
            5,
            "a focused row above the viewport aligns its leading edge"
        );

        session.scroll.set_scroll_offset(5);
        session.ensure_focus_visible();
        assert_eq!(
            session.scroll.scroll_offset(),
            6,
            "a row taller than the viewport tail-aligns when its start equals the offset"
        );

        session.scroll.set_scroll_offset(4);
        session.ensure_focus_visible();
        assert_eq!(
            session.scroll.scroll_offset(),
            6,
            "a focused row below the viewport aligns its trailing edge"
        );
    }

    /// A short viewport can cut the top row from a bordered input.
    ///
    /// The surviving band starts with the value row. It must not restart the
    /// control at its top border and hide the value.
    #[test]
    fn a_top_clipped_add_input_shows_its_surviving_value_row() {
        let mut state = AddWorkflowState::new(Vec::new());
        let _ = state.reduce(AddAction::SetSourcePath("/work/later-row.py".to_owned()));
        let mut session = AddScreenSession::default();

        let (terminal, geometry) = draw(&state, &mut session, 48, 5);
        let rendered = text_of(&terminal);

        assert_eq!(geometry.body.height, 2, "the input must be top-clipped");
        assert_eq!(
            geometry.first_visible, 2,
            "the visible band starts at the value row"
        );
        assert!(
            rendered.contains("/work/later-row.py"),
            "the surviving value row is missing:\n{rendered}"
        );
    }

    #[test]
    fn a_top_clipped_focused_add_select_shows_its_surviving_value_row() {
        let state = source("task.prompt.md", b"Task", KnownEntryKind::Prompt);
        let mut session = AddScreenSession::default();
        session.sync(&state);
        session.focus.set(AddControlId::Runner);
        session.sync_values(&state);
        let mut terminal = Terminal::new(TestBackend::new(48, 2)).unwrap();

        terminal
            .draw(|frame| {
                render_row(
                    frame,
                    RowClip::new(3, 1, frame.area()),
                    &RenderRow::Select(AddSelectControl::Runner, "Prompt runner".to_owned()),
                    &state,
                    &mut session,
                    Locale::En,
                );
            })
            .unwrap();
        let rendered = text_of(&terminal);

        assert!(
            rendered.contains("ask on the run form"),
            "the surviving select value is missing: {rendered}"
        );
    }

    #[test]
    fn source_keyboard_and_browse_mouse_have_positive_typed_paths() {
        let localized = AddWorkflowState::new(Vec::new());
        for (locale, expected) in [
            (Locale::En, "[Ctrl+O] Select"),
            // Ratatui's TestBackend exposes the continuation cell of each wide glyph as a space.
            (Locale::ZhCn, "[Ctrl+O] 选 择"),
            (Locale::ZhTw, "[Ctrl+O] 選 擇"),
        ] {
            let mut localized_session = AddScreenSession::default();
            let (terminal, geometry) =
                draw_locale(&localized, &mut localized_session, 80, 24, locale);
            let browse = geometry
                .hits
                .iter()
                .find(|hit| hit.target == AddControlId::BrowseSource)
                .expect("the visible Browse button is a typed mouse hit");
            assert_eq!(hit_text(&terminal, browse.area).trim(), expected);
        }

        let mut state = AddWorkflowState::new(Vec::new());
        let mut session = AddScreenSession::default();
        let (terminal, geometry) = draw(&state, &mut session, 80, 24);
        assert!(text_of(&terminal).contains("Command template"));
        let typed = session.handle_event(
            key(KeyCode::Char('x'), KeyModifiers::NONE),
            &state,
            &geometry,
        );
        assert_eq!(
            typed,
            Some(AddScreenEvent::Action(AddAction::SetSourcePath(
                "x".to_owned()
            )))
        );
        if let Some(AddScreenEvent::Action(action)) = typed {
            let _ = state.reduce(action);
        }
        assert!(matches!(
            session.handle_event(
                key(KeyCode::Char('o'), KeyModifiers::CONTROL),
                &state,
                &geometry,
            ),
            Some(AddScreenEvent::OpenPathPicker(_))
        ));
        assert_eq!(
            session.focused(),
            Some(&AddControlId::Text(AddTextField::SourcePath))
        );
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
        assert!(matches!(
            click_control(
                &mut session,
                &state,
                &geometry,
                browse.area.x,
                browse.area.y
            ),
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
        assert_eq!(
            click_control(
                &mut session,
                &state,
                &geometry,
                write_script.area.x,
                write_script.area.y,
            ),
            Some(AddScreenEvent::Action(AddAction::NewDraft(
                DraftKind::Script
            )))
        );

        let effects = state.reduce(AddAction::NewDraft(DraftKind::Script));
        let mut request = None;
        for effect in effects {
            if let AddEffect::AuthorDraft {
                request: current, ..
            } = effect
            {
                request = Some(current);
            }
        }
        let request = request.unwrap();
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
    fn add_line_input_click_places_the_caret_before_typing() {
        let mut state = AddWorkflowState::new(Vec::new());
        let _ = state.reduce(AddAction::SetSourcePath("abcdef".to_owned()));
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw(&state, &mut session, 80, 20);
        let hit = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Text(AddTextField::SourcePath))
            .unwrap();
        let _ = click_control(
            &mut session,
            &state,
            &geometry,
            hit.area.x.saturating_add(3),
            hit.area.y.saturating_add(1),
        );
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char('X'), KeyModifiers::NONE),
                &state,
                &geometry,
            ),
            Some(AddScreenEvent::Action(AddAction::SetSourcePath(
                "abXcdef".to_owned()
            )))
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
        tab_until(
            &mut session,
            &state,
            &geometry,
            "a review candidate",
            |focused| matches!(focused, Some(AddControlId::Candidate(_))),
        );
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
        tab_until(&mut session, &state, &geometry, "Save", |focused| {
            focused == Some(&AddControlId::Save)
        });
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
            assert_eq!(
                click_control(&mut session, &state, &geometry, hit.area.x, hit.area.y),
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

    /// Tab onto a draft row, then Ctrl+D, must open the delete ask for that row.
    ///
    /// Version 0.4 deletes the highlighted draft with no activation step
    /// (`OptionList.highlighted`, `src/skit/tui_add.py:481-490`). The recorded
    /// walkthrough proved the old screen ignored Ctrl+D on a row the keyboard
    /// had only focused, so this drives the real key path end to end.
    #[test]
    fn a_tab_focused_draft_row_answers_the_delete_chord() {
        let draft = skit_ui::DraftSummary {
            path: PathBuf::from("skit-new-kept.py"),
            modified: 1,
            identity: None,
            permissions: SourcePermissions::default(),
            content_hash: None,
        };
        let mut state = AddWorkflowState::new(vec![draft.clone()]);
        let mut session = AddScreenSession::default();
        session.sync(&state);
        let geometry = AddScreenGeometry::default();
        let mut landed = false;
        for _ in 0..12 {
            let event =
                session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &state, &geometry);
            if let Some(AddScreenEvent::Action(action)) = event {
                let _ = state.reduce(action);
            }
            if session.focused() == Some(&AddControlId::Draft(0)) {
                landed = true;
                break;
            }
        }
        assert!(landed, "focus never reached the draft row");
        assert_eq!(state.source().selected_draft(), Some(&draft));

        let chord = session.handle_event(
            key(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        );
        assert_eq!(
            chord,
            Some(AddScreenEvent::Action(AddAction::DeleteSelectedDraft)),
            "Ctrl+D on a focused draft row must act"
        );
        let _ = state.reduce(AddAction::DeleteSelectedDraft);
        assert_eq!(state.stage(), AddStage::ConfirmDraftDelete);
        assert_eq!(state.delete_candidate(), Some(&draft));
    }

    #[test]
    fn source_kind_and_delete_confirmation_drive_complete_key_and_pointer_surfaces() {
        let drafts = (0..25)
            .map(|index| skit_ui::DraftSummary {
                path: PathBuf::from(format!("draft-{index}.py")),
                modified: u64::try_from(index).unwrap(),
                identity: None,
                permissions: SourcePermissions::default(),
                content_hash: None,
            })
            .collect::<Vec<_>>();
        let mut state = AddWorkflowState::new(drafts);
        let _ = state.reduce(AddAction::SelectDraft(0));
        let mut session = AddScreenSession::default();
        session.sync(&state);
        let _ = session.handle_event(
            key(KeyCode::Tab, KeyModifiers::NONE),
            &state,
            &AddScreenGeometry::default(),
        );
        let (terminal, geometry) = draw(&state, &mut session, 48, 12);
        assert_eq!(state.source().draft_overflow(), 5);
        assert!(!text_of(&terminal).is_empty());
        assert_eq!(
            session.activate(AddControlId::NewPrompt, &state),
            Some(AddScreenEvent::Action(AddAction::NewDraft(
                DraftKind::Prompt
            )))
        );

        for hit in geometry.hits.clone() {
            assert!(
                click_control(&mut session, &state, &geometry, hit.area.x, hit.area.y).is_some(),
                "visible source control has no mouse action: {:?}",
                hit.target
            );
        }
        let cancel = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Cancel)
            .unwrap();
        let _ = session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                cancel.area.x,
                cancel.area.y,
            ),
            &state,
            &geometry,
        );
        let _ = session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                cancel.area.x,
                cancel.area.y,
            ),
            &state,
            &geometry,
        );
        assert_eq!(
            session.handle_event(
                key(KeyCode::Char('x'), KeyModifiers::NONE),
                &state,
                &geometry,
            ),
            None
        );
        for event in [
            mouse(MouseEventKind::Moved, 0, 0),
            mouse(MouseEventKind::Up(MouseButton::Left), 0, 0),
            Event::Paste("ignored".to_owned()),
            Event::Resize(10, 10),
            Event::FocusGained,
        ] {
            assert_eq!(session.handle_event(event, &state, &geometry), None);
        }
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new_with_kind(
                    KeyCode::Char('x'),
                    KeyModifiers::NONE,
                    KeyEventKind::Release,
                )),
                &state,
                &geometry,
            ),
            None
        );
        for (code, modifiers) in [
            (KeyCode::Char('p'), KeyModifiers::CONTROL),
            (KeyCode::Char('d'), KeyModifiers::CONTROL),
            (KeyCode::BackTab, KeyModifiers::NONE),
            (KeyCode::Down, KeyModifiers::NONE),
            (KeyCode::Up, KeyModifiers::NONE),
            (KeyCode::PageDown, KeyModifiers::NONE),
            (KeyCode::Esc, KeyModifiers::NONE),
        ] {
            let _ = session.handle_event(key(code, modifiers), &state, &geometry);
        }
        let _ = session.handle_event(
            mouse(MouseEventKind::ScrollDown, geometry.body.x, geometry.body.y),
            &state,
            &geometry,
        );

        let effects = state.reduce(AddAction::DeleteSelectedDraft);
        assert!(effects.is_empty());
        assert_eq!(state.stage(), AddStage::ConfirmDraftDelete);
        let (_, confirm_geometry) = draw(&state, &mut session, 48, 10);
        for target in [AddControlId::DeleteDraft, AddControlId::Cancel] {
            let hit = confirm_geometry
                .hits
                .iter()
                .find(|hit| hit.target == target)
                .unwrap();
            assert!(
                click_control(
                    &mut session,
                    &state,
                    &confirm_geometry,
                    hit.area.x,
                    hit.area.y,
                )
                .is_some()
            );
        }
        assert!(matches!(
            session.handle_event(
                key(KeyCode::Esc, KeyModifiers::NONE),
                &state,
                &confirm_geometry
            ),
            Some(AddScreenEvent::Action(AddAction::ConfirmDraftDelete(false)))
        ));

        for path in ["unknown.txt", "likely.md"] {
            let kind_state = ambiguous(path, b"plain body\n");
            assert_eq!(kind_state.stage(), AddStage::Kind);
            let mut kind_session = AddScreenSession::default();
            let (_, kind_geometry) = draw(&kind_state, &mut kind_session, 42, 12);
            for code in [
                KeyCode::Up,
                KeyCode::Down,
                KeyCode::Home,
                KeyCode::End,
                KeyCode::Enter,
                KeyCode::Esc,
                KeyCode::Char('x'),
            ] {
                let _ = kind_session.handle_event(
                    key(code, KeyModifiers::NONE),
                    &kind_state,
                    &kind_geometry,
                );
            }
            for hit in kind_geometry.hits.clone() {
                let _ = click_control(
                    &mut kind_session,
                    &kind_state,
                    &kind_geometry,
                    hit.area.x,
                    hit.area.y,
                );
            }
        }
    }

    #[test]
    fn review_controls_render_and_dispatch_storage_runner_checkbox_and_dropdown_paths() {
        let body = (0..23)
            .map(|index| format!("{{{{field{index}}}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        let defaults = ReviewDefaults {
            runner: Some("beta".to_owned()),
            runner_names: vec!["alpha".to_owned(), "beta".to_owned()],
            ..ReviewDefaults::default()
        };
        let review = ReviewState::from_source(
            SourceSnapshot {
                path: PathBuf::from("task.prompt.md"),
                source_record: "task.prompt.md".to_owned(),
                bytes: body.into_bytes(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            KnownEntryKind::Prompt,
            defaults,
        );
        let state = AddWorkflowState::from_review(review);
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw(&state, &mut session, 80, 55);
        for hit in geometry.hits.clone() {
            let result = click_control(&mut session, &state, &geometry, hit.area.x, hit.area.y);
            if hit.target != AddControlId::ToggleFocused {
                assert!(result.is_some(), "review hit is inert: {:?}", hit.target);
            }
        }

        let interpolate = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Interpolate)
            .unwrap();
        let _ = click_control(
            &mut session,
            &state,
            &geometry,
            interpolate.area.x,
            interpolate.area.y,
        );
        let toggle = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::ToggleFocused)
            .unwrap();
        assert!(
            click_control(
                &mut session,
                &state,
                &geometry,
                toggle.area.x,
                toggle.area.y,
            )
            .is_some()
        );

        for target in [
            AddControlId::Text(AddTextField::ReviewName),
            AddControlId::Interpolate,
            AddControlId::PromptCandidate("field0".to_owned()),
            AddControlId::Runner,
            AddControlId::RunnerOption(0),
            AddControlId::RunnerOption(2),
            AddControlId::NewRunner,
            AddControlId::Continue,
            AddControlId::EditSource,
            AddControlId::Save,
            AddControlId::ToggleFocused,
            AddControlId::NextField,
            AddControlId::PreviousField,
            AddControlId::Cancel,
        ] {
            let _ = session.activate(target, &state);
        }
        tab_until(&mut session, &state, &geometry, "Runner", |focused| {
            focused == Some(&AddControlId::Runner)
        });
        for code in [KeyCode::Enter, KeyCode::Down, KeyCode::Up, KeyCode::Esc] {
            let _ = session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry);
        }

        let mut python = source("tool.py", b"print('ok')\n", KnownEntryKind::Python);
        let _ = python.reduce(AddAction::SetReviewStorage(StorageMode::Reference));
        let mut python_session = AddScreenSession::default();
        let (_, python_geometry) = draw(&python, &mut python_session, 70, 20);
        let storage = python_geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Storage)
            .unwrap();
        let _ = click_control(
            &mut python_session,
            &python,
            &python_geometry,
            storage.area.x,
            storage.area.y,
        );
        for code in [KeyCode::Up, KeyCode::Enter] {
            let _ = python_session.handle_event(
                key(code, KeyModifiers::NONE),
                &python,
                &python_geometry,
            );
        }
        assert_eq!(
            python_session.handle_event(
                key(KeyCode::Char('x'), KeyModifiers::NONE),
                &python,
                &python_geometry,
            ),
            None
        );
        let _ = python_session.activate(AddControlId::Storage, &python);
        for index in [0, 1] {
            let _ = python_session.activate(AddControlId::StorageOption(index), &python);
        }

        let copy_python = source("copy.py", b"print('ok')\n", KnownEntryKind::Python);
        let mut copy_session = AddScreenSession::default();
        let (_, copy_geometry) = draw(&copy_python, &mut copy_session, 70, 20);
        let storage = copy_geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Storage)
            .unwrap();
        let _ = click_control(
            &mut copy_session,
            &copy_python,
            &copy_geometry,
            storage.area.x,
            storage.area.y,
        );
        let _ = copy_session.handle_event(
            key(KeyCode::Down, KeyModifiers::NONE),
            &copy_python,
            &copy_geometry,
        );
        assert!(matches!(
            copy_session.handle_event(
                key(KeyCode::Enter, KeyModifiers::NONE),
                &copy_python,
                &copy_geometry,
            ),
            Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                StorageMode::Reference
            )))
        ));

        for (path, bytes, kind) in [
            (
                "tool.js",
                b"console.log('x')\n".as_slice(),
                KnownEntryKind::JavaScript,
            ),
            ("tool.exe", b"binary".as_slice(), KnownEntryKind::Executable),
            (
                "tool.py",
                b"# /// script\n# dependencies=[]\n# ///\n".as_slice(),
                KnownEntryKind::Python,
            ),
        ] {
            let variant = source(path, bytes, kind);
            let mut variant_session = AddScreenSession::default();
            let _ = draw_locale(&variant, &mut variant_session, 54, 16, Locale::ZhTw);
        }
    }

    #[test]
    fn add_feedback_text_covers_every_typed_problem_in_all_locales() {
        assert_eq!(checkbox_action(&AddControlId::Cancel, true), None);
        let problems = [
            AddProblem::SourceUnavailable {
                path: PathBuf::from("missing"),
                reason: "gone".to_owned(),
            },
            AddProblem::MissingCommandName,
            AddProblem::InvalidKind,
            AddProblem::InvalidPromptEncoding,
            AddProblem::InvalidDependency {
                value: "bad dep".to_owned(),
            },
            AddProblem::InvalidPythonConstraint {
                value: "bad python".to_owned(),
            },
            AddProblem::SourceEdit {
                reason: "edit".to_owned(),
            },
            AddProblem::CommitFailed {
                reason: "commit".to_owned(),
            },
            AddProblem::EditFailed {
                reason: "editor".to_owned(),
            },
            AddProblem::DraftDeleteFailed {
                reason: "delete".to_owned(),
            },
            AddProblem::DraftChanged {
                path: PathBuf::from("draft.py"),
            },
        ];
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
            for problem in &problems {
                assert!(!problem_text(problem, locale).is_empty());
            }
            for notice in [
                AddNotice::NothingWritten,
                AddNotice::DraftKept(PathBuf::from("draft.py")),
                AddNotice::DraftDeleted(PathBuf::from("draft.py")),
            ] {
                assert!(!notice_text(&notice, locale).is_empty());
            }
        }
    }

    #[test]
    fn analyzer_prompt_flood_dropdown_and_terminal_edge_render_paths_are_owned() {
        let analyzer = source(
            "analysis.py",
            concat!(
                "import sys\n",
                "COUNT = 0\nCOUNT += 1\n",
                "answer = input('Question?')\n",
                "open('input.csv')\nprint(sys.argv, COUNT, answer)\n",
            )
            .as_bytes(),
            KnownEntryKind::Python,
        );
        let mut analyzer_session = AddScreenSession::default();
        let (terminal, geometry) = draw(&analyzer, &mut analyzer_session, 94, 50);
        let rendered = text_of(&terminal);
        assert!(rendered.contains("input()"), "{rendered}");
        assert!(rendered.contains("loop accumulator"), "{rendered}");
        assert!(rendered.contains("input.csv"), "{rendered}");
        assert!(rendered.contains("extra-arguments"), "{rendered}");
        for field in [
            AddTextField::ReviewName,
            AddTextField::ReviewDescription,
            AddTextField::Dependencies,
            AddTextField::PythonConstraint,
        ] {
            let hit = geometry
                .hits
                .iter()
                .find(|hit| hit.target == AddControlId::Text(field))
                .unwrap();
            let _ = click_control(
                &mut analyzer_session,
                &analyzer,
                &geometry,
                hit.area.x,
                hit.area.y,
            );
            assert!(matches!(
                analyzer_session.handle_event(
                    key(KeyCode::Char('z'), KeyModifiers::NONE),
                    &analyzer,
                    &geometry,
                ),
                Some(AddScreenEvent::Action(_))
            ));
        }

        for count in [0, 31] {
            let body = (0..count)
                .map(|index| format!("{{{{p{index}}}}}"))
                .collect::<Vec<_>>()
                .join(" ");
            let prompt = source("prompt.prompt.md", body.as_bytes(), KnownEntryKind::Prompt);
            let mut prompt_session = AddScreenSession::default();
            let (terminal, _) = draw(&prompt, &mut prompt_session, 76, 28);
            let screen = text_of(&terminal);
            if count == 0 {
                assert!(screen.contains("No {{name}} placeholders"), "{screen}");
            } else {
                assert!(
                    screen.contains("probably not written for insertion"),
                    "{screen}"
                );
            }
        }

        let defaults = ReviewDefaults {
            runner_names: vec!["alpha".to_owned(), "beta".to_owned()],
            ..ReviewDefaults::default()
        };
        let prompt = AddWorkflowState::from_review(ReviewState::from_source(
            SourceSnapshot {
                path: "runner.prompt.md".into(),
                source_record: "runner.prompt.md".to_owned(),
                bytes: b"Hello {{name}}".to_vec(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            KnownEntryKind::Prompt,
            defaults,
        ));
        let mut prompt_session = AddScreenSession::default();
        let (_, closed) = draw(&prompt, &mut prompt_session, 76, 30);
        let runner = closed
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Runner)
            .unwrap();
        let _ = click_control(
            &mut prompt_session,
            &prompt,
            &closed,
            runner.area.x,
            runner.area.y,
        );
        assert_eq!(prompt_session.focused(), Some(&AddControlId::Runner));
        for code in [KeyCode::Down, KeyCode::Enter] {
            assert!(
                prompt_session
                    .handle_event(key(code, KeyModifiers::NONE), &prompt, &closed,)
                    .is_some()
            );
        }
        assert_eq!(
            prompt_session.handle_event(
                key(KeyCode::Char('x'), KeyModifiers::NONE),
                &prompt,
                &closed,
            ),
            None
        );
        let _ = prompt_session.activate(AddControlId::Runner, &prompt);
        let (_, open) = draw(&prompt, &mut prompt_session, 76, 30);
        for option in open
            .hits
            .iter()
            .filter(|hit| matches!(hit.target, AddControlId::RunnerOption(_)))
        {
            assert!(
                click_control(
                    &mut prompt_session,
                    &prompt,
                    &open,
                    option.area.x,
                    option.area.y,
                )
                .is_some()
            );
        }

        let mut cancelled = AddWorkflowState::new(Vec::new());
        let _ = cancelled.reduce(AddAction::Cancel);
        assert_eq!(cancelled.stage(), AddStage::Cancelled);
        assert!(footer_chips(&cancelled, Locale::En).is_empty());
        let mut cancelled_session = AddScreenSession::default();
        let (_, cancelled_geometry) = draw(&cancelled, &mut cancelled_session, 1, 1);
        assert_eq!(
            cancelled_session.handle_event(
                key(KeyCode::Tab, KeyModifiers::NONE),
                &cancelled,
                &cancelled_geometry,
            ),
            Some(AddScreenEvent::Changed)
        );
        assert_eq!(cancelled_session.focused(), None);
        assert_eq!(
            cancelled_session.handle_event(
                key(KeyCode::Char('x'), KeyModifiers::NONE),
                &cancelled,
                &cancelled_geometry,
            ),
            None
        );
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| {
                assert!(
                    render_footer(
                        frame,
                        Rect::default(),
                        &cancelled,
                        &mut cancelled_session,
                        Locale::En,
                    )
                    .is_empty()
                );
            })
            .unwrap();

        let mut command = AddWorkflowState::new(Vec::new());
        let _ = command.reduce(AddAction::SetCommandTemplate("echo {name}".to_owned()));
        let _ = command.reduce(AddAction::Continue);
        let mut command_session = AddScreenSession::default();
        let (terminal, command_geometry) = draw(&command, &mut command_session, 64, 18);
        assert!(text_of(&terminal).contains("Name"));
        for field in [
            AddTextField::CommandTemplate,
            AddTextField::CommandName,
            AddTextField::CommandDescription,
        ] {
            let hit = command_geometry
                .hits
                .iter()
                .find(|hit| hit.target == AddControlId::Text(field))
                .unwrap();
            let _ = click_control(
                &mut command_session,
                &command,
                &command_geometry,
                hit.area.x,
                hit.area.y,
            );
            assert!(matches!(
                command_session.handle_event(
                    key(KeyCode::Char('x'), KeyModifiers::NONE),
                    &command,
                    &command_geometry,
                ),
                Some(AddScreenEvent::Action(_))
            ));
        }

        let root_review = AddWorkflowState::from_review(ReviewState::from_source(
            SourceSnapshot {
                path: PathBuf::from("/"),
                source_record: "/".to_owned(),
                bytes: b"echo ok\n".to_vec(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            KnownEntryKind::Shell,
            ReviewDefaults::default(),
        ));
        let mut root_session = AddScreenSession::default();
        let _ = draw(&root_review, &mut root_session, 40, 10);

        let secret_prompt = source(
            "secret.prompt.md",
            b"Use {{api_key}}",
            KnownEntryKind::Prompt,
        );
        let mut secret_session = AddScreenSession::default();
        let (terminal, _) = draw(&secret_prompt, &mut secret_session, 60, 18);
        assert!(text_of(&terminal).contains("secret"));
    }

    #[test]
    fn add_review_selects_route_keys_and_overlays_to_their_rendered_owner() {
        let mut runner_names = vec!["alpha".to_owned(), "beta".to_owned()];
        runner_names.extend((2..12).map(|index| format!("runner-{index}")));
        let defaults = ReviewDefaults {
            runner_names,
            ..ReviewDefaults::default()
        };
        let state = AddWorkflowState::from_review(ReviewState::from_source(
            SourceSnapshot {
                path: "task.prompt.md".into(),
                source_record: "task.prompt.md".to_owned(),
                bytes: b"Hello {{name}}".to_vec(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            KnownEntryKind::Prompt,
            defaults,
        ));
        let mut session = AddScreenSession::default();
        let (_, closed) = draw(&state, &mut session, 96, 40);

        let storage = closed
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Storage)
            .expect("the rendered review owns its Storage select")
            .area;
        tab_until(&mut session, &state, &closed, "Storage mode", |focused| {
            focused == Some(&AddControlId::Storage)
        });
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Changed)
        );
        let (_, keyboard_open) = draw(&state, &mut session, 96, 40);
        assert!(
            keyboard_open
                .hits
                .iter()
                .any(|hit| matches!(hit.target, AddControlId::StorageOption(_)))
        );
        assert_eq!(
            session.handle_event(
                key(KeyCode::Esc, KeyModifiers::NONE),
                &state,
                &keyboard_open
            ),
            Some(AddScreenEvent::Changed)
        );
        assert!(click_control(&mut session, &state, &closed, storage.x, storage.y).is_some());
        assert_eq!(
            session.handle_event(key(KeyCode::Home, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                StorageMode::Copy
            )))
        );
        assert!(click_control(&mut session, &state, &closed, storage.x, storage.y).is_some());
        assert_eq!(
            session.handle_event(key(KeyCode::End, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                StorageMode::Reference
            )))
        );

        let runner = closed
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Runner)
            .expect("the rendered review owns its Runner select")
            .area;
        assert!(click_control(&mut session, &state, &closed, runner.x, runner.y).is_some());
        assert_eq!(
            session.handle_event(key(KeyCode::Home, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Action(AddAction::SetPromptRunner {
                name: String::new(),
                picked: true,
            }))
        );
        assert!(click_control(&mut session, &state, &closed, runner.x, runner.y).is_some());
        assert_eq!(
            session.handle_event(key(KeyCode::Down, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Changed)
        );
        assert_eq!(
            session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &closed),
            Some(AddScreenEvent::Action(AddAction::SetPromptRunner {
                name: "alpha".to_owned(),
                picked: true,
            }))
        );
        assert!(click_control(&mut session, &state, &closed, runner.x, runner.y).is_some());
        tab_until(&mut session, &state, &closed, "Add Runner", |focused| {
            focused == Some(&AddControlId::NewRunner)
        });
        assert_eq!(
            session.handle_event(key(KeyCode::Char('x'), KeyModifiers::NONE), &state, &closed,),
            None,
            "a non-Runner control must not inherit an open Runner select"
        );

        for (owner, expected_options) in [
            (
                AddControlId::Storage,
                vec![(AddControlId::StorageOption(0), 0, "Keep a copy")],
            ),
            (
                AddControlId::Runner,
                vec![(AddControlId::RunnerOption(1), 1, "alpha")],
            ),
        ] {
            let mut overlay_session = AddScreenSession::default();
            let (_, base) = draw(&state, &mut overlay_session, 96, 40);
            let anchor = base
                .hits
                .iter()
                .find(|hit| hit.target == owner)
                .expect("the select anchor is rendered")
                .area;
            assert!(
                click_control(&mut overlay_session, &state, &base, anchor.x, anchor.y,).is_some()
            );
            let (terminal, open) = draw(&state, &mut overlay_session, 96, 40);
            for (target, index, label) in expected_options {
                let option = open
                    .hits
                    .iter()
                    .find(|hit| hit.target == target)
                    .expect("the open select owns the typed option")
                    .area;
                assert_eq!(option.x, anchor.x.saturating_add(1));
                assert_eq!(
                    option.y,
                    anchor
                        .bottom()
                        .saturating_add(1)
                        .saturating_add(u16::try_from(index).unwrap())
                );
                assert_eq!(option.width, anchor.width.saturating_sub(2));
                assert!(hit_text(&terminal, option).contains(label));
            }
        }

        let mut scrolled = AddScreenSession::default();
        let (_, base) = draw(&state, &mut scrolled, 96, 40);
        let runner = base
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Runner)
            .expect("the Runner select is rendered")
            .area;
        assert!(click_control(&mut scrolled, &state, &base, runner.x, runner.y).is_some());
        assert_eq!(
            scrolled.handle_event(key(KeyCode::End, KeyModifiers::NONE), &state, &base),
            Some(AddScreenEvent::Changed)
        );
        let (terminal, open) = draw(&state, &mut scrolled, 96, 40);
        let last = open
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::RunnerOption(12))
            .expect("the scrolled dropdown owns its last semantic option")
            .area;
        assert!(hit_text(&terminal, last).contains("runner-11"));
        assert_eq!(
            click_control(&mut scrolled, &state, &open, last.x, last.y),
            Some(AddScreenEvent::Action(AddAction::SetPromptRunner {
                name: "runner-11".to_owned(),
                picked: true,
            }))
        );
    }

    #[test]
    fn add_review_body_rows_are_visible_positive_and_non_overlapping() {
        let defaults = ReviewDefaults {
            runner_names: vec!["alpha".to_owned()],
            ..ReviewDefaults::default()
        };
        let state = AddWorkflowState::from_review(ReviewState::from_source(
            SourceSnapshot {
                path: "task.prompt.md".into(),
                source_record: "task.prompt.md".to_owned(),
                bytes: b"Hello {{name}}".to_vec(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: false,
                identity: None,
            },
            KnownEntryKind::Prompt,
            defaults,
        ));
        let mut session = AddScreenSession::default();
        let (terminal, geometry) = draw(&state, &mut session, 96, 40);
        let expected = [
            AddControlId::Text(AddTextField::ReviewName),
            AddControlId::Text(AddTextField::ReviewDescription),
            AddControlId::Storage,
            AddControlId::Interpolate,
            AddControlId::PromptCandidate("name".to_owned()),
            AddControlId::Runner,
            AddControlId::NewRunner,
            AddControlId::EditSource,
        ];
        let mut previous_bottom = geometry.body.y;
        for target in expected {
            let hit = geometry
                .hits
                .iter()
                .find(|hit| hit.target == target)
                .unwrap_or_else(|| panic!("missing visible body row: {target:?}"));
            assert!(hit.area.width > 0 && hit.area.height > 0, "{target:?}");
            assert!(
                geometry.body.contains((hit.area.x, hit.area.y).into()),
                "{target:?}"
            );
            assert!(hit.area.bottom() <= geometry.body.bottom(), "{target:?}");
            assert!(
                hit.area.y >= previous_bottom,
                "rows overlap before {target:?}"
            );
            assert!(
                !hit_text(&terminal, hit.area).trim().is_empty(),
                "{target:?}"
            );
            previous_bottom = hit.area.bottom();
        }
    }

    #[test]
    fn add_recompose_cancels_changed_option_identity_but_preserves_equal_identity() {
        let state_with_runner = |runner: &str| {
            AddWorkflowState::from_review(ReviewState::from_source(
                SourceSnapshot {
                    path: "task.prompt.md".into(),
                    source_record: "task.prompt.md".to_owned(),
                    bytes: b"Hello".to_vec(),
                    permissions: SourcePermissions::default(),
                    executable: None,
                    is_regular: true,
                    is_directory: false,
                    is_draft: false,
                    identity: None,
                },
                KnownEntryKind::Prompt,
                ReviewDefaults {
                    runner_names: vec![runner.to_owned()],
                    ..ReviewDefaults::default()
                },
            ))
        };

        let alpha = state_with_runner("alpha");
        let beta = state_with_runner("beta");
        let mut changed = AddScreenSession::default();
        let (_, closed) = draw(&alpha, &mut changed, 80, 30);
        let runner = closed
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Runner)
            .unwrap()
            .area;
        assert!(click_control(&mut changed, &alpha, &closed, runner.x, runner.y).is_some());
        let (_, alpha_open) = draw(&alpha, &mut changed, 80, 30);
        let alpha_option = alpha_open
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::RunnerOption(1))
            .unwrap()
            .area;
        assert_eq!(
            changed.handle_event(
                mouse(
                    MouseEventKind::Down(MouseButton::Left),
                    alpha_option.x,
                    alpha_option.y,
                ),
                &alpha,
                &alpha_open,
            ),
            Some(AddScreenEvent::Changed)
        );
        let (_, beta_open) = draw(&beta, &mut changed, 80, 30);
        assert_eq!(
            changed.handle_event(
                mouse(
                    MouseEventKind::Up(MouseButton::Left),
                    alpha_option.x,
                    alpha_option.y,
                ),
                &beta,
                &beta_open,
            ),
            None,
            "a press on alpha must not activate beta after the runner inventory changes"
        );

        let mut equal = AddScreenSession::default();
        let (_, closed) = draw(&alpha, &mut equal, 80, 30);
        let runner = closed
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Runner)
            .unwrap()
            .area;
        assert!(click_control(&mut equal, &alpha, &closed, runner.x, runner.y).is_some());
        let (_, before) = draw(&alpha, &mut equal, 80, 30);
        let option = before
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::RunnerOption(1))
            .unwrap()
            .area;
        assert_eq!(
            equal.handle_event(
                mouse(MouseEventKind::Down(MouseButton::Left), option.x, option.y),
                &alpha,
                &before,
            ),
            Some(AddScreenEvent::Changed)
        );
        let (_, after) = draw(&alpha, &mut equal, 80, 30);
        assert_eq!(
            equal.handle_event(
                mouse(MouseEventKind::Up(MouseButton::Left), option.x, option.y),
                &alpha,
                &after,
            ),
            Some(AddScreenEvent::Action(AddAction::SetPromptRunner {
                name: "alpha".to_owned(),
                picked: true,
            })),
            "an equal semantic rerender must preserve a valid matching release"
        );
    }

    #[test]
    fn add_footer_clamps_after_growth_and_empty_inventory_has_no_rows() {
        let (empty, rows) = position_footer_chips(Vec::new(), 20);
        assert!(empty.is_empty());
        assert_eq!(rows, 0);

        let state = AddWorkflowState::new(Vec::new());
        let mut session = AddScreenSession::default();
        session.sync(&state);
        let mut narrow = Terminal::new(TestBackend::new(20, 1)).unwrap();
        let mut geometry = AddScreenGeometry::default();
        narrow
            .draw(|frame| {
                geometry.hits =
                    render_footer(frame, frame.area(), &state, &mut session, Locale::En);
            })
            .unwrap();
        assert_eq!(
            session.handle_event(mouse(MouseEventKind::ScrollDown, 0, 0), &state, &geometry,),
            Some(AddScreenEvent::Changed)
        );
        assert!(session.footer_scroll.scroll_offset() > 0);

        let mut wide = Terminal::new(TestBackend::new(120, 2)).unwrap();
        wide.draw(|frame| {
            let hits = render_footer(frame, frame.area(), &state, &mut session, Locale::En);
            assert!(!hits.is_empty());
        })
        .unwrap();
        assert_eq!(session.footer_scroll.scroll_offset(), 0);
    }

    /// Version 0.4 names the draft and says the copy is the only one
    /// (`src/skit/tui_add.py:176`). "Remove this entry:" belongs to the entry-removal modal, and a
    /// kept draft is a file, so the confirmation has to say which file and that it does not come
    /// back.
    #[test]
    fn the_draft_confirmation_names_the_draft_and_warns_it_is_the_only_copy() {
        let drafts = vec![skit_ui::DraftSummary {
            path: PathBuf::from("/tmp/drafts/skit-new-task.py"),
            modified: 1,
            identity: None,
            permissions: SourcePermissions::default(),
            content_hash: None,
        }];
        let mut state = AddWorkflowState::new(drafts);
        let _ = state.reduce(AddAction::SelectDraft(0));
        let _ = state.reduce(AddAction::DeleteSelectedDraft);
        assert_eq!(state.stage(), AddStage::ConfirmDraftDelete);

        let mut session = AddScreenSession::default();
        session.sync(&state);
        let (terminal, _) = draw(&state, &mut session, 80, 14);
        let rendered = text_of(&terminal);
        assert!(
            rendered.contains("Delete the draft \"skit-new-task.py\"? It is the only copy."),
            "{rendered}"
        );
        assert!(!rendered.contains("Remove this entry:"), "{rendered}");
    }

    /// A script that carries its own PEP 723 fence is read-only at add time, so the review has to
    /// print what the fence asks for: the Python requirement and each install
    /// (`src/skit/tui_add.py:935-940`). Naming only the dependencies drops the Python line, and a
    /// fence that declares neither would otherwise render as an empty list.
    #[test]
    fn a_declared_dependency_fence_names_its_python_and_its_installs() {
        let owned = source(
            "tool.py",
            b"# /// script\n# requires-python = \">=3.12\"\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
            KnownEntryKind::Python,
        );
        let mut session = AddScreenSession::default();
        session.sync(&owned);
        let (terminal, _) = draw(&owned, &mut session, 80, 20);
        let rendered = text_of(&terminal);
        assert!(
            rendered.contains("The script declares its own dependencies (PEP 723):"),
            "{rendered}"
        );
        assert!(rendered.contains("needs Python >=3.12"), "{rendered}");
        assert!(rendered.contains("installs rich"), "{rendered}");
    }

    /// The editable dependency and Python fields say where their prefill came from and what an
    /// empty value means (`src/skit/tui_add.py:925`, `:963`). Without the lines the reader cannot
    /// tell a scanned suggestion from a value skit will keep.
    #[test]
    fn the_editable_dependency_fields_explain_their_prefill() {
        let script = source(
            "tool.py",
            b"import rich\nprint(1)\n",
            KnownEntryKind::Python,
        );
        let mut session = AddScreenSession::default();
        session.sync(&script);
        let (terminal, _) = draw(&script, &mut session, 100, 24);
        let rendered = text_of(&terminal);
        assert!(
            rendered.contains("detected from the script's imports"),
            "{rendered}"
        );
        assert!(
            rendered.contains("Python version (requires-python)"),
            "{rendered}"
        );
    }
}
