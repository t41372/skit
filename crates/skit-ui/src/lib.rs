//! Frontend-neutral state and reducer shared by terminal and future GUI adapters.

#![forbid(unsafe_code)]

mod add;
mod management;
mod picker;
mod preferences;
mod run;
mod settings;

pub use add::*;
pub use management::*;
pub use picker::*;
pub use preferences::{
    AgentSkillInstallView, PreferencesAction, PreferencesChoiceControl, PreferencesControl,
    PreferencesControlId, PreferencesControlKind, PreferencesDisplayText, PreferencesEffect,
    PreferencesOption, PreferencesSection, PreferencesSectionId, PreferencesTextControl,
    PreferencesTextPlacement, PreferencesView,
};
pub use run::{
    ChoiceControl, ChoicePresentation, FormControl, FormInputKind, RunDegradationNotice, RunField,
    RunFieldFeedback, RunFieldRole, RunFormContext, RunFormOptions, RunFormView, RunPathContext,
    RunTokenError, RunTokenOption, RunValidationError, TextControl,
};
pub use skit_form::field::{
    ArgumentDialect, ChoiceOption, Field, FieldCapabilities, FieldKind, FieldOwner, FieldValue,
    ReadOnlyReason, TypedValue,
};

pub use settings::{
    ADD_PARAMETER_KEY, DEPENDENCIES_KEY, DESCRIPTION_KEY, DependencyFlavor, INTERPOLATE_KEY,
    INTERPRETER_KEY, MANAGE_KEY, NAME_KEY, NEEDS_KEY, NORMALIZE_KEY, PRESET_PREFIX, PYTHON_KEY,
    RESYNC_KEY, RUNNER_KEY, SettingsAction, SettingsEffect, SettingsError, SettingsInputs,
    SettingsItem, SettingsNote, SettingsSection, SettingsSectionId, SettingsView, TEMPLATE_KEY,
    WORKDIR_CUSTOM, WORKDIR_KEY, WORKDIR_PATH_KEY, preset_key,
};
pub use skit_application::path_insertion::RunPathInsertMode;

use std::collections::{BTreeMap, BTreeSet};

use nucleo_matcher::{
    Config as MatcherConfig, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};
use serde::{Deserialize, Serialize};
use skit_application::{Diagnostic, LibraryScan};
// The Library detail facts are stable frontend data, so they live in the application layer next to
// `LibraryScan`. Re-exported here because every frontend reaches them through the view model.
pub use skit_application::library_detail::{
    LibraryEntryDetail, LibraryLastRun, LibraryParameterDetail, LibraryPromptRunner, LibraryRunAge,
    LibrarySurface,
};
use skit_domain::{EntrySummary, Slug};

/// Which key grammar is currently active.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputMode {
    /// Navigation and command shortcuts.
    #[default]
    Browse,
    /// Printable keys edit the library filter.
    Search,
    /// Printable keys edit the active form field.
    Form,
}

/// A frontend-neutral key used by the command registry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiKey {
    /// One printable character.
    Character(char),
    /// The Enter key.
    Enter,
    /// The Escape key.
    Escape,
    /// The forward-delete key.
    Delete,
    /// The backward-delete key.
    Backspace,
    /// The Tab key.
    Tab,
    /// The reverse Tab key reported by terminal adapters.
    BackTab,
    /// The Up arrow.
    Up,
    /// The Down arrow.
    Down,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Home.
    Home,
    /// End.
    End,
    /// One function key.
    Function(u8),
}

/// Modifier keys attached to a frontend-neutral key.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct UiModifiers {
    /// The Control modifier is active.
    pub control: bool,
    /// The Alt modifier is active.
    pub alt: bool,
    /// The Shift modifier is active.
    pub shift: bool,
}

impl UiModifiers {
    /// No modifier keys.
    pub const NONE: Self = Self {
        control: false,
        alt: false,
        shift: false,
    };
    /// Only Control.
    pub const CONTROL: Self = Self {
        control: true,
        alt: false,
        shift: false,
    };
    /// Only Shift.
    pub const SHIFT: Self = Self {
        control: false,
        alt: false,
        shift: true,
    };
}

/// One key chord and its two presentation hints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiBinding {
    /// The physical or logical key.
    pub key: UiKey,
    /// Active modifier keys.
    pub modifiers: UiModifiers,
    /// Full key hint for a wide footer or help view.
    pub hint: &'static str,
    /// Compact key hint for a narrow footer.
    pub compact_hint: &'static str,
}

macro_rules! plain_binding {
    ($key:expr, $hint:expr, $compact_hint:expr $(,)?) => {
        UiBinding {
            key: $key,
            modifiers: UiModifiers::NONE,
            hint: $hint,
            compact_hint: $compact_hint,
        }
    };
}

macro_rules! control_binding {
    ($key:expr, $hint:expr, $compact_hint:expr $(,)?) => {
        UiBinding {
            key: $key,
            modifiers: UiModifiers::CONTROL,
            hint: $hint,
            compact_hint: $compact_hint,
        }
    };
}

macro_rules! shift_binding {
    ($key:expr, $hint:expr, $compact_hint:expr $(,)?) => {
        UiBinding {
            key: $key,
            modifiers: UiModifiers::SHIFT,
            hint: $hint,
            compact_hint: $compact_hint,
        }
    };
}

/// The active command surface.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandContext {
    /// The library list owns the keyboard.
    LibraryBrowse,
    /// The search field owns printable input.
    LibrarySearch,
    /// A generic form owns the keyboard.
    Form,
    /// The typed launch form owns the keyboard.
    RunForm,
    /// A preset-name dialog owns one mature text input.
    RunPresetName,
    /// A typed run-time value menu owns one mature list picker.
    RunTokenMenu,
    /// The typed application-preferences workflow owns the keyboard.
    Preferences,
    /// The typed add and source-review workflow owns the keyboard.
    Add,
    /// The typed actionable Health workflow owns the keyboard.
    Health,
    /// The typed prompt-runner manager owns the keyboard.
    Runners,
    /// The typed entry-settings workflow owns the keyboard.
    Settings,
    /// The reusable prompt-runner editor modal owns the keyboard.
    RunnerEditor,
    /// A read-only report owns the keyboard.
    Report,
    /// The remove confirmation owns the keyboard.
    ConfirmRemove,
    /// A dirty-edit discard guard owns the keyboard.
    ConfirmDiscard,
    /// The help overlay owns the keyboard.
    Help,
}

/// A stable command identity shared by keys, footers, help, and mouse targets.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UiCommand {
    /// Launch through the visible run form.
    Run,
    /// Repeat the last launch without hiding it behind reload behavior.
    Rerun,
    /// Add an entry.
    Add,
    /// Edit the selected source.
    Edit,
    /// Open entry settings.
    Settings,
    /// Open entry presets.
    Presets,
    /// Rename the selected entry.
    Rename,
    /// Ask to remove the selected entry.
    Remove,
    /// Open application preferences.
    Preferences,
    /// Open the health report.
    Health,
    /// Open the prompt runner manager.
    Runners,
    /// Enter search mode.
    Search,
    /// Leave search mode.
    LeaveSearch,
    /// Toggle and pin the detail pane.
    ToggleDetail,
    /// Open the help overlay.
    Help,
    /// Reload repository data.
    Reload,
    /// Quit the frontend.
    Quit,
    /// Select the preceding row.
    Previous,
    /// Select the next row.
    Next,
    /// Move one page toward the start.
    PagePrevious,
    /// Move one page toward the end.
    PageNext,
    /// Select the first row.
    Home,
    /// Select the last row.
    End,
    /// Delete one search or field character.
    Backspace,
    /// Clear the current search.
    ClearSearch,
    /// Focus the next form field.
    FocusNext,
    /// Focus the previous form field.
    FocusPrevious,
    /// Submit the active form or confirmation.
    Submit,
    /// Open the run-time value insertion menu.
    InsertValue,
    /// Open the filesystem picker for the focused run field.
    BrowsePath,
    /// Restore the focused run field to its declared default.
    ResetDefault,
    /// Name and persist the current parameter snapshot.
    SavePreset,
    /// Open the shared runner editor from a runner picker.
    NewRunner,
    /// Persist the complete validated Preferences transaction.
    SavePreferences,
    /// Close Preferences, with its typed discard confirmation when needed.
    ClosePreferences,
    /// Open prompt-runner management from Preferences.
    ManageAgents,
    /// Discover Agent Skill install targets without writing.
    InstallAgentSkill,
    /// Persist the complete validated entry-settings transaction.
    SaveSettings,
    /// Read the script's own parameter definitions again on the next entry-settings save.
    ResyncSettings,
    /// Close entry settings, through the discard guard when anything moved.
    CloseSettings,
    /// Return to the library workflow.
    Back,
    /// Close the active modal.
    CloseModal,
    /// Discard the active workflow's unsaved edits.
    DiscardChanges,
    /// Close a discard guard and continue editing.
    KeepEditing,
}

impl UiCommand {
    /// Convert a context-free command identity to a frontend-neutral reducer action.
    ///
    /// Contextual commands return `None`. The frontend must attach the rendered
    /// state before it sends those commands to the reducer.
    #[must_use]
    pub const fn direct_action(self) -> Option<Action> {
        Some(match self {
            Self::Run => Action::OpenRun,
            Self::Rerun => Action::Rerun,
            Self::Add => Action::OpenAdd,
            Self::Edit => Action::Edit,
            Self::Settings => Action::OpenSettings,
            Self::Presets => Action::OpenPresets,
            Self::Rename => Action::OpenRename,
            Self::Remove => Action::AskRemove,
            Self::Preferences => Action::OpenPreferences,
            Self::Health => Action::OpenHealth,
            Self::Runners => Action::OpenRunners,
            Self::Search => Action::BeginSearch,
            Self::LeaveSearch => Action::FinishSearch,
            Self::ToggleDetail => return None,
            Self::Help => Action::OpenHelp,
            Self::Reload => Action::Reload,
            Self::Quit => Action::Quit,
            Self::Previous => Action::Previous,
            Self::Next => Action::Next,
            Self::PagePrevious => Action::PagePrevious,
            Self::PageNext => Action::PageNext,
            Self::Home => Action::Home,
            Self::End => Action::End,
            Self::Backspace => Action::Backspace,
            Self::ClearSearch => Action::ClearSearch,
            Self::FocusNext => Action::FocusNext,
            Self::FocusPrevious => Action::FocusPrevious,
            Self::Submit => Action::Submit,
            Self::InsertValue => Action::OpenRunTokenMenu,
            Self::BrowsePath => Action::OpenFocusedRunFilePicker,
            Self::ResetDefault => Action::ResetFocusedRunField,
            Self::SavePreset => Action::OpenRunPresetSave,
            Self::NewRunner => Action::OpenRunRunnerEditor,
            Self::SavePreferences => Action::Preferences(PreferencesAction::Save),
            Self::ClosePreferences => Action::Preferences(PreferencesAction::Close),
            Self::ManageAgents => Action::Preferences(PreferencesAction::ManageAgents),
            Self::InstallAgentSkill => Action::Preferences(PreferencesAction::InstallAgentSkill),
            Self::SaveSettings => Action::Settings(SettingsAction::Save),
            Self::ResyncSettings => Action::Settings(SettingsAction::Resync),
            Self::CloseSettings => Action::Settings(SettingsAction::Close),
            Self::Back | Self::CloseModal => Action::Back,
            Self::DiscardChanges => Action::DiscardChanges,
            Self::KeepEditing => Action::KeepEditing,
        })
    }
}

/// One declarative command entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiCommandSpec {
    /// Stable command identity.
    pub command: UiCommand,
    /// Surface on which the command is active.
    pub context: CommandContext,
    /// Keyboard chords that invoke the command. The first chord is advertised.
    pub bindings: &'static [UiBinding],
    /// English source key for the localized label.
    pub label: &'static str,
    /// Show this command in the contextual footer.
    pub footer: bool,
    /// Show this command in the library help overlay.
    pub help: bool,
}

macro_rules! command_spec {
    ($command:expr, $context:expr, $bindings:expr, $label:expr, $footer:expr, $help:expr $(,)?) => {
        UiCommandSpec {
            command: $command,
            context: $context,
            bindings: $bindings,
            label: $label,
            footer: $footer,
            help: $help,
        }
    };
}

const PLAIN_ENTER: UiBinding = plain_binding!(UiKey::Enter, "Enter", "↵");
const PLAIN_ESCAPE: UiBinding = plain_binding!(UiKey::Escape, "Esc", "Esc");
static COMMAND_SPECS: &[UiCommandSpec] = &[
    command_spec!(
        UiCommand::Run,
        CommandContext::LibraryBrowse,
        &[PLAIN_ENTER],
        "Run",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Rerun,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Character('r'), "r", "r")],
        "Rerun",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Settings,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Character('p'), "p", "p")],
        "Entry settings",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Edit,
        CommandContext::LibraryBrowse,
        &[
            plain_binding!(UiKey::Character('e'), "e", "e"),
            control_binding!(UiKey::Character('e'), "Ctrl+E", "^E"),
        ],
        "Edit source",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Remove,
        CommandContext::LibraryBrowse,
        &[
            plain_binding!(UiKey::Delete, "Del", "Del"),
            plain_binding!(UiKey::Backspace, "Backspace", "⌫"),
        ],
        "Remove",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Add,
        CommandContext::LibraryBrowse,
        &[
            plain_binding!(UiKey::Character('a'), "a", "a"),
            control_binding!(UiKey::Character('n'), "Ctrl+N", "^N"),
        ],
        "Add entry",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Presets,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Character('s'), "s", "s")],
        "Presets",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Search,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Character('/'), "/", "/")],
        "Search",
        true,
        true,
    ),
    command_spec!(
        UiCommand::ToggleDetail,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Tab, "Tab", "Tab")],
        "Detail pane",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Preferences,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Character(','), ",", ",")],
        "Preferences",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Health,
        CommandContext::LibraryBrowse,
        &[
            shift_binding!(UiKey::Character('D'), "D", "D"),
            plain_binding!(UiKey::Character('h'), "h", "h"),
        ],
        "Health check",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Help,
        CommandContext::LibraryBrowse,
        &[shift_binding!(UiKey::Character('?'), "?", "?")],
        "Help",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Runners,
        CommandContext::LibraryBrowse,
        &[shift_binding!(UiKey::Character('R'), "R", "R")],
        "Runners",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Rename,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Function(2), "F2", "F2")],
        "Rename",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Reload,
        CommandContext::LibraryBrowse,
        &[control_binding!(UiKey::Character('r'), "Ctrl+R", "^R")],
        "Reload",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Quit,
        CommandContext::LibraryBrowse,
        &[
            plain_binding!(UiKey::Character('q'), "q", "q"),
            PLAIN_ESCAPE,
        ],
        "Quit",
        true,
        true,
    ),
    command_spec!(
        UiCommand::Previous,
        CommandContext::LibraryBrowse,
        &[
            plain_binding!(UiKey::Up, "Up", "↑"),
            plain_binding!(UiKey::Character('k'), "k", "k"),
        ],
        "Previous",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Next,
        CommandContext::LibraryBrowse,
        &[
            plain_binding!(UiKey::Down, "Down", "↓"),
            plain_binding!(UiKey::Character('j'), "j", "j"),
        ],
        "Next",
        false,
        false,
    ),
    command_spec!(
        UiCommand::PagePrevious,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::PageUp, "Page Up", "PgUp")],
        "Previous page",
        false,
        false,
    ),
    command_spec!(
        UiCommand::PageNext,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::PageDown, "Page Down", "PgDn")],
        "Next page",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Home,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::Home, "Home", "Home")],
        "First",
        false,
        false,
    ),
    command_spec!(
        UiCommand::End,
        CommandContext::LibraryBrowse,
        &[plain_binding!(UiKey::End, "End", "End")],
        "Last",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Run,
        CommandContext::LibrarySearch,
        &[PLAIN_ENTER],
        "Run",
        true,
        false,
    ),
    command_spec!(
        UiCommand::LeaveSearch,
        CommandContext::LibrarySearch,
        &[PLAIN_ESCAPE],
        "Back to list",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Previous,
        CommandContext::LibrarySearch,
        &[plain_binding!(UiKey::Up, "Up", "↑")],
        "Previous",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Next,
        CommandContext::LibrarySearch,
        &[plain_binding!(UiKey::Down, "Down", "↓")],
        "Next",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Backspace,
        CommandContext::LibrarySearch,
        &[plain_binding!(UiKey::Backspace, "Backspace", "⌫")],
        "Backspace",
        false,
        false,
    ),
    command_spec!(
        UiCommand::ClearSearch,
        CommandContext::LibrarySearch,
        &[control_binding!(UiKey::Character('u'), "Ctrl+U", "^U")],
        "Clear search",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Back,
        CommandContext::Form,
        &[PLAIN_ESCAPE],
        "Back",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusNext,
        CommandContext::Form,
        &[
            plain_binding!(UiKey::Tab, "Tab", "Tab"),
            plain_binding!(UiKey::Down, "Down", "↓"),
            PLAIN_ENTER,
        ],
        "Next field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusPrevious,
        CommandContext::Form,
        &[
            shift_binding!(UiKey::BackTab, "Shift+Tab", "⇧Tab"),
            plain_binding!(UiKey::Up, "Up", "↑"),
        ],
        "Previous field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Backspace,
        CommandContext::Form,
        &[plain_binding!(UiKey::Backspace, "Backspace", "⌫")],
        "Backspace",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Submit,
        CommandContext::Form,
        &[control_binding!(UiKey::Character('s'), "Ctrl+S", "^S")],
        "Submit",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Submit,
        CommandContext::RunForm,
        &[
            PLAIN_ENTER,
            control_binding!(UiKey::Character('r'), "Ctrl+R", "^R"),
        ],
        "Run",
        true,
        false,
    ),
    command_spec!(
        UiCommand::InsertValue,
        CommandContext::RunForm,
        &[control_binding!(UiKey::Character('t'), "Ctrl+T", "^T")],
        "Insert value",
        true,
        false,
    ),
    command_spec!(
        UiCommand::ResetDefault,
        CommandContext::RunForm,
        &[control_binding!(UiKey::Character('o'), "Ctrl+O", "^O")],
        "Reset to default",
        true,
        false,
    ),
    command_spec!(
        UiCommand::SavePreset,
        CommandContext::RunForm,
        &[control_binding!(UiKey::Character('s'), "Ctrl+S", "^S")],
        "Save as preset",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Back,
        CommandContext::RunForm,
        &[PLAIN_ESCAPE],
        "Cancel",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusNext,
        CommandContext::RunForm,
        &[
            plain_binding!(UiKey::Tab, "Tab", "Tab"),
            plain_binding!(UiKey::Down, "Down", "↓"),
        ],
        "Next field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusPrevious,
        CommandContext::RunForm,
        &[
            shift_binding!(UiKey::BackTab, "Shift+Tab", "⇧Tab"),
            plain_binding!(UiKey::Up, "Up", "↑"),
        ],
        "Previous field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::NewRunner,
        CommandContext::RunForm,
        &[control_binding!(UiKey::Character('n'), "Ctrl+N", "^N")],
        "New agent",
        false,
        false,
    ),
    command_spec!(
        UiCommand::Submit,
        CommandContext::RunPresetName,
        &[PLAIN_ENTER],
        "Save",
        true,
        false,
    ),
    command_spec!(
        UiCommand::CloseModal,
        CommandContext::RunPresetName,
        &[PLAIN_ESCAPE],
        "Cancel",
        true,
        false,
    ),
    command_spec!(
        UiCommand::CloseModal,
        CommandContext::RunTokenMenu,
        &[PLAIN_ESCAPE],
        "Cancel",
        true,
        false,
    ),
    command_spec!(
        UiCommand::SaveSettings,
        CommandContext::Settings,
        &[control_binding!(UiKey::Character('s'), "Ctrl+S", "^S")],
        "Save",
        true,
        false,
    ),
    command_spec!(
        UiCommand::NewRunner,
        CommandContext::Settings,
        &[control_binding!(UiKey::Character('n'), "Ctrl+N", "^N")],
        "New agent",
        true,
        false,
    ),
    // Version 0.4 advertises this only where a resync does something, because "advertising a key
    // that silently no-ops … teaches a dead chord" (`src/skit/tui_settings.py:408-415`).
    command_spec!(
        UiCommand::ResyncSettings,
        CommandContext::Settings,
        &[control_binding!(UiKey::Character('r'), "Ctrl+R", "^R")],
        "Resync",
        true,
        false,
    ),
    command_spec!(
        UiCommand::CloseSettings,
        CommandContext::Settings,
        &[PLAIN_ESCAPE],
        "Back",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusNext,
        CommandContext::Settings,
        &[
            plain_binding!(UiKey::Tab, "Tab", "Tab"),
            plain_binding!(UiKey::Down, "Down", "↓"),
        ],
        "Next field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusPrevious,
        CommandContext::Settings,
        &[
            shift_binding!(UiKey::BackTab, "Shift+Tab", "⇧Tab"),
            plain_binding!(UiKey::Up, "Up", "↑"),
        ],
        "Previous field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::SavePreferences,
        CommandContext::Preferences,
        &[control_binding!(UiKey::Character('s'), "Ctrl+S", "^S")],
        "Save",
        true,
        false,
    ),
    command_spec!(
        UiCommand::ClosePreferences,
        CommandContext::Preferences,
        &[PLAIN_ESCAPE],
        "Cancel",
        true,
        false,
    ),
    command_spec!(
        UiCommand::ManageAgents,
        CommandContext::Preferences,
        &[control_binding!(UiKey::Character('o'), "Ctrl+O", "^O")],
        "Manage agents…",
        true,
        false,
    ),
    command_spec!(
        UiCommand::InstallAgentSkill,
        CommandContext::Preferences,
        &[control_binding!(UiKey::Character('k'), "Ctrl+K", "^K")],
        "Teach an AI agent skit…",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusNext,
        CommandContext::Preferences,
        &[
            plain_binding!(UiKey::Tab, "Tab", "Tab"),
            plain_binding!(UiKey::Down, "Down", "↓"),
        ],
        "Next field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::FocusPrevious,
        CommandContext::Preferences,
        &[
            shift_binding!(UiKey::BackTab, "Shift+Tab", "⇧Tab"),
            plain_binding!(UiKey::Up, "Up", "↑"),
        ],
        "Previous field",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Back,
        CommandContext::Report,
        &[
            PLAIN_ESCAPE,
            plain_binding!(UiKey::Character('q'), "q", "q"),
        ],
        "Back",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Reload,
        CommandContext::Report,
        &[
            plain_binding!(UiKey::Character('r'), "r", "r"),
            control_binding!(UiKey::Character('r'), "Ctrl+R", "^R"),
        ],
        "Reload",
        true,
        false,
    ),
    command_spec!(
        UiCommand::Submit,
        CommandContext::ConfirmRemove,
        &[plain_binding!(UiKey::Character('y'), "y", "y")],
        "Remove",
        true,
        false,
    ),
    command_spec!(
        UiCommand::CloseModal,
        CommandContext::ConfirmRemove,
        &[
            PLAIN_ESCAPE,
            plain_binding!(UiKey::Character('n'), "n", "n"),
        ],
        "Cancel",
        true,
        false,
    ),
    command_spec!(
        UiCommand::DiscardChanges,
        CommandContext::ConfirmDiscard,
        // Version 0.4 binds `y` alone here (`src/skit/tui_settings.py:43-46`). Enter is left
        // deliberately unbound: a guard exists to catch a reflex, so the answer a reflex reaches
        // must be the one that keeps the work, not the one that throws it away.
        &[plain_binding!(UiKey::Character('y'), "y", "y")],
        "Discard",
        true,
        false,
    ),
    command_spec!(
        UiCommand::KeepEditing,
        CommandContext::ConfirmDiscard,
        &[
            PLAIN_ESCAPE,
            plain_binding!(UiKey::Character('n'), "n", "n"),
        ],
        "Keep editing",
        true,
        false,
    ),
    command_spec!(
        UiCommand::CloseModal,
        CommandContext::Help,
        &[
            PLAIN_ESCAPE,
            shift_binding!(UiKey::Character('?'), "?", "?"),
        ],
        "Close",
        true,
        false,
    ),
];

/// Iterate over every command active on one surface.
pub fn command_specs(
    context: CommandContext,
) -> impl DoubleEndedIterator<Item = &'static UiCommandSpec> {
    COMMAND_SPECS
        .iter()
        .filter(move |command| command.context == context)
}

/// Identify data that a frontend must request from its host adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostRequest {
    /// Build a launch form for the selected entry.
    Run,
    /// Build a form for a new library entry.
    Add,
    /// Build an entry settings form.
    Settings,
    /// Build the application preferences form.
    Preferences,
    /// Build a health report.
    Health,
    /// Build the prompt runner manager.
    Runners,
    /// Build the preset manager for the selected entry.
    Presets,
    /// Build a rename form for the selected entry.
    Rename,
}

/// Everything one screen submits, keyed by stable field key.
///
/// The values are typed, not text. Version 0.4's own settings save reads one widget per axis and
/// never infers an intent from a string (`src/skit/tui_settings.py:928-1001`), and the field model
/// already refuses to let an empty string stand for two different intents: `Inherit` means nothing
/// explicit is set here, `Explicit(Text(""))` means the user cleared it on purpose. Flattening this
/// to text at the boundary would put that inference back, once per frontend, and two frontends that
/// inferred differently would write different records from the same screen.
///
/// A key that is absent was never offered, or never moved. Either way the host leaves that axis
/// exactly as it found it; there is no value it could write that would mean "do not touch".
pub type SubmittedValues = BTreeMap<String, FieldValue>;

/// Identify the operation that owns a generic form.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormPurpose {
    /// Launch one entry.
    Run,
    /// Add one entry.
    Add,
    /// Change one entry.
    Settings,
    /// Change application preferences.
    Preferences,
    /// Add or change a prompt runner.
    Runners,
    /// Rename one entry.
    Rename,
}

/// One editable frontend-neutral form field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormField {
    /// Stable field key used by host adapters.
    pub key: String,
    /// User-visible field label.
    pub label: String,
    /// Values inserted into a catalog label template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_arguments: Vec<String>,
    /// Translate the label as application text instead of preserving user text.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub translate_label: bool,
    /// Current text value.
    pub value: String,
    /// Hide the value during presentation.
    pub secret: bool,
    /// Permit embedded line breaks.
    pub multiline: bool,
}

impl FormField {
    /// Create one plain text field.
    #[must_use]
    pub fn text(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            label_arguments: Vec::new(),
            translate_label: true,
            value: value.into(),
            secret: false,
            multiline: false,
        }
    }

    /// Create one text field whose catalog label takes user-data arguments.
    #[must_use]
    pub fn text_with_arguments(
        key: impl Into<String>,
        label: impl Into<String>,
        label_arguments: Vec<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            label_arguments,
            ..Self::text(key, label, value)
        }
    }

    /// Create one field whose label is user-authored text.
    #[must_use]
    pub fn text_raw(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            translate_label: false,
            ..Self::text(key, label, value)
        }
    }

    /// Create one masked text field.
    #[must_use]
    pub fn secret(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            secret: true,
            ..Self::text(key, label, value)
        }
    }

    /// Create one masked field whose label is user-authored text.
    #[must_use]
    pub fn secret_raw(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            secret: true,
            ..Self::text_raw(key, label, value)
        }
    }

    /// Create one multiline text field.
    #[must_use]
    pub fn multiline(
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            multiline: true,
            ..Self::text(key, label, value)
        }
    }

    /// Create one multiline field whose catalog label takes user-data arguments.
    #[must_use]
    pub fn multiline_with_arguments(
        key: impl Into<String>,
        label: impl Into<String>,
        label_arguments: Vec<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            multiline: true,
            ..Self::text_with_arguments(key, label, label_arguments, value)
        }
    }
}

const fn default_true() -> bool {
    true
}

const fn is_true(value: &bool) -> bool {
    *value
}

/// One complete form that any frontend can render.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormView {
    /// Operation performed when the form is submitted.
    pub purpose: FormPurpose,
    /// User-visible form title.
    pub title: String,
    /// Values inserted into a catalog title template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub title_arguments: Vec<String>,
    /// Translate the title as application text instead of preserving user text.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub translate_title: bool,
    /// Entry selector when the form owns an existing entry.
    pub selector: Option<String>,
    /// Fields in navigation order.
    pub fields: Vec<FormField>,
    /// Active field index.
    pub focused: usize,
    /// User-visible label for the submit action.
    pub submit_label: String,
}

/// One row in a read-only report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportItem {
    /// Short machine-stable status such as `ok` or `error`.
    pub status: String,
    /// User-visible check name.
    pub label: String,
    /// Translate the check name as application text.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub translate_label: bool,
    /// User-visible result detail.
    pub detail: String,
    /// Translate the result detail as application text.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub translate_detail: bool,
}

/// A read-only report that a frontend can present.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReportView {
    /// User-visible report title.
    pub title: String,
    /// Report rows.
    pub items: Vec<ReportItem>,
}

/// Current frontend screen.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Screen {
    /// Searchable library browser.
    #[default]
    Library,
    /// Typed launch form with control semantics and stable field roles.
    Run(Box<RunFormView>),
    /// Typed application Preferences workflow.
    Preferences(Box<PreferencesView>),
    /// Typed add, classification, and pre-commit review workflow.
    Add(Box<AddWorkflowState>),
    /// Typed actionable Health workflow.
    Health(Box<HealthView>),
    /// Typed prompt-runner management workflow.
    Runners(Box<RunnerManagerView>),
    /// Typed entry-settings workflow.
    Settings(Box<SettingsView>),
    /// Generic editable form.
    Form(FormView),
    /// Generic read-only report.
    Report(ReportView),
}

/// The non-modal workflow that remains active behind any overlay.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct WorkflowState {
    active: Screen,
    history: Vec<Screen>,
}

impl WorkflowState {
    /// Return the active workflow screen.
    #[must_use]
    pub const fn active(&self) -> &Screen {
        &self.active
    }

    fn present(&mut self, screen: Screen) {
        self.history
            .push(std::mem::replace(&mut self.active, screen));
    }

    fn back(&mut self) {
        self.active = self.history.pop().unwrap_or(Screen::Library);
    }

    fn previous(&self) -> Option<&Screen> {
        self.history.last()
    }

    fn replace_from_back(&mut self, screen: Screen) {
        let _ = self.history.pop();
        self.active = screen;
    }

    fn return_to_library(&mut self) {
        self.active = Screen::Library;
        self.history.clear();
    }
}

/// Workflow that owns one standalone prompt-runner editor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerEditorOwner {
    /// A typed launch form receives the new runner.
    Run {
        /// Stable entry selector used to reject a stale host response.
        selector: String,
    },
    /// An add-time prompt review receives the new runner.
    Add,
    /// The entry-settings screen receives the new runner.
    Settings {
        /// Stable entry selector used to reject a stale host response.
        selector: String,
    },
}

/// Host routing for one validated prompt-runner save.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerSaveOwner {
    /// The complete prompt-runner manager owns the mutation.
    Manager,
    /// One standalone editor modal owns the mutation.
    Editor(RunnerEditorOwner),
}

/// One modal overlay. Its owner workflow remains explicit and serializable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModalState {
    /// The complete command reminder.
    Help,
    /// Confirmation for removal of one selected entry.
    ConfirmRemove {
        /// Stable entry selector.
        selector: String,
        /// User-visible entry name.
        name: String,
        /// Whether an existing original file remains outside the removal transaction.
        original_file_preserved: bool,
    },
    /// Ask whether one dirty typed workflow can be discarded.
    ConfirmDiscardChanges,
    /// Name a parameter snapshot without leaving the launch form.
    RunPresetName {
        /// Mature input value.
        value: String,
        /// Existing names used for the overwrite notice.
        existing: BTreeSet<String>,
    },
    /// Choose a discoverable run-time value for one text field.
    RunTokenMenu {
        /// Target launch-field index.
        field: usize,
        /// Typed choices in display order.
        options: Vec<RunTokenOption>,
    },
    /// Filter or type one environment-variable name.
    RunEnvironmentPicker {
        /// Target launch-field index.
        field: usize,
        /// Available names captured with the launch token context.
        names: Vec<String>,
        /// Mature input value used by every frontend.
        query: String,
        /// Fuzzy-ranked names after applying the query.
        visible: Vec<String>,
    },
    /// Browse from the launch form's deterministic path roots.
    RunFilePicker {
        /// Target launch-field index.
        field: usize,
        /// Workdir and invocation roots supplied by the host adapter.
        context: RunPathContext,
        /// Field-shaped insertion grammar.
        mode: RunPathInsertMode,
    },
    /// Reusable typed prompt-runner editor above its owning workflow.
    RunnerEditor {
        /// Workflow that receives a successful new runner.
        owner: RunnerEditorOwner,
        /// Serializable editor values and validation state.
        view: Box<RunnerEditorView>,
        /// Status published if this editor was the only route to a runnable prompt.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancel_status: Option<String>,
    },
}

/// How resize logic must treat the detail pane.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailPaneMode {
    /// Let the frontend select the responsive default.
    #[default]
    Automatic,
    /// Keep the pane visible across resize events.
    PinnedOpen,
    /// Keep the pane hidden across resize events.
    PinnedClosed,
}

/// A user intent independent of terminal, webview, or native-window event types.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Select the preceding visible entry.
    Previous,
    /// Select the next visible entry.
    Next,
    /// Move up by a viewport-sized approximation.
    PagePrevious,
    /// Move down by a viewport-sized approximation.
    PageNext,
    /// Select the first visible entry.
    Home,
    /// Select the final visible entry.
    End,
    /// Select a row by its index in the filtered projection.
    SelectVisible(usize),
    /// Enter search-editing mode.
    BeginSearch,
    /// Leave search-editing mode while keeping the filter.
    FinishSearch,
    /// Append one character to the filter.
    Input(char),
    /// Delete one character from the filter.
    Backspace,
    /// Insert pasted text into the active search field.
    Paste(String),
    /// Replace the complete search value after a mature widget edit.
    SetSearchQuery(String),
    /// Clear the filter.
    ClearSearch,
    /// Replace every library-derived projection after a refresh.
    Replace {
        /// Current entry summaries and diagnostics.
        scan: LibraryScan,
        /// Entry identities that have a recorded launch to repeat.
        rerunnable: Vec<Slug>,
    },
    /// Replace the complete Library projection after one authoritative refresh.
    ReplaceSurface {
        /// List rows and all host-projected detail facts from the same refresh.
        surface: LibrarySurface,
        /// Entry identities that have a recorded launch to repeat.
        rerunnable: Vec<Slug>,
    },
    /// Replace the entry identities that have a recorded launch to repeat.
    ReplaceRerunnable(Vec<Slug>),
    /// Ask the host adapter to refresh application data.
    Reload,
    /// Repeat the selected entry with its last launch values.
    Rerun,
    /// Request a launch form for the selected entry.
    OpenRun,
    /// Request the add-entry form.
    OpenAdd,
    /// Request settings for the selected entry.
    OpenSettings,
    /// Request application preferences.
    OpenPreferences,
    /// Request the health report.
    OpenHealth,
    /// Request the prompt runner manager.
    OpenRunners,
    /// Request presets for the selected entry.
    OpenPresets,
    /// Request a rename form for the selected entry.
    OpenRename,
    /// Ask the host to open the selected entry in an editor.
    Edit,
    /// Show a remove confirmation for the selected entry.
    AskRemove,
    /// Show the command help overlay.
    OpenHelp,
    /// Toggle and pin the detail pane from its current rendered visibility.
    ToggleDetail {
        /// Whether the pane is visible when the user toggles it.
        currently_visible: bool,
    },
    /// Move focus to the next form field.
    FocusNext,
    /// Move focus to the preceding form field.
    FocusPrevious,
    /// Focus one form field by its index.
    FocusField(usize),
    /// Replace the value of one text control after a widget edit.
    SetFieldValue {
        /// Field index in the active typed launch form.
        field: usize,
        /// Complete value after the edit.
        value: String,
    },
    /// Apply a glob count only if the field still has the requested value.
    SetRunGlobCount {
        /// Field index in the active typed launch form.
        field: usize,
        /// Raw value that produced the host request.
        value: String,
        /// Total matches returned by the filesystem port.
        count: usize,
    },
    /// Open the save-preset name dialog from a typed launch form.
    OpenRunPresetSave,
    /// Open the run-time token menu for the focused launch field.
    OpenRunTokenMenu,
    /// Open the run-time token menu for one named launch-field index.
    OpenRunTokenMenuFor(usize),
    /// Chain from the token menu into the environment-variable picker.
    OpenRunEnvironmentPicker(usize),
    /// Replace the complete environment-picker query after a mature widget edit.
    SetRunEnvironmentQuery(String),
    /// Chain from the token menu into the filesystem picker.
    OpenRunFilePicker(usize),
    /// Open the filesystem picker for the focused launch field.
    OpenFocusedRunFilePicker,
    /// Apply a cursor-aware widget result and close the active run-value modal.
    SetRunFieldValueAndCloseModal {
        /// Target launch-field index.
        field: usize,
        /// Complete value after insertion or path replacement.
        value: String,
    },
    /// Apply a filesystem-picker result with the field's typed insertion grammar.
    SetRunPickedPathAndCloseModal {
        /// Target launch-field index.
        field: usize,
        /// Relative or absolute path text returned by the shared picker.
        path: String,
    },
    /// Restore the focused launch field to its declared default.
    ResetFocusedRunField,
    /// Open the shared runner editor for the active launch form.
    OpenRunRunnerEditor,
    /// Delegate one semantic action to the typed add reducer.
    Add(AddAction),
    /// Open the shared runner editor for an add-time prompt review.
    OpenAddRunnerEditor,
    /// Delegate one semantic action to the typed Health reducer.
    Health(HealthAction),
    /// Delegate one semantic action to the complete runner manager.
    Runners(RunnerManagerAction),
    /// Delegate one semantic action to the standalone runner editor modal.
    RunnerEditor(RunnerEditorAction),
    /// Apply a successful standalone runner save to its exact workflow owner.
    RunnerEditorSaved {
        /// Owner echoed from the save request.
        owner: RunnerEditorOwner,
        /// Stable saved runner name.
        name: String,
        /// Localized completion status.
        message: String,
    },
    /// Keep the standalone editor and all input after a host mutation refusal.
    RunnerEditorSaveFailed {
        /// Owner echoed from the save request.
        owner: RunnerEditorOwner,
        /// Localized refusal detail.
        message: String,
    },
    /// Close runner management into an authoritative rebuilt Preferences screen.
    RunnerManagerClosed {
        /// Fresh configuration projection including runner names and pin counts.
        preferences: Box<PreferencesView>,
    },
    /// Delegate one semantic action to the typed Preferences reducer.
    Preferences(PreferencesAction),
    /// Typed entry-settings edit.
    Settings(SettingsAction),
    /// Finish an atomic Preferences save and publish the newly effective locale.
    PreferencesSaved {
        /// Canonical negotiated language tag for immediate frontend switching.
        locale: String,
        /// Localized completion status.
        message: String,
    },
    /// Close a discard guard without changing its owner workflow.
    KeepEditing,
    /// Discard the active workflow and return to the library.
    DiscardChanges,
    /// Replace the complete input value in the active text-input modal.
    SetModalInput(String),
    /// Apply the host's saved preset map without rebuilding the launch screen.
    RunPresetSaved {
        /// Newly saved preset name.
        name: String,
        /// Complete refreshed preset map.
        presets: BTreeMap<String, BTreeMap<String, String>>,
        /// Localized host completion status.
        message: String,
    },
    /// Toggle one checkbox control.
    ToggleField(usize),
    /// Select one valid value in a choice control.
    SelectFieldOption {
        /// Field index in the active typed launch form.
        field: usize,
        /// Stable option value.
        value: String,
    },
    /// Restore one launch field to its declared default.
    ResetRunField(usize),
    /// Submit the active form or confirmation.
    Submit,
    /// Return to the library.
    Back,
    /// Present a screen built by the host adapter.
    Present(Screen),
    /// Present a prompt launch form and require one configured runner before it can run.
    PromptRunnerRequired {
        /// Existing launch form that receives a successfully created runner.
        form: Box<RunFormView>,
        /// Localized status published if the runner editor is cancelled.
        cancel_status: String,
    },
    /// Finish an add transaction and select the created entry after its authoritative reload.
    AddCompleted {
        /// New library surface after the atomic create, detail facts included.
        surface: LibrarySurface,
        /// Refreshed entry identities that have a recorded launch to repeat.
        rerunnable: Vec<Slug>,
        /// Created entry identity to select when it survives the active filter.
        slug: Slug,
        /// Localized completion status.
        message: String,
    },
    /// Finish an add workflow without creating an entry.
    AddCancelled,
    /// Finish a host operation and optionally replace the whole library surface.
    ///
    /// The surface, never the scan alone: the detail pane's facts are projected beside the entry
    /// list, and an entry that arrives without them draws a pane with its name and nothing else.
    Complete {
        /// New library state after a mutation.
        surface: Option<LibrarySurface>,
        /// Refreshed entry identities that have a recorded launch to repeat.
        rerunnable: Option<Vec<Slug>>,
        /// User-visible completion message.
        message: String,
    },
    /// Set a host-generated status line.
    SetStatus(String),
    /// Clear the status line.
    ClearStatus,
    /// Ask the host adapter to exit.
    Quit,
}

/// Side effect requested by the pure reducer.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// No host-side work.
    #[default]
    None,
    /// Reload application data through the repository port.
    Reload,
    /// Close the frontend.
    Quit,
    /// Repeat the last successful launch request for one entry.
    Rerun {
        /// Stable entry selector.
        selector: String,
    },
    /// Ask the host to build one screen.
    Open {
        /// Requested operation.
        request: HostRequest,
        /// Selected entry, when the operation needs one.
        selector: Option<String>,
    },
    /// Submit a generic form to the host.
    Submit {
        /// Operation that owns the form.
        purpose: FormPurpose,
        /// Entry selector, when present.
        selector: Option<String>,
        /// Typed field values indexed by stable key.
        values: SubmittedValues,
    },
    /// Count glob matches without putting filesystem rules in a frontend reducer.
    CountRunGlob {
        /// Stable entry selector.
        selector: String,
        /// Field index whose feedback receives the result.
        field: usize,
        /// Raw value used to reject a stale response.
        value: String,
        /// Typed split request for the host port.
        request: skit_application::form_feedback::GlobCountRequest,
    },
    /// Persist one named nonsecret parameter snapshot.
    SaveRunPreset {
        /// Stable entry selector.
        selector: String,
        /// Trimmed preset name.
        name: String,
        /// Exact visible snapshot keyed by parameter name.
        values: BTreeMap<String, String>,
        /// Secret names that the adapter must never persist.
        secret_names: BTreeSet<String>,
    },
    /// Ordered host work requested by the typed add reducer.
    Add(Vec<AddEffect>),
    /// Rebuild the registry and collect a fresh typed Health snapshot.
    HealthRebuild,
    /// Persist one validated prompt runner for its exact workflow owner.
    SaveRunner {
        /// Compare-and-swap save request.
        request: RunnerSaveRequest,
        /// Owner that must receive the host response.
        owner: RunnerSaveOwner,
    },
    /// Remove one stable runner key or one malformed raw row.
    RemoveRunner(RunnerRemoveRequest),
    /// Rebuild Preferences after its runner manager closes.
    RefreshPreferencesAfterRunners,
    /// Host work requested by the typed Preferences reducer.
    Preferences(PreferencesEffect),
    /// Open the selected entry in the configured editor.
    Edit {
        /// Stable entry selector.
        selector: String,
    },
    /// Remove one entry after explicit confirmation.
    Remove {
        /// Stable entry selector.
        selector: String,
    },
}

/// Serializable state rendered by Ratatui today and available to a future Tauri shell.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct LibraryState {
    entries: Vec<EntrySummary>,
    diagnostics: Vec<Diagnostic>,
    #[serde(default)]
    details: BTreeMap<Slug, LibraryEntryDetail>,
    query: String,
    input_mode: InputMode,
    selected: Option<usize>,
    visible: Vec<usize>,
    rerunnable: BTreeSet<Slug>,
    status: Option<String>,
    workflow: WorkflowState,
    modal: Option<ModalState>,
    detail_pane: DetailPaneMode,
}

impl LibraryState {
    /// Build state from one application-layer scan.
    #[must_use]
    pub fn from_scan(scan: LibraryScan) -> Self {
        let mut state = Self {
            entries: scan.entries,
            diagnostics: scan.diagnostics,
            ..Self::default()
        };
        state.recompute_visible(None);
        state
    }

    /// Build state from one complete host-projected Library refresh.
    #[must_use]
    pub fn from_surface(scan: LibraryScan, details: BTreeMap<Slug, LibraryEntryDetail>) -> Self {
        Self::from_library_surface(LibrarySurface { scan, details })
    }

    /// Build state from one complete host-projected Library refresh value.
    #[must_use]
    pub fn from_library_surface(surface: LibrarySurface) -> Self {
        let mut state = Self::default();
        state.replace_surface(surface);
        state.recompute_visible(None);
        state
    }

    /// Apply one action and return only the host effect it requests.
    pub fn update(&mut self, action: Action) -> Effect {
        match action {
            Action::Previous => self.move_selection(-1),
            Action::Next => self.move_selection(1),
            Action::PagePrevious => self.move_selection(-10),
            Action::PageNext => self.move_selection(10),
            Action::Home => self.select_boundary(false),
            Action::End => self.select_boundary(true),
            Action::SelectVisible(index) => {
                if index < self.visible.len() {
                    self.selected = Some(index);
                }
            }
            Action::BeginSearch => self.input_mode = InputMode::Search,
            Action::FinishSearch => self.input_mode = InputMode::Browse,
            Action::Input(character) => match self.input_mode {
                InputMode::Search => {
                    let selected = self.selected_slug().cloned();
                    self.query.push(character);
                    self.recompute_visible(selected.as_ref());
                }
                InputMode::Form => {
                    self.append_form_input(&character.to_string());
                }
                InputMode::Browse => {}
            },
            Action::Paste(value) => match self.input_mode {
                InputMode::Search => {
                    let selected = self.selected_slug().cloned();
                    self.query.push_str(&value);
                    self.recompute_visible(selected.as_ref());
                }
                InputMode::Form => self.append_form_input(&value),
                InputMode::Browse => {}
            },
            Action::SetSearchQuery(value) => {
                if self.input_mode == InputMode::Search {
                    let selected = self.selected_slug().cloned();
                    self.query = value;
                    self.recompute_visible(selected.as_ref());
                }
            }
            Action::Backspace => match self.input_mode {
                InputMode::Search => {
                    let selected = self.selected_slug().cloned();
                    self.query.pop();
                    self.recompute_visible(selected.as_ref());
                }
                InputMode::Form => {
                    self.backspace_form_input();
                }
                InputMode::Browse => {}
            },
            Action::ClearSearch => {
                let selected = self.selected_slug().cloned();
                self.query.clear();
                self.recompute_visible(selected.as_ref());
            }
            Action::Replace { scan, rerunnable } => {
                let selected = self.selected_slug().cloned();
                self.entries = scan.entries;
                self.diagnostics = scan.diagnostics;
                self.details.clear();
                self.rerunnable = rerunnable.into_iter().collect();
                self.recompute_visible(selected.as_ref());
            }
            Action::ReplaceSurface {
                surface,
                rerunnable,
            } => {
                let selected = self.selected_slug().cloned();
                self.replace_surface(surface);
                self.rerunnable = rerunnable.into_iter().collect();
                self.recompute_visible(selected.as_ref());
            }
            Action::ReplaceRerunnable(selectors) => {
                self.rerunnable = selectors.into_iter().collect();
            }
            Action::Reload => return Effect::Reload,
            Action::Rerun => {
                return self
                    .selected_selector()
                    .map_or(Effect::None, |selector| Effect::Rerun { selector });
            }
            Action::OpenRun => {
                self.input_mode = InputMode::Browse;
                return self.open_selected(HostRequest::Run);
            }
            Action::OpenAdd => return self.open(HostRequest::Add, None),
            Action::OpenSettings => return self.open_selected(HostRequest::Settings),
            Action::OpenPreferences => return self.open(HostRequest::Preferences, None),
            Action::OpenHealth => return self.open(HostRequest::Health, None),
            Action::OpenRunners => return self.open(HostRequest::Runners, None),
            Action::OpenPresets => return self.open_selected(HostRequest::Presets),
            Action::OpenRename => return self.open_selected(HostRequest::Rename),
            Action::Edit => {
                return self
                    .selected_selector()
                    .map_or(Effect::None, |selector| Effect::Edit { selector });
            }
            Action::AskRemove => {
                if let Some(entry) = self.selected() {
                    self.modal = Some(ModalState::ConfirmRemove {
                        selector: entry.slug.as_str().to_owned(),
                        name: entry.name.clone(),
                        original_file_preserved: self
                            .details
                            .get(&entry.slug)
                            .is_some_and(|detail| detail.original_file_preserved),
                    });
                    self.input_mode = InputMode::Browse;
                }
            }
            Action::OpenHelp => {
                self.modal = Some(ModalState::Help);
                self.input_mode = InputMode::Browse;
            }
            Action::ToggleDetail { currently_visible } => {
                self.detail_pane = if currently_visible {
                    DetailPaneMode::PinnedClosed
                } else {
                    DetailPaneMode::PinnedOpen
                };
            }
            Action::FocusNext => self.move_form_focus(1),
            Action::FocusPrevious => self.move_form_focus(-1),
            Action::FocusField(index) => self.focus_form_field(index),
            Action::SetFieldValue { field, value } => match &mut self.workflow.active {
                Screen::Run(form) => {
                    let raw = value.clone();
                    if let Some(request) = form.set_field_value(field, value) {
                        return Effect::CountRunGlob {
                            selector: form.selector.clone(),
                            field,
                            value: raw,
                            request,
                        };
                    }
                }
                Screen::Form(form) => {
                    if let Some(field) = form.fields.get_mut(field) {
                        field.value = value;
                    }
                }
                Screen::Library
                | Screen::Preferences(_)
                | Screen::Add(_)
                | Screen::Health(_)
                | Screen::Runners(_)
                | Screen::Settings(_)
                | Screen::Report(_) => return Effect::None,
            },
            Action::SetRunGlobCount {
                field,
                value,
                count,
            } => {
                if let Screen::Run(form) = &mut self.workflow.active {
                    form.set_glob_count(field, &value, count);
                }
            }
            Action::OpenRunTokenMenu => {
                if let Screen::Run(form) = &mut self.workflow.active
                    && let Some(options) = form.token_options(form.focused())
                {
                    self.modal = Some(ModalState::RunTokenMenu {
                        field: form.focused(),
                        options,
                    });
                }
            }
            Action::OpenRunTokenMenuFor(field) => {
                if let Screen::Run(form) = &mut self.workflow.active
                    && let Some(options) = form.token_options(field)
                {
                    form.focused = field;
                    self.modal = Some(ModalState::RunTokenMenu { field, options });
                }
            }
            Action::OpenRunEnvironmentPicker(field) => {
                if matches!(self.modal.as_ref(), Some(ModalState::RunTokenMenu { field: target, .. }) if *target == field)
                    && let Screen::Run(form) = &self.workflow.active
                    && let Some(context) = form.context()
                {
                    self.modal = Some(ModalState::RunEnvironmentPicker {
                        field,
                        names: context.tokens.env.keys().cloned().collect::<Vec<_>>(),
                        query: String::new(),
                        visible: context.tokens.env.keys().cloned().collect(),
                    });
                }
            }
            Action::SetRunEnvironmentQuery(query) => {
                if let Some(ModalState::RunEnvironmentPicker {
                    names,
                    query: current,
                    visible,
                    ..
                }) = &mut self.modal
                {
                    *visible = run::filter_environment_names(names, &query);
                    *current = query;
                }
            }
            Action::OpenRunFilePicker(field) => {
                let owns_request = self.modal.is_none()
                    || matches!(self.modal.as_ref(), Some(ModalState::RunTokenMenu { field: target, .. }) if *target == field);
                if owns_request
                    && let Screen::Run(form) = &mut self.workflow.active
                    && let Some((context, mode)) = form.path_picker_contract(field)
                {
                    form.focused = field;
                    self.modal = Some(ModalState::RunFilePicker {
                        field,
                        context,
                        mode,
                    });
                }
            }
            Action::OpenFocusedRunFilePicker => {
                if self.modal.is_none()
                    && let Screen::Run(form) = &mut self.workflow.active
                    && let Some((context, mode)) = form.path_picker_contract(form.focused())
                {
                    self.modal = Some(ModalState::RunFilePicker {
                        field: form.focused(),
                        context,
                        mode,
                    });
                }
            }
            Action::SetRunFieldValueAndCloseModal { field, value } => {
                if matches!(
                    self.modal.as_ref(),
                    Some(
                        ModalState::RunTokenMenu { field: target, .. }
                            | ModalState::RunEnvironmentPicker { field: target, .. }
                    ) if *target == field
                ) {
                    self.modal = None;
                    if let Screen::Run(form) = &mut self.workflow.active {
                        let raw = value.clone();
                        if let Some(request) = form.set_field_value(field, value) {
                            return Effect::CountRunGlob {
                                selector: form.selector.clone(),
                                field,
                                value: raw,
                                request,
                            };
                        }
                    }
                }
            }
            Action::SetRunPickedPathAndCloseModal { field, path } => {
                let mode = match self.modal.as_ref() {
                    Some(ModalState::RunFilePicker {
                        field: target,
                        mode,
                        ..
                    }) if *target == field => Some(*mode),
                    _ => None,
                };
                if let Some(mode) = mode
                    && let Screen::Run(form) = &mut self.workflow.active
                    && let Ok(request) = form.insert_picked_path(field, &path, mode)
                {
                    self.modal = None;
                    if let Some(request) = request {
                        return Effect::CountRunGlob {
                            selector: form.selector.clone(),
                            field,
                            value: form.fields[field].control.value(),
                            request,
                        };
                    }
                }
            }
            Action::ResetFocusedRunField => {
                if let Screen::Run(form) = &mut self.workflow.active {
                    form.reset_field(form.focused());
                }
            }
            Action::OpenRunRunnerEditor => {
                if let Screen::Run(form) = &self.workflow.active
                    && form.has_runner_picker()
                {
                    self.modal = Some(ModalState::RunnerEditor {
                        owner: RunnerEditorOwner::Run {
                            selector: form.selector().to_owned(),
                        },
                        view: Box::default(),
                        cancel_status: None,
                    });
                }
            }
            Action::Add(action) => {
                if let Screen::Add(view) = &mut self.workflow.active {
                    let effects = view.reduce(action);
                    if !effects.is_empty() {
                        return Effect::Add(effects);
                    }
                }
            }
            Action::OpenAddRunnerEditor => {
                if let Screen::Add(view) = &self.workflow.active
                    && view
                        .review()
                        .is_some_and(|review| review.lane() == ReviewLane::Prompt)
                {
                    self.modal = Some(ModalState::RunnerEditor {
                        owner: RunnerEditorOwner::Add,
                        view: Box::default(),
                        cancel_status: None,
                    });
                }
            }
            Action::Health(action) => {
                let effect = match &mut self.workflow.active {
                    Screen::Health(view) => view.reduce(action),
                    Screen::Library
                    | Screen::Run(_)
                    | Screen::Preferences(_)
                    | Screen::Add(_)
                    | Screen::Runners(_)
                    | Screen::Settings(_)
                    | Screen::Form(_)
                    | Screen::Report(_) => HealthEffect::None,
                };
                match effect {
                    HealthEffect::None => {}
                    HealthEffect::JumpToEntry(selector) => {
                        if let Ok(slug) = Slug::parse(&selector) {
                            self.recompute_visible(Some(&slug));
                            self.workflow.return_to_library();
                            self.modal = None;
                            self.input_mode = InputMode::Browse;
                        }
                    }
                    HealthEffect::Rebuild => return Effect::HealthRebuild,
                    HealthEffect::Close => {
                        self.workflow.back();
                        self.modal = None;
                        self.input_mode = InputMode::Browse;
                    }
                }
            }
            Action::Runners(action) => {
                let effect = match &mut self.workflow.active {
                    Screen::Runners(view) => view.reduce(action),
                    Screen::Library
                    | Screen::Run(_)
                    | Screen::Preferences(_)
                    | Screen::Add(_)
                    | Screen::Health(_)
                    | Screen::Form(_)
                    | Screen::Settings(_)
                    | Screen::Report(_) => RunnerManagerEffect::None,
                };
                match effect {
                    RunnerManagerEffect::None => {}
                    RunnerManagerEffect::Save(request) => {
                        return Effect::SaveRunner {
                            request,
                            owner: RunnerSaveOwner::Manager,
                        };
                    }
                    RunnerManagerEffect::Remove(request) => {
                        return Effect::RemoveRunner(request);
                    }
                    RunnerManagerEffect::Close => {
                        if matches!(self.workflow.previous(), Some(Screen::Preferences(_))) {
                            return Effect::RefreshPreferencesAfterRunners;
                        }
                        self.workflow.back();
                        self.modal = None;
                        self.input_mode = InputMode::Browse;
                    }
                }
            }
            Action::RunnerEditor(action) => {
                let Some(ModalState::RunnerEditor {
                    owner,
                    view,
                    cancel_status,
                }) = &mut self.modal
                else {
                    return Effect::None;
                };
                match view.reduce(action) {
                    RunnerEditorEffect::None => {}
                    RunnerEditorEffect::Save(request) => {
                        return Effect::SaveRunner {
                            request,
                            owner: RunnerSaveOwner::Editor(owner.clone()),
                        };
                    }
                    RunnerEditorEffect::Cancel => {
                        let cancel_status = cancel_status.clone();
                        self.modal = None;
                        if let Some(message) = cancel_status {
                            self.workflow.return_to_library();
                            self.input_mode = InputMode::Browse;
                            self.status = Some(message);
                        }
                    }
                }
            }
            Action::RunnerEditorSaved {
                owner,
                name,
                message,
            } => {
                let owned = matches!(
                    self.modal.as_ref(),
                    Some(ModalState::RunnerEditor { owner: current, .. }) if current == &owner
                );
                if !owned {
                    return Effect::None;
                }
                match &owner {
                    RunnerEditorOwner::Run { selector } => {
                        if let Screen::Run(form) = &mut self.workflow.active {
                            form.add_and_select_runner(selector, name);
                        }
                    }
                    RunnerEditorOwner::Add => {
                        if let Screen::Add(view) = &mut self.workflow.active {
                            let _ = view.reduce(AddAction::PromptRunnerAdded(name));
                        }
                    }
                    RunnerEditorOwner::Settings { selector } => {
                        if let Screen::Settings(view) = &mut self.workflow.active {
                            view.add_and_select_runner(selector, name);
                        }
                    }
                }
                self.modal = None;
                self.status = Some(message);
            }
            Action::RunnerEditorSaveFailed { owner, message } => {
                if let Some(ModalState::RunnerEditor {
                    owner: current,
                    view,
                    ..
                }) = &mut self.modal
                    && current == &owner
                {
                    let _ = view.reduce(RunnerEditorAction::MutationFailed(message));
                }
            }
            Action::RunnerManagerClosed { preferences } => {
                if matches!(self.workflow.active(), Screen::Runners(_))
                    && matches!(self.workflow.previous(), Some(Screen::Preferences(_)))
                {
                    self.workflow
                        .replace_from_back(Screen::Preferences(preferences));
                    self.modal = None;
                    self.input_mode = InputMode::Browse;
                }
            }
            Action::Settings(action) => {
                let Screen::Settings(view) = &mut self.workflow.active else {
                    return Effect::None;
                };
                match view.update(action) {
                    SettingsEffect::None => {}
                    SettingsEffect::Close => {
                        self.workflow.back();
                        self.modal = None;
                        self.input_mode = InputMode::Browse;
                    }
                    // Leaving with unsaved work asks first (`src/skit/tui_settings.py:43-46`).
                    SettingsEffect::ConfirmDiscard => {
                        self.modal = Some(ModalState::ConfirmDiscardChanges);
                    }
                    // Version 0.4 explains the refusal and writes nothing
                    // (`src/skit/tui_settings.py:517-523`, `:939-941`).
                    SettingsEffect::Refused(error) => {
                        self.status = Some(error.message().to_owned());
                    }
                    SettingsEffect::Save => {
                        return Effect::Submit {
                            purpose: FormPurpose::Settings,
                            selector: Some(view.selector.clone()),
                            values: view.submitted_values(),
                        };
                    }
                    // The runner editor is a modal this reducer owns, exactly as the launch form
                    // and the add review own theirs. A host round trip would put a screen where a
                    // modal belongs and lose the settings work underneath it.
                    SettingsEffect::NewRunner => {
                        let selector = view.selector.clone();
                        self.modal = Some(ModalState::RunnerEditor {
                            owner: RunnerEditorOwner::Settings { selector },
                            view: Box::default(),
                            cancel_status: None,
                        });
                    }
                }
            }
            Action::Preferences(action) => {
                if let Screen::Preferences(view) = &mut self.workflow.active {
                    let installed_message = match &action {
                        PreferencesAction::AgentSkillInstalled { message } => Some(message.clone()),
                        _ => None,
                    };
                    let effect = view.update(action);
                    if let Some(message) = installed_message {
                        self.status = Some(message);
                    }
                    match effect {
                        PreferencesEffect::None => {}
                        PreferencesEffect::Close => {
                            self.workflow.back();
                            self.modal = None;
                            self.input_mode = InputMode::Browse;
                        }
                        PreferencesEffect::ConfirmDiscard => {
                            self.modal = Some(ModalState::ConfirmDiscardChanges);
                        }
                        effect => return Effect::Preferences(effect),
                    }
                }
            }
            Action::PreferencesSaved { locale: _, message } => {
                self.status = Some(message);
                self.workflow.return_to_library();
                self.modal = None;
                self.input_mode = InputMode::Browse;
            }
            Action::KeepEditing => {
                if matches!(self.modal, Some(ModalState::ConfirmDiscardChanges)) {
                    self.modal = None;
                }
            }
            Action::DiscardChanges => {
                if matches!(self.modal, Some(ModalState::ConfirmDiscardChanges)) {
                    self.modal = None;
                    self.workflow.back();
                    self.input_mode = InputMode::Browse;
                }
            }
            Action::OpenRunPresetSave => {
                if let Screen::Run(form) = &self.workflow.active
                    && form.has_parameters()
                {
                    self.modal = Some(ModalState::RunPresetName {
                        value: String::new(),
                        existing: form.preset_names().map(str::to_owned).collect(),
                    });
                }
            }
            Action::SetModalInput(value) => {
                if let Some(ModalState::RunPresetName { value: current, .. }) = &mut self.modal {
                    *current = value;
                }
            }
            Action::RunPresetSaved {
                name,
                presets,
                message,
            } => {
                if let Screen::Run(form) = &mut self.workflow.active {
                    form.refresh_presets(name, presets);
                    self.status = Some(message);
                }
                self.modal = None;
            }
            Action::ToggleField(index) => {
                if let Screen::Run(form) = &mut self.workflow.active
                    && index < form.fields.len()
                {
                    form.focused = index;
                    form.fields[index].control.toggle();
                    form.fields[index].validation_error = None;
                }
            }
            Action::SelectFieldOption { field, value } => {
                if let Screen::Run(form) = &mut self.workflow.active {
                    if field < form.fields.len() {
                        form.focused = field;
                    }
                    form.select_option(field, &value);
                }
            }
            Action::ResetRunField(index) => {
                if let Screen::Run(form) = &mut self.workflow.active {
                    form.reset_field(index);
                }
            }
            Action::Submit => return self.submit(),
            Action::Back => {
                if self.modal.take().is_none() {
                    self.workflow.back();
                    self.input_mode = InputMode::Browse;
                }
            }
            Action::Present(screen) => {
                self.input_mode = if matches!(screen, Screen::Run(_) | Screen::Form(_)) {
                    InputMode::Form
                } else {
                    InputMode::Browse
                };
                self.workflow.present(screen);
                self.modal = None;
            }
            Action::PromptRunnerRequired {
                form,
                cancel_status,
            } => {
                let selector = form.selector().to_owned();
                self.input_mode = InputMode::Form;
                self.workflow.present(Screen::Run(form));
                self.modal = Some(ModalState::RunnerEditor {
                    owner: RunnerEditorOwner::Run { selector },
                    view: Box::default(),
                    cancel_status: Some(cancel_status),
                });
            }
            Action::AddCompleted {
                surface,
                rerunnable,
                slug,
                message,
            } => {
                // The created entry needs its detail facts here or its pane opens empty — and this
                // is the one entry the user is looking at, because the add selects it.
                self.replace_surface(surface);
                self.rerunnable = rerunnable.into_iter().collect();
                self.recompute_visible(Some(&slug));
                self.status = Some(message);
                self.workflow.return_to_library();
                self.modal = None;
                self.input_mode = InputMode::Browse;
            }
            Action::AddCancelled => {
                self.workflow.return_to_library();
                self.modal = None;
                self.input_mode = InputMode::Browse;
            }
            Action::Complete {
                surface,
                rerunnable,
                message,
            } => {
                // The whole surface, not the entry list alone. The detail pane's facts live beside
                // the scan, and an entry that arrives without them shows a pane with its name and
                // nothing else — which is what a freshly added entry did, because it had no row in
                // the map and nothing put one there.
                if let Some(surface) = surface {
                    let selected = self.selected_slug().cloned();
                    self.replace_surface(surface);
                    self.recompute_visible(selected.as_ref());
                }
                if let Some(rerunnable) = rerunnable {
                    self.rerunnable = rerunnable.into_iter().collect();
                }
                self.status = Some(message);
                self.workflow.return_to_library();
                self.modal = None;
                self.input_mode = InputMode::Browse;
            }
            Action::SetStatus(message) => self.status = Some(message),
            Action::ClearStatus => self.status = None,
            Action::Quit => return Effect::Quit,
        }
        Effect::None
    }

    /// Return the current input grammar.
    #[must_use]
    pub const fn input_mode(&self) -> InputMode {
        self.input_mode
    }

    /// Return the active screen.
    #[must_use]
    pub const fn screen(&self) -> &Screen {
        self.workflow.active()
    }

    /// Return the current non-modal workflow state.
    #[must_use]
    pub const fn workflow(&self) -> &WorkflowState {
        &self.workflow
    }

    /// Return the active overlay, when present.
    #[must_use]
    pub const fn modal(&self) -> Option<&ModalState> {
        self.modal.as_ref()
    }

    /// Return the responsive detail-pane policy.
    #[must_use]
    pub const fn detail_pane_mode(&self) -> DetailPaneMode {
        self.detail_pane
    }

    /// Return the active command surface.
    #[must_use]
    pub const fn command_context(&self) -> CommandContext {
        match self.modal {
            Some(ModalState::Help) => CommandContext::Help,
            Some(ModalState::ConfirmRemove { .. }) => CommandContext::ConfirmRemove,
            Some(ModalState::ConfirmDiscardChanges) => CommandContext::ConfirmDiscard,
            Some(ModalState::RunPresetName { .. }) => CommandContext::RunPresetName,
            Some(
                ModalState::RunTokenMenu { .. }
                | ModalState::RunEnvironmentPicker { .. }
                | ModalState::RunFilePicker { .. },
            ) => CommandContext::RunTokenMenu,
            Some(ModalState::RunnerEditor { .. }) => CommandContext::RunnerEditor,
            None => match self.workflow.active {
                Screen::Library if matches!(self.input_mode, InputMode::Search) => {
                    CommandContext::LibrarySearch
                }
                Screen::Library => CommandContext::LibraryBrowse,
                Screen::Run(_) => CommandContext::RunForm,
                Screen::Preferences(_) => CommandContext::Preferences,
                Screen::Settings(_) => CommandContext::Settings,
                Screen::Add(_) => CommandContext::Add,
                Screen::Health(_) => CommandContext::Health,
                Screen::Runners(_) => CommandContext::Runners,
                Screen::Form(_) => CommandContext::Form,
                Screen::Report(_) => CommandContext::Report,
            },
        }
    }

    /// Return the settings screen when one is active.
    #[must_use]
    pub const fn settings_view(&self) -> Option<&SettingsView> {
        match &self.workflow.active {
            Screen::Settings(view) => Some(view),
            _ => None,
        }
    }

    /// Report whether the current state can execute one command truthfully.
    #[must_use]
    pub fn command_enabled(&self, command: UiCommand) -> bool {
        match command {
            UiCommand::Run
            | UiCommand::Settings
            | UiCommand::Edit
            | UiCommand::Rename
            | UiCommand::Remove => self.selected().is_some(),
            UiCommand::Rerun => self
                .selected_slug()
                .is_some_and(|slug| self.rerunnable.contains(slug)),
            UiCommand::InsertValue => true,
            UiCommand::BrowsePath => self.run_form().is_some_and(|form| {
                form.context()
                    .and_then(|context| context.path.as_ref())
                    .is_some()
                    && form
                        .fields()
                        .get(form.focused())
                        .is_some_and(RunField::browsable)
            }),
            UiCommand::ResetDefault => self
                .run_form()
                .is_some_and(RunFormView::has_resettable_fields),
            UiCommand::SavePreset => self.run_form().is_some_and(RunFormView::has_parameters),
            // The settings screen carries its own runner picker, so the chip belongs to whichever
            // screen is actually showing one.
            UiCommand::NewRunner => {
                self.run_form().is_some_and(RunFormView::has_runner_picker)
                    || self
                        .settings_view()
                        .is_some_and(|view| view.has_section(SettingsSectionId::Runner))
            }
            UiCommand::SaveSettings | UiCommand::CloseSettings => self.settings_view().is_some(),
            // The chip follows the control, not a second copy of the rule that built it.
            UiCommand::ResyncSettings => self
                .settings_view()
                .is_some_and(|view| view.field(RESYNC_KEY).is_some()),
            UiCommand::SavePreferences
            | UiCommand::ClosePreferences
            | UiCommand::ManageAgents
            | UiCommand::InstallAgentSkill => {
                matches!(self.workflow.active, Screen::Preferences(_))
            }
            UiCommand::Add
            | UiCommand::Presets
            | UiCommand::Preferences
            | UiCommand::Health
            | UiCommand::Runners
            | UiCommand::Search
            | UiCommand::LeaveSearch
            | UiCommand::ToggleDetail
            | UiCommand::Help
            | UiCommand::Reload
            | UiCommand::Quit
            | UiCommand::Previous
            | UiCommand::Next
            | UiCommand::PagePrevious
            | UiCommand::PageNext
            | UiCommand::Home
            | UiCommand::End
            | UiCommand::Backspace
            | UiCommand::ClearSearch
            | UiCommand::FocusNext
            | UiCommand::FocusPrevious
            | UiCommand::Submit
            | UiCommand::Back
            | UiCommand::CloseModal => true,
            UiCommand::DiscardChanges | UiCommand::KeepEditing => {
                matches!(self.modal, Some(ModalState::ConfirmDiscardChanges))
            }
        }
    }

    /// Return the active form, when present.
    #[must_use]
    pub fn form(&self) -> Option<&FormView> {
        match &self.workflow.active {
            Screen::Form(form) => Some(form),
            Screen::Library
            | Screen::Run(_)
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Report(_) => None,
        }
    }

    /// Return the active typed launch form, when present.
    #[must_use]
    pub fn run_form(&self) -> Option<&RunFormView> {
        match &self.workflow.active {
            Screen::Run(form) => Some(form),
            Screen::Library
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Form(_)
            | Screen::Report(_) => None,
        }
    }

    /// Return the active typed Preferences workflow, when present.
    #[must_use]
    pub fn preferences(&self) -> Option<&PreferencesView> {
        match &self.workflow.active {
            Screen::Preferences(view) => Some(view),
            Screen::Library
            | Screen::Run(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Form(_)
            | Screen::Report(_) => None,
        }
    }

    /// Return the active typed Add workflow, when present.
    #[must_use]
    pub fn add_workflow(&self) -> Option<&AddWorkflowState> {
        match &self.workflow.active {
            Screen::Add(view) => Some(view),
            Screen::Library
            | Screen::Run(_)
            | Screen::Preferences(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Form(_)
            | Screen::Report(_) => None,
        }
    }

    /// Return the active control index for any editable screen.
    #[must_use]
    pub const fn focused_form_field(&self) -> Option<usize> {
        match &self.workflow.active {
            Screen::Run(form) => Some(form.focused),
            Screen::Form(form) => Some(form.focused),
            Screen::Library
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Report(_) => None,
        }
    }

    /// Return the active filter text.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Return all scan diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Return the current status line.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Iterate over entries surviving the current filter.
    pub fn visible_entries(&self) -> impl ExactSizeIterator<Item = &EntrySummary> {
        self.visible.iter().map(|index| &self.entries[*index])
    }

    /// Return the host-projected detail facts for one entry.
    #[must_use]
    pub fn entry_detail(&self, slug: &Slug) -> Option<&LibraryEntryDetail> {
        self.details.get(slug)
    }

    /// Return the host-projected detail facts for the selected entry.
    #[must_use]
    pub fn selected_detail(&self) -> Option<&LibraryEntryDetail> {
        self.selected()
            .and_then(|entry| self.details.get(&entry.slug))
    }

    /// Return the number of entries before the current search filter.
    #[must_use]
    pub const fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Return the number of entries after the current search filter.
    #[must_use]
    pub const fn visible_entry_count(&self) -> usize {
        self.visible.len()
    }

    /// Return the selected visible-row index.
    #[must_use]
    pub const fn selected_visible_index(&self) -> Option<usize> {
        self.selected
    }

    /// Return the selected entry.
    #[must_use]
    pub fn selected(&self) -> Option<&EntrySummary> {
        self.selected
            .and_then(|visible_index| self.visible.get(visible_index))
            .and_then(|entry_index| self.entries.get(*entry_index))
    }

    fn selected_slug(&self) -> Option<&Slug> {
        self.selected().map(|entry| &entry.slug)
    }

    fn selected_selector(&self) -> Option<String> {
        self.selected_slug().map(|slug| slug.as_str().to_owned())
    }

    fn open_selected(&self, request: HostRequest) -> Effect {
        self.selected_selector()
            .map_or(Effect::None, |selector| self.open(request, Some(selector)))
    }

    fn open(&self, request: HostRequest, selector: Option<String>) -> Effect {
        Effect::Open { request, selector }
    }

    fn move_form_focus(&mut self, delta: isize) {
        match &mut self.workflow.active {
            Screen::Run(form) => {
                form.focused = moved_focus(form.focused, form.fields.len(), delta);
            }
            Screen::Form(form) => {
                form.focused = moved_focus(form.focused, form.fields.len(), delta);
            }
            // The settings cursor is keyed by field, not by index, so it moves through its own
            // stops. The shared nav commands still drive it: a footer chip that no-ops is the dead
            // chord version 0.4 refuses to advertise (`src/skit/tui_settings.py:408-415`).
            Screen::Settings(view) => view.move_focus(delta > 0),
            Screen::Preferences(view) => {
                let _ = view.update(if delta > 0 {
                    PreferencesAction::Next
                } else {
                    PreferencesAction::Previous
                });
            }
            Screen::Library
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Report(_) => {}
        }
    }

    fn focus_form_field(&mut self, index: usize) {
        match &mut self.workflow.active {
            Screen::Run(form) if index < form.fields.len() => form.focused = index,
            Screen::Form(form) if index < form.fields.len() => form.focused = index,
            Screen::Library
            | Screen::Run(_)
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Form(_)
            | Screen::Report(_) => (),
        }
    }

    fn append_form_input(&mut self, value: &str) {
        match &mut self.workflow.active {
            Screen::Run(form) => {
                if let Some(field) = form.fields.get_mut(form.focused) {
                    field.control.append(value);
                }
            }
            Screen::Form(form) => {
                if let Some(field) = form.fields.get_mut(form.focused) {
                    field.value.push_str(value);
                }
            }
            Screen::Library
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Report(_) => (),
        }
    }

    fn backspace_form_input(&mut self) {
        match &mut self.workflow.active {
            Screen::Run(form) => {
                if let Some(field) = form.fields.get_mut(form.focused) {
                    field.control.backspace();
                }
            }
            Screen::Form(form) => {
                if let Some(field) = form.fields.get_mut(form.focused) {
                    field.value.pop();
                }
            }
            Screen::Library
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Report(_) => (),
        }
    }

    fn submit(&mut self) -> Effect {
        if let Some(ModalState::RunPresetName { value, .. }) = &self.modal {
            let name = value.trim();
            if name.is_empty() {
                return Effect::None;
            }
            if let Screen::Run(form) = &self.workflow.active {
                return Effect::SaveRunPreset {
                    selector: form.selector.clone(),
                    name: name.to_owned(),
                    values: form.preset_snapshot(),
                    secret_names: form.secret_names().map(str::to_owned).collect(),
                };
            }
            return Effect::None;
        }
        if let Some(ModalState::ConfirmRemove { selector, .. }) = &self.modal {
            return Effect::Remove {
                selector: selector.clone(),
            };
        }
        match &mut self.workflow.active {
            Screen::Run(form) => {
                if form.validate() {
                    Effect::Submit {
                        purpose: FormPurpose::Run,
                        selector: Some(form.selector.clone()),
                        values: form.values(),
                    }
                } else {
                    Effect::None
                }
            }
            // A generic form is one request, so every control it drew travels. There is no
            // "unchanged" concept to express: the host is being asked to do the thing, not to
            // diff a record.
            Screen::Form(form) => Effect::Submit {
                purpose: form.purpose,
                selector: form.selector.clone(),
                values: form
                    .fields
                    .iter()
                    .map(|field| (field.key.clone(), FieldValue::text(&field.value)))
                    .collect(),
            },
            Screen::Library
            | Screen::Preferences(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Settings(_)
            | Screen::Report(_) => Effect::None,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected.unwrap_or_default();
        let last = self.visible.len() - 1;
        let next = if delta.is_negative() {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        self.selected = Some(next);
    }

    fn select_boundary(&mut self, end: bool) {
        self.selected = if self.visible.is_empty() {
            None
        } else if end {
            Some(self.visible.len() - 1)
        } else {
            Some(0)
        };
    }

    fn recompute_visible(&mut self, preferred: Option<&Slug>) {
        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut matcher = Matcher::new(MatcherConfig::DEFAULT);
        let mut utf32 = Vec::new();
        self.visible = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                fuzzy_matches(entry, &pattern, &mut matcher, &mut utf32).then_some(index)
            })
            .collect();
        self.selected = if self.visible.is_empty() {
            None
        } else {
            preferred
                .and_then(|slug| {
                    self.visible
                        .iter()
                        .position(|index| self.entries[*index].slug == *slug)
                })
                .or(Some(0))
        };
    }

    fn replace_surface(&mut self, surface: LibrarySurface) {
        self.entries = surface.scan.entries;
        self.diagnostics = surface.scan.diagnostics;
        self.details = surface.details;
        self.details
            .retain(|slug, _| self.entries.iter().any(|entry| &entry.slug == slug));
        self.entries.sort_by(|left, right| {
            let left_activity = self
                .details
                .get(&left.slug)
                .map_or("", LibraryEntryDetail::activity_at);
            let right_activity = self
                .details
                .get(&right.slug)
                .map_or("", LibraryEntryDetail::activity_at);
            right_activity.cmp(left_activity)
        });
    }
}

fn moved_focus(current: usize, field_count: usize, delta: isize) -> usize {
    let Some(last) = field_count.checked_sub(1) else {
        return 0;
    };
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as usize).min(last)
    }
}

fn fuzzy_matches(
    entry: &EntrySummary,
    pattern: &Pattern,
    matcher: &mut Matcher,
    utf32: &mut Vec<char>,
) -> bool {
    let historical = format!("{} {}", entry.name, entry.description);
    [
        historical.as_str(),
        entry.slug.as_str(),
        entry.kind.as_str(),
    ]
    .into_iter()
    .any(|candidate| {
        pattern
            .score(Utf32Str::new(candidate, utf32), matcher)
            .is_some()
    })
}
