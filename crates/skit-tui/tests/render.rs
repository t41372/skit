use ratatui::{Terminal, backend::TestBackend, crossterm::event};
use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_tui::{HitAction, map_event, render};
use skit_ui::{Action, LibraryState};

fn state() -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries: vec![EntrySummary {
            slug: Slug::parse("hello").unwrap(),
            name: "Hello".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            description: "A friendly script".to_owned(),
            target: None,
        }],
        diagnostics: Vec::new(),
    })
}

#[test]
fn renderer_exposes_rows_and_clickable_footer_chips() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut geometry = None;

    terminal
        .draw(|frame| geometry = Some(render(frame, &state())))
        .unwrap();

    let geometry = geometry.unwrap();
    assert!(geometry.rows.width > 0);
    assert!(geometry.rows.height > 0);
    assert!(geometry.hits.iter().any(|hit| hit.action == HitAction::Quit));
    assert!(geometry.hits.iter().any(|hit| hit.action == HitAction::Reload));
    assert!(geometry.hits.iter().any(|hit| hit.action == HitAction::Search));
}

#[test]
fn keyboard_events_follow_browse_and_search_grammar() {
    let browse = state();
    assert_eq!(
        map_event(event::Event::Key(event::KeyEvent::new(
            event::KeyCode::Char('q'),
            event::KeyModifiers::NONE,
        )), &browse, &Default::default()),
        Some(Action::Quit)
    );
    assert_eq!(
        map_event(event::Event::Key(event::KeyEvent::new(
            event::KeyCode::Char('/'),
            event::KeyModifiers::NONE,
        )), &browse, &Default::default()),
        Some(Action::BeginSearch)
    );

    let mut searching = browse;
    searching.update(Action::BeginSearch);
    assert_eq!(
        map_event(event::Event::Key(event::KeyEvent::new(
            event::KeyCode::Char('q'),
            event::KeyModifiers::NONE,
        )), &searching, &Default::default()),
        Some(Action::Input('q'))
    );
}

#[test]
fn mouse_wheel_and_row_clicks_have_the_same_actions_as_the_keyboard() {
    let mut geometry = skit_tui::ViewGeometry::default();
    geometry.rows = ratatui::layout::Rect::new(2, 3, 30, 4);
    geometry.first_visible = 5;

    let wheel = event::Event::Mouse(event::MouseEvent {
        kind: event::MouseEventKind::ScrollDown,
        column: 10,
        row: 10,
        modifiers: event::KeyModifiers::NONE,
    });
    assert_eq!(map_event(wheel, &state(), &geometry), Some(Action::Next));

    let click = event::Event::Mouse(event::MouseEvent {
        kind: event::MouseEventKind::Down(event::MouseButton::Left),
        column: 4,
        row: 4,
        modifiers: event::KeyModifiers::NONE,
    });
    assert_eq!(
        map_event(click, &state(), &geometry),
        Some(Action::SelectVisible(6))
    );
}
