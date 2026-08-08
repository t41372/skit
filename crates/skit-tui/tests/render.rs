use ratatui_core::{backend::TestBackend, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{Diagnostic, DiagnosticCode, LibraryScan};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{HitAction, HitRegion, ViewGeometry, map_event, render, render_localized};
use skit_ui::{
    Action, FormField, FormPurpose, FormView, LibraryState, ReportItem, ReportView, Screen,
};

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

    for action in [
        HitAction::Run,
        HitAction::Add,
        HitAction::Edit,
        HitAction::Settings,
        HitAction::Presets,
        HitAction::Rename,
        HitAction::Remove,
        HitAction::Preferences,
        HitAction::Health,
        HitAction::Runners,
    ] {
        assert!(
            geometry.hits.iter().any(|hit| hit.action == action),
            "missing footer action {action:?}"
        );
    }
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

    let mut form_state = state();
    form_state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Add,
        title: "Add an entry".to_owned(),
        selector: None,
        fields: vec![FormField::text("source", "Source path", "")],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    terminal.clear().unwrap();
    terminal
        .draw(|frame| {
            let _ = render_localized(frame, &form_state, Locale::ZhTw);
        })
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("新 增 項 目"));
    assert!(text.contains("來 源 路 徑"), "{text:?}");
    assert!(text.contains("儲 存"), "{text:?}");
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
fn every_library_footer_command_has_a_positive_keyboard_mapping() {
    let browse = state();
    let geometry = ViewGeometry::default();
    let cases = [
        (KeyCode::Enter, KeyModifiers::NONE, Action::OpenRun),
        (KeyCode::Char('n'), KeyModifiers::CONTROL, Action::OpenAdd),
        (KeyCode::Char('e'), KeyModifiers::CONTROL, Action::Edit),
        (KeyCode::Char('s'), KeyModifiers::NONE, Action::OpenSettings),
        (KeyCode::Char('p'), KeyModifiers::NONE, Action::OpenPresets),
        (KeyCode::F(2), KeyModifiers::NONE, Action::OpenRename),
        (KeyCode::Delete, KeyModifiers::NONE, Action::AskRemove),
        (
            KeyCode::Char(','),
            KeyModifiers::NONE,
            Action::OpenPreferences,
        ),
        (KeyCode::Char('h'), KeyModifiers::NONE, Action::OpenHealth),
        (KeyCode::Char('a'), KeyModifiers::NONE, Action::OpenRunners),
        (KeyCode::Char('r'), KeyModifiers::CONTROL, Action::Reload),
    ];

    for (code, modifiers, action) in cases {
        assert_eq!(
            map_event(key(code, modifiers), &browse, &geometry),
            Some(action)
        );
    }
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

fn form_state() -> LibraryState {
    let mut state = state();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Script settings".to_owned(),
        selector: Some("hello".to_owned()),
        fields: vec![
            FormField::text("name", "Name", "Hello"),
            FormField::secret("token", "Token", "secret"),
            FormField::multiline("description", "Description", "Line one"),
        ],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    state
}

#[test]
fn form_report_and_confirmation_screens_render_inside_small_terminals() {
    let mut report = state();
    report.update(Action::Present(Screen::Report(ReportView {
        title: "Health".to_owned(),
        items: vec![ReportItem {
            status: "ok".to_owned(),
            label: "Library".to_owned(),
            detail: "Ready".to_owned(),
        }],
    })));
    let mut confirm = state();
    confirm.update(Action::AskRemove);

    for view in [form_state(), report, confirm] {
        let backend = TestBackend::new(38, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut geometry = None;
        terminal
            .draw(|frame| geometry = Some(render(frame, &view)))
            .unwrap();
        let geometry = geometry.unwrap();
        assert!(
            geometry
                .hits
                .iter()
                .all(|hit| hit.rect.right() <= 38 && hit.rect.bottom() <= 12)
        );
    }
}

#[test]
fn form_keys_preserve_text_editing_and_advertised_screen_chords() {
    let form = form_state();
    let geometry = ViewGeometry::default();
    let cases = [
        (KeyCode::Tab, KeyModifiers::NONE, Action::FocusNext),
        (KeyCode::BackTab, KeyModifiers::SHIFT, Action::FocusPrevious),
        (KeyCode::Backspace, KeyModifiers::NONE, Action::Backspace),
        (KeyCode::Char('s'), KeyModifiers::CONTROL, Action::Submit),
        (KeyCode::Esc, KeyModifiers::NONE, Action::Back),
        (KeyCode::Char('x'), KeyModifiers::NONE, Action::Input('x')),
    ];
    for (code, modifiers, action) in cases {
        assert_eq!(
            map_event(key(code, modifiers), &form, &geometry),
            Some(action)
        );
    }
    assert_eq!(
        map_event(
            key(KeyCode::Char('e'), KeyModifiers::CONTROL),
            &form,
            &geometry
        ),
        None,
        "Ctrl+E must remain available to the input"
    );
}

#[test]
fn report_and_confirmation_keys_match_their_footer_actions() {
    let geometry = ViewGeometry::default();
    let mut report = state();
    report.update(Action::Present(Screen::Report(ReportView {
        title: "Health".to_owned(),
        items: vec![],
    })));
    assert_eq!(
        map_event(key(KeyCode::Esc, KeyModifiers::NONE), &report, &geometry),
        Some(Action::Back)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &report,
            &geometry
        ),
        Some(Action::Reload)
    );

    let mut confirm = state();
    confirm.update(Action::AskRemove);
    assert_eq!(
        map_event(key(KeyCode::Enter, KeyModifiers::NONE), &confirm, &geometry),
        Some(Action::Submit)
    );
    assert_eq!(
        map_event(key(KeyCode::Esc, KeyModifiers::NONE), &confirm, &geometry),
        Some(Action::Back)
    );
}

#[test]
fn every_rendered_chip_and_form_row_is_clickable() {
    for view in [state(), form_state()] {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut geometry = None;
        terminal
            .draw(|frame| geometry = Some(render(frame, &view)))
            .unwrap();
        let geometry = geometry.unwrap();
        assert!(!geometry.hits.is_empty());
        for hit in &geometry.hits {
            assert!(
                map_event(
                    mouse(
                        MouseEventKind::Down(MouseButton::Left),
                        hit.rect.x,
                        hit.rect.y,
                    ),
                    &view,
                    &geometry,
                )
                .is_some(),
                "unmapped hit {hit:?}"
            );
        }
    }
}
