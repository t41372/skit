use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot,
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, render_with_session};
use skit_ui::{
    Action, LibraryState, ModalState, PreferencesAction, PreferencesView, Screen, UiCommand,
};

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

#[test]
fn dirty_preferences_discard_guard_has_exact_keys_and_clickable_actions() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::SetEditor(
        "micro".to_owned(),
    )));
    state.update(Action::Preferences(PreferencesAction::Close));
    assert_eq!(state.modal(), Some(&ModalState::ConfirmDiscardChanges));

    let mut terminal = Terminal::new(TestBackend::new(72, 20)).unwrap();
    let mut session = TuiSession::default();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    // Version 0.4 shows the question once, inside an untitled border
    // (`src/skit/tui_settings.py:42-65`). One more copy names the surface in
    // the screen header, the port's screen-swap convention; the old panel
    // title made it three.
    assert_eq!(rendered.matches("Discard unsaved changes?").count(), 2);
    assert!(rendered.contains("Discard"));
    assert!(rendered.contains("Keep editing"));

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::KeepEditing)
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::DiscardChanges)
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::KeepEditing)
    );
    // Version 0.4 binds `y` and `escape,n` and nothing else
    // (`src/skit/tui_settings.py:43-46`). Enter must not throw the user's work away: the answer
    // reached by reflex has to be the safe one.
    assert_ne!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::DiscardChanges),
        "Enter discarded unsaved work"
    );

    for (command, expected) in [
        (UiCommand::DiscardChanges, Action::DiscardChanges),
        (UiCommand::KeepEditing, Action::KeepEditing),
    ] {
        let area = geometry
            .hits
            .iter()
            .find_map(|hit| (hit.action == HitTarget::Command(command)).then_some(hit.rect))
            .expect("the visible discard action must be clickable");
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: area.x,
                    row: area.y,
                    modifiers: KeyModifiers::NONE,
                }),
                &state,
                &geometry,
            ),
            EventHandling::Action(expected)
        );
    }
}
