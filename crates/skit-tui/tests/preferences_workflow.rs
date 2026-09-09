use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
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
    Action, LibraryState, ModalState, PreferencesAction, PreferencesControlId, PreferencesView,
    Screen, UiCommand,
};

fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> Terminal<TestBackend> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    terminal
}

fn rendered_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn buffer_position(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let cells = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<Vec<_>>();
        for column in 0..cells.len() {
            if cells[column..].concat().starts_with(needle) {
                return (u16::try_from(column).unwrap(), row);
            }
        }
    }
    panic!("missing {needle:?}");
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
    // Version 0.4 shows the question once inside the confirmation panel.
    assert_eq!(rendered.matches("Discard unsaved changes?").count(), 1);
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
            EventHandling::Consumed,
            "primary Down must arm {command:?} without activating it"
        );
        assert_eq!(
            session.handle_event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Up(MouseButton::Left),
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

#[test]
fn resize_only_keeps_the_focused_preferences_control_visible() {
    let mut view = preferences();
    let _ = view.update(PreferencesAction::Focus(PreferencesControlId::NpmChoice));
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(Box::new(view))));
    let mut session = TuiSession::default();

    let large = draw(&mut session, &state, 70, 100);
    assert!(rendered_text(large.backend().buffer()).contains("npm registry"));
    let small = draw(&mut session, &state, 70, 18);
    let rendered = rendered_text(small.backend().buffer());
    assert!(
        rendered.contains("npm registry"),
        "resize-only reflow hid the focused Preferences control: {rendered}"
    );
}

#[test]
fn preferences_arm_cannot_survive_a_release_owned_by_the_global_footer() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Preferences(
        Box::new(preferences()),
    )));
    state.update(Action::Preferences(PreferencesAction::SetEditor(
        "editor-probe".to_owned(),
    )));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 28)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let editor = buffer_position(terminal.backend().buffer(), "editor-probe");
    let footer = geometry
        .hits
        .iter()
        .find(|hit| matches!(hit.action, HitTarget::Command(_)))
        .expect("the global footer must have a hit")
        .rect;
    let mouse = |kind, column, row| {
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        })
    };

    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), editor.0, editor.1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), footer.x, footer.y),
            &state,
            &geometry,
        ),
        EventHandling::Ignored
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), editor.0, editor.1),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a release in another owner must cancel the Preferences arm"
    );
}
