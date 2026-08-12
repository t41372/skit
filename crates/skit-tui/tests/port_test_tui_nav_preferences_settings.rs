//! Preferences and Settings positive navigation pilots from Python `tests/test_tui_nav.py`
//! at `main@206f9ef`.
//!
//! These are interaction contracts, not command-registry shape tests: every advertised navigation
//! chip must be rendered and clickable, arrow twins must move focus where the focused widget does
//! not own them, and the direct Tab / Shift+Tab keys must reach the same controls.

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot,
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, LibraryState, PreferencesAction, PreferencesControlId, PreferencesView, Screen,
    SettingsInputs, SettingsView, UiCommand,
};

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(130, 40)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
}

fn rendered(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn drive(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    event: Event,
) -> EventHandling {
    let handling = session.handle_event(event, state, geometry);
    if let EventHandling::Action(action) = &handling {
        state.update(action.clone());
    }
    handling
}

fn click_footer_command(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    command: UiCommand,
) -> EventHandling {
    let area = geometry
        .hits
        .iter()
        .find_map(|hit| (hit.action == HitTarget::Command(command)).then_some(hit.rect))
        .unwrap_or_else(|| panic!("advertised footer command {command:?} has no click region"));
    drive(session, state, geometry, mouse(area.x, area.y))
}

fn preferences() -> PreferencesView {
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
        runner_names: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

fn preferences_focus(state: &LibraryState) -> PreferencesControlId {
    let Screen::Preferences(view) = state.screen() else {
        panic!("preferences screen is not active");
    };
    view.focused()
}

#[test]
fn test_prefs_boots_on_language_and_arrows_move() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(Box::new(preferences()))));
    let mut session = TuiSession::default();

    let (terminal, _) = draw(&mut session, &state);
    assert_eq!(preferences_focus(&state), PreferencesControlId::Language);
    let first_frame = rendered(terminal.backend().buffer());
    assert!(first_frame.contains("Preferences"), "{first_frame}");

    // Python explicitly focuses the editor before testing the arrow twin. Do the same rather than
    // relying on how many controls lie between Language and Editor.
    state.update(Action::Preferences(PreferencesAction::Focus(
        PreferencesControlId::Editor,
    )));
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        ),
        EventHandling::Action(Action::Preferences(PreferencesAction::Next))
    );
    assert_eq!(
        preferences_focus(&state),
        PreferencesControlId::InteractiveForm
    );

    // A radio group owns its arrows. Down changes the option but must not move focus to the next
    // section; Tab (or the footer navigation command) is what leaves the group.
    let (_, geometry) = draw(&mut session, &state);
    let handling = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Down, KeyModifiers::NONE),
    );
    assert!(
        matches!(
            handling,
            EventHandling::Action(Action::Preferences(
                PreferencesAction::SetInteractiveForm(_)
            )) | EventHandling::Consumed
        ),
        "radio Down must be owned by the radio control: {handling:?}"
    );
    assert_eq!(
        preferences_focus(&state),
        PreferencesControlId::InteractiveForm
    );

    let (terminal, geometry) = draw(&mut session, &state);
    let footer = rendered(terminal.backend().buffer());
    assert!(footer.contains("Tab/↓"), "missing forward navigation pill:\n{footer}");
    assert!(
        footer.contains("Shift+Tab/↑"),
        "missing backward navigation pill:\n{footer}"
    );
    let _ = click_footer_command(
        &mut session,
        &mut state,
        &geometry,
        UiCommand::FocusNext,
    );
    assert_eq!(preferences_focus(&state), PreferencesControlId::AfterRun);

    let (_, geometry) = draw(&mut session, &state);
    let _ = click_footer_command(
        &mut session,
        &mut state,
        &geometry,
        UiCommand::FocusPrevious,
    );
    assert_eq!(
        preferences_focus(&state),
        PreferencesControlId::InteractiveForm
    );

    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    assert_eq!(preferences_focus(&state), PreferencesControlId::Editor);

    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    assert_eq!(
        preferences_focus(&state),
        PreferencesControlId::InteractiveForm
    );
}

fn settings() -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "two".to_owned(),
        kind: "python".to_owned(),
        name: "two".to_owned(),
        description: "two fields".to_owned(),
        source: "/tmp/two.py".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        ..SettingsInputs::default()
    })
}

fn settings_focus(state: &LibraryState) -> String {
    let Screen::Settings(view) = state.screen() else {
        panic!("settings screen is not active");
    };
    view.focused().to_owned()
}

#[test]
fn test_settings_boots_on_name_and_arrows_move() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Settings(Box::new(settings()))));
    let mut session = TuiSession::default();

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(settings_focus(&state), "name");
    assert_eq!(
        drive(
            &mut session,
            &mut state,
            &geometry,
            key(KeyCode::Down, KeyModifiers::NONE),
        ),
        EventHandling::Action(Action::Settings(skit_ui::SettingsAction::FocusNext))
    );
    let second = settings_focus(&state);
    assert_ne!(second, "name");

    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Up, KeyModifiers::NONE),
    );
    assert_eq!(settings_focus(&state), "name");

    let (terminal, geometry) = draw(&mut session, &state);
    let footer = rendered(terminal.backend().buffer());
    assert!(footer.contains("Tab/↓"), "missing forward navigation pill:\n{footer}");
    assert!(
        footer.contains("Shift+Tab/↑"),
        "missing backward navigation pill:\n{footer}"
    );
    let _ = click_footer_command(
        &mut session,
        &mut state,
        &geometry,
        UiCommand::FocusNext,
    );
    assert_eq!(settings_focus(&state), second);

    let (_, geometry) = draw(&mut session, &state);
    let _ = click_footer_command(
        &mut session,
        &mut state,
        &geometry,
        UiCommand::FocusPrevious,
    );
    assert_eq!(settings_focus(&state), "name");

    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Tab, KeyModifiers::NONE),
    );
    assert_eq!(settings_focus(&state), second);

    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::BackTab, KeyModifiers::SHIFT),
    );
    assert_eq!(settings_focus(&state), "name");
}
