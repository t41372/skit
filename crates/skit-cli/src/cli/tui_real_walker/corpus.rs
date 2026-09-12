//! Corpus operations, their JSON values, and their resolution against the live screen.

use std::path::Path;

use ratatui_core::layout::{Rect, Size};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use serde_json::{Value, json};
use skit_application::AgentScope;
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddTextField, EventHandling, HitTarget, LocalActionOutcome, LocalActionTarget,
    LocalAdvertisedAction, RunFieldCommand, ScreenTarget, ScreenTargetError, ScreenTargetInventory,
    TuiSession, map_event,
};
use skit_tui_walker_model::parity;
use skit_tui_walker_support::{
    StyledFrameSnapshot, TimelinePhase, bundle, engine::OperationResolution,
};
use skit_ui::{
    Action, CommandContext, LibraryState, PreferencesControlId, UiBinding, UiCommand, UiKey,
    command_specs,
};

use super::frontend::RealFrontend;

pub(super) const EFFECT_LIMIT: usize = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SmokeOperation {
    OpenRun,
    OpenPreferences,
    OpenLanguagePicker,
    NextLanguage,
    ChooseLanguage,
    SavePreferences,
    FinalLiveness,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocaleSmokeTarget {
    SimplifiedChinese,
    TraditionalChinese,
}

impl LocaleSmokeTarget {
    pub(super) const fn tag(self) -> &'static str {
        match self {
            Self::SimplifiedChinese => "zh-CN",
            Self::TraditionalChinese => "zh-TW",
        }
    }

    pub(super) const fn next_count(self) -> usize {
        match self {
            Self::SimplifiedChinese => 2,
            Self::TraditionalChinese => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmokeResolution {
    pub(super) event: SmokeEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SmokeEvent {
    pub(super) key: KeyCode,
    pub(super) modifiers: KeyModifiers,
}

impl SmokeEvent {
    pub(super) fn terminal_event(self) -> Event {
        Event::Key(KeyEvent::new(self.key, self.modifiers))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SmokeHandling {
    Consumed,
    Ignored,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct CorpusModifiers {
    pub(super) control: bool,
    pub(super) alt: bool,
    pub(super) shift: bool,
}

impl CorpusModifiers {
    pub(super) const NONE: Self = Self {
        control: false,
        alt: false,
        shift: false,
    };

    pub(super) fn terminal(self) -> KeyModifiers {
        let mut modifiers = KeyModifiers::NONE;
        modifiers.set(KeyModifiers::CONTROL, self.control);
        modifiers.set(KeyModifiers::ALT, self.alt);
        modifiers.set(KeyModifiers::SHIFT, self.shift);
        modifiers
    }

    pub(super) fn from_terminal(modifiers: KeyModifiers) -> Self {
        Self {
            control: modifiers.contains(KeyModifiers::CONTROL),
            alt: modifiers.contains(KeyModifiers::ALT),
            shift: modifiers.contains(KeyModifiers::SHIFT),
        }
    }

    pub(super) fn value(self) -> Value {
        json!({
            "alt": self.alt,
            "control": self.control,
            "shift": self.shift,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CorpusKey {
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
    pub(super) const fn terminal(self) -> KeyCode {
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

    pub(super) fn from_terminal(code: KeyCode) -> Option<Self> {
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

    pub(super) fn from_ui(key: UiKey) -> Self {
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

    pub(super) fn value(self) -> Value {
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
pub(super) enum CorpusKeyKind {
    Press,
    Repeat,
    Release,
}

impl CorpusKeyKind {
    pub(super) const fn terminal(self) -> KeyEventKind {
        match self {
            Self::Press => KeyEventKind::Press,
            Self::Repeat => KeyEventKind::Repeat,
            Self::Release => KeyEventKind::Release,
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Press => "press",
            Self::Repeat => "repeat",
            Self::Release => "release",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CorpusKeyEvent {
    pub(super) code: CorpusKey,
    pub(super) modifiers: CorpusModifiers,
    pub(super) kind: CorpusKeyKind,
}

impl CorpusKeyEvent {
    pub(super) const fn press(code: CorpusKey, modifiers: CorpusModifiers) -> Self {
        Self {
            code,
            modifiers,
            kind: CorpusKeyKind::Press,
        }
    }

    pub(crate) fn from_terminal(event: KeyEvent) -> Option<Self> {
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

    pub(super) fn terminal(self) -> KeyEvent {
        KeyEvent::new_with_kind(
            self.code.terminal(),
            self.modifiers.terminal(),
            self.kind.terminal(),
        )
    }

    pub(super) fn value(self) -> Value {
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
pub(crate) enum CorpusMouseKind {
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
    pub(crate) const ALL: &'static [Self] = &[
        Self::PrimaryDown,
        Self::SecondaryDown,
        Self::MiddleDown,
        Self::PrimaryUp,
        Self::PrimaryDrag,
        Self::Move,
        Self::ScrollUp,
        Self::ScrollDown,
    ];

    pub(super) const fn label(self) -> &'static str {
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

    pub(super) const fn terminal(self) -> MouseEventKind {
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
pub(crate) enum CorpusEvent {
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
    pub(super) const fn mouse(column: u16, row: u16, kind: CorpusMouseKind) -> Self {
        Self::Mouse { column, row, kind }
    }

    pub(super) fn terminal(&self) -> Event {
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

    pub(super) fn value(&self) -> Value {
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
pub(crate) enum CorpusOperation {
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

pub(crate) fn corpus_canvas_size(initial: Size, operations: &[CorpusOperation]) -> Size {
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
pub(super) enum CorpusInputKind {
    NotApplicable,
    Event,
    EventChain,
    PrimaryClick,
    LocalEvent,
    LocalPrimaryClick,
    Resize,
}

impl CorpusInputKind {
    pub(super) const fn label(self) -> &'static str {
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
pub(super) enum CorpusNotApplicable {
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
    pub(super) const fn label(self) -> &'static str {
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
pub(super) enum CorpusSemanticTarget {
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
pub(crate) struct CorpusResolution {
    pub(super) input: CorpusInputKind,
    pub(super) events: Vec<CorpusEvent>,
    pub(super) target: CorpusSemanticTarget,
    pub(super) refusal: Option<CorpusNotApplicable>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FrontendObservation {
    pub(super) state: LibraryState,
    pub(super) session: Value,
    pub(super) styled_frame: StyledFrameSnapshot,
    pub(super) geometry: Value,
    pub(super) locale: Locale,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FrontendParity {
    pub(super) state: LibraryState,
    pub(super) session: Value,
    pub(super) locale: Locale,
}

fn corpus_run_field_command_value(command: RunFieldCommand) -> Value {
    json!(match command {
        RunFieldCommand::BrowsePath => "browse_path",
        RunFieldCommand::InsertValue => "insert_value",
        RunFieldCommand::ResetDefault => "reset_default",
    })
}

pub(super) fn corpus_hit_target_value(target: HitTarget) -> Value {
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

pub(super) fn corpus_local_target_value(target: &LocalActionTarget) -> Value {
    match target {
        LocalActionTarget::Add(target) => json!({"add": target}),
        LocalActionTarget::Health(action) => json!({"health": action}),
        LocalActionTarget::Runners(action) => json!({"runners": action}),
        LocalActionTarget::RunnerEditor(action) => json!({"runner_editor": action}),
    }
}

pub(super) fn corpus_add_text_field_value(field: AddTextField) -> Value {
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

pub(super) fn corpus_add_control_value(target: &AddControlId) -> Value {
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

pub(super) fn corpus_preferences_control_value(target: PreferencesControlId) -> Value {
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

pub(super) const fn corpus_agent_scope_label(scope: AgentScope) -> &'static str {
    match scope {
        AgentScope::User => "user",
        AgentScope::Project => "project",
    }
}

pub(super) fn corpus_relative_component_values(path: &Path) -> Result<Vec<Value>, String> {
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

pub(super) fn corpus_screen_target_value(target: &ScreenTarget) -> Result<Value, String> {
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

pub(super) fn corpus_local_outcome_value(outcome: &LocalActionOutcome) -> Value {
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

pub(crate) fn corpus_operation_value(operation: &CorpusOperation) -> Result<Value, String> {
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

pub(super) fn corpus_semantic_target_value(target: &CorpusSemanticTarget) -> Result<Value, String> {
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

pub(super) fn corpus_resolution_value(
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

pub(super) const fn screen_target_error_message(error: ScreenTargetError) -> &'static str {
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

pub(super) fn validate_screen_target(target: &ScreenTarget) -> Result<(), String> {
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

pub(super) fn resolve_screen_operation_with_inventory(
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

pub(super) fn resolve_corpus_operation(
    frontend: &RealFrontend,
    operation: &CorpusOperation,
) -> Result<OperationResolution<CorpusEvent, CorpusResolution>, String> {
    resolve_corpus_operation_with_local_actions(
        frontend,
        operation,
        &frontend.session.local_action_inventory().actions,
    )
}

pub(super) fn resolve_corpus_operation_with_local_actions(
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
