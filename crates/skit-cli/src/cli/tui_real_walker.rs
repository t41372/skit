//! Generic walker smoke coverage over the real production TUI host.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use ratatui_core::{
    backend::TestBackend,
    layout::{Rect, Size},
    terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde_json::{Value, json};
use skit_application::{AgentScope, CreateEntry, EntryPayload, SourcePermissions};
use skit_domain::{EntryKind, EntrySettings, StorageMode, parameters::ParamDecl};
use skit_i18n::{Locale, detect_locale};
use skit_store::PromptRunner;
use skit_tui::{
    AddControlId, AddTextField, EventHandling, HitTarget, LocalActionOutcome, LocalActionTarget,
    LocalAdvertisedAction, RunFieldCommand, ScreenTarget, ScreenTargetError, ScreenTargetInventory,
    TuiSession, ViewGeometry, map_event, render_with_session,
};
use skit_tui_walker_model::{invariants, model::LiveInventory, parity};
use skit_tui_walker_support::{
    EventChainIdentity, LivenessResult, ObjectKind, ObjectRef, Presentation, RectSnapshot,
    StyledFrameSnapshot, TimelineBoundary, TimelinePhase, TimelineRow, TransitionCause,
    asciicast::{AsciicastRecorder, FRAME_INTERVAL},
    build_object, bundle,
    engine::{
        CheckpointCapture, CheckpointCauseProjection, CheckpointFor, CheckpointSink,
        DispatchOutcome, EffectRoute, EngineBoundary, EngineCause, EngineCheckpoint, EnginePhase,
        FrontendAdapter, HostAdapter, OperationResolution, ReplayFactory, WalkerEngine,
        replay_prefix_with_sink,
    },
    expected_presentation,
    sandbox::{SafeProfileId, SandboxMetadata, SandboxMode, SandboxPlatform},
    timeline_row_digest, validate_asciicast_v3, validate_object, validate_styled_frame,
    validate_timeline, validate_timeline_semantics,
};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use skit_tui_walker_support::{
    canonical_json_bytes,
    sandbox::{SANDBOX_MARKER_FILE, STABLE_SANDBOX_NAMESPACE, profile_sandbox_path},
    sha256_hex,
};
use skit_ui::{
    Action, CommandContext, Effect, HealthAction, HostRequest, LibraryState, PreferencesControlId,
    RunnerEditorAction, RunnerManagerAction, Screen, UiBinding, UiCommand, UiKey, command_specs,
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use super::tui_real_host::ArtifactLeakOracleFact;
use super::tui_real_host::{
    HostObservation, LeakOracleFacts, RealWalkerHost, SandboxError, StableHostError,
    StableHostPrimaryError, StableSandboxNamespace, WalkerDirectoryRoot, WalkerDirectorySeed,
    WalkerExternalReferenceSeed, WalkerExternalSeed, WalkerFilePickerTree, WalkerFormSeed,
    WalkerLastRunSeed, WalkerSeedSpec,
};

const EFFECT_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SmokeOperation {
    OpenRun,
    OpenPreferences,
    OpenLanguagePicker,
    NextLanguage,
    ChooseLanguage,
    SavePreferences,
    FinalLiveness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocaleSmokeTarget {
    SimplifiedChinese,
    TraditionalChinese,
}

impl LocaleSmokeTarget {
    const fn tag(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::TraditionalChinese => "zh-TW",
        }
    }

    const fn next_count(self) -> usize {
        match self {
            Self::SimplifiedChinese => 2,
            Self::TraditionalChinese => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SmokeResolution {
    event: SmokeEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SmokeEvent {
    key: KeyCode,
    modifiers: KeyModifiers,
}

impl SmokeEvent {
    fn terminal_event(self) -> Event {
        Event::Key(KeyEvent::new(self.key, self.modifiers))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SmokeHandling {
    Consumed,
    Ignored,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CorpusModifiers {
    control: bool,
    alt: bool,
    shift: bool,
}

impl CorpusModifiers {
    const NONE: Self = Self {
        control: false,
        alt: false,
        shift: false,
    };

    fn terminal(self) -> KeyModifiers {
        let mut modifiers = KeyModifiers::NONE;
        modifiers.set(KeyModifiers::CONTROL, self.control);
        modifiers.set(KeyModifiers::ALT, self.alt);
        modifiers.set(KeyModifiers::SHIFT, self.shift);
        modifiers
    }

    fn from_terminal(modifiers: KeyModifiers) -> Self {
        Self {
            control: modifiers.contains(KeyModifiers::CONTROL),
            alt: modifiers.contains(KeyModifiers::ALT),
            shift: modifiers.contains(KeyModifiers::SHIFT),
        }
    }

    fn value(self) -> Value {
        json!({
            "alt": self.alt,
            "control": self.control,
            "shift": self.shift,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorpusKey {
    Character(char),
    Enter,
    Escape,
    Delete,
    Backspace,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Function(u8),
}

impl CorpusKey {
    const fn terminal(self) -> KeyCode {
        match self {
            Self::Character(character) => KeyCode::Char(character),
            Self::Enter => KeyCode::Enter,
            Self::Escape => KeyCode::Esc,
            Self::Delete => KeyCode::Delete,
            Self::Backspace => KeyCode::Backspace,
            Self::Tab => KeyCode::Tab,
            Self::BackTab => KeyCode::BackTab,
            Self::Up => KeyCode::Up,
            Self::Down => KeyCode::Down,
            Self::Left => KeyCode::Left,
            Self::Right => KeyCode::Right,
            Self::PageUp => KeyCode::PageUp,
            Self::PageDown => KeyCode::PageDown,
            Self::Home => KeyCode::Home,
            Self::End => KeyCode::End,
            Self::Function(number) => KeyCode::F(number),
        }
    }

    fn from_terminal(code: KeyCode) -> Option<Self> {
        match code {
            KeyCode::Char(character) => Some(Self::Character(character)),
            KeyCode::Enter => Some(Self::Enter),
            KeyCode::Esc => Some(Self::Escape),
            KeyCode::Delete => Some(Self::Delete),
            KeyCode::Backspace => Some(Self::Backspace),
            KeyCode::Tab => Some(Self::Tab),
            KeyCode::BackTab => Some(Self::BackTab),
            KeyCode::Up => Some(Self::Up),
            KeyCode::Down => Some(Self::Down),
            KeyCode::Left => Some(Self::Left),
            KeyCode::Right => Some(Self::Right),
            KeyCode::PageUp => Some(Self::PageUp),
            KeyCode::PageDown => Some(Self::PageDown),
            KeyCode::Home => Some(Self::Home),
            KeyCode::End => Some(Self::End),
            KeyCode::F(number) => Some(Self::Function(number)),
            KeyCode::Insert
            | KeyCode::Null
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
            | KeyCode::KeypadBegin
            | KeyCode::Media(_)
            | KeyCode::Modifier(_) => None,
        }
    }

    fn from_ui(key: UiKey) -> Self {
        match key {
            UiKey::Character(character) => Self::Character(character),
            UiKey::Enter => Self::Enter,
            UiKey::Escape => Self::Escape,
            UiKey::Delete => Self::Delete,
            UiKey::Backspace => Self::Backspace,
            UiKey::Tab => Self::Tab,
            UiKey::BackTab => Self::BackTab,
            UiKey::Up => Self::Up,
            UiKey::Down => Self::Down,
            UiKey::PageUp => Self::PageUp,
            UiKey::PageDown => Self::PageDown,
            UiKey::Home => Self::Home,
            UiKey::End => Self::End,
            UiKey::Function(number) => Self::Function(number),
        }
    }

    fn value(self) -> Value {
        match self {
            Self::Character(character) => json!({"character": character}),
            Self::Enter => json!("enter"),
            Self::Escape => json!("escape"),
            Self::Delete => json!("delete"),
            Self::Backspace => json!("backspace"),
            Self::Tab => json!("tab"),
            Self::BackTab => json!("back_tab"),
            Self::Up => json!("up"),
            Self::Down => json!("down"),
            Self::Left => json!("left"),
            Self::Right => json!("right"),
            Self::PageUp => json!("page_up"),
            Self::PageDown => json!("page_down"),
            Self::Home => json!("home"),
            Self::End => json!("end"),
            Self::Function(number) => json!({"function": number}),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorpusKeyKind {
    Press,
    Repeat,
    Release,
}

impl CorpusKeyKind {
    const fn terminal(self) -> KeyEventKind {
        match self {
            Self::Press => KeyEventKind::Press,
            Self::Repeat => KeyEventKind::Repeat,
            Self::Release => KeyEventKind::Release,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Repeat => "repeat",
            Self::Release => "release",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CorpusKeyEvent {
    code: CorpusKey,
    modifiers: CorpusModifiers,
    kind: CorpusKeyKind,
}

impl CorpusKeyEvent {
    const fn press(code: CorpusKey, modifiers: CorpusModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: CorpusKeyKind::Press,
        }
    }

    pub(super) fn from_terminal(event: KeyEvent) -> Option<Self> {
        let kind = match event.kind {
            KeyEventKind::Press => CorpusKeyKind::Press,
            KeyEventKind::Repeat => CorpusKeyKind::Repeat,
            KeyEventKind::Release => CorpusKeyKind::Release,
        };
        Some(Self {
            code: CorpusKey::from_terminal(event.code)?,
            modifiers: CorpusModifiers::from_terminal(event.modifiers),
            kind,
        })
    }

    fn terminal(self) -> KeyEvent {
        KeyEvent::new_with_kind(
            self.code.terminal(),
            self.modifiers.terminal(),
            self.kind.terminal(),
        )
    }

    fn value(self) -> Value {
        json!({
            "code": self.code.value(),
            "kind": self.kind.label(),
            "modifiers": self.modifiers.value(),
        })
    }
}

/// One pointer event kind that the corpus sends.
///
/// The vocabulary names the primary button, so one term covers the semantic click and the raw
/// pointer operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CorpusMouseKind {
    PrimaryDown,
    SecondaryDown,
    MiddleDown,
    PrimaryUp,
    PrimaryDrag,
    Move,
    ScrollUp,
    ScrollDown,
}

impl CorpusMouseKind {
    pub(super) const ALL: &'static [Self] = &[
        Self::PrimaryDown,
        Self::SecondaryDown,
        Self::MiddleDown,
        Self::PrimaryUp,
        Self::PrimaryDrag,
        Self::Move,
        Self::ScrollUp,
        Self::ScrollDown,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::PrimaryDown => "primary_down",
            Self::SecondaryDown => "secondary_down",
            Self::MiddleDown => "middle_down",
            Self::PrimaryUp => "primary_up",
            Self::PrimaryDrag => "primary_drag",
            Self::Move => "move",
            Self::ScrollUp => "scroll_up",
            Self::ScrollDown => "scroll_down",
        }
    }

    const fn terminal(self) -> MouseEventKind {
        match self {
            Self::PrimaryDown => MouseEventKind::Down(MouseButton::Left),
            Self::SecondaryDown => MouseEventKind::Down(MouseButton::Right),
            Self::MiddleDown => MouseEventKind::Down(MouseButton::Middle),
            Self::PrimaryUp => MouseEventKind::Up(MouseButton::Left),
            Self::PrimaryDrag => MouseEventKind::Drag(MouseButton::Left),
            Self::Move => MouseEventKind::Moved,
            Self::ScrollUp => MouseEventKind::ScrollUp,
            Self::ScrollDown => MouseEventKind::ScrollDown,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CorpusEvent {
    Key(CorpusKeyEvent),
    Mouse {
        column: u16,
        row: u16,
        kind: CorpusMouseKind,
    },
    Paste(String),
    Focus {
        gained: bool,
    },
    Resize {
        width: u16,
        height: u16,
    },
}

impl CorpusEvent {
    const fn mouse(column: u16, row: u16, kind: CorpusMouseKind) -> Self {
        Self::Mouse { column, row, kind }
    }

    fn terminal(&self) -> Event {
        match self {
            Self::Key(event) => Event::Key(event.terminal()),
            Self::Mouse { column, row, kind } => Event::Mouse(MouseEvent {
                kind: kind.terminal(),
                column: *column,
                row: *row,
                modifiers: KeyModifiers::NONE,
            }),
            Self::Paste(value) => Event::Paste(value.clone()),
            Self::Focus { gained: true } => Event::FocusGained,
            Self::Focus { gained: false } => Event::FocusLost,
            Self::Resize { width, height } => Event::Resize(*width, *height),
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Key(event) => json!({"type": "key", "key": event.value()}),
            Self::Mouse { column, row, kind } => json!({
                "type": "mouse",
                "kind": kind.label(),
                "column": column,
                "row": row,
                "modifiers": CorpusModifiers::NONE.value(),
            }),
            Self::Paste(value) => json!({"type": "paste", "value": value}),
            Self::Focus { gained } => json!({"type": "focus", "gained": gained}),
            Self::Resize { width, height } => {
                json!({"type": "resize", "width": width, "height": height})
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CorpusOperation {
    CommandKeyboard(UiCommand),
    HitTarget(HitTarget),
    ScreenFocus(ScreenTarget),
    ScreenHit(ScreenTarget),
    LocalKeyboard {
        target: LocalActionTarget,
        key: CorpusKeyEvent,
    },
    LocalHit(LocalActionTarget),
    RawKey(CorpusKeyEvent),
    RawMouse {
        column: u16,
        row: u16,
        kind: CorpusMouseKind,
    },
    Paste(String),
    Focus {
        gained: bool,
    },
    Resize {
        width: u16,
        height: u16,
    },
    FinalLiveness,
}

const fn corpus_resize_size(width: u16, height: u16) -> Option<Size> {
    if width == 0 || height == 0 {
        None
    } else {
        Some(Size::new(width, height))
    }
}

pub(super) fn corpus_canvas_size(initial: Size, operations: &[CorpusOperation]) -> Size {
    let (width, height) = operations
        .iter()
        .filter_map(|operation| match operation {
            CorpusOperation::Resize { width, height } => {
                corpus_resize_size(*width, *height).map(|size| (size.width, size.height))
            }
            CorpusOperation::CommandKeyboard(_)
            | CorpusOperation::HitTarget(_)
            | CorpusOperation::ScreenFocus(_)
            | CorpusOperation::ScreenHit(_)
            | CorpusOperation::LocalKeyboard { .. }
            | CorpusOperation::LocalHit(_)
            | CorpusOperation::RawKey(_)
            | CorpusOperation::RawMouse { .. }
            | CorpusOperation::Paste(_)
            | CorpusOperation::Focus { .. }
            | CorpusOperation::FinalLiveness => None,
        })
        .fold((initial.width, initial.height), bundle::maximum_canvas);
    Size::new(width, height)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorpusInputKind {
    NotApplicable,
    Event,
    EventChain,
    PrimaryClick,
    LocalEvent,
    LocalPrimaryClick,
    Resize,
}

impl CorpusInputKind {
    const fn label(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::Event => "event",
            Self::EventChain => "event_chain",
            Self::PrimaryClick => "primary_click",
            Self::LocalEvent => "local_event",
            Self::LocalPrimaryClick => "local_primary_click",
            Self::Resize => "resize",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CorpusNotApplicable {
    Unavailable,
    Clipped,
    Ambiguous,
    AlreadyFocused,
    NotKeyboardFocusable,
    InvalidViewport,
    QuitFiltered,
    EventWouldQuit,
}

impl CorpusNotApplicable {
    const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Clipped => "clipped",
            Self::Ambiguous => "ambiguous",
            Self::AlreadyFocused => "already_focused",
            Self::NotKeyboardFocusable => "not_keyboard_focusable",
            Self::InvalidViewport => "invalid_viewport",
            Self::QuitFiltered => "quit_filtered",
            Self::EventWouldQuit => "event_would_quit",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum CorpusSemanticTarget {
    Requested(CorpusOperation),
    Command {
        context: CommandContext,
        command: UiCommand,
        binding: UiBinding,
    },
    Hit {
        target: HitTarget,
        rect: Rect,
    },
    Local {
        target: LocalActionTarget,
        keys: Vec<CorpusKeyEvent>,
        hit: Option<Rect>,
        outcome: LocalActionOutcome,
    },
    ScreenFocus {
        target: ScreenTarget,
        current: ScreenTarget,
        order: Vec<ScreenTarget>,
    },
    ScreenHit {
        target: ScreenTarget,
        rect: Rect,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct CorpusResolution {
    input: CorpusInputKind,
    events: Vec<CorpusEvent>,
    target: CorpusSemanticTarget,
    refusal: Option<CorpusNotApplicable>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrontendObservation {
    state: LibraryState,
    session: Value,
    styled_frame: StyledFrameSnapshot,
    geometry: Value,
    locale: Locale,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct FrontendParity {
    state: LibraryState,
    session: Value,
    locale: Locale,
}

/// Return the geometry that the parity probe reads.
///
/// The live walk reads the geometry of the frame it just drew. A contract replaces this reader to
/// prove that a broken geometry poisons the engine.
fn live_probe_geometry(geometry: &ViewGeometry) -> ViewGeometry {
    geometry.clone()
}

// `SchemaThreeSink` is visible to the random walk, and it records the smoke checkpoint of this
// frontend. The `private_interfaces` lint needs the same visibility here.
pub(super) struct RealFrontend {
    state: LibraryState,
    locale: Locale,
    session: TuiSession,
    terminal: Terminal<TestBackend>,
    geometry: ViewGeometry,
    probe_geometry: fn(&ViewGeometry) -> ViewGeometry,
}

impl RealFrontend {
    fn new(state: LibraryState, locale: Locale, size: Size) -> Result<Self, String> {
        if size.width == 0 || size.height == 0 {
            return Err("the real walker smoke viewport must be positive".to_owned());
        }
        Ok(Self {
            state,
            locale,
            session: TuiSession::default(),
            terminal: Terminal::new(TestBackend::new(size.width, size.height))
                .map_err(|error| error.to_string())?,
            geometry: ViewGeometry::default(),
            probe_geometry: live_probe_geometry,
        })
    }

    fn session_value(&self) -> Result<Value, String> {
        serde_json::to_value(
            self.session
                .agent_review_snapshot()
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())
    }

    fn geometry_value(geometry: &ViewGeometry) -> Value {
        json!({
            "rows": RectSnapshot::from(geometry.rows),
            "first_visible": geometry.first_visible,
            "hits": geometry.hits.iter().map(|hit| json!({
                "rect": RectSnapshot::from(hit.rect),
                "action": format!("{:?}", hit.action),
            })).collect::<Vec<_>>(),
            "detail_pane_visible": geometry.detail_pane_visible,
        })
    }

    fn dispatch_terminal_event(
        &mut self,
        event: Event,
    ) -> Result<DispatchOutcome<Action, SmokeHandling>, String> {
        let handling = self
            .session
            .handle_event(event, &self.state, &self.geometry);
        Ok(match handling {
            EventHandling::Action(action) => DispatchOutcome::Action(action),
            EventHandling::Consumed => DispatchOutcome::Session(SmokeHandling::Consumed),
            EventHandling::Ignored => DispatchOutcome::Session(SmokeHandling::Ignored),
        })
    }

    fn reduce_action(&mut self, action: Action) -> Effect {
        if let Action::PreferencesSaved { locale, .. } = &action {
            self.locale = detect_locale(Some(locale));
        }
        self.state.update(action)
    }

    const fn route_effect(effect: &Effect) -> EffectRoute {
        match effect {
            Effect::None => EffectRoute::Settled,
            Effect::Quit => EffectRoute::Quit,
            _ => EffectRoute::Host,
        }
    }

    fn observe_frontend(&mut self) -> Result<FrontendObservation, String> {
        // The legacy walker reads the invariants of the state that produced the frame, and it
        // reports the violation after the frame exists. The failing screen then stays available
        // for the artifact beside the named rule.
        let invariant = invariants::check_state(&self.state);
        let mut geometry = ViewGeometry::default();
        let completed = self
            .terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &self.state, self.locale, &mut self.session);
            })
            .map_err(|error| error.to_string())?;
        self.geometry = geometry;
        let buffer = completed.buffer.clone();
        #[allow(deprecated)]
        let styled_frame = StyledFrameSnapshot::from_buffer(
            &buffer,
            self.terminal.backend().cursor_position(),
            self.terminal.backend().cursor_visible(),
        );
        validate_styled_frame(&styled_frame).map_err(|error| error.to_string())?;
        invariant.map_err(|violation| format!("the walker state broke one rule: {violation}"))?;
        parity::check_public_hit_parity(
            &self.state,
            &(self.probe_geometry)(&self.geometry),
            buffer.area.as_size(),
            &self.session,
            self.locale,
        )
        .map_err(|violation| format!("the walker frame broke one rule: {violation}"))?;
        Ok(FrontendObservation {
            state: self.state.clone(),
            session: self.session_value()?,
            styled_frame,
            geometry: Self::geometry_value(&self.geometry),
            locale: self.locale,
        })
    }

    fn frontend_parity(&self) -> Result<FrontendParity, String> {
        Ok(FrontendParity {
            state: self.state.clone(),
            session: self.session_value()?,
            locale: self.locale,
        })
    }
}

impl FrontendAdapter for RealFrontend {
    type Operation = SmokeOperation;
    type Resolution = SmokeResolution;
    type Event = SmokeEvent;
    type Action = Action;
    type Effect = Effect;
    type Handling = SmokeHandling;
    type Observation = FrontendObservation;
    type ParitySnapshot = FrontendParity;

    fn resolve(
        &self,
        operation: &Self::Operation,
    ) -> Result<OperationResolution<Self::Event, Self::Resolution>, String> {
        let event = match operation {
            SmokeOperation::OpenRun => SmokeEvent {
                key: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::OpenPreferences => SmokeEvent {
                key: KeyCode::Char(','),
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::OpenLanguagePicker | SmokeOperation::ChooseLanguage => SmokeEvent {
                key: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::NextLanguage => SmokeEvent {
                key: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
            },
            SmokeOperation::SavePreferences => SmokeEvent {
                key: KeyCode::Char('s'),
                modifiers: KeyModifiers::CONTROL,
            },
            SmokeOperation::FinalLiveness => SmokeEvent {
                key: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
            },
        };
        Ok(OperationResolution::Events {
            resolved: SmokeResolution { event },
            events: vec![event],
        })
    }

    fn dispatch(
        &mut self,
        event: Self::Event,
    ) -> Result<DispatchOutcome<Self::Action, Self::Handling>, String> {
        self.dispatch_terminal_event(event.terminal_event())
    }

    fn reduce(&mut self, action: Self::Action) -> Result<Self::Effect, String> {
        Ok(self.reduce_action(action))
    }

    fn effect_route(&self, effect: &Self::Effect) -> EffectRoute {
        Self::route_effect(effect)
    }

    fn observe(&mut self) -> Result<Self::Observation, String> {
        self.observe_frontend()
    }

    fn parity_snapshot(&self) -> Result<Self::ParitySnapshot, String> {
        self.frontend_parity()
    }
}

pub(super) struct CorpusFrontend {
    inner: RealFrontend,
}

impl CorpusFrontend {
    fn new(
        state: LibraryState,
        locale: Locale,
        size: Size,
        file_picker_tree: WalkerFilePickerTree,
    ) -> Result<Self, String> {
        let inner = RealFrontend::new(state, locale, size)?;
        Ok(Self::from_real(inner, file_picker_tree))
    }

    /// Give one real frontend the deterministic file-picker tree of its host.
    fn from_real(mut inner: RealFrontend, file_picker_tree: WalkerFilePickerTree) -> Self {
        inner.session = TuiSession::with_file_picker_tree(
            file_picker_tree.root,
            file_picker_tree.directories,
            file_picker_tree.files,
        );
        Self { inner }
    }

    fn resize(&mut self, width: u16, height: u16) -> Result<(), String> {
        self.inner.terminal.backend_mut().resize(width, height);
        self.inner
            .terminal
            .resize(Rect::new(0, 0, width, height))
            .map_err(|error| error.to_string())
    }

    /// Collect everything the current frame offers to the random operation binder.
    pub(super) fn live_inventory(&self) -> Result<LiveInventory, String> {
        let screen_targets = self
            .inner
            .session
            .screen_target_inventory(&self.inner.state)
            .map_err(screen_target_error_message)?;
        Ok(LiveInventory::new(
            &self.inner.state,
            &self.inner.geometry,
            self.inner.session.local_action_inventory(),
            Some(&screen_targets),
            self.inner.terminal.backend().buffer().area.as_size(),
        ))
    }
}

fn corpus_run_field_command_value(command: RunFieldCommand) -> Value {
    json!(match command {
        RunFieldCommand::BrowsePath => "browse_path",
        RunFieldCommand::InsertValue => "insert_value",
        RunFieldCommand::ResetDefault => "reset_default",
    })
}

fn corpus_hit_target_value(target: HitTarget) -> Value {
    match target {
        HitTarget::Command(command) => json!({"command": command}),
        HitTarget::RunFieldCommand { field, command } => json!({
            "run_field_command": {
                "field": field,
                "command": corpus_run_field_command_value(command),
            },
        }),
        HitTarget::FocusField(field) => json!({"focus_field": field}),
        HitTarget::ToggleField(field) => json!({"toggle_field": field}),
        HitTarget::SelectFieldOption { field, option } => json!({
            "select_field_option": {"field": field, "option": option},
        }),
    }
}

fn corpus_local_target_value(target: &LocalActionTarget) -> Value {
    match target {
        LocalActionTarget::Add(target) => json!({"add": target}),
        LocalActionTarget::Health(action) => json!({"health": action}),
        LocalActionTarget::Runners(action) => json!({"runners": action}),
        LocalActionTarget::RunnerEditor(action) => json!({"runner_editor": action}),
    }
}

fn corpus_add_text_field_value(field: AddTextField) -> Value {
    json!(match field {
        AddTextField::SourcePath => "source_path",
        AddTextField::CommandTemplate => "command_template",
        AddTextField::CommandName => "command_name",
        AddTextField::CommandDescription => "command_description",
        AddTextField::ReviewName => "review_name",
        AddTextField::ReviewDescription => "review_description",
        AddTextField::Dependencies => "dependencies",
        AddTextField::PythonConstraint => "python_constraint",
    })
}

fn corpus_add_control_value(target: &AddControlId) -> Value {
    match target {
        AddControlId::Text(field) => json!({"text": corpus_add_text_field_value(*field)}),
        AddControlId::BrowseSource => json!("browse_source"),
        AddControlId::Draft(index) => json!({"draft": index}),
        AddControlId::NewScript => json!("new_script"),
        AddControlId::NewPrompt => json!("new_prompt"),
        AddControlId::DeleteDraft => json!("delete_draft"),
        AddControlId::Continue => json!("continue"),
        AddControlId::Kind(index) => json!({"kind": index}),
        AddControlId::PickFocusedKind => json!("pick_focused_kind"),
        AddControlId::Storage => json!("storage"),
        AddControlId::StorageOption(index) => json!({"storage_option": index}),
        AddControlId::Candidate(name) => json!({"candidate": name}),
        AddControlId::Interpolate => json!("interpolate"),
        AddControlId::PromptCandidate(name) => json!({"prompt_candidate": name}),
        AddControlId::Runner => json!("runner"),
        AddControlId::RunnerOption(index) => json!({"runner_option": index}),
        AddControlId::NewRunner => json!("new_runner"),
        AddControlId::EditSource => json!("edit_source"),
        AddControlId::Save => json!("save"),
        AddControlId::ToggleFocused => json!("toggle_focused"),
        AddControlId::NextField => json!("next_field"),
        AddControlId::PreviousField => json!("previous_field"),
        AddControlId::Cancel => json!("cancel"),
    }
}

fn corpus_preferences_control_value(target: PreferencesControlId) -> Value {
    json!(match target {
        PreferencesControlId::Language => "language",
        PreferencesControlId::Editor => "editor",
        PreferencesControlId::InteractiveForm => "interactive_form",
        PreferencesControlId::AfterRun => "after_run",
        PreferencesControlId::Javascript => "javascript",
        PreferencesControlId::BashPath => "bash_path",
        PreferencesControlId::ManageAgents => "manage_agents",
        PreferencesControlId::InstallAgentSkill => "install_agent_skill",
        PreferencesControlId::MirrorMaster => "mirror_master",
        PreferencesControlId::PypiChoice => "pypi_choice",
        PreferencesControlId::PypiUrl => "pypi_url",
        PreferencesControlId::GithubChoice => "github_choice",
        PreferencesControlId::GithubUrl => "github_url",
        PreferencesControlId::NpmChoice => "npm_choice",
        PreferencesControlId::NpmUrl => "npm_url",
    })
}

const fn corpus_agent_scope_label(scope: AgentScope) -> &'static str {
    match scope {
        AgentScope::User => "user",
        AgentScope::Project => "project",
    }
}

fn corpus_relative_component_values(path: &Path) -> Result<Vec<Value>, String> {
    #[cfg(unix)]
    let invalid_spelling = {
        use std::os::unix::ffi::OsStrExt as _;
        let bytes = path.as_os_str().as_bytes();
        bytes.is_empty()
            || bytes.starts_with(b"/")
            || bytes.ends_with(b"/")
            || bytes
                .split(|byte| *byte == b'/')
                .any(|part| part.is_empty() || part == b"." || part == b"..")
    };
    #[cfg(windows)]
    let invalid_spelling = {
        use std::os::windows::ffi::OsStrExt as _;
        let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
        units.is_empty()
            || units.ends_with(&[u16::from(b'/')])
            || units.ends_with(&[u16::from(b'\\')])
            || units
                .split(|unit| *unit == u16::from(b'/') || *unit == u16::from(b'\\'))
                .any(|part| {
                    part.is_empty()
                        || part == [u16::from(b'.')]
                        || part == [u16::from(b'.'), u16::from(b'.')]
                })
    };
    #[cfg(not(any(unix, windows)))]
    let invalid_spelling = path.as_os_str().is_empty();
    if invalid_spelling
        || !path
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err("a file-picker screen target is not a strict UTF-8 relative path".to_owned());
    }
    path.iter()
        .map(|component| {
            component
                .to_str()
                .map(|component| json!(component))
                .ok_or_else(|| {
                    "a file-picker screen target is not a strict UTF-8 relative path".to_owned()
                })
        })
        .collect()
}

fn corpus_screen_target_value(target: &ScreenTarget) -> Result<Value, String> {
    match target {
        ScreenTarget::Add(target) => Ok(json!({"add": corpus_add_control_value(target)})),
        ScreenTarget::Preferences(target) => {
            Ok(json!({"preferences": corpus_preferences_control_value(*target)}))
        }
        ScreenTarget::AgentSkill { name, scope } => Ok(json!({
            "agent_skill": {"name": name, "scope": corpus_agent_scope_label(*scope)},
        })),
        ScreenTarget::Runner { name } => {
            if name.is_empty() {
                return Err("a runner screen target has an empty name".to_owned());
            }
            Ok(json!({"runner": {"name": name}}))
        }
        ScreenTarget::FilePickerEntry { relative } => Ok(json!({
            "file_picker_entry": {"components": corpus_relative_component_values(relative)?},
        })),
    }
}

fn corpus_local_outcome_value(outcome: &LocalActionOutcome) -> Value {
    match outcome {
        LocalActionOutcome::Action(action) => json!({"action": action}),
        LocalActionOutcome::Consumed => json!("consumed"),
    }
}

fn corpus_binding_event(binding: UiBinding) -> CorpusKeyEvent {
    CorpusKeyEvent::press(
        CorpusKey::from_ui(binding.key),
        CorpusModifiers {
            control: binding.modifiers.control,
            alt: binding.modifiers.alt,
            shift: binding.modifiers.shift,
        },
    )
}

fn corpus_binding_value(binding: UiBinding) -> Value {
    corpus_binding_event(binding).value()
}

pub(super) fn corpus_operation_value(operation: &CorpusOperation) -> Result<Value, String> {
    match operation {
        CorpusOperation::CommandKeyboard(command) => {
            Ok(json!({"operation": "command_keyboard", "command": command}))
        }
        CorpusOperation::HitTarget(target) => Ok(json!({
            "operation": "hit_target",
            "target": corpus_hit_target_value(*target),
        })),
        CorpusOperation::ScreenFocus(target) => Ok(json!({
            "operation": "screen_focus",
            "target": corpus_screen_target_value(target)?,
        })),
        CorpusOperation::ScreenHit(target) => Ok(json!({
            "operation": "screen_hit",
            "target": corpus_screen_target_value(target)?,
        })),
        CorpusOperation::LocalKeyboard { target, key } => Ok(json!({
            "operation": "local_keyboard",
            "target": corpus_local_target_value(target),
            "key": key.value(),
        })),
        CorpusOperation::LocalHit(target) => Ok(json!({
            "operation": "local_hit",
            "target": corpus_local_target_value(target),
        })),
        CorpusOperation::RawKey(key) => Ok(json!({"operation": "raw_key", "key": key.value()})),
        CorpusOperation::RawMouse { column, row, kind } => Ok(json!({
            "operation": "raw_mouse",
            "column": column,
            "row": row,
            "kind": kind.label(),
        })),
        CorpusOperation::Paste(value) => Ok(json!({"operation": "paste", "value": value})),
        CorpusOperation::Focus { gained } => Ok(json!({"operation": "focus", "gained": gained})),
        CorpusOperation::Resize { width, height } => {
            Ok(json!({"operation": "resize", "width": width, "height": height}))
        }
        CorpusOperation::FinalLiveness => Ok(json!({"synthetic": "final_liveness"})),
    }
}

fn corpus_rect_value(rect: Rect) -> Value {
    json!({
        "x": rect.x,
        "y": rect.y,
        "width": rect.width,
        "height": rect.height,
    })
}

fn corpus_semantic_target_value(target: &CorpusSemanticTarget) -> Result<Value, String> {
    match target {
        CorpusSemanticTarget::Requested(operation) => Ok(json!({
            "requested": corpus_operation_value(operation)?,
        })),
        CorpusSemanticTarget::Command {
            context,
            command,
            binding,
        } => Ok(json!({
            "command": command,
            "context": context,
            "binding": corpus_binding_value(*binding),
        })),
        CorpusSemanticTarget::Hit { target, rect } => Ok(json!({
            "hit_target": corpus_hit_target_value(*target),
            "rect": corpus_rect_value(*rect),
        })),
        CorpusSemanticTarget::Local {
            target,
            keys,
            hit,
            outcome,
        } => Ok(json!({
            "local_target": corpus_local_target_value(target),
            "keys": keys.iter().map(|key| key.value()).collect::<Vec<_>>(),
            "hit": hit.map(corpus_rect_value),
            "outcome": corpus_local_outcome_value(outcome),
        })),
        CorpusSemanticTarget::ScreenFocus {
            target,
            current,
            order,
        } => Ok(json!({
            "screen_focus": {
                "target": corpus_screen_target_value(target)?,
                "current": corpus_screen_target_value(current)?,
                "order": order
                    .iter()
                    .map(corpus_screen_target_value)
                    .collect::<Result<Vec<_>, _>>()?,
            },
        })),
        CorpusSemanticTarget::ScreenHit { target, rect } => Ok(json!({
            "screen_hit": {
                "target": corpus_screen_target_value(target)?,
                "rect": corpus_rect_value(*rect),
            },
        })),
    }
}

fn corpus_resolution_value(
    phase: TimelinePhase,
    resolution: &CorpusResolution,
) -> Result<Value, String> {
    if phase == TimelinePhase::FinalLiveness {
        let event = resolution
            .events
            .first()
            .filter(|_| resolution.events.len() == 1)
            .ok_or_else(|| "final liveness must resolve to one real event".to_owned())?;
        return Ok(json!({"event": event.value()}));
    }
    let input = match resolution.input {
        CorpusInputKind::NotApplicable => json!({"kind": resolution.input.label()}),
        CorpusInputKind::Event | CorpusInputKind::LocalEvent | CorpusInputKind::Resize => {
            let event = resolution
                .events
                .first()
                .filter(|_| resolution.events.len() == 1)
                .ok_or_else(|| "a single corpus input must contain one event".to_owned())?;
            json!({"kind": resolution.input.label(), "event": event.value()})
        }
        CorpusInputKind::EventChain => {
            if resolution.events.is_empty() {
                return Err("a corpus event chain must contain at least one event".to_owned());
            }
            json!({
                "kind": resolution.input.label(),
                "events": resolution.events.iter().map(CorpusEvent::value).collect::<Vec<_>>(),
            })
        }
        CorpusInputKind::PrimaryClick | CorpusInputKind::LocalPrimaryClick => {
            if resolution.events.len() != 2 {
                return Err("a corpus click must contain press and release events".to_owned());
            }
            json!({
                "kind": resolution.input.label(),
                "events": resolution.events.iter().map(CorpusEvent::value).collect::<Vec<_>>(),
            })
        }
    };
    let mut value = json!({
        "input": input,
        "semantic_target": corpus_semantic_target_value(&resolution.target)?,
    });
    if let Some(refusal) = resolution.refusal {
        value["refusal"] = json!(refusal.label());
    }
    Ok(value)
}

fn corpus_not_applicable(
    operation: &CorpusOperation,
    reason: CorpusNotApplicable,
) -> OperationResolution<CorpusEvent, CorpusResolution> {
    OperationResolution::NotApplicable {
        resolved: CorpusResolution {
            input: CorpusInputKind::NotApplicable,
            events: Vec::new(),
            target: CorpusSemanticTarget::Requested(operation.clone()),
            refusal: Some(reason),
        },
    }
}

fn corpus_events(
    input: CorpusInputKind,
    events: Vec<CorpusEvent>,
    target: CorpusSemanticTarget,
) -> OperationResolution<CorpusEvent, CorpusResolution> {
    OperationResolution::Events {
        resolved: CorpusResolution {
            input,
            events: events.clone(),
            target,
            refusal: None,
        },
        events,
    }
}

fn corpus_click_events(rect: Rect) -> Option<Vec<CorpusEvent>> {
    if rect.width == 0 || rect.height == 0 {
        return None;
    }
    let column = rect.x.saturating_add((rect.width - 1) / 2);
    let row = rect.y.saturating_add((rect.height - 1) / 2);
    Some(vec![
        CorpusEvent::mouse(column, row, CorpusMouseKind::PrimaryDown),
        CorpusEvent::mouse(column, row, CorpusMouseKind::PrimaryUp),
    ])
}

const SESSION_FORK_REFUSAL: &str = "the persistent TUI session cannot fork for a corpus probe";

fn fork_session(frontend: &RealFrontend) -> Result<TuiSession, String> {
    // `try_fork` returns `None` only while a path-completion worker owns a channel. A walker
    // session never starts that worker.
    frontend
        .session
        .try_fork()
        .ok_or_else(|| SESSION_FORK_REFUSAL.to_owned())
}

fn event_maps_to_quit(frontend: &RealFrontend, event: &CorpusEvent) -> bool {
    map_event(event.terminal(), &frontend.state, &frontend.geometry) == Some(Action::Quit)
}

/// Check the exact raw event on a fork without changing the live pointer gesture.
fn pointer_event_quits(
    frontend: &RealFrontend,
    column: u16,
    row: u16,
    kind: CorpusMouseKind,
) -> Result<bool, String> {
    let mut fork = fork_session(frontend)?;
    Ok(fork.handle_event(
        CorpusEvent::mouse(column, row, kind).terminal(),
        &frontend.state,
        &frontend.geometry,
    ) == EventHandling::Action(Action::Quit))
}

/// Refuse a local descriptor whose live endpoint differs from its advertised outcome.
///
/// The corpus resolver takes the descriptor of one advertised local action. The fork replays the
/// planned events and compares the live endpoint against that descriptor.
fn check_local_action_endpoint(
    frontend: &RealFrontend,
    events: &[CorpusEvent],
    advertised: &LocalAdvertisedAction,
) -> Result<(), String> {
    let (last, arming) = events
        .split_last()
        .ok_or("a local corpus action needs one event")?;
    let mut fork = fork_session(frontend)?;
    for event in arming {
        let _ = fork.handle_event(event.terminal(), &frontend.state, &frontend.geometry);
    }
    let handling = fork.handle_event(last.terminal(), &frontend.state, &frontend.geometry);
    parity::check_local_endpoint(&last.terminal(), advertised, &handling)
}

const fn screen_target_error_message(error: ScreenTargetError) -> &'static str {
    match error {
        ScreenTargetError::ScreenUnavailable => "the screen target inventory is unavailable",
        ScreenTargetError::StaleSession => "the screen target inventory is stale",
        ScreenTargetError::RealFilesystemPicker => {
            "the screen target inventory uses the real filesystem picker"
        }
        ScreenTargetError::InvalidMemoryEntry => {
            "the screen target inventory contains an invalid memory entry"
        }
    }
}

fn validate_screen_target(target: &ScreenTarget) -> Result<(), String> {
    match target {
        ScreenTarget::Runner { name } if name.is_empty() => {
            return Err("a runner screen target has an empty name".to_owned());
        }
        ScreenTarget::FilePickerEntry { relative } => {
            let _ = corpus_relative_component_values(relative)?;
        }
        ScreenTarget::Add(_)
        | ScreenTarget::Preferences(_)
        | ScreenTarget::AgentSkill { .. }
        | ScreenTarget::Runner { .. } => {}
    }
    Ok(())
}

fn validate_screen_inventory(inventory: &ScreenTargetInventory) -> Result<(), String> {
    for target in &inventory.available {
        validate_screen_target(target)?;
    }
    for hit in &inventory.hits {
        validate_screen_target(&hit.target)?;
        if !inventory.available.contains(&hit.target) {
            return Err("a screen target hit is absent from the available targets".to_owned());
        }
    }
    if let Some(focus) = &inventory.focus {
        for target in &focus.order {
            validate_screen_target(target)?;
            if !inventory.available.contains(target) {
                return Err("a screen focus target is absent from the available targets".to_owned());
            }
        }
        if let Some(current) = &focus.current {
            validate_screen_target(current)?;
            if !focus.order.contains(current) {
                return Err("the current screen focus is absent from its focus order".to_owned());
            }
        }
    }
    Ok(())
}

fn rect_is_fully_inside(rect: Rect, viewport: Rect) -> bool {
    !rect.is_empty()
        && rect.x >= viewport.x
        && rect.y >= viewport.y
        && rect.right() <= viewport.right()
        && rect.bottom() <= viewport.bottom()
}

fn resolve_screen_hit_with_inventory(
    frontend: &RealFrontend,
    operation: &CorpusOperation,
    target: &ScreenTarget,
    inventory: &ScreenTargetInventory,
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    let available = inventory
        .available
        .iter()
        .filter(|candidate| *candidate == target)
        .count();
    if available == 0 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Unavailable,
        ));
    }
    if available > 1 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Ambiguous,
        ));
    }
    let hits = inventory
        .hits
        .iter()
        .filter(|hit| &hit.target == target)
        .collect::<Vec<_>>();
    if hits.is_empty() {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Clipped,
        ));
    }
    if hits.len() > 1 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Ambiguous,
        ));
    }
    let hit = hits[0];
    let viewport = frontend.terminal.backend().buffer().area;
    if !rect_is_fully_inside(hit.rect, viewport) {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Clipped,
        ));
    }
    let events = corpus_click_events(hit.rect)
        .ok_or_else(|| "a nonempty screen target did not produce click events".to_owned())?;
    Ok(corpus_events(
        CorpusInputKind::PrimaryClick,
        events,
        CorpusSemanticTarget::ScreenHit {
            target: target.clone(),
            rect: hit.rect,
        },
    ))
}

fn keyboard_focus_target(target: &ScreenTarget) -> bool {
    matches!(target, ScreenTarget::Add(_) | ScreenTarget::Preferences(_))
}

fn resolve_screen_focus_with_inventory(
    operation: &CorpusOperation,
    target: &ScreenTarget,
    inventory: &ScreenTargetInventory,
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    let available = inventory
        .available
        .iter()
        .filter(|candidate| *candidate == target)
        .count();
    if available == 0 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Unavailable,
        ));
    }
    if available > 1 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Ambiguous,
        ));
    }
    if !keyboard_focus_target(target) {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::NotKeyboardFocusable,
        ));
    }
    let Some(focus) = inventory.focus.as_ref() else {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::NotKeyboardFocusable,
        ));
    };
    let target_indices = focus
        .order
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| (candidate == target).then_some(index))
        .collect::<Vec<_>>();
    if target_indices.is_empty() {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::NotKeyboardFocusable,
        ));
    }
    if target_indices.len() > 1 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Ambiguous,
        ));
    }
    let Some(current) = focus.current.as_ref() else {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::NotKeyboardFocusable,
        ));
    };
    if current == target {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::AlreadyFocused,
        ));
    }
    let current_indices = focus
        .order
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| (candidate == current).then_some(index))
        .collect::<Vec<_>>();
    if current_indices.len() > 1 {
        return Ok(corpus_not_applicable(
            operation,
            CorpusNotApplicable::Ambiguous,
        ));
    }
    let length = focus.order.len();
    let current_index = current_indices[0];
    let target_index = target_indices[0];
    let forward = target_index
        .saturating_add(length)
        .saturating_sub(current_index)
        % length;
    let backward = current_index
        .saturating_add(length)
        .saturating_sub(target_index)
        % length;
    let (key, steps) = if forward < backward {
        (CorpusKey::Tab, forward)
    } else {
        (CorpusKey::BackTab, backward)
    };
    let event = CorpusEvent::Key(CorpusKeyEvent::press(key, CorpusModifiers::NONE));
    Ok(corpus_events(
        CorpusInputKind::EventChain,
        vec![event; steps],
        CorpusSemanticTarget::ScreenFocus {
            target: target.clone(),
            current: current.clone(),
            order: focus.order.clone(),
        },
    ))
}

fn resolve_screen_operation_with_inventory(
    frontend: &RealFrontend,
    operation: &CorpusOperation,
    inventory: &ScreenTargetInventory,
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    validate_screen_inventory(inventory)?;
    match operation {
        CorpusOperation::ScreenFocus(target) => {
            validate_screen_target(target)?;
            resolve_screen_focus_with_inventory(operation, target, inventory)
        }
        CorpusOperation::ScreenHit(target) => {
            validate_screen_target(target)?;
            resolve_screen_hit_with_inventory(frontend, operation, target, inventory)
        }
        CorpusOperation::CommandKeyboard(_)
        | CorpusOperation::HitTarget(_)
        | CorpusOperation::LocalKeyboard { .. }
        | CorpusOperation::LocalHit(_)
        | CorpusOperation::RawKey(_)
        | CorpusOperation::RawMouse { .. }
        | CorpusOperation::Paste(_)
        | CorpusOperation::Focus { .. }
        | CorpusOperation::Resize { .. }
        | CorpusOperation::FinalLiveness => {
            Err("the screen resolver received a non-screen operation".to_owned())
        }
    }
}

fn resolve_screen_operation(
    frontend: &RealFrontend,
    operation: &CorpusOperation,
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    let inventory = frontend
        .session
        .screen_target_inventory(&frontend.state)
        .map_err(|error| screen_target_error_message(error).to_owned())?;
    resolve_screen_operation_with_inventory(frontend, operation, &inventory)
}

fn resolve_corpus_operation(
    frontend: &RealFrontend,
    operation: &CorpusOperation,
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    resolve_corpus_operation_with_local_actions(
        frontend,
        operation,
        &frontend.session.local_action_inventory().actions,
    )
}

fn resolve_corpus_operation_with_local_actions(
    frontend: &RealFrontend,
    operation: &CorpusOperation,
    local_actions: &[LocalAdvertisedAction],
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    match operation {
        CorpusOperation::CommandKeyboard(UiCommand::Quit)
        | CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Quit)) => Ok(
            corpus_not_applicable(operation, CorpusNotApplicable::QuitFiltered),
        ),
        CorpusOperation::ScreenFocus(_) | CorpusOperation::ScreenHit(_) => {
            resolve_screen_operation(frontend, operation)
        }
        CorpusOperation::CommandKeyboard(command) => {
            let context = frontend.state.command_context();
            let binding = command_specs(context)
                .find(|spec| spec.command == *command && frontend.state.command_enabled(*command))
                .and_then(|spec| spec.bindings.first().copied());
            let Some(binding) = binding else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            };
            let event = CorpusEvent::Key(corpus_binding_event(binding));
            Ok(corpus_events(
                CorpusInputKind::Event,
                vec![event],
                CorpusSemanticTarget::Command {
                    context,
                    command: *command,
                    binding,
                },
            ))
        }
        CorpusOperation::HitTarget(target) => {
            let mut hits = frontend
                .geometry
                .hits
                .iter()
                .filter(|hit| hit.action == *target);
            let Some(hit) = hits.next() else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            };
            if hits.next().is_some() {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            }
            let Some(events) = corpus_click_events(hit.rect) else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Clipped,
                ));
            };
            Ok(corpus_events(
                CorpusInputKind::PrimaryClick,
                events,
                CorpusSemanticTarget::Hit {
                    target: *target,
                    rect: hit.rect,
                },
            ))
        }
        CorpusOperation::LocalKeyboard { target, key } => {
            let actions = local_actions
                .iter()
                .filter(|advertised| advertised.target == *target)
                .map(|advertised| {
                    let keys = advertised
                        .keys
                        .iter()
                        .map(|binding| CorpusKeyEvent::from_terminal(binding.event()))
                        .collect::<Option<Vec<_>>>()
                        .ok_or_else(|| "a local action advertised an unsupported key".to_owned())?;
                    Ok((advertised, keys))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut matching = actions
                .iter()
                .filter(|(_, advertised_keys)| advertised_keys.contains(key));
            let Some((advertised, keys)) = matching.next() else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            };
            if matching.next().is_some() {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            }
            let events = vec![CorpusEvent::Key(*key)];
            check_local_action_endpoint(frontend, &events, advertised)?;
            Ok(corpus_events(
                CorpusInputKind::LocalEvent,
                events,
                CorpusSemanticTarget::Local {
                    target: target.clone(),
                    keys: keys.clone(),
                    hit: advertised.hit,
                    outcome: advertised.outcome.clone(),
                },
            ))
        }
        CorpusOperation::LocalHit(target) => {
            let mut actions = local_actions
                .iter()
                .filter(|advertised| advertised.target == *target);
            let Some(advertised) = actions.next() else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            };
            if actions.next().is_some() {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Unavailable,
                ));
            }
            let keys = advertised
                .keys
                .iter()
                .map(|binding| CorpusKeyEvent::from_terminal(binding.event()))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| "a local action advertised an unsupported key".to_owned())?;
            let Some(rect) = advertised.hit else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Clipped,
                ));
            };
            let Some(events) = corpus_click_events(rect) else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::Clipped,
                ));
            };
            check_local_action_endpoint(frontend, &events, advertised)?;
            Ok(corpus_events(
                CorpusInputKind::LocalPrimaryClick,
                events,
                CorpusSemanticTarget::Local {
                    target: target.clone(),
                    keys,
                    hit: advertised.hit,
                    outcome: advertised.outcome.clone(),
                },
            ))
        }
        CorpusOperation::RawKey(key) => {
            let event = CorpusEvent::Key(*key);
            if event_maps_to_quit(frontend, &event) {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::EventWouldQuit,
                ));
            }
            Ok(corpus_events(
                CorpusInputKind::Event,
                vec![event],
                CorpusSemanticTarget::Requested(operation.clone()),
            ))
        }
        CorpusOperation::RawMouse { column, row, kind } => {
            if pointer_event_quits(frontend, *column, *row, *kind)? {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::EventWouldQuit,
                ));
            }
            Ok(corpus_events(
                CorpusInputKind::Event,
                vec![CorpusEvent::mouse(*column, *row, *kind)],
                CorpusSemanticTarget::Requested(operation.clone()),
            ))
        }
        CorpusOperation::Paste(value) => Ok(corpus_events(
            CorpusInputKind::Event,
            vec![CorpusEvent::Paste(value.clone())],
            CorpusSemanticTarget::Requested(operation.clone()),
        )),
        CorpusOperation::Focus { gained } => Ok(corpus_events(
            CorpusInputKind::Event,
            vec![CorpusEvent::Focus { gained: *gained }],
            CorpusSemanticTarget::Requested(operation.clone()),
        )),
        CorpusOperation::Resize { width, height } => {
            let Some(size) = corpus_resize_size(*width, *height) else {
                return Ok(corpus_not_applicable(
                    operation,
                    CorpusNotApplicable::InvalidViewport,
                ));
            };
            Ok(corpus_events(
                CorpusInputKind::Resize,
                vec![CorpusEvent::Resize {
                    width: size.width,
                    height: size.height,
                }],
                CorpusSemanticTarget::Requested(operation.clone()),
            ))
        }
        CorpusOperation::FinalLiveness => Ok(corpus_events(
            CorpusInputKind::Event,
            vec![CorpusEvent::Focus { gained: true }],
            CorpusSemanticTarget::Requested(operation.clone()),
        )),
    }
}

impl FrontendAdapter for CorpusFrontend {
    type Operation = CorpusOperation;
    type Resolution = CorpusResolution;
    type Event = CorpusEvent;
    type Action = Action;
    type Effect = Effect;
    type Handling = SmokeHandling;
    type Observation = FrontendObservation;
    type ParitySnapshot = FrontendParity;

    fn resolve(
        &self,
        operation: &Self::Operation,
    ) -> Result<OperationResolution<Self::Event, Self::Resolution>, String> {
        resolve_corpus_operation(&self.inner, operation)
    }

    fn dispatch(
        &mut self,
        event: Self::Event,
    ) -> Result<DispatchOutcome<Self::Action, Self::Handling>, String> {
        if let CorpusEvent::Resize { width, height } = event {
            self.resize(width, height)?;
            return self
                .inner
                .dispatch_terminal_event(Event::Resize(width, height));
        }
        self.inner.dispatch_terminal_event(event.terminal())
    }

    fn reduce(&mut self, action: Self::Action) -> Result<Self::Effect, String> {
        Ok(self.inner.reduce_action(action))
    }

    fn effect_route(&self, effect: &Self::Effect) -> EffectRoute {
        RealFrontend::route_effect(effect)
    }

    fn observe(&mut self) -> Result<Self::Observation, String> {
        self.inner.observe_frontend()
    }

    fn parity_snapshot(&self) -> Result<Self::ParitySnapshot, String> {
        self.inner.frontend_parity()
    }
}

pub(super) struct RealHostBoundary {
    host: RealWalkerHost,
}

impl RealHostBoundary {
    fn sandbox_metadata(
        &self,
        review_profile: &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError> {
        self.host.sandbox_metadata(review_profile)
    }

    fn leak_oracle_facts(&self) -> LeakOracleFacts {
        self.host.leak_oracle_facts()
    }

    fn close(self) -> Result<(), SandboxError> {
        self.host.close()
    }

    fn serve_effect(&mut self, effect: Effect) -> Result<Action, String> {
        self.host
            .dispatch(effect)
            .map_err(|error| error.to_string())
    }

    fn capture_frontend(
        &mut self,
        mut frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, HostObservation>, String> {
        let host = self.host.capture_checkpoint_parts(
            &mut frontend.state,
            &mut frontend.session,
            cause,
        )?;
        Ok(CheckpointCapture { frontend, host })
    }
}

impl HostAdapter<RealFrontend> for RealHostBoundary {
    type Observation = HostObservation;

    fn serve(&mut self, effect: Effect) -> Result<Action, String> {
        self.serve_effect(effect)
    }

    fn capture_checkpoint(
        &mut self,
        frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
        self.capture_frontend(frontend, cause)
    }
}

pub(super) struct CorpusHostBoundary {
    inner: RealHostBoundary,
    leak_sandbox: Option<SandboxMetadata>,
}

/// Facts for one projected checkpoint. They never enter the stored host observation.
struct CheckpointLeakContext {
    sandbox: SandboxMetadata,
    facts: LeakOracleFacts,
}

pub(super) struct CorpusHostObservation {
    observation: HostObservation,
    leak_context: Option<CheckpointLeakContext>,
}

impl CorpusHostBoundary {
    fn close(self) -> Result<(), SandboxError> {
        self.inner.close()
    }
}

impl HostAdapter<CorpusFrontend> for CorpusHostBoundary {
    type Observation = CorpusHostObservation;

    fn serve(&mut self, effect: Effect) -> Result<Action, String> {
        self.inner.serve_effect(effect)
    }

    fn capture_checkpoint(
        &mut self,
        frontend: FrontendObservation,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
        let CheckpointCapture { frontend, host } = self.inner.capture_frontend(frontend, cause)?;
        let leak_context = self
            .leak_sandbox
            .as_ref()
            .map(|sandbox| CheckpointLeakContext {
                sandbox: sandbox.clone(),
                facts: self.inner.leak_oracle_facts(),
            });
        Ok(CheckpointCapture {
            frontend,
            host: CorpusHostObservation {
                observation: host,
                leak_context,
            },
        })
    }
}

#[derive(Clone, Debug)]
struct RealWalkerFactory {
    seed: WalkerSeedSpec,
    review_profile: SafeProfileId,
    locale: Locale,
    size: Size,
}

impl RealWalkerFactory {
    fn frontend_for_host(
        &self,
        host: RealWalkerHost,
    ) -> Result<(RealFrontend, RealHostBoundary), String> {
        self.frontend_for_host_with_initial_state(host, |host| {
            host.initial_state().map_err(|error| error.to_string())
        })
    }

    fn frontend_for_host_with_initial_state(
        &self,
        host: RealWalkerHost,
        initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    ) -> Result<(RealFrontend, RealHostBoundary), String> {
        let state = match initial_state(&host) {
            Ok(state) => state,
            Err(error) => return Err(close_after_primary(host, error)),
        };
        let frontend = match RealFrontend::new(state, self.locale, self.size) {
            Ok(frontend) => frontend,
            Err(error) => return Err(close_after_primary(host, error)),
        };
        Ok((frontend, RealHostBoundary { host }))
    }
}

impl ReplayFactory for RealWalkerFactory {
    type Frontend = RealFrontend;
    type Host = RealHostBoundary;

    fn create(&self) -> Result<(Self::Frontend, Self::Host), String> {
        let host = RealWalkerHost::spawn(self.seed.clone())?;
        self.frontend_for_host(host)
    }

    fn effect_limit(&self) -> usize {
        EFFECT_LIMIT
    }
}

/// One engine that runs corpus operations against a real host.
pub(super) type CorpusEngine = WalkerEngine<CorpusFrontend, CorpusHostBoundary>;

/// A factory that spawns one fresh random-mode host for the corpus adapter.
#[derive(Clone, Debug)]
pub(super) struct CorpusWalkerFactory {
    inner: RealWalkerFactory,
}

impl CorpusWalkerFactory {
    /// Create one corpus frontend and host pair from one caller-supplied initial state.
    pub(super) fn create_with_initial_state(
        &self,
        initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    ) -> Result<(CorpusFrontend, CorpusHostBoundary), String> {
        let host = RealWalkerHost::spawn(self.inner.seed.clone())?;
        let file_picker_tree = host.file_picker_tree();
        let (frontend, inner) = self
            .inner
            .frontend_for_host_with_initial_state(host, initial_state)?;
        Ok((
            CorpusFrontend::from_real(frontend, file_picker_tree),
            CorpusHostBoundary {
                inner,
                leak_sandbox: None,
            },
        ))
    }

    /// Create one unstarted engine from one caller-supplied initial state.
    pub(super) fn engine_with_initial_state(
        &self,
        initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    ) -> Result<CorpusEngine, String> {
        let (frontend, host) = self.create_with_initial_state(initial_state)?;
        Ok(WalkerEngine::new(frontend, host, EFFECT_LIMIT)
            .expect("the corpus effect limit is positive"))
    }

    /// Create one recorder for a walk that resizes into `canvas`.
    pub(super) fn walk_sink(&self, canvas: Size) -> Result<SchemaThreeSink, String> {
        SchemaThreeSink::new(
            self.inner.review_profile.clone(),
            self.inner.locale,
            self.inner.size,
            canvas,
        )
    }
}

impl ReplayFactory for CorpusWalkerFactory {
    type Frontend = CorpusFrontend;
    type Host = CorpusHostBoundary;

    fn create(&self) -> Result<(Self::Frontend, Self::Host), String> {
        self.create_with_initial_state(real_initial_state)
    }

    fn effect_limit(&self) -> usize {
        EFFECT_LIMIT
    }
}

/// Read the initial state of one real host.
pub(super) fn real_initial_state(host: &RealWalkerHost) -> Result<LibraryState, String> {
    host.initial_state().map_err(|error| error.to_string())
}

/// Build one random-mode corpus factory for one locale and viewport.
pub(super) fn random_walk_factory(locale: Locale, size: Size) -> CorpusWalkerFactory {
    let profile = format!(
        "random-{}-{}x{}",
        locale.tag().to_ascii_lowercase(),
        size.width,
        size.height
    );
    CorpusWalkerFactory {
        inner: RealWalkerFactory {
            seed: canonical_corpus_seed(locale),
            review_profile: SafeProfileId::try_from(profile)
                .expect("the random walk profile identifier is valid"),
            locale,
            size,
        },
    }
}

/// Close the real host of one corpus engine.
pub(super) fn close_corpus_engine(engine: CorpusEngine) -> Result<(), String> {
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().map_err(|error| error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CauseObservation {
    Initial,
    Reducer {
        dispatch_sequence: usize,
        operation_index: usize,
        operation: SmokeOperation,
        resolved: SmokeResolution,
        event: SmokeEvent,
        selector: String,
    },
    Host {
        dispatch_sequence: usize,
        operation_index: usize,
        operation: SmokeOperation,
        round: usize,
        selector: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct CanonicalCheckpoint {
    phase: EnginePhase,
    boundary: EngineBoundary,
    cause: CauseObservation,
    canonical_state: Value,
    host: HostObservation,
    session: Value,
    styled_frame: StyledFrameSnapshot,
    geometry: Value,
    locale: Locale,
}

#[derive(Default)]
struct MemoryTrace {
    checkpoints: Vec<CanonicalCheckpoint>,
}

impl CheckpointSink<CheckpointFor<RealFrontend, RealHostBoundary>> for MemoryTrace {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<RealFrontend, RealHostBoundary>,
    ) -> Result<(), String> {
        let cause = match &checkpoint.cause {
            EngineCause::Initial => CauseObservation::Initial,
            EngineCause::Reducer {
                dispatch_sequence,
                operation_index: Some(operation_index),
                operation,
                resolved,
                event,
                action: Action::OpenRun,
                emitted:
                    Effect::Open {
                        request: HostRequest::Run,
                        selector: Some(selector),
                    },
            } => CauseObservation::Reducer {
                dispatch_sequence: *dispatch_sequence,
                operation_index: *operation_index,
                operation: *operation,
                resolved: *resolved,
                event: *event,
                selector: selector.clone(),
            },
            EngineCause::Host {
                dispatch_sequence,
                operation_index: Some(operation_index),
                operation,
                round,
                request:
                    Effect::Open {
                        request: HostRequest::Run,
                        selector: Some(selector),
                    },
                response: Action::Present(Screen::Run(_)),
                emitted: Effect::None,
            } => CauseObservation::Host {
                dispatch_sequence: *dispatch_sequence,
                operation_index: *operation_index,
                operation: *operation,
                round: *round,
                selector: selector.clone(),
            },
            EngineCause::NotApplicable { .. } | EngineCause::Session { .. } => {
                return Err("the real walker smoke operation did not cross the host".to_owned());
            }
            EngineCause::Reducer { .. } | EngineCause::Host { .. } => {
                return Err("the real walker smoke host transition changed shape".to_owned());
            }
        };
        self.checkpoints.push(CanonicalCheckpoint {
            phase: checkpoint.phase,
            boundary: checkpoint.boundary,
            cause,
            canonical_state: checkpoint.host.state.clone(),
            host: checkpoint.host,
            session: checkpoint.frontend.session,
            styled_frame: checkpoint.frontend.styled_frame,
            geometry: checkpoint.frontend.geometry,
            locale: checkpoint.frontend.locale,
        });
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RealTrace {
    pub(super) review_profile: SafeProfileId,
    pub(super) locale: String,
    pub(super) viewport: RectSnapshot,
    pub(super) operations: Vec<Value>,
    pub(super) final_liveness_requested: Value,
    pub(super) rows: Vec<TimelineRow>,
    pub(super) objects: BTreeMap<(ObjectKind, String), Vec<u8>>,
    pub(super) cast: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct RecordedRealTrace {
    pub(super) trace: RealTrace,
    pub(super) sandbox: SandboxMetadata,
    pub(super) leak_oracle_facts: LeakOracleFacts,
}

impl std::ops::Deref for RecordedRealTrace {
    type Target = RealTrace;

    fn deref(&self) -> &Self::Target {
        &self.trace
    }
}

impl std::ops::DerefMut for RecordedRealTrace {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.trace
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedRealCorpus {
    pub(super) profiles: Vec<RecordedRealTrace>,
    pub(super) sandbox: SandboxMetadata,
}

impl RecordedRealCorpus {
    pub(super) fn new(profiles: Vec<RecordedRealTrace>) -> Result<Self, String> {
        let required = skit_tui_walker_support::required_review_profiles();
        if profiles.len() != required.len() {
            return Err("the recorded corpus does not contain the exact four profiles".to_owned());
        }
        let operations = profiles[0].operations.clone();
        let final_liveness = profiles[0].final_liveness_requested.clone();
        if operations.is_empty() {
            return Err("the recorded corpus has an empty operation vector".to_owned());
        }
        let platform = profiles[0].sandbox.platform();
        let root = profiles[0].sandbox.root().to_owned();
        let mut ids = BTreeSet::new();
        for (recorded, expected) in profiles.iter().zip(required) {
            let id = SafeProfileId::try_from(expected.id).map_err(|error| error.to_string())?;
            if recorded.review_profile != id {
                return Err("the recorded corpus profiles are not in required order".to_owned());
            }
            if recorded.locale != expected.locale || recorded.viewport != expected.viewport {
                return Err("a recorded corpus profile has the wrong locale or viewport".to_owned());
            }
            if recorded.operations != operations
                || recorded.final_liveness_requested != final_liveness
            {
                return Err("recorded corpus profiles do not share one operation vector".to_owned());
            }
            if recorded.rows.last().and_then(|row| row.liveness.clone())
                != Some(LivenessResult::Passed)
            {
                return Err("a recorded corpus profile did not pass final liveness".to_owned());
            }
            if recorded.rows.iter().any(|row| {
                matches!(
                    &row.cause,
                    TransitionCause::Session { handling, .. } if handling == "not_applicable"
                )
            }) {
                return Err("a recorded corpus profile contains a refused operation".to_owned());
            }
            if recorded.rows.iter().any(|row| row.profile != id) {
                return Err("a recorded corpus timeline has the wrong profile".to_owned());
            }
            if recorded.sandbox.mode() != SandboxMode::Stable
                || recorded.sandbox.platform() != platform
                || recorded.sandbox.root() != root
            {
                return Err(
                    "recorded corpus sandboxes do not share one stable namespace".to_owned(),
                );
            }
            let singleton =
                SandboxMetadata::stable(platform, root.clone(), BTreeSet::from([id.clone()]))
                    .map_err(|error| error.to_string())?;
            if recorded.sandbox != singleton {
                return Err("a recorded corpus profile has invalid singleton metadata".to_owned());
            }
            ids.insert(id);
        }
        let sandbox =
            SandboxMetadata::stable(platform, root, ids).map_err(|error| error.to_string())?;
        Ok(Self { profiles, sandbox })
    }
}

#[derive(Clone, Debug)]
pub(super) struct StableCorpusTracePair {
    pub(super) main: RecordedRealCorpus,
    pub(super) replay: RecordedRealCorpus,
}

impl StableCorpusTracePair {
    pub(super) fn new(
        main: RecordedRealCorpus,
        replay: RecordedRealCorpus,
    ) -> Result<Self, String> {
        if main.sandbox != replay.sandbox || main.profiles.len() != replay.profiles.len() {
            return Err("stable corpus generations use different aggregate metadata".to_owned());
        }
        if let Some((diverging, _)) = main
            .profiles
            .iter()
            .zip(&replay.profiles)
            .find(|(main, replay)| main.trace != replay.trace || main.sandbox != replay.sandbox)
        {
            return Err(format!(
                "stable corpus main and replay traces differ at {}",
                diverging.review_profile
            ));
        }
        Ok(Self { main, replay })
    }
}

#[derive(Clone, Debug)]
pub(super) struct StableTracePair {
    pub(super) main: RecordedRealTrace,
    pub(super) replay: RecordedRealTrace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum StablePairPhase {
    Main,
    Replay,
}

#[derive(Debug)]
pub(super) enum StablePairPrimaryError {
    Sandbox(Box<SandboxError>),
    Seed(StableHostPrimaryError),
    InitialState(String),
    Frontend(String),
    Trace(String),
    Close(Box<SandboxError>),
}

#[derive(Debug)]
pub(super) struct StablePairError {
    pub(super) phase: StablePairPhase,
    pub(super) primary: StablePairPrimaryError,
    pub(super) cleanup: Option<Box<SandboxError>>,
}

impl std::fmt::Display for StablePairPrimaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sandbox(error) => error.fmt(formatter),
            Self::Seed(error) => error.fmt(formatter),
            Self::InitialState(error) => {
                write!(
                    formatter,
                    "could not read the stable walker initial state: {error}"
                )
            }
            Self::Frontend(error) => {
                write!(
                    formatter,
                    "could not create the stable walker frontend: {error}"
                )
            }
            Self::Trace(error) => write!(formatter, "could not record the stable walker: {error}"),
            Self::Close(error) => write!(formatter, "could not close the stable walker: {error}"),
        }
    }
}

impl std::fmt::Display for StablePairError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match self.phase {
            StablePairPhase::Main => "main",
            StablePairPhase::Replay => "replay",
        };
        write!(formatter, "stable walker {phase} failed: {}", self.primary)?;
        if let Some(cleanup) = &self.cleanup {
            write!(formatter, "; sandbox cleanup also failed: {cleanup}")?;
        }
        Ok(())
    }
}

impl std::error::Error for StablePairError {}

pub(super) struct SchemaThreeSink {
    review_profile: SafeProfileId,
    initial_locale: String,
    viewport: RectSnapshot,
    canvas: Size,
    rows: Vec<TimelineRow>,
    objects: BTreeMap<(ObjectKind, String), Vec<u8>>,
    cast: AsciicastRecorder,
}

struct ProjectedCheckpoint {
    phase: EnginePhase,
    boundary: EngineBoundary,
    operation_index: Option<u32>,
    event_chain: Option<EventChainIdentity>,
    cause: TransitionCause,
    frontend: FrontendObservation,
    host: HostObservation,
    leak_context: Option<CheckpointLeakContext>,
}

impl SchemaThreeSink {
    fn new(
        review_profile: SafeProfileId,
        locale: Locale,
        viewport: Size,
        canvas: Size,
    ) -> Result<Self, String> {
        Ok(Self {
            review_profile,
            initial_locale: locale.tag().to_owned(),
            viewport: RectSnapshot {
                x: 0,
                y: 0,
                width: viewport.width,
                height: viewport.height,
            },
            canvas,
            rows: Vec::new(),
            objects: BTreeMap::new(),
            cast: AsciicastRecorder::new(canvas.width, canvas.height)
                .map_err(|error| error.to_string())?,
        })
    }

    /// Return the cast that this sink recorded so far.
    pub(super) fn cast_bytes(&self) -> &[u8] {
        self.cast.as_bytes()
    }

    /// Record the frame that a failing frontend last drew.
    ///
    /// A checkpoint that breaks one rule never reaches this sink. The failing screen still stays
    /// in the backend, so the artifact keeps one diagnostic frame beside the named rule.
    pub(super) fn record_frontend_frame(
        &mut self,
        frontend: &CorpusFrontend,
    ) -> Result<(), String> {
        self.cast
            .record_frame(FRAME_INTERVAL, frontend.inner.terminal.backend())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn put_value(&mut self, kind: ObjectKind, value: Value) -> Result<ObjectRef, String> {
        let mut staged = BTreeMap::new();
        let reference = Self::stage_value(&self.objects, &mut staged, kind, value)?;
        self.objects.extend(staged);
        Ok(reference)
    }

    fn stage_value(
        objects: &BTreeMap<(ObjectKind, String), Vec<u8>>,
        staged: &mut BTreeMap<(ObjectKind, String), Vec<u8>>,
        kind: ObjectKind,
        value: Value,
    ) -> Result<ObjectRef, String> {
        let (reference, bytes) = build_object(kind, &value).map_err(|error| error.to_string())?;
        let decoded = validate_object(&reference, &bytes).map_err(|error| error.to_string())?;
        debug_assert_eq!(decoded, value);
        let key = (reference.kind, reference.sha256.clone());
        if let Some(existing) = staged.get(&key).or_else(|| objects.get(&key)) {
            if existing != &bytes {
                return Err("one content digest mapped to different object bytes".to_owned());
            }
        } else {
            staged.insert(key, bytes);
        }
        Ok(reference)
    }

    fn object_value(&self, reference: &ObjectRef) -> Result<Value, String> {
        let bytes = self
            .objects
            .get(&(reference.kind, reference.sha256.clone()))
            .ok_or_else(|| "a timeline object is absent from memory".to_owned())?;
        validate_object(reference, bytes).map_err(|error| error.to_string())
    }

    fn validate_reducer_transitions(&self) -> Result<(), String> {
        let initial = self
            .rows
            .first()
            .ok_or_else(|| "the schema three trace has no checkpoint".to_owned())?;
        let initial_value = self.object_value(&initial.reducer)?;
        parse_library_state(&initial_value)?;

        for pair in self.rows.windows(2) {
            let previous = &pair[0];
            let current = &pair[1];
            let previous_value = self.object_value(&previous.reducer)?;
            let current_value = self.object_value(&current.reducer)?;
            parse_library_state(&current_value)?;
            let result = match &current.cause {
                TransitionCause::Initial => {
                    Err("only the first reducer object can be initial".to_owned())
                }
                TransitionCause::Session { .. } => {
                    if current_value != previous_value {
                        Err("a session checkpoint changed reducer state".to_owned())
                    } else {
                        Ok(())
                    }
                }
                TransitionCause::Reducer {
                    action, emitted, ..
                } => validate_reducer_action(&previous_value, &current_value, action, emitted),
                TransitionCause::Host {
                    response, emitted, ..
                } => validate_reducer_action(&previous_value, &current_value, response, emitted),
            };
            result.map_err(|error| {
                format!(
                    "timeline row {} operation {:?}: {error}",
                    current.sequence, current.operation_index
                )
            })?;
        }
        Ok(())
    }

    fn finish(
        mut self,
        operations: Vec<Value>,
        final_liveness_requested: Value,
    ) -> Result<RealTrace, String> {
        self.validate_reducer_transitions()?;
        let final_row = self
            .rows
            .last()
            .ok_or_else(|| "the schema three trace has no checkpoint".to_owned())?;
        let final_state = self.object_value(&final_row.reducer)?;
        if final_state.pointer("/workflow/active") != Some(&json!("library"))
            || final_state
                .get("modal")
                .is_some_and(|modal| !modal.is_null())
        {
            return Err("final liveness did not return the real reducer to the library".to_owned());
        }
        self.rows
            .last_mut()
            .expect("the final row was checked above")
            .liveness = Some(LivenessResult::Passed);

        let operation_count = checked_u32(operations.len(), "operation count")?;
        validate_timeline(&self.rows, operation_count).map_err(|error| error.to_string())?;
        validate_timeline_semantics(
            &self.rows,
            &operations,
            &final_liveness_requested,
            &self.initial_locale,
        )
        .map_err(|error| error.to_string())?;
        validate_effect_chain_termination(&self.rows)?;

        let canvas = bundle::cast_canvas(&self.rows).map_err(|error| error.to_string())?;
        if canvas != (self.canvas.width, self.canvas.height) {
            return Err("the timeline canvas does not match the live asciicast".to_owned());
        }
        let mut rebuilt = AsciicastRecorder::new(self.canvas.width, self.canvas.height)
            .map_err(|error| error.to_string())?;
        for row in &self.rows {
            for reference in [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ] {
                self.object_value(reference)?;
            }
            let frame_value = self.object_value(&row.styled_frame)?;
            let frame: StyledFrameSnapshot =
                serde_json::from_value(frame_value).map_err(|error| error.to_string())?;
            validate_styled_frame(&frame).map_err(|error| error.to_string())?;
            if frame.area != row.viewport {
                return Err("a styled frame does not match its timeline viewport".to_owned());
            }
            record_presentation(&mut rebuilt, row.presentation, &frame)?;
        }
        validate_asciicast_v3(self.cast.as_bytes()).map_err(|error| error.to_string())?;
        validate_asciicast_v3(rebuilt.as_bytes()).map_err(|error| error.to_string())?;
        if self.cast.as_bytes() != rebuilt.as_bytes() {
            return Err("stored styled frames did not rebuild the live asciicast".to_owned());
        }

        Ok(RealTrace {
            review_profile: self.review_profile,
            locale: self.initial_locale,
            viewport: self.viewport,
            operations,
            final_liveness_requested,
            rows: self.rows,
            objects: self.objects,
            cast: self.cast.as_bytes().to_vec(),
        })
    }

    fn finish_smoke(self) -> Result<RealTrace, String> {
        self.finish(
            vec![smoke_operation_value(SmokeOperation::OpenRun)],
            smoke_operation_value(SmokeOperation::FinalLiveness),
        )
    }

    fn record_checkpoint(&mut self, checkpoint: ProjectedCheckpoint) -> Result<(), String> {
        let ProjectedCheckpoint {
            phase,
            boundary,
            operation_index,
            event_chain,
            cause,
            frontend,
            host,
            leak_context,
        } = checkpoint;
        let host_value = json_value(&host);
        if let Some(context) = leak_context {
            let cause_value = json_value(&cause);
            super::tui_walker_bundle::validate_checkpoint_leaks(
                &context.sandbox,
                &context.facts,
                &[
                    ("cause", &cause_value),
                    ("reducer", &host.state),
                    ("host", &host_value),
                    ("session", &frontend.session),
                    ("geometry", &frontend.geometry),
                ],
                &frontend.styled_frame,
            )
            .map_err(|error| {
                format!("checkpoint phase {phase:?} operation index {operation_index:?}: {error}")
            })?;
        }
        let mut staged = BTreeMap::new();
        let reducer = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::Reducer,
            host.state.clone(),
        )?;
        let host = Self::stage_value(&self.objects, &mut staged, ObjectKind::Host, host_value)?;
        let session = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::Session,
            frontend.session,
        )?;
        let frame = frontend.styled_frame;
        let styled_frame = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::StyledFrame,
            json_value(&frame),
        )?;
        let geometry = Self::stage_value(
            &self.objects,
            &mut staged,
            ObjectKind::Geometry,
            frontend.geometry,
        )?;
        let sequence = u32::try_from(self.rows.len())
            .map_err(|_| "the schema three trace has too many checkpoints".to_owned())?;
        let previous_row_sha256 = self
            .rows
            .last()
            .map(timeline_row_digest)
            .transpose()
            .map_err(|error| error.to_string())?;
        let mut row = TimelineRow {
            schema: 3,
            profile: self.review_profile.clone(),
            sequence,
            phase: timeline_phase(phase),
            event_chain,
            operation_index,
            boundary: timeline_boundary(boundary),
            cause,
            presentation: Presentation::Presented,
            reducer,
            host,
            session,
            styled_frame,
            geometry,
            locale: frontend.locale.tag().to_owned(),
            viewport: frame.area,
            liveness: None,
            previous_row_sha256,
        };
        row.presentation = expected_presentation(&row);
        record_presentation(&mut self.cast, row.presentation, &frame)?;
        self.objects.extend(staged);
        self.rows.push(row);
        Ok(())
    }
}

pub(super) fn validate_reducer_action(
    previous: &Value,
    current: &Value,
    action: &Value,
    emitted: &Value,
) -> Result<(), String> {
    let mut state = parse_library_state(previous)?;
    let action: Action = serde_json::from_value(action.clone())
        .map_err(|error| format!("a reducer cause is not an Action: {error}"))?;
    let actual_effect = state.update(action);
    let actual_state = json_value(&state);
    // PathBuf equality keeps native separator semantics. The round trip still rejects
    // fields that deserialization would discard.
    let effect_matches = serde_json::from_value::<Effect>(emitted.clone())
        .is_ok_and(|expected| actual_effect == expected && json_value(&expected) == *emitted);
    if !effect_matches {
        return Err("a reducer cause emitted a different production Effect".to_owned());
    }
    if &actual_state != current {
        return Err("a reducer object does not match its production Action".to_owned());
    }
    Ok(())
}

pub(super) fn parse_library_state(value: &Value) -> Result<LibraryState, String> {
    let state: LibraryState = serde_json::from_value(value.clone())
        .map_err(|error| format!("a reducer object is not a LibraryState: {error}"))?;
    if json_value(&state) != *value {
        return Err("a reducer object is not a canonical LibraryState".to_owned());
    }
    Ok(state)
}

impl CheckpointSink<CheckpointFor<RealFrontend, RealHostBoundary>> for SchemaThreeSink {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<RealFrontend, RealHostBoundary>,
    ) -> Result<(), String> {
        let phase = timeline_phase(checkpoint.phase);
        let (operation_index, event_chain, cause) = transition_cause(phase, &checkpoint.cause)?;
        self.record_checkpoint(ProjectedCheckpoint {
            phase: checkpoint.phase,
            boundary: checkpoint.boundary,
            operation_index,
            event_chain,
            cause,
            frontend: checkpoint.frontend,
            host: checkpoint.host,
            leak_context: None,
        })
    }
}

impl CheckpointSink<CheckpointFor<CorpusFrontend, CorpusHostBoundary>> for SchemaThreeSink {
    fn record(
        &mut self,
        checkpoint: CheckpointFor<CorpusFrontend, CorpusHostBoundary>,
    ) -> Result<(), String> {
        let phase = timeline_phase(checkpoint.phase);
        let (operation_index, event_chain, cause) =
            corpus_transition_cause(phase, &checkpoint.cause)?;
        self.record_checkpoint(ProjectedCheckpoint {
            phase: checkpoint.phase,
            boundary: checkpoint.boundary,
            operation_index,
            event_chain,
            cause,
            frontend: checkpoint.frontend,
            host: checkpoint.host.observation,
            leak_context: checkpoint.host.leak_context,
        })
    }
}

fn record_presentation(
    recorder: &mut AsciicastRecorder,
    presentation: Presentation,
    frame: &StyledFrameSnapshot,
) -> Result<(), String> {
    match presentation {
        Presentation::Presented => recorder
            .record_snapshot(FRAME_INTERVAL, frame)
            .map(|_| ())
            .map_err(|error| error.to_string()),
        Presentation::NotPresented => {
            recorder.skip(FRAME_INTERVAL);
            Ok(())
        }
    }
}

const fn timeline_phase(phase: EnginePhase) -> TimelinePhase {
    match phase {
        EnginePhase::Operations => TimelinePhase::Operations,
        EnginePhase::FinalLiveness => TimelinePhase::FinalLiveness,
    }
}

const fn timeline_boundary(boundary: EngineBoundary) -> TimelineBoundary {
    match boundary {
        EngineBoundary::Initial => TimelineBoundary::Initial,
        EngineBoundary::Session => TimelineBoundary::Session,
        EngineBoundary::Reducer => TimelineBoundary::UserAction,
        EngineBoundary::Host => TimelineBoundary::HostAction,
    }
}

fn smoke_operation_value(operation: SmokeOperation) -> Value {
    match operation {
        SmokeOperation::OpenRun => json!({"operation": "open_run"}),
        SmokeOperation::OpenPreferences => json!({"operation": "open_preferences"}),
        SmokeOperation::OpenLanguagePicker => json!({"operation": "open_language_picker"}),
        SmokeOperation::NextLanguage => json!({"operation": "next_language"}),
        SmokeOperation::ChooseLanguage => json!({"operation": "choose_language"}),
        SmokeOperation::SavePreferences => json!({"operation": "save_preferences"}),
        SmokeOperation::FinalLiveness => json!({"synthetic": "final_liveness"}),
    }
}

fn smoke_event_value(event: SmokeEvent) -> Value {
    json!({
        "type": "key",
        "code": json_value(&event.key),
        "modifiers": json_value(&event.modifiers),
    })
}

fn smoke_resolution_value(
    phase: TimelinePhase,
    operation: SmokeOperation,
    resolution: SmokeResolution,
) -> Value {
    let event = smoke_event_value(resolution.event);
    match phase {
        TimelinePhase::Operations => json!({
            "input": {"kind": "event", "event": event},
            "semantic_target": match operation {
                SmokeOperation::OpenRun => json!({"command": "run"}),
                SmokeOperation::OpenPreferences => json!({"command": "preferences"}),
                SmokeOperation::OpenLanguagePicker => {
                    json!({"control": "language", "action": "open"})
                }
                SmokeOperation::NextLanguage => {
                    json!({"control": "language", "action": "next"})
                }
                SmokeOperation::ChooseLanguage => {
                    json!({"control": "language", "action": "choose"})
                }
                SmokeOperation::SavePreferences => json!({"command": "save_preferences"}),
                SmokeOperation::FinalLiveness => json!({"synthetic": "final_liveness"}),
            },
        }),
        TimelinePhase::FinalLiveness => json!({"event": event}),
    }
}

fn json_value(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).expect("walker schema values must serialize")
}

fn checked_u32(value: usize, label: &str) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("{label} exceeds u32"))
}

fn transition_cause(
    phase: TimelinePhase,
    cause: &EngineCause<SmokeOperation, SmokeResolution, SmokeEvent, Action, Effect, SmokeHandling>,
) -> Result<(Option<u32>, Option<EventChainIdentity>, TransitionCause), String> {
    match cause {
        EngineCause::Initial => Ok((None, None, TransitionCause::Initial)),
        EngineCause::NotApplicable {
            dispatch_sequence: _,
            operation_index: _,
            operation: _,
            resolved: _,
        } => Err("the real walker smoke has no not-applicable operation projection".to_owned()),
        EngineCause::Session {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            handling,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Session {
                    requested: smoke_operation_value(*operation),
                    resolved: smoke_resolution_value(phase, *operation, *resolved),
                    event: smoke_event_value(*event),
                    handling: json!(match handling {
                        SmokeHandling::Consumed => "consumed",
                        SmokeHandling::Ignored => "ignored",
                    }),
                },
            ))
        }
        EngineCause::Reducer {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            action,
            emitted,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Reducer {
                    requested: smoke_operation_value(*operation),
                    resolved: smoke_resolution_value(phase, *operation, *resolved),
                    event: smoke_event_value(*event),
                    action: json_value(action),
                    emitted: json_value(emitted),
                },
            ))
        }
        EngineCause::Host {
            dispatch_sequence,
            operation_index,
            round,
            request,
            response,
            emitted,
            ..
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Host {
                    operation_index,
                    round: u16::try_from(*round)
                        .map_err(|_| "host round exceeds u16".to_owned())?,
                    request: json_value(request),
                    response: json_value(response),
                    emitted: json_value(emitted),
                },
            ))
        }
    }
}

fn corpus_transition_cause(
    phase: TimelinePhase,
    cause: &EngineCause<
        CorpusOperation,
        CorpusResolution,
        CorpusEvent,
        Action,
        Effect,
        SmokeHandling,
    >,
) -> Result<(Option<u32>, Option<EventChainIdentity>, TransitionCause), String> {
    match cause {
        EngineCause::Initial => Ok((None, None, TransitionCause::Initial)),
        EngineCause::NotApplicable {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Session {
                    requested: corpus_operation_value(operation)?,
                    resolved: corpus_resolution_value(phase, resolved)?,
                    event: json!({"synthetic": "not_applicable"}),
                    handling: json!("not_applicable"),
                },
            ))
        }
        EngineCause::Session {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            handling,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Session {
                    requested: corpus_operation_value(operation)?,
                    resolved: corpus_resolution_value(phase, resolved)?,
                    event: event.value(),
                    handling: json!(match handling {
                        SmokeHandling::Consumed => "consumed",
                        SmokeHandling::Ignored => "ignored",
                    }),
                },
            ))
        }
        EngineCause::Reducer {
            dispatch_sequence,
            operation_index,
            operation,
            resolved,
            event,
            action,
            emitted,
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Reducer {
                    requested: corpus_operation_value(operation)?,
                    resolved: corpus_resolution_value(phase, resolved)?,
                    event: event.value(),
                    action: json_value(action),
                    emitted: json_value(emitted),
                },
            ))
        }
        EngineCause::Host {
            dispatch_sequence,
            operation_index,
            round,
            request,
            response,
            emitted,
            ..
        } => {
            let operation_index = operation_index
                .map(|index| checked_u32(index, "operation index"))
                .transpose()?;
            Ok((
                operation_index,
                Some(EventChainIdentity {
                    phase,
                    sequence: checked_u32(*dispatch_sequence, "dispatch sequence")?,
                }),
                TransitionCause::Host {
                    operation_index,
                    round: u16::try_from(*round)
                        .map_err(|_| "host round exceeds u16".to_owned())?,
                    request: json_value(request),
                    response: json_value(response),
                    emitted: json_value(emitted),
                },
            ))
        }
    }
}

pub(super) fn validate_effect_chain_termination(rows: &[TimelineRow]) -> Result<(), String> {
    let mut pending = false;
    let mut previous_emitted = None;
    for row in rows {
        match &row.cause {
            TransitionCause::Initial | TransitionCause::Session { .. } => {
                if pending {
                    return Err("a host effect was left pending before another event".to_owned());
                }
                previous_emitted = None;
            }
            TransitionCause::Reducer { emitted, .. } => {
                pending = emitted != "none";
                previous_emitted = Some(emitted);
            }
            TransitionCause::Host {
                request, emitted, ..
            } => {
                if !pending {
                    return Err("a host transition has no pending reducer effect".to_owned());
                }
                if previous_emitted != Some(request) {
                    return Err("a host request does not equal its predecessor effect".to_owned());
                }
                pending = emitted != "none";
                previous_emitted = Some(emitted);
            }
        }
    }
    if pending {
        return Err("the timeline ends with a pending host effect".to_owned());
    }
    Ok(())
}

fn new_schema_three_sink(factory: &RealWalkerFactory) -> Result<SchemaThreeSink, String> {
    SchemaThreeSink::new(
        factory.review_profile.clone(),
        factory.locale,
        factory.size,
        factory.size,
    )
}

fn new_corpus_schema_three_sink(
    factory: &RealWalkerFactory,
    operations: &[CorpusOperation],
) -> Result<SchemaThreeSink, String> {
    SchemaThreeSink::new(
        factory.review_profile.clone(),
        factory.locale,
        factory.size,
        corpus_canvas_size(factory.size, operations),
    )
}

fn combine_corpus_result_after_close<T>(
    result: Result<T, String>,
    close: Result<(), impl std::fmt::Display>,
) -> Result<T, String> {
    match (result, close) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(format!("could not close the real corpus host: {error}")),
        (Err(primary), Ok(())) => Err(primary),
        (Err(primary), Err(cleanup)) => Err(format!(
            "{primary}; real corpus host cleanup also failed: {cleanup}"
        )),
    }
}

fn prepare_corpus_host_with(
    host: RealWalkerHost,
    profile: &SafeProfileId,
    context: &impl Fn(String) -> String,
    read_metadata: impl FnOnce(&RealWalkerHost, &SafeProfileId) -> Result<SandboxMetadata, String>,
    read_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
) -> Result<
    (
        RealWalkerHost,
        SandboxMetadata,
        LibraryState,
        WalkerFilePickerTree,
    ),
    String,
> {
    let sandbox = match read_metadata(&host, profile) {
        Ok(sandbox) => sandbox,
        Err(error) => {
            return combine_corpus_result_after_close(Err(context(error)), host.close());
        }
    };
    let state = match read_state(&host) {
        Ok(state) => state,
        Err(error) => {
            return combine_corpus_result_after_close(Err(context(error)), host.close());
        }
    };
    let file_picker_tree = host.file_picker_tree();
    Ok((host, sandbox, state, file_picker_tree))
}

fn record_corpus_profile(
    factory: &RealWalkerFactory,
    namespace: StableSandboxNamespace,
    operations: &[CorpusOperation],
    phase: StablePairPhase,
) -> Result<RecordedRealTrace, String> {
    let profile = factory.review_profile.clone();
    let phase_label = match phase {
        StablePairPhase::Main => "main",
        StablePairPhase::Replay => "replay",
    };
    let context = |message: String| {
        format!("stable corpus {phase_label} profile {profile} failed: {message}")
    };
    let host = RealWalkerHost::spawn_stable_in(factory.seed.clone(), profile.clone(), namespace)
        .map_err(|error| context(error.to_string()))?;
    let read_metadata = |host: &RealWalkerHost, profile: &SafeProfileId| {
        host.sandbox_metadata(profile)
            .map_err(|error| error.to_string())
    };
    let read_state =
        |host: &RealWalkerHost| host.initial_state().map_err(|error| error.to_string());
    let prepared = prepare_corpus_host_with(host, &profile, &context, read_metadata, read_state);
    let (host, sandbox, state, file_picker_tree) = prepared?;
    let frontend = match CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree)
    {
        Ok(frontend) => frontend,
        Err(error) => {
            return combine_corpus_result_after_close(Err(context(error)), host.close());
        }
    };
    let boundary = CorpusHostBoundary {
        inner: RealHostBoundary { host },
        leak_sandbox: Some(sandbox.clone()),
    };
    let mut engine = WalkerEngine::new(frontend, boundary, EFFECT_LIMIT)
        .expect("the corpus effect limit is positive");
    let mut sink = Some(
        new_corpus_schema_three_sink(factory, operations)
            .expect("the validated positive corpus viewport creates an asciicast"),
    );
    let result = (|| {
        let sink_ref = sink
            .as_mut()
            .expect("the corpus sink exists before trace finish");
        engine.start(sink_ref).map_err(&context)?;
        for (index, operation) in operations.iter().enumerate() {
            engine
                .run_operation(operation.clone(), sink_ref)
                .map_err(|error| {
                    context(format!(
                        "operation {} {operation:?}: {error}",
                        index.saturating_add(1)
                    ))
                })?;
        }
        engine
            .run_final_liveness(CorpusOperation::FinalLiveness, sink_ref)
            .map_err(|error| context(format!("final liveness: {error}")))?;
        let successful_operations = engine.successful_operations();
        let operation_values = successful_operations
            .iter()
            .map(corpus_operation_value)
            .collect::<Result<Vec<_>, _>>()
            .map_err(&context)?;
        let final_liveness = corpus_operation_value(&CorpusOperation::FinalLiveness)
            .expect("the fixed final-liveness operation serializes");
        let facts = engine.host().inner.leak_oracle_facts();
        let trace = sink
            .take()
            .expect("the corpus sink is consumed exactly once")
            .finish(operation_values, final_liveness)
            .map_err(&context)?;
        Ok((trace, facts))
    })();
    drop(sink);
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    combine_corpus_result_after_close(result, host.close()).map(|(trace, leak_oracle_facts)| {
        RecordedRealTrace {
            trace,
            sandbox,
            leak_oracle_facts,
        }
    })
}

/// Record every required review profile in order into one stable namespace.
pub(super) fn record_real_review_corpus_in(
    namespace: StableSandboxNamespace,
    operations: &[CorpusOperation],
    phase: StablePairPhase,
) -> Result<RecordedRealCorpus, String> {
    let mut profiles = Vec::new();
    for factory in required_corpus_factories()? {
        profiles.push(record_corpus_profile(
            &factory,
            namespace.clone(),
            operations,
            phase,
        )?);
    }
    RecordedRealCorpus::new(profiles)
}

fn finish_smoke_trace(
    sink: SchemaThreeSink,
    successful_operations: &[SmokeOperation],
) -> Result<RealTrace, String> {
    sink.finish(
        successful_operations
            .iter()
            .copied()
            .map(smoke_operation_value)
            .collect(),
        smoke_operation_value(SmokeOperation::FinalLiveness),
    )
}

fn run_smoke_prefix(
    engine: &mut WalkerEngine<RealFrontend, RealHostBoundary>,
    sink: &mut SchemaThreeSink,
    prefix: &[SmokeOperation],
) -> Result<(), String> {
    engine.start(sink)?;
    for operation in prefix {
        engine.run_operation(*operation, sink)?;
    }
    engine.run_final_liveness(SmokeOperation::FinalLiveness, sink)
}

fn close_after_primary(host: RealWalkerHost, primary: String) -> String {
    primary_with_cleanup(primary, host.close().err())
}

fn primary_with_cleanup(primary: String, cleanup: Option<impl std::fmt::Display>) -> String {
    match cleanup {
        None => primary,
        Some(cleanup) => format!("{primary}; sandbox cleanup also failed: {cleanup}"),
    }
}

fn close_random_engine(engine: WalkerEngine<RealFrontend, RealHostBoundary>) -> Result<(), String> {
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    host.close().map_err(|error| error.to_string())
}

fn random_trace_failure(
    engine: WalkerEngine<RealFrontend, RealHostBoundary>,
    sink: SchemaThreeSink,
    primary: String,
) -> String {
    drop(sink);
    primary_with_cleanup(primary, close_random_engine(engine).err())
}

fn record_schema_three_random(
    factory: &RealWalkerFactory,
    prefix: &[SmokeOperation],
) -> Result<RecordedRealTrace, String> {
    record_schema_three_random_with_hooks(
        factory,
        prefix,
        noop_real_host_boundary,
        new_schema_three_sink,
        read_boundary_sandbox_metadata,
        noop_schema_sink,
        noop_leak_oracle_facts,
    )
}

fn record_schema_three_random_with_hooks(
    factory: &RealWalkerFactory,
    prefix: &[SmokeOperation],
    after_frontend: impl FnOnce(&mut RealHostBoundary),
    make_sink: impl FnOnce(&RealWalkerFactory) -> Result<SchemaThreeSink, String>,
    sandbox_metadata: impl FnOnce(&RealHostBoundary, &SafeProfileId) -> Result<SandboxMetadata, String>,
    before_finish: impl FnOnce(&mut SchemaThreeSink),
    after_facts: impl FnOnce(&LeakOracleFacts),
) -> Result<RecordedRealTrace, String> {
    let (frontend, mut host) = factory.create()?;
    after_frontend(&mut host);
    let mut engine = WalkerEngine::new(frontend, host, factory.effect_limit())
        .expect("the real walker effect limit is positive");
    let mut sink = match make_sink(factory) {
        Ok(sink) => sink,
        Err(primary) => {
            return Err(primary_with_cleanup(
                primary,
                close_random_engine(engine).err(),
            ));
        }
    };
    if let Err(primary) = run_smoke_prefix(&mut engine, &mut sink, prefix) {
        return Err(random_trace_failure(engine, sink, primary));
    }
    let successful_operations = engine.successful_operations().to_vec();
    let leak_oracle_facts = engine.host().leak_oracle_facts();
    let sandbox = match sandbox_metadata(engine.host(), &factory.review_profile) {
        Ok(metadata) => metadata,
        Err(primary) => return Err(random_trace_failure(engine, sink, primary)),
    };
    before_finish(&mut sink);
    let trace = match finish_smoke_trace(sink, &successful_operations) {
        Ok(trace) => trace,
        Err(primary) => {
            return Err(primary_with_cleanup(
                primary,
                close_random_engine(engine).err(),
            ));
        }
    };
    after_facts(&leak_oracle_facts);
    close_random_engine(engine)?;
    Ok(RecordedRealTrace {
        trace,
        sandbox,
        leak_oracle_facts,
    })
}

fn noop_real_host_boundary(_host: &mut RealHostBoundary) {}

fn read_boundary_sandbox_metadata(
    host: &RealHostBoundary,
    review_profile: &SafeProfileId,
) -> Result<SandboxMetadata, String> {
    host.sandbox_metadata(review_profile)
        .map_err(|error| error.to_string())
}

fn record_schema_three_main(factory: &RealWalkerFactory) -> Result<RecordedRealTrace, String> {
    record_schema_three_random(factory, &[SmokeOperation::OpenRun])
}

fn record_schema_three_replay(factory: &RealWalkerFactory) -> Result<RecordedRealTrace, String> {
    record_schema_three_random(factory, &[SmokeOperation::OpenRun])
}

fn stable_pair_error_from_host(phase: StablePairPhase, error: StableHostError) -> StablePairError {
    match error {
        StableHostError::Sandbox(primary) => StablePairError {
            phase,
            primary: StablePairPrimaryError::Sandbox(Box::new(primary)),
            cleanup: None,
        },
        StableHostError::Primary { primary, cleanup } => StablePairError {
            phase,
            primary: StablePairPrimaryError::Seed(primary),
            cleanup: cleanup.map(Box::new),
        },
    }
}

fn stable_host_failure(
    phase: StablePairPhase,
    host: RealWalkerHost,
    primary: StablePairPrimaryError,
) -> StablePairError {
    StablePairError {
        phase,
        primary,
        cleanup: host.close().err().map(Box::new),
    }
}

fn stable_engine_failure(
    phase: StablePairPhase,
    engine: WalkerEngine<RealFrontend, RealHostBoundary>,
    sink: SchemaThreeSink,
    primary: StablePairPrimaryError,
) -> StablePairError {
    drop(sink);
    let (frontend, host) = engine.into_parts();
    drop(frontend);
    StablePairError {
        phase,
        primary,
        cleanup: host.close().err().map(Box::new),
    }
}

fn record_schema_three_stable(
    factory: &RealWalkerFactory,
    namespace: StableSandboxNamespace,
    phase: StablePairPhase,
    prefix: &[SmokeOperation],
) -> Result<(RecordedRealTrace, Vec<SmokeOperation>), StablePairError> {
    record_schema_three_stable_with_hooks(
        factory,
        namespace,
        phase,
        prefix,
        read_sandbox_metadata,
        |host| host.initial_state().map_err(|error| error.to_string()),
        noop_stable_host,
        new_schema_three_sink,
        noop_schema_sink,
        noop_leak_oracle_facts,
        noop_stable_host,
    )
}

fn noop_stable_host(_host: &mut RealWalkerHost) {}

fn noop_schema_sink(_sink: &mut SchemaThreeSink) {}

fn noop_leak_oracle_facts(_facts: &LeakOracleFacts) {}

#[allow(clippy::too_many_arguments)]
fn record_schema_three_stable_with_hooks(
    factory: &RealWalkerFactory,
    namespace: StableSandboxNamespace,
    phase: StablePairPhase,
    prefix: &[SmokeOperation],
    sandbox_metadata: impl FnOnce(
        &RealWalkerHost,
        &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError>,
    initial_state: impl FnOnce(&RealWalkerHost) -> Result<LibraryState, String>,
    after_frontend: impl FnOnce(&mut RealWalkerHost),
    make_sink: impl FnOnce(&RealWalkerFactory) -> Result<SchemaThreeSink, String>,
    before_finish: impl FnOnce(&mut SchemaThreeSink),
    after_facts: impl FnOnce(&LeakOracleFacts),
    before_close: impl FnOnce(&mut RealWalkerHost),
) -> Result<(RecordedRealTrace, Vec<SmokeOperation>), StablePairError> {
    let mut host = RealWalkerHost::spawn_stable_in(
        factory.seed.clone(),
        factory.review_profile.clone(),
        namespace,
    )
    .map_err(|error| stable_pair_error_from_host(phase, error))?;
    let sandbox = match sandbox_metadata(&host, &factory.review_profile) {
        Ok(metadata) => metadata,
        Err(primary) => {
            return Err(stable_host_failure(
                phase,
                host,
                StablePairPrimaryError::Sandbox(Box::new(primary)),
            ));
        }
    };
    let state = match initial_state(&host) {
        Ok(state) => state,
        Err(primary) => {
            return Err(stable_host_failure(
                phase,
                host,
                StablePairPrimaryError::InitialState(primary),
            ));
        }
    };
    let frontend = match RealFrontend::new(state, factory.locale, factory.size) {
        Ok(frontend) => frontend,
        Err(primary) => {
            return Err(stable_host_failure(
                phase,
                host,
                StablePairPrimaryError::Frontend(primary),
            ));
        }
    };
    after_frontend(&mut host);
    let boundary = RealHostBoundary { host };
    let mut engine = WalkerEngine::new(frontend, boundary, factory.effect_limit())
        .expect("the real walker effect limit is positive");
    let mut sink = match make_sink(factory) {
        Ok(sink) => sink,
        Err(primary) => {
            let (frontend, host) = engine.into_parts();
            drop(frontend);
            return Err(StablePairError {
                phase,
                primary: StablePairPrimaryError::Trace(primary),
                cleanup: host.close().err().map(Box::new),
            });
        }
    };
    if let Err(primary) = run_smoke_prefix(&mut engine, &mut sink, prefix) {
        return Err(stable_engine_failure(
            phase,
            engine,
            sink,
            StablePairPrimaryError::Trace(primary),
        ));
    }
    let successful_operations = engine.successful_operations().to_vec();
    let leak_oracle_facts = engine.host().leak_oracle_facts();
    before_finish(&mut sink);
    let trace = match finish_smoke_trace(sink, &successful_operations) {
        Ok(trace) => trace,
        Err(primary) => {
            let (frontend, host) = engine.into_parts();
            drop(frontend);
            return Err(StablePairError {
                phase,
                primary: StablePairPrimaryError::Trace(primary),
                cleanup: host.close().err().map(Box::new),
            });
        }
    };
    after_facts(&leak_oracle_facts);
    let (frontend, mut host) = engine.into_parts();
    drop(frontend);
    before_close(&mut host.host);
    host.close().map_err(|primary| StablePairError {
        phase,
        primary: StablePairPrimaryError::Close(Box::new(primary)),
        cleanup: None,
    })?;
    Ok((
        RecordedRealTrace {
            trace,
            sandbox,
            leak_oracle_facts,
        },
        successful_operations,
    ))
}

fn read_sandbox_metadata(
    host: &RealWalkerHost,
    review_profile: &SafeProfileId,
) -> Result<SandboxMetadata, SandboxError> {
    host.sandbox_metadata(review_profile)
}

fn smoke_factory() -> RealWalkerFactory {
    RealWalkerFactory {
        seed: smoke_seed(),
        review_profile: SafeProfileId::try_from("engine-smoke-60x24")
            .expect("the smoke review profile is valid"),
        locale: Locale::En,
        size: Size::new(60, 24),
    }
}

fn canonical_corpus_seed(locale: Locale) -> WalkerSeedSpec {
    let mut pattern = ParamDecl::new("pattern");
    pattern.multiple = true;
    let source = b"#!/bin/sh\nprintf '%s\\n' \"$1\"\n".to_vec();
    WalkerSeedSpec {
        profile: "canonical-corpus".to_owned(),
        entries: vec![CreateEntry {
            name: "A Corpus Tool".to_owned(),
            kind: EntryKind::parse("shell").expect("shell is a supported kind"),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: "Real-host corpus entry.".to_owned(),
            payload: Some(EntryPayload {
                bytes: source,
                stored_name: Some("corpus.sh".to_owned()),
                permissions: SourcePermissions {
                    readonly: false,
                    unix_mode: Some(0o640),
                },
            }),
            settings: EntrySettings {
                params: vec![pattern.name.clone()],
                parameters: vec![pattern],
                ..EntrySettings::default()
            },
        }],
        settings: BTreeMap::from([
            ("after_run".to_owned(), "stay".to_owned()),
            ("lang".to_owned(), locale.tag().to_owned()),
        ]),
        runners: vec![PromptRunner {
            name: "seed-agent".to_owned(),
            argv: vec![
                "corpus-agent".to_owned(),
                "--message".to_owned(),
                "{{prompt}}".to_owned(),
            ],
        }],
        forms: vec![WalkerFormSeed {
            selector: "A Corpus Tool".to_owned(),
            values: BTreeMap::new(),
            extra_args: vec!["seed-tail".to_owned()],
            extra_args_raw: false,
            preset: Some("seed".to_owned()),
            last_run: Some(WalkerLastRunSeed {
                exit: 0,
                at: "2026-09-02T12:00:00+00:00".to_owned(),
                values: Some(BTreeMap::new()),
            }),
        }],
        prompt_runner: "seed-agent".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("picked.sh"),
            bytes: b"#!/bin/sh\nprintf picked\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        }],
        external_references: Vec::new(),
        directories: vec![
            WalkerDirectorySeed {
                root: WalkerDirectoryRoot::Home,
                path: PathBuf::from(".codex/skills"),
            },
            WalkerDirectorySeed {
                root: WalkerDirectoryRoot::Cwd,
                path: PathBuf::from(".claude/skills"),
            },
        ],
        editor_writes: vec![
            b"#!/bin/sh\nprintf edited\n".to_vec(),
            b"#!/bin/sh\nprintf kept\n".to_vec(),
            b"Write {{topic}} as a short note.\n".to_vec(),
            b"Write {{topic}} as a revised note.\n".to_vec(),
        ],
    }
}

fn required_corpus_factories() -> Result<Vec<RealWalkerFactory>, String> {
    skit_tui_walker_support::required_review_profiles()
        .iter()
        .map(|required| {
            let locale = detect_locale(Some(required.locale));
            Ok(RealWalkerFactory {
                seed: canonical_corpus_seed(locale),
                review_profile: SafeProfileId::try_from(required.id)
                    .map_err(|error| error.to_string())?,
                locale,
                size: Size::new(required.viewport.width, required.viewport.height),
            })
        })
        .collect()
}

pub(super) fn canonical_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let control = |character| {
        CorpusKeyEvent::press(
            CorpusKey::Character(character),
            CorpusModifiers {
                control: true,
                ..CorpusModifiers::NONE
            },
        )
    };
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    let preferences_focus =
        |target| CorpusOperation::ScreenFocus(ScreenTarget::Preferences(target));
    let preferences_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Preferences(target));
    let runner_hit = |name: &str| {
        CorpusOperation::ScreenHit(ScreenTarget::Runner {
            name: name.to_owned(),
        })
    };
    let mut operations = vec![
        CorpusOperation::Focus { gained: false },
        CorpusOperation::CommandKeyboard(UiCommand::Run),
        CorpusOperation::HitTarget(HitTarget::FocusField(1)),
        CorpusOperation::Paste("*".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::SavePreset),
        CorpusOperation::Paste("corpus".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Rerun),
        CorpusOperation::CommandKeyboard(UiCommand::Presets),
        CorpusOperation::CommandKeyboard(UiCommand::CloseSettings),
        CorpusOperation::CommandKeyboard(UiCommand::Settings),
        CorpusOperation::CommandKeyboard(UiCommand::SaveSettings),
        CorpusOperation::CommandKeyboard(UiCommand::Edit),
        CorpusOperation::CommandKeyboard(UiCommand::Rename),
        CorpusOperation::Paste("-renamed".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Health),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Health(HealthAction::Rebuild),
            key: control('r'),
        },
        CorpusOperation::LocalHit(LocalActionTarget::Health(HealthAction::Back)),
        CorpusOperation::CommandKeyboard(UiCommand::Runners),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::Back),
            key: plain(CorpusKey::Escape),
        },
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        CorpusOperation::CommandKeyboard(UiCommand::SavePreferences),
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        preferences_focus(PreferencesControlId::InstallAgentSkill),
        preferences_hit(PreferencesControlId::InstallAgentSkill),
        CorpusOperation::ScreenHit(ScreenTarget::AgentSkill {
            name: "codex".to_owned(),
            scope: AgentScope::User,
        }),
        preferences_focus(PreferencesControlId::ManageAgents),
        preferences_hit(PreferencesControlId::ManageAgents),
        runner_hit("claude"),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::RemoveSelected),
            key: plain(CorpusKey::Character('d')),
        },
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::ConfirmRemove),
            key: plain(CorpusKey::Character('y')),
        },
        runner_hit("codex"),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::RemoveSelected),
            key: plain(CorpusKey::Character('d')),
        },
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::ConfirmRemove),
            key: plain(CorpusKey::Character('y')),
        },
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Runners(RunnerManagerAction::Back),
            key: plain(CorpusKey::Escape),
        },
        CorpusOperation::CommandKeyboard(UiCommand::ClosePreferences),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_hit(AddControlId::BrowseSource),
        CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("picked.sh"),
        }),
        add_focus(AddControlId::Continue),
        add_local(AddControlId::Continue, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewScript),
        add_local(AddControlId::NewScript, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::Draft(0)),
        add_local(AddControlId::Draft(0), plain(CorpusKey::Enter)),
        add_focus(AddControlId::DeleteDraft),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewPrompt),
        add_local(AddControlId::NewPrompt, plain(CorpusKey::Enter)),
        add_focus(AddControlId::Interpolate),
        add_hit(AddControlId::Interpolate),
        add_focus(AddControlId::EditSource),
        add_local(AddControlId::EditSource, control('e')),
        add_focus(AddControlId::Text(AddTextField::ReviewName)),
        CorpusOperation::RawKey(control('u')),
        CorpusOperation::Paste("Corpus Prompt".to_owned()),
        add_focus(AddControlId::NewRunner),
        add_hit(AddControlId::NewRunner),
        CorpusOperation::Paste("new-agent".to_owned()),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::RunnerEditor(RunnerEditorAction::FocusNext),
            key: plain(CorpusKey::Tab),
        },
        CorpusOperation::Paste("agent {{prompt}}".to_owned()),
        CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::RunnerEditor(RunnerEditorAction::Submit),
            key: plain(CorpusKey::Enter),
        },
        add_focus(AddControlId::Save),
        add_local(AddControlId::Save, control('s')),
        CorpusOperation::CommandKeyboard(UiCommand::Remove),
        CorpusOperation::CommandKeyboard(UiCommand::Submit),
        CorpusOperation::CommandKeyboard(UiCommand::Reload),
        CorpusOperation::CommandKeyboard(UiCommand::Search),
        CorpusOperation::Paste("Corpus".to_owned()),
        CorpusOperation::CommandKeyboard(UiCommand::ClearSearch),
        CorpusOperation::CommandKeyboard(UiCommand::LeaveSearch),
        CorpusOperation::CommandKeyboard(UiCommand::ToggleDetail),
        CorpusOperation::CommandKeyboard(UiCommand::ToggleDetail),
        CorpusOperation::CommandKeyboard(UiCommand::Help),
        CorpusOperation::CommandKeyboard(UiCommand::CloseModal),
        CorpusOperation::CommandKeyboard(UiCommand::Next),
        CorpusOperation::CommandKeyboard(UiCommand::Previous),
        CorpusOperation::CommandKeyboard(UiCommand::Home),
        CorpusOperation::CommandKeyboard(UiCommand::End),
        CorpusOperation::CommandKeyboard(UiCommand::Run),
        CorpusOperation::CommandKeyboard(UiCommand::Back),
        CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Run)),
        CorpusOperation::CommandKeyboard(UiCommand::Back),
        CorpusOperation::CommandKeyboard(UiCommand::ToggleDetail),
        CorpusOperation::CommandKeyboard(UiCommand::Help),
        CorpusOperation::CommandKeyboard(UiCommand::CloseModal),
        CorpusOperation::Focus { gained: false },
    ];
    assert_eq!(operations.len(), 98);
    operations.push(CorpusOperation::Resize {
        width: 24,
        height: 6,
    });
    operations.push(CorpusOperation::Focus { gained: true });
    operations
}

/// One corpus operation that the resolver refuses in every profile.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) fn invalid_corpus_operation() -> CorpusOperation {
    CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
        relative: PathBuf::from("../outside"),
    })
}

/// The short viewport-independent operation vector for review corpus contracts.
///
/// Every required profile completes it with no refused operation, and it records one host row.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) fn review_corpus_contract_operations() -> Vec<CorpusOperation> {
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Run),
        CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Escape,
            CorpusModifiers::NONE,
        )),
        CorpusOperation::Focus { gained: false },
    ]
}

/// The short viewport-independent operation vector for the draft allocator contracts.
///
/// It authors one draft, opens the draft again, and confirms one deletion. The deletion
/// quarantines the draft file, so the recording reaches the draft quarantine allocator.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) fn draft_quarantine_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewScript),
        add_local(AddControlId::NewScript, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::Draft(0)),
        add_local(AddControlId::Draft(0), plain(CorpusKey::Enter)),
        add_focus(AddControlId::DeleteDraft),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::DeleteDraft, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
    ]
}

/// The short operation vector that cuts the derived Add review name.
///
/// It authors one prompt draft, reaches the review name, and cuts that name with `Control+U`.
/// The cut name stays in the input cut buffer after the paste replaces the value.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) fn review_name_cut_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let control = |character| {
        CorpusKeyEvent::press(
            CorpusKey::Character(character),
            CorpusModifiers {
                control: true,
                ..CorpusModifiers::NONE
            },
        )
    };
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_focus(AddControlId::NewPrompt),
        add_local(AddControlId::NewPrompt, plain(CorpusKey::Enter)),
        add_focus(AddControlId::Interpolate),
        add_hit(AddControlId::Interpolate),
        add_focus(AddControlId::EditSource),
        add_local(AddControlId::EditSource, control('e')),
        add_focus(AddControlId::Text(AddTextField::ReviewName)),
        CorpusOperation::RawKey(control('u')),
        CorpusOperation::Paste("Corpus Prompt".to_owned()),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
    ]
}

/// The short operation vector that paints an absolute sandbox path on a real screen.
///
/// It installs one Agent Skill, opens the source file picker, and picks one seeded file. The
/// Agent Skill status, the picker directory header, and the Add source field each paint a path
/// that a declared sandbox root holds.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) fn rendered_sandbox_root_corpus_operations() -> Vec<CorpusOperation> {
    let plain = |code| CorpusKeyEvent::press(code, CorpusModifiers::NONE);
    let add_focus = |target| CorpusOperation::ScreenFocus(ScreenTarget::Add(target));
    let add_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Add(target));
    let add_local = |target, key| CorpusOperation::LocalKeyboard {
        target: LocalActionTarget::Add(target),
        key,
    };
    let preferences_focus =
        |target| CorpusOperation::ScreenFocus(ScreenTarget::Preferences(target));
    let preferences_hit = |target| CorpusOperation::ScreenHit(ScreenTarget::Preferences(target));
    vec![
        CorpusOperation::CommandKeyboard(UiCommand::Preferences),
        preferences_focus(PreferencesControlId::InstallAgentSkill),
        preferences_hit(PreferencesControlId::InstallAgentSkill),
        CorpusOperation::ScreenHit(ScreenTarget::AgentSkill {
            name: "codex".to_owned(),
            scope: AgentScope::User,
        }),
        CorpusOperation::CommandKeyboard(UiCommand::ClosePreferences),
        CorpusOperation::CommandKeyboard(UiCommand::Add),
        add_hit(AddControlId::BrowseSource),
        CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("picked.sh"),
        }),
        add_focus(AddControlId::Continue),
        add_local(AddControlId::Continue, plain(CorpusKey::Enter)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
        add_local(AddControlId::Cancel, plain(CorpusKey::Escape)),
    ]
}

pub(super) fn canonical_corpus_artifact_values() -> Result<(Vec<Value>, Value), String> {
    let operations = canonical_corpus_operations()
        .iter()
        .map(corpus_operation_value)
        .collect::<Result<Vec<_>, _>>()?;
    let final_liveness = corpus_operation_value(&CorpusOperation::FinalLiveness)?;
    Ok((operations, final_liveness))
}

pub(super) fn record_real_smoke_main() -> Result<RecordedRealTrace, String> {
    record_schema_three_main(&smoke_factory())
}

pub(super) fn record_real_smoke_replay() -> Result<RecordedRealTrace, String> {
    record_schema_three_replay(&smoke_factory())
}

pub(super) fn record_real_locale_smoke(
    target: LocaleSmokeTarget,
) -> Result<RecordedRealTrace, String> {
    let mut operations = vec![
        SmokeOperation::OpenPreferences,
        SmokeOperation::OpenLanguagePicker,
    ];
    for _ in 0..target.next_count() {
        operations.push(SmokeOperation::NextLanguage);
    }
    operations.extend([
        SmokeOperation::ChooseLanguage,
        SmokeOperation::SavePreferences,
        SmokeOperation::OpenRun,
    ]);
    // Reference details paint the random directory after the locale changes. Keep this
    // locale fixture on a copied entry; stable pairs cover rendered reference paths.
    let mut factory = smoke_factory();
    let entry = &mut factory.seed.external_references[0].request;
    entry.mode = StorageMode::Copy;
    entry.workdir = "store".to_owned();
    record_schema_three_random(&factory, &operations)
}

pub(super) fn record_real_smoke_stable_pair_in(
    namespace: StableSandboxNamespace,
) -> Result<StableTracePair, StablePairError> {
    let factory = smoke_factory();
    let (main, successful_operations) = record_schema_three_stable(
        &factory,
        namespace.clone(),
        StablePairPhase::Main,
        &[SmokeOperation::OpenRun],
    )?;
    let (replay, replayed_operations) = record_schema_three_stable(
        &factory,
        namespace,
        StablePairPhase::Replay,
        &successful_operations,
    )?;
    debug_assert_eq!(replayed_operations, successful_operations);
    Ok(StableTracePair { main, replay })
}

fn smoke_seed() -> WalkerSeedSpec {
    let bytes = b"printf smoke\n".to_vec();
    WalkerSeedSpec {
        profile: "engine-smoke".to_owned(),
        external: vec![WalkerExternalSeed::File {
            path: PathBuf::from("original.sh"),
            bytes: bytes.clone(),
            readonly: false,
            unix_mode: 0o640,
        }],
        external_references: vec![WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Reference".to_owned(),
                kind: EntryKind::parse("shell").expect("shell is a supported kind"),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: "real walker reference".to_owned(),
                payload: Some(EntryPayload {
                    bytes,
                    stored_name: Some("original.sh".to_owned()),
                    permissions: SourcePermissions {
                        readonly: false,
                        unix_mode: Some(0o640),
                    },
                }),
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("original.sh"),
        }],
        ..WalkerSeedSpec::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use skit_ui::{HealthAction, PreferencesAction, RunnerEditorAction, RunnerManagerAction};

    #[derive(Default)]
    struct CorpusRefusalSink {
        refusals: Vec<(usize, CorpusNotApplicable)>,
        boundaries: Vec<EngineBoundary>,
        last: Option<(EnginePhase, Option<usize>)>,
    }

    impl CheckpointSink<CheckpointFor<CorpusFrontend, CorpusHostBoundary>> for CorpusRefusalSink {
        fn record(
            &mut self,
            checkpoint: CheckpointFor<CorpusFrontend, CorpusHostBoundary>,
        ) -> Result<(), String> {
            let operation_index = match &checkpoint.cause {
                EngineCause::Initial => None,
                EngineCause::NotApplicable {
                    operation_index,
                    resolved,
                    ..
                } => {
                    let refusal = operation_index
                        .map(|index| {
                            resolved
                                .refusal
                                .ok_or(
                                    "a not-applicable corpus operation has no refusal".to_owned(),
                                )
                                .map(|refusal| (index, refusal))
                        })
                        .transpose()?;
                    self.refusals.extend(refusal);
                    *operation_index
                }
                EngineCause::Session {
                    operation_index, ..
                }
                | EngineCause::Reducer {
                    operation_index, ..
                }
                | EngineCause::Host {
                    operation_index, ..
                } => *operation_index,
            };
            self.boundaries.push(checkpoint.boundary);
            self.last = Some((checkpoint.phase, operation_index));
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    fn synthetic_stable_root(alternate: bool) -> &'static str {
        if alternate {
            "/tmp/other/skit-ui-walker-v1"
        } else {
            "/tmp/skit-ui-walker-v1"
        }
    }

    #[cfg(target_os = "windows")]
    fn synthetic_stable_root(alternate: bool) -> &'static str {
        if alternate {
            r"C:\tmp\other\skit-ui-walker-v1"
        } else {
            r"C:\tmp\skit-ui-walker-v1"
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn synthetic_recorded_corpus_profiles() -> Vec<RecordedRealTrace> {
        let template = record_real_smoke_main().unwrap();
        let operations = canonical_corpus_operations()
            .iter()
            .map(corpus_operation_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let final_liveness = corpus_operation_value(&CorpusOperation::FinalLiveness).unwrap();
        let root = synthetic_stable_root(false).to_owned();
        skit_tui_walker_support::required_review_profiles()
            .iter()
            .map(|required| {
                let id = SafeProfileId::try_from(required.id).unwrap();
                let mut recorded = template.clone();
                recorded.trace.review_profile = id.clone();
                recorded.trace.locale = required.locale.to_owned();
                recorded.trace.viewport = required.viewport;
                recorded.trace.operations = operations.clone();
                recorded.trace.final_liveness_requested = final_liveness.clone();
                for row in &mut recorded.trace.rows {
                    row.profile = id.clone();
                    row.locale = required.locale.to_owned();
                }
                recorded.sandbox = SandboxMetadata::stable(
                    template.sandbox.platform(),
                    root.clone(),
                    BTreeSet::from([id]),
                )
                .unwrap();
                recorded.leak_oracle_facts = LeakOracleFacts::default();
                recorded
            })
            .collect()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn recorded_corpus_requires_exact_shared_profiles_and_values() {
        let profiles = synthetic_recorded_corpus_profiles();
        let corpus = RecordedRealCorpus::new(profiles.clone()).unwrap();
        assert_eq!(corpus.profiles.len(), 4);
        assert_eq!(corpus.sandbox.profiles().len(), 4);
        let mut independent_facts = profiles.clone();
        independent_facts[0].leak_oracle_facts.artifacts.insert(
            "projected-only".to_owned(),
            ArtifactLeakOracleFact::default(),
        );
        let pair = StableCorpusTracePair::new(
            corpus.clone(),
            RecordedRealCorpus::new(independent_facts).unwrap(),
        )
        .unwrap();
        assert_eq!(pair.main.sandbox, pair.replay.sandbox);
        assert_ne!(
            pair.main.profiles[0].leak_oracle_facts,
            pair.replay.profiles[0].leak_oracle_facts
        );

        let mut missing = profiles.clone();
        missing.pop();
        assert_eq!(
            RecordedRealCorpus::new(missing).unwrap_err(),
            "the recorded corpus does not contain the exact four profiles"
        );

        let mut extra = profiles.clone();
        extra.push(profiles[0].clone());
        assert_eq!(
            RecordedRealCorpus::new(extra).unwrap_err(),
            "the recorded corpus does not contain the exact four profiles"
        );

        let mut wrong_order = profiles.clone();
        wrong_order[0].trace.review_profile = profiles[1].review_profile.clone();
        assert_eq!(
            RecordedRealCorpus::new(wrong_order).unwrap_err(),
            "the recorded corpus profiles are not in required order"
        );

        let mut wrong_locale = profiles.clone();
        wrong_locale[3].trace.locale = "en".to_owned();
        assert_eq!(
            RecordedRealCorpus::new(wrong_locale).unwrap_err(),
            "a recorded corpus profile has the wrong locale or viewport"
        );

        let mut wrong_viewport = profiles.clone();
        wrong_viewport[3].trace.viewport.width = 1;
        assert_eq!(
            RecordedRealCorpus::new(wrong_viewport).unwrap_err(),
            "a recorded corpus profile has the wrong locale or viewport"
        );

        let mut wrong_operation = profiles.clone();
        wrong_operation[3].trace.operations[0] = json!({"wrong": true});
        assert_eq!(
            RecordedRealCorpus::new(wrong_operation).unwrap_err(),
            "recorded corpus profiles do not share one operation vector"
        );

        let mut wrong_liveness = profiles.clone();
        wrong_liveness[3].trace.final_liveness_requested = json!({"wrong": true});
        assert_eq!(
            RecordedRealCorpus::new(wrong_liveness).unwrap_err(),
            "recorded corpus profiles do not share one operation vector"
        );

        let mut empty_operations = profiles.clone();
        for recorded in &mut empty_operations {
            recorded.trace.operations.clear();
        }
        assert_eq!(
            RecordedRealCorpus::new(empty_operations).unwrap_err(),
            "the recorded corpus has an empty operation vector"
        );

        let mut no_liveness = profiles.clone();
        no_liveness[3].trace.rows.last_mut().unwrap().liveness = None;
        assert_eq!(
            RecordedRealCorpus::new(no_liveness).unwrap_err(),
            "a recorded corpus profile did not pass final liveness"
        );

        let mut refused = profiles.clone();
        refused[3].trace.rows[0].cause = TransitionCause::Session {
            requested: Value::Null,
            resolved: json!({"refusal": "unavailable"}),
            event: Value::Null,
            handling: json!("not_applicable"),
        };
        assert_eq!(
            RecordedRealCorpus::new(refused).unwrap_err(),
            "a recorded corpus profile contains a refused operation"
        );

        let mut wrong_timeline = profiles.clone();
        wrong_timeline[3].trace.rows[0].profile = wrong_timeline[0].review_profile.clone();
        assert_eq!(
            RecordedRealCorpus::new(wrong_timeline).unwrap_err(),
            "a recorded corpus timeline has the wrong profile"
        );

        let platform = profiles[3].sandbox.platform();
        let shared_root = profiles[3].sandbox.root().to_owned();

        let mut split_root = profiles.clone();
        split_root[3].sandbox = SandboxMetadata::stable(
            platform,
            synthetic_stable_root(true),
            BTreeSet::from([split_root[3].review_profile.clone()]),
        )
        .unwrap();
        assert_eq!(
            RecordedRealCorpus::new(split_root).unwrap_err(),
            "recorded corpus sandboxes do not share one stable namespace"
        );

        let mut random = profiles.clone();
        random[3].sandbox = SandboxMetadata::random(
            platform,
            shared_root.clone(),
            random[3].review_profile.clone(),
        )
        .unwrap();
        assert_eq!(
            RecordedRealCorpus::new(random).unwrap_err(),
            "recorded corpus sandboxes do not share one stable namespace"
        );

        let mut wrong_singleton = profiles.clone();
        wrong_singleton[3].sandbox = SandboxMetadata::stable(
            platform,
            shared_root,
            BTreeSet::from([SafeProfileId::try_from("wrong-profile").unwrap()]),
        )
        .unwrap();
        assert_eq!(
            RecordedRealCorpus::new(wrong_singleton).unwrap_err(),
            "a recorded corpus profile has invalid singleton metadata"
        );

        let mut changed_trace = RecordedRealCorpus::new(profiles.clone()).unwrap();
        changed_trace.profiles[3].trace.cast.push(b'X');
        assert_eq!(
            StableCorpusTracePair::new(corpus.clone(), changed_trace).unwrap_err(),
            "stable corpus main and replay traces differ at pseudo-120x12"
        );

        let mut other_root = profiles;
        for recorded in &mut other_root {
            recorded.sandbox = SandboxMetadata::stable(
                platform,
                synthetic_stable_root(true),
                BTreeSet::from([recorded.review_profile.clone()]),
            )
            .unwrap();
        }
        let other_root = RecordedRealCorpus::new(other_root).unwrap();
        assert_eq!(
            StableCorpusTracePair::new(corpus, other_root).unwrap_err(),
            "stable corpus generations use different aggregate metadata"
        );
    }

    #[test]
    fn corpus_result_preserves_primary_and_cleanup_errors() {
        assert_eq!(
            combine_corpus_result_after_close(Ok(7), Ok::<(), &str>(())),
            Ok(7)
        );
        assert_eq!(
            combine_corpus_result_after_close(Ok(7), Err("close")),
            Err("could not close the real corpus host: close".to_owned())
        );
        assert_eq!(
            combine_corpus_result_after_close::<u8>(Err("primary".to_owned()), Ok::<(), &str>(())),
            Err("primary".to_owned())
        );
        assert_eq!(
            combine_corpus_result_after_close::<u8>(Err("primary".to_owned()), Err("close")),
            Err("primary; real corpus host cleanup also failed: close".to_owned())
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn corpus_host_preparation_closes_metadata_and_state_failures() {
        let profile = smoke_factory().review_profile;
        let context = |message| format!("context: {message}");

        let metadata_parent = tempfile::TempDir::new().unwrap();
        let metadata_namespace =
            StableSandboxNamespace::explicit(metadata_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let metadata_host = RealWalkerHost::spawn_stable_in(
            smoke_seed(),
            profile.clone(),
            metadata_namespace.clone(),
        )
        .unwrap();
        let metadata_error = prepare_corpus_host_with(
            metadata_host,
            &profile,
            &context,
            |_, _| Err("metadata".to_owned()),
            read_initial_state,
        )
        .unwrap_err();
        assert_eq!(metadata_error, "context: metadata");
        record_corpus_profile(
            &smoke_factory(),
            metadata_namespace,
            &[CorpusOperation::Focus { gained: false }],
            StablePairPhase::Main,
        )
        .unwrap();

        let state_parent = tempfile::TempDir::new().unwrap();
        let state_namespace =
            StableSandboxNamespace::explicit(state_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let state_host =
            RealWalkerHost::spawn_stable_in(smoke_seed(), profile.clone(), state_namespace.clone())
                .unwrap();
        let state_error = prepare_corpus_host_with(
            state_host,
            &profile,
            &context,
            |host, profile| {
                host.sandbox_metadata(profile)
                    .map_err(|error| error.to_string())
            },
            |_| Err("state".to_owned()),
        )
        .unwrap_err();
        assert_eq!(state_error, "context: state");
        record_corpus_profile(
            &smoke_factory(),
            state_namespace,
            &[CorpusOperation::Focus { gained: false }],
            StablePairPhase::Main,
        )
        .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn corpus_profile_recorder_reports_phase_and_releases_failures() {
        let replay_parent = tempfile::TempDir::new().unwrap();
        let replay_namespace =
            StableSandboxNamespace::explicit(replay_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let replay = record_corpus_profile(
            &smoke_factory(),
            replay_namespace,
            &[CorpusOperation::Focus { gained: false }],
            StablePairPhase::Replay,
        )
        .unwrap();
        assert_eq!(replay.operations.len(), 1);

        let frontend_parent = tempfile::TempDir::new().unwrap();
        let frontend_namespace =
            StableSandboxNamespace::explicit(frontend_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let mut invalid_frontend = smoke_factory();
        invalid_frontend.size = Size::new(0, 24);
        let error = record_corpus_profile(
            &invalid_frontend,
            frontend_namespace.clone(),
            &[],
            StablePairPhase::Main,
        )
        .unwrap_err();
        assert!(error.contains("engine-smoke-60x24"));
        record_corpus_profile(
            &smoke_factory(),
            frontend_namespace,
            &[CorpusOperation::Focus { gained: false }],
            StablePairPhase::Main,
        )
        .unwrap();

        let operation_parent = tempfile::TempDir::new().unwrap();
        let operation_namespace = StableSandboxNamespace::explicit(
            operation_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        )
        .unwrap();
        let invalid_operation = CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("../outside"),
        });
        let error = record_corpus_profile(
            &smoke_factory(),
            operation_namespace.clone(),
            &[invalid_operation],
            StablePairPhase::Main,
        )
        .unwrap_err();
        assert!(error.contains("operation 1"));
        record_corpus_profile(
            &smoke_factory(),
            operation_namespace,
            &[CorpusOperation::Focus { gained: false }],
            StablePairPhase::Main,
        )
        .unwrap();
    }

    struct FailPreferencesOpenHost {
        inner: RealHostBoundary,
    }

    impl FailPreferencesOpenHost {
        fn new(inner: RealHostBoundary) -> Self {
            Self { inner }
        }

        fn close(self) -> Result<(), String> {
            self.inner.close().map_err(|error| error.to_string())
        }
    }

    impl HostAdapter<RealFrontend> for FailPreferencesOpenHost {
        type Observation = HostObservation;

        fn serve(&mut self, _effect: Effect) -> Result<Action, String> {
            Err("injected Preferences host failure".to_owned())
        }

        fn capture_checkpoint(
            &mut self,
            frontend: FrontendObservation,
            cause: CheckpointCauseProjection<'_, Action, Effect>,
        ) -> Result<CheckpointCapture<FrontendObservation, Self::Observation>, String> {
            self.inner.capture_checkpoint(frontend, cause)
        }
    }

    #[derive(Default)]
    struct BoundarySink(Vec<EngineBoundary>);

    impl CheckpointSink<CheckpointFor<RealFrontend, FailPreferencesOpenHost>> for BoundarySink {
        fn record(
            &mut self,
            checkpoint: CheckpointFor<RealFrontend, FailPreferencesOpenHost>,
        ) -> Result<(), String> {
            self.0.push(checkpoint.boundary);
            Ok(())
        }
    }

    fn reducer_emitted(cause: &mut TransitionCause) -> &mut Value {
        match cause {
            TransitionCause::Reducer { emitted, .. } => emitted,
            TransitionCause::Initial
            | TransitionCause::Session { .. }
            | TransitionCause::Host { .. } => {
                panic!("the test checkpoint must be a reducer action");
            }
        }
    }

    #[test]
    #[should_panic(expected = "the test checkpoint must be a reducer action")]
    fn reducer_emitted_rejects_a_non_reducer_checkpoint() {
        let mut cause = TransitionCause::Initial;
        let _ = reducer_emitted(&mut cause);
    }

    fn prepared_schema_three_sink(factory: &RealWalkerFactory) -> SchemaThreeSink {
        let (frontend, host) = factory.create().unwrap();
        let mut engine = WalkerEngine::new(frontend, host, EFFECT_LIMIT).unwrap();
        let mut sink = new_schema_three_sink(factory).unwrap();
        engine.start(&mut sink).unwrap();
        engine
            .run_operation(SmokeOperation::OpenRun, &mut sink)
            .unwrap();
        engine
            .run_final_liveness(SmokeOperation::FinalLiveness, &mut sink)
            .unwrap();
        sink
    }

    #[test]
    fn checkpoint_leak_names_the_cause_and_keeps_the_sink_unchanged() {
        let factory = smoke_factory();
        let mut sink = prepared_schema_three_sink(&factory);
        let rows = sink.rows.clone();
        let objects = sink.objects.clone();
        let cast = sink.cast_bytes().to_vec();
        let checkpoint = checkpoint_with_cause(EngineCause::Initial);
        let sandbox = SandboxMetadata::stable(
            SandboxPlatform::Linux,
            "/sandbox/skit-ui-walker-v1",
            BTreeSet::from([factory.review_profile]),
        )
        .unwrap();
        let facts = LeakOracleFacts {
            ambient_paths: BTreeSet::from(["/ambient/review-root".to_owned()]),
            ..LeakOracleFacts::default()
        };

        let error = sink
            .record_checkpoint(ProjectedCheckpoint {
                phase: EnginePhase::Operations,
                boundary: EngineBoundary::Host,
                operation_index: Some(7),
                event_chain: None,
                cause: TransitionCause::Host {
                    operation_index: Some(7),
                    round: 0,
                    request: json!({"add": [{"commit": {"entry": {"payload": {
                        "stored_name": "/ambient/review-root/item",
                    }}}}]}),
                    response: json!("none"),
                    emitted: json!("none"),
                },
                frontend: checkpoint.frontend,
                host: checkpoint.host,
                leak_context: Some(CheckpointLeakContext { sandbox, facts }),
            })
            .unwrap_err();

        assert!(error.contains("phase Operations"), "{error}");
        assert!(error.contains("operation index Some(7)"), "{error}");
        assert!(error.contains("cause"), "{error}");
        assert!(
            error.contains("/request/add/0/commit/entry/payload/stored_name"),
            "{error}"
        );
        assert_eq!(sink.rows, rows);
        assert_eq!(sink.objects, objects);
        assert_eq!(sink.cast_bytes(), cast);
    }

    fn trace_frame_text(trace: &RealTrace, row: &TimelineRow) -> String {
        let bytes = &trace.objects[&(row.styled_frame.kind, row.styled_frame.sha256.clone())];
        let value = validate_object(&row.styled_frame, bytes).unwrap();
        let frame: StyledFrameSnapshot = serde_json::from_value(value).unwrap();
        frame.readable_lines().unwrap().join("\n")
    }

    #[test]
    fn smoke_resolution_projection_is_total_for_synthetic_liveness() {
        let event = SmokeEvent {
            key: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        };
        assert_eq!(
            smoke_resolution_value(
                TimelinePhase::Operations,
                SmokeOperation::FinalLiveness,
                SmokeResolution { event },
            ),
            json!({
                "input": {"kind": "event", "event": {
                    "type": "key", "code": "Esc", "modifiers": "",
                }},
                "semantic_target": {"synthetic": "final_liveness"},
            })
        );
    }

    fn open_real_preferences(frontend: &mut RealFrontend, host: &mut RealHostBoundary) {
        let request = frontend.reduce(Action::OpenPreferences).unwrap();
        assert!(matches!(
            &request,
            Effect::Open {
                request: HostRequest::Preferences,
                selector: None,
            }
        ));
        let response = host.serve(request).unwrap();
        assert!(matches!(response, Action::Present(Screen::Preferences(_))));
        assert_eq!(frontend.reduce(response).unwrap(), Effect::None);
    }

    #[test]
    fn reducer_replay_compares_native_paths_without_accepting_schema_or_target_drift() {
        let (mut frontend, mut host) = smoke_factory().create().unwrap();
        open_real_preferences(&mut frontend, &mut host);
        let mut state = frontend.state;
        state.update(Action::Preferences(
            skit_ui::PreferencesAction::PresentAgentSkillTargets(vec![
                skit_application::AgentTarget {
                    name: "codex".to_owned(),
                    scope: skit_application::AgentScope::User,
                    base: PathBuf::from("/declared/agent"),
                },
            ]),
        ));
        let previous = json_value(&state);
        let action = Action::Preferences(skit_ui::PreferencesAction::ActivateAgentSkillTarget(0));
        let effect = state.update(action.clone());
        let current = json_value(&state);
        let action = json_value(&action);
        let mut recorded = json_value(&effect);
        recorded["preferences"]["install_agent_skill"]["skills_dir"] =
            json!("/declared/agent//skills");
        validate_reducer_action(&previous, &current, &action, &recorded).unwrap();
        recorded["preferences"]["install_agent_skill"]["extra"] = json!(true);
        assert!(validate_reducer_action(&previous, &current, &action, &recorded).is_err());
        recorded["preferences"]["install_agent_skill"]
            .as_object_mut()
            .unwrap()
            .remove("extra");
        recorded["preferences"]["install_agent_skill"]["skills_dir"] = json!("/elsewhere/skills");
        assert!(validate_reducer_action(&previous, &current, &action, &recorded).is_err());
        assert!(validate_reducer_action(&previous, &current, &action, &json!(42)).is_err());
    }

    fn checkpoint_with_cause(
        mut cause: EngineCause<
            SmokeOperation,
            SmokeResolution,
            SmokeEvent,
            Action,
            Effect,
            SmokeHandling,
        >,
    ) -> CheckpointFor<RealFrontend, RealHostBoundary> {
        let (mut frontend, mut host) = smoke_factory().create().unwrap();
        let frontend = frontend.observe().unwrap();
        let projection = match &mut cause {
            EngineCause::Initial
            | EngineCause::NotApplicable { .. }
            | EngineCause::Session { .. } => CheckpointCauseProjection::Observation,
            EngineCause::Reducer {
                action, emitted, ..
            } => CheckpointCauseProjection::Reducer { action, emitted },
            EngineCause::Host {
                request,
                response,
                emitted,
                ..
            } => CheckpointCauseProjection::Host {
                request,
                response,
                emitted,
            },
        };
        let CheckpointCapture { frontend, host } =
            host.capture_checkpoint(frontend, projection).unwrap();
        EngineCheckpoint {
            phase: EnginePhase::Operations,
            boundary: EngineBoundary::Session,
            cause,
            frontend,
            host,
        }
    }

    #[test]
    fn real_frontend_and_smoke_sink_reject_out_of_contract_inputs() {
        assert!(
            RealFrontend::new(LibraryState::default(), Locale::En, Size::new(0, 24))
                .err()
                .unwrap()
                .contains("positive")
        );
        let mut invalid_factory = smoke_factory();
        invalid_factory.size = Size::new(0, 24);
        assert!(invalid_factory.create().err().unwrap().contains("positive"));
        let host = RealWalkerHost::spawn(smoke_seed()).unwrap();
        assert!(
            smoke_factory()
                .frontend_for_host_with_initial_state(host, |_| {
                    Err("injected random initial-state failure".to_owned())
                })
                .err()
                .unwrap()
                .contains("injected random initial-state failure")
        );

        let (mut consumed_frontend, _) = smoke_factory().create().unwrap();
        assert_eq!(
            consumed_frontend
                .dispatch(SmokeEvent {
                    key: KeyCode::Char('c'),
                    modifiers: KeyModifiers::CONTROL,
                })
                .unwrap(),
            DispatchOutcome::Session(SmokeHandling::Consumed)
        );

        let (mut frontend, _) = smoke_factory().create().unwrap();
        assert_eq!(
            frontend
                .dispatch(SmokeEvent {
                    key: KeyCode::F(24),
                    modifiers: KeyModifiers::NONE,
                })
                .unwrap(),
            DispatchOutcome::Session(SmokeHandling::Ignored)
        );
        assert_eq!(frontend.effect_route(&Effect::Quit), EffectRoute::Quit);
        assert_eq!(
            frontend.parity_snapshot().unwrap(),
            FrontendParity {
                state: frontend.state.clone(),
                session: frontend.session_value().unwrap(),
                locale: Locale::En,
            }
        );

        let mut trace = MemoryTrace::default();
        assert!(
            trace
                .record(checkpoint_with_cause(EngineCause::NotApplicable {
                    dispatch_sequence: 0,
                    operation_index: Some(0),
                    operation: SmokeOperation::OpenRun,
                    resolved: SmokeResolution {
                        event: SmokeEvent {
                            key: KeyCode::Enter,
                            modifiers: KeyModifiers::NONE,
                        },
                    },
                }))
                .unwrap_err()
                .contains("did not cross")
        );
        assert!(
            trace
                .record(checkpoint_with_cause(EngineCause::Reducer {
                    dispatch_sequence: 0,
                    operation_index: Some(0),
                    operation: SmokeOperation::OpenRun,
                    resolved: SmokeResolution {
                        event: SmokeEvent {
                            key: KeyCode::Enter,
                            modifiers: KeyModifiers::NONE,
                        },
                    },
                    event: SmokeEvent {
                        key: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                    },
                    action: Action::Back,
                    emitted: Effect::None,
                }))
                .unwrap_err()
                .contains("changed shape")
        );
        assert!(
            trace
                .record(checkpoint_with_cause(EngineCause::Host {
                    dispatch_sequence: 0,
                    operation_index: Some(0),
                    operation: SmokeOperation::OpenRun,
                    round: 0,
                    request: Effect::None,
                    response: Action::Back,
                    emitted: Effect::None,
                }))
                .unwrap_err()
                .contains("changed shape")
        );
    }

    #[test]
    fn real_frontend_switches_locale_only_when_it_consumes_preferences_saved() {
        let (mut frontend, _) = smoke_factory().create().unwrap();
        assert_eq!(frontend.locale, Locale::En);

        assert_eq!(
            frontend
                .reduce(Action::Preferences(PreferencesAction::SetLanguage(
                    "zh-TW".to_owned(),
                )))
                .unwrap(),
            Effect::None
        );
        assert_eq!(frontend.locale, Locale::En);

        assert_eq!(
            frontend
                .reduce(Action::SetStatus("host failure".to_owned()))
                .unwrap(),
            Effect::None
        );
        assert_eq!(frontend.locale, Locale::En);
        assert_eq!(frontend.reduce(Action::ClearStatus).unwrap(), Effect::None);
        assert_eq!(frontend.locale, Locale::En);

        for (tag, expected) in [
            ("en", Locale::En),
            ("zh-CN", Locale::ZhCn),
            ("zh-TW", Locale::ZhTw),
            ("x-pseudo", Locale::Pseudo),
            ("zh", Locale::ZhCn),
            ("not-a-supported-locale", Locale::En),
        ] {
            assert_eq!(
                frontend
                    .reduce(Action::PreferencesSaved {
                        locale: tag.to_owned(),
                        message: "saved".to_owned(),
                    })
                    .unwrap(),
                Effect::None
            );
            assert_eq!(frontend.locale, expected, "{tag}");
        }
    }

    #[test]
    fn real_preferences_validation_failure_keeps_the_frontend_locale() {
        let (mut frontend, mut host) = smoke_factory().create().unwrap();
        open_real_preferences(&mut frontend, &mut host);
        assert_eq!(
            frontend
                .reduce(Action::Preferences(PreferencesAction::SetLanguage(
                    "zh-TW".to_owned(),
                )))
                .unwrap(),
            Effect::None
        );
        let missing = host.host.sandbox_root().join("missing-bash");
        assert_eq!(
            frontend
                .reduce(Action::Preferences(PreferencesAction::SetBashPath(
                    missing.display().to_string(),
                )))
                .unwrap(),
            Effect::None
        );
        let request = frontend
            .reduce(Action::Preferences(PreferencesAction::Save))
            .unwrap();
        assert!(matches!(request, Effect::Preferences(_)));
        assert_eq!(frontend.locale, Locale::En);

        let response = host.serve(request).unwrap();
        assert!(matches!(
            response,
            Action::Preferences(PreferencesAction::ValidationFailed(_))
        ));
        assert_eq!(frontend.reduce(response).unwrap(), Effect::None);
        let observation = frontend.observe().unwrap();
        assert_eq!(observation.locale, Locale::En);
        assert!(
            observation
                .styled_frame
                .readable_lines()
                .unwrap()
                .join("\n")
                .contains("Preferences")
        );
        drop(frontend);
        host.close().unwrap();
    }

    #[test]
    fn real_host_serve_error_keeps_locale_and_records_no_response_checkpoint() {
        let factory = smoke_factory();
        let (frontend, host) = factory.create().unwrap();
        let host = FailPreferencesOpenHost::new(host);
        let mut engine = WalkerEngine::new(frontend, host, EFFECT_LIMIT).unwrap();
        let mut sink = BoundarySink::default();
        engine.start(&mut sink).unwrap();

        let error = engine
            .run_operation(SmokeOperation::OpenPreferences, &mut sink)
            .unwrap_err();
        assert_eq!(error, "injected Preferences host failure");
        assert_eq!(engine.frontend().locale, Locale::En);
        assert_eq!(sink.0, [EngineBoundary::Initial, EngineBoundary::Reducer]);

        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();
    }

    #[test]
    fn real_frontend_parity_includes_the_live_locale() {
        let (frontend, _) = smoke_factory().create().unwrap();
        let changed_locale =
            RealFrontend::new(frontend.state.clone(), Locale::ZhTw, smoke_factory().size).unwrap();

        assert_ne!(
            frontend.parity_snapshot().unwrap(),
            changed_locale.parity_snapshot().unwrap()
        );
    }

    #[test]
    fn real_preferences_operation_switches_the_first_host_frame_and_replays_exactly() {
        for (target, library, preferences, saved) in [
            (
                LocaleSmokeTarget::SimplifiedChinese,
                "工具库",
                "偏好设置",
                "偏好设置已保存",
            ),
            (
                LocaleSmokeTarget::TraditionalChinese,
                "工具庫",
                "偏好設定",
                "偏好設定已儲存",
            ),
        ] {
            let tag = target.tag();
            let main = record_real_locale_smoke(target).unwrap();
            let replay = record_real_locale_smoke(target).unwrap();
            assert_eq!(main.trace, replay.trace, "{tag}");
            assert_eq!(main.locale, "en");
            assert_eq!(main.operations.len(), target.next_count() + 5, "{tag}");

            let saved_index = main
                .rows
                .iter()
                .position(|row| {
                    matches!(
                        &row.cause,
                        TransitionCause::Host { response, .. }
                            if response.get("preferences_saved").is_some()
                    )
                })
                .unwrap();
            let save_request = &main.rows[saved_index - 1];
            let host_cause = json_value(&main.rows[saved_index].cause);
            assert_eq!(save_request.locale, "en");
            assert_eq!(save_request.boundary, TimelineBoundary::UserAction);
            assert_eq!(
                save_request.cause,
                TransitionCause::Reducer {
                    requested: json!({"operation": "save_preferences"}),
                    resolved: json!({
                        "input": {"kind": "event", "event": {
                            "type": "key", "code": {"Char": "s"}, "modifiers": "CONTROL",
                        }},
                        "semantic_target": {"command": "save_preferences"},
                    }),
                    event: json!({
                        "type": "key", "code": {"Char": "s"}, "modifiers": "CONTROL",
                    }),
                    action: json!({"preferences": "save"}),
                    emitted: host_cause["request"].clone(),
                }
            );
            assert_eq!(main.rows[saved_index].locale, tag);
            assert_eq!(
                main.rows[saved_index].boundary,
                TimelineBoundary::HostAction
            );
            assert!(
                main.rows[saved_index..].iter().all(|row| row.locale == tag),
                "{tag} reverted after the successful response"
            );

            let before = trace_frame_text(&main, save_request);
            assert!(before.contains("Preferences"), "{tag}: {before}");
            assert!(!before.contains(preferences), "{tag}: {before}");
            let after = trace_frame_text(&main, &main.rows[saved_index]);
            assert!(after.contains(library), "{tag}: {after}");
            assert!(after.contains(saved), "{tag}: {after}");

            assert_eq!(
                &host_cause["response"],
                &json!({"preferences_saved": {"locale": tag, "message": saved}})
            );
        }
    }

    #[test]
    fn real_frontend_and_host_move_raw_values_through_the_production_chain() {
        let (mut frontend, mut host) = smoke_factory().create().unwrap();
        let action = Action::OpenRun;
        assert_eq!(
            frontend
                .dispatch(SmokeEvent {
                    key: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                })
                .unwrap(),
            DispatchOutcome::Action(action.clone())
        );

        let emitted = frontend.reduce(action).unwrap();
        let emitted_evidence = json_value(&emitted);
        assert_eq!(
            emitted_evidence,
            json!({"open": {"request": "run", "selector": "reference"}})
        );

        let response: Action = host.serve(emitted).unwrap();
        let response_evidence = json_value(&response);
        assert_eq!(
            response_evidence.pointer("/present/run/selector"),
            Some(&json!("reference"))
        );
        assert_eq!(frontend.reduce(response).unwrap(), Effect::None);
    }

    #[test]
    fn real_host_engine_records_and_replays_one_production_open() {
        let factory = smoke_factory();
        let operation = SmokeOperation::OpenRun;
        let (frontend, host) = factory.create().unwrap();
        let main_root = host.host.roots().data.clone();
        let mut main = WalkerEngine::new(frontend, host, EFFECT_LIMIT).unwrap();
        let mut main_trace = MemoryTrace::default();
        main.start(&mut main_trace).unwrap();
        main.run_operation(operation, &mut main_trace).unwrap();

        let mut replay_trace = MemoryTrace::default();
        let replay = replay_prefix_with_sink(&factory, &[operation], &mut replay_trace).unwrap();
        let replay_root = replay.host().host.roots().data.clone();

        assert_ne!(main_root, replay_root);
        assert_eq!(main.successful_operations(), &[operation]);
        assert_eq!(replay.successful_operations(), &[operation]);
        assert_eq!(main_trace.checkpoints, replay_trace.checkpoints);
        assert_eq!(
            main_trace
                .checkpoints
                .iter()
                .map(|checkpoint| (checkpoint.boundary, checkpoint.cause.clone()))
                .collect::<Vec<_>>(),
            [
                (EngineBoundary::Initial, CauseObservation::Initial),
                (
                    EngineBoundary::Reducer,
                    CauseObservation::Reducer {
                        dispatch_sequence: 0,
                        operation_index: 0,
                        operation,
                        resolved: SmokeResolution {
                            event: SmokeEvent {
                                key: KeyCode::Enter,
                                modifiers: KeyModifiers::NONE,
                            },
                        },
                        event: SmokeEvent {
                            key: KeyCode::Enter,
                            modifiers: KeyModifiers::NONE,
                        },
                        selector: "reference".to_owned(),
                    },
                ),
                (
                    EngineBoundary::Host,
                    CauseObservation::Host {
                        dispatch_sequence: 0,
                        operation_index: 0,
                        operation,
                        round: 0,
                        selector: "reference".to_owned(),
                    },
                ),
            ]
        );

        let observations =
            serde_json::to_string(&main_trace.checkpoints.last().unwrap().host).unwrap();
        assert!(observations.contains("<profile:engine-smoke>/external/original.sh"));
        assert!(!observations.contains(&main_root.display().to_string()));
        assert!(!observations.contains(&replay_root.display().to_string()));

        assert!(main_trace.checkpoints.iter().all(|checkpoint| {
            checkpoint.styled_frame.area
                == RectSnapshot {
                    x: 0,
                    y: 0,
                    width: 60,
                    height: 24,
                }
        }));
        let initial = &main_trace.checkpoints[0];
        let settled = &main_trace.checkpoints[2];
        assert_eq!(
            initial.canonical_state.pointer("/workflow/active"),
            Some(&json!("library"))
        );
        assert_eq!(
            settled
                .canonical_state
                .pointer("/workflow/active/run/selector"),
            Some(&json!("reference"))
        );
        assert_ne!(initial.canonical_state, settled.canonical_state);
        assert_eq!(initial.canonical_state, initial.host.state);
        assert_eq!(settled.canonical_state, settled.host.state);
        assert_ne!(initial.styled_frame, settled.styled_frame);
        let initial_text = initial.styled_frame.readable_lines().unwrap().join("\n");
        let settled_text = settled.styled_frame.readable_lines().unwrap().join("\n");
        assert!(initial_text.contains("Library"));
        assert!(initial_text.contains("Reference"));
        assert!(settled_text.contains("Run Reference"));
        assert!(
            initial
                .session
                .pointer("/run/fields/signature")
                .is_some_and(Value::is_null)
        );
        assert!(
            settled
                .session
                .pointer("/run/fields/signature")
                .is_some_and(|signature| !signature.is_null())
        );
        let rows = settled.geometry.get("rows").unwrap();
        assert_eq!(rows.get("x"), Some(&json!(1)));
        assert_eq!(rows.get("y"), Some(&json!(1)));
        assert_eq!(rows.get("width"), Some(&json!(58)));
        assert!(rows.get("height").is_some_and(Value::is_u64));
        assert!(
            settled
                .geometry
                .get("hits")
                .and_then(Value::as_array)
                .is_some_and(|hits| !hits.is_empty())
        );
    }

    #[test]
    fn schema_three_projection_keeps_full_objects_timeline_and_cast_in_memory() {
        let factory = smoke_factory();

        let main = record_schema_three_main(&factory).unwrap();
        let replay = record_schema_three_replay(&factory).unwrap();

        assert_eq!(main.trace, replay.trace);
        assert_eq!(main.sandbox.mode(), SandboxMode::Random);
        assert_eq!(replay.sandbox.mode(), SandboxMode::Random);
        assert_eq!(
            main.sandbox.profiles().get(&factory.review_profile),
            Some(&main.sandbox.root().to_owned())
        );
        assert_eq!(
            replay.sandbox.profiles().get(&factory.review_profile),
            Some(&replay.sandbox.root().to_owned())
        );
        #[cfg(target_os = "linux")]
        assert_eq!(main.sandbox.platform(), SandboxPlatform::Linux);
        #[cfg(target_os = "macos")]
        assert_eq!(main.sandbox.platform(), SandboxPlatform::Macos);
        #[cfg(target_os = "windows")]
        assert_eq!(main.sandbox.platform(), SandboxPlatform::Windows);
        assert_eq!(main.rows.len(), 4);
        assert_eq!(
            main.rows
                .iter()
                .map(|row| row.presentation)
                .collect::<Vec<_>>(),
            [
                Presentation::Presented,
                Presentation::NotPresented,
                Presentation::Presented,
                Presentation::Presented,
            ]
        );
        assert_eq!(
            main.rows.last().unwrap().liveness,
            Some(LivenessResult::Passed)
        );
        assert_eq!(main.operations.len(), 1);
        assert_eq!(main.review_profile.as_str(), "engine-smoke-60x24");
        assert!(
            main.rows
                .iter()
                .all(|row| row.profile == main.review_profile)
        );
        assert_eq!(
            main.rows
                .iter()
                .map(|row| {
                    (
                        row.sequence,
                        row.phase,
                        row.event_chain,
                        row.operation_index,
                        row.boundary,
                    )
                })
                .collect::<Vec<_>>(),
            [
                (
                    0,
                    TimelinePhase::Operations,
                    None,
                    None,
                    TimelineBoundary::Initial,
                ),
                (
                    1,
                    TimelinePhase::Operations,
                    Some(EventChainIdentity {
                        phase: TimelinePhase::Operations,
                        sequence: 0,
                    }),
                    Some(0),
                    TimelineBoundary::UserAction,
                ),
                (
                    2,
                    TimelinePhase::Operations,
                    Some(EventChainIdentity {
                        phase: TimelinePhase::Operations,
                        sequence: 0,
                    }),
                    Some(0),
                    TimelineBoundary::HostAction,
                ),
                (
                    3,
                    TimelinePhase::FinalLiveness,
                    Some(EventChainIdentity {
                        phase: TimelinePhase::FinalLiveness,
                        sequence: 0,
                    }),
                    None,
                    TimelineBoundary::UserAction,
                ),
            ]
        );
        assert_eq!(main.rows[0].cause, TransitionCause::Initial);
        assert_eq!(
            main.rows[1].cause,
            TransitionCause::Reducer {
                requested: json!({"operation": "open_run"}),
                resolved: json!({
                    "input": {"kind": "event", "event": {
                        "type": "key", "code": "Enter", "modifiers": "",
                    }},
                    "semantic_target": {"command": "run"},
                }),
                event: json!({"type": "key", "code": "Enter", "modifiers": ""}),
                action: json!("open_run"),
                emitted: json!({"open": {"request": "run", "selector": "reference"}}),
            }
        );
        let reducer_cause = json_value(&main.rows[1].cause);
        let host_cause = json_value(&main.rows[2].cause);
        assert_eq!(
            reducer_cause.pointer("/emitted"),
            host_cause.pointer("/request")
        );
        assert_eq!(host_cause.pointer("/cause"), Some(&json!("host")));
        assert_eq!(host_cause.pointer("/operation_index"), Some(&json!(0)));
        assert_eq!(host_cause.pointer("/round"), Some(&json!(0)));
        assert_eq!(
            host_cause.pointer("/request"),
            Some(&json!({"open": {"request": "run", "selector": "reference"}}))
        );
        assert_eq!(
            host_cause.pointer("/response/present/run/selector"),
            Some(&json!("reference"))
        );
        assert_eq!(
            host_cause.pointer("/response/present/run/name"),
            Some(&json!("Reference"))
        );
        assert_eq!(host_cause.pointer("/emitted"), Some(&json!("none")));
        let round_trip: Action = serde_json::from_value(host_cause["response"].clone()).unwrap();
        assert!(matches!(
            round_trip,
            Action::Present(Screen::Run(screen)) if screen.selector() == "reference"
        ));
        assert_eq!(
            main.rows[3].cause,
            TransitionCause::Reducer {
                requested: json!({"synthetic": "final_liveness"}),
                resolved: json!({"event": {
                    "type": "key", "code": "Esc", "modifiers": "",
                }}),
                event: json!({"type": "key", "code": "Esc", "modifiers": ""}),
                action: json!("back"),
                emitted: json!("none"),
            }
        );
        for row in &main.rows {
            for reference in [
                &row.reducer,
                &row.host,
                &row.session,
                &row.styled_frame,
                &row.geometry,
            ] {
                assert!(
                    main.objects
                        .contains_key(&(reference.kind, reference.sha256.clone()))
                );
            }
        }
        let cast = main
            .cast
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(cast.len(), 4);
        assert_eq!(
            cast[0],
            json!({"version": 3, "term": {"cols": 60, "rows": 24}})
        );
        assert_eq!(
            cast[1..]
                .iter()
                .map(|event| (event[0].as_f64().unwrap(), event[1].as_str().unwrap()))
                .collect::<Vec<_>>(),
            [(0.1, "o"), (0.2, "o"), (0.1, "o")]
        );
    }

    #[test]
    fn random_trace_failures_still_use_explicit_close() {
        let factory = smoke_factory();
        let prefix = [SmokeOperation::OpenRun];

        let error = record_schema_three_random_with_hooks(
            &factory,
            &prefix,
            noop_real_host_boundary,
            |_| Err("injected random sink failure".to_owned()),
            read_boundary_sandbox_metadata,
            noop_schema_sink,
            noop_leak_oracle_facts,
        )
        .unwrap_err();
        assert!(error.contains("injected random sink failure"));

        let error = record_schema_three_random_with_hooks(
            &factory,
            &prefix,
            noop_real_host_boundary,
            new_schema_three_sink,
            |_, _| Err("injected random metadata failure".to_owned()),
            noop_schema_sink,
            noop_leak_oracle_facts,
        )
        .unwrap_err();
        assert!(error.contains("injected random metadata failure"));

        let error = record_schema_three_random_with_hooks(
            &factory,
            &prefix,
            noop_real_host_boundary,
            new_schema_three_sink,
            read_boundary_sandbox_metadata,
            |sink| sink.rows.clear(),
            noop_leak_oracle_facts,
        )
        .unwrap_err();
        assert!(error.contains("no checkpoint"));

        let error = record_schema_three_random_with_hooks(
            &factory,
            &prefix,
            |host| {
                std::fs::write(
                    host.host.sandbox_root().join("system-temp/residual"),
                    b"residual",
                )
                .unwrap();
            },
            new_schema_three_sink,
            read_boundary_sandbox_metadata,
            noop_schema_sink,
            noop_leak_oracle_facts,
        )
        .unwrap_err();
        assert!(error.contains("system temp retained an artifact"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let retained_root = std::cell::RefCell::new(None);
            let retained_child = std::cell::RefCell::new(None);
            let error = record_schema_three_random_with_hooks(
                &factory,
                &prefix,
                |host| {
                    let root = host.host.sandbox_root().to_path_buf();
                    let blocked = root.join("blocked-cleanup");
                    std::fs::create_dir(&blocked).unwrap();
                    std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000))
                        .unwrap();
                    std::fs::write(root.join("system-temp/residual"), b"residual").unwrap();
                    retained_root.replace(Some(root));
                    retained_child.replace(Some(blocked));
                },
                new_schema_three_sink,
                read_boundary_sandbox_metadata,
                noop_schema_sink,
                noop_leak_oracle_facts,
            )
            .unwrap_err();
            assert!(error.contains("sandbox cleanup also failed"));
            let root = retained_root.into_inner().unwrap();
            let child = retained_child.into_inner().unwrap();
            std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn unrendered_random_draft_retains_facts_for_recorded_data() {
        let factory = smoke_factory();
        let raw_path = std::rc::Rc::new(std::cell::RefCell::new(None));
        let raw_path_from_hook = raw_path.clone();
        let recorded = record_schema_three_random_with_hooks(
            &factory,
            &[SmokeOperation::OpenRun],
            move |host| {
                let drafts =
                    super::super::create_owned_drafts_dir(&host.host.roots().data).unwrap();
                let draft = drafts.join("skit-new-000000.py");
                std::fs::write(&draft, b"print('facts')\n").unwrap();
                raw_path_from_hook.replace(Some(draft.display().to_string()));
            },
            new_schema_three_sink,
            read_boundary_sandbox_metadata,
            noop_schema_sink,
            noop_leak_oracle_facts,
        )
        .unwrap();
        let raw_path = raw_path.borrow().clone().unwrap();
        assert!(
            recorded
                .leak_oracle_facts
                .renderer_drafts
                .values()
                .any(|draft| draft.raw_path == raw_path)
        );
        assert!(!recorded.leak_oracle_facts.artifacts.is_empty());

        let mut trace_bytes = serde_json::to_vec(&recorded.operations).unwrap();
        trace_bytes.extend(serde_json::to_vec(&recorded.final_liveness_requested).unwrap());
        trace_bytes.extend(serde_json::to_vec(&recorded.rows).unwrap());
        for object in recorded.objects.values() {
            trace_bytes.extend(object);
        }
        trace_bytes.extend(&recorded.cast);
        assert!(
            !trace_bytes
                .windows(raw_path.len())
                .any(|window| window == raw_path.as_bytes())
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn stable_add_draft_state(
        host: &RealWalkerHost,
        modified_seconds: u64,
    ) -> Result<LibraryState, String> {
        let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
        let path = drafts.join("skit-new-000000.py");
        std::fs::write(&path, b"print('stable facts')\n").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(modified_seconds),
            ))
            .unwrap();
        Ok(host.initial_state().unwrap())
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_main_and_replay_keep_equal_traces_but_independent_raw_facts() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let factory = smoke_factory();
        let (main, operations) = record_schema_three_stable_with_hooks(
            &factory,
            namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            |host| stable_add_draft_state(host, 10),
            noop_stable_host,
            new_schema_three_sink,
            noop_schema_sink,
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap();
        let (replay, replayed) = record_schema_three_stable_with_hooks(
            &factory,
            namespace,
            StablePairPhase::Replay,
            &operations,
            read_sandbox_metadata,
            |host| stable_add_draft_state(host, 20),
            noop_stable_host,
            new_schema_three_sink,
            noop_schema_sink,
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap();

        assert_eq!(replayed, operations);
        assert_eq!(main.trace, replay.trace);
        assert_ne!(main.leak_oracle_facts, replay.leak_oracle_facts);
        assert!(main.leak_oracle_facts.artifacts.values().any(|artifact| {
            artifact
                .modified_values
                .iter()
                .any(|pair| pair.raw != pair.projected)
        }));
        assert!(replay.leak_oracle_facts.artifacts.values().any(|artifact| {
            artifact
                .modified_values
                .iter()
                .any(|pair| pair.raw != pair.projected)
        }));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn facts_snapshot_precedes_an_explicit_close_failure() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let order = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let facts = std::rc::Rc::new(std::cell::RefCell::new(None));
        let order_at_snapshot = order.clone();
        let facts_at_snapshot = facts.clone();
        let order_at_close = order.clone();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            namespace,
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            read_initial_state,
            |host| {
                let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
                std::fs::write(
                    drafts.join("skit-new-000000.py"),
                    b"print('unrendered hook fact')\n",
                )
                .unwrap();
            },
            new_schema_three_sink,
            noop_schema_sink,
            move |snapshot| {
                assert!(
                    snapshot
                        .renderer_drafts
                        .values()
                        .any(|draft| { draft.raw_kind_picker_basename == "skit-new-000000.py" })
                );
                assert!(!snapshot.artifacts.is_empty());
                order_at_snapshot.borrow_mut().push("facts");
                facts_at_snapshot.replace(Some(snapshot.clone()));
            },
            move |host| {
                order_at_close.borrow_mut().push("close");
                std::fs::write(
                    host.sandbox_root().join(SANDBOX_MARKER_FILE),
                    b"corrupt marker",
                )
                .unwrap();
            },
        )
        .unwrap_err();

        assert!(matches!(error.primary, StablePairPrimaryError::Close(_)));
        assert_eq!(*order.borrow(), ["facts", "close"]);
        assert!(facts.borrow().is_some());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_pair_closes_main_before_fresh_replay_in_the_same_namespace() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();

        let pair = record_real_smoke_stable_pair_in(namespace).unwrap();

        assert_eq!(pair.main.trace, pair.replay.trace);
        assert_eq!(pair.main.sandbox, pair.replay.sandbox);
        assert_eq!(pair.main.sandbox.mode(), SandboxMode::Stable);
        assert_eq!(
            pair.main.sandbox.root(),
            namespace_root.to_str().expect("the test path is UTF-8")
        );
        let profile_root = namespace_root.join(profile_sandbox_path(&pair.main.review_profile));
        assert_eq!(
            pair.main.sandbox.profiles().get(&pair.main.review_profile),
            Some(&profile_root.to_string_lossy().into_owned())
        );
        assert!(!profile_root.exists());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_pair_refuses_a_live_main_profile_without_random_fallback() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let factory = smoke_factory();
        let held = RealWalkerHost::spawn_stable_in(
            factory.seed.clone(),
            factory.review_profile.clone(),
            namespace.clone(),
        )
        .unwrap();

        let error = record_real_smoke_stable_pair_in(namespace).unwrap_err();

        assert_eq!(error.phase, StablePairPhase::Main);
        assert!(error.to_string().contains("is busy"));
        assert!(matches!(
            &error.primary,
            StablePairPrimaryError::Sandbox(error)
                if matches!(error.as_ref(), SandboxError::Busy { .. })
        ));
        assert!(error.cleanup.is_none());
        held.close().unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_frontend_failure_explicitly_closes_the_profile() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let mut invalid = smoke_factory();
        invalid.size = Size::new(0, 24);

        let error = record_schema_three_stable(
            &invalid,
            namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
        )
        .unwrap_err();
        assert!(error.to_string().contains("could not create"));
        assert!(matches!(
            &error.primary,
            StablePairPrimaryError::Frontend(_)
        ));
        assert!(error.cleanup.is_none());

        record_real_smoke_stable_pair_in(namespace).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_pair_preserves_typed_primary_and_cleanup_failures() {
        let seed_parent = tempfile::TempDir::new().unwrap();
        let seed_namespace =
            StableSandboxNamespace::explicit(seed_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let mut invalid_seed = smoke_factory();
        invalid_seed.seed.external = vec![WalkerExternalSeed::File {
            path: PathBuf::from("../outside"),
            bytes: Vec::new(),
            readonly: false,
            unix_mode: 0o600,
        }];
        let error = record_schema_three_stable(
            &invalid_seed,
            seed_namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
        )
        .unwrap_err();
        assert!(error.to_string().contains("could not seed"));
        assert!(matches!(&error.primary, StablePairPrimaryError::Seed(_)));
        assert!(error.cleanup.is_none());
        record_real_smoke_stable_pair_in(seed_namespace).unwrap();

        let metadata_parent = tempfile::TempDir::new().unwrap();
        let metadata_namespace =
            StableSandboxNamespace::explicit(metadata_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            metadata_namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            |host, _| {
                Err(SandboxError::InvalidEvidence {
                    path: host.sandbox_root().to_path_buf(),
                    reason: "injected metadata failure".to_owned(),
                })
            },
            read_initial_state,
            noop_stable_host,
            new_schema_three_sink,
            noop_schema_sink,
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected metadata failure"));
        assert!(matches!(&error.primary, StablePairPrimaryError::Sandbox(_)));
        assert!(error.cleanup.is_none());
        record_real_smoke_stable_pair_in(metadata_namespace).unwrap();

        let initial_parent = tempfile::TempDir::new().unwrap();
        let initial_namespace =
            StableSandboxNamespace::explicit(initial_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            initial_namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            |_| Err("injected initial-state failure".to_owned()),
            noop_stable_host,
            new_schema_three_sink,
            noop_schema_sink,
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap_err();
        assert!(error.to_string().contains("could not read"));
        assert!(
            matches!(&error.primary, StablePairPrimaryError::InitialState(_)),
            "{error:?}"
        );
        assert!(error.cleanup.is_none());
        record_real_smoke_stable_pair_in(initial_namespace).unwrap();

        let sink_parent = tempfile::TempDir::new().unwrap();
        let sink_namespace =
            StableSandboxNamespace::explicit(sink_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            sink_namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            read_initial_state,
            noop_stable_host,
            |_| Err("injected sink failure".to_owned()),
            noop_schema_sink,
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap_err();
        assert!(error.to_string().contains("injected sink failure"));
        assert!(matches!(&error.primary, StablePairPrimaryError::Trace(_)));
        assert!(error.cleanup.is_none());
        record_real_smoke_stable_pair_in(sink_namespace).unwrap();

        let trace_parent = tempfile::TempDir::new().unwrap();
        let trace_namespace =
            StableSandboxNamespace::explicit(trace_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            trace_namespace.clone(),
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            read_initial_state,
            |host| {
                std::fs::write(
                    host.sandbox_root().join("system-temp/residual"),
                    b"residual",
                )
                .unwrap();
            },
            new_schema_three_sink,
            noop_schema_sink,
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap_err();
        assert!(error.to_string().contains("could not record"));
        assert!(matches!(&error.primary, StablePairPrimaryError::Trace(_)));
        assert!(error.cleanup.is_none());
        record_real_smoke_stable_pair_in(trace_namespace).unwrap();

        let aggregate_parent = tempfile::TempDir::new().unwrap();
        let aggregate_namespace = StableSandboxNamespace::explicit(
            aggregate_parent.path().join(STABLE_SANDBOX_NAMESPACE),
        )
        .unwrap();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            aggregate_namespace,
            StablePairPhase::Replay,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            read_initial_state,
            |host| {
                std::fs::write(
                    host.sandbox_root().join(SANDBOX_MARKER_FILE),
                    b"corrupt marker",
                )
                .unwrap();
            },
            new_schema_three_sink,
            |sink| sink.rows.clear(),
            noop_leak_oracle_facts,
            noop_stable_host,
        )
        .unwrap_err();
        assert!(matches!(&error.primary, StablePairPrimaryError::Trace(_)));
        assert!(error.cleanup.is_some());
        let message = error.to_string();
        assert!(message.contains("stable walker replay failed"));
        assert!(message.contains("sandbox cleanup also failed"));

        let close_parent = tempfile::TempDir::new().unwrap();
        let close_namespace =
            StableSandboxNamespace::explicit(close_parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let error = record_schema_three_stable_with_hooks(
            &smoke_factory(),
            close_namespace,
            StablePairPhase::Main,
            &[SmokeOperation::OpenRun],
            read_sandbox_metadata,
            read_initial_state,
            noop_stable_host,
            new_schema_three_sink,
            noop_schema_sink,
            noop_leak_oracle_facts,
            |host| {
                std::fs::write(
                    host.sandbox_root().join(SANDBOX_MARKER_FILE),
                    b"corrupt marker",
                )
                .unwrap();
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("could not close"));
        assert!(matches!(&error.primary, StablePairPrimaryError::Close(_)));
        assert!(error.cleanup.is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn read_initial_state(host: &RealWalkerHost) -> Result<LibraryState, String> {
        host.initial_state().map_err(|error| error.to_string())
    }

    #[test]
    fn schema_three_sink_rejects_missing_corrupt_and_inconsistent_artifacts() {
        let factory = smoke_factory();
        assert!(
            new_schema_three_sink(&factory)
                .unwrap()
                .finish_smoke()
                .unwrap_err()
                .contains("no checkpoint")
        );

        let mut collision = new_schema_three_sink(&factory).unwrap();
        let reference = collision
            .put_value(ObjectKind::Reducer, json!({"stable": true}))
            .unwrap();
        let key = (reference.kind, reference.sha256.clone());
        collision.objects.insert(key.clone(), b"corrupt".to_vec());
        assert!(
            collision
                .put_value(ObjectKind::Reducer, json!({"stable": true}))
                .unwrap_err()
                .contains("different object bytes")
        );
        assert_eq!(collision.objects[&key], b"corrupt");
        assert!(
            collision
                .object_value(&reference)
                .unwrap_err()
                .contains("expected")
        );
        collision.objects.remove(&key);
        assert!(
            collision
                .object_value(&reference)
                .unwrap_err()
                .contains("absent")
        );

        let mut malformed_final = prepared_schema_three_sink(&factory);
        let malformed_state = malformed_final
            .put_value(
                ObjectKind::Reducer,
                json!({"workflow": {"active": "library"}, "modal": null}),
            )
            .unwrap();
        malformed_final.rows.last_mut().unwrap().reducer = malformed_state;
        let error = malformed_final.finish_smoke().unwrap_err();
        assert!(error.contains("not a LibraryState"), "{error}");

        let mut noncanonical_final = prepared_schema_three_sink(&factory);
        let final_ref = noncanonical_final.rows.last().unwrap().reducer.clone();
        let mut noncanonical_state = noncanonical_final.object_value(&final_ref).unwrap();
        noncanonical_state
            .as_object_mut()
            .unwrap()
            .remove("details");
        let noncanonical_ref = noncanonical_final
            .put_value(ObjectKind::Reducer, noncanonical_state)
            .unwrap();
        noncanonical_final.rows.last_mut().unwrap().reducer = noncanonical_ref;
        assert!(
            noncanonical_final
                .finish_smoke()
                .unwrap_err()
                .contains("canonical LibraryState")
        );

        let mut wrong_host_state = prepared_schema_three_sink(&factory);
        wrong_host_state.rows[2].reducer = wrong_host_state.rows[0].reducer.clone();
        assert!(
            wrong_host_state
                .finish_smoke()
                .unwrap_err()
                .contains("production Action")
        );

        let mut wrong_effect = prepared_schema_three_sink(&factory);
        *reducer_emitted(&mut wrong_effect.rows[1].cause) = json!("none");
        assert!(
            wrong_effect
                .validate_reducer_transitions()
                .unwrap_err()
                .contains("production Effect")
        );

        let mut repeated_initial = prepared_schema_three_sink(&factory);
        repeated_initial.rows[1].cause = TransitionCause::Initial;
        assert!(
            repeated_initial
                .validate_reducer_transitions()
                .unwrap_err()
                .contains("only the first")
        );

        let mut unchanged_session = prepared_schema_three_sink(&factory);
        unchanged_session.rows[1].cause = TransitionCause::Session {
            requested: json!({"operation": "session"}),
            resolved: json!({
                "input": {"kind": "event", "event": {
                    "type": "key", "code": "Enter", "modifiers": "",
                }},
                "semantic_target": null,
            }),
            event: json!({"type": "key", "code": "Enter", "modifiers": ""}),
            handling: json!("consumed"),
        };
        unchanged_session.rows[1].reducer = unchanged_session.rows[0].reducer.clone();
        unchanged_session.validate_reducer_transitions().unwrap();
        unchanged_session.rows[1].reducer = unchanged_session.rows[2].reducer.clone();
        assert!(
            unchanged_session
                .validate_reducer_transitions()
                .unwrap_err()
                .contains("session checkpoint")
        );

        let mut nonterminal_final = prepared_schema_three_sink(&factory);
        let mut only_run = nonterminal_final.rows[2].clone();
        only_run.sequence = 0;
        only_run.previous_row_sha256 = None;
        only_run.event_chain = None;
        only_run.operation_index = None;
        only_run.boundary = TimelineBoundary::Initial;
        only_run.cause = TransitionCause::Initial;
        nonterminal_final.rows = vec![only_run];
        assert!(
            nonterminal_final
                .finish_smoke()
                .unwrap_err()
                .contains("return the real reducer")
        );

        let mut missing = prepared_schema_three_sink(&factory);
        let missing_ref = missing.rows.last().unwrap().geometry.clone();
        missing
            .objects
            .remove(&(missing_ref.kind, missing_ref.sha256));
        assert!(missing.finish_smoke().unwrap_err().contains("absent"));

        let mut wrong_canvas = prepared_schema_three_sink(&factory);
        for row in &mut wrong_canvas.rows {
            row.viewport.width = 59;
        }
        for index in 1..wrong_canvas.rows.len() {
            wrong_canvas.rows[index].previous_row_sha256 =
                Some(timeline_row_digest(&wrong_canvas.rows[index - 1]).unwrap());
        }
        assert!(
            wrong_canvas
                .finish_smoke()
                .unwrap_err()
                .contains("timeline canvas")
        );

        let mut wrong_viewport = prepared_schema_three_sink(&factory);
        wrong_viewport.rows.last_mut().unwrap().viewport.width = 59;
        assert!(
            wrong_viewport
                .finish_smoke()
                .unwrap_err()
                .contains("timeline viewport")
        );

        let mut wrong_cast = prepared_schema_three_sink(&factory);
        let frame_ref = wrong_cast.rows.last().unwrap().styled_frame.clone();
        let mut extra_frame: StyledFrameSnapshot =
            serde_json::from_value(wrong_cast.object_value(&frame_ref).unwrap()).unwrap();
        extra_frame.cells[0].symbol = "X".to_owned();
        wrong_cast
            .cast
            .record_snapshot(FRAME_INTERVAL, &extra_frame)
            .unwrap();
        assert!(
            wrong_cast
                .finish_smoke()
                .unwrap_err()
                .contains("did not rebuild")
        );
    }

    #[test]
    fn schema_three_sink_failure_publishes_no_checkpoint_or_artifact() {
        let factory = smoke_factory();
        let mut sink = new_schema_three_sink(&factory).unwrap();
        let mut checkpoint = checkpoint_with_cause(EngineCause::Initial);
        checkpoint.boundary = EngineBoundary::Initial;
        checkpoint.frontend.styled_frame.cells.clear();
        let rows = sink.rows.clone();
        let objects = sink.objects.clone();
        let cast = sink.cast.as_bytes().to_vec();

        assert!(sink.record(checkpoint).is_err());
        assert_eq!(sink.rows, rows);
        assert_eq!(sink.objects, objects);
        assert_eq!(sink.cast.as_bytes(), cast);
    }

    #[test]
    fn not_presented_distinct_frame_is_absent_and_its_interval_reaches_the_next_frame() {
        let factory = smoke_factory();
        let trace = record_schema_three_main(&factory).unwrap();
        let reference = &trace.rows[0].styled_frame;
        let bytes = &trace.objects[&(reference.kind, reference.sha256.clone())];
        let value = validate_object(reference, bytes).unwrap();
        let mut first: StyledFrameSnapshot = serde_json::from_value(value).unwrap();
        first.cells[0].symbol = "A".to_owned();
        let mut diagnostic = first.clone();
        diagnostic.cells[0].symbol = "¤".to_owned();
        let mut last = first.clone();
        last.cells[0].symbol = "B".to_owned();

        let mut recorder = AsciicastRecorder::new(60, 24).unwrap();
        record_presentation(&mut recorder, Presentation::Presented, &first).unwrap();
        record_presentation(&mut recorder, Presentation::NotPresented, &diagnostic).unwrap();
        record_presentation(&mut recorder, Presentation::Presented, &last).unwrap();

        let lines = recorder
            .as_bytes()
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1][0], json!(0.1));
        assert_eq!(lines[2][0], json!(0.2));
        assert!(!String::from_utf8_lossy(recorder.as_bytes()).contains('¤'));
    }

    #[test]
    fn schema_three_cause_projection_covers_session_refusal_and_bounds() {
        assert_eq!(
            timeline_boundary(EngineBoundary::Session),
            TimelineBoundary::Session
        );
        assert_eq!(
            smoke_event_value(SmokeEvent {
                key: KeyCode::F(24),
                modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            }),
            json!({
                "type": "key",
                "code": {"F": 24},
                "modifiers": "SHIFT | CONTROL",
            })
        );

        assert!(
            transition_cause(
                TimelinePhase::Operations,
                &EngineCause::NotApplicable {
                    dispatch_sequence: 3,
                    operation_index: Some(2),
                    operation: SmokeOperation::OpenRun,
                    resolved: SmokeResolution {
                        event: SmokeEvent {
                            key: KeyCode::Enter,
                            modifiers: KeyModifiers::NONE,
                        },
                    },
                },
            )
            .unwrap_err()
            .contains("no not-applicable")
        );

        for (handling, expected) in [
            (SmokeHandling::Consumed, "consumed"),
            (SmokeHandling::Ignored, "ignored"),
        ] {
            let (_, _, cause) = transition_cause(
                TimelinePhase::Operations,
                &EngineCause::Session {
                    dispatch_sequence: 1,
                    operation_index: Some(0),
                    operation: SmokeOperation::OpenRun,
                    resolved: SmokeResolution {
                        event: SmokeEvent {
                            key: KeyCode::Enter,
                            modifiers: KeyModifiers::NONE,
                        },
                    },
                    event: SmokeEvent {
                        key: KeyCode::Enter,
                        modifiers: KeyModifiers::NONE,
                    },
                    handling,
                },
            )
            .unwrap();
            assert!(matches!(
                cause,
                TransitionCause::Session { handling, .. } if handling == json!(expected)
            ));
        }

        #[cfg(target_pointer_width = "64")]
        {
            let overflow = usize::try_from(u64::from(u32::MAX) + 1).unwrap();
            assert!(checked_u32(overflow, "dispatch sequence").is_err());
            let host = EngineCause::Host {
                dispatch_sequence: 0,
                operation_index: None,
                operation: SmokeOperation::FinalLiveness,
                round: usize::from(u16::MAX) + 1,
                request: Effect::None,
                response: Action::Back,
                emitted: Effect::None,
            };
            assert!(transition_cause(TimelinePhase::FinalLiveness, &host).is_err());
        }
    }

    #[test]
    fn schema_three_termination_check_rejects_every_dangling_effect_shape() {
        let factory = smoke_factory();
        let trace = record_schema_three_main(&factory).unwrap();
        let reducer = trace.rows[1].clone();
        let initial = trace.rows[0].clone();
        let host = trace.rows[2].clone();

        assert!(
            validate_effect_chain_termination(&[reducer.clone(), initial])
                .unwrap_err()
                .contains("before another event")
        );
        assert!(
            validate_effect_chain_termination(std::slice::from_ref(&host))
                .unwrap_err()
                .contains("no pending")
        );
        let mut wrong_host = host;
        if let TransitionCause::Host { request, .. } = &mut wrong_host.cause {
            *request = json!({"wrong": true});
        }
        assert!(
            validate_effect_chain_termination(&[reducer.clone(), wrong_host])
                .unwrap_err()
                .contains("predecessor")
        );
        assert!(
            validate_effect_chain_termination(&[reducer])
                .unwrap_err()
                .contains("ends with")
        );
    }

    fn corpus_engine() -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
        let factory = smoke_factory();
        let host = RealWalkerHost::spawn(factory.seed).unwrap();
        let file_picker_tree = host.file_picker_tree();
        let state = host.initial_state().unwrap();
        let frontend =
            CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
        WalkerEngine::new(
            frontend,
            CorpusHostBoundary {
                inner: RealHostBoundary { host },
                leak_sandbox: None,
            },
            EFFECT_LIMIT,
        )
        .unwrap()
    }

    fn corpus_engine_for_factory(
        factory: RealWalkerFactory,
    ) -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
        let host = RealWalkerHost::spawn(factory.seed).unwrap();
        let file_picker_tree = host.file_picker_tree();
        let state = host.initial_state().unwrap();
        let frontend =
            CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
        WalkerEngine::new(
            frontend,
            CorpusHostBoundary {
                inner: RealHostBoundary { host },
                leak_sandbox: None,
            },
            EFFECT_LIMIT,
        )
        .unwrap()
    }

    fn resolution_parts(
        resolution: OperationResolution<CorpusEvent, CorpusResolution>,
    ) -> (CorpusResolution, Vec<CorpusEvent>) {
        match resolution {
            OperationResolution::NotApplicable { resolved } => (resolved, Vec::new()),
            OperationResolution::Events { resolved, events } => (resolved, events),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn stable_corpus_engine_for_factory(
        factory: RealWalkerFactory,
        namespace: StableSandboxNamespace,
    ) -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
        let host = RealWalkerHost::spawn_stable_in(
            factory.seed,
            factory.review_profile.clone(),
            namespace,
        )
        .unwrap();
        let sandbox = host.sandbox_metadata(&factory.review_profile).unwrap();
        let file_picker_tree = host.file_picker_tree();
        let state = host.initial_state().unwrap();
        let frontend =
            CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
        WalkerEngine::new(
            frontend,
            CorpusHostBoundary {
                inner: RealHostBoundary { host },
                leak_sandbox: Some(sandbox),
            },
            EFFECT_LIMIT,
        )
        .unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn stable_corpus_engine(
        namespace: StableSandboxNamespace,
    ) -> WalkerEngine<CorpusFrontend, CorpusHostBoundary> {
        stable_corpus_engine_for_factory(smoke_factory(), namespace)
    }

    #[test]
    fn random_corpus_capture_keeps_diagnostic_recording_without_full_leak_scans() {
        let (mut frontend, mut host) = corpus_engine().into_parts();
        let capture = host
            .capture_checkpoint(
                frontend.observe().unwrap(),
                CheckpointCauseProjection::Observation,
            )
            .unwrap();

        assert!(capture.host.leak_context.is_none());
        drop(frontend);
        host.close().unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_corpus_capture_snapshots_facts_after_registering_a_new_draft() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let (mut frontend, mut host) = stable_corpus_engine(namespace).into_parts();
        let drafts = super::super::create_owned_drafts_dir(&host.inner.host.roots().data).unwrap();
        let raw_path = drafts.join("skit-new-000000.py");
        std::fs::write(&raw_path, b"print('checkpoint facts')\n").unwrap();
        let capture = host
            .capture_checkpoint(
                frontend.observe().unwrap(),
                CheckpointCauseProjection::Observation,
            )
            .unwrap();

        let context = capture.host.leak_context.unwrap();
        assert_eq!(context.sandbox, host.leak_sandbox.clone().unwrap());
        let draft = context
            .facts
            .renderer_drafts
            .values()
            .find(|draft| draft.raw_path == raw_path.display().to_string())
            .unwrap();
        let artifact = &context.facts.artifacts[&draft.projected_path];
        assert!(artifact.raw_path_spellings.contains(&draft.raw_path));
        assert!(!artifact.source_identities.is_empty());
        assert!(!artifact.modified_values.is_empty());
        assert_eq!(
            capture.host.observation.drafts[0]["path"],
            draft.projected_path
        );
        drop(frontend);
        host.close().unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn corpus_parity_after(
        namespace: StableSandboxNamespace,
        operations: &[CorpusOperation],
    ) -> FrontendParity {
        let mut engine = stable_corpus_engine(namespace);
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        for operation in operations {
            engine.run_operation(operation.clone(), &mut sink).unwrap();
        }
        let parity = engine.parity_snapshot().unwrap();
        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();
        parity
    }

    #[test]
    fn corpus_screen_picker_uses_the_seeded_tree_and_real_add_chain() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let operations = [
            CorpusOperation::CommandKeyboard(UiCommand::Add),
            CorpusOperation::ScreenHit(skit_tui::ScreenTarget::Add(
                skit_tui::AddControlId::BrowseSource,
            )),
            CorpusOperation::ScreenHit(skit_tui::ScreenTarget::FilePickerEntry {
                relative: PathBuf::from("original.sh"),
            }),
            CorpusOperation::ScreenFocus(skit_tui::ScreenTarget::Add(
                skit_tui::AddControlId::Continue,
            )),
            CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Enter,
                CorpusModifiers::NONE,
            )),
        ];
        for operation in operations {
            engine.run_operation(operation, &mut sink).unwrap();
        }

        assert!(
            engine
                .frontend()
                .inner
                .session_value()
                .unwrap()
                .to_string()
                .contains("memory_file_picker_source")
        );
        assert!(matches!(
            engine.frontend().inner.state.screen(),
            Screen::Add(view) if view.stage() == skit_ui::AddStage::Review
        ));
        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn corpus_screen_focus_and_hit_reach_the_same_preferences_endpoint() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let focus = CorpusOperation::ScreenFocus(skit_tui::ScreenTarget::Preferences(
            skit_ui::PreferencesControlId::ManageAgents,
        ));
        let keyboard = corpus_parity_after(
            namespace.clone(),
            &[
                CorpusOperation::CommandKeyboard(UiCommand::Preferences),
                focus.clone(),
                CorpusOperation::RawKey(CorpusKeyEvent::press(
                    CorpusKey::Enter,
                    CorpusModifiers::NONE,
                )),
            ],
        );
        let mouse = corpus_parity_after(
            namespace,
            &[
                CorpusOperation::CommandKeyboard(UiCommand::Preferences),
                focus,
                CorpusOperation::ScreenHit(skit_tui::ScreenTarget::Preferences(
                    skit_ui::PreferencesControlId::ManageAgents,
                )),
            ],
        );

        assert_eq!(keyboard, mouse);
        assert!(matches!(keyboard.state.screen(), Screen::Runners(_)));
    }

    #[test]
    fn corpus_screen_agent_hit_uses_name_and_scope_from_the_live_overlay() {
        let mut factory = smoke_factory();
        factory
            .seed
            .directories
            .push(super::super::tui_real_host::WalkerDirectorySeed {
                root: super::super::tui_real_host::WalkerDirectoryRoot::Home,
                path: PathBuf::from(".codex/skills"),
            });
        let mut engine = corpus_engine_for_factory(factory);
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        for operation in [
            CorpusOperation::CommandKeyboard(UiCommand::Preferences),
            CorpusOperation::ScreenFocus(skit_tui::ScreenTarget::Preferences(
                skit_ui::PreferencesControlId::InstallAgentSkill,
            )),
            CorpusOperation::ScreenHit(skit_tui::ScreenTarget::Preferences(
                skit_ui::PreferencesControlId::InstallAgentSkill,
            )),
            CorpusOperation::ScreenHit(skit_tui::ScreenTarget::AgentSkill {
                name: "codex".to_owned(),
                scope: skit_application::AgentScope::User,
            }),
        ] {
            engine.run_operation(operation, &mut sink).unwrap();
        }

        assert!(matches!(
            engine.frontend().inner.state.screen(),
            Screen::Preferences(view) if view.agent_skill_install().is_none()
        ));
        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();
    }

    #[test]
    fn corpus_screen_resolver_has_distinct_total_refusals_and_event_chains() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let (mut frontend, host) = engine.into_parts();
        let target = skit_tui::ScreenTarget::Add(skit_tui::AddControlId::RunnerOption(1));
        let operation = CorpusOperation::ScreenHit(target.clone());
        let resolve = |operation: &CorpusOperation, inventory: &skit_tui::ScreenTargetInventory| {
            resolution_parts(
                resolve_screen_operation_with_inventory(&frontend.inner, operation, inventory)
                    .unwrap(),
            )
        };
        let assert_refusal = |inventory: skit_tui::ScreenTargetInventory,
                              expected: CorpusNotApplicable| {
            let (resolved, events) = resolve(&operation, &inventory);
            assert!(events.is_empty());
            assert_eq!(resolved.refusal, Some(expected));
        };
        assert_refusal(
            skit_tui::ScreenTargetInventory::default(),
            CorpusNotApplicable::Unavailable,
        );
        assert_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![target.clone()],
                ..skit_tui::ScreenTargetInventory::default()
            },
            CorpusNotApplicable::Clipped,
        );
        assert_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![target.clone(), target.clone()],
                ..skit_tui::ScreenTargetInventory::default()
            },
            CorpusNotApplicable::Ambiguous,
        );
        let hit = |rect| skit_tui::ScreenTargetHit {
            target: target.clone(),
            rect,
        };
        assert_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![target.clone()],
                hits: vec![hit(Rect::new(0, 0, 0, 1))],
                focus: None,
            },
            CorpusNotApplicable::Clipped,
        );
        assert_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![target.clone()],
                hits: vec![hit(Rect::new(60, 0, 1, 1))],
                focus: None,
            },
            CorpusNotApplicable::Clipped,
        );
        assert_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![target.clone()],
                hits: vec![hit(Rect::new(1, 1, 1, 1)), hit(Rect::new(2, 1, 1, 1))],
                focus: None,
            },
            CorpusNotApplicable::Ambiguous,
        );
        let inventory_error = |inventory: skit_tui::ScreenTargetInventory| {
            resolve_screen_operation_with_inventory(&frontend.inner, &operation, &inventory)
                .unwrap_err()
        };
        assert!(
            inventory_error(skit_tui::ScreenTargetInventory {
                available: Vec::new(),
                hits: vec![hit(Rect::new(1, 1, 1, 1))],
                focus: None,
            })
            .contains("hit is absent")
        );
        assert!(
            inventory_error(skit_tui::ScreenTargetInventory {
                available: Vec::new(),
                hits: Vec::new(),
                focus: Some(skit_tui::ScreenFocusInventory {
                    current: None,
                    order: vec![target.clone()],
                }),
            })
            .contains("focus target is absent")
        );
        assert!(
            inventory_error(skit_tui::ScreenTargetInventory {
                available: vec![target.clone()],
                hits: Vec::new(),
                focus: Some(skit_tui::ScreenFocusInventory {
                    current: Some(target.clone()),
                    order: Vec::new(),
                }),
            })
            .contains("current screen focus is absent")
        );

        let focus_target = skit_tui::ScreenTarget::Add(skit_tui::AddControlId::Draft(0));
        let current = skit_tui::ScreenTarget::Add(skit_tui::AddControlId::Continue);
        let focus_operation = CorpusOperation::ScreenFocus(focus_target.clone());
        let assert_focus_refusal =
            |inventory: skit_tui::ScreenTargetInventory, expected: CorpusNotApplicable| {
                let (resolved, events) = resolve(&focus_operation, &inventory);
                assert!(events.is_empty());
                assert_eq!(resolved.refusal, Some(expected));
            };
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory::default(),
            CorpusNotApplicable::Unavailable,
        );
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![focus_target.clone(), focus_target.clone()],
                ..skit_tui::ScreenTargetInventory::default()
            },
            CorpusNotApplicable::Ambiguous,
        );
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![focus_target.clone()],
                ..skit_tui::ScreenTargetInventory::default()
            },
            CorpusNotApplicable::NotKeyboardFocusable,
        );
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![focus_target.clone(), current.clone()],
                focus: Some(skit_tui::ScreenFocusInventory {
                    current: Some(current.clone()),
                    order: vec![current.clone()],
                }),
                hits: Vec::new(),
            },
            CorpusNotApplicable::NotKeyboardFocusable,
        );
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![focus_target.clone(), current.clone()],
                focus: Some(skit_tui::ScreenFocusInventory {
                    current: Some(current.clone()),
                    order: vec![focus_target.clone(), focus_target.clone(), current.clone()],
                }),
                hits: Vec::new(),
            },
            CorpusNotApplicable::Ambiguous,
        );
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![focus_target.clone()],
                focus: Some(skit_tui::ScreenFocusInventory {
                    current: None,
                    order: vec![focus_target.clone()],
                }),
                hits: Vec::new(),
            },
            CorpusNotApplicable::NotKeyboardFocusable,
        );
        assert_focus_refusal(
            skit_tui::ScreenTargetInventory {
                available: vec![focus_target.clone(), current.clone()],
                focus: Some(skit_tui::ScreenFocusInventory {
                    current: Some(current.clone()),
                    order: vec![focus_target.clone(), current.clone(), current.clone()],
                }),
                hits: Vec::new(),
            },
            CorpusNotApplicable::Ambiguous,
        );
        let focus_inventory = skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone(), current.clone()],
            focus: Some(skit_tui::ScreenFocusInventory {
                current: Some(current.clone()),
                order: vec![focus_target.clone(), current],
            }),
            hits: Vec::new(),
        };
        let (resolved, events) = resolution_parts(
            resolve_screen_operation_with_inventory(
                &frontend.inner,
                &focus_operation,
                &focus_inventory,
            )
            .unwrap(),
        );
        assert_eq!(resolved.input, CorpusInputKind::EventChain);
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.as_slice(),
            [CorpusEvent::Key(CorpusKeyEvent {
                code: CorpusKey::BackTab,
                ..
            })]
        ));

        let already = skit_tui::ScreenTargetInventory {
            available: vec![focus_target.clone()],
            focus: Some(skit_tui::ScreenFocusInventory {
                current: Some(focus_target.clone()),
                order: vec![focus_target.clone()],
            }),
            hits: Vec::new(),
        };
        let (resolved, events) = resolution_parts(
            resolve_screen_operation_with_inventory(&frontend.inner, &focus_operation, &already)
                .unwrap(),
        );
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::AlreadyFocused));

        let agent = skit_tui::ScreenTarget::AgentSkill {
            name: "codex".to_owned(),
            scope: skit_application::AgentScope::Project,
        };
        let (resolved, events) = resolution_parts(
            resolve_screen_operation_with_inventory(
                &frontend.inner,
                &CorpusOperation::ScreenFocus(agent.clone()),
                &skit_tui::ScreenTargetInventory {
                    available: vec![agent],
                    focus: None,
                    hits: Vec::new(),
                },
            )
            .unwrap(),
        );
        assert!(events.is_empty());
        assert_eq!(
            resolved.refusal,
            Some(CorpusNotApplicable::NotKeyboardFocusable)
        );

        let invalid = CorpusOperation::ScreenHit(skit_tui::ScreenTarget::FilePickerEntry {
            relative: PathBuf::from("../outside"),
        });
        assert!(frontend.resolve(&invalid).is_err());
        assert!(
            resolve_screen_operation_with_inventory(
                &frontend.inner,
                &CorpusOperation::RawKey(CorpusKeyEvent::press(
                    CorpusKey::Enter,
                    CorpusModifiers::NONE,
                )),
                &skit_tui::ScreenTargetInventory::default(),
            )
            .unwrap_err()
            .contains("non-screen operation")
        );
        let geometry = frontend.inner.geometry.clone();
        let state = frontend.inner.state.clone();
        let _ = frontend
            .inner
            .session
            .handle_event(Event::FocusLost, &state, &geometry);
        assert!(frontend.resolve(&operation).is_err());

        drop(frontend);
        host.close().unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn record_stable_corpus_prefix(
        namespace: StableSandboxNamespace,
        operations: &[CorpusOperation],
    ) -> RecordedRealTrace {
        let factory = smoke_factory();
        record_corpus_profile(&factory, namespace, operations, StablePairPhase::Main).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn corpus_keyboard_and_mouse_paths_reach_the_same_command_and_local_endpoint() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let back = LocalActionTarget::Health(HealthAction::Back);
        let keyboard = corpus_parity_after(
            namespace.clone(),
            &[
                CorpusOperation::CommandKeyboard(UiCommand::Health),
                CorpusOperation::LocalKeyboard {
                    target: back.clone(),
                    key: CorpusKeyEvent::press(CorpusKey::Escape, CorpusModifiers::NONE),
                },
            ],
        );
        let mouse = corpus_parity_after(
            namespace,
            &[
                CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Health)),
                CorpusOperation::LocalHit(back),
            ],
        );

        assert_eq!(keyboard, mouse);
        assert!(matches!(keyboard.state.screen(), Screen::Library));
    }

    /// Publish one hit that reaches past the right edge of the viewport.
    fn out_of_bounds_probe_geometry(geometry: &ViewGeometry) -> ViewGeometry {
        let mut forged = geometry.clone();
        forged.hits.push(skit_tui::HitRegion {
            rect: Rect::new(0, 0, u16::MAX, 1),
            action: HitTarget::Command(UiCommand::Help),
        });
        forged
    }

    fn corpus_engine_with_probe(probe: fn(&ViewGeometry) -> ViewGeometry) -> CorpusEngine {
        let factory = smoke_factory();
        let host = RealWalkerHost::spawn(factory.seed).unwrap();
        let file_picker_tree = host.file_picker_tree();
        let state = host.initial_state().unwrap();
        let mut frontend =
            CorpusFrontend::new(state, factory.locale, factory.size, file_picker_tree).unwrap();
        frontend.inner.probe_geometry = probe;
        WalkerEngine::new(
            frontend,
            CorpusHostBoundary {
                inner: RealHostBoundary { host },
                leak_sandbox: None,
            },
            EFFECT_LIMIT,
        )
        .unwrap()
    }

    #[test]
    fn a_forged_geometry_poisons_the_engine_with_the_named_parity_rule() {
        let mut engine = corpus_engine_with_probe(out_of_bounds_probe_geometry);
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;

        let error = engine.start(&mut sink).unwrap_err();

        assert!(error.contains("PUBLIC_HIT_BOUNDS"), "{error}");
        assert!(engine.is_poisoned());
        close_corpus_engine(engine).unwrap();
    }

    #[test]
    fn a_consumed_operation_records_a_session_checkpoint_and_empties_the_local_inventory() {
        let mut engine = corpus_engine();
        let mut sink = CorpusRefusalSink::default();
        engine.start(&mut sink).unwrap();
        let before = sink.last;

        engine
            .run_operation(CorpusOperation::Focus { gained: true }, &mut sink)
            .unwrap();

        assert_ne!(sink.last, before);
        assert_eq!(sink.last, Some((EnginePhase::Operations, Some(0))));
        assert_eq!(sink.boundaries.last(), Some(&EngineBoundary::Session));
        assert!(sink.refusals.is_empty());

        engine
            .run_operation(CorpusOperation::CommandKeyboard(UiCommand::Add), &mut sink)
            .unwrap();
        let browse = engine
            .frontend()
            .inner
            .session
            .local_action_inventory()
            .actions
            .iter()
            .find(|advertised| {
                advertised.target == LocalActionTarget::Add(AddControlId::BrowseSource)
            })
            .cloned()
            .expect("the rendered Add footer advertises its path picker");
        engine
            .run_operation(
                CorpusOperation::LocalKeyboard {
                    target: browse.target.clone(),
                    key: CorpusKeyEvent::from_terminal(browse.keys[0].event()).unwrap(),
                },
                &mut sink,
            )
            .unwrap();

        assert!(
            engine
                .frontend()
                .inner
                .session
                .local_action_inventory()
                .actions
                .is_empty(),
            "the open local overlay leaves the inventory empty"
        );
        assert_eq!(sink.boundaries.last(), Some(&EngineBoundary::Session));
        close_corpus_engine(engine).unwrap();
    }

    #[test]
    fn a_forged_local_descriptor_refuses_with_the_named_endpoint_rule() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        engine
            .run_operation(CorpusOperation::CommandKeyboard(UiCommand::Add), &mut sink)
            .unwrap();
        let live = engine
            .frontend()
            .inner
            .session
            .local_action_inventory()
            .actions
            .clone();
        let advertised = live
            .iter()
            .find(|advertised| {
                advertised.target == LocalActionTarget::Add(AddControlId::BrowseSource)
            })
            .cloned()
            .expect("the Add screen advertises its source picker");
        let operation = CorpusOperation::LocalKeyboard {
            target: advertised.target.clone(),
            key: CorpusKeyEvent::from_terminal(advertised.keys[0].event()).unwrap(),
        };
        let mut forged = advertised;
        forged.outcome = LocalActionOutcome::Action(Action::Quit);

        assert!(
            resolve_corpus_operation_with_local_actions(
                &engine.frontend().inner,
                &operation,
                &live
            )
            .is_ok()
        );
        let error = resolve_corpus_operation_with_local_actions(
            &engine.frontend().inner,
            &operation,
            &[forged],
        )
        .unwrap_err();

        assert!(error.contains("LOCAL_ACTION_ENDPOINT"), "{error}");
        assert!(!engine.is_quit());
        close_corpus_engine(engine).unwrap();
    }

    #[test]
    fn random_inventory_reports_a_stale_render_instead_of_empty_targets() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let inner = &engine.frontend().inner;
        let mut frontend = CorpusFrontend::from_real(
            RealFrontend::new(inner.state.clone(), Locale::En, Size::new(80, 24)).unwrap(),
            engine.host().inner.host.file_picker_tree(),
        );
        frontend.inner.observe_frontend().unwrap();
        assert!(frontend.live_inventory().is_ok());
        let _ = frontend.inner.session.handle_event(
            Event::FocusLost,
            &frontend.inner.state,
            &frontend.inner.geometry,
        );
        assert_eq!(
            frontend.live_inventory().unwrap_err(),
            "the screen target inventory is stale"
        );
        close_corpus_engine(engine).unwrap();
    }

    #[test]
    fn raw_quit_press_is_dispatched_without_inventing_a_release() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let frontend = &engine.frontend().inner;
        let rect = frontend
            .geometry
            .hits
            .iter()
            .find(|hit| hit.action == HitTarget::Command(UiCommand::Quit) && !hit.rect.is_empty())
            .unwrap()
            .rect;
        let operation = CorpusOperation::RawMouse {
            column: rect.x,
            row: rect.y,
            kind: CorpusMouseKind::PrimaryDown,
        };
        let (resolved, events) =
            resolution_parts(resolve_corpus_operation(frontend, &operation).unwrap());
        assert_eq!(resolved.refusal, None);
        assert_eq!(
            events,
            vec![CorpusEvent::mouse(
                rect.x,
                rect.y,
                CorpusMouseKind::PrimaryDown
            )]
        );
        engine.run_operation(operation, &mut sink).unwrap();
        let release = CorpusOperation::RawMouse {
            column: rect.x,
            row: rect.y,
            kind: CorpusMouseKind::PrimaryUp,
        };
        let (resolved, events) =
            resolution_parts(resolve_corpus_operation(&engine.frontend().inner, &release).unwrap());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::EventWouldQuit));
        assert!(events.is_empty());
        close_corpus_engine(engine).unwrap();
    }

    #[test]
    fn every_cell_of_the_live_quit_chip_accepts_an_unarmed_raw_event() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let frontend = &engine.frontend().inner;
        let quit = frontend
            .geometry
            .hits
            .iter()
            .find(|hit| hit.action == HitTarget::Command(UiCommand::Quit) && !hit.rect.is_empty())
            .cloned()
            .expect("the library footer advertises a visible Quit chip");

        let mut swept = 0_usize;
        for row in quit.rect.rows() {
            for column in quit.rect.columns() {
                for kind in [CorpusMouseKind::PrimaryDown, CorpusMouseKind::PrimaryUp] {
                    let operation = CorpusOperation::RawMouse {
                        column: column.x,
                        row: row.y,
                        kind,
                    };
                    let (resolved, events) =
                        resolution_parts(resolve_corpus_operation(frontend, &operation).unwrap());
                    assert_eq!(events, vec![CorpusEvent::mouse(column.x, row.y, kind)]);
                    assert_eq!(
                        resolved.refusal, None,
                        "cell {},{} kind {kind:?}",
                        column.x, row.y
                    );
                    swept = swept.saturating_add(1);
                }
            }
        }
        assert!(swept > 0);

        let outside = CorpusOperation::RawMouse {
            column: 0,
            row: 0,
            kind: CorpusMouseKind::Move,
        };
        let (resolved, events) =
            resolution_parts(resolve_corpus_operation(frontend, &outside).unwrap());
        assert_eq!(resolved.refusal, None);
        assert_eq!(
            events,
            vec![CorpusEvent::mouse(0, 0, CorpusMouseKind::Move)]
        );
        close_corpus_engine(engine).unwrap();
    }

    #[test]
    fn every_pointer_kind_keeps_its_raw_event_shape() {
        for kind in CorpusMouseKind::ALL {
            let event = CorpusEvent::mouse(3, 4, *kind);
            assert_eq!(event.value()["kind"], json!(kind.label()));
            assert!(
                matches!(event.terminal(), Event::Mouse(mouse) if mouse.kind == kind.terminal())
            );
            assert_eq!(
                corpus_operation_value(&CorpusOperation::RawMouse {
                    column: 3,
                    row: 4,
                    kind: *kind,
                })
                .unwrap(),
                json!({
                    "operation": "raw_mouse",
                    "column": 3,
                    "row": 4,
                    "kind": kind.label(),
                })
            );
        }
        assert_eq!(
            CorpusMouseKind::ALL
                .iter()
                .map(|kind| kind.label())
                .collect::<Vec<_>>(),
            [
                "primary_down",
                "secondary_down",
                "middle_down",
                "primary_up",
                "primary_drag",
                "move",
                "scroll_up",
                "scroll_down",
            ]
        );
        assert_eq!(
            corpus_canvas_size(
                Size::new(10, 10),
                &[CorpusOperation::RawMouse {
                    column: 0,
                    row: 0,
                    kind: CorpusMouseKind::Move,
                }],
            ),
            Size::new(10, 10)
        );
        assert!(
            resolve_screen_operation_with_inventory(
                &RealFrontend::new(LibraryState::default(), Locale::En, Size::new(4, 4)).unwrap(),
                &CorpusOperation::RawMouse {
                    column: 0,
                    row: 0,
                    kind: CorpusMouseKind::Move,
                },
                &ScreenTargetInventory {
                    available: Vec::new(),
                    focus: None,
                    hits: Vec::new(),
                },
            )
            .unwrap_err()
            .contains("non-screen operation")
        );
    }

    #[test]
    fn corpus_resolver_records_typed_refusals_and_real_input_plans() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let frontend = engine.frontend();

        for operation in [
            CorpusOperation::CommandKeyboard(UiCommand::Quit),
            CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Quit)),
        ] {
            let (resolved, events) = resolution_parts(frontend.resolve(&operation).unwrap());
            assert!(events.is_empty());
            assert_eq!(resolved.refusal, Some(CorpusNotApplicable::QuitFiltered));
        }

        let raw_quit = CorpusOperation::RawKey(CorpusKeyEvent::press(
            CorpusKey::Character('q'),
            CorpusModifiers::NONE,
        ));
        let (resolved, events) = resolution_parts(frontend.resolve(&raw_quit).unwrap());
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::EventWouldQuit));

        let unavailable_command = CorpusOperation::CommandKeyboard(UiCommand::SavePreferences);
        let (resolved, events) = resolution_parts(frontend.resolve(&unavailable_command).unwrap());
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Unavailable));

        let missing = CorpusOperation::HitTarget(HitTarget::FocusField(usize::MAX));
        let (resolved, events) = resolution_parts(frontend.resolve(&missing).unwrap());
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Unavailable));
        assert_eq!(
            corpus_resolution_value(TimelinePhase::Operations, &resolved).unwrap(),
            json!({
                "input": {"kind": "not_applicable"},
                "semantic_target": {"requested": {
                    "operation": "hit_target",
                    "target": {"focus_field": usize::MAX},
                }},
                "refusal": "unavailable",
            })
        );

        let invalid_resize = CorpusOperation::Resize {
            width: 0,
            height: 24,
        };
        let (resolved, events) = resolution_parts(frontend.resolve(&invalid_resize).unwrap());
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::InvalidViewport));

        let target = LocalActionTarget::Health(HealthAction::Back);
        let key = CorpusKeyEvent::press(CorpusKey::Enter, CorpusModifiers::NONE);
        let local_keyboard = CorpusOperation::LocalKeyboard {
            target: target.clone(),
            key,
        };
        let local_hit = CorpusOperation::LocalHit(target.clone());
        let assert_local_refusal =
            |operation: &CorpusOperation,
             local_actions: &[LocalAdvertisedAction],
             expected: CorpusNotApplicable| {
                let (resolved, events) = resolution_parts(
                    resolve_corpus_operation_with_local_actions(
                        &frontend.inner,
                        operation,
                        local_actions,
                    )
                    .unwrap(),
                );
                assert!(events.is_empty());
                assert_eq!(resolved.refusal, Some(expected));
            };
        assert_local_refusal(&local_keyboard, &[], CorpusNotApplicable::Unavailable);
        assert_local_refusal(&local_hit, &[], CorpusNotApplicable::Unavailable);

        let advertised = LocalAdvertisedAction {
            target,
            keys: Vec::new(),
            hit: Some(Rect::new(2, 3, 4, 1)),
            outcome: LocalActionOutcome::Consumed,
        };
        let duplicate_actions = [advertised.clone(), advertised.clone()];
        assert_local_refusal(
            &local_keyboard,
            &duplicate_actions,
            CorpusNotApplicable::Unavailable,
        );
        assert_local_refusal(
            &local_hit,
            &duplicate_actions,
            CorpusNotApplicable::Unavailable,
        );
        assert_local_refusal(
            &local_keyboard,
            std::slice::from_ref(&advertised),
            CorpusNotApplicable::Unavailable,
        );

        let mut clipped = advertised.clone();
        clipped.hit = None;
        assert_local_refusal(
            &local_hit,
            std::slice::from_ref(&clipped),
            CorpusNotApplicable::Clipped,
        );
        clipped.hit = Some(Rect::new(2, 3, 0, 1));
        assert_local_refusal(
            &local_hit,
            std::slice::from_ref(&clipped),
            CorpusNotApplicable::Clipped,
        );

        for operation in [
            CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Function(24),
                CorpusModifiers::NONE,
            )),
            CorpusOperation::Paste("typed".to_owned()),
            CorpusOperation::Focus { gained: false },
            CorpusOperation::Resize {
                width: 72,
                height: 20,
            },
            CorpusOperation::FinalLiveness,
        ] {
            let (resolved, events) = resolution_parts(frontend.resolve(&operation).unwrap());
            assert_eq!(events.len(), 1);
            let phase = if operation == CorpusOperation::FinalLiveness {
                TimelinePhase::FinalLiveness
            } else {
                TimelinePhase::Operations
            };
            assert!(corpus_resolution_value(phase, &resolved).is_ok());
        }

        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();
    }

    #[test]
    fn corpus_clipped_hit_is_not_applicable_without_guessing_an_ordinal() {
        let mut engine = corpus_engine();
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        let (mut frontend, host) = engine.into_parts();
        let target = HitTarget::FocusField(42);
        frontend.inner.geometry.hits.push(skit_tui::HitRegion {
            rect: Rect::new(3, 4, 0, 1),
            action: target,
        });

        let (resolved, events) = resolution_parts(
            frontend
                .resolve(&CorpusOperation::HitTarget(target))
                .unwrap(),
        );
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Clipped));

        let duplicate_target = HitTarget::FocusField(43);
        for rect in [Rect::new(1, 1, 2, 1), Rect::new(5, 1, 2, 1)] {
            frontend.inner.geometry.hits.push(skit_tui::HitRegion {
                rect,
                action: duplicate_target,
            });
        }
        let (resolved, events) = resolution_parts(
            frontend
                .resolve(&CorpusOperation::HitTarget(duplicate_target))
                .unwrap(),
        );
        assert!(events.is_empty());
        assert_eq!(resolved.refusal, Some(CorpusNotApplicable::Unavailable));

        drop(frontend);
        host.close().unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn canonical_corpus_has_exact_operation_count_and_required_profile_order() {
        let operations = canonical_corpus_operations();
        assert_eq!(operations.len(), 100);
        assert_eq!(
            operations[98],
            CorpusOperation::Resize {
                width: 24,
                height: 6,
            }
        );
        assert_eq!(operations[99], CorpusOperation::Focus { gained: true });
        assert!(!operations.iter().any(|operation| matches!(
            operation,
            CorpusOperation::CommandKeyboard(UiCommand::Quit)
                | CorpusOperation::HitTarget(HitTarget::Command(UiCommand::Quit))
                | CorpusOperation::FinalLiveness
        )));
        assert!(!operations.iter().any(|operation| matches!(
            operation,
            CorpusOperation::RawKey(CorpusKeyEvent {
                code: CorpusKey::Enter,
                ..
            })
        )));
        let values = operations
            .iter()
            .map(corpus_operation_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let bytes = canonical_json_bytes(&Value::Array(values)).unwrap();
        assert_eq!(bytes.len(), 7_839);
        assert_eq!(
            sha256_hex(&bytes),
            "824a42732e08119f3c57286f636d3f12936e3a6b0aa99947c8da17db92d0e922"
        );

        let factories = required_corpus_factories().unwrap();
        assert_eq!(factories.len(), 4);
        for (factory, required) in factories
            .iter()
            .zip(skit_tui_walker_support::required_review_profiles())
        {
            assert_eq!(factory.review_profile.as_str(), required.id);
            assert_eq!(factory.locale.tag(), required.locale);
            assert_eq!(factory.size.width, required.viewport.width);
            assert_eq!(factory.size.height, required.viewport.height);
            assert_eq!(corpus_canvas_size(factory.size, &operations), factory.size);
        }
    }

    #[test]
    fn corpus_refusal_sink_records_a_real_not_applicable_operation() {
        let mut engine = corpus_engine();
        let mut sink = CorpusRefusalSink::default();
        engine.start(&mut sink).unwrap();
        engine
            .run_operation(
                CorpusOperation::HitTarget(HitTarget::FocusField(usize::MAX)),
                &mut sink,
            )
            .unwrap();
        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();

        assert_eq!(sink.refusals, [(0, CorpusNotApplicable::Unavailable)]);
        assert_eq!(sink.last, Some((EnginePhase::Operations, Some(0))));
    }

    #[test]
    fn local_keyboard_uses_the_exact_key_when_one_add_target_has_two_chords() {
        let mut engine = corpus_engine_for_factory(required_corpus_factories().unwrap().remove(0));
        let mut sink = skit_tui_walker_support::engine::NoopCheckpointSink;
        engine.start(&mut sink).unwrap();
        for operation in [
            CorpusOperation::CommandKeyboard(UiCommand::Add),
            CorpusOperation::ScreenFocus(ScreenTarget::Add(AddControlId::NewScript)),
        ] {
            engine.run_operation(operation, &mut sink).unwrap();
        }
        let operation = CorpusOperation::LocalKeyboard {
            target: LocalActionTarget::Add(AddControlId::NewScript),
            key: CorpusKeyEvent::press(CorpusKey::Enter, CorpusModifiers::NONE),
        };
        let actions = engine
            .frontend()
            .inner
            .session
            .local_action_inventory()
            .actions
            .clone();
        assert_eq!(
            actions
                .iter()
                .filter(|advertised| advertised.target
                    == LocalActionTarget::Add(AddControlId::NewScript))
                .count(),
            2
        );
        let enter = actions
            .iter()
            .find(|advertised| {
                advertised.keys.iter().any(|binding| {
                    CorpusKeyEvent::from_terminal(binding.event())
                        == Some(CorpusKeyEvent::press(
                            CorpusKey::Enter,
                            CorpusModifiers::NONE,
                        ))
                })
            })
            .unwrap()
            .clone();
        let (duplicate, events) = resolution_parts(
            resolve_corpus_operation_with_local_actions(
                &engine.frontend().inner,
                &operation,
                &[enter.clone(), enter],
            )
            .unwrap(),
        );
        assert!(events.is_empty());
        assert_eq!(duplicate.refusal, Some(CorpusNotApplicable::Unavailable));
        engine.run_operation(operation, &mut sink).unwrap();
        let review = matches!(
            engine.frontend().inner.state.screen(),
            Screen::Add(view) if view.stage() == skit_ui::AddStage::Review
        );
        let (frontend, host) = engine.into_parts();
        drop(frontend);
        host.close().unwrap();

        assert!(review);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn canonical_corpus_preflight_returns_the_real_host_to_the_library() {
        let operations = canonical_corpus_operations();
        let mut refusals = Vec::new();
        for factory in required_corpus_factories().unwrap() {
            let profile = factory.review_profile.clone();
            let parent = tempfile::TempDir::new().unwrap();
            let namespace =
                StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE))
                    .unwrap();
            let mut engine = stable_corpus_engine_for_factory(factory, namespace);
            let mut sink = CorpusRefusalSink::default();
            let result = (|| {
                engine.start(&mut sink)?;
                let mut contexts = Vec::new();
                for (index, operation) in operations.iter().enumerate() {
                    let operation_context = format!(
                        "profile {profile} operation {} {operation:?} failed",
                        index.saturating_add(1)
                    );
                    engine
                        .run_operation(operation.clone(), &mut sink)
                        .expect(&operation_context);
                    contexts.push((
                        index.saturating_add(1),
                        engine.frontend().inner.state.command_context(),
                    ));
                }
                engine
                    .run_final_liveness(CorpusOperation::FinalLiveness, &mut sink)
                    .map_err(|error| format!("profile {profile} final liveness failed: {error}"))?;
                let successful_operations = engine.successful_operations().len();
                let quit = engine.is_quit();
                let library = matches!(engine.frontend().inner.state.screen(), Screen::Library)
                    && engine.frontend().inner.state.modal().is_none();
                let final_cause = sink.last == Some((EnginePhase::FinalLiveness, None));
                Ok::<_, String>((
                    library,
                    successful_operations,
                    quit,
                    final_cause,
                    sink.refusals.clone(),
                    contexts,
                ))
            })();
            let (frontend, host) = engine.into_parts();
            drop(frontend);
            host.close().unwrap();
            let (library, successful_operations, quit, final_cause, profile_refusals, contexts) =
                result.unwrap();
            let final_contexts = &contexts[contexts.len().saturating_sub(20)..];

            assert!(
                library,
                "profile {profile} refusals {profile_refusals:?}; final contexts are {:?}",
                final_contexts
            );
            assert_eq!(successful_operations, 100, "profile {profile}");
            assert!(!quit, "profile {profile} requested quit");
            assert!(final_cause, "profile {profile} has a bad final cause");
            refusals.push((profile, profile_refusals));
        }
        assert!(
            refusals
                .iter()
                .all(|(_, profile_refusals)| profile_refusals.is_empty()),
            "canonical refusals: {refusals:?}"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn short_stable_corpus_prefix_replays_the_complete_real_trace() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let missing = CorpusOperation::HitTarget(HitTarget::FocusField(usize::MAX));
        let operations = vec![
            CorpusOperation::CommandKeyboard(UiCommand::Add),
            CorpusOperation::ScreenHit(ScreenTarget::Add(AddControlId::BrowseSource)),
            CorpusOperation::ScreenHit(ScreenTarget::FilePickerEntry {
                relative: PathBuf::from("original.sh"),
            }),
            CorpusOperation::ScreenFocus(ScreenTarget::Add(AddControlId::Continue)),
            CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Enter,
                CorpusModifiers::NONE,
            )),
            CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Escape,
                CorpusModifiers::NONE,
            )),
            CorpusOperation::RawKey(CorpusKeyEvent::press(
                CorpusKey::Escape,
                CorpusModifiers::NONE,
            )),
            missing.clone(),
        ];

        let main = record_stable_corpus_prefix(namespace.clone(), &operations);
        let replay = record_stable_corpus_prefix(namespace, &operations);

        assert_eq!(main.trace, replay.trace);
        assert_eq!(main.sandbox, replay.sandbox);
        assert_eq!(main.operations.len(), operations.len());
        assert_eq!(
            main.operations,
            operations
                .iter()
                .map(corpus_operation_value)
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        );
        assert_eq!(
            main.final_liveness_requested,
            corpus_operation_value(&CorpusOperation::FinalLiveness).unwrap()
        );
        let missing_value = corpus_operation_value(&missing).unwrap();
        assert!(main.rows.iter().any(|row| {
            matches!(
                &row.cause,
                TransitionCause::Session {
                    requested,
                    handling,
                    ..
                } if requested == &missing_value && handling == "not_applicable"
            )
        }));
        assert_eq!(
            main.rows.last().unwrap().phase,
            TimelinePhase::FinalLiveness
        );
        assert_eq!(
            main.rows.last().unwrap().liveness,
            Some(LivenessResult::Passed)
        );
        validate_effect_chain_termination(&main.rows).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn diagnostic_stable_corpus_resize_replays_backend_terminal_event_and_viewport_bytes() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let resize = CorpusOperation::Resize {
            width: 48,
            height: 18,
        };
        let operations = [resize.clone()];

        let main = record_stable_corpus_prefix(namespace.clone(), &operations);
        let replay = record_stable_corpus_prefix(namespace, &operations);
        let resize_value = corpus_operation_value(&resize).unwrap();

        assert_eq!(main.trace, replay.trace);
        assert_eq!(main.sandbox, replay.sandbox);
        assert_eq!(
            main.operations.as_slice(),
            std::slice::from_ref(&resize_value)
        );
        let (row, resolved, event, handling) = main
            .rows
            .iter()
            .find_map(|row| match &row.cause {
                TransitionCause::Session {
                    requested,
                    resolved,
                    event,
                    handling,
                } if requested == &resize_value => Some((row, resolved, event, handling)),
                TransitionCause::Initial
                | TransitionCause::Session { .. }
                | TransitionCause::Reducer { .. }
                | TransitionCause::Host { .. } => None,
            })
            .unwrap();
        let expected_event = json!({
            "type": "resize",
            "width": 48,
            "height": 18,
        });
        assert_eq!(event, &expected_event);
        assert_eq!(
            resolved,
            &json!({
                "input": {
                    "kind": "resize",
                    "event": expected_event,
                },
                "semantic_target": {"requested": {
                    "operation": "resize",
                    "width": 48,
                    "height": 18,
                }},
            })
        );
        assert_eq!(handling, "ignored");
        assert_eq!(
            row.viewport,
            RectSnapshot {
                x: 0,
                y: 0,
                width: 48,
                height: 18,
            }
        );
        let frame_bytes = &main.objects[&(row.styled_frame.kind, row.styled_frame.sha256.clone())];
        let frame: StyledFrameSnapshot =
            serde_json::from_value(validate_object(&row.styled_frame, frame_bytes).unwrap())
                .unwrap();
        assert_eq!(frame.area, row.viewport);
        assert_eq!(main.rows.last().unwrap().viewport, row.viewport);

        let cast_header: Value =
            serde_json::from_slice(main.cast.split(|byte| *byte == b'\n').next().unwrap()).unwrap();
        assert_eq!(
            cast_header,
            json!({"version": 3, "term": {"cols": 60, "rows": 24}})
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_corpus_grow_resize_uses_the_timeline_maximum_canvas() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let operations = [CorpusOperation::Resize {
            width: 72,
            height: 30,
        }];
        let recorded = record_stable_corpus_prefix(namespace, &operations);

        assert_eq!(bundle::cast_canvas(&recorded.rows).unwrap(), (72, 30));
        let cast_header: Value =
            serde_json::from_slice(recorded.cast.split(|byte| *byte == b'\n').next().unwrap())
                .unwrap();
        assert_eq!(
            cast_header,
            json!({"version": 3, "term": {"cols": 72, "rows": 30}})
        );
        let rebuilt = bundle::rebuild_presented_cast(&recorded.rows, |reference| {
            let bytes = &recorded.objects[&(reference.kind, reference.sha256.clone())];
            let value = validate_object(reference, bytes).map_err(|error| error.to_string())?;
            serde_json::from_value(value).map_err(|error| error.to_string())
        })
        .unwrap();
        assert_eq!(recorded.cast, rebuilt);
    }

    #[test]
    fn corpus_canvas_forecast_ignores_each_zero_dimension_resize() {
        let initial = Size::new(60, 24);
        let operations = [
            CorpusOperation::Resize {
                width: 0,
                height: 100,
            },
            CorpusOperation::Resize {
                width: 100,
                height: 0,
            },
            CorpusOperation::Resize {
                width: 72,
                height: 12,
            },
            CorpusOperation::Resize {
                width: 40,
                height: 30,
            },
            CorpusOperation::FinalLiveness,
        ];
        assert_eq!(corpus_canvas_size(initial, &operations), Size::new(72, 30));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_corpus_zero_dimension_resizes_do_not_expand_the_canvas() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE)).unwrap();
        let operations = [
            CorpusOperation::Resize {
                width: 0,
                height: 100,
            },
            CorpusOperation::Resize {
                width: 100,
                height: 0,
            },
        ];
        let recorded = record_stable_corpus_prefix(namespace, &operations);

        assert_eq!(bundle::cast_canvas(&recorded.rows).unwrap(), (60, 24));
        let requested = operations
            .iter()
            .map(corpus_operation_value)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let refusals = recorded
            .rows
            .iter()
            .filter_map(|row| match &row.cause {
                TransitionCause::Session {
                    requested,
                    resolved,
                    handling,
                    ..
                } if handling == "not_applicable" => Some((requested, resolved)),
                TransitionCause::Initial
                | TransitionCause::Session { .. }
                | TransitionCause::Reducer { .. }
                | TransitionCause::Host { .. } => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(refusals.len(), 2);
        for ((actual, resolved), expected) in refusals.into_iter().zip(&requested) {
            assert_eq!(actual, expected);
            assert_eq!(resolved["refusal"], "invalid_viewport");
        }
    }

    #[test]
    fn corpus_machine_mappings_are_total_and_stable() {
        let key_cases = [
            (CorpusKey::Character('界'), json!({"character": "界"})),
            (CorpusKey::Enter, json!("enter")),
            (CorpusKey::Escape, json!("escape")),
            (CorpusKey::Delete, json!("delete")),
            (CorpusKey::Backspace, json!("backspace")),
            (CorpusKey::Tab, json!("tab")),
            (CorpusKey::BackTab, json!("back_tab")),
            (CorpusKey::Up, json!("up")),
            (CorpusKey::Down, json!("down")),
            (CorpusKey::Left, json!("left")),
            (CorpusKey::Right, json!("right")),
            (CorpusKey::PageUp, json!("page_up")),
            (CorpusKey::PageDown, json!("page_down")),
            (CorpusKey::Home, json!("home")),
            (CorpusKey::End, json!("end")),
            (CorpusKey::Function(2), json!({"function": 2})),
        ];
        for (key, value) in key_cases {
            assert_eq!(key.value(), value);
            assert_eq!(CorpusKey::from_terminal(key.terminal()), Some(key));
        }
        assert_eq!(CorpusKey::from_terminal(KeyCode::Insert), None);

        for (key, expected) in [
            (UiKey::Character('x'), CorpusKey::Character('x')),
            (UiKey::Enter, CorpusKey::Enter),
            (UiKey::Escape, CorpusKey::Escape),
            (UiKey::Delete, CorpusKey::Delete),
            (UiKey::Backspace, CorpusKey::Backspace),
            (UiKey::Tab, CorpusKey::Tab),
            (UiKey::BackTab, CorpusKey::BackTab),
            (UiKey::Up, CorpusKey::Up),
            (UiKey::Down, CorpusKey::Down),
            (UiKey::PageUp, CorpusKey::PageUp),
            (UiKey::PageDown, CorpusKey::PageDown),
            (UiKey::Home, CorpusKey::Home),
            (UiKey::End, CorpusKey::End),
            (UiKey::Function(12), CorpusKey::Function(12)),
        ] {
            assert_eq!(CorpusKey::from_ui(key), expected);
        }

        let modifiers = CorpusModifiers {
            control: true,
            alt: true,
            shift: true,
        };
        assert_eq!(
            CorpusModifiers::from_terminal(modifiers.terminal()),
            modifiers
        );
        for kind in [
            CorpusKeyKind::Press,
            CorpusKeyKind::Repeat,
            CorpusKeyKind::Release,
        ] {
            let key = CorpusKeyEvent {
                code: CorpusKey::Character('x'),
                modifiers,
                kind,
            };
            assert_eq!(CorpusKeyEvent::from_terminal(key.terminal()), Some(key));
            assert_eq!(key.value()["kind"], kind.label());
        }

        for (target, expected) in [
            (
                HitTarget::Command(UiCommand::Health),
                json!({"command": "health"}),
            ),
            (
                HitTarget::RunFieldCommand {
                    field: 1,
                    command: RunFieldCommand::BrowsePath,
                },
                json!({"run_field_command": {"field": 1, "command": "browse_path"}}),
            ),
            (
                HitTarget::RunFieldCommand {
                    field: 2,
                    command: RunFieldCommand::InsertValue,
                },
                json!({"run_field_command": {"field": 2, "command": "insert_value"}}),
            ),
            (
                HitTarget::RunFieldCommand {
                    field: 3,
                    command: RunFieldCommand::ResetDefault,
                },
                json!({"run_field_command": {"field": 3, "command": "reset_default"}}),
            ),
            (HitTarget::FocusField(4), json!({"focus_field": 4})),
            (HitTarget::ToggleField(5), json!({"toggle_field": 5})),
            (
                HitTarget::SelectFieldOption {
                    field: 6,
                    option: 7,
                },
                json!({"select_field_option": {"field": 6, "option": 7}}),
            ),
        ] {
            assert_eq!(corpus_hit_target_value(target), expected);
        }

        for (target, key) in [
            (
                LocalActionTarget::Add(skit_tui::AddControlId::Cancel),
                "add",
            ),
            (LocalActionTarget::Health(HealthAction::Back), "health"),
            (
                LocalActionTarget::Runners(RunnerManagerAction::Back),
                "runners",
            ),
            (
                LocalActionTarget::RunnerEditor(RunnerEditorAction::Cancel),
                "runner_editor",
            ),
        ] {
            assert!(corpus_local_target_value(&target).get(key).is_some());
        }
        for (field, expected) in [
            (AddTextField::SourcePath, "source_path"),
            (AddTextField::CommandTemplate, "command_template"),
            (AddTextField::CommandName, "command_name"),
            (AddTextField::CommandDescription, "command_description"),
            (AddTextField::ReviewName, "review_name"),
            (AddTextField::ReviewDescription, "review_description"),
            (AddTextField::Dependencies, "dependencies"),
            (AddTextField::PythonConstraint, "python_constraint"),
        ] {
            assert_eq!(corpus_add_text_field_value(field), json!(expected));
        }
        for (target, expected) in [
            (
                AddControlId::Text(AddTextField::SourcePath),
                json!({"text": "source_path"}),
            ),
            (AddControlId::BrowseSource, json!("browse_source")),
            (AddControlId::Draft(2), json!({"draft": 2})),
            (AddControlId::NewScript, json!("new_script")),
            (AddControlId::NewPrompt, json!("new_prompt")),
            (AddControlId::DeleteDraft, json!("delete_draft")),
            (AddControlId::Continue, json!("continue")),
            (AddControlId::Kind(3), json!({"kind": 3})),
            (AddControlId::PickFocusedKind, json!("pick_focused_kind")),
            (AddControlId::Storage, json!("storage")),
            (AddControlId::StorageOption(1), json!({"storage_option": 1})),
            (
                AddControlId::Candidate("python".to_owned()),
                json!({"candidate": "python"}),
            ),
            (AddControlId::Interpolate, json!("interpolate")),
            (
                AddControlId::PromptCandidate("name".to_owned()),
                json!({"prompt_candidate": "name"}),
            ),
            (AddControlId::Runner, json!("runner")),
            (AddControlId::RunnerOption(4), json!({"runner_option": 4})),
            (AddControlId::NewRunner, json!("new_runner")),
            (AddControlId::EditSource, json!("edit_source")),
            (AddControlId::Save, json!("save")),
            (AddControlId::ToggleFocused, json!("toggle_focused")),
            (AddControlId::NextField, json!("next_field")),
            (AddControlId::PreviousField, json!("previous_field")),
            (AddControlId::Cancel, json!("cancel")),
        ] {
            assert_eq!(corpus_add_control_value(&target), expected);
        }
        for (target, expected) in [
            (PreferencesControlId::Language, "language"),
            (PreferencesControlId::Editor, "editor"),
            (PreferencesControlId::InteractiveForm, "interactive_form"),
            (PreferencesControlId::AfterRun, "after_run"),
            (PreferencesControlId::Javascript, "javascript"),
            (PreferencesControlId::BashPath, "bash_path"),
            (PreferencesControlId::ManageAgents, "manage_agents"),
            (
                PreferencesControlId::InstallAgentSkill,
                "install_agent_skill",
            ),
            (PreferencesControlId::MirrorMaster, "mirror_master"),
            (PreferencesControlId::PypiChoice, "pypi_choice"),
            (PreferencesControlId::PypiUrl, "pypi_url"),
            (PreferencesControlId::GithubChoice, "github_choice"),
            (PreferencesControlId::GithubUrl, "github_url"),
            (PreferencesControlId::NpmChoice, "npm_choice"),
            (PreferencesControlId::NpmUrl, "npm_url"),
        ] {
            assert_eq!(corpus_preferences_control_value(target), json!(expected));
        }
        assert_eq!(
            [AgentScope::User, AgentScope::Project].map(corpus_agent_scope_label),
            ["user", "project"]
        );
        for (target, expected) in [
            (
                ScreenTarget::Preferences(PreferencesControlId::Language),
                json!({"preferences": "language"}),
            ),
            (
                ScreenTarget::AgentSkill {
                    name: "codex".to_owned(),
                    scope: AgentScope::Project,
                },
                json!({"agent_skill": {"name": "codex", "scope": "project"}}),
            ),
            (
                ScreenTarget::Runner {
                    name: "seed-agent".to_owned(),
                },
                json!({"runner": {"name": "seed-agent"}}),
            ),
            (
                ScreenTarget::FilePickerEntry {
                    relative: PathBuf::from("nested/picked.py"),
                },
                json!({"file_picker_entry": {"components": ["nested", "picked.py"]}}),
            ),
        ] {
            assert_eq!(corpus_screen_target_value(&target).unwrap(), expected);
        }
        assert_eq!(
            corpus_local_outcome_value(&LocalActionOutcome::Consumed),
            json!("consumed")
        );
        assert_eq!(
            corpus_local_outcome_value(&LocalActionOutcome::Action(Action::Back)),
            json!({"action": "back"})
        );
        assert_eq!(
            corpus_semantic_target_value(&CorpusSemanticTarget::Hit {
                target: HitTarget::FocusField(7),
                rect: Rect::new(2, 3, 4, 5),
            })
            .unwrap(),
            json!({
                "hit_target": {"focus_field": 7},
                "rect": {"x": 2, "y": 3, "width": 4, "height": 5},
            })
        );
        let local = corpus_semantic_target_value(&CorpusSemanticTarget::Local {
            target: LocalActionTarget::Health(HealthAction::Back),
            keys: vec![CorpusKeyEvent::press(
                CorpusKey::Escape,
                CorpusModifiers::NONE,
            )],
            hit: Some(Rect::new(1, 2, 3, 4)),
            outcome: LocalActionOutcome::Consumed,
        })
        .unwrap();
        assert_eq!(local["local_target"], json!({"health": "back"}));
        assert_eq!(
            local["hit"],
            json!({"x": 1, "y": 2, "width": 3, "height": 4})
        );

        assert_eq!(
            CorpusMouseKind::ALL
                .iter()
                .map(|kind| kind.label())
                .collect::<Vec<_>>(),
            [
                "primary_down",
                "secondary_down",
                "middle_down",
                "primary_up",
                "primary_drag",
                "move",
                "scroll_up",
                "scroll_down",
            ]
        );
        for kind in CorpusMouseKind::ALL {
            let event = CorpusEvent::mouse(2, 3, *kind);
            assert_eq!(
                event.value(),
                json!({
                    "type": "mouse",
                    "kind": kind.label(),
                    "column": 2,
                    "row": 3,
                    "modifiers": {"alt": false, "control": false, "shift": false},
                })
            );
            assert!(
                matches!(event.terminal(), Event::Mouse(mouse) if mouse.kind == kind.terminal())
            );
        }

        let events = [
            CorpusEvent::Key(CorpusKeyEvent::press(
                CorpusKey::Enter,
                CorpusModifiers::NONE,
            )),
            CorpusEvent::mouse(2, 3, CorpusMouseKind::PrimaryDown),
            CorpusEvent::mouse(2, 3, CorpusMouseKind::PrimaryUp),
            CorpusEvent::Paste("value".to_owned()),
            CorpusEvent::Focus { gained: true },
            CorpusEvent::Focus { gained: false },
            CorpusEvent::Resize {
                width: 80,
                height: 24,
            },
        ];
        for event in events {
            assert!(event.value()["type"].is_string());
            let _ = event.terminal();
        }

        assert_eq!(
            [
                CorpusInputKind::NotApplicable,
                CorpusInputKind::Event,
                CorpusInputKind::EventChain,
                CorpusInputKind::PrimaryClick,
                CorpusInputKind::LocalEvent,
                CorpusInputKind::LocalPrimaryClick,
                CorpusInputKind::Resize,
            ]
            .map(CorpusInputKind::label),
            [
                "not_applicable",
                "event",
                "event_chain",
                "primary_click",
                "local_event",
                "local_primary_click",
                "resize",
            ]
        );
        assert_eq!(
            [
                CorpusNotApplicable::Unavailable,
                CorpusNotApplicable::Clipped,
                CorpusNotApplicable::Ambiguous,
                CorpusNotApplicable::AlreadyFocused,
                CorpusNotApplicable::NotKeyboardFocusable,
                CorpusNotApplicable::InvalidViewport,
                CorpusNotApplicable::QuitFiltered,
                CorpusNotApplicable::EventWouldQuit,
            ]
            .map(CorpusNotApplicable::label),
            [
                "unavailable",
                "clipped",
                "ambiguous",
                "already_focused",
                "not_keyboard_focusable",
                "invalid_viewport",
                "quit_filtered",
                "event_would_quit",
            ]
        );

        let target = CorpusSemanticTarget::Requested(CorpusOperation::Focus { gained: true });
        let invalid_single = CorpusResolution {
            input: CorpusInputKind::Event,
            events: Vec::new(),
            target: target.clone(),
            refusal: None,
        };
        assert!(
            corpus_resolution_value(TimelinePhase::Operations, &invalid_single)
                .unwrap_err()
                .contains("one event")
        );
        assert!(
            corpus_resolution_value(TimelinePhase::FinalLiveness, &invalid_single)
                .unwrap_err()
                .contains("one real event")
        );
        let invalid_click = CorpusResolution {
            input: CorpusInputKind::PrimaryClick,
            events: vec![CorpusEvent::Focus { gained: true }],
            target,
            refusal: None,
        };
        assert!(
            corpus_resolution_value(TimelinePhase::Operations, &invalid_click)
                .unwrap_err()
                .contains("press and release")
        );
        let invalid_chain = CorpusResolution {
            input: CorpusInputKind::EventChain,
            events: Vec::new(),
            target: CorpusSemanticTarget::Requested(CorpusOperation::Focus { gained: true }),
            refusal: None,
        };
        assert!(
            corpus_resolution_value(TimelinePhase::Operations, &invalid_chain)
                .unwrap_err()
                .contains("at least one event")
        );
        assert_eq!(
            [
                ScreenTargetError::ScreenUnavailable,
                ScreenTargetError::StaleSession,
                ScreenTargetError::RealFilesystemPicker,
                ScreenTargetError::InvalidMemoryEntry,
            ]
            .map(screen_target_error_message),
            [
                "the screen target inventory is unavailable",
                "the screen target inventory is stale",
                "the screen target inventory uses the real filesystem picker",
                "the screen target inventory contains an invalid memory entry",
            ]
        );
    }

    #[test]
    fn corpus_file_targets_require_strict_utf8_relative_components() {
        let empty_runner = ScreenTarget::Runner {
            name: String::new(),
        };
        assert!(validate_screen_target(&empty_runner).is_err());
        assert!(corpus_operation_value(&CorpusOperation::ScreenHit(empty_runner)).is_err());
        let assert_invalid = |relative: PathBuf| {
            let target = ScreenTarget::FilePickerEntry { relative };
            assert!(validate_screen_target(&target).is_err());
            assert!(corpus_operation_value(&CorpusOperation::ScreenHit(target)).is_err());
        };
        for spelling in [
            "",
            ".",
            "..",
            "/absolute",
            "nested//original.sh",
            "./nested/original.sh",
            "nested/./original.sh",
            "nested/../original.sh",
            "nested/",
        ] {
            assert_invalid(PathBuf::from(spelling));
        }
        #[cfg(windows)]
        for spelling in [
            r"C:original.sh",
            r"C:\nested\original.sh",
            r"\\server\share\original.sh",
        ] {
            assert_invalid(PathBuf::from(spelling));
        }
        #[cfg(unix)]
        {
            use std::{ffi::OsStr, os::unix::ffi::OsStrExt as _};
            assert_invalid(PathBuf::from(OsStr::from_bytes(b"original-\xff.sh")));
        }
        #[cfg(windows)]
        {
            use std::{ffi::OsString, os::windows::ffi::OsStringExt as _};
            assert_invalid(PathBuf::from(OsString::from_wide(&[
                u16::from(b'o'),
                0xd800,
                u16::from(b's'),
            ])));
        }
        assert_eq!(
            corpus_relative_component_values(Path::new("nested/original.sh")).unwrap(),
            [json!("nested"), json!("original.sh")]
        );
    }

    #[test]
    fn corpus_screen_operations_and_resolutions_have_exact_outer_shapes() {
        let focus_target = ScreenTarget::Preferences(PreferencesControlId::ManageAgents);
        let focus_current = ScreenTarget::Preferences(PreferencesControlId::Language);
        let focus_order = vec![focus_current.clone(), focus_target.clone()];
        let focus_operation = CorpusOperation::ScreenFocus(focus_target.clone());
        let focus_operation_value = json!({
            "operation": "screen_focus",
            "target": {"preferences": "manage_agents"},
        });
        assert_eq!(
            corpus_operation_value(&focus_operation).unwrap(),
            focus_operation_value
        );
        let focus_resolution = CorpusResolution {
            input: CorpusInputKind::EventChain,
            events: vec![CorpusEvent::Key(CorpusKeyEvent::press(
                CorpusKey::Tab,
                CorpusModifiers::NONE,
            ))],
            target: CorpusSemanticTarget::ScreenFocus {
                target: focus_target,
                current: focus_current,
                order: focus_order,
            },
            refusal: None,
        };
        assert_eq!(
            corpus_resolution_value(TimelinePhase::Operations, &focus_resolution).unwrap(),
            json!({
                "input": {
                    "kind": "event_chain",
                    "events": [{
                        "type": "key",
                        "key": {
                            "code": "tab",
                            "kind": "press",
                            "modifiers": {"alt": false, "control": false, "shift": false},
                        },
                    }],
                },
                "semantic_target": {"screen_focus": {
                    "target": {"preferences": "manage_agents"},
                    "current": {"preferences": "language"},
                    "order": [
                        {"preferences": "language"},
                        {"preferences": "manage_agents"},
                    ],
                }},
            })
        );

        for (target, target_value) in [
            (
                ScreenTarget::Add(AddControlId::BrowseSource),
                json!({"add": "browse_source"}),
            ),
            (
                ScreenTarget::AgentSkill {
                    name: "codex".to_owned(),
                    scope: AgentScope::Project,
                },
                json!({"agent_skill": {"name": "codex", "scope": "project"}}),
            ),
            (
                ScreenTarget::Runner {
                    name: "seed-agent".to_owned(),
                },
                json!({"runner": {"name": "seed-agent"}}),
            ),
            (
                ScreenTarget::FilePickerEntry {
                    relative: PathBuf::from("nested/original.sh"),
                },
                json!({"file_picker_entry": {"components": ["nested", "original.sh"]}}),
            ),
        ] {
            let operation = CorpusOperation::ScreenHit(target.clone());
            let operation_value = corpus_operation_value(&operation).unwrap();
            assert_eq!(
                operation_value,
                json!({"operation": "screen_hit", "target": target_value.clone()})
            );
            let resolution = CorpusResolution {
                input: CorpusInputKind::PrimaryClick,
                events: vec![
                    CorpusEvent::mouse(4, 3, CorpusMouseKind::PrimaryDown),
                    CorpusEvent::mouse(4, 3, CorpusMouseKind::PrimaryUp),
                ],
                target: CorpusSemanticTarget::ScreenHit {
                    target,
                    rect: Rect::new(2, 3, 5, 1),
                },
                refusal: None,
            };
            let resolution_value =
                corpus_resolution_value(TimelinePhase::Operations, &resolution).unwrap();
            assert_eq!(
                resolution_value,
                json!({
                    "input": {
                        "kind": "primary_click",
                        "events": [
                            {
                                "type": "mouse",
                                "kind": "primary_down",
                                "column": 4,
                                "row": 3,
                                "modifiers": {"alt": false, "control": false, "shift": false},
                            },
                            {
                                "type": "mouse",
                                "kind": "primary_up",
                                "column": 4,
                                "row": 3,
                                "modifiers": {"alt": false, "control": false, "shift": false},
                            },
                        ],
                    },
                    "semantic_target": {"screen_hit": {
                        "target": target_value,
                        "rect": {"x": 2, "y": 3, "width": 5, "height": 1},
                    }},
                })
            );
            let machine_bytes = format!("{operation_value}{resolution_value}");
            for forbidden in ["/nested", "\\nested", "\"base\"", "skills_dir"] {
                assert!(!machine_bytes.contains(forbidden), "found {forbidden}");
            }
        }
    }

    #[test]
    fn corpus_operations_have_stable_source_owned_shapes() {
        let raw = CorpusKeyEvent::press(
            CorpusKey::Character('*'),
            CorpusModifiers {
                control: true,
                alt: false,
                shift: true,
            },
        );
        let local = LocalActionTarget::Health(HealthAction::Back);
        let cases = [
            (
                CorpusOperation::CommandKeyboard(UiCommand::Health),
                json!({"operation": "command_keyboard", "command": "health"}),
            ),
            (
                CorpusOperation::HitTarget(skit_tui::HitTarget::Command(UiCommand::Health)),
                json!({
                    "operation": "hit_target",
                    "target": {"command": "health"},
                }),
            ),
            (
                CorpusOperation::RawKey(raw),
                json!({
                    "operation": "raw_key",
                    "key": {
                        "code": {"character": "*"},
                        "kind": "press",
                        "modifiers": {"alt": false, "control": true, "shift": true},
                    },
                }),
            ),
            (
                CorpusOperation::LocalKeyboard {
                    target: local.clone(),
                    key: CorpusKeyEvent::press(CorpusKey::Escape, CorpusModifiers::NONE),
                },
                json!({
                    "operation": "local_keyboard",
                    "target": {"health": "back"},
                    "key": {
                        "code": "escape",
                        "kind": "press",
                        "modifiers": {"alt": false, "control": false, "shift": false},
                    },
                }),
            ),
            (
                CorpusOperation::LocalHit(local),
                json!({"operation": "local_hit", "target": {"health": "back"}}),
            ),
            (
                CorpusOperation::RawMouse {
                    column: 3,
                    row: 4,
                    kind: CorpusMouseKind::ScrollUp,
                },
                json!({
                    "operation": "raw_mouse",
                    "column": 3,
                    "row": 4,
                    "kind": "scroll_up",
                }),
            ),
            (
                CorpusOperation::Paste("value".to_owned()),
                json!({"operation": "paste", "value": "value"}),
            ),
            (
                CorpusOperation::Focus { gained: false },
                json!({"operation": "focus", "gained": false}),
            ),
            (
                CorpusOperation::Resize {
                    width: 80,
                    height: 24,
                },
                json!({"operation": "resize", "width": 80, "height": 24}),
            ),
            (
                CorpusOperation::FinalLiveness,
                json!({"synthetic": "final_liveness"}),
            ),
        ];

        for (operation, expected) in cases {
            assert_eq!(corpus_operation_value(&operation).unwrap(), expected);
        }
    }
}
