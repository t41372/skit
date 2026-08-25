use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, LibraryState};

fn state() -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: vec![EntrySummary {
            slug: Slug::parse("unicode").unwrap(),
            name: "Unicode".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            description: String::new(),
            target: None,
        }],
        diagnostics: Vec::new(),
    })
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
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

#[test]
fn search_uses_grapheme_editing_paste_navigation_and_a_real_cursor() {
    let mut state = state();
    state.update(Action::BeginSearch);
    state.update(Action::SetSearchQuery("e\u{301}👨‍👩‍👧‍👦x".to_owned()));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);

    for event in [
        key(KeyCode::Home, KeyModifiers::NONE),
        key(KeyCode::Right, KeyModifiers::NONE),
        key(KeyCode::Delete, KeyModifiers::NONE),
    ] {
        let _ = drive(&mut session, &mut state, &geometry, event);
    }
    assert_eq!(state.query(), "e\u{301}x");

    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        key(KeyCode::Char('u'), KeyModifiers::CONTROL),
    );
    assert_eq!(state.query(), "");
    let _ = drive(
        &mut session,
        &mut state,
        &geometry,
        Event::Paste("界q".to_owned()),
    );
    assert_eq!(state.query(), "界q");

    let (terminal, _) = draw(&mut session, &state);
    let cursor = terminal.backend().cursor_position();
    assert!(
        cursor.y < 3,
        "search must expose a real cursor in the header"
    );
    assert!(cursor.x > 1);
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry,),
        EventHandling::Action(Action::OpenRun)
    );
}

#[test]
fn clicking_the_search_header_enters_search_mode() {
    let state = state();
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 2,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::BeginSearch)
    );
}

#[test]
fn ctrl_c_requires_a_second_press_and_uses_a_transient_mature_toast() {
    for searching in [false, true] {
        let mut state = state();
        if searching {
            state.update(Action::BeginSearch);
        }
        let mut session = TuiSession::default();
        let (_, geometry) = draw(&mut session, &state);
        let ctrl_c = || key(KeyCode::Char('c'), KeyModifiers::CONTROL);

        assert_eq!(
            session.handle_event(ctrl_c(), &state, &geometry),
            EventHandling::Consumed,
            "the first Ctrl+C must arm quit without closing either Library focus mode"
        );
        let (terminal, _) = draw(&mut session, &state);
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(
            rendered.contains("Press Ctrl+C again to quit"),
            "the mature transient notice must make the second chord discoverable: {rendered}"
        );
        assert_eq!(
            session.handle_event(ctrl_c(), &state, &geometry),
            EventHandling::Action(Action::Quit),
            "the second Ctrl+C inside the main two-second window must quit"
        );
    }
}
