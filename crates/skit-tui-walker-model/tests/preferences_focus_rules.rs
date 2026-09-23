//! Preferences focus rules: a frame advertises only the keys the focused widget releases.
//!
//! Agent management has one door, so no focus state advertises a chord or a chip for it. Every
//! state is built from `LibraryState` values, so no host is involved. The probe helpers come from
//! this crate's `parity` module.

use ratatui_core::{buffer::Buffer, layout::Size};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot, ThemeChoice,
};
use skit_i18n::Locale;
use skit_tui::{HitTarget, TuiSession};
use skit_tui_walker_model::parity::{ProbeEndpoint, render_probe_endpoint, session_action};
use skit_ui::{
    Action, CommandContext, Effect, LibraryState, PreferencesAction, PreferencesControlId,
    PreferencesView, RunnerRow, RunnerRowIdentity, Screen, UiCommand, UiKey, command_specs,
};

/// The English labels of the two agent doors that Preferences paints.
const DOOR_LABELS: [&str; 2] = ["New agent…", "Teach an AI agent skit…"];

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

/// Return the text of the footer band: every row from the first chip down to the last row.
///
/// The doors paint their own labels in the body, so only the footer band can answer whether the
/// frame still advertises a key for them.
fn footer_text(endpoint: &ProbeEndpoint) -> String {
    let buffer = endpoint.backend.buffer();
    let area = buffer.area;
    let top = endpoint
        .geometry
        .hits
        .iter()
        .map(|hit| hit.rect.y)
        .min()
        .expect("every Preferences frame advertises at least one footer chip");
    (top..area.bottom())
        .flat_map(|row| (area.x..area.right()).map(move |column| (column, row)))
        .map(|position| buffer[position].symbol())
        .collect()
}

// --------------------------------------------------------------------------
// state fixture
// --------------------------------------------------------------------------

fn preferences_runner_row(index: usize, name: &str) -> RunnerRow {
    let identity = RunnerRowIdentity {
        index: Some(index),
        snapshot_token: format!("token-{index}"),
    };
    RunnerRow {
        key_identities: vec![identity.clone()],
        identity,
        name: Some(name.to_owned()),
        argv: Some(vec![name.to_owned(), "{{prompt}}".to_owned()]),
        reason: None,
        descriptor: format!("prompt.runners[{index}]"),
        pinned_count: 0,
    }
}

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
            theme: ThemeChoice::Terminal,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runners: vec![preferences_runner_row(0, "codex")],
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

    // The agent doors are reached by Tab and the mouse, never by a chord. The footer prints a
    // door label only as the verb of the Enter key while that door itself holds the focus.
    let controls = state
        .preferences()
        .expect("the fixture presents Preferences")
        .controls();
    assert!(
        controls
            .iter()
            .any(|control| control.id == PreferencesControlId::NewRunner)
    );
    for control in controls {
        assert_eq!(
            state.update(Action::Preferences(PreferencesAction::Focus(control.id))),
            Effect::None
        );
        let (session, frame) = draw(&state);
        let footer = footer_text(&frame);
        let activation = state
            .preferences()
            .expect("the fixture presents Preferences")
            .activation();
        for door in DOOR_LABELS {
            assert_eq!(
                footer.contains(door),
                activation.as_ref().is_some_and(|(verb, _)| *verb == door),
                "the focused {:?} control prints {door} in the footer: {footer}",
                control.id
            );
        }
        // Enter is printed exactly while a control that Enter activates holds the focus.
        assert_eq!(
            session
                .advertised_command_bindings(&state, UiCommand::Submit)
                .iter()
                .map(|binding| binding.key)
                .collect::<Vec<_>>(),
            if activation.is_some() {
                vec![UiKey::Enter]
            } else {
                Vec::new()
            },
            "{:?}",
            control.id
        );
        if let Some((verb, _)) = &activation {
            assert!(
                footer.contains(&format!("Enter {verb}")),
                "the focused {:?} control prints no verb for Enter: {footer}",
                control.id
            );
        }
        for hit in &frame.geometry.hits {
            let HitTarget::Command(command) = hit.action else {
                continue;
            };
            let label = command_specs(CommandContext::Preferences)
                .find(|spec| spec.command == command)
                .map_or("", |spec| spec.label);
            assert!(
                !DOOR_LABELS.contains(&label),
                "the focused {:?} control advertises a {label} chip",
                control.id
            );
        }
    }

    // The agent list names the verb of its cursor row, and drops it when no editor can open.
    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::Runners
        ))),
        Effect::None
    );
    let (_, frame) = draw(&state);
    assert!(footer_text(&frame).contains("Enter Edit"));
    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::ToggleRunnerRemoval)),
        Effect::None
    );
    let (session, frame) = draw(&state);
    let footer = footer_text(&frame);
    assert!(!footer.contains("Enter Edit"), "{footer}");
    assert!(
        session
            .advertised_command_bindings(&state, UiCommand::Submit)
            .is_empty()
    );
    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::ToggleRunnerRemoval)),
        Effect::None
    );

    for (door, action) in [
        (
            PreferencesControlId::InstallAgentSkill,
            Action::Preferences(PreferencesAction::InstallAgentSkill),
        ),
        (
            PreferencesControlId::NewRunner,
            Action::Preferences(PreferencesAction::NewRunner),
        ),
    ] {
        assert_eq!(
            state.update(Action::Preferences(PreferencesAction::Focus(door))),
            Effect::None
        );
        let (mut session, frame) = draw(&state);
        assert_eq!(
            session_action(
                &mut session,
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                &state,
                &frame.geometry,
            )
            .unwrap(),
            action,
            "Enter on the focused {door:?} door dispatches its typed action"
        );
    }
}
