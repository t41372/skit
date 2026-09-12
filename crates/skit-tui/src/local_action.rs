use ratatui_core::layout::Rect;
use ratatui_crossterm::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use skit_ui::{Action, HealthAction, RunnerEditorAction, RunnerManagerAction};

use crate::AddControlId;

/// One typed local-screen action that is not part of the shared command registry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalActionTarget {
    /// Add-workflow control.
    Add(AddControlId),
    /// Health-screen action.
    Health(HealthAction),
    /// Runner-management action.
    Runners(RunnerManagerAction),
    /// Standalone runner-editor action.
    RunnerEditor(RunnerEditorAction),
}

/// One exact key chord for a visible local action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocalKeyBinding {
    event: KeyEvent,
}

impl LocalKeyBinding {
    /// Return the Crossterm event that follows this advertised path.
    #[must_use]
    pub const fn event(self) -> KeyEvent {
        self.event
    }
}

/// One local action as it appeared in the most recent frame.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalAdvertisedAction {
    /// Typed semantic identity shared by the key and mouse paths.
    pub target: LocalActionTarget,
    /// Every key chord printed by the visible chip.
    pub keys: Vec<LocalKeyBinding>,
    /// Visible clickable cells for the same chip.
    pub hit: Option<Rect>,
    /// Exact session result that both input paths must produce.
    pub outcome: LocalActionOutcome,
}

/// Exact result of dispatching one advertised local action.
#[derive(Clone, Debug, PartialEq)]
pub enum LocalActionOutcome {
    /// Dispatch this frontend-neutral action through the reducer.
    Action(Action),
    /// Change terminal-only widget state without a reducer action.
    Consumed,
}

/// Local action inventory from the most recent rendered frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LocalActionInventory {
    /// Visible actions. Hidden and fully clipped chips are not present.
    pub actions: Vec<LocalAdvertisedAction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalKey {
    Enter,
    Escape,
    Space,
    Character(char),
    Control(char),
    Tab,
    BackTab,
    NextField,
    PreviousField,
}

impl LocalKey {
    pub(crate) fn accepts(self, key: &KeyEvent) -> bool {
        self.bindings().iter().any(|binding| {
            let advertised = binding.event();
            advertised.code == key.code
                && (advertised.modifiers == key.modifiers
                    || matches!(advertised.code, KeyCode::BackTab) && key.modifiers.is_empty())
        })
    }

    pub(crate) fn hint(self) -> String {
        match self {
            Self::Enter => "Enter".to_owned(),
            Self::Escape => "Esc".to_owned(),
            Self::Space => "Space".to_owned(),
            Self::Character(character) => character.to_string(),
            Self::Control(character) => format!("Ctrl+{}", character.to_ascii_uppercase()),
            Self::Tab => "Tab".to_owned(),
            Self::BackTab => "Shift+Tab".to_owned(),
            Self::NextField => "Tab/↓".to_owned(),
            Self::PreviousField => "Shift+Tab/↑".to_owned(),
        }
    }

    pub(crate) fn bindings(self) -> Vec<LocalKeyBinding> {
        let binding = |code, modifiers| LocalKeyBinding {
            event: KeyEvent::new(code, modifiers),
        };
        match self {
            Self::Enter => vec![binding(KeyCode::Enter, KeyModifiers::NONE)],
            Self::Escape => vec![binding(KeyCode::Esc, KeyModifiers::NONE)],
            Self::Space => vec![binding(KeyCode::Char(' '), KeyModifiers::NONE)],
            Self::Character(character) => {
                vec![binding(KeyCode::Char(character), KeyModifiers::NONE)]
            }
            Self::Control(character) => {
                vec![binding(KeyCode::Char(character), KeyModifiers::CONTROL)]
            }
            Self::Tab => vec![binding(KeyCode::Tab, KeyModifiers::NONE)],
            Self::BackTab => vec![binding(KeyCode::BackTab, KeyModifiers::SHIFT)],
            Self::NextField => vec![
                binding(KeyCode::Tab, KeyModifiers::NONE),
                binding(KeyCode::Down, KeyModifiers::NONE),
            ],
            Self::PreviousField => vec![
                binding(KeyCode::BackTab, KeyModifiers::SHIFT),
                binding(KeyCode::Up, KeyModifiers::NONE),
            ],
        }
    }
}
