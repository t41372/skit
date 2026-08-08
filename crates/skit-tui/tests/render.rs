use ratatui_core::{backend::TestBackend, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{Diagnostic, DiagnosticCode, LibraryScan};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{HitAction, HitRegion, ViewGeometry, map_event, render, render_localized};
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

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
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
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.action == HitAction::Quit)
    );
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.action == HitAction::Reload)
    );
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.action == HitAction::Search)
    );
}

#[test]
fn renderer_uses_the_explicit_frontend_locale() {
    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_localized(frame, &state(), Locale::ZhTw);
        })
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("程 式 庫"));
    assert!(text.contains("項 目"));
    assert!(text.contains("詳 細 資 料"));
    assert!(text.contains("結 束"));
    assert!(!text.contains("Library"));
}

#[test]
fn renderer_handles_narrow_empty_search_status_and_diagnostics_views() {
    let diagnostic = Diagnostic {
        code: DiagnosticCode::CorruptMetadata,
        slug: Some("bad".to_owned()),
        message: "bad TOML".to_owned(),
    };
    let mut states = vec![
        LibraryState::default(),
        LibraryState::from_scan(LibraryScan {
            entries: Vec::new(),
            diagnostics: vec![diagnostic],
        }),
    ];
    let mut searching = state();
    searching.update(Action::BeginSearch);
    searching.update(Action::Input('x'));
    states.push(searching);
    let mut status = state();
    status.update(Action::SetStatus("reload failed".to_owned()));
    states.push(status);

    for state in states {
        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let geometry = render(frame, &state);
                assert!(geometry.rows.width > 0);
            })
            .unwrap();
    }
}

#[test]
fn browse_keyboard_events_cover_navigation_commands_and_ignored_input() {
    let browse = state();
    let geometry = ViewGeometry::default();
    let cases = [
        (KeyCode::Char('q'), KeyModifiers::NONE, Action::Quit),
        (KeyCode::Esc, KeyModifiers::NONE, Action::Quit),
        (KeyCode::Char('r'), KeyModifiers::NONE, Action::Reload),
        (KeyCode::Char('/'), KeyModifiers::NONE, Action::BeginSearch),
        (KeyCode::Up, KeyModifiers::NONE, Action::Previous),
        (KeyCode::Char('k'), KeyModifiers::NONE, Action::Previous),
        (KeyCode::Down, KeyModifiers::NONE, Action::Next),
        (KeyCode::Char('j'), KeyModifiers::NONE, Action::Next),
        (KeyCode::PageUp, KeyModifiers::NONE, Action::PagePrevious),
        (KeyCode::PageDown, KeyModifiers::NONE, Action::PageNext),
        (KeyCode::Home, KeyModifiers::NONE, Action::Home),
        (KeyCode::End, KeyModifiers::NONE, Action::End),
        (KeyCode::Char('c'), KeyModifiers::CONTROL, Action::Quit),
    ];

    for (code, modifiers, action) in cases {
        assert_eq!(
            map_event(key(code, modifiers), &browse, &geometry),
            Some(action)
        );
    }
    assert_eq!(
        map_event(
            key(KeyCode::Char('x'), KeyModifiers::NONE),
            &browse,
            &geometry
        ),
        None
    );
}

#[test]
fn search_keyboard_events_edit_or_finish_without_triggering_browse_shortcuts() {
    let mut searching = state();
    searching.update(Action::BeginSearch);
    let geometry = ViewGeometry::default();

    assert_eq!(
        map_event(
            key(KeyCode::Char('q'), KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        Some(Action::Input('q'))
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('Q'), KeyModifiers::SHIFT),
            &searching,
            &geometry
        ),
        Some(Action::Input('Q'))
    );
    assert_eq!(
        map_event(
            key(KeyCode::Backspace, KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        Some(Action::Backspace)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Enter, KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        Some(Action::FinishSearch)
    );
    assert_eq!(
        map_event(key(KeyCode::Esc, KeyModifiers::NONE), &searching, &geometry),
        Some(Action::FinishSearch)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &searching,
            &geometry
        ),
        Some(Action::ClearSearch)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('x'), KeyModifiers::ALT),
            &searching,
            &geometry
        ),
        None
    );
    assert_eq!(
        map_event(
            key(KeyCode::Left, KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        None
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &searching,
            &geometry
        ),
        Some(Action::Quit)
    );
}

#[test]
fn release_focus_paste_and_resize_events_are_ignored() {
    let state = state();
    let geometry = ViewGeometry::default();
    let events = [
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Char('q'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        )),
        Event::FocusGained,
        Event::FocusLost,
        Event::Paste("text".to_owned()),
        Event::Resize(80, 24),
    ];

    for event in events {
        assert_eq!(map_event(event, &state, &geometry), None);
    }
}

#[test]
fn mouse_wheel_rows_and_footer_hits_map_to_frontend_neutral_actions() {
    let geometry = ViewGeometry {
        rows: Rect::new(2, 3, 30, 4),
        first_visible: 5,
        hits: vec![
            HitRegion {
                rect: Rect::new(0, 10, 5, 1),
                action: HitAction::Quit,
            },
            HitRegion {
                rect: Rect::new(6, 10, 7, 1),
                action: HitAction::Reload,
            },
            HitRegion {
                rect: Rect::new(14, 10, 8, 1),
                action: HitAction::Search,
            },
        ],
    };
    let state = state();

    assert_eq!(
        map_event(mouse(MouseEventKind::ScrollUp, 40, 20), &state, &geometry),
        Some(Action::Previous)
    );
    assert_eq!(
        map_event(mouse(MouseEventKind::ScrollDown, 40, 20), &state, &geometry),
        Some(Action::Next)
    );
    assert_eq!(
        map_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 4, 4),
            &state,
            &geometry
        ),
        Some(Action::SelectVisible(6))
    );
    assert_eq!(
        map_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, 10),
            &state,
            &geometry
        ),
        Some(Action::Quit)
    );
    assert_eq!(
        map_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 8, 10),
            &state,
            &geometry
        ),
        Some(Action::Reload)
    );
    assert_eq!(
        map_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 16, 10),
            &state,
            &geometry
        ),
        Some(Action::BeginSearch)
    );
    assert_eq!(
        map_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 99, 99),
            &state,
            &geometry
        ),
        None
    );
}

#[test]
fn unsupported_mouse_gestures_are_ignored() {
    let state = state();
    let geometry = ViewGeometry::default();
    let kinds = [
        MouseEventKind::Down(MouseButton::Right),
        MouseEventKind::Down(MouseButton::Middle),
        MouseEventKind::Up(MouseButton::Left),
        MouseEventKind::Drag(MouseButton::Left),
        MouseEventKind::Moved,
        MouseEventKind::ScrollLeft,
        MouseEventKind::ScrollRight,
    ];

    for kind in kinds {
        assert_eq!(map_event(mouse(kind, 0, 0), &state, &geometry), None);
    }
}
