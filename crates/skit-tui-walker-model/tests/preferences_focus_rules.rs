//! Preferences focus rules: a frame advertises only the keys the focused widget releases.
//!
//! A shared footer chord that the focused widget owns must stay unreachable, and a `Tab` prefix
//! must not launder it back. Every state is built from `LibraryState` values, so no host is
//! involved. The probe helpers come from this crate's `parity` module.

use ratatui_core::{buffer::Buffer, layout::Size};
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot,
};
use skit_i18n::Locale;
use skit_tui::{HitTarget, TuiSession};
use skit_tui_walker_model::parity::{
    ProbeEndpoint, command_bindings, command_key_action, render_probe_endpoint,
};
use skit_ui::{
    Action, Effect, LibraryState, PreferencesAction, PreferencesControlId, PreferencesView, Screen,
    UiCommand, UiKey,
};

// --------------------------------------------------------------------------
// helpers this file owns
// --------------------------------------------------------------------------

/// Render one Preferences frame on a fresh session and keep both the session and the frame.
fn draw(state: &LibraryState) -> (TuiSession, ProbeEndpoint) {
    let mut session = TuiSession::default();
    let endpoint =
        render_probe_endpoint(&mut session, state, Locale::En, Size::new(100, 30)).unwrap();
    (session, endpoint)
}

fn rendered_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

// --------------------------------------------------------------------------
// state fixture
// --------------------------------------------------------------------------

fn preferences_state() -> LibraryState {
    let mut state = LibraryState::default();
    let effect = state.update(Action::Present(Screen::Preferences(Box::new(
        PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runner_names: vec!["codex".to_owned()],
            mirror: MirrorConfiguration::default(),
        })),
    ))));
    assert_eq!(effect, Effect::None);
    state
}

// --------------------------------------------------------------------------
// contract
// --------------------------------------------------------------------------

/// Ledger row `driver.rs:3622`.
#[test]
fn the_first_preferences_frame_advertises_only_keys_the_focused_widget_releases() {
    let mut state = preferences_state();

    let (session, frame) = draw(&state);
    assert_eq!(
        session
            .advertised_command_bindings(&state, UiCommand::FocusNext)
            .iter()
            .map(|binding| binding.key)
            .collect::<Vec<_>>(),
        [UiKey::Tab],
        "the first Preferences frame advertises only Tab for FocusNext"
    );
    let text = rendered_text(frame.backend.buffer());
    assert!(
        !text.contains("Tab/↓"),
        "the first Preferences frame rendered a key the focused widget owns: {text}"
    );

    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::Editor,
        ))),
        Effect::None
    );
    let (session, frame) = draw(&state);
    for command in [UiCommand::ManageAgents, UiCommand::InstallAgentSkill] {
        assert!(
            session
                .advertised_command_bindings(&state, command)
                .is_empty(),
            "the focused Editor field must hide the {command:?} binding"
        );
        assert!(
            frame
                .geometry
                .hits
                .iter()
                .all(|hit| hit.action != HitTarget::Command(command)),
            "the focused Editor field must hide the {command:?} hit"
        );
    }
    let text = rendered_text(frame.backend.buffer());
    assert!(!text.contains("Manage agents"), "{text}");
    assert!(!text.contains("Teach an AI agent skit"), "{text}");

    let blocked = command_bindings(&state, UiCommand::InstallAgentSkill).unwrap();
    // The crate reports an advertised command with no chord as an empty list. This rule needs one
    // real chord, so the refusal stays here.
    assert!(
        !blocked.is_empty(),
        "visible command InstallAgentSkill has no key binding"
    );
    let expected = Action::Preferences(PreferencesAction::InstallAgentSkill);
    assert!(
        command_key_action(
            &session,
            &state,
            &frame.geometry,
            UiCommand::InstallAgentSkill,
            &blocked,
            &expected,
            0,
        )
        .is_err(),
        "a shared footer key must not use a Tab prefix to hide immediate widget ownership"
    );
    assert!(
        command_key_action(
            &session,
            &state,
            &frame.geometry,
            UiCommand::InstallAgentSkill,
            &blocked,
            &expected,
            64,
        )
        .is_ok(),
        "the same chord must remain reachable after a deliberate focus move"
    );
}
