//! Ephemeral state for mature terminal widgets.

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use ratatui_core::{
    layout::Rect,
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui_interact::{
    components::{
        Button, ButtonState, ButtonStyle, ButtonVariant, CheckBox, CheckBoxState, CheckBoxStyle,
        Select, SelectAction, SelectState, SelectStyle, Toast, ToastState, ToastStyle,
        handle_select_key,
    },
    state::FocusManager,
    traits::ClickRegion,
};
use ratatui_textarea::{CursorMove, TextArea as RichTextArea};
use ratatui_widgets::{
    block::Block,
    borders::Borders,
    paragraph::{Paragraph, Wrap},
    scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use skit_application::path_completion::{
    PathCompletionProvider, PathCompletionRequest, PathInputDialect,
};
use skit_domain::parameters::ParameterType;
use skit_i18n::{Locale, format_text, text};
use skit_ui::{
    Action, AddAction, AddWorkflowState, ChoicePresentation, FormControl, FormField, FormView,
    InputMode, LibraryState, ModalState, RunDegradationNotice, RunField, RunFieldRole, RunFormView,
    RunTokenError, RunValidationError, RunnerEditorOwner, Screen, UiCommand,
};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    HitRegion, HitTarget, RunFieldCommand, ViewGeometry, command_action,
    footer::FooterSession,
    map_event,
    pointer::{
        ClickDispatch, ClickOutcome, ClickTracker, EditableGeometry, HitMap, TextAreaGeometry,
        TextAreaViewport, display_cursor, display_scroll, is_primary_down, secret_display,
    },
    rowclip::{RowClip, bounded_textarea_lines, editor_cursor_virtual_row},
    run_field_command_action,
    screens::add::{AddScreenEvent, AddScreenGeometry, AddScreenSession, render_add},
    screens::library::{LibraryClickTarget, LibraryPointerHandling, LibraryScreenSession},
    screens::management::{
        HealthEventHandling, HealthScreenSession, RunnerEditorEventHandling, RunnerEditorSession,
        RunnerManagerEventHandling, RunnerManagerSession,
    },
    screens::modal::{ConfirmRemoveEvent, ConfirmRemoveSession, HelpScreenSession},
    screens::picker::{
        ChoicePickerGeometry, FilePickerEvent, FilePickerGeometry, FilePickerSession,
        PromptCandidatePickerEvent, PromptCandidatePickerSession, render_file_picker,
        render_prompt_candidate_picker,
    },
    screens::preferences::{
        AgentSkillOverlayEventHandling, PreferencesEventHandling, PreferencesWidgetSession,
    },
    screens::report::ReportScreenSession,
    screens::run_modal::{RunModalEvent, RunModalSession},
    screens::settings::{
        SettingsScreenEvent, SettingsScreenGeometry, SettingsScreenSession, render_settings,
    },
    theme::{ACCENT, BOX_DIM, BOX_MAROON, SELECT_BG, SELECT_FG, panel_block},
    viewport::{AlignmentSignature, VirtualScrollState},
};

/// Result of one stateful terminal event dispatch.
#[derive(Clone, Debug, PartialEq)]
pub enum EventHandling {
    /// Dispatch this frontend-neutral action through the reducer.
    Action(Action),
    /// A widget changed only ephemeral state such as its cursor or scroll offset.
    Consumed,
    /// No active widget or command accepted the event.
    Ignored,
}

/// One header shape that can own visible rows in the terminal layout.
pub(crate) enum HeaderKind<'a> {
    Library { query: &'a str, search: bool },
    Report(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TopLevelClickTarget {
    SearchInput,
    Command(UiCommand),
    RunFieldCommand(usize, RunFieldCommand),
    Library(LibraryClickTarget),
}

/// Stateful terminal widget session. This state is not serialized into `skit-ui`.
#[derive(Debug, Default)]
pub struct TuiSession {
    quit_armed_at: Option<Instant>,
    quit_toast: ToastState,
    search: SearchWidgetSession,
    library: LibraryScreenSession,
    help: HelpScreenSession,
    confirm_remove: ConfirmRemoveSession,
    run: RunWidgetSession,
    path_suggestions: PathSuggestionSession,
    run_modal: RunModalSession,
    preferences: PreferencesWidgetSession,
    report: ReportScreenSession,
    settings: SettingsScreenSession,
    settings_geometry: SettingsScreenGeometry,
    settings_prompt_overlay: Option<(PromptCandidatePickerSession, ChoicePickerGeometry)>,
    add: AddScreenSession,
    add_geometry: AddScreenGeometry,
    add_overlay: Option<AddOverlay>,
    health: HealthScreenSession,
    runners: RunnerManagerSession,
    runner_editor: RunnerEditorSession,
    form: FormWidgetSession,
    footer: FooterSession,
    clicks: HitMap<TopLevelClickTarget>,
    top_level_click: ClickTracker<TopLevelClickTarget>,
}

#[derive(Debug)]
struct PathSuggestionJob {
    generation: u64,
    field: usize,
    request: Box<PathCompletionRequest>,
}

#[derive(Debug)]
struct PathSuggestionResult {
    generation: u64,
    field: usize,
    value: String,
    suggestion: Option<String>,
}

#[derive(Debug)]
struct VisiblePathSuggestion {
    generation: u64,
    field: usize,
    value: String,
    suggestion: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PathSuggestionKey<'a> {
    generation: u64,
    field: usize,
    value: &'a str,
}

#[derive(Debug)]
struct ExpectedPathSuggestion {
    generation: u64,
    field: usize,
    request: PathCompletionRequest,
}

impl ExpectedPathSuggestion {
    fn key(&self) -> PathSuggestionKey<'_> {
        PathSuggestionKey {
            generation: self.generation,
            field: self.field,
            value: &self.request.value,
        }
    }

    fn request_key(&self) -> (usize, &PathCompletionRequest) {
        (self.field, &self.request)
    }
}

impl PathSuggestionResult {
    fn key(&self) -> PathSuggestionKey<'_> {
        PathSuggestionKey {
            generation: self.generation,
            field: self.field,
            value: &self.value,
        }
    }
}

impl VisiblePathSuggestion {
    fn key(&self) -> PathSuggestionKey<'_> {
        PathSuggestionKey {
            generation: self.generation,
            field: self.field,
            value: &self.value,
        }
    }
}

#[derive(Debug, Default)]
struct PathSuggestionSession {
    requests: Option<mpsc::SyncSender<PathSuggestionJob>>,
    results: Option<mpsc::Receiver<PathSuggestionResult>>,
    generation: u64,
    expected: Option<ExpectedPathSuggestion>,
    in_flight: bool,
    retry_pending: bool,
    visible: Option<VisiblePathSuggestion>,
}

impl PathSuggestionSession {
    fn new(provider: Arc<dyn PathCompletionProvider>) -> Self {
        let (request_tx, request_rx) = mpsc::sync_channel::<PathSuggestionJob>(2);
        let (result_tx, result_rx) = mpsc::channel::<PathSuggestionResult>();
        let request_rx = Arc::new(Mutex::new(request_rx));
        for _ in 0..2 {
            let provider = Arc::clone(&provider);
            let requests = Arc::clone(&request_rx);
            let results = result_tx.clone();
            let _ = thread::Builder::new()
                .name("skit-path-completion".to_owned())
                .spawn(move || run_path_suggestion_worker(provider, requests, results));
        }
        Self {
            requests: Some(request_tx),
            results: Some(result_rx),
            ..Self::default()
        }
    }

    fn ensure(&mut self, field: usize, request: Option<PathCompletionRequest>) {
        let Some(request) = request else {
            self.clear();
            return;
        };
        let current_matches = self
            .expected
            .as_ref()
            .is_some_and(|expected| expected.request_key() == (field, &request));
        if current_matches {
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.visible = None;
        let job = PathSuggestionJob {
            generation,
            field,
            request: Box::new(request.clone()),
        };
        match self.requests.as_ref().map(|sender| sender.try_send(job)) {
            Some(Ok(())) => {
                self.expected = Some(ExpectedPathSuggestion {
                    generation,
                    field,
                    request,
                });
                self.in_flight = true;
                self.retry_pending = false;
            }
            Some(Err(mpsc::TrySendError::Full(_))) => {
                self.expected = None;
                self.in_flight = false;
                self.retry_pending = true;
            }
            Some(Err(mpsc::TrySendError::Disconnected(_))) | None => self.clear(),
        }
    }

    fn refresh(&mut self) -> bool {
        let mut changed = false;
        let Some(results) = &self.results else {
            return false;
        };
        while let Ok(result) = results.try_recv() {
            let Some(expected) = self.expected.as_ref() else {
                continue;
            };
            if expected.key() != result.key() {
                continue;
            }
            self.visible = result.suggestion.and_then(|suggestion| {
                if suggestion == result.value || !suggestion.starts_with(&result.value) {
                    return None;
                }
                Some(VisiblePathSuggestion {
                    generation: result.generation,
                    field: result.field,
                    value: result.value,
                    suggestion,
                })
            });
            self.in_flight = false;
            changed = true;
        }
        changed || self.retry_pending
    }

    fn visible(&self, field: usize, value: &str) -> Option<&str> {
        self.visible.as_ref().and_then(|visible| {
            let expected = self.expected.as_ref()?;
            let requested = PathSuggestionKey {
                generation: visible.generation,
                field,
                value,
            };
            if visible.key() != requested || expected.key() != requested {
                return None;
            }
            Some(visible.suggestion.as_str())
        })
    }

    fn take(&mut self, field: usize, value: &str) -> Option<String> {
        let suggestion = self.visible(field, value)?.to_owned();
        self.clear();
        Some(suggestion)
    }

    fn clear(&mut self) {
        self.expected = None;
        self.in_flight = false;
        self.retry_pending = false;
        self.visible = None;
    }

    fn has_pending_work(&self) -> bool {
        self.in_flight || self.retry_pending
    }
}

fn run_path_suggestion_worker(
    provider: Arc<dyn PathCompletionProvider>,
    requests: Arc<Mutex<mpsc::Receiver<PathSuggestionJob>>>,
    results: mpsc::Sender<PathSuggestionResult>,
) {
    loop {
        let job = {
            let Ok(receiver) = requests.lock() else {
                return;
            };
            let Ok(job) = receiver.recv() else {
                return;
            };
            job
        };
        let request = *job.request;
        let suggestion = provider.complete(&request);
        let result = PathSuggestionResult {
            generation: job.generation,
            field: job.field,
            value: request.value,
            suggestion,
        };
        if results.send(result).is_err() {
            return;
        }
    }
}

#[derive(Debug)]
enum AddOverlay {
    File {
        session: FilePickerSession,
        geometry: FilePickerGeometry,
    },
    Prompt {
        session: PromptCandidatePickerSession,
        geometry: ChoicePickerGeometry,
    },
}

enum AddOverlayEvent {
    File(FilePickerEvent),
    Prompt(PromptCandidatePickerEvent),
}

#[derive(Debug, Default)]
struct SearchWidgetSession {
    input: LineInput,
    editable: Option<EditableGeometry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RunClickTarget {
    FocusField(usize),
    Checkbox(usize),
    Select(usize),
    SelectOption { field: usize, value: String },
    RadioOption { field: usize, value: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RunSignature {
    selector: String,
    fields: Vec<FieldSignature>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FieldSignature {
    key: String,
    shape: ControlShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ControlShape {
    Input {
        secret: bool,
        multiline: bool,
    },
    Checkbox,
    Choice {
        options: Vec<String>,
        presentation: ChoicePresentation,
    },
}

#[derive(Debug, Default)]
struct RunWidgetSession {
    signature: Option<RunSignature>,
    controls: Vec<WidgetControl>,
    focus: FocusManager<usize>,
    scroll: VirtualScrollState,
    viewport: Rect,
    visible_height: usize,
    row_starts: Vec<usize>,
    row_heights: Vec<usize>,
    editables: Vec<Option<EditableGeometry>>,
    textarea_editables: Vec<Option<TextAreaGeometry>>,
    textarea_viewports: Vec<TextAreaViewport>,
    select_areas: Vec<Option<Rect>>,
    dropdown_regions: Vec<Vec<ClickRegion<SelectAction>>>,
    pending_ensure_focus: bool,
    alignment: Option<AlignmentSignature<usize, (usize, usize, usize)>>,
    hits: HitMap<RunClickTarget>,
    click: ClickTracker<RunClickTarget>,
}

#[derive(Clone, Debug)]
struct RunLayout {
    items: Vec<PositionedRunItem>,
    control_starts: Vec<usize>,
    control_heights: Vec<usize>,
    height: usize,
}

#[derive(Clone, Debug)]
struct PositionedRunItem {
    start: usize,
    height: usize,
    item: RunRenderItem,
}

#[derive(Clone, Debug)]
enum RunRenderItem {
    Copy(RunCopy),
    /// One label row and the chips that share it.
    ///
    /// Version 0.4 builds a single `Static` from the label plus its browse/insert/default links
    /// (`src/skit/tui_form.py:190-215`), so a field costs one row here, not two.
    Chips {
        label: Option<RunCopy>,
        chips: Vec<RunChip>,
    },
    Control(usize),
    Spacer,
}

#[derive(Clone, Debug)]
struct RunCopy {
    line: Line<'static>,
}

#[derive(Clone, Debug)]
struct RunChip {
    label: String,
    x: u16,
    width: u16,
    target: HitTarget,
}

#[derive(Debug, Default)]
struct FormWidgetSession {
    signature: Option<Vec<FieldSignature>>,
    controls: Vec<FormWidgetControl>,
    clicks: HitMap<usize>,
    focus: FocusManager<usize>,
    scroll: VirtualScrollState,
    viewport: Rect,
    visible_height: usize,
    row_starts: Vec<usize>,
    row_heights: Vec<usize>,
    editables: Vec<Option<EditableGeometry>>,
    textarea_editables: Vec<Option<TextAreaGeometry>>,
    textarea_viewports: Vec<TextAreaViewport>,
    click: ClickTracker<usize>,
    pending_ensure_focus: bool,
    alignment: Option<AlignmentSignature<usize, (usize, usize, usize)>>,
}

#[derive(Debug)]
enum FormWidgetControl {
    Input {
        state: LineInput,
        secret: bool,
        focused: bool,
    },
    TextArea {
        state: Box<RichTextArea<'static>>,
        focused: bool,
        undo_group: usize,
        redo_group: usize,
    },
}

#[derive(Debug)]
enum WidgetControl {
    Input {
        state: LineInput,
        secret: bool,
        focused: bool,
    },
    TextArea {
        state: Box<RichTextArea<'static>>,
        focused: bool,
        undo_group: usize,
        redo_group: usize,
    },
    Checkbox(CheckBoxState),
    Choice {
        state: SelectState,
        options: Vec<String>,
        presentation: ChoicePresentation,
        buttons: Vec<ButtonState>,
    },
}

impl TuiSession {
    /// Construct a session whose path queries run on bounded background workers.
    #[must_use]
    pub fn with_path_completion(provider: Arc<dyn PathCompletionProvider>) -> Self {
        Self {
            path_suggestions: PathSuggestionSession::new(provider),
            ..Self::default()
        }
    }

    /// Apply completed background work before the next draw.
    #[must_use]
    pub fn refresh_background(&mut self) -> bool {
        self.path_suggestions.refresh()
    }

    /// Report whether the terminal must poll for path-completion progress.
    #[must_use]
    pub(crate) fn has_pending_path_completion(&self) -> bool {
        self.path_suggestions.has_pending_work()
    }

    /// Dispatch one terminal event through the active mature widget first.
    #[must_use]
    pub fn handle_event(
        &mut self,
        event: Event,
        state: &LibraryState,
        geometry: &ViewGeometry,
    ) -> EventHandling {
        if matches!(event, Event::Resize(_, _)) {
            self.top_level_click.cancel();
            self.cancel_owner_clicks();
        }
        if let Event::Mouse(mouse) = &event
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
                    | MouseEventKind::ScrollLeft
                    | MouseEventKind::ScrollRight
            )
        {
            self.top_level_click.cancel();
            self.cancel_owner_clicks();
        }
        if is_ctrl_c(&event) {
            return self.handle_ctrl_c();
        }
        if let Some(handling) = self.handle_blocking_overlay_event(&event, state, geometry) {
            return handling;
        }
        if !crate::footer::is_suppressed(state)
            && let Event::Mouse(mouse) = &event
            && self.footer.handle_mouse(mouse)
        {
            return EventHandling::Consumed;
        }
        if state.modal().is_none()
            && matches!(state.screen(), Screen::Library)
            && let Event::Mouse(mouse) = &event
            && matches!(
                mouse.kind,
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
            )
        {
            return match self.library.handle_wheel(mouse, geometry) {
                LibraryPointerHandling::Action(action) => EventHandling::Action(action),
                LibraryPointerHandling::Consumed => EventHandling::Consumed,
                LibraryPointerHandling::Ignored => EventHandling::Ignored,
            };
        }
        if let Event::Mouse(mouse) = &event {
            let mut target = self.clicks.topmost(mouse.column, mouse.row).cloned();
            if target.is_none()
                && state.modal().is_none()
                && matches!(state.screen(), Screen::Library)
            {
                target = self
                    .library
                    .click_target(mouse, geometry)
                    .map(TopLevelClickTarget::Library);
            }
            if is_primary_down(mouse)
                && matches!(target, Some(TopLevelClickTarget::SearchInput))
                && let Some(editable) = self.search.editable
            {
                let _ = editable.place_cursor(&mut self.search.input, mouse.column, mouse.row);
            }
            if target.is_some()
                && matches!(
                    mouse.kind,
                    MouseEventKind::Down(_) | MouseEventKind::Up(_) | MouseEventKind::Drag(_)
                )
            {
                self.cancel_owner_clicks();
            }
            let dispatch = self.top_level_click.dispatch(mouse, target.as_ref());
            if let ClickDispatch::Captured(outcome) = dispatch {
                return match outcome {
                    ClickOutcome::Armed => EventHandling::Consumed,
                    ClickOutcome::Activated(TopLevelClickTarget::SearchInput) => {
                        if state.input_mode() == InputMode::Browse {
                            EventHandling::Action(Action::BeginSearch)
                        } else {
                            EventHandling::Consumed
                        }
                    }
                    ClickOutcome::Activated(TopLevelClickTarget::Command(command)) => {
                        EventHandling::Action(command_action(
                            command,
                            state.command_context(),
                            geometry,
                        ))
                    }
                    ClickOutcome::Activated(TopLevelClickTarget::RunFieldCommand(
                        field,
                        command,
                    )) => EventHandling::Action(run_field_command_action(field, command)),
                    ClickOutcome::Activated(TopLevelClickTarget::Library(target)) => self
                        .library
                        .activate_click(target, state)
                        .map_or(EventHandling::Consumed, EventHandling::Action),
                    ClickOutcome::Ignored => EventHandling::Ignored,
                };
            }
        }
        if state.modal().is_none()
            && matches!(state.screen(), Screen::Library)
            && self.library.handle_event(&event)
        {
            return EventHandling::Consumed;
        }
        if state.modal().is_none()
            && matches!(state.screen(), Screen::Library)
            && state.input_mode() == InputMode::Search
        {
            self.search.sync(state.query());
            return self.handle_search_event(event, state, geometry);
        }
        if matches!(state.modal(), Some(ModalState::Help)) && self.help.handle_event(&event) {
            return EventHandling::Consumed;
        }
        if matches!(state.modal(), Some(ModalState::ConfirmRemove { .. })) {
            match self.confirm_remove.handle_event(&event) {
                ConfirmRemoveEvent::Submit => return EventHandling::Action(Action::Submit),
                ConfirmRemoveEvent::Close => return EventHandling::Action(Action::Back),
                ConfirmRemoveEvent::Consumed => return EventHandling::Consumed,
                ConfirmRemoveEvent::Ignored => {}
            }
        }
        if let Some(
            modal @ (ModalState::RunPresetName { .. }
            | ModalState::RunTokenMenu { .. }
            | ModalState::RunEnvironmentPicker { .. }
            | ModalState::RunFilePicker { .. }),
        ) = state.modal()
        {
            let fallback = event.clone();
            return match self.run_modal.handle_event(event, modal) {
                RunModalEvent::Handling(EventHandling::Ignored) => {
                    map_event(fallback, state, geometry)
                        .map_or(EventHandling::Ignored, EventHandling::Action)
                }
                RunModalEvent::Handling(handling) => handling,
                RunModalEvent::Insert { field, text } => self.insert_run_text(field, &text),
                RunModalEvent::OpenEnvironment { field } => {
                    EventHandling::Action(Action::OpenRunEnvironmentPicker(field))
                }
                RunModalEvent::OpenFile { field } => {
                    EventHandling::Action(Action::OpenRunFilePicker(field))
                }
            };
        }
        if state.modal().is_some() {
            return map_event(event, state, geometry)
                .map_or(EventHandling::Ignored, EventHandling::Action);
        }
        if let Screen::Health(view) = state.screen() {
            return match self.health.handle_event(event, view) {
                HealthEventHandling::Action(action) => {
                    EventHandling::Action(Action::Health(action))
                }
                HealthEventHandling::Consumed => EventHandling::Consumed,
                HealthEventHandling::Ignored => EventHandling::Ignored,
            };
        }
        if matches!(state.screen(), Screen::Report(_)) && self.report.handle_event(&event) {
            return EventHandling::Consumed;
        }
        if let Screen::Runners(view) = state.screen() {
            return match self.runners.handle_event(event, view) {
                RunnerManagerEventHandling::Action(action) => {
                    EventHandling::Action(Action::Runners(action))
                }
                RunnerManagerEventHandling::Consumed => EventHandling::Consumed,
                RunnerManagerEventHandling::Ignored => EventHandling::Ignored,
            };
        }
        if let Screen::Add(view) = state.screen() {
            return self.handle_add_event(event, state, view, geometry);
        }
        if let Screen::Settings(view) = state.screen() {
            return match self
                .settings
                .handle_event(event.clone(), view, &self.settings_geometry)
            {
                Some(SettingsScreenEvent::Action(action)) => {
                    EventHandling::Action(Action::Settings(action))
                }
                Some(SettingsScreenEvent::Changed) => EventHandling::Consumed,
                Some(SettingsScreenEvent::OpenPromptCandidates) => {
                    self.open_settings_prompt_picker(view);
                    EventHandling::Consumed
                }
                None => map_event(event, state, geometry)
                    .map_or(EventHandling::Ignored, EventHandling::Action),
            };
        }
        if let Screen::Preferences(view) = state.screen() {
            return match self.preferences.handle_event(event.clone(), view) {
                PreferencesEventHandling::Action(action) => {
                    EventHandling::Action(Action::Preferences(action))
                }
                PreferencesEventHandling::Consumed => EventHandling::Consumed,
                PreferencesEventHandling::Ignored => map_event(event, state, geometry)
                    .map_or(EventHandling::Ignored, EventHandling::Action),
            };
        }
        if let Screen::Run(form) = state.screen() {
            self.run.sync(form);
            return match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_run_key(key, form)
                }
                Event::Mouse(mouse) => {
                    let handling = self.handle_run_mouse(mouse, form, geometry);
                    if handling == EventHandling::Ignored {
                        map_event(Event::Mouse(mouse), state, geometry)
                            .map_or(EventHandling::Ignored, EventHandling::Action)
                    } else {
                        handling
                    }
                }
                Event::Paste(value) => self.handle_run_paste(&value, form),
                Event::FocusGained | Event::FocusLost | Event::Key(_) | Event::Resize(_, _) => {
                    EventHandling::Ignored
                }
            };
        }
        if let Screen::Form(form) = state.screen() {
            self.form.sync(form);
            return match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    self.handle_form_key(key, form)
                }
                Event::Mouse(mouse) => {
                    let handling = self.handle_form_mouse(mouse, form, geometry);
                    if handling == EventHandling::Ignored {
                        map_event(Event::Mouse(mouse), state, geometry)
                            .map_or(EventHandling::Ignored, EventHandling::Action)
                    } else {
                        handling
                    }
                }
                Event::Paste(value) => self.handle_form_paste(&value, form),
                Event::FocusGained | Event::FocusLost | Event::Key(_) | Event::Resize(_, _) => {
                    EventHandling::Ignored
                }
            };
        }
        map_event(event, state, geometry).map_or(EventHandling::Ignored, EventHandling::Action)
    }

    fn handle_blocking_overlay_event(
        &mut self,
        event: &Event,
        state: &LibraryState,
        geometry: &ViewGeometry,
    ) -> Option<EventHandling> {
        if let Some(ModalState::RunnerEditor { owner, view, .. }) = state.modal() {
            self.top_level_click.cancel();
            match owner {
                RunnerEditorOwner::Run { .. } => self.run.click.cancel(),
                RunnerEditorOwner::Add => self.add.cancel_click(),
                RunnerEditorOwner::Settings { .. } => self.settings.cancel_click(),
            }
            let pointer_lifecycle = matches!(
                event,
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(_) | MouseEventKind::Up(_) | MouseEventKind::Drag(_),
                    ..
                })
            );
            return Some(match self.runner_editor.handle_event(event.clone(), view) {
                RunnerEditorEventHandling::Action(action) => {
                    EventHandling::Action(Action::RunnerEditor(action))
                }
                RunnerEditorEventHandling::Consumed => EventHandling::Consumed,
                RunnerEditorEventHandling::Ignored if pointer_lifecycle => EventHandling::Consumed,
                RunnerEditorEventHandling::Ignored => EventHandling::Ignored,
            });
        }
        if let Screen::Add(view) = state.screen()
            && self.add_overlay.is_some()
        {
            self.top_level_click.cancel();
            self.add.cancel_click();
            return Some(self.handle_add_event(event.clone(), state, view, geometry));
        }
        if matches!(state.screen(), Screen::Settings(_))
            && let Some((session, overlay_geometry)) = self.settings_prompt_overlay.as_mut()
        {
            self.top_level_click.cancel();
            self.settings.cancel_click();
            let blocked_input = matches!(event, Event::Key(_) | Event::Mouse(_) | Event::Paste(_));
            return Some(
                match session.handle_event(event.clone(), overlay_geometry) {
                    Some(PromptCandidatePickerEvent::Changed) => EventHandling::Consumed,
                    Some(PromptCandidatePickerEvent::Cancelled) => {
                        self.settings_prompt_overlay = None;
                        EventHandling::Consumed
                    }
                    Some(PromptCandidatePickerEvent::Accepted(names)) => {
                        self.settings_prompt_overlay = None;
                        EventHandling::Action(Action::Settings(
                            skit_ui::SettingsAction::SetPromptCandidates(names),
                        ))
                    }
                    None if blocked_input => EventHandling::Consumed,
                    None => EventHandling::Ignored,
                },
            );
        }
        if let Screen::Preferences(view) = state.screen()
            && let Some(picker) = view.agent_skill_install()
        {
            self.top_level_click.cancel();
            self.preferences.cancel_underlay_click();
            return Some(
                match self
                    .preferences
                    .handle_agent_skill_overlay_event(event.clone(), view, picker)
                {
                    AgentSkillOverlayEventHandling::Action(action) => {
                        EventHandling::Action(Action::Preferences(action))
                    }
                    AgentSkillOverlayEventHandling::Consumed => EventHandling::Consumed,
                },
            );
        }
        None
    }

    fn cancel_owner_clicks(&mut self) {
        self.runner_editor.cancel_click();
        self.run_modal.cancel_click();
        self.run.click.cancel();
        self.form.click.cancel();
        self.preferences.cancel_click();
        self.settings.cancel_click();
        if let Some((picker, _)) = self.settings_prompt_overlay.as_mut() {
            picker.cancel_click();
        }
        self.add.cancel_click();
        match self.add_overlay.as_mut() {
            Some(AddOverlay::File { session, .. }) => session.cancel_click(),
            Some(AddOverlay::Prompt { session, .. }) => session.cancel_click(),
            None => {}
        }
        self.health.cancel_click();
        self.runners.cancel_click();
    }

    fn open_settings_prompt_picker(&mut self, view: &skit_ui::SettingsView) {
        if view.prompt_picker_available() {
            self.settings_prompt_overlay = Some((
                PromptCandidatePickerSession::new(view.prompt_picker()),
                ChoicePickerGeometry::default(),
            ));
        }
    }

    fn handle_add_event(
        &mut self,
        event: Event,
        state: &LibraryState,
        view: &AddWorkflowState,
        geometry: &ViewGeometry,
    ) -> EventHandling {
        let overlay_event = match self.add_overlay.as_mut() {
            Some(AddOverlay::File { session, geometry }) => session
                .handle_event(event.clone(), geometry)
                .map(AddOverlayEvent::File),
            Some(AddOverlay::Prompt { session, geometry }) => session
                .handle_event(event.clone(), geometry)
                .map(AddOverlayEvent::Prompt),
            None => None,
        };
        if let Some(overlay_event) = overlay_event {
            return match overlay_event {
                AddOverlayEvent::File(FilePickerEvent::Changed)
                | AddOverlayEvent::Prompt(PromptCandidatePickerEvent::Changed) => {
                    EventHandling::Consumed
                }
                AddOverlayEvent::File(FilePickerEvent::Cancelled)
                | AddOverlayEvent::Prompt(PromptCandidatePickerEvent::Cancelled) => {
                    self.add_overlay = None;
                    EventHandling::Consumed
                }
                AddOverlayEvent::File(FilePickerEvent::Accepted(paths)) => {
                    self.add_overlay = None;
                    paths
                        .into_iter()
                        .next()
                        .map_or(EventHandling::Consumed, |path| {
                            EventHandling::Action(Action::Add(AddAction::SetSourcePath(
                                path.to_string_lossy().into_owned(),
                            )))
                        })
                }
                AddOverlayEvent::Prompt(PromptCandidatePickerEvent::Accepted(names)) => {
                    self.add_overlay = None;
                    EventHandling::Action(Action::Add(AddAction::SetPromptCandidates(names)))
                }
            };
        }
        if self.add_overlay.is_some() {
            if matches!(event, Event::Mouse(_)) {
                return EventHandling::Consumed;
            }
            return map_event(event, state, geometry)
                .map_or(EventHandling::Ignored, EventHandling::Action);
        }
        match self
            .add
            .handle_event(event.clone(), view, &self.add_geometry)
        {
            Some(AddScreenEvent::Action(action)) => EventHandling::Action(Action::Add(action)),
            Some(AddScreenEvent::OpenPathPicker(contract)) => {
                self.add_overlay = Some(AddOverlay::File {
                    session: FilePickerSession::new(contract),
                    geometry: FilePickerGeometry::default(),
                });
                EventHandling::Consumed
            }
            Some(AddScreenEvent::OpenPromptCandidates) => {
                if let Some(picker) = view.review().map(skit_ui::ReviewState::prompt_picker) {
                    self.add_overlay = Some(AddOverlay::Prompt {
                        session: PromptCandidatePickerSession::new(picker),
                        geometry: ChoicePickerGeometry::default(),
                    });
                }
                EventHandling::Consumed
            }
            Some(AddScreenEvent::OpenRunnerEditor) => {
                EventHandling::Action(Action::OpenAddRunnerEditor)
            }
            Some(AddScreenEvent::Changed) => EventHandling::Consumed,
            None => map_event(event, state, geometry)
                .map_or(EventHandling::Ignored, EventHandling::Action),
        }
    }

    pub(crate) fn begin_render(&mut self, state: &LibraryState) {
        self.quit_toast.clear_if_expired();
        self.clicks.clear();
        self.search.sync(state.query());
        if let Screen::Run(form) = state.screen() {
            self.run.sync(form);
            let field = form.focused();
            let value = self.run.input_value(field).map(str::to_owned);
            let request = value
                .as_deref()
                .and_then(|value| form.path_completion_request(field, value, host_path_dialect()));
            self.path_suggestions.ensure(field, request);
        } else if let Screen::Add(view) = state.screen() {
            self.path_suggestions.clear();
            self.add.sync(view);
        } else if let Screen::Settings(view) = state.screen() {
            self.path_suggestions.clear();
            self.settings.sync(view);
        } else if let Screen::Form(form) = state.screen() {
            self.path_suggestions.clear();
            self.form.sync(form);
        } else {
            self.path_suggestions.clear();
            self.add_overlay = None;
            self.settings_prompt_overlay = None;
        }
    }

    pub(crate) fn register_geometry(&mut self, geometry: &ViewGeometry) {
        for hit in &geometry.hits {
            match hit.action {
                HitTarget::Command(command) => {
                    self.clicks
                        .register(hit.rect, TopLevelClickTarget::Command(command));
                }
                HitTarget::RunFieldCommand { field, command } => self.clicks.register(
                    hit.rect,
                    TopLevelClickTarget::RunFieldCommand(field, command),
                ),
                HitTarget::FocusField(_) => {}
            }
        }
    }

    pub(crate) fn render_footer(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
        decorated: bool,
    ) -> Vec<HitRegion> {
        self.footer
            .render_with_decoration(frame, area, state, locale, decorated)
    }

    pub(crate) fn render_quit_toast(&mut self, frame: &mut Frame, locale: Locale) {
        self.quit_toast.clear_if_expired();
        let Some(source) = self.quit_toast.get_message() else {
            return;
        };
        let message = text(locale, source);
        Toast::new(&message)
            .style(ToastStyle::Info)
            .max_width(frame.area().width.saturating_sub(2))
            .render_with_clear(frame.area(), frame.buffer_mut());
    }

    fn handle_ctrl_c(&mut self) -> EventHandling {
        const WINDOW: Duration = Duration::from_secs(2);
        let now = Instant::now();
        if self
            .quit_armed_at
            .is_some_and(|armed| now.saturating_duration_since(armed) <= WINDOW)
        {
            self.quit_armed_at = None;
            self.quit_toast.clear();
            return EventHandling::Action(Action::Quit);
        }
        self.quit_armed_at = Some(now);
        self.quit_toast
            .show("Press Ctrl+C again to quit", WINDOW.as_millis() as i64);
        EventHandling::Consumed
    }

    pub(crate) fn render_header(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        kind: HeaderKind<'_>,
        locale: Locale,
    ) {
        let title = match kind {
            HeaderKind::Library { query, search } => {
                self.search.sync(query);
                let label = text(locale, "Search");
                self.search.editable = if area.height < 3 {
                    render_flat_search_input(frame, area, &self.search.input, search, &label)
                } else {
                    render_line_input(frame, area, &self.search.input, false, search, &label)
                };
                self.clicks.register(area, TopLevelClickTarget::SearchInput);
                return;
            }
            HeaderKind::Report(title) => text(locale, title).into_owned(),
        };
        frame.render_widget(
            Paragraph::new(title).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(BOX_DIM))
                    .title(" skit "),
            ),
            area,
        );
    }

    pub(crate) fn render_form(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        form: &FormView,
        locale: Locale,
    ) -> ViewGeometry {
        self.form.sync(form);
        self.form.clicks.clear();
        let block = panel_block(crate::form_title(locale, form), BOX_MAROON);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.form.prepare_layout(form, inner);
        self.form.editables = vec![None; self.form.controls.len()];
        self.form.textarea_editables = vec![None; self.form.controls.len()];

        let mut hits = Vec::new();
        for (index, field) in form.fields.iter().enumerate() {
            let Some(clip) = self.form.visible_band(index) else {
                continue;
            };
            let label = crate::field_label(locale, field);
            match &mut self.form.controls[index] {
                FormWidgetControl::Input {
                    state,
                    secret,
                    focused,
                } => {
                    self.form.editables[index] =
                        render_line_input_band(frame, clip, state, *secret, *focused, &label, None);
                }
                FormWidgetControl::TextArea { state, focused, .. } => {
                    self.form.textarea_editables[index] = render_textarea_band(
                        frame,
                        clip,
                        state,
                        &mut self.form.textarea_viewports[index],
                        *focused,
                        &label,
                    );
                }
            }
            self.form.clicks.register(clip.area(), index);
            hits.push(HitRegion {
                rect: clip.area(),
                action: HitTarget::FocusField(index),
            });
        }
        ViewGeometry {
            rows: inner,
            first_visible: self.form.scroll.scroll_offset(),
            hits,
            detail_pane_visible: false,
        }
    }

    pub(crate) fn render_library(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
    ) -> ViewGeometry {
        self.library.render(frame, area, state, locale)
    }

    pub(crate) fn render_help(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        locale: Locale,
    ) -> ViewGeometry {
        self.help.render(frame, area, locale)
    }

    pub(crate) fn render_confirm_remove(
        &mut self,
        frame: &mut Frame,
        name: &str,
        original_file_preserved: bool,
        locale: Locale,
    ) -> ViewGeometry {
        self.confirm_remove
            .render(frame, name, original_file_preserved, locale)
    }

    pub(crate) fn render_run(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        form: &RunFormView,
        locale: Locale,
    ) -> ViewGeometry {
        self.run.sync(form);
        self.run.hits.clear();
        let block = panel_block(
            format!("{} {}", text(locale, "Run"), form.name()),
            BOX_MAROON,
        );
        let inner = block.inner(area);
        frame.render_widget(block, area);
        // Version 0.4 hosts the form in a `VerticalScroll`, whose scrollbar is the only thing that
        // tells a user the content continues below the fold (`src/skit/tui_form.py:380-384`).
        // Measure against the full width first, then keep a column for the bar when it is needed.
        let overflows = run_layout(form, locale, inner.width).height > usize::from(inner.height);
        let content = if overflows {
            Rect {
                width: inner.width.saturating_sub(1),
                ..inner
            }
        } else {
            inner
        };
        let layout = run_layout(form, locale, content.width);
        self.run.prepare_layout(&layout, form.focused(), content);
        self.run.editables = vec![None; self.run.controls.len()];
        self.run.textarea_editables = vec![None; self.run.controls.len()];
        self.run.select_areas = vec![None; self.run.controls.len()];
        self.run.dropdown_regions = vec![Vec::new(); self.run.controls.len()];

        let mut hits = Vec::new();
        for item in &layout.items {
            let Some(visible) = self.run.visible_rect(item.start, item.height) else {
                continue;
            };
            match &item.item {
                RunRenderItem::Copy(copy) => {
                    let clipped_top = item
                        .start
                        .max(self.run.scroll.scroll_offset())
                        .saturating_sub(item.start);
                    frame.render_widget(
                        Paragraph::new(copy.line.clone())
                            .wrap(Wrap { trim: false })
                            .scroll((u16::try_from(clipped_top).unwrap_or(u16::MAX), 0)),
                        visible,
                    );
                }
                RunRenderItem::Chips { label, chips } => {
                    if let Some(label) = label {
                        frame.render_widget(Paragraph::new(label.line.clone()), visible);
                    }
                    self.render_run_chips(frame, visible, chips, &mut hits);
                }
                RunRenderItem::Control(index) => {
                    let is_full = usize::from(visible.height) == item.height;
                    let is_textarea = matches!(
                        self.run.controls.get(*index),
                        Some(WidgetControl::TextArea { .. })
                    );
                    if is_full || is_textarea {
                        let clipped_top =
                            self.run.scroll.scroll_offset().saturating_sub(item.start);
                        self.render_run_control(
                            frame,
                            RowClip::new(item.height, clipped_top, visible),
                            *index,
                            locale,
                            &mut hits,
                        );
                    }
                }
                RunRenderItem::Spacer => {}
            }
        }
        let mut scrollbar =
            ScrollbarState::new(layout.height.saturating_sub(usize::from(content.height)))
                .position(self.run.scroll.scroll_offset())
                .viewport_content_length(usize::from(content.height));
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight).style(run_scrollbar_style()),
            inner,
            &mut scrollbar,
        );
        self.render_open_dropdowns(frame);

        ViewGeometry {
            rows: content,
            first_visible: self.run.scroll.scroll_offset(),
            hits,
            detail_pane_visible: false,
        }
    }

    pub(crate) fn render_preferences(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &skit_ui::PreferencesView,
        locale: Locale,
    ) -> ViewGeometry {
        self.preferences.render(frame, area, view, locale);
        ViewGeometry {
            rows: area,
            first_visible: 0,
            hits: Vec::new(),
            detail_pane_visible: false,
        }
    }

    pub(crate) fn render_report(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        report: &skit_ui::ReportView,
        locale: Locale,
    ) -> ViewGeometry {
        self.report.render(frame, area, report, locale)
    }

    pub(crate) fn render_settings(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &skit_ui::SettingsView,
        locale: Locale,
    ) -> ViewGeometry {
        if let Some((session, geometry)) = self.settings_prompt_overlay.as_mut() {
            *geometry = render_prompt_candidate_picker(frame, area, session, locale);
            return ViewGeometry {
                rows: geometry.rows,
                first_visible: 0,
                hits: Vec::new(),
                detail_pane_visible: false,
            };
        }
        self.settings_geometry = render_settings(frame, area, view, &mut self.settings, locale);
        ViewGeometry {
            rows: self.settings_geometry.body,
            first_visible: self.settings_geometry.first_visible,
            hits: Vec::new(),
            detail_pane_visible: false,
        }
    }

    pub(crate) fn render_add(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &AddWorkflowState,
        locale: Locale,
    ) -> ViewGeometry {
        match self.add_overlay.as_mut() {
            Some(AddOverlay::File { session, geometry }) => {
                *geometry = render_file_picker(frame, area, session, locale);
                ViewGeometry {
                    rows: geometry.rows,
                    first_visible: 0,
                    hits: Vec::new(),
                    detail_pane_visible: false,
                }
            }
            Some(AddOverlay::Prompt { session, geometry }) => {
                *geometry = render_prompt_candidate_picker(frame, area, session, locale);
                ViewGeometry {
                    rows: geometry.rows,
                    first_visible: 0,
                    hits: Vec::new(),
                    detail_pane_visible: false,
                }
            }
            None => {
                self.add_geometry = render_add(frame, area, view, &mut self.add, locale);
                ViewGeometry {
                    rows: self.add_geometry.body,
                    first_visible: self.add_geometry.first_visible,
                    hits: Vec::new(),
                    detail_pane_visible: false,
                }
            }
        }
    }

    pub(crate) fn render_health(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &skit_ui::HealthView,
        locale: Locale,
    ) -> ViewGeometry {
        self.health.render(frame, area, view, locale);
        ViewGeometry {
            rows: area,
            first_visible: 0,
            hits: Vec::new(),
            detail_pane_visible: false,
        }
    }

    pub(crate) fn render_runners(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &skit_ui::RunnerManagerView,
        locale: Locale,
    ) -> ViewGeometry {
        self.runners.render(frame, area, view, locale);
        ViewGeometry {
            rows: area,
            first_visible: 0,
            hits: Vec::new(),
            detail_pane_visible: false,
        }
    }

    pub(crate) fn render_runner_editor(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        view: &skit_ui::RunnerEditorView,
        locale: Locale,
    ) {
        self.runner_editor.render(frame, area, view, locale);
    }

    pub(crate) fn render_run_modal(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        modal: &ModalState,
        locale: Locale,
    ) -> ViewGeometry {
        self.run_modal.render(frame, area, modal, locale)
    }

    fn handle_search_event(
        &mut self,
        event: Event,
        state: &LibraryState,
        geometry: &ViewGeometry,
    ) -> EventHandling {
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.code == KeyCode::Esc {
                    return EventHandling::Action(Action::FinishSearch);
                }
                let before = self.search.input.value().to_owned();
                if self.search.input.handle_event(&Event::Key(key)).is_some() {
                    return if before == self.search.input.value() {
                        EventHandling::Consumed
                    } else {
                        EventHandling::Action(Action::SetSearchQuery(
                            self.search.input.value().to_owned(),
                        ))
                    };
                }
                map_event(Event::Key(key), state, geometry)
                    .map_or(EventHandling::Ignored, EventHandling::Action)
            }
            Event::Paste(value) => {
                for character in value.chars() {
                    let _ = self
                        .search
                        .input
                        .handle(InputRequest::InsertChar(character));
                }
                EventHandling::Action(Action::SetSearchQuery(self.search.input.value().to_owned()))
            }
            Event::Mouse(mouse) => map_event(Event::Mouse(mouse), state, geometry)
                .map_or(EventHandling::Ignored, EventHandling::Action),
            Event::FocusGained | Event::FocusLost | Event::Key(_) | Event::Resize(_, _) => {
                EventHandling::Ignored
            }
        }
    }

    fn render_run_control(
        &mut self,
        frame: &mut Frame,
        clip: RowClip,
        index: usize,
        locale: Locale,
        hits: &mut Vec<HitRegion>,
    ) {
        let area = clip.area();
        let (Some(area_width), Some(_height)) = (
            std::num::NonZeroU16::new(area.width),
            std::num::NonZeroU16::new(area.height),
        ) else {
            return;
        };
        let select_style = select_style();
        match &mut self.run.controls[index] {
            WidgetControl::Input {
                state,
                secret,
                focused,
            } => {
                let suggestion = (*focused)
                    .then(|| self.path_suggestions.visible(index, state.value()))
                    .flatten();
                self.run.editables[index] = render_line_input_with_suggestion(
                    frame, area, state, *secret, *focused, "", suggestion,
                );
                self.run
                    .hits
                    .register(area, RunClickTarget::FocusField(index));
                hits.push(HitRegion {
                    rect: area,
                    action: HitTarget::FocusField(index),
                });
            }
            WidgetControl::TextArea { state, focused, .. } => {
                self.run.textarea_editables[index] = render_textarea_band(
                    frame,
                    clip,
                    state,
                    &mut self.run.textarea_viewports[index],
                    *focused,
                    "",
                );
                self.run
                    .hits
                    .register(area, RunClickTarget::FocusField(index));
                hits.push(HitRegion {
                    rect: area,
                    action: HitTarget::FocusField(index),
                });
            }
            WidgetControl::Checkbox(state) => {
                let shown = text(locale, if state.checked { "on" } else { "off" });
                let region = CheckBox::new(&shown, state)
                    .style(checkbox_style())
                    .render_stateful(area, frame.buffer_mut());
                self.run
                    .hits
                    .register(region.area, RunClickTarget::Checkbox(index));
                hits.push(HitRegion {
                    rect: region.area,
                    action: HitTarget::FocusField(index),
                });
            }
            WidgetControl::Choice {
                state,
                options,
                presentation: ChoicePresentation::Picker,
                ..
            } => {
                let placeholder = text(locale, "Select");
                let region = Select::new(options, state)
                    .label("")
                    .placeholder(&placeholder)
                    .style(select_style)
                    .render_stateful(frame, area);
                self.run.select_areas[index] = Some(region.area);
                self.run
                    .hits
                    .register(region.area, RunClickTarget::Select(index));
                hits.push(HitRegion {
                    rect: region.area,
                    action: HitTarget::FocusField(index),
                });
            }
            WidgetControl::Choice {
                state,
                options,
                presentation: ChoicePresentation::Radio,
                buttons,
            } => {
                let field_area = area;
                self.run
                    .hits
                    .register(field_area, RunClickTarget::FocusField(index));
                let mut x = area.x;
                let mut y = area.y;
                for (option_label, button) in options.iter().zip(buttons.iter()) {
                    let width = u16::try_from(option_label.width().saturating_add(2))
                        .unwrap_or(u16::MAX)
                        .min(area_width.get());
                    if x.saturating_add(width) > area.right() {
                        x = area.x;
                        y = y.saturating_add(1);
                    }
                    let option_area = Rect::new(x, y, width, 1);
                    let region = Button::new(option_label, button)
                        .variant(ButtonVariant::Toggle)
                        .style(radio_style())
                        .render_stateful(option_area, frame.buffer_mut());
                    self.run.hits.register(
                        region.area,
                        RunClickTarget::RadioOption {
                            field: index,
                            value: option_label.clone(),
                        },
                    );
                    x = x.saturating_add(width).saturating_add(1);
                }
                hits.push(HitRegion {
                    rect: field_area,
                    action: HitTarget::FocusField(index),
                });
                state.ensure_visible(1);
            }
        }
    }

    fn render_run_chips(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        chips: &[RunChip],
        hits: &mut Vec<HitRegion>,
    ) {
        for chip in chips {
            let width = chip.width.min(area.width.saturating_sub(chip.x));
            let Some(width) = std::num::NonZeroU16::new(width) else {
                continue;
            };
            let chip_area = Rect::new(area.x.saturating_add(chip.x), area.y, width.get(), 1);
            let state = ButtonState::enabled();
            let _region = Button::new(&chip.label, &state)
                .variant(ButtonVariant::SingleLine)
                .style(run_chip_style())
                .render_stateful(chip_area, frame.buffer_mut());
            hits.push(HitRegion {
                rect: chip_area,
                action: chip.target,
            });
        }
    }

    fn render_open_dropdowns(&mut self, frame: &mut Frame) {
        let screen = frame.area();
        for (index, control) in self.run.controls.iter().enumerate() {
            let WidgetControl::Choice {
                state,
                options,
                presentation: ChoicePresentation::Picker,
                ..
            } = control
            else {
                continue;
            };
            if state.is_open {
                let Some(anchor) = self.run.select_areas[index] else {
                    self.run.dropdown_regions[index].clear();
                    continue;
                };
                let regions = Select::new(options, state)
                    .style(select_style())
                    .render_dropdown(frame, anchor, screen);
                self.run.dropdown_regions[index] = regions;
            }
        }
    }

    fn handle_run_key(&mut self, key: KeyEvent, form: &RunFormView) -> EventHandling {
        let focused = form
            .focused()
            .min(self.run.controls.len().saturating_sub(1));
        if let Some(handling) = self.handle_open_select_key(focused, &key, form) {
            return handling;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Char('r'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return EventHandling::Action(Action::Submit);
            }
            (KeyCode::Char('s'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return EventHandling::Action(Action::OpenRunPresetSave);
            }
            (KeyCode::Char('t'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return EventHandling::Action(Action::OpenRunTokenMenu);
            }
            (KeyCode::Char('o'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return EventHandling::Action(Action::ResetFocusedRunField);
            }
            (KeyCode::Char('n'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return EventHandling::Action(Action::OpenRunRunnerEditor);
            }
            (KeyCode::Esc, _) => return EventHandling::Action(Action::Back),
            (KeyCode::Tab, _) => return self.move_focus(true),
            (KeyCode::BackTab, _) => return self.move_focus(false),
            (KeyCode::PageUp | KeyCode::PageDown, _) => {
                let _ = self.run.scroll.handle_key(&key, self.run.visible_height);
                return EventHandling::Consumed;
            }
            _ => {}
        }

        match &mut self.run.controls[focused] {
            WidgetControl::Input { state, .. } => {
                if key.code == KeyCode::Right
                    && key.modifiers.is_empty()
                    && state.cursor() == state.value().chars().count()
                    && let Some(suggestion) = self.path_suggestions.take(focused, state.value())
                {
                    *state = LineInput::new(suggestion.clone());
                    let request =
                        form.path_completion_request(focused, &suggestion, host_path_dialect());
                    self.path_suggestions.ensure(focused, request);
                    return EventHandling::Action(Action::SetFieldValue {
                        field: focused,
                        value: suggestion,
                    });
                }
                let before = state.value().to_owned();
                let response = state.handle_event(&Event::Key(key));
                if response.is_none() {
                    if key.code == KeyCode::Enter {
                        return EventHandling::Action(Action::Submit);
                    }
                    if key.code == KeyCode::Down {
                        return self.move_focus(true);
                    }
                    if key.code == KeyCode::Up {
                        return self.move_focus(false);
                    }
                    return EventHandling::Ignored;
                }
                if before == state.value() {
                    EventHandling::Consumed
                } else {
                    let value = state.value().to_owned();
                    let request =
                        form.path_completion_request(focused, &value, host_path_dialect());
                    self.path_suggestions.ensure(focused, request);
                    EventHandling::Action(Action::SetFieldValue {
                        field: focused,
                        value,
                    })
                }
            }
            WidgetControl::TextArea {
                state,
                undo_group,
                redo_group,
                ..
            } => {
                let before = textarea_text(state);
                let before_cursor = state.cursor();
                match edit_textarea(state, key, undo_group, redo_group) {
                    TextAreaEventHandling::Ignored => return EventHandling::Ignored,
                    TextAreaEventHandling::Consumed | TextAreaEventHandling::VerticalBoundary => {}
                }
                let after = textarea_text(state);
                if state.cursor() != before_cursor {
                    self.run.pending_ensure_focus = true;
                }
                if before == after {
                    EventHandling::Consumed
                } else {
                    EventHandling::Action(Action::SetFieldValue {
                        field: focused,
                        value: after,
                    })
                }
            }
            WidgetControl::Checkbox(state) => match key.code {
                KeyCode::Char(' ') => {
                    state.toggle();
                    EventHandling::Action(Action::ToggleField(focused))
                }
                KeyCode::Enter => EventHandling::Action(Action::Submit),
                KeyCode::Down => self.move_focus(true),
                KeyCode::Up => self.move_focus(false),
                _ => EventHandling::Ignored,
            },
            WidgetControl::Choice {
                state,
                options,
                presentation: ChoicePresentation::Radio,
                ..
            } => match key.code {
                KeyCode::Left => select_radio(focused, state, options, false),
                KeyCode::Right | KeyCode::Char(' ') => select_radio(focused, state, options, true),
                KeyCode::Enter => EventHandling::Action(Action::Submit),
                KeyCode::Down => self.move_focus(true),
                KeyCode::Up => self.move_focus(false),
                _ => EventHandling::Ignored,
            },
            WidgetControl::Choice {
                presentation: ChoicePresentation::Picker,
                ..
            } => EventHandling::Ignored,
        }
    }

    fn handle_open_select_key(
        &mut self,
        focused: usize,
        key: &KeyEvent,
        _form: &RunFormView,
    ) -> Option<EventHandling> {
        let WidgetControl::Choice {
            state,
            options,
            presentation: ChoicePresentation::Picker,
            ..
        } = self.run.controls.get_mut(focused)?
        else {
            return None;
        };
        let relevant = if state.is_open {
            matches!(
                key.code,
                KeyCode::Esc
                    | KeyCode::Enter
                    | KeyCode::Char(' ')
                    | KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::PageUp
                    | KeyCode::PageDown
            )
        } else {
            matches!(
                key.code,
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Down
            )
        };
        if !relevant {
            return None;
        }
        let action = handle_select_key(key, state);
        Some(match action {
            Some(SelectAction::Select(option)) => {
                EventHandling::Action(Action::SelectFieldOption {
                    field: focused,
                    value: options.get(option).cloned().unwrap_or_default(),
                })
            }
            Some(SelectAction::Focus | SelectAction::Open | SelectAction::Close) | None => {
                EventHandling::Consumed
            }
        })
    }

    fn handle_run_paste(&mut self, value: &str, form: &RunFormView) -> EventHandling {
        let focused = form
            .focused()
            .min(self.run.controls.len().saturating_sub(1));
        match &mut self.run.controls[focused] {
            WidgetControl::Input { state, .. } => {
                for character in value.chars() {
                    let _ = state.handle(InputRequest::InsertChar(character));
                }
                let value = state.value().to_owned();
                let request = form.path_completion_request(focused, &value, host_path_dialect());
                self.path_suggestions.ensure(focused, request);
                EventHandling::Action(Action::SetFieldValue {
                    field: focused,
                    value,
                })
            }
            WidgetControl::TextArea {
                state,
                undo_group,
                redo_group,
                ..
            } => {
                let selected = state.is_selecting();
                state.insert_str(value);
                *undo_group = 1 + usize::from(selected && !value.is_empty());
                *redo_group = 0;
                EventHandling::Action(Action::SetFieldValue {
                    field: focused,
                    value: textarea_text(state),
                })
            }
            WidgetControl::Checkbox(_) | WidgetControl::Choice { .. } => EventHandling::Ignored,
        }
    }

    fn insert_run_text(&mut self, field: usize, text: &str) -> EventHandling {
        let Some(control) = self.run.controls.get_mut(field) else {
            return EventHandling::Ignored;
        };
        let value = match control {
            WidgetControl::Input { state, .. } => {
                for character in text.chars() {
                    let _ = state.handle(InputRequest::InsertChar(character));
                }
                state.value().to_owned()
            }
            WidgetControl::TextArea {
                state,
                undo_group,
                redo_group,
                ..
            } => {
                state.insert_str(text);
                *undo_group = 1;
                *redo_group = 0;
                textarea_text(state)
            }
            WidgetControl::Checkbox(_) | WidgetControl::Choice { .. } => {
                return EventHandling::Ignored;
            }
        };
        EventHandling::Action(Action::SetRunFieldValueAndCloseModal { field, value })
    }

    fn handle_run_mouse(
        &mut self,
        mouse: MouseEvent,
        form: &RunFormView,
        _geometry: &ViewGeometry,
    ) -> EventHandling {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            for (index, control) in self.run.controls.iter_mut().enumerate() {
                let WidgetControl::Choice {
                    state,
                    presentation: ChoicePresentation::Picker,
                    ..
                } = control
                else {
                    continue;
                };
                if state.is_open
                    && self.run.dropdown_regions[index]
                        .iter()
                        .any(|region| region.contains(mouse.column, mouse.row))
                {
                    self.run.click.cancel();
                    if mouse.kind == MouseEventKind::ScrollUp {
                        state.highlight_prev();
                    } else {
                        state.highlight_next();
                    }
                    state.ensure_visible(self.run.dropdown_regions[index].len().max(1));
                    return EventHandling::Consumed;
                }
            }
        }
        if self
            .run
            .scroll
            .handle_mouse(&mouse, self.run.viewport, self.run.visible_height)
        {
            return EventHandling::Consumed;
        }
        if !matches!(
            mouse.kind,
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
        ) {
            return EventHandling::Ignored;
        }

        let dropdown_target =
            self.run
                .controls
                .iter()
                .enumerate()
                .find_map(|(index, control)| {
                    let WidgetControl::Choice {
                        state,
                        options,
                        presentation: ChoicePresentation::Picker,
                        ..
                    } = control
                    else {
                        return None;
                    };
                    if !state.is_open {
                        return None;
                    }
                    self.run.dropdown_regions[index]
                        .iter()
                        .rev()
                        .find(|region| region.contains(mouse.column, mouse.row))
                        .and_then(|region| match region.data {
                            SelectAction::Select(option) => {
                                options.get(option).cloned().map(|value| {
                                    RunClickTarget::SelectOption {
                                        field: index,
                                        value,
                                    }
                                })
                            }
                            SelectAction::Focus | SelectAction::Open | SelectAction::Close => None,
                        })
                });

        let target = dropdown_target
            .as_ref()
            .or_else(|| self.run.hits.topmost(mouse.column, mouse.row));
        match self.run.click.update(&mouse, target) {
            ClickOutcome::Armed => {
                if let Some(RunClickTarget::FocusField(index)) = target {
                    self.place_run_cursor(*index, mouse.column, mouse.row);
                }
                EventHandling::Consumed
            }
            ClickOutcome::Ignored => EventHandling::Ignored,
            ClickOutcome::Activated(target) => match target {
                RunClickTarget::FocusField(index) => {
                    self.place_run_cursor(index, mouse.column, mouse.row);
                    if form.focused() == index {
                        EventHandling::Consumed
                    } else {
                        EventHandling::Action(Action::FocusField(index))
                    }
                }
                RunClickTarget::Checkbox(index) => {
                    EventHandling::Action(Action::ToggleField(index))
                }
                RunClickTarget::Select(index) => {
                    if let Some(WidgetControl::Choice { state, .. }) =
                        self.run.controls.get_mut(index)
                    {
                        state.open();
                    }
                    if form.focused() == index {
                        EventHandling::Consumed
                    } else {
                        EventHandling::Action(Action::FocusField(index))
                    }
                }
                RunClickTarget::SelectOption { field, value } => {
                    EventHandling::Action(Action::SelectFieldOption { field, value })
                }
                RunClickTarget::RadioOption { field, value } => {
                    EventHandling::Action(Action::SelectFieldOption { field, value })
                }
            },
        }
    }

    fn place_run_cursor(&mut self, index: usize, column: u16, row: u16) {
        if let Some((editable, WidgetControl::Input { state, .. })) = self
            .run
            .editables
            .get(index)
            .copied()
            .flatten()
            .zip(self.run.controls.get_mut(index))
        {
            let _ = editable.place_cursor(state, column, row);
        }
        if let Some((editable, WidgetControl::TextArea { state, .. })) = self
            .run
            .textarea_editables
            .get(index)
            .copied()
            .flatten()
            .zip(self.run.controls.get_mut(index))
        {
            let _ = editable.place_cursor(state, column, row);
        }
    }

    fn handle_form_key(&mut self, key: KeyEvent, form: &FormView) -> EventHandling {
        let focused = form.focused.min(self.form.controls.len().saturating_sub(1));
        match (key.code, key.modifiers) {
            (KeyCode::Char('s'), modifiers) if modifiers.contains(KeyModifiers::CONTROL) => {
                return EventHandling::Action(Action::Submit);
            }
            (KeyCode::Esc, _) => return EventHandling::Action(Action::Back),
            (KeyCode::Tab, _) => return self.move_form_focus(true),
            (KeyCode::BackTab, _) => return self.move_form_focus(false),
            _ => {}
        }

        match &mut self.form.controls[focused] {
            FormWidgetControl::Input { state, .. } => {
                let before = state.value().to_owned();
                let response = state.handle_event(&Event::Key(key));
                if response.is_none() {
                    return match key.code {
                        KeyCode::Enter | KeyCode::Down => self.move_form_focus(true),
                        KeyCode::Up => self.move_form_focus(false),
                        KeyCode::PageUp | KeyCode::PageDown => {
                            let _ = self.form.scroll.handle_key(&key, self.form.visible_height);
                            EventHandling::Consumed
                        }
                        _ => EventHandling::Ignored,
                    };
                }
                if before == state.value() {
                    EventHandling::Consumed
                } else {
                    EventHandling::Action(Action::SetFieldValue {
                        field: focused,
                        value: state.value().to_owned(),
                    })
                }
            }
            FormWidgetControl::TextArea {
                state,
                undo_group,
                redo_group,
                ..
            } => {
                let before = textarea_text(state);
                let before_cursor = state.cursor();
                match edit_textarea(state, key, undo_group, redo_group) {
                    TextAreaEventHandling::Ignored => return EventHandling::Ignored,
                    TextAreaEventHandling::Consumed | TextAreaEventHandling::VerticalBoundary => {}
                }
                let after = textarea_text(state);
                if state.cursor() != before_cursor {
                    self.form.pending_ensure_focus = true;
                }
                if before == after {
                    EventHandling::Consumed
                } else {
                    EventHandling::Action(Action::SetFieldValue {
                        field: focused,
                        value: after,
                    })
                }
            }
        }
    }

    fn handle_form_paste(&mut self, value: &str, form: &FormView) -> EventHandling {
        let focused = form.focused.min(self.form.controls.len().saturating_sub(1));
        match &mut self.form.controls[focused] {
            FormWidgetControl::Input { state, .. } => {
                for character in value.chars() {
                    let _ = state.handle(InputRequest::InsertChar(character));
                }
                EventHandling::Action(Action::SetFieldValue {
                    field: focused,
                    value: state.value().to_owned(),
                })
            }
            FormWidgetControl::TextArea {
                state,
                undo_group,
                redo_group,
                ..
            } => {
                let selected = state.is_selecting();
                let _ = state.insert_str(value);
                *undo_group = 1 + usize::from(selected && !value.is_empty());
                *redo_group = 0;
                EventHandling::Action(Action::SetFieldValue {
                    field: focused,
                    value: textarea_text(state),
                })
            }
        }
    }

    fn handle_form_mouse(
        &mut self,
        mouse: MouseEvent,
        form: &FormView,
        geometry: &ViewGeometry,
    ) -> EventHandling {
        if self
            .form
            .scroll
            .handle_mouse(&mouse, self.form.viewport, self.form.visible_height)
        {
            return EventHandling::Consumed;
        }
        let target = self.form.clicks.topmost(mouse.column, mouse.row).copied();
        match self.form.click.update(&mouse, target.as_ref()) {
            ClickOutcome::Armed => {
                let index = target.expect("an armed form click has one target");
                self.place_form_cursor(index, mouse.column, mouse.row);
                EventHandling::Consumed
            }
            ClickOutcome::Activated(index) => {
                if form.focused == index {
                    EventHandling::Consumed
                } else {
                    EventHandling::Action(Action::FocusField(index))
                }
            }
            ClickOutcome::Ignored => {
                let _ = geometry;
                EventHandling::Ignored
            }
        }
    }

    fn place_form_cursor(&mut self, index: usize, column: u16, row: u16) {
        if let Some((editable, FormWidgetControl::Input { state, .. })) = self
            .form
            .editables
            .get(index)
            .copied()
            .flatten()
            .zip(self.form.controls.get_mut(index))
        {
            let _ = editable.place_cursor(state, column, row);
        }
        if let Some((editable, FormWidgetControl::TextArea { state, .. })) = self
            .form
            .textarea_editables
            .get(index)
            .copied()
            .flatten()
            .zip(self.form.controls.get_mut(index))
        {
            let _ = editable.place_cursor(state, column, row);
        }
    }

    fn move_focus(&mut self, forward: bool) -> EventHandling {
        self.path_suggestions.clear();
        if forward {
            self.run.focus.next();
        } else {
            self.run.focus.prev();
        }
        // The session moves its own cursor here, so the next sync sees no change and would never
        // scroll. Without this the keyboard can focus a field that stays off screen.
        self.run.pending_ensure_focus = true;
        self.run
            .focus
            .current()
            .copied()
            .map_or(EventHandling::Consumed, |index| {
                EventHandling::Action(Action::FocusField(index))
            })
    }

    fn move_form_focus(&mut self, forward: bool) -> EventHandling {
        if forward {
            self.form.focus.next();
        } else {
            self.form.focus.prev();
        }
        self.form.pending_ensure_focus = true;
        self.form
            .focus
            .current()
            .copied()
            .map_or(EventHandling::Consumed, |index| {
                EventHandling::Action(Action::FocusField(index))
            })
    }
}

fn is_ctrl_c(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers,
            kind,
            ..
        }) if *kind != KeyEventKind::Release && modifiers.contains(KeyModifiers::CONTROL)
    )
}

impl RunWidgetSession {
    fn input_value(&self, index: usize) -> Option<&str> {
        match self.controls.get(index)? {
            WidgetControl::Input { state, .. } => Some(state.value()),
            WidgetControl::TextArea { .. }
            | WidgetControl::Checkbox(_)
            | WidgetControl::Choice { .. } => None,
        }
    }

    fn sync(&mut self, form: &RunFormView) {
        let signature = RunSignature {
            selector: form.selector().to_owned(),
            fields: form.fields().iter().map(field_signature).collect(),
        };
        if self.signature.as_ref() != Some(&signature) {
            self.controls = form.fields().iter().map(widget_control).collect();
            self.textarea_viewports = vec![TextAreaViewport::default(); self.controls.len()];
            self.focus.clear();
            self.focus.register_all(0..self.controls.len());
            self.scroll = VirtualScrollState::default();
            self.signature = Some(signature);
            self.pending_ensure_focus = true;
        } else {
            for (control, field) in self.controls.iter_mut().zip(form.fields()) {
                control.sync_value(&field.control);
            }
        }
        if self.focus.current() != Some(&form.focused()) {
            self.focus.set(form.focused());
            self.pending_ensure_focus = true;
        }
        for (index, control) in self.controls.iter_mut().enumerate() {
            control.set_focused(self.focus.is_focused(&index));
        }
    }

    fn prepare_layout(&mut self, layout: &RunLayout, focused: usize, viewport: Rect) {
        self.viewport = viewport;
        self.visible_height = usize::from(viewport.height);
        self.row_starts.clone_from(&layout.control_starts);
        self.row_heights.clone_from(&layout.control_heights);
        self.scroll.set_line_count(layout.height);
        let maximum = layout.height.saturating_sub(self.visible_height);
        self.scroll
            .set_scroll_offset(self.scroll.scroll_offset().min(maximum));
        let target = self
            .row_starts
            .get(focused)
            .copied()
            .zip(self.row_heights.get(focused).copied());
        let reflow = target.map_or((layout.height, 0, 0), |(start, height)| {
            (layout.height, start, height)
        });
        let alignment_changed =
            AlignmentSignature::update(&mut self.alignment, focused, viewport, reflow);
        if (self.pending_ensure_focus || alignment_changed)
            && let Some((start, height)) = target
        {
            let offset = self.scroll.scroll_offset();
            let end = start.saturating_add(height);
            if height.cmp(&self.visible_height) == std::cmp::Ordering::Greater
                && let Some(WidgetControl::TextArea { state, .. }) = self.controls.get(focused)
            {
                let last_content_row = height.saturating_sub(3);
                let cursor =
                    editor_cursor_virtual_row(start, state.cursor().0.min(last_content_row));
                let next = start.max(cursor.saturating_add(1).saturating_sub(self.visible_height));
                self.scroll.set_scroll_offset(next);
            } else {
                match start.cmp(&offset) {
                    std::cmp::Ordering::Less => self.scroll.set_scroll_offset(start),
                    std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => self
                        .scroll
                        .set_scroll_offset(offset.max(end.saturating_sub(self.visible_height))),
                }
            }
            self.pending_ensure_focus = false;
        }
    }

    fn visible_rect(&self, start: usize, height: usize) -> Option<Rect> {
        let offset = self.scroll.scroll_offset();
        let viewport_end = offset.saturating_add(self.visible_height);
        let end = start.saturating_add(height);
        if end <= offset || start >= viewport_end {
            return None;
        }
        let clipped_start = start.max(offset);
        let clipped_end = end.min(viewport_end);
        Some(Rect::new(
            self.viewport.x,
            self.viewport.y.saturating_add(
                u16::try_from(clipped_start.saturating_sub(offset)).unwrap_or(u16::MAX),
            ),
            self.viewport.width,
            u16::try_from(clipped_end.saturating_sub(clipped_start)).unwrap_or(u16::MAX),
        ))
    }
}

const fn path_dialect_for(windows: bool) -> PathInputDialect {
    if windows {
        PathInputDialect::Windows
    } else {
        PathInputDialect::Posix
    }
}

const fn host_path_dialect() -> PathInputDialect {
    path_dialect_for(cfg!(windows))
}

impl SearchWidgetSession {
    fn sync(&mut self, value: &str) {
        if self.input.value() != value {
            self.input = LineInput::new(value.to_owned());
        }
    }
}

impl FormWidgetSession {
    fn sync(&mut self, form: &FormView) {
        let signature = form.fields.iter().map(form_field_signature).collect();
        if self.signature.as_ref() != Some(&signature) {
            self.controls = form.fields.iter().map(form_widget_control).collect();
            self.textarea_viewports = vec![TextAreaViewport::default(); self.controls.len()];
            self.focus.clear();
            self.focus.register_all(0..self.controls.len());
            self.scroll = VirtualScrollState::default();
            self.signature = Some(signature);
            self.pending_ensure_focus = true;
        } else {
            for (control, field) in self.controls.iter_mut().zip(&form.fields) {
                control.sync_value(field);
            }
        }
        if self.focus.current() != Some(&form.focused) {
            self.focus.set(form.focused);
            self.pending_ensure_focus = true;
        }
        for (index, control) in self.controls.iter_mut().enumerate() {
            control.set_focused(self.focus.is_focused(&index));
        }
    }

    fn prepare_layout(&mut self, form: &FormView, viewport: Rect) {
        self.viewport = viewport;
        self.visible_height = usize::from(viewport.height);
        self.row_starts.clear();
        self.row_heights.clear();
        let mut next = 0_usize;
        for field in &form.fields {
            self.row_starts.push(next);
            let height = form_control_height(field);
            self.row_heights.push(height);
            next = next.saturating_add(height);
        }
        self.scroll.set_line_count(next);
        let maximum = next.saturating_sub(self.visible_height);
        self.scroll
            .set_scroll_offset(self.scroll.scroll_offset().min(maximum));
        let target = self
            .row_starts
            .get(form.focused)
            .copied()
            .zip(self.row_heights.get(form.focused).copied());
        let reflow = target.map_or((next, 0, 0), |(start, height)| (next, start, height));
        let alignment_changed =
            AlignmentSignature::update(&mut self.alignment, form.focused, viewport, reflow);
        if (self.pending_ensure_focus || alignment_changed)
            && let Some((start, height)) = target
        {
            let offset = self.scroll.scroll_offset();
            let end = start.saturating_add(height);
            if height.cmp(&self.visible_height) == std::cmp::Ordering::Greater
                && let FormWidgetControl::TextArea { state, .. } = &self.controls[form.focused]
            {
                let cursor = editor_cursor_virtual_row(start, state.cursor().0);
                let next = start.max(cursor.saturating_add(1).saturating_sub(self.visible_height));
                self.scroll.set_scroll_offset(next);
            } else {
                match start.cmp(&offset) {
                    std::cmp::Ordering::Less => self.scroll.set_scroll_offset(start),
                    std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => self
                        .scroll
                        .set_scroll_offset(offset.max(end.saturating_sub(self.visible_height))),
                }
            }
            self.pending_ensure_focus = false;
        }
    }

    fn visible_band(&self, index: usize) -> Option<RowClip> {
        let start = *self.row_starts.get(index)?;
        let height = *self.row_heights.get(index)?;
        let offset = self.scroll.scroll_offset();
        let viewport_end = offset.saturating_add(self.visible_height);
        let end = start.saturating_add(height);
        if end <= offset || start >= viewport_end {
            return None;
        }
        let clipped_start = start.max(offset);
        let clipped_end = end.min(viewport_end);
        Some(RowClip::new(
            height,
            clipped_start.saturating_sub(start),
            Rect::new(
                self.viewport.x,
                self.viewport.y.saturating_add(
                    u16::try_from(clipped_start.saturating_sub(offset))
                        .expect("the form band starts inside its viewport"),
                ),
                self.viewport.width,
                u16::try_from(clipped_end.saturating_sub(clipped_start))
                    .expect("the form band height fits its viewport"),
            ),
        ))
    }
}

impl FormWidgetControl {
    fn sync_value(&mut self, field: &FormField) {
        match self {
            Self::Input { state, .. } if state.value() != field.value => {
                *state = LineInput::new(field.value.clone());
            }
            Self::TextArea { state, .. } if textarea_text(state) != field.value => {
                **state = new_textarea(&field.value);
            }
            Self::Input { .. } | Self::TextArea { .. } => {}
        }
    }

    fn set_focused(&mut self, focused: bool) {
        match self {
            Self::Input {
                focused: is_focused,
                ..
            }
            | Self::TextArea {
                focused: is_focused,
                ..
            } => *is_focused = focused,
        }
    }
}

impl WidgetControl {
    fn sync_value(&mut self, model: &FormControl) {
        match (self, model) {
            (Self::Input { state, .. }, FormControl::Text(text)) if state.value() != text.value => {
                *state = LineInput::new(text.value.clone());
            }
            (Self::TextArea { state, .. }, FormControl::Text(text))
                if textarea_text(state) != text.value =>
            {
                **state = new_textarea(&text.value);
            }
            (Self::Checkbox(state), FormControl::Checkbox { checked }) => {
                state.set_checked(*checked);
            }
            (
                Self::Choice {
                    state,
                    options,
                    buttons,
                    ..
                },
                FormControl::Choice(choice),
            ) => {
                let selected = choice
                    .options
                    .iter()
                    .position(|value| value == &choice.selected);
                if state.selected_index != selected {
                    state.selected_index = selected;
                    state.highlighted_index = selected.unwrap_or_default();
                }
                for (index, button) in buttons.iter_mut().enumerate() {
                    button.toggled = selected == Some(index);
                }
                *options = choice.options.clone();
                state.set_total(options.len());
            }
            _ => {}
        }
    }

    fn set_focused(&mut self, focused: bool) {
        match self {
            Self::Input {
                focused: is_focused,
                ..
            }
            | Self::TextArea {
                focused: is_focused,
                ..
            } => *is_focused = focused,
            Self::Checkbox(state) => state.set_focused(focused),
            Self::Choice { state, buttons, .. } => {
                state.focused = focused;
                let active = state.selected_index.unwrap_or(state.highlighted_index);
                for (index, button) in buttons.iter_mut().enumerate() {
                    button.set_focused(focused && index == active);
                }
                if !focused {
                    state.close();
                }
            }
        }
    }
}

fn field_signature(field: &RunField) -> FieldSignature {
    let shape = match &field.control {
        FormControl::Text(control) => ControlShape::Input {
            secret: control.secret,
            multiline: control.multiline,
        },
        FormControl::Checkbox { .. } => ControlShape::Checkbox,
        FormControl::Choice(control) => ControlShape::Choice {
            options: control.options.clone(),
            presentation: control.presentation,
        },
    };
    FieldSignature {
        key: field.key.clone(),
        shape,
    }
}

fn form_field_signature(field: &FormField) -> FieldSignature {
    FieldSignature {
        key: field.key.clone(),
        shape: ControlShape::Input {
            secret: field.secret,
            multiline: field.multiline,
        },
    }
}

fn form_widget_control(field: &FormField) -> FormWidgetControl {
    if field.multiline && !field.secret {
        FormWidgetControl::TextArea {
            state: Box::new(new_textarea(&field.value)),
            focused: false,
            undo_group: 0,
            redo_group: 0,
        }
    } else {
        FormWidgetControl::Input {
            state: LineInput::new(field.value.clone()),
            secret: field.secret,
            focused: false,
        }
    }
}

fn form_control_height(field: &FormField) -> usize {
    if field.multiline && !field.secret {
        textarea_control_height(&field.value, 4)
    } else {
        3
    }
}

fn widget_control(field: &RunField) -> WidgetControl {
    match &field.control {
        FormControl::Text(control) if control.multiline && !control.secret => {
            WidgetControl::TextArea {
                state: Box::new(new_textarea(&control.value)),
                focused: false,
                undo_group: 0,
                redo_group: 0,
            }
        }
        FormControl::Text(control) => WidgetControl::Input {
            state: LineInput::new(control.value.clone()),
            secret: control.secret,
            focused: false,
        },
        FormControl::Checkbox { checked } => WidgetControl::Checkbox(CheckBoxState::new(*checked)),
        FormControl::Choice(control) => {
            let selected = control
                .options
                .iter()
                .position(|value| value == &control.selected);
            let mut state = selected.map_or_else(
                || SelectState::new(control.options.len()),
                |index| SelectState::with_selected(control.options.len(), index),
            );
            state.highlighted_index = selected.unwrap_or_default();
            WidgetControl::Choice {
                state,
                options: control.options.clone(),
                presentation: control.presentation,
                buttons: (0..control.options.len())
                    .map(|index| ButtonState::toggled(selected == Some(index)))
                    .collect(),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TextAreaEventHandling {
    Ignored,
    Consumed,
    VerticalBoundary,
}

pub(crate) fn edit_textarea(
    state: &mut RichTextArea<'static>,
    key: KeyEvent,
    undo_group: &mut usize,
    redo_group: &mut usize,
) -> TextAreaEventHandling {
    if key.code == KeyCode::Char('z') && key.modifiers == KeyModifiers::CONTROL {
        let count = (*undo_group).max(1);
        for _ in 0..count {
            let _ = state.undo();
        }
        *redo_group = count;
        *undo_group = 0;
        return TextAreaEventHandling::Consumed;
    }
    if (key.code == KeyCode::Char('z')
        && key
            .modifiers
            .contains(KeyModifiers::CONTROL.union(KeyModifiers::SHIFT)))
        || (key.code == KeyCode::Char('y') && key.modifiers == KeyModifiers::CONTROL)
    {
        let count = (*redo_group).max(1);
        for _ in 0..count {
            let _ = state.redo();
        }
        *undo_group = count;
        *redo_group = 0;
        return TextAreaEventHandling::Consumed;
    }
    let before = textarea_text(state);
    let selected = state.is_selecting();
    let cursor = state.cursor();
    let _ = state.input(key);
    if textarea_text(state) != before {
        let inserts_after_delete =
            selected && matches!(key.code, KeyCode::Char(_) | KeyCode::Enter | KeyCode::Tab);
        *undo_group = 1 + usize::from(inserts_after_delete);
        *redo_group = 0;
    }
    if !textarea_accepts(key) {
        TextAreaEventHandling::Ignored
    } else if matches!(key.code, KeyCode::Up | KeyCode::Down)
        && key.modifiers.is_empty()
        && !selected
        && state.cursor() == cursor
    {
        TextAreaEventHandling::VerticalBoundary
    } else {
        TextAreaEventHandling::Consumed
    }
}

fn textarea_accepts(key: KeyEvent) -> bool {
    matches!(
        key.code,
        KeyCode::Char(_)
            | KeyCode::Enter
            | KeyCode::Tab
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
    )
}

fn select_radio(
    field: usize,
    state: &mut SelectState,
    options: &[String],
    forward: bool,
) -> EventHandling {
    if options.is_empty() {
        return EventHandling::Consumed;
    }
    let current = state.selected_index.unwrap_or_default();
    let next = if forward {
        (current + 1).min(options.len() - 1)
    } else {
        current.saturating_sub(1)
    };
    state.select(next);
    EventHandling::Action(Action::SelectFieldOption {
        field,
        value: options[next].clone(),
    })
}

#[cfg(test)]
mod editor_contract_tests {
    use super::*;

    fn edit(
        state: &mut RichTextArea<'static>,
        undo_group: &mut usize,
        redo_group: &mut usize,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> TextAreaEventHandling {
        edit_textarea(
            state,
            KeyEvent::new(code, modifiers),
            undo_group,
            redo_group,
        )
    }

    fn undone_edit() -> (RichTextArea<'static>, usize, usize) {
        let mut state = new_textarea("ab");
        let (mut undo_group, mut redo_group) = (0, 0);
        assert_eq!(
            edit(
                &mut state,
                &mut undo_group,
                &mut redo_group,
                KeyCode::Char('c'),
                KeyModifiers::NONE,
            ),
            TextAreaEventHandling::Consumed
        );
        assert_eq!(textarea_text(&state), "abc");
        assert_eq!(
            edit(
                &mut state,
                &mut undo_group,
                &mut redo_group,
                KeyCode::Char('z'),
                KeyModifiers::CONTROL,
            ),
            TextAreaEventHandling::Consumed
        );
        assert_eq!(textarea_text(&state), "ab");
        (state, undo_group, redo_group)
    }

    #[test]
    fn textarea_undo_and_redo_require_their_exact_key_chords() {
        for (code, modifiers) in [
            (
                KeyCode::Char('z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            (
                KeyCode::Char('z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT,
            ),
            (KeyCode::Char('y'), KeyModifiers::CONTROL),
        ] {
            let (mut state, mut undo_group, mut redo_group) = undone_edit();
            assert_eq!(
                edit(
                    &mut state,
                    &mut undo_group,
                    &mut redo_group,
                    code,
                    modifiers,
                ),
                TextAreaEventHandling::Consumed
            );
            assert_eq!(textarea_text(&state), "abc", "redo chord {code:?}");
        }

        for (code, modifiers) in [
            (KeyCode::Char('q'), KeyModifiers::CONTROL),
            (
                KeyCode::Char('q'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            (
                KeyCode::Char('y'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
        ] {
            let (mut state, mut undo_group, mut redo_group) = undone_edit();
            let _ = edit(
                &mut state,
                &mut undo_group,
                &mut redo_group,
                code,
                modifiers,
            );
            assert_eq!(textarea_text(&state), "ab", "false redo chord {code:?}");
        }

        for (code, modifiers, expected) in [
            (KeyCode::Char('z'), KeyModifiers::NONE, "abz"),
            (KeyCode::Char('z'), KeyModifiers::SHIFT, "abz"),
            (KeyCode::Char('y'), KeyModifiers::NONE, "aby"),
        ] {
            let (mut state, mut undo_group, mut redo_group) = undone_edit();
            let _ = edit(
                &mut state,
                &mut undo_group,
                &mut redo_group,
                code,
                modifiers,
            );
            assert_eq!(textarea_text(&state), expected, "plain character {code:?}");
        }
    }

    #[test]
    fn textarea_undo_groups_selection_replacement_and_deletion_exactly() {
        let mut replaced = new_textarea("ab");
        let (mut undo_group, mut redo_group) = (0, 0);
        for (code, modifiers) in [
            (KeyCode::End, KeyModifiers::NONE),
            (KeyCode::Left, KeyModifiers::SHIFT),
            (KeyCode::Char('界'), KeyModifiers::NONE),
            (KeyCode::Left, KeyModifiers::NONE),
        ] {
            let _ = edit(
                &mut replaced,
                &mut undo_group,
                &mut redo_group,
                code,
                modifiers,
            );
        }
        assert_eq!(textarea_text(&replaced), "a界");
        let _ = edit(
            &mut replaced,
            &mut undo_group,
            &mut redo_group,
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(textarea_text(&replaced), "ab");

        let mut deleted = new_textarea("ab");
        let (mut undo_group, mut redo_group) = (0, 0);
        for (code, modifiers) in [
            (KeyCode::End, KeyModifiers::NONE),
            (KeyCode::Char('Q'), KeyModifiers::NONE),
            (KeyCode::Left, KeyModifiers::SHIFT),
            (KeyCode::Backspace, KeyModifiers::NONE),
        ] {
            let _ = edit(
                &mut deleted,
                &mut undo_group,
                &mut redo_group,
                code,
                modifiers,
            );
        }
        assert_eq!(textarea_text(&deleted), "ab");
        let _ = edit(
            &mut deleted,
            &mut undo_group,
            &mut redo_group,
            KeyCode::Char('z'),
            KeyModifiers::CONTROL,
        );
        assert_eq!(textarea_text(&deleted), "abQ");
    }

    #[test]
    fn textarea_reports_only_unmodified_vertical_edges_as_boundaries() {
        for (code, modifiers, expected) in [
            (
                KeyCode::F(2),
                KeyModifiers::NONE,
                TextAreaEventHandling::Ignored,
            ),
            (
                KeyCode::Up,
                KeyModifiers::NONE,
                TextAreaEventHandling::VerticalBoundary,
            ),
            (
                KeyCode::Up,
                KeyModifiers::SHIFT,
                TextAreaEventHandling::Consumed,
            ),
            (
                KeyCode::Char('x'),
                KeyModifiers::NONE,
                TextAreaEventHandling::Consumed,
            ),
        ] {
            let mut state = new_textarea("ab");
            let (mut undo_group, mut redo_group) = (0, 0);
            assert_eq!(
                edit(
                    &mut state,
                    &mut undo_group,
                    &mut redo_group,
                    code,
                    modifiers,
                ),
                expected,
                "event classification for {code:?}"
            );
        }

        let mut selected = new_textarea("a\nb");
        selected.start_selection();
        let (mut undo_group, mut redo_group) = (0, 0);
        assert_eq!(
            edit(
                &mut selected,
                &mut undo_group,
                &mut redo_group,
                KeyCode::Up,
                KeyModifiers::NONE,
            ),
            TextAreaEventHandling::Consumed
        );
    }

    #[test]
    fn radio_navigation_stops_at_both_ends_and_moves_one_option() {
        let options = ["zero".to_owned(), "one".to_owned(), "two".to_owned()];
        for (selected, forward, expected) in
            [(1, true, 2), (2, true, 2), (1, false, 0), (0, false, 0)]
        {
            let mut state = SelectState::with_selected(options.len(), selected);
            assert_eq!(
                select_radio(4, &mut state, &options, forward),
                EventHandling::Action(Action::SelectFieldOption {
                    field: 4,
                    value: options[expected].clone(),
                })
            );
            assert_eq!(state.selected_index, Some(expected));
        }
    }
}

fn run_layout(form: &RunFormView, locale: Locale, width: u16) -> RunLayout {
    let mut items = Vec::new();
    let mut start = 0_usize;
    for line in &form.drift_lines {
        push_run_copy(
            &mut items,
            &mut start,
            run_copy(line.clone(), Style::default().fg(Color::Yellow)),
            width,
        );
    }
    if let Some(notice) = form.degradation_notice() {
        let key = match notice {
            RunDegradationNotice::Subcommands => {
                "This script has subcommands skit can't model — type everything into the extra-arguments field."
            }
            RunDegradationNotice::DynamicArguments => {
                "skit couldn't read this script's argument declarations — type everything into the extra-arguments field."
            }
        };
        push_run_copy(
            &mut items,
            &mut start,
            run_copy(
                text(locale, key).into_owned(),
                Style::default().fg(Color::Yellow),
            ),
            width,
        );
    }
    if form.has_parameters() && form.preset_names().next().is_none() {
        // Version 0.4 keeps the row labelled even when it is empty: `Preset:` then the hint, on
        // one line (`src/skit/tui_form.py:741-757`). Without the label the sentence floats free
        // and never says which control it is about.
        push_run_copy(
            &mut items,
            &mut start,
            RunCopy {
                line: Line::from(vec![
                    Span::styled(
                        format!("{} ", text(locale, "Preset:")),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        text(
                            locale,
                            "none yet — fill the form and press Ctrl+S to save one",
                        )
                        .into_owned(),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]),
            },
            width,
        );
    }

    let mut control_starts = Vec::with_capacity(form.fields().len());
    let mut control_heights = Vec::with_capacity(form.fields().len());
    for (index, field) in form.fields().iter().enumerate() {
        let label = run_field_label(field, locale);
        let label_width = u16::try_from(label.width()).unwrap_or(u16::MAX);
        // The chips continue the label's own row, two columns after it, exactly as version 0.4
        // joins the pieces with two spaces (`src/skit/tui_form.py:215`).
        let chip_start = label_width.saturating_add(2);
        let mut chip_rows =
            run_chip_rows(form, index, field, locale, width, chip_start).into_iter();
        push_run_item(
            &mut items,
            &mut start,
            RunRenderItem::Chips {
                label: Some(RunCopy { line: label }),
                chips: chip_rows.next().unwrap_or_default(),
            },
            1,
        );
        for chips in chip_rows {
            push_run_item(
                &mut items,
                &mut start,
                RunRenderItem::Chips { label: None, chips },
                1,
            );
        }
        let control_start = start;
        let control_height = run_control_height(field, width);
        push_run_item(
            &mut items,
            &mut start,
            RunRenderItem::Control(index),
            control_height,
        );
        control_starts.push(control_start);
        control_heights.push(control_height);
        for note in run_field_notes(field, locale) {
            push_run_copy(&mut items, &mut start, note, width);
        }
        if index + 1 < form.fields().len() {
            push_run_item(&mut items, &mut start, RunRenderItem::Spacer, 1);
        }
    }
    RunLayout {
        items,
        control_starts,
        control_heights,
        height: start,
    }
}

fn push_run_copy(items: &mut Vec<PositionedRunItem>, start: &mut usize, copy: RunCopy, width: u16) {
    let height = Paragraph::new(copy.line.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
        .max(1);
    push_run_item(items, start, RunRenderItem::Copy(copy), height);
}

fn push_run_item(
    items: &mut Vec<PositionedRunItem>,
    start: &mut usize,
    item: RunRenderItem,
    height: usize,
) {
    items.push(PositionedRunItem {
        start: *start,
        height,
        item,
    });
    *start = start.saturating_add(height);
}

fn run_copy(value: String, style: Style) -> RunCopy {
    RunCopy {
        line: Line::from(Span::styled(value, style)),
    }
}

fn run_field_label(field: &RunField, locale: Locale) -> Line<'static> {
    let mut spans = vec![Span::styled(
        run_field_display_label(field, locale),
        Style::default().fg(Color::White),
    )];
    if field.required {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            text(locale, "required").into_owned(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(type_label) = run_type_label(field.parameter_type, locale) {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            type_label,
            Style::default().fg(Color::DarkGray),
        ));
    }
    if field.secret() {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("🔒 {}", text(locale, "never saved to disk")),
            Style::default().fg(Color::DarkGray),
        ));
    }
    Line::from(spans)
}

fn run_field_display_label(field: &RunField, locale: Locale) -> String {
    match field.role {
        RunFieldRole::Runner => text(locale, "Runner").into_owned(),
        RunFieldRole::Preset => text(locale, "Preset").into_owned(),
        RunFieldRole::ExtraArguments => text(locale, &field.label).into_owned(),
        RunFieldRole::Parameter { .. } => field.label.clone(),
    }
}

fn run_type_label(parameter_type: ParameterType, locale: Locale) -> Option<String> {
    let key = match parameter_type {
        ParameterType::Int => "whole number",
        ParameterType::Float => "number",
        ParameterType::Bool => "on/off",
        ParameterType::Path => "path",
        ParameterType::Str | ParameterType::Choice => return None,
    };
    Some(text(locale, key).into_owned())
}

fn run_field_notes(field: &RunField, locale: Locale) -> Vec<RunCopy> {
    let mut notes = Vec::new();
    if !field.help.is_empty() {
        notes.push(run_copy(
            field.help.clone(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if field.degraded {
        notes.push(run_copy(
            text(locale, "Leave empty to use the script's own default.").into_owned(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if field.input_binding {
        notes.push(run_copy(
            text(
                locale,
                "Leave empty and the script will ask you in the terminal.",
            )
            .into_owned(),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if let Some(environment) = field.environment_source() {
        notes.push(run_copy(
            format_text(
                locale,
                "Leave empty to read it from the environment variable {}.",
                &[&environment],
            ),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if let Some(expanded) = &field.feedback.expanded {
        notes.push(run_copy(
            format!("→ {expanded}"),
            Style::default().fg(Color::DarkGray),
        ));
    }
    if let Some(error) = &field.feedback.token_error {
        let message = match error {
            RunTokenError::MissingEnvironment { name, token } => format_text(
                locale,
                "The environment variable {} isn't set (needed by {}).",
                &[name, token],
            ),
        };
        notes.push(run_copy(
            format!("→ {message}"),
            Style::default().fg(Color::Yellow),
        ));
    }
    if let Some(count) = field.feedback.glob_count {
        let (message, color) = if count == 0 {
            (
                text(locale, "⚠ matches no files yet").into_owned(),
                Color::Yellow,
            )
        } else {
            (
                format_text(locale, "✓ matches {} file(s)", &[&count]),
                Color::Green,
            )
        };
        notes.push(run_copy(message, Style::default().fg(color)));
    }
    if let Some(error) = field.validation_error {
        notes.push(run_copy(
            run_validation_message(field, error, locale),
            Style::default().fg(Color::Red),
        ));
    }
    notes
}

fn run_validation_message(field: &RunField, error: RunValidationError, locale: Locale) -> String {
    let label = run_field_display_label(field, locale);
    match error {
        RunValidationError::Required => format_text(locale, "{} is required.", &[&label]),
        RunValidationError::InvalidType => {
            let kind = match field.parameter_type {
                ParameterType::Int => text(locale, "a whole number"),
                ParameterType::Float => text(locale, "a number"),
                ParameterType::Bool => text(locale, "on or off"),
                ParameterType::Str | ParameterType::Choice | ParameterType::Path => {
                    text(locale, "text")
                }
            };
            let value = quoted_value(&field.control.value());
            format_text(
                locale,
                "{} needs {} — you typed {}.",
                &[&label, &kind, &value],
            )
        }
        RunValidationError::InvalidChoice => {
            let choices = match &field.control {
                FormControl::Choice(control) => control.options.join(", "),
                FormControl::Text(_) | FormControl::Checkbox { .. } => String::new(),
            };
            format_text(locale, "{} must be one of: {}", &[&label, &choices])
        }
    }
}

fn quoted_value(value: &str) -> String {
    format!(
        "'{}'",
        value
            .chars()
            .flat_map(char::escape_default)
            .collect::<String>()
    )
}

fn run_field_chips(
    form: &RunFormView,
    index: usize,
    field: &RunField,
    locale: Locale,
) -> Vec<(String, HitTarget)> {
    let mut chips = Vec::new();
    if matches!(field.role, RunFieldRole::Runner) {
        chips.push((
            format!("Ctrl+N {}", text(locale, "New agent…")),
            HitTarget::Command(UiCommand::NewRunner),
        ));
    }
    if form.can_browse_field(index) {
        chips.push((
            format!("📁 {}", text(locale, "browse")),
            HitTarget::RunFieldCommand {
                field: index,
                command: RunFieldCommand::BrowsePath,
            },
        ));
    }
    if form.can_insert_field(index) {
        chips.push((
            format!("▾ {}", text(locale, "insert")),
            HitTarget::RunFieldCommand {
                field: index,
                command: RunFieldCommand::InsertValue,
            },
        ));
    }
    if field.resettable() {
        chips.push((
            format!("↺ {}", text(locale, "default")),
            HitTarget::RunFieldCommand {
                field: index,
                command: RunFieldCommand::ResetDefault,
            },
        ));
    }
    chips
}

fn run_chip_rows(
    form: &RunFormView,
    index: usize,
    field: &RunField,
    locale: Locale,
    width: u16,
    first_row_x: u16,
) -> Vec<Vec<RunChip>> {
    let available = width.max(1);
    let mut rows = Vec::<Vec<RunChip>>::new();
    let mut row = Vec::new();
    let mut x = first_row_x.min(available.saturating_sub(1));
    for (label, target) in run_field_chips(form, index, field, locale) {
        let wanted = u16::try_from(label.width().saturating_add(2))
            .unwrap_or(u16::MAX)
            .min(available);
        if x != 0 && x.saturating_add(wanted) > available {
            rows.push(row);
            row = Vec::new();
            x = 0;
        }
        row.push(RunChip {
            label,
            x,
            width: wanted,
            target,
        });
        x = x.saturating_add(wanted).saturating_add(1);
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows
}

fn run_control_height(field: &RunField, width: u16) -> usize {
    match &field.control {
        FormControl::Text(control) if control.multiline && !control.secret => 6,
        FormControl::Text(_)
        | FormControl::Choice(skit_ui::ChoiceControl {
            presentation: ChoicePresentation::Picker,
            ..
        }) => 3,
        FormControl::Checkbox { .. } => 1,
        FormControl::Choice(control) => packed_row_count(&control.options, width),
    }
}

fn packed_row_count(labels: &[String], width: u16) -> usize {
    let available = width.max(1);
    let mut rows = 1_usize;
    let mut x = 0_u16;
    for label in labels {
        let wanted = u16::try_from(label.width().saturating_add(2))
            .unwrap_or(u16::MAX)
            .min(available);
        if x.saturating_add(wanted) > available {
            rows = rows.saturating_add(1);
            x = 0;
        }
        x = x.saturating_add(wanted).saturating_add(1);
    }
    rows
}

/// The run form's scroll affordance colour.
fn run_scrollbar_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn run_chip_style() -> ButtonStyle {
    ButtonStyle::new(ButtonVariant::SingleLine)
        .focused(Color::White, ACCENT)
        .unfocused(ACCENT, SELECT_BG)
}

pub(crate) fn new_textarea(value: &str) -> RichTextArea<'static> {
    let mut state = RichTextArea::new(value.split('\n').map(str::to_owned).collect());
    state.move_cursor(CursorMove::Bottom);
    state.move_cursor(CursorMove::End);
    state
}

pub(crate) fn textarea_control_height(value: &str, minimum_content_height: usize) -> usize {
    value.split('\n').count().max(minimum_content_height) + 2
}

pub(crate) fn textarea_text(state: &RichTextArea<'_>) -> String {
    state.lines().join("\n")
}

pub(crate) fn render_line_input(
    frame: &mut Frame,
    area: Rect,
    state: &LineInput,
    secret: bool,
    focused: bool,
    label: &str,
) -> Option<EditableGeometry> {
    render_line_input_band(
        frame,
        RowClip::new(3, 0, area),
        state,
        secret,
        focused,
        label,
        None,
    )
}

pub(crate) fn render_search_line_input(
    frame: &mut Frame,
    area: Rect,
    state: &LineInput,
    label: &str,
) -> Option<EditableGeometry> {
    if area.height < 3 {
        render_flat_search_input(frame, area, state, true, label)
    } else {
        render_line_input(frame, area, state, false, true, label)
    }
}

fn render_line_input_with_suggestion(
    frame: &mut Frame,
    area: Rect,
    state: &LineInput,
    secret: bool,
    focused: bool,
    label: &str,
    suggestion: Option<&str>,
) -> Option<EditableGeometry> {
    render_line_input_band(
        frame,
        RowClip::new(3, 0, area),
        state,
        secret,
        focused,
        label,
        suggestion,
    )
}

/// Draw only the visible band of one bordered line input.
pub(crate) fn render_line_input_band(
    frame: &mut Frame,
    clip: RowClip,
    state: &LineInput,
    secret: bool,
    focused: bool,
    label: &str,
    suggestion: Option<&str>,
) -> Option<EditableGeometry> {
    let border = if focused { ACCENT } else { BOX_DIM };
    let width = usize::from(clip.area().width.saturating_sub(2).max(1));
    let scroll = display_scroll(state.value(), state.cursor(), width, secret);
    let display = if secret {
        Line::from(Span::styled(
            secret_display(state.value()),
            Style::default().fg(Color::White),
        ))
    } else {
        let suffix = suggestion.and_then(|suggestion| suggestion.strip_prefix(state.value()));
        Line::from(vec![
            Span::styled(state.value().to_owned(), Style::default().fg(Color::White)),
            Span::styled(
                suffix.unwrap_or_default().to_owned(),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };
    clip.paint_bordered_paragraph(
        frame.buffer_mut(),
        Paragraph::new(display),
        Line::from(label),
        Style::default().fg(border),
        u16::try_from(scroll).unwrap_or(u16::MAX),
    );
    if focused
        && let Some(row) = clip.row(1)
        && row.width > 2
    {
        let visual_cursor = display_cursor(state.value(), state.cursor(), secret);
        let x = visual_cursor
            .saturating_sub(scroll)
            .min(width.saturating_sub(1));
        frame.set_cursor_position((
            row.x
                .saturating_add(1)
                .saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
            row.y,
        ));
    }
    clip.row(1).and_then(|row| {
        (row.width > 2).then(|| {
            EditableGeometry::new(
                Rect::new(
                    row.x.saturating_add(1),
                    row.y,
                    row.width.saturating_sub(2),
                    1,
                ),
                scroll,
                secret,
            )
        })
    })
}

fn render_flat_search_input(
    frame: &mut Frame,
    area: Rect,
    state: &LineInput,
    focused: bool,
    label: &str,
) -> Option<EditableGeometry> {
    let content = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height.min(1),
    );
    let width = usize::from(std::num::NonZeroU16::new(content.width)?.get());
    let _height = std::num::NonZeroU16::new(content.height)?;
    let scroll = display_scroll(state.value(), state.cursor(), width, false);
    let shown = if state.value().is_empty() {
        label.to_owned()
    } else {
        state.value().to_owned()
    };
    frame.render_widget(
        Paragraph::new(shown)
            .style(Style::default().fg(Color::White))
            .scroll((0, u16::try_from(scroll).unwrap_or(u16::MAX))),
        content,
    );
    if focused {
        let x = display_cursor(state.value(), state.cursor(), false)
            .saturating_sub(scroll)
            .min(width.saturating_sub(1));
        frame.set_cursor_position((
            content
                .x
                .saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
            content.y,
        ));
    }
    Some(EditableGeometry::new(content, scroll, false))
}

/// Draw only the visible band of one bordered text area.
pub(crate) fn render_textarea_band(
    frame: &mut Frame,
    clip: RowClip,
    state: &mut RichTextArea<'static>,
    viewport: &mut TextAreaViewport,
    focused: bool,
    label: &str,
) -> Option<TextAreaGeometry> {
    state.set_block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if focused { ACCENT } else { BOX_DIM }))
            .title(label.to_owned()),
    );
    state.set_style(Style::default().fg(Color::White));
    state.set_cursor_line_style(Style::default());
    state.set_cursor_style(if focused {
        Style::default().fg(Color::Black).bg(ACCENT)
    } else {
        Style::default().fg(Color::White)
    });
    state.set_selection_style(Style::default().fg(SELECT_FG).bg(SELECT_BG));
    let content_width = usize::from(clip.area().width.saturating_sub(2));
    let content_height = clip.full_height().saturating_sub(2);
    viewport.align(state, content_width, content_height);
    let content_start = clip.top().max(1);
    let content_end = clip
        .top()
        .saturating_add(usize::from(clip.area().height))
        .min(clip.full_height().saturating_sub(1));
    let first_row = viewport
        .top_row()
        .saturating_add(content_start.saturating_sub(1));
    let lines = bounded_textarea_lines(
        state,
        first_row,
        content_end.saturating_sub(content_start),
        viewport.left_cell(),
        content_width,
        Style::default().fg(SELECT_FG).bg(SELECT_BG),
    );
    clip.paint_bordered_lines(
        frame.buffer_mut(),
        lines,
        Line::from(label.to_owned()),
        Style::default().fg(if focused { ACCENT } else { BOX_DIM }),
    );
    let first = clip.row(content_start)?;
    (content_start < content_end && first.width > 2).then(|| {
        viewport.geometry(
            Rect::new(
                first.x.saturating_add(1),
                first.y,
                first.width.saturating_sub(2),
                u16::try_from(content_end.saturating_sub(content_start)).unwrap_or(u16::MAX),
            ),
            content_start.saturating_sub(1),
            usize::from(state.tab_length()),
        )
    })
}

pub(crate) fn checkbox_style() -> CheckBoxStyle {
    CheckBoxStyle::unicode()
        .focused_fg(ACCENT)
        .unfocused_fg(Color::White)
        .checked_fg(Color::Green)
}

pub(crate) fn select_style() -> SelectStyle {
    SelectStyle {
        focused_border: ACCENT,
        unfocused_border: BOX_DIM,
        dropdown_border: ACCENT,
        highlight_style: Style::default().fg(SELECT_FG).bg(SELECT_BG),
        ..SelectStyle::default()
    }
}

pub(crate) fn radio_style() -> ButtonStyle {
    ButtonStyle::new(ButtonVariant::Toggle)
        .focused(SELECT_FG, SELECT_BG)
        .unfocused(Color::White, Color::Reset)
        .toggled(SELECT_FG, SELECT_BG)
}

#[cfg(test)]
mod textarea_band_tests {
    use std::collections::BTreeMap;

    use ratatui_core::{backend::TestBackend, buffer::Buffer, style::Color, terminal::Terminal};

    use super::*;

    fn rendered(buffer: &Buffer) -> String {
        buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn an_open_run_select_without_a_visible_anchor_has_no_origin_dropdown() {
        let mut select = SelectState::new(2);
        select.open();
        let mut session = TuiSession::default();
        session.run.controls.push(WidgetControl::Choice {
            state: select,
            options: vec!["first".to_owned(), "second".to_owned()],
            presentation: ChoicePresentation::Picker,
            buttons: Vec::new(),
        });
        session.run.select_areas.push(None);
        session.run.dropdown_regions.push(Vec::new());
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        terminal
            .draw(|frame| session.render_open_dropdowns(frame))
            .unwrap();
        assert!(
            session.run.dropdown_regions[0].is_empty(),
            "a clipped select rendered a ghost dropdown at the default anchor"
        );
    }

    #[test]
    fn open_run_select_wheel_is_contained_and_reaches_the_last_option() {
        let options = (0..20)
            .map(|index| format!("option-{index:02}"))
            .collect::<Vec<_>>();
        let form = RunFormView::from_declarations(
            "demo",
            "Demo",
            &[],
            &BTreeMap::new(),
            &options,
            &options[0],
            &BTreeMap::new(),
            "",
        );
        let mut session = TuiSession::default();
        session.run.sync(&form);
        let index = 0;
        assert_eq!(
            session.handle_open_select_key(
                index,
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &form,
            ),
            Some(EventHandling::Consumed)
        );
        assert_eq!(
            session.handle_open_select_key(
                index,
                &KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE),
                &form,
            ),
            None,
            "an open picker must leave unrelated keys to the form"
        );
        session.run.dropdown_regions = vec![Vec::new(); session.run.controls.len()];
        session.run.dropdown_regions[index].push(ClickRegion::new(
            Rect::new(4, 4, 20, 1),
            SelectAction::Focus,
        ));
        session.run.dropdown_regions[index].extend((0..3).map(|option| {
            ClickRegion::new(
                Rect::new(5, 5 + u16::try_from(option).unwrap(), 20, 1),
                SelectAction::Select(option),
            )
        }));
        assert_eq!(
            session.handle_run_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 4,
                    row: 4,
                    modifiers: KeyModifiers::NONE,
                },
                &form,
                &ViewGeometry::default(),
            ),
            EventHandling::Ignored,
            "a non-option dropdown region must not arm an option"
        );
        assert_eq!(
            session.handle_run_mouse(
                MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: 5,
                    row: 5,
                    modifiers: KeyModifiers::NONE,
                },
                &form,
                &ViewGeometry::default(),
            ),
            EventHandling::Consumed
        );
        assert_eq!(
            session.handle_run_mouse(
                MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
                    column: 5,
                    row: 5,
                    modifiers: KeyModifiers::NONE,
                },
                &form,
                &ViewGeometry::default(),
            ),
            EventHandling::Action(Action::SelectFieldOption {
                field: index,
                value: "option-00".to_owned(),
            })
        );
        for _ in 0..30 {
            assert_eq!(
                session.handle_run_mouse(
                    MouseEvent {
                        kind: MouseEventKind::ScrollDown,
                        column: 5,
                        row: 5,
                        modifiers: KeyModifiers::NONE,
                    },
                    &form,
                    &ViewGeometry::default(),
                ),
                EventHandling::Consumed
            );
        }
        for kind in [MouseEventKind::ScrollUp, MouseEventKind::ScrollDown] {
            assert_eq!(
                session.handle_run_mouse(
                    MouseEvent {
                        kind,
                        column: 5,
                        row: 5,
                        modifiers: KeyModifiers::NONE,
                    },
                    &form,
                    &ViewGeometry::default(),
                ),
                EventHandling::Consumed
            );
        }
        assert_eq!(
            session.handle_open_select_key(
                index,
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &form,
            ),
            Some(EventHandling::Action(Action::SelectFieldOption {
                field: index,
                value: "option-19".to_owned(),
            }))
        );
    }

    #[test]
    fn search_mouse_fallback_ignores_pointer_motion() {
        let mut state = LibraryState::default();
        state.update(Action::BeginSearch);
        let mut session = TuiSession::default();

        assert_eq!(
            session.handle_search_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: 0,
                    row: 0,
                    modifiers: KeyModifiers::NONE,
                }),
                &state,
                &ViewGeometry::default(),
            ),
            EventHandling::Ignored
        );
    }

    #[test]
    fn search_and_bordered_inputs_publish_only_real_content_cells() {
        let input = LineInput::new("界abc".to_owned());

        let mut compact = Terminal::new(TestBackend::new(8, 2)).unwrap();
        let mut compact_geometry = None;
        compact
            .draw(|frame| {
                compact_geometry = render_search_line_input(frame, frame.area(), &input, "Search");
            })
            .unwrap();
        let compact_text = rendered(compact.backend().buffer());
        assert!(compact_geometry.is_some());
        assert!(
            !compact_text.contains('╭'),
            "a two-row search drew a border"
        );

        let mut bordered = Terminal::new(TestBackend::new(8, 3)).unwrap();
        let mut bordered_geometry = None;
        bordered
            .draw(|frame| {
                bordered_geometry = render_search_line_input(frame, frame.area(), &input, "Search");
            })
            .unwrap();
        assert!(bordered_geometry.is_some());
        assert!(
            rendered(bordered.backend().buffer()).contains('┌'),
            "an exact three-row search lost its border"
        );
        assert_eq!(
            bordered.backend().cursor_position().y,
            1,
            "the exact-width Unicode input lost its content cursor"
        );

        let mut two_columns = Terminal::new(TestBackend::new(2, 3)).unwrap();
        let mut two_column_geometry = None;
        two_columns
            .draw(|frame| {
                two_column_geometry = render_line_input(
                    frame,
                    frame.area(),
                    &LineInput::new(String::new()),
                    false,
                    true,
                    "Value",
                );
            })
            .unwrap();
        assert!(two_column_geometry.is_none());
        assert_ne!(
            two_columns.backend().cursor_position(),
            ratatui_core::layout::Position::new(1, 1),
            "a border-only row fabricated a content cursor"
        );

        let mut three_columns = Terminal::new(TestBackend::new(3, 3)).unwrap();
        let mut three_column_geometry = None;
        three_columns
            .draw(|frame| {
                three_column_geometry = render_line_input(
                    frame,
                    frame.area(),
                    &LineInput::new(String::new()),
                    false,
                    true,
                    "Value",
                );
            })
            .unwrap();
        assert!(
            three_column_geometry.is_some(),
            "one content cell lost geometry"
        );
        assert_eq!(
            three_columns.backend().cursor_position(),
            ratatui_core::layout::Position::new(1, 1)
        );

        for (width, height, focused, expected_geometry) in [
            (2, 1, true, false),
            (3, 0, true, false),
            (3, 1, false, true),
            (3, 1, true, true),
        ] {
            let backend_height = height.max(1);
            let mut terminal = Terminal::new(TestBackend::new(width, backend_height)).unwrap();
            let mut geometry = None;
            terminal
                .draw(|frame| {
                    geometry = render_flat_search_input(
                        frame,
                        Rect::new(0, 0, width, height),
                        &input,
                        focused,
                        "Search",
                    );
                })
                .unwrap();
            assert_eq!(
                geometry.is_some(),
                expected_geometry,
                "flat search geometry at {width}x{height}, focused={focused}"
            );
            if focused && !expected_geometry {
                assert_ne!(
                    terminal.backend().cursor_position(),
                    ratatui_core::layout::Position::new(1, 0),
                    "a flat input without content fabricated a cursor"
                );
            }
        }
    }

    #[test]
    fn flat_search_paints_and_places_its_caret_in_exact_nonzero_content() {
        for (area, value, label, painted, expected_cursor) in [
            (
                Rect::new(2, 1, 3, 1),
                "",
                "Q",
                vec![(3, "Q")],
                ratatui_core::layout::Position::new(3, 1),
            ),
            (
                Rect::new(2, 1, 5, 1),
                "ab",
                "Search",
                vec![(3, "a"), (4, "b")],
                ratatui_core::layout::Position::new(5, 1),
            ),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
            let mut geometry = None;
            terminal
                .draw(|frame| {
                    geometry = render_flat_search_input(
                        frame,
                        area,
                        &LineInput::new(value.to_owned()),
                        true,
                        label,
                    );
                })
                .unwrap();

            assert!(geometry.is_some(), "nonzero content lost edit geometry");
            for (column, symbol) in painted {
                assert_eq!(
                    terminal.backend().buffer()[(column, area.y)].symbol(),
                    symbol,
                    "the flat input did not paint its final visible cells"
                );
            }
            assert_eq!(
                terminal.backend().cursor_position(),
                expected_cursor,
                "the focused flat input did not place its terminal caret"
            );
        }
    }

    #[test]
    fn clipped_textarea_geometry_excludes_borders_and_zero_width_content() {
        fn draw_band(top: u16, width: u16, height: u16) -> (bool, String) {
            let backend_height = height.max(1);
            let mut terminal = Terminal::new(TestBackend::new(width, backend_height)).unwrap();
            let mut state = new_textarea("zero\none\ntwo\nthree");
            let mut viewport = TextAreaViewport::default();
            let mut has_geometry = false;
            terminal
                .draw(|frame| {
                    has_geometry = render_textarea_band(
                        frame,
                        RowClip::new(6, usize::from(top), Rect::new(0, 0, width, height)),
                        &mut state,
                        &mut viewport,
                        true,
                        "Body",
                    )
                    .is_some();
                })
                .unwrap();
            (has_geometry, rendered(terminal.backend().buffer()))
        }

        assert!(!draw_band(0, 8, 1).0, "a top border became editable");
        assert!(draw_band(1, 8, 1).0, "one content row lost geometry");
        assert!(
            !draw_band(1, 2, 1).0,
            "a border-only textarea width became editable"
        );
        assert!(!draw_band(5, 8, 1).0, "a bottom border became editable");
        let (has_geometry, rendered) = draw_band(4, 8, 2);
        assert!(has_geometry, "the last content row lost geometry");
        assert!(
            rendered.contains("three"),
            "the bottom-clipped editor did not paint its last content row: {rendered:?}"
        );
    }

    #[test]
    fn bounded_textarea_render_keeps_complete_unicode_graphemes_and_cursor_style() {
        for (value, expected) in [("e\u{301}x", "e\u{301}"), ("👨‍👩‍👧x", "👨‍👩‍👧")]
        {
            for (width, horizontal_tail) in [(12, false), (5, true)] {
                let value = if horizontal_tail {
                    format!("abcd{value}")
                } else {
                    value.to_owned()
                };
                let mut state = new_textarea(&value);
                if !horizontal_tail {
                    state.move_cursor(CursorMove::Head);
                }
                let mut viewport = TextAreaViewport::default();
                let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
                terminal
                    .draw(|frame| {
                        let _ = render_textarea_band(
                            frame,
                            RowClip::new(3, 0, frame.area()),
                            &mut state,
                            &mut viewport,
                            true,
                            "Body",
                        );
                    })
                    .unwrap();
                let content_x = 1;
                let cell = &terminal.backend().buffer()[(content_x, 1)];
                assert_eq!(
                    cell.symbol(),
                    expected,
                    "the final buffer split {expected:?} at width {width}"
                );
                if !horizontal_tail {
                    assert_eq!(cell.fg, Color::Black);
                    assert_eq!(cell.bg, ACCENT, "the cursor did not style one grapheme");
                }
            }
        }
    }

    #[test]
    fn one_cell_textarea_keeps_a_caret_for_a_cursor_inside_one_grapheme() {
        for value in ["e\u{301}x", "👨‍👩‍👧x"] {
            let mut state = new_textarea(value);
            state.move_cursor(CursorMove::Head);
            state.move_cursor(CursorMove::Forward);
            assert_eq!(
                state.cursor().1,
                1,
                "the fixture cursor must be inside {value:?}"
            );
            let mut viewport = TextAreaViewport::default();
            let mut terminal = Terminal::new(TestBackend::new(3, 3)).unwrap();

            terminal
                .draw(|frame| {
                    let _ = render_textarea_band(
                        frame,
                        RowClip::new(3, 0, frame.area()),
                        &mut state,
                        &mut viewport,
                        true,
                        "Body",
                    );
                })
                .unwrap();

            let caret = &terminal.backend().buffer()[(1, 1)];
            assert_eq!(caret.fg, Color::Black, "the caret vanished for {value:?}");
            assert_eq!(caret.bg, ACCENT, "the caret vanished for {value:?}");
            assert_eq!(
                textarea_text(&state),
                value,
                "render changed the model value"
            );
        }

        let mut state = new_textarea("a\tX");
        state.move_cursor(CursorMove::Head);
        state.move_cursor(CursorMove::Forward);
        state.move_cursor(CursorMove::Forward);
        assert_eq!(state.cursor().1, 2, "the cursor must follow the tab");
        let mut viewport = TextAreaViewport::default();
        let mut terminal = Terminal::new(TestBackend::new(3, 3)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_textarea_band(
                    frame,
                    RowClip::new(3, 0, frame.area()),
                    &mut state,
                    &mut viewport,
                    true,
                    "Body",
                );
            })
            .unwrap();
        let caret = &terminal.backend().buffer()[(1, 1)];
        assert_eq!(caret.symbol(), "X");
        assert_eq!((caret.fg, caret.bg), (Color::Black, ACCENT));
    }

    #[test]
    fn rendered_textarea_clicks_use_tab_cells_and_rows_above_u16() {
        for column in [0, 1, 2] {
            let mut state = new_textarea("\tX");
            state.move_cursor(CursorMove::Head);
            let mut viewport = TextAreaViewport::default();
            let mut geometry = None;
            let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
            terminal
                .draw(|frame| {
                    geometry = render_textarea_band(
                        frame,
                        RowClip::new(3, 0, frame.area()),
                        &mut state,
                        &mut viewport,
                        true,
                        "Body",
                    );
                })
                .unwrap();
            let geometry = geometry.expect("the tab row is editable");
            assert!(geometry.place_cursor(&mut state, 1 + column, 1));
            state.insert_char('Z');
            assert_eq!(textarea_text(&state), "Z\tX", "tab cell {column}");
        }

        let mut lines = vec![String::new(); usize::from(u16::MAX) + 2];
        lines[usize::from(u16::MAX) + 1] = "abcdef".to_owned();
        let mut state = RichTextArea::new(lines);
        state.move_cursor(CursorMove::Bottom);
        state.move_cursor(CursorMove::Head);
        let mut viewport = TextAreaViewport::default();
        let mut geometry = None;
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
        terminal
            .draw(|frame| {
                geometry = render_textarea_band(
                    frame,
                    RowClip::new(3, 0, frame.area()),
                    &mut state,
                    &mut viewport,
                    true,
                    "Body",
                );
            })
            .unwrap();
        let geometry = geometry.expect("the virtual tail row is editable");
        assert!(geometry.place_cursor(&mut state, 4, 1));
        state.insert_char('X');
        assert_eq!(state.lines().last().unwrap(), "abcXdef");

        let mut state = new_textarea("a");
        state.move_cursor(CursorMove::Head);
        let mut viewport = TextAreaViewport::default();
        let mut geometry = None;
        let mut terminal = Terminal::new(TestBackend::new(10, 3)).unwrap();
        terminal
            .draw(|frame| {
                geometry = render_textarea_band(
                    frame,
                    RowClip::new(3, 0, frame.area()),
                    &mut state,
                    &mut viewport,
                    true,
                    "Body",
                );
            })
            .unwrap();
        let geometry = geometry.expect("the short row is editable");
        assert!(geometry.place_cursor(&mut state, 7, 1));
        state.insert_char('X');
        assert_eq!(textarea_text(&state), "aX");
    }

    #[test]
    fn run_checkbox_style_is_visible_on_the_rendered_control() {
        for (checked, focused, symbol, color) in [
            (false, false, "☐", Color::White),
            (false, true, "☐", ACCENT),
            (true, false, "☑", Color::Green),
            (true, true, "☑", ACCENT),
        ] {
            let state = CheckBoxState {
                checked,
                focused,
                ..CheckBoxState::default()
            };
            let mut terminal = Terminal::new(TestBackend::new(12, 1)).unwrap();
            terminal
                .draw(|frame| {
                    let _ = CheckBox::new("value", &state)
                        .style(checkbox_style())
                        .render_stateful(frame.area(), frame.buffer_mut());
                })
                .unwrap();
            let cell = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .find(|cell| cell.symbol() == symbol)
                .expect("checkbox symbol");
            assert_eq!(cell.fg, color, "checked={checked}, focused={focused}");
        }
    }

    #[test]
    fn an_oversized_focused_run_group_does_not_align_to_its_note_tail() {
        let mut run = RunWidgetSession {
            pending_ensure_focus: true,
            ..RunWidgetSession::default()
        };
        let layout = RunLayout {
            items: Vec::new(),
            control_starts: vec![1],
            control_heights: vec![3],
            height: 12,
        };
        run.prepare_layout(&layout, 0, Rect::new(0, 0, 24, 4));
        assert!(
            run.scroll.scroll_offset() < 8,
            "the focused control was hidden to show the tail of its oversized note group"
        );
    }

    fn positioned_run_layout(start: usize, height: usize, total: usize) -> RunLayout {
        RunLayout {
            items: Vec::new(),
            control_starts: vec![start],
            control_heights: vec![height],
            height: total,
        }
    }

    fn run_session_at(offset: usize) -> RunWidgetSession {
        let mut run = RunWidgetSession {
            pending_ensure_focus: true,
            ..RunWidgetSession::default()
        };
        run.scroll.set_lines(vec![String::new(); 12]);
        run.scroll.set_scroll_offset(offset);
        run
    }

    #[test]
    fn run_focus_alignment_changes_only_for_a_control_outside_the_viewport() {
        let viewport = Rect::new(5, 7, 20, 4);

        let mut exact = run_session_at(1);
        exact.prepare_layout(&positioned_run_layout(1, 4, 12), 0, viewport);
        assert_eq!(
            exact.scroll.scroll_offset(),
            1,
            "a control on both viewport boundaries changed the user offset"
        );

        let mut above = run_session_at(2);
        above.prepare_layout(&positioned_run_layout(1, 2, 12), 0, viewport);
        assert_eq!(above.scroll.scroll_offset(), 1);

        let mut below = run_session_at(1);
        below.prepare_layout(&positioned_run_layout(2, 4, 12), 0, viewport);
        assert_eq!(
            below.scroll.scroll_offset(),
            2,
            "one hidden row did not move the focused control into view"
        );

        below.scroll.set_scroll_offset(5);
        below.prepare_layout(&positioned_run_layout(2, 4, 12), 0, viewport);
        assert_eq!(
            below.scroll.scroll_offset(),
            5,
            "an unchanged layout reset a later user scroll"
        );

        let mut clamped = run_session_at(7);
        clamped.prepare_layout(&positioned_run_layout(0, 1, 8), 1, viewport);
        assert_eq!(
            clamped.scroll.scroll_offset(),
            4,
            "a shorter layout retained an offset after its last full viewport"
        );
    }

    #[test]
    fn long_run_textarea_cursor_stays_inside_its_fixed_outer_control() {
        let value = (0..30)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut run = run_session_at(0);
        run.controls.push(WidgetControl::TextArea {
            state: Box::new(new_textarea(&value)),
            focused: true,
            undo_group: 0,
            redo_group: 0,
        });
        let layout = positioned_run_layout(2, 6, 12);
        run.prepare_layout(&layout, 0, Rect::new(0, 0, 24, 5));
        assert_eq!(
            run.scroll.scroll_offset(),
            2,
            "the textarea's internal cursor escaped the outer layout extent"
        );
        assert!(run.scroll.scroll_offset() <= layout.height.saturating_sub(run.visible_height));
    }

    #[test]
    fn run_visible_rect_rejects_each_disjoint_side_and_clips_each_overlap() {
        let mut run = run_session_at(3);
        run.viewport = Rect::new(5, 7, 20, 4);
        run.visible_height = 4;

        assert_eq!(run.visible_rect(0, 3), None, "an item above became a hit");
        assert_eq!(run.visible_rect(7, 2), None, "an item below became a hit");
        assert_eq!(run.visible_rect(2, 3), Some(Rect::new(5, 7, 20, 2)));
        assert_eq!(run.visible_rect(5, 3), Some(Rect::new(5, 9, 20, 2)));
    }

    fn generic_form(fields: Vec<FormField>, focused: usize) -> FormView {
        FormView {
            purpose: skit_ui::FormPurpose::Settings,
            title: "Form".to_owned(),
            title_arguments: Vec::new(),
            translate_title: false,
            selector: None,
            fields,
            focused,
            submit_label: "Save".to_owned(),
        }
    }

    #[test]
    fn generic_form_alignment_and_clipping_use_exact_viewport_boundaries() {
        let fields = (0..4)
            .map(|index| FormField::text(format!("field-{index}"), "Field", "value"))
            .collect::<Vec<_>>();
        let viewport = Rect::new(4, 6, 18, 3);
        let form = generic_form(fields, 1);

        let mut exact = FormWidgetSession {
            pending_ensure_focus: true,
            ..FormWidgetSession::default()
        };
        exact.sync(&form);
        exact.scroll.set_lines(vec![String::new(); 12]);
        exact.scroll.set_scroll_offset(3);
        exact.prepare_layout(&form, viewport);
        assert_eq!(exact.scroll.scroll_offset(), 3);

        let mut above = FormWidgetSession {
            pending_ensure_focus: true,
            ..FormWidgetSession::default()
        };
        above.sync(&form);
        above.scroll.set_lines(vec![String::new(); 12]);
        above.scroll.set_scroll_offset(4);
        above.prepare_layout(&form, viewport);
        assert_eq!(above.scroll.scroll_offset(), 3);

        let mut below = FormWidgetSession {
            pending_ensure_focus: true,
            ..FormWidgetSession::default()
        };
        below.sync(&form);
        below.scroll.set_lines(vec![String::new(); 12]);
        below.scroll.set_scroll_offset(2);
        below.prepare_layout(&form, viewport);
        assert_eq!(below.scroll.scroll_offset(), 3);

        exact.scroll.set_scroll_offset(7);
        exact.prepare_layout(&form, viewport);
        assert_eq!(
            exact.scroll.scroll_offset(),
            7,
            "an unchanged form reflow reset a user scroll"
        );

        let mut no_target = form.clone();
        no_target.focused = usize::MAX;
        let mut clamped = FormWidgetSession::default();
        clamped.sync(&no_target);
        clamped.scroll.set_lines(vec![String::new(); 12]);
        clamped.scroll.set_scroll_offset(11);
        clamped.prepare_layout(&no_target, viewport);
        assert_eq!(
            clamped.scroll.scroll_offset(),
            9,
            "a shorter form retained an offset after its last full viewport"
        );

        exact.row_starts = vec![0, 2, 3, 5, 6];
        exact.row_heights = vec![3, 3, 3, 3, 2];
        exact.scroll.set_scroll_offset(3);
        assert!(exact.visible_band(0).is_none(), "an above band survived");
        assert!(exact.visible_band(4).is_none(), "a below band survived");
        let clipped_top = exact.visible_band(1).expect("top-clipped band");
        assert_eq!(clipped_top.area(), Rect::new(4, 6, 18, 2));
        let top = exact.visible_band(2).expect("exact visible band");
        assert_eq!(top.area(), Rect::new(4, 6, 18, 3));
        let clipped_bottom = exact.visible_band(3).expect("bottom-clipped band");
        assert_eq!(clipped_bottom.area(), Rect::new(4, 8, 18, 1));
    }

    #[test]
    fn oversized_form_textarea_aligns_its_cursor_row_without_hiding_its_start() {
        let form = generic_form(
            vec![
                FormField::text("prefix", "Prefix", "value"),
                FormField::multiline("body", "Body", "zero\none\ntwo\nthree\nfour"),
            ],
            1,
        );
        let viewport = Rect::new(0, 0, 20, 4);
        let mut session = TuiSession::default();
        session.form.sync(&form);
        session.form.prepare_layout(&form, viewport);
        assert_eq!(
            session.form.scroll.scroll_offset(),
            5,
            "the last textarea row was not aligned inside the viewport"
        );

        for _ in 0..10 {
            assert_eq!(
                session.handle_form_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &form,),
                EventHandling::Consumed,
                "the textarea did not accept an upward cursor command"
            );
        }
        session.form.prepare_layout(&form, viewport);
        assert_eq!(
            session.form.scroll.scroll_offset(),
            3,
            "the first textarea row was hidden by absolute-row arithmetic"
        );
    }

    #[test]
    fn run_textarea_outer_alignment_distinguishes_cursor_viewport_boundaries() {
        let value = (0..6)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let layout = positioned_run_layout(5, 6, 15);
        let viewport = Rect::new(0, 0, 24, 3);

        for (cursor_row, expected_offset) in [(0, 5), (2, 6), (3, 7)] {
            let mut state = new_textarea(&value);
            state.move_cursor(CursorMove::Jump(cursor_row, 0));
            let mut run = run_session_at(0);
            run.controls.push(WidgetControl::TextArea {
                state: Box::new(state),
                focused: true,
                undo_group: 0,
                redo_group: 0,
            });

            run.prepare_layout(&layout, 0, viewport);

            assert_eq!(
                run.scroll.scroll_offset(),
                expected_offset,
                "cursor row {cursor_row} did not keep the outer textarea caret visible"
            );
        }
    }

    #[test]
    fn form_textarea_alignment_distinguishes_cursor_viewport_boundaries() {
        let value = (0..6)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let form = generic_form(
            vec![
                FormField::text("prefix", "Prefix", "value"),
                FormField::multiline("body", "Body", &value),
            ],
            1,
        );
        let viewport = Rect::new(0, 0, 24, 3);

        for (cursor_row, expected_offset) in [(0, 3), (2, 4), (3, 5)] {
            let mut session = TuiSession::default();
            session.form.sync(&form);
            for _ in 0..10 {
                assert_eq!(
                    session.handle_form_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), &form,),
                    EventHandling::Consumed
                );
            }
            for _ in 0..cursor_row {
                assert_eq!(
                    session
                        .handle_form_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE), &form,),
                    EventHandling::Consumed
                );
            }

            session.form.prepare_layout(&form, viewport);

            assert_eq!(
                session.form.scroll.scroll_offset(),
                expected_offset,
                "cursor row {cursor_row} did not keep the form textarea caret visible"
            );
        }

        let mut unchanged = FormWidgetSession::default();
        unchanged.sync(&form);
        unchanged.prepare_layout(&form, viewport);
        unchanged.scroll.set_scroll_offset(7);
        unchanged.prepare_layout(&form, viewport);
        assert_eq!(
            unchanged.scroll.scroll_offset(),
            7,
            "an unchanged textarea layout reset a later user scroll"
        );
    }

    #[test]
    fn generic_form_control_shape_requires_multiline_and_nonsecret_text() {
        let plain = FormField::text("plain", "Plain", "value");
        let multiline = FormField::multiline("body", "Body", "first\nsecond");
        let mut secret_multiline = multiline.clone();
        secret_multiline.secret = true;

        assert!(matches!(
            form_widget_control(&plain),
            FormWidgetControl::Input { secret: false, .. }
        ));
        assert!(matches!(
            form_widget_control(&multiline),
            FormWidgetControl::TextArea { .. }
        ));
        assert!(matches!(
            form_widget_control(&secret_multiline),
            FormWidgetControl::Input { secret: true, .. }
        ));
    }

    #[test]
    fn generic_form_controls_sync_external_values_without_resetting_equal_values() {
        let mut input_form = generic_form(vec![FormField::text("name", "Name", "abcdef")], 0);
        let mut input_session = TuiSession::default();
        input_session.form.sync(&input_form);
        assert_eq!(
            input_session.handle_form_key(
                KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                &input_form,
            ),
            EventHandling::Consumed
        );
        input_session.form.sync(&input_form);
        assert_eq!(
            input_session.handle_form_key(
                KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE),
                &input_form,
            ),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "abcdeXf".to_owned(),
            }),
            "an equal external value reset the line-input cursor"
        );
        input_form.fields[0].value = "changed".to_owned();
        input_session.form.sync(&input_form);
        assert_eq!(
            input_session.handle_form_key(
                KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
                &input_form,
            ),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "changed!".to_owned(),
            }),
            "an external line-input value did not replace the stale editor value"
        );

        let mut textarea_form = generic_form(
            vec![FormField::multiline("body", "Body", "first\nsecond")],
            0,
        );
        let mut textarea_session = TuiSession::default();
        textarea_session.form.sync(&textarea_form);
        assert_eq!(
            textarea_session.handle_form_key(
                KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                &textarea_form,
            ),
            EventHandling::Consumed
        );
        textarea_session.form.sync(&textarea_form);
        assert_eq!(
            textarea_session.handle_form_key(
                KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE),
                &textarea_form,
            ),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "first\nseconXd".to_owned(),
            }),
            "an equal external value reset the textarea cursor"
        );
        textarea_form.fields[0].value = "new\nvalue".to_owned();
        textarea_session.form.sync(&textarea_form);
        assert_eq!(
            textarea_session.handle_form_key(
                KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE),
                &textarea_form,
            ),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "new\nvalue!".to_owned(),
            }),
            "an external textarea value did not replace the stale editor value"
        );
    }

    fn text_control(value: &str, multiline: bool, secret: bool) -> FormControl {
        FormControl::Text(skit_ui::TextControl {
            value: value.to_owned(),
            multiline,
            secret,
            ..skit_ui::TextControl::default()
        })
    }

    fn choice_control(options: &[&str], selected: &str) -> FormControl {
        FormControl::Choice(skit_ui::ChoiceControl {
            options: options.iter().map(|value| (*value).to_owned()).collect(),
            selected: selected.to_owned(),
            presentation: ChoicePresentation::Radio,
        })
    }

    fn run_field_with_control(key: &str, control: FormControl) -> RunField {
        RunField {
            key: key.to_owned(),
            label: key.to_owned(),
            help: String::new(),
            role: RunFieldRole::Parameter {
                name: key.to_owned(),
            },
            parameter_type: ParameterType::Str,
            multiple: false,
            binding: skit_domain::parameters::ParameterBinding::None,
            delivery: skit_domain::parameters::ParameterDelivery::Flag,
            control,
            required: false,
            default: None,
            degraded: false,
            input_binding: false,
            env_source: String::new(),
            validation_error: None,
            feedback: Default::default(),
        }
    }

    fn rendered_run_control(control: WidgetControl) -> Vec<(String, Color)> {
        let mut session = TuiSession::default();
        session.run.controls.push(control);
        session.run.editables.push(None);
        session.run.textarea_editables.push(None);
        session
            .run
            .textarea_viewports
            .push(TextAreaViewport::default());
        session.run.select_areas.push(None);
        session.run.dropdown_regions.push(Vec::new());
        let mut terminal = Terminal::new(TestBackend::new(24, 3)).unwrap();
        terminal
            .draw(|frame| {
                session.render_run_control(
                    frame,
                    RowClip::new(1, 0, Rect::new(0, 0, 24, 1)),
                    0,
                    Locale::En,
                    &mut Vec::new(),
                );
            })
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| (cell.symbol().to_owned(), cell.fg))
            .collect()
    }

    fn run_form_for_event_tests() -> RunFormView {
        RunFormView::from_declarations(
            "demo",
            "Demo",
            &[],
            &BTreeMap::new(),
            &[],
            "",
            &BTreeMap::new(),
            "",
        )
    }

    #[test]
    fn run_control_shape_and_choice_selection_follow_the_typed_model() {
        let multiline = run_field_with_control("body", text_control("a\nb", true, false));
        let secret_multiline = run_field_with_control("secret", text_control("a\nb", true, true));
        assert!(matches!(
            widget_control(&multiline),
            WidgetControl::TextArea { .. }
        ));
        assert!(matches!(
            widget_control(&secret_multiline),
            WidgetControl::Input { secret: true, .. }
        ));

        let selected = rendered_run_control(widget_control(&run_field_with_control(
            "choice",
            choice_control(&["a", "b"], "b"),
        )));
        assert_eq!(
            selected
                .iter()
                .find(|(symbol, _)| symbol == "a")
                .expect("first radio option")
                .1,
            Color::White
        );
        assert_eq!(
            selected
                .iter()
                .find(|(symbol, _)| symbol == "b")
                .expect("selected radio option")
                .1,
            SELECT_FG
        );

        let missing = rendered_run_control(widget_control(&run_field_with_control(
            "choice",
            choice_control(&["a", "b"], "missing"),
        )));
        for option in ["a", "b"] {
            assert_eq!(
                missing
                    .iter()
                    .find(|(symbol, _)| symbol == option)
                    .expect("unselected radio option")
                    .1,
                Color::White,
                "a missing model selection toggled {option}"
            );
        }
    }

    #[test]
    fn run_controls_sync_every_external_value_and_preserve_equal_text_cursors() {
        let form = run_form_for_event_tests();
        let mut input_session = TuiSession::default();
        input_session
            .run
            .controls
            .push(widget_control(&run_field_with_control(
                "input",
                text_control("abcdef", false, false),
            )));
        assert_eq!(
            input_session.handle_run_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &form,),
            EventHandling::Consumed
        );
        input_session.run.controls[0].sync_value(&text_control("abcdef", false, false));
        assert_eq!(
            input_session
                .handle_run_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE), &form,),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "abcdeXf".to_owned(),
            }),
            "an equal run value reset the line-input cursor"
        );
        input_session.run.controls[0].sync_value(&text_control("changed", false, false));
        assert_eq!(
            input_session
                .handle_run_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE), &form,),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "changed!".to_owned(),
            }),
            "an external run value did not replace stale line-input text"
        );

        let mut textarea_session = TuiSession::default();
        textarea_session
            .run
            .controls
            .push(widget_control(&run_field_with_control(
                "body",
                text_control("first\nsecond", true, false),
            )));
        assert_eq!(
            textarea_session
                .handle_run_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE), &form,),
            EventHandling::Consumed
        );
        textarea_session.run.controls[0].sync_value(&text_control("first\nsecond", true, false));
        assert_eq!(
            textarea_session
                .handle_run_key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE), &form,),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "first\nseconXd".to_owned(),
            }),
            "an equal run value reset the textarea cursor"
        );
        textarea_session.run.controls[0].sync_value(&text_control("new\nvalue", true, false));
        assert_eq!(
            textarea_session
                .handle_run_key(KeyEvent::new(KeyCode::Char('!'), KeyModifiers::NONE), &form,),
            EventHandling::Action(Action::SetFieldValue {
                field: 0,
                value: "new\nvalue!".to_owned(),
            }),
            "an external run value did not replace stale textarea text"
        );

        let mut checkbox = WidgetControl::Checkbox(CheckBoxState::new(false));
        checkbox.sync_value(&FormControl::Checkbox { checked: true });
        let checked = rendered_run_control(checkbox);
        assert!(
            checked.iter().any(|(symbol, _)| symbol == "☑"),
            "the external boolean value did not reach the rendered checkbox"
        );

        let mut choice = widget_control(&run_field_with_control(
            "choice",
            choice_control(&["a", "b"], "a"),
        ));
        choice.sync_value(&choice_control(&["a", "b"], "b"));
        let selected = rendered_run_control(choice);
        assert_eq!(
            selected
                .iter()
                .find(|(symbol, _)| symbol == "b")
                .expect("externally selected radio option")
                .1,
            SELECT_FG
        );
        assert_eq!(
            selected
                .iter()
                .find(|(symbol, _)| symbol == "a")
                .expect("externally unselected radio option")
                .1,
            Color::White
        );
    }

    #[test]
    fn run_choice_focus_marks_only_the_active_option_and_closes_on_blur() {
        let missing = run_field_with_control("choice", choice_control(&["a", "b"], "missing"));
        let mut focused = widget_control(&missing);
        focused.set_focused(true);
        let focused = rendered_run_control(focused);
        assert_eq!(
            focused
                .iter()
                .find(|(symbol, _)| symbol == "a")
                .expect("focused fallback radio option")
                .1,
            SELECT_FG
        );
        assert_eq!(
            focused
                .iter()
                .find(|(symbol, _)| symbol == "b")
                .expect("unfocused radio option")
                .1,
            Color::White
        );

        let mut unfocused = widget_control(&missing);
        unfocused.set_focused(false);
        let unfocused = rendered_run_control(unfocused);
        for option in ["a", "b"] {
            assert_eq!(
                unfocused
                    .iter()
                    .find(|(symbol, _)| symbol == option)
                    .expect("blurred radio option")
                    .1,
                Color::White,
                "blur left focus on {option}"
            );
        }

        let picker = FormControl::Choice(skit_ui::ChoiceControl {
            options: vec!["a".to_owned(), "b".to_owned()],
            selected: "b".to_owned(),
            presentation: ChoicePresentation::Picker,
        });
        let mut session = TuiSession::default();
        session
            .run
            .controls
            .push(widget_control(&run_field_with_control("choice", picker)));
        session.run.controls[0].set_focused(true);
        let form = run_form_for_event_tests();
        assert_eq!(
            session.handle_open_select_key(
                0,
                &KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                &form,
            ),
            Some(EventHandling::Consumed),
            "the focused picker did not open"
        );
        session.run.controls[0].set_focused(false);
        assert_eq!(
            session.handle_open_select_key(
                0,
                &KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
                &form,
            ),
            None,
            "a blurred picker stayed open"
        );
    }

    #[test]
    fn a_top_clipped_form_textarea_keeps_later_rows_cursor_and_selection_styles() {
        let form = FormView {
            purpose: skit_ui::FormPurpose::Settings,
            title: "Editor".to_owned(),
            title_arguments: Vec::new(),
            translate_title: false,
            selector: None,
            fields: vec![FormField::multiline("body", "Body", "first\nmiddle\nlast")],
            focused: 0,
            submit_label: "Save".to_owned(),
        };
        let mut library = LibraryState::default();
        library.update(Action::Present(Screen::Form(form.clone())));
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(24, 4)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = session.render_form(frame, frame.area(), &form, Locale::En);
            })
            .unwrap();
        for code in [KeyCode::Up, KeyCode::Up, KeyCode::Home] {
            assert_eq!(
                session.handle_event(
                    Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                    &library,
                    &geometry,
                ),
                EventHandling::Consumed
            );
        }
        terminal
            .draw(|frame| {
                geometry = session.render_form(frame, frame.area(), &form, Locale::En);
            })
            .unwrap();
        assert!(
            rendered(terminal.backend().buffer()).contains("first"),
            "the real cursor move did not bring the first row into view"
        );
        for code in [KeyCode::Down, KeyCode::Down, KeyCode::End] {
            assert_eq!(
                session.handle_event(
                    Event::Key(KeyEvent::new(code, KeyModifiers::SHIFT)),
                    &library,
                    &geometry,
                ),
                EventHandling::Consumed
            );
        }
        terminal
            .draw(|frame| {
                geometry = session.render_form(frame, frame.area(), &form, Locale::En);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let frame = rendered(buffer);
        let rows = buffer
            .content()
            .chunks(usize::from(buffer.area.width))
            .collect::<Vec<_>>();
        let middle = rows
            .iter()
            .find(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .contains("middle")
            })
            .unwrap();
        let last = rows
            .iter()
            .find(|row| {
                row.iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .contains("last")
            })
            .unwrap();

        assert_eq!(geometry.first_visible, 2);
        assert!(
            frame.contains("middle"),
            "the middle row is missing:\n{frame}"
        );
        assert!(frame.contains("last"), "the last row is missing:\n{frame}");
        assert!(
            !frame.contains("first"),
            "the clipped first row returned:\n{frame}"
        );
        assert!(
            middle
                .iter()
                .any(|cell| cell.symbol() != " " && cell.bg == SELECT_BG),
            "the selection style is missing from the middle row:\n{frame}"
        );
        assert!(
            last.iter()
                .any(|cell| cell.fg == Color::Black && cell.bg == ACCENT),
            "the focused cursor style is missing from the last row:\n{frame}"
        );

        let followed = geometry.first_visible;
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column: geometry.rows.x,
                    row: geometry.rows.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &library,
                &geometry,
            ),
            EventHandling::Consumed
        );
        terminal
            .draw(|frame| {
                geometry = session.render_form(frame, frame.area(), &form, Locale::En);
            })
            .unwrap();
        assert!(geometry.first_visible > followed);
        let wheel = geometry.first_visible;
        terminal
            .draw(|frame| {
                geometry = session.render_form(frame, frame.area(), &form, Locale::En);
            })
            .unwrap();
        assert_eq!(
            geometry.first_visible, wheel,
            "a render without a cursor move undid the reader's wheel scroll"
        );
    }

    #[test]
    fn form_focus_follow_brings_fields_below_and_above_the_fold_into_view() {
        let mut library = LibraryState::default();
        library.update(Action::Present(Screen::Form(FormView {
            purpose: skit_ui::FormPurpose::Settings,
            title: "Fields".to_owned(),
            title_arguments: Vec::new(),
            translate_title: false,
            selector: None,
            fields: vec![
                FormField::text("zero", "Zero", "value-zero"),
                FormField::text("one", "One", "value-one"),
                FormField::text("two", "Two", "value-two"),
                FormField::text("three", "Three", "value-three"),
            ],
            focused: 0,
            submit_label: "Save".to_owned(),
        })));
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(24, 5)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                if let Screen::Form(form) = library.screen() {
                    geometry = session.render_form(frame, frame.area(), form, Locale::En);
                }
            })
            .unwrap();

        for _ in 0..2 {
            if let EventHandling::Action(action) = session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
                &library,
                &geometry,
            ) {
                library.update(action);
            }
        }
        terminal
            .draw(|frame| {
                if let Screen::Form(form) = library.screen() {
                    geometry = session.render_form(frame, frame.area(), form, Locale::En);
                }
            })
            .unwrap();
        assert!(
            rendered(terminal.backend().buffer()).contains("value-two"),
            "focus below the fold did not end-align its field"
        );

        if let EventHandling::Action(action) = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
            &library,
            &geometry,
        ) {
            library.update(action);
        }
        terminal
            .draw(|frame| {
                if let Screen::Form(form) = library.screen() {
                    geometry = session.render_form(frame, frame.area(), form, Locale::En);
                }
            })
            .unwrap();
        assert!(
            rendered(terminal.backend().buffer()).contains("value-one"),
            "focus above the viewport did not snap its field to the start"
        );
    }
}

#[cfg(test)]
mod path_suggestion_tests {
    use std::{
        collections::BTreeMap,
        path::{Path, PathBuf},
    };

    use skit_application::{
        path_completion::{PathCompletionContext, PathCompletionKind},
        tokens::TokenContext,
    };

    use super::*;

    #[derive(Debug)]
    struct ContextProvider;

    impl PathCompletionProvider for ContextProvider {
        fn complete(&self, request: &PathCompletionRequest) -> Option<String> {
            if request.context.workdir.as_path() == Path::new("/old") {
                thread::sleep(Duration::from_millis(75));
                Some("old.txt".to_owned())
            } else {
                Some("new.txt".to_owned())
            }
        }
    }

    fn request(workdir: &str) -> PathCompletionRequest {
        PathCompletionRequest {
            value: "n".to_owned(),
            kind: PathCompletionKind::Path,
            shlexy: false,
            placeholder_braces: false,
            dialect: PathInputDialect::Posix,
            context: PathCompletionContext {
                workdir: PathBuf::from(workdir),
                tokens: TokenContext {
                    cwd: "/invoke".to_owned(),
                    home: None,
                    env: BTreeMap::new(),
                    today: "2026-08-21".to_owned(),
                    now: "12-00-00".to_owned(),
                },
            },
        }
    }

    fn request_with_value(value: &str) -> PathCompletionRequest {
        PathCompletionRequest {
            value: value.to_owned(),
            ..request("/new")
        }
    }

    fn seeded_visible_session(
        visible: (u64, usize, &str),
        expected: (u64, usize, &str),
    ) -> PathSuggestionSession {
        PathSuggestionSession {
            expected: Some(ExpectedPathSuggestion {
                generation: expected.0,
                field: expected.1,
                request: request_with_value(expected.2),
            }),
            visible: Some(VisiblePathSuggestion {
                generation: visible.0,
                field: visible.1,
                value: visible.2.to_owned(),
                suggestion: format!("{}.txt", visible.2),
            }),
            ..PathSuggestionSession::default()
        }
    }

    #[test]
    fn refresh_rejects_identical_and_unrelated_suggestions() {
        for suggestion in ["n", "old.txt"] {
            let (result_tx, result_rx) = mpsc::channel();
            let mut suggestions = PathSuggestionSession {
                results: Some(result_rx),
                expected: Some(ExpectedPathSuggestion {
                    generation: 7,
                    field: 2,
                    request: request_with_value("n"),
                }),
                in_flight: true,
                ..PathSuggestionSession::default()
            };
            result_tx
                .send(PathSuggestionResult {
                    generation: 7,
                    field: 2,
                    value: "n".to_owned(),
                    suggestion: Some(suggestion.to_owned()),
                })
                .unwrap();

            assert!(suggestions.refresh());
            assert_eq!(suggestions.visible(2, "n"), None, "accepted {suggestion:?}");
        }
    }

    #[test]
    fn refresh_drains_results_that_arrive_after_cancellation() {
        let (request_tx, request_rx) = mpsc::sync_channel(2);
        let (result_tx, result_rx) = mpsc::channel();
        let mut suggestions = PathSuggestionSession {
            requests: Some(request_tx),
            results: Some(result_rx),
            ..PathSuggestionSession::default()
        };

        suggestions.ensure(2, Some(request_with_value("old")));
        let old_job = request_rx.recv().unwrap();
        for suffix in ["txt", "toml"] {
            result_tx
                .send(PathSuggestionResult {
                    generation: old_job.generation,
                    field: old_job.field,
                    value: old_job.request.value.clone(),
                    suggestion: Some(format!("old.{suffix}")),
                })
                .unwrap();
        }

        suggestions.clear();
        assert!(!suggestions.refresh());
        assert_eq!(suggestions.visible(2, "old"), None);
        assert!(matches!(
            suggestions.results.as_ref().unwrap().try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));

        suggestions.ensure(2, Some(request_with_value("new")));
        let new_job = request_rx.recv().unwrap();
        result_tx
            .send(PathSuggestionResult {
                generation: new_job.generation,
                field: new_job.field,
                value: new_job.request.value.clone(),
                suggestion: Some("new.txt".to_owned()),
            })
            .unwrap();

        assert!(suggestions.refresh());
        assert_eq!(suggestions.visible(2, "new"), Some("new.txt"));
    }

    #[test]
    fn visible_suggestion_requires_one_complete_identity() {
        assert_eq!(
            seeded_visible_session((7, 2, "n"), (7, 2, "n")).visible(2, "n"),
            Some("n.txt")
        );
        for (label, visible, expected, requested) in [
            ("visible field", (7, 1, "n"), (7, 2, "n"), (2, "n")),
            ("visible value", (7, 2, "old"), (7, 2, "n"), (2, "n")),
            ("expected generation", (7, 2, "n"), (8, 2, "n"), (2, "n")),
            ("expected field", (7, 2, "n"), (7, 3, "n"), (2, "n")),
            ("expected value", (7, 2, "n"), (7, 2, "old"), (2, "n")),
        ] {
            assert_eq!(
                seeded_visible_session(visible, expected).visible(requested.0, requested.1),
                None,
                "accepted stale {label}"
            );
        }
    }

    #[test]
    fn accepting_or_leaving_the_run_screen_clears_suggestion_state() {
        let mut accepted = seeded_visible_session((7, 2, "n"), (7, 2, "n"));
        assert_eq!(accepted.take(2, "n"), Some("n.txt".to_owned()));
        assert_eq!(accepted.take(2, "n"), None);

        let (_result_tx, result_rx) = mpsc::channel();
        let mut session = TuiSession {
            path_suggestions: PathSuggestionSession {
                results: Some(result_rx),
                retry_pending: true,
                ..seeded_visible_session((7, 2, "n"), (7, 2, "n"))
            },
            ..TuiSession::default()
        };
        session.begin_render(&LibraryState::default());
        assert!(!session.refresh_background());
        assert_eq!(session.path_suggestions.visible(2, "n"), None);
    }

    #[test]
    fn terminal_poll_deadline_tracks_in_flight_and_retry_work() {
        use crate::terminal::{TerminalEventWait, terminal_event_wait};

        let mut session = TuiSession::default();
        assert_eq!(
            terminal_event_wait(session.has_pending_path_completion()),
            TerminalEventWait::Blocking
        );

        session.path_suggestions.in_flight = true;
        assert_eq!(
            terminal_event_wait(session.has_pending_path_completion()),
            TerminalEventWait::Poll(Duration::from_millis(25))
        );

        session.path_suggestions.in_flight = false;
        session.path_suggestions.retry_pending = true;
        assert_eq!(
            terminal_event_wait(session.has_pending_path_completion()),
            TerminalEventWait::Poll(Duration::from_millis(25))
        );
    }

    #[test]
    fn same_value_in_a_new_context_replaces_the_old_pending_request() {
        let mut suggestions = PathSuggestionSession::new(Arc::new(ContextProvider));
        assert!(!suggestions.has_pending_work());
        suggestions.ensure(0, Some(request("/old")));
        assert!(suggestions.has_pending_work());
        suggestions.ensure(0, Some(request("/new")));

        let deadline = Instant::now() + Duration::from_secs(2);
        while suggestions.visible(0, "n") != Some("new.txt") {
            let _ = suggestions.refresh();
            assert!(Instant::now() < deadline, "new context did not complete");
            thread::yield_now();
        }
        assert!(!suggestions.has_pending_work());

        thread::sleep(Duration::from_millis(100));
        let _ = suggestions.refresh();
        assert_eq!(suggestions.visible(0, "n"), Some("new.txt"));
        assert!(!suggestions.has_pending_work());

        suggestions.clear();
        assert!(!suggestions.has_pending_work());
    }

    #[test]
    fn a_full_request_queue_retries_instead_of_publishing_a_stale_expectation() {
        let (requests, held_requests) = mpsc::sync_channel(1);
        requests
            .try_send(PathSuggestionJob {
                generation: 1,
                field: 0,
                request: Box::new(request("/held")),
            })
            .unwrap();
        let (_results, result_rx) = mpsc::channel();
        let mut suggestions = PathSuggestionSession {
            requests: Some(requests),
            results: Some(result_rx),
            ..PathSuggestionSession::default()
        };

        suggestions.ensure(1, Some(request("/new")));

        assert!(suggestions.expected.is_none());
        assert!(!suggestions.in_flight);
        assert!(suggestions.retry_pending);
        assert!(
            suggestions.refresh(),
            "a pending retry must request the render pass that calls ensure again"
        );
        drop(held_requests);
    }

    #[test]
    fn a_poisoned_worker_queue_stops_without_running_the_provider() {
        let (_requests, request_rx) = mpsc::sync_channel(1);
        let requests = Arc::new(Mutex::new(request_rx));
        let poison = Arc::clone(&requests);
        let _ = thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("poison the worker queue");
        })
        .join();
        let (results, result_rx) = mpsc::channel();

        run_path_suggestion_worker(Arc::new(ContextProvider), requests, results);

        assert!(matches!(
            result_rx.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }

    /// The session can drop its result channel while a worker still computes a suggestion. The
    /// worker must then stop instead of looping. This return only ran when a test's teardown
    /// happened to race a computation, so it owns the path deterministically: one queued job, a
    /// receiver that is already gone, and the worker called on this thread — returning IS the
    /// proof, because a worker that ignored the closed channel would wait here forever.
    #[test]
    fn a_worker_stops_when_the_session_no_longer_listens_for_results() {
        let (requests_tx, request_rx) = mpsc::sync_channel(1);
        requests_tx
            .try_send(PathSuggestionJob {
                generation: 1,
                field: 0,
                request: Box::new(request("/new")),
            })
            .unwrap();
        let requests = Arc::new(Mutex::new(request_rx));
        let (results, result_rx) = mpsc::channel();
        drop(result_rx);

        run_path_suggestion_worker(Arc::new(ContextProvider), requests, results);

        drop(requests_tx);
    }

    #[test]
    fn path_dialect_policy_keeps_both_host_shapes_explicit() {
        assert_eq!(path_dialect_for(false), PathInputDialect::Posix);
        assert_eq!(path_dialect_for(true), PathInputDialect::Windows);
        assert_eq!(host_path_dialect(), path_dialect_for(cfg!(windows)));
    }

    #[test]
    fn an_overlay_ignores_events_that_do_not_belong_to_its_picker() {
        let names = (0..=skit_ui::PROMPT_LIST_PREVIEW_LIMIT)
            .map(|index| format!("VALUE_{index}"))
            .collect::<Vec<_>>();
        let view = skit_ui::SettingsView::from_inputs(&skit_ui::SettingsInputs {
            kind: "prompt".to_owned(),
            name: "Prompt".to_owned(),
            supports_modes: true,
            interpolate: true,
            candidates: names,
            ..skit_ui::SettingsInputs::default()
        });
        assert!(view.prompt_picker_available());
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Settings(Box::new(view.clone()))));
        let mut session = TuiSession {
            settings_prompt_overlay: Some((
                PromptCandidatePickerSession::new(view.prompt_picker()),
                ChoicePickerGeometry::default(),
            )),
            ..TuiSession::default()
        };

        assert_eq!(
            session.handle_event(Event::FocusGained, &state, &ViewGeometry::default()),
            EventHandling::Ignored
        );
        assert!(session.settings_prompt_overlay.is_some());
        assert_eq!(
            session.handle_event(Event::Resize(40, 10), &state, &ViewGeometry::default()),
            EventHandling::Ignored
        );
        assert!(session.settings_prompt_overlay.is_some());
    }
}
