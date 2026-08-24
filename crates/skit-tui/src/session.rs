//! Ephemeral state for mature terminal widgets.

use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use ratatui_core::{
    layout::Rect,
    style::{Color, Modifier, Style},
    terminal::Frame,
    text::{Line, Span},
    widgets::Widget,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui_interact::{
    components::{
        Button, ButtonState, ButtonStyle, ButtonVariant, CheckBox, CheckBoxState, CheckBoxStyle,
        ScrollableContentState, Select, SelectAction, SelectState, SelectStyle, Toast, ToastState,
        ToastStyle, handle_scrollable_content_key, handle_scrollable_content_mouse,
        handle_select_key, handle_select_mouse,
    },
    state::FocusManager,
    traits::{ClickRegion, ClickRegionRegistry},
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
    RunTokenError, RunValidationError, Screen, UiCommand,
};
use tui_input::{Input as LineInput, InputRequest, backend::crossterm::EventHandler as _};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    HitRegion, HitTarget, ViewGeometry, command_action,
    footer::FooterSession,
    map_event,
    rowclip::RowClip,
    run_field_command_action,
    screens::add::{AddScreenEvent, AddScreenGeometry, AddScreenSession, render_add},
    screens::library::LibraryScreenSession,
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
    screens::preferences::{PreferencesEventHandling, PreferencesWidgetSession},
    screens::run_modal::{RunModalEvent, RunModalSession},
    screens::settings::{
        SettingsScreenEvent, SettingsScreenGeometry, SettingsScreenSession, render_settings,
    },
    theme::{ACCENT, BOX_DIM, BOX_MAROON, SELECT_BG, SELECT_FG, panel_block},
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
    Help,
    ConfirmRemove,
    ConfirmDiscardChanges,
    RunPresetName,
    RunTokenMenu,
    RunEnvironmentPicker,
    RunFilePicker,
    RunnerEditor(skit_ui::RunnerEditorMode),
    Library { query: &'a str, search: bool },
    Preferences,
    Add,
    Health,
    Runners,
    Report(&'a str),
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
    clicks: ClickRegionRegistry<SessionHit>,
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

#[derive(Debug, Default)]
struct PathSuggestionSession {
    requests: Option<mpsc::SyncSender<PathSuggestionJob>>,
    results: Option<mpsc::Receiver<PathSuggestionResult>>,
    generation: u64,
    expected: Option<(u64, usize, PathCompletionRequest)>,
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
            .is_some_and(|(_, target, expected)| *target == field && expected == &request);
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
                self.expected = Some((generation, field, request));
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
            let is_current = self
                .expected
                .as_ref()
                .is_some_and(|(generation, field, request)| {
                    *generation == result.generation
                        && *field == result.field
                        && request.value == result.value
                });
            if !is_current {
                continue;
            }
            self.visible = result.suggestion.and_then(|suggestion| {
                (suggestion != result.value && suggestion.starts_with(&result.value)).then_some(
                    VisiblePathSuggestion {
                        generation: result.generation,
                        field: result.field,
                        value: result.value,
                        suggestion,
                    },
                )
            });
            self.in_flight = false;
            changed = true;
        }
        changed
    }

    fn visible(&self, field: usize, value: &str) -> Option<&str> {
        self.visible.as_ref().and_then(|visible| {
            (visible.field == field
                && visible.value == value
                && self.expected.as_ref().is_some_and(|expected| {
                    expected.0 == visible.generation
                        && expected.1 == field
                        && expected.2.value == value
                }))
            .then_some(visible.suggestion.as_str())
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
}

#[derive(Clone, Debug)]
enum SessionHit {
    SearchInput,
    Target(HitTarget),
    Checkbox(usize),
    Select(usize),
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
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    row_starts: Vec<usize>,
    row_heights: Vec<usize>,
    select_areas: Vec<Rect>,
    dropdown_regions: Vec<Vec<ClickRegion<SelectAction>>>,
    pending_ensure_focus: bool,
}

#[derive(Clone, Debug)]
struct RunLayout {
    items: Vec<PositionedRunItem>,
    field_starts: Vec<usize>,
    field_heights: Vec<usize>,
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
    clicks: ClickRegionRegistry<usize>,
    focus: FocusManager<usize>,
    scroll: ScrollableContentState,
    viewport: Rect,
    visible_height: usize,
    row_starts: Vec<usize>,
    row_heights: Vec<usize>,
    pending_ensure_focus: bool,
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
        if is_ctrl_c(&event) {
            return self.handle_ctrl_c();
        }
        if !crate::footer::is_suppressed(state)
            && let Event::Mouse(mouse) = &event
            && self.footer.handle_mouse(mouse)
        {
            return EventHandling::Consumed;
        }
        if let Event::Mouse(mouse) = &event
            && matches!(mouse.kind, MouseEventKind::Down(_))
            && matches!(
                self.clicks.handle_click(mouse.column, mouse.row),
                Some(SessionHit::SearchInput)
            )
        {
            return EventHandling::Action(Action::BeginSearch);
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
        if let Some(ModalState::RunnerEditor { view, .. }) = state.modal() {
            return match self.runner_editor.handle_event(event, view) {
                RunnerEditorEventHandling::Action(action) => {
                    EventHandling::Action(Action::RunnerEditor(action))
                }
                RunnerEditorEventHandling::Consumed => EventHandling::Consumed,
                RunnerEditorEventHandling::Ignored => EventHandling::Ignored,
            };
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
            if let Some((session, geometry)) = self.settings_prompt_overlay.as_mut() {
                return match session.handle_event(event, geometry) {
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
                    None => EventHandling::Ignored,
                };
            }
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
                    let handling = self.handle_form_mouse(mouse, geometry);
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
            self.clicks
                .register(hit.rect, SessionHit::Target(hit.action));
        }
    }

    pub(crate) fn render_footer(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        state: &LibraryState,
        locale: Locale,
    ) -> Vec<HitRegion> {
        self.footer.render(frame, area, state, locale)
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
        let library_browse = matches!(&kind, HeaderKind::Library { search: false, .. });
        let title = match kind {
            HeaderKind::Help => text(locale, "Help").into_owned(),
            HeaderKind::ConfirmRemove => text(locale, "Confirm removal").into_owned(),
            HeaderKind::ConfirmDiscardChanges => {
                text(locale, "Discard unsaved changes?").into_owned()
            }
            HeaderKind::RunPresetName => text(locale, "Save as preset").into_owned(),
            HeaderKind::RunTokenMenu => text(locale, "Insert a run-time value").into_owned(),
            HeaderKind::RunEnvironmentPicker => text(locale, "Environment variable").into_owned(),
            HeaderKind::RunFilePicker => text(locale, "Insert a file or folder").into_owned(),
            HeaderKind::RunnerEditor(mode) => match mode {
                skit_ui::RunnerEditorMode::New => text(locale, "New agent (runner)").into_owned(),
                skit_ui::RunnerEditorMode::Edit | skit_ui::RunnerEditorMode::Repair => {
                    text(locale, "Edit agent (runner)").into_owned()
                }
            },
            HeaderKind::Library {
                query,
                search: true,
            } => {
                self.search.sync(query);
                let label = text(locale, "Search");
                if area.height < 3 {
                    render_flat_search_input(frame, area, &self.search.input, &label);
                } else {
                    render_line_input(frame, area, &self.search.input, false, true, &label);
                }
                self.clicks.register(area, SessionHit::SearchInput);
                return;
            }
            HeaderKind::Library {
                query,
                search: false,
            } => format!(
                "{}: {}",
                text(locale, "Library"),
                if query.is_empty() {
                    text(locale, "all entries").into_owned()
                } else {
                    query.to_owned()
                }
            ),
            HeaderKind::Preferences => text(locale, "Preferences").into_owned(),
            HeaderKind::Add => text(locale, "Add").into_owned(),
            HeaderKind::Health => text(locale, "Health").into_owned(),
            HeaderKind::Runners => text(locale, "Agents (prompt runners)").into_owned(),
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
        if library_browse {
            self.clicks.register(area, SessionHit::SearchInput);
        }
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
                } => render_line_input_band(frame, clip, state, *secret, *focused, &label, None),
                FormWidgetControl::TextArea { state, focused, .. } => {
                    render_textarea_band(frame, clip, state, *focused, &label);
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
        self.run.select_areas = vec![Rect::default(); self.run.controls.len()];
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
                RunRenderItem::Control(index) if usize::from(visible.height) == item.height => {
                    self.render_run_control(frame, visible, *index, locale, &mut hits);
                }
                RunRenderItem::Control(_) | RunRenderItem::Spacer => {}
            }
        }
        if layout.height > usize::from(content.height) {
            let mut scrollbar =
                ScrollbarState::new(layout.height.saturating_sub(usize::from(content.height)))
                    .position(self.run.scroll.scroll_offset())
                    .viewport_content_length(usize::from(content.height));
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight).style(run_scrollbar_style()),
                inner,
                &mut scrollbar,
            );
        }
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
        area: Rect,
        index: usize,
        locale: Locale,
        hits: &mut Vec<HitRegion>,
    ) {
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
                render_line_input_with_suggestion(
                    frame, area, state, *secret, *focused, "", suggestion,
                );
                self.clicks
                    .register(area, SessionHit::Target(HitTarget::FocusField(index)));
                hits.push(HitRegion {
                    rect: area,
                    action: HitTarget::FocusField(index),
                });
            }
            WidgetControl::TextArea { state, focused, .. } => {
                render_textarea(frame, area, state, *focused, "");
                self.clicks
                    .register(area, SessionHit::Target(HitTarget::FocusField(index)));
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
                self.clicks
                    .register(region.area, SessionHit::Checkbox(index));
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
                self.run.select_areas[index] = region.area;
                self.clicks.register(region.area, SessionHit::Select(index));
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
                let mut x = area.x;
                let mut y = area.y;
                if area.width > 0 {
                    for (option_label, button) in options.iter().zip(buttons.iter()) {
                        let width = u16::try_from(option_label.width().saturating_add(2))
                            .unwrap_or(u16::MAX)
                            .min(area.width);
                        if x > area.x && x.saturating_add(width) > area.right() {
                            x = area.x;
                            y = y.saturating_add(1);
                        }
                        let option_area = Rect::new(x, y, width, 1);
                        let region = Button::new(option_label, button)
                            .variant(ButtonVariant::Toggle)
                            .style(radio_style())
                            .render_stateful(option_area, frame.buffer_mut());
                        self.clicks.register(
                            region.area,
                            SessionHit::RadioOption {
                                field: index,
                                value: option_label.clone(),
                            },
                        );
                        x = x.saturating_add(width).saturating_add(1);
                    }
                }
                let field_area = area;
                self.clicks
                    .register(field_area, SessionHit::Target(HitTarget::FocusField(index)));
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
            if width > 0 {
                let chip_area = Rect::new(area.x.saturating_add(chip.x), area.y, width, 1);
                let state = ButtonState::enabled();
                let region = Button::new(&chip.label, &state)
                    .variant(ButtonVariant::SingleLine)
                    .style(run_chip_style())
                    .render_stateful(chip_area, frame.buffer_mut());
                self.clicks
                    .register(region.area, SessionHit::Target(chip.target));
                hits.push(HitRegion {
                    rect: region.area,
                    action: chip.target,
                });
            }
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
                let regions = Select::new(options, state)
                    .style(select_style())
                    .render_dropdown(frame, self.run.select_areas[index], screen);
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
                let _ = handle_scrollable_content_key(
                    &mut self.run.scroll,
                    &key,
                    self.run.visible_height,
                );
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
                match edit_textarea(state, key, undo_group, redo_group) {
                    TextAreaEventHandling::Ignored => return EventHandling::Ignored,
                    TextAreaEventHandling::Consumed | TextAreaEventHandling::VerticalBoundary => {}
                }
                let after = textarea_text(state);
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
        geometry: &ViewGeometry,
    ) -> EventHandling {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && handle_scrollable_content_mouse(
            &mut self.run.scroll,
            &mouse,
            self.run.viewport,
            self.run.visible_height,
        )
        .is_some()
        {
            return EventHandling::Consumed;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(_)) {
            return EventHandling::Ignored;
        }

        for index in 0..self.run.controls.len() {
            let WidgetControl::Choice {
                state,
                options,
                presentation: ChoicePresentation::Picker,
                ..
            } = &mut self.run.controls[index]
            else {
                continue;
            };
            if !state.is_open {
                continue;
            }
            if let Some(action) = handle_select_mouse(
                &mouse,
                state,
                self.run.select_areas[index],
                &self.run.dropdown_regions[index],
            ) {
                if let SelectAction::Select(option) = action {
                    return EventHandling::Action(Action::SelectFieldOption {
                        field: index,
                        value: options.get(option).cloned().unwrap_or_default(),
                    });
                }
                if ClickRegion::new(self.run.select_areas[index], ())
                    .contains(mouse.column, mouse.row)
                {
                    return EventHandling::Consumed;
                }
            }
        }

        match self.clicks.handle_click(mouse.column, mouse.row).cloned() {
            None | Some(SessionHit::SearchInput) => {
                let _ = geometry;
                EventHandling::Ignored
            }
            Some(SessionHit::Target(HitTarget::Command(command))) => {
                EventHandling::Action(command_action(command, geometry))
            }
            Some(SessionHit::Target(HitTarget::RunFieldCommand { field, command })) => {
                EventHandling::Action(run_field_command_action(field, command))
            }
            Some(SessionHit::Target(HitTarget::FocusField(index))) => {
                EventHandling::Action(Action::FocusField(index))
            }
            Some(SessionHit::Checkbox(index)) => EventHandling::Action(Action::ToggleField(index)),
            Some(SessionHit::Select(index)) => {
                if let Some(WidgetControl::Choice { state, .. }) = self.run.controls.get_mut(index)
                {
                    state.open();
                }
                if form.focused() == index {
                    EventHandling::Consumed
                } else {
                    EventHandling::Action(Action::FocusField(index))
                }
            }
            Some(SessionHit::RadioOption { field, value }) => {
                EventHandling::Action(Action::SelectFieldOption { field, value })
            }
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
                            let _ = handle_scrollable_content_key(
                                &mut self.form.scroll,
                                &key,
                                self.form.visible_height,
                            );
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
                match edit_textarea(state, key, undo_group, redo_group) {
                    TextAreaEventHandling::Ignored => return EventHandling::Ignored,
                    TextAreaEventHandling::Consumed | TextAreaEventHandling::VerticalBoundary => {}
                }
                let after = textarea_text(state);
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

    fn handle_form_mouse(&mut self, mouse: MouseEvent, geometry: &ViewGeometry) -> EventHandling {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) && handle_scrollable_content_mouse(
            &mut self.form.scroll,
            &mouse,
            self.form.viewport,
            self.form.visible_height,
        )
        .is_some()
        {
            return EventHandling::Consumed;
        }
        if !matches!(mouse.kind, MouseEventKind::Down(_)) {
            return EventHandling::Ignored;
        }
        let Some(index) = self
            .form
            .clicks
            .handle_click(mouse.column, mouse.row)
            .copied()
        else {
            let _ = geometry;
            return EventHandling::Ignored;
        };
        EventHandling::Action(Action::FocusField(index))
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
            self.focus.clear();
            self.focus.register_all(0..self.controls.len());
            self.scroll = ScrollableContentState::empty();
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
        self.row_starts.clone_from(&layout.field_starts);
        self.row_heights.clone_from(&layout.field_heights);
        self.scroll.set_lines(vec![String::new(); layout.height]);
        let maximum = layout.height.saturating_sub(self.visible_height);
        if self.scroll.scroll_offset() > maximum {
            self.scroll.set_scroll_offset(maximum);
        }
        if self.pending_ensure_focus
            && let (Some(start), Some(height)) =
                (self.row_starts.get(focused), self.row_heights.get(focused))
        {
            let offset = self.scroll.scroll_offset();
            let end = start.saturating_add(*height);
            if *start < offset {
                self.scroll.set_scroll_offset(*start);
            } else if end > offset.saturating_add(self.visible_height) {
                self.scroll
                    .set_scroll_offset(end.saturating_sub(self.visible_height));
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
            self.focus.clear();
            self.focus.register_all(0..self.controls.len());
            self.scroll = ScrollableContentState::empty();
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
        self.scroll.set_lines(vec![String::new(); next]);
        if self.pending_ensure_focus
            && let (Some(start), Some(height)) = (
                self.row_starts.get(form.focused),
                self.row_heights.get(form.focused),
            )
        {
            let offset = self.scroll.scroll_offset();
            let end = start.saturating_add(*height);
            if *start < offset {
                self.scroll.set_scroll_offset(*start);
            } else if end > offset.saturating_add(self.visible_height) {
                self.scroll
                    .set_scroll_offset(end.saturating_sub(self.visible_height));
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
            u16::try_from(clipped_start.saturating_sub(start))
                .expect("the form band offset fits Ratatui geometry"),
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
            .contains(KeyModifiers::CONTROL | KeyModifiers::SHIFT))
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
    if form.has_parameters() && form.preset_names().len() == 0 {
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

    let mut field_starts = Vec::with_capacity(form.fields().len());
    let mut field_heights = Vec::with_capacity(form.fields().len());
    for (index, field) in form.fields().iter().enumerate() {
        let field_start = start;
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
        push_run_item(
            &mut items,
            &mut start,
            RunRenderItem::Control(index),
            run_control_height(field, width),
        );
        for note in run_field_notes(field, locale) {
            push_run_copy(&mut items, &mut start, note, width);
        }
        if index + 1 < form.fields().len() {
            push_run_item(&mut items, &mut start, RunRenderItem::Spacer, 1);
        }
        field_starts.push(field_start);
        field_heights.push(start.saturating_sub(field_start));
    }
    RunLayout {
        items,
        field_starts,
        field_heights,
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
                command: UiCommand::BrowsePath,
            },
        ));
    }
    if form.can_insert_field(index) {
        chips.push((
            format!("▾ {}", text(locale, "insert")),
            HitTarget::RunFieldCommand {
                field: index,
                command: UiCommand::InsertValue,
            },
        ));
    }
    if field.resettable() {
        chips.push((
            format!("↺ {}", text(locale, "default")),
            HitTarget::RunFieldCommand {
                field: index,
                command: UiCommand::ResetDefault,
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
        if x > 0 && x.saturating_add(wanted) > available && !(row.is_empty() && rows.is_empty()) {
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
        if x > 0 && x.saturating_add(wanted) > available {
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
) {
    render_line_input_band(
        frame,
        RowClip::new(3, 0, area),
        state,
        secret,
        focused,
        label,
        None,
    );
}

fn render_line_input_with_suggestion(
    frame: &mut Frame,
    area: Rect,
    state: &LineInput,
    secret: bool,
    focused: bool,
    label: &str,
    suggestion: Option<&str>,
) {
    render_line_input_band(
        frame,
        RowClip::new(3, 0, area),
        state,
        secret,
        focused,
        label,
        suggestion,
    );
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
) {
    let border = if focused { ACCENT } else { BOX_DIM };
    let width = usize::from(clip.area().width.saturating_sub(2).max(1));
    let scroll = state.visual_scroll(width);
    let display = if secret {
        Line::from(Span::styled(
            "•".repeat(state.value().chars().count()),
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
        let visual_cursor = if secret {
            state.cursor()
        } else {
            state.visual_cursor()
        };
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
}

fn render_flat_search_input(frame: &mut Frame, area: Rect, state: &LineInput, label: &str) {
    let content = Rect::new(
        area.x.saturating_add(1),
        area.y,
        area.width.saturating_sub(2),
        area.height.min(1),
    );
    let width = usize::from(content.width.max(1));
    let scroll = state.visual_scroll(width);
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
    if content.width > 0 && content.height > 0 {
        let x = state
            .visual_cursor()
            .saturating_sub(scroll)
            .min(width.saturating_sub(1));
        frame.set_cursor_position((
            content
                .x
                .saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
            content.y,
        ));
    }
}

pub(crate) fn render_textarea(
    frame: &mut Frame,
    area: Rect,
    state: &mut RichTextArea<'static>,
    focused: bool,
    label: &str,
) {
    render_textarea_band(frame, RowClip::new(6, 0, area), state, focused, label);
}

/// Draw only the visible band of one bordered text area.
pub(crate) fn render_textarea_band(
    frame: &mut Frame,
    clip: RowClip,
    state: &mut RichTextArea<'static>,
    focused: bool,
    label: &str,
) {
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
    if clip.is_full() {
        (&*state).render(clip.area(), frame.buffer_mut());
    } else {
        clip.paint_bounded_stateful_editor(frame.buffer_mut(), &*state);
    }
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
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(24, 4)).unwrap();
        terminal
            .draw(|frame| {
                session.render_form(frame, frame.area(), &form, Locale::En);
            })
            .unwrap();
        // A failed destructure skips the cursor moves, and the style
        // assertions below then fail: no dead refusal arm is needed.
        if let FormWidgetControl::TextArea { state, .. } = &mut session.form.controls[0] {
            state.move_cursor(CursorMove::Jump(1, 0));
            state.start_selection();
            state.move_cursor(CursorMove::Bottom);
            state.move_cursor(CursorMove::End);
        }
        // Source row 0 is the top border and source row 1 is `first`.
        session.form.scroll.set_scroll_offset(2);

        let mut geometry = ViewGeometry::default();
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
    }
}
