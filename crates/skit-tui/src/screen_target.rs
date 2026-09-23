//! Owned live input targets for the corpus adapter.

use std::path::PathBuf;

use ratatui_core::layout::Rect;
use skit_application::AgentScope;
use skit_ui::PreferencesControlId;

use crate::AddControlId;

/// One closed identity from the currently rendered terminal screen.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenTarget {
    /// One Add screen control.
    Add(AddControlId),
    /// One Preferences control.
    Preferences(PreferencesControlId),
    /// One detected Agent Skill target.
    AgentSkill {
        /// Stable detected agent name.
        name: String,
        /// Stable detected agent scope.
        scope: AgentScope,
    },
    /// One prompt-runner row of the agent list.
    Runner {
        /// Zero-based draft row, which addresses a malformed or a repeated name.
        row: usize,
        /// Stable runner name the row shows, or `None` for a malformed row.
        name: Option<String>,
    },
    /// One command chip of the agent-list cursor row.
    RunnerChip {
        /// Zero-based draft row that owns the chip.
        row: usize,
        /// Command the chip performs.
        chip: RunnerChip,
    },
    /// One portable entry below the configured memory picker root.
    FilePickerEntry {
        /// Relative path below the configured picker root.
        relative: PathBuf,
    },
}

/// One command a prompt-runner row offers on the cursor row.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerChip {
    /// Open the shared runner editor on the row.
    Edit,
    /// Stage or cancel the removal of the row.
    Remove,
}

/// The current keyboard-focus state for one rendered screen.
#[doc(hidden)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScreenFocusInventory {
    /// Current focus target, if the screen accepts keyboard focus.
    pub current: Option<ScreenTarget>,
    /// Exact current focus order.
    pub order: Vec<ScreenTarget>,
}

/// One nonempty clickable rectangle from the latest rendered frame.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScreenTargetHit {
    /// Typed identity of the clicked control.
    pub target: ScreenTarget,
    /// Rectangle emitted by the current renderer.
    pub rect: Rect,
}

/// Immutable target discovery from the latest rendered screen.
#[doc(hidden)]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScreenTargetInventory {
    /// Existing typed targets, including currently clipped targets.
    pub available: Vec<ScreenTarget>,
    /// Current keyboard focus order when the active screen has one.
    pub focus: Option<ScreenFocusInventory>,
    /// Current nonempty renderer hit rectangles.
    pub hits: Vec<ScreenTargetHit>,
}

/// The current state and the latest terminal session do not agree.
#[doc(hidden)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScreenTargetError {
    /// The requested screen has no target inventory.
    ScreenUnavailable,
    /// The rendered session belongs to a different screen shape.
    StaleSession,
    /// The active file picker reads the real filesystem.
    RealFilesystemPicker,
    /// A memory-picker entry is outside its configured root.
    InvalidMemoryEntry,
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, path::PathBuf};

    use ratatui_core::backend::TestBackend;
    use ratatui_core::terminal::Terminal;
    use ratatui_crossterm::crossterm::event::{
        Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    };
    use skit_application::{
        AgentScope, AgentTarget,
        preferences::{
            AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
            PreferencesDraft, PreferencesSnapshot, ThemeChoice,
        },
    };
    use skit_i18n::Locale;
    use skit_ui::{
        Action, AddWorkflowState, DraftSummary, LibraryState, PreferencesAction,
        PreferencesControlId, PreferencesView, Screen,
    };

    use super::*;
    use crate::{AddControlId, TuiSession, ViewGeometry, render_with_session};

    fn render(
        terminal: &mut Terminal<TestBackend>,
        state: &LibraryState,
        session: &mut TuiSession,
    ) -> ViewGeometry {
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, state, Locale::En, session);
            })
            .unwrap();
        geometry
    }

    fn preferences_state() -> LibraryState {
        let draft = PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: "en".to_owned(),
            available_languages: vec!["en".to_owned()],
            effective_language: "en".to_owned(),
            editor: "vi".to_owned(),
            editor_fallback: None,
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Stay,
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: Vec::new(),
            mirror: MirrorConfiguration::default(),
        });
        let mut state = LibraryState::default();
        let _ = state.update(Action::Present(Screen::Preferences(Box::new(
            PreferencesView::new(draft),
        ))));
        state
    }

    fn add_state(drafts: Vec<DraftSummary>) -> LibraryState {
        let mut state = LibraryState::default();
        let _ = state.update(Action::Present(Screen::Add(Box::new(
            AddWorkflowState::new(drafts),
        ))));
        state
    }

    fn primary(rect: ratatui_core::layout::Rect, kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn library_has_an_empty_live_target_inventory() {
        let state = LibraryState::default();
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();

        assert_eq!(
            session.screen_target_inventory(&state).unwrap(),
            ScreenTargetInventory::default()
        );
    }

    #[test]
    fn add_inventory_uses_the_live_focus_and_body_geometry() {
        let draft = DraftSummary {
            path: PathBuf::from("draft.py"),
            modified: 1,
            identity: None,
            permissions: Default::default(),
            content_hash: None,
        };
        let state = add_state(vec![draft]);
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let _ = render(&mut terminal, &state, &mut session);

        let inventory = session.screen_target_inventory(&state).unwrap();
        assert!(
            inventory
                .available
                .contains(&ScreenTarget::Add(AddControlId::Text(
                    crate::AddTextField::SourcePath
                )))
        );
        assert!(
            inventory
                .available
                .contains(&ScreenTarget::Add(AddControlId::Draft(0)))
        );
        assert!(
            inventory
                .available
                .contains(&ScreenTarget::Add(AddControlId::BrowseSource))
        );
        assert!(
            inventory
                .hits
                .iter()
                .any(|hit| hit.target == ScreenTarget::Add(AddControlId::BrowseSource))
        );
        assert_eq!(
            inventory
                .focus
                .as_ref()
                .and_then(|focus| focus.current.clone()),
            Some(ScreenTarget::Add(AddControlId::Text(
                crate::AddTextField::SourcePath
            )))
        );
    }

    #[test]
    fn preferences_inventory_uses_controls_and_hides_them_behind_agent_overlay() {
        let mut state = preferences_state();
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        let _ = render(&mut terminal, &state, &mut session);
        let inventory = session.screen_target_inventory(&state).unwrap();
        for id in [
            PreferencesControlId::NewRunner,
            PreferencesControlId::InstallAgentSkill,
        ] {
            assert!(inventory.available.contains(&ScreenTarget::Preferences(id)));
            assert!(
                inventory
                    .hits
                    .iter()
                    .any(|hit| hit.target == ScreenTarget::Preferences(id))
            );
        }

        let target = AgentTarget {
            name: "codex".to_owned(),
            scope: AgentScope::Project,
            base: PathBuf::from("/project/.codex"),
        };
        let _ = state.update(Action::Preferences(
            PreferencesAction::PresentAgentSkillTargets(vec![target]),
        ));
        let _ = render(&mut terminal, &state, &mut session);
        let overlay = session.screen_target_inventory(&state).unwrap();
        assert_eq!(
            overlay.available,
            vec![ScreenTarget::AgentSkill {
                name: "codex".to_owned(),
                scope: AgentScope::Project,
            }]
        );
        assert!(
            overlay
                .hits
                .iter()
                .all(|hit| { matches!(hit.target, ScreenTarget::AgentSkill { .. }) })
        );
        assert!(overlay.focus.is_none());
    }

    #[test]
    fn memory_picker_uses_relative_entries_and_real_picker_refuses() {
        let state = add_state(Vec::new());
        let root = PathBuf::from("/memory");
        let directory = root.join("nested");
        let file = directory.join("source.py");
        let mut memory_session = TuiSession::with_file_picker_tree(
            root.clone(),
            BTreeSet::from([root.clone(), directory.clone()]),
            BTreeSet::from([file]),
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        let geometry = render(&mut terminal, &state, &mut memory_session);
        let browse = memory_session
            .screen_target_inventory(&state)
            .unwrap()
            .hits
            .into_iter()
            .find(|hit| hit.target == ScreenTarget::Add(AddControlId::BrowseSource))
            .unwrap()
            .rect;
        let _ = memory_session.handle_event(
            primary(browse, MouseEventKind::Down(MouseButton::Left)),
            &state,
            &geometry,
        );
        let _ = memory_session.handle_event(
            primary(browse, MouseEventKind::Up(MouseButton::Left)),
            &state,
            &geometry,
        );
        let overlay_geometry = render(&mut terminal, &state, &mut memory_session);
        let inventory = memory_session.screen_target_inventory(&state).unwrap();
        assert_eq!(
            inventory.available,
            vec![ScreenTarget::FilePickerEntry {
                relative: PathBuf::from("nested"),
            }]
        );
        assert!(
            inventory
                .hits
                .iter()
                .all(|hit| { matches!(hit.target, ScreenTarget::FilePickerEntry { .. }) })
        );
        let nested = inventory
            .hits
            .iter()
            .find(|hit| {
                hit.target
                    == ScreenTarget::FilePickerEntry {
                        relative: PathBuf::from("nested"),
                    }
            })
            .unwrap()
            .rect;
        let _ = memory_session.handle_event(
            primary(nested, MouseEventKind::Down(MouseButton::Left)),
            &state,
            &overlay_geometry,
        );
        let _ = memory_session.handle_event(
            primary(nested, MouseEventKind::Up(MouseButton::Left)),
            &state,
            &overlay_geometry,
        );
        let _ = render(&mut terminal, &state, &mut memory_session);
        let nested_inventory = memory_session.screen_target_inventory(&state).unwrap();
        assert_eq!(
            nested_inventory.available,
            vec![ScreenTarget::FilePickerEntry {
                relative: PathBuf::from("nested/source.py"),
            }]
        );

        let mut real_session = TuiSession::default();
        let geometry = render(&mut terminal, &state, &mut real_session);
        let _ = real_session.handle_event(
            primary(browse, MouseEventKind::Down(MouseButton::Left)),
            &state,
            &geometry,
        );
        let _ = real_session.handle_event(
            primary(browse, MouseEventKind::Up(MouseButton::Left)),
            &state,
            &geometry,
        );
        let _ = render(&mut terminal, &state, &mut real_session);
        assert_eq!(
            real_session.screen_target_inventory(&state),
            Err(ScreenTargetError::RealFilesystemPicker)
        );
    }

    #[test]
    fn inventories_are_owned_and_keep_clipped_targets() {
        let state = add_state(Vec::new());
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        let _ = render(&mut terminal, &state, &mut session);
        let inventory = session.screen_target_inventory(&state).unwrap();
        assert!(!inventory.available.is_empty());
        assert!(inventory.hits.is_empty());
        let saved = inventory.clone();

        let library = LibraryState::default();
        let _ = render(&mut terminal, &library, &mut session);
        assert_eq!(inventory, saved);
    }

    #[test]
    fn events_and_screen_changes_require_a_fresh_target_render() {
        let state = add_state(Vec::new());
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let geometry = render(&mut terminal, &state, &mut session);
        let before_resize = session.screen_target_inventory(&state).unwrap();
        let _ = session.handle_event(Event::Resize(60, 18), &state, &geometry);
        assert_eq!(
            session.screen_target_inventory(&state),
            Err(ScreenTargetError::StaleSession)
        );
        terminal.backend_mut().resize(60, 18);
        terminal.resize(Rect::new(0, 0, 60, 18)).unwrap();
        let _ = render(&mut terminal, &state, &mut session);
        let after_resize = session.screen_target_inventory(&state).unwrap();
        assert_eq!(after_resize.available, before_resize.available);
        assert_ne!(after_resize.hits, before_resize.hits);
        assert!(after_resize.hits.iter().all(|hit| {
            !hit.rect.is_empty() && hit.rect.right() <= 60 && hit.rect.bottom() <= 18
        }));

        let mut preferences = preferences_state();
        assert_eq!(
            session.screen_target_inventory(&preferences),
            Err(ScreenTargetError::StaleSession)
        );
        let _ = render(&mut terminal, &preferences, &mut session);
        assert!(session.screen_target_inventory(&preferences).is_ok());

        let _ = preferences.update(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::NewRunner,
        )));
        assert_eq!(
            session.screen_target_inventory(&preferences),
            Err(ScreenTargetError::StaleSession)
        );
        let _ = render(&mut terminal, &preferences, &mut session);
        assert_eq!(
            session
                .screen_target_inventory(&preferences)
                .unwrap()
                .focus
                .and_then(|focus| focus.current),
            Some(ScreenTarget::Preferences(PreferencesControlId::NewRunner))
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unserializable_render_state_never_becomes_a_fresh_inventory() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt as _};

        let draft = |suffix| DraftSummary {
            path: PathBuf::from(OsString::from_vec(vec![
                b'd', b'r', b'a', b'f', b't', suffix,
            ])),
            modified: 1,
            identity: None,
            permissions: Default::default(),
            content_hash: None,
        };
        let rendered = add_state(vec![draft(0xff)]);
        let changed = add_state(vec![draft(0xfe)]);
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let _ = render(&mut terminal, &rendered, &mut session);

        assert_eq!(
            session.screen_target_inventory(&rendered),
            Err(ScreenTargetError::StaleSession)
        );
        assert_eq!(
            session.screen_target_inventory(&changed),
            Err(ScreenTargetError::StaleSession)
        );
    }

    #[test]
    fn open_overlay_and_modal_hide_or_refuse_stale_targets() {
        let mut state = add_state(Vec::new());
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let geometry = render(&mut terminal, &state, &mut session);
        let browse = session
            .screen_target_inventory(&state)
            .unwrap()
            .hits
            .into_iter()
            .find(|hit| hit.target == ScreenTarget::Add(AddControlId::BrowseSource))
            .unwrap()
            .rect;
        let _ = session.handle_event(
            primary(browse, MouseEventKind::Down(MouseButton::Left)),
            &state,
            &geometry,
        );
        let _ = session.handle_event(
            primary(browse, MouseEventKind::Up(MouseButton::Left)),
            &state,
            &geometry,
        );
        assert_eq!(
            session.screen_target_inventory(&state),
            Err(ScreenTargetError::StaleSession)
        );
        let _ = render(&mut terminal, &state, &mut session);
        assert_eq!(
            session.screen_target_inventory(&state),
            Err(ScreenTargetError::RealFilesystemPicker)
        );

        let _ = state.update(Action::OpenHelp);
        assert_eq!(
            session.screen_target_inventory(&state),
            Err(ScreenTargetError::StaleSession)
        );
        let _ = render(&mut terminal, &state, &mut session);
        assert_eq!(
            session.screen_target_inventory(&state).unwrap(),
            ScreenTargetInventory::default()
        );
    }
}
