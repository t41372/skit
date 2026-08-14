use ratatui_core::{
    backend::TestBackend,
    layout::Rect,
    style::{Color, Modifier},
    terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{Diagnostic, DiagnosticCode, LibraryScan};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, HitRegion, HitTarget, TuiSession, ViewGeometry, map_event, render,
    render_localized, render_with_session,
};
use skit_ui::{
    Action, FormField, FormPurpose, FormView, LibraryState, ReportItem, ReportView, Screen,
    UiCommand,
};

fn state() -> LibraryState {
    let mut state = LibraryState::from_scan(LibraryScan {
        entries: vec![EntrySummary {
            slug: Slug::parse("hello").unwrap(),
            name: "Hello".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            description: "A friendly script".to_owned(),
            target: None,
        }],
        diagnostics: Vec::new(),
    });
    state.update(Action::ReplaceRerunnable(vec![
        Slug::parse("hello").unwrap(),
    ]));
    state
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

fn scroll_footer_commands(
    view: &LibraryState,
    locale: Locale,
    width: u16,
    height: u16,
) -> (Vec<UiCommand>, Vec<ViewGeometry>) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut session = TuiSession::default();
    let mut commands = Vec::new();
    let mut frames = Vec::new();
    for _ in 0..32 {
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, view, locale, &mut session);
            })
            .unwrap();
        for command in geometry.hits.iter().filter_map(|hit| match hit.action {
            HitTarget::Command(command) => Some(command),
            HitTarget::RunFieldCommand { .. } | HitTarget::FocusField(_) => None,
        }) {
            if !commands.contains(&command) {
                commands.push(command);
            }
        }
        let Some(hit) = geometry
            .hits
            .iter()
            .find(|hit| matches!(hit.action, HitTarget::Command(_)))
        else {
            frames.push(geometry);
            break;
        };
        let scroll = mouse(MouseEventKind::ScrollDown, hit.rect.x, hit.rect.y);
        assert_eq!(
            session.handle_event(scroll, view, &geometry),
            EventHandling::Consumed,
            "the mature footer viewport must accept wheel scrolling"
        );
        frames.push(geometry);
    }
    (commands, frames)
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
            .any(|hit| hit.action == HitTarget::Command(UiCommand::Quit))
    );
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.action == HitTarget::Command(UiCommand::Reload))
    );
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.action == HitTarget::Command(UiCommand::Search))
    );

    for command in [
        UiCommand::Run,
        UiCommand::Rerun,
        UiCommand::Add,
        UiCommand::Edit,
        UiCommand::Settings,
        UiCommand::Presets,
        UiCommand::Rename,
        UiCommand::Remove,
        UiCommand::Preferences,
        UiCommand::Health,
        UiCommand::Runners,
        UiCommand::ToggleDetail,
        UiCommand::Help,
    ] {
        assert!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.action == HitTarget::Command(command)),
            "missing footer command {command:?}"
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
    assert!(text.contains("工 具 庫"));
    assert!(text.contains("項 目"));
    assert!(text.contains("詳 細 資 料"));
    assert!(text.contains("結 束"));
    assert!(text.contains("副 本 由  skit"), "{text:?}");
    assert!(!text.contains("Library"));

    let mut form_state = state();
    form_state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Add,
        title: "Add an entry".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: None,
        fields: vec![
            FormField::text("source", "Source path", ""),
            FormField::text_raw("raw", "Library", "value"),
            FormField::text_with_arguments("typed", "{} type", vec!["Library".to_owned()], "str"),
        ],
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
    assert!(text.contains("Library"), "{text:?}");
    assert!(!text.contains("程 式 庫 種 類"), "{text:?}");
    assert!(text.contains("Library 類 型"), "{text:?}");
    assert!(text.contains("儲 存"), "{text:?}");
}

#[test]
fn renderer_handles_narrow_empty_search_status_and_diagnostics_views() {
    let diagnostic = Diagnostic::plain(
        DiagnosticCode::CorruptMetadata,
        Some("bad".to_owned()),
        "bad TOML".to_owned(),
    );
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
        (KeyCode::Char('r'), KeyModifiers::NONE, Action::Rerun),
        (KeyCode::Char('/'), KeyModifiers::NONE, Action::BeginSearch),
        (KeyCode::Up, KeyModifiers::NONE, Action::Previous),
        (KeyCode::Char('k'), KeyModifiers::NONE, Action::Previous),
        (KeyCode::Down, KeyModifiers::NONE, Action::Next),
        (KeyCode::Char('j'), KeyModifiers::NONE, Action::Next),
        (KeyCode::PageUp, KeyModifiers::NONE, Action::PagePrevious),
        (KeyCode::PageDown, KeyModifiers::NONE, Action::PageNext),
        (KeyCode::Home, KeyModifiers::NONE, Action::Home),
        (KeyCode::End, KeyModifiers::NONE, Action::End),
    ];

    for (code, modifiers, action) in cases {
        assert_eq!(
            map_event(key(code, modifiers), &browse, &geometry),
            Some(action)
        );
    }
    assert_eq!(
        map_event(
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &browse,
            &geometry
        ),
        None,
        "the persistent session owns the two-step Ctrl+C chord"
    );
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
        (KeyCode::Char('r'), KeyModifiers::NONE, Action::Rerun),
        (KeyCode::Char('a'), KeyModifiers::NONE, Action::OpenAdd),
        (KeyCode::Char('n'), KeyModifiers::CONTROL, Action::OpenAdd),
        (KeyCode::Char('e'), KeyModifiers::NONE, Action::Edit),
        (KeyCode::Char('e'), KeyModifiers::CONTROL, Action::Edit),
        (KeyCode::Char('p'), KeyModifiers::NONE, Action::OpenSettings),
        (KeyCode::Char('s'), KeyModifiers::NONE, Action::OpenPresets),
        (KeyCode::F(2), KeyModifiers::NONE, Action::OpenRename),
        (KeyCode::Delete, KeyModifiers::NONE, Action::AskRemove),
        (KeyCode::Backspace, KeyModifiers::NONE, Action::AskRemove),
        (
            KeyCode::Char(','),
            KeyModifiers::NONE,
            Action::OpenPreferences,
        ),
        (KeyCode::Char('D'), KeyModifiers::SHIFT, Action::OpenHealth),
        (KeyCode::Char('h'), KeyModifiers::NONE, Action::OpenHealth),
        (KeyCode::Char('?'), KeyModifiers::SHIFT, Action::OpenHelp),
        (
            KeyCode::Tab,
            KeyModifiers::NONE,
            Action::ToggleDetail {
                currently_visible: false,
            },
        ),
        (KeyCode::Char('R'), KeyModifiers::SHIFT, Action::OpenRunners),
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
fn stateless_mapping_defers_search_edits_to_the_mature_session() {
    let mut searching = state();
    searching.update(Action::BeginSearch);
    let geometry = ViewGeometry::default();

    assert_eq!(
        map_event(
            key(KeyCode::Char('q'), KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        None
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('Q'), KeyModifiers::SHIFT),
            &searching,
            &geometry
        ),
        None
    );
    assert_eq!(
        map_event(
            key(KeyCode::Backspace, KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        None
    );
    assert_eq!(
        map_event(
            key(KeyCode::Enter, KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        Some(Action::OpenRun)
    );
    assert_eq!(
        map_event(key(KeyCode::Esc, KeyModifiers::NONE), &searching, &geometry),
        Some(Action::FinishSearch)
    );
    assert_eq!(
        map_event(key(KeyCode::Up, KeyModifiers::NONE), &searching, &geometry),
        Some(Action::Previous)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Down, KeyModifiers::NONE),
            &searching,
            &geometry
        ),
        Some(Action::Next)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &searching,
            &geometry
        ),
        None
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
        None,
        "the persistent session owns the two-step Ctrl+C chord"
    );
}

#[test]
fn release_focus_and_resize_events_are_ignored() {
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
        Event::Resize(80, 24),
    ];

    for event in events {
        assert_eq!(map_event(event, &state, &geometry), None);
    }
}

#[test]
fn stateless_mapping_defers_paste_to_the_mature_session() {
    let mut searching = state();
    searching.update(Action::BeginSearch);
    assert_eq!(
        map_event(
            Event::Paste("alpha".to_owned()),
            &searching,
            &ViewGeometry::default()
        ),
        None
    );
    assert_eq!(
        map_event(
            Event::Paste("alpha".to_owned()),
            &state(),
            &ViewGeometry::default()
        ),
        None
    );
}

#[test]
fn mouse_wheel_rows_and_footer_hits_map_to_frontend_neutral_actions() {
    let geometry = ViewGeometry {
        rows: Rect::new(2, 3, 30, 4),
        first_visible: 5,
        detail_pane_visible: false,
        hits: vec![
            HitRegion {
                rect: Rect::new(0, 10, 5, 1),
                action: HitTarget::Command(UiCommand::Quit),
            },
            HitRegion {
                rect: Rect::new(6, 10, 7, 1),
                action: HitTarget::Command(UiCommand::Reload),
            },
            HitRegion {
                rect: Rect::new(14, 10, 8, 1),
                action: HitTarget::Command(UiCommand::Search),
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

#[test]
fn modal_input_does_not_leak_into_the_library_workflow() {
    let mut help = state();
    help.update(Action::OpenHelp);
    let geometry = ViewGeometry {
        rows: Rect::new(0, 0, 20, 10),
        ..ViewGeometry::default()
    };

    assert_eq!(
        map_event(
            key(KeyCode::Char('a'), KeyModifiers::NONE),
            &help,
            &geometry
        ),
        None
    );
    assert_eq!(
        map_event(mouse(MouseEventKind::ScrollDown, 2, 2), &help, &geometry),
        None
    );
    assert_eq!(
        map_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 2, 2),
            &help,
            &geometry
        ),
        None
    );
}

fn form_state() -> LibraryState {
    let mut state = state();
    state.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Settings,
        title: "Script settings".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
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
            translate_label: true,
            detail: "Ready".to_owned(),
            translate_detail: true,
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
        (KeyCode::Char('s'), KeyModifiers::CONTROL, Action::Submit),
        (KeyCode::Esc, KeyModifiers::NONE, Action::Back),
        (KeyCode::Enter, KeyModifiers::NONE, Action::FocusNext),
    ];
    for (code, modifiers, action) in cases {
        assert_eq!(
            map_event(key(code, modifiers), &form, &geometry),
            Some(action)
        );
    }
    for code in [KeyCode::Backspace, KeyCode::Char('x')] {
        assert_eq!(
            map_event(key(code, KeyModifiers::NONE), &form, &geometry),
            None,
            "the persistent widget session owns text editing"
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
        None,
        "latest main requires the explicit y verb; Enter cannot remove an entry"
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('y'), KeyModifiers::NONE),
            &confirm,
            &geometry
        ),
        Some(Action::Submit)
    );
    assert_eq!(
        map_event(key(KeyCode::Esc, KeyModifiers::NONE), &confirm, &geometry),
        Some(Action::Back)
    );

    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    let mut session = TuiSession::default();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &confirm, Locale::En, &mut session);
        })
        .unwrap();
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), 51, 17),
            &confirm,
            &geometry,
        ),
        EventHandling::Action(Action::Submit),
        "the mature dialog's visible Remove button must be a positive mouse path"
    );
}

#[test]
fn every_rendered_chip_and_form_row_is_clickable() {
    let mut report = state();
    report.update(Action::Present(Screen::Report(ReportView {
        title: "Health".to_owned(),
        items: vec![],
    })));
    let mut confirmation = state();
    confirmation.update(Action::AskRemove);
    let mut searching = state();
    searching.update(Action::BeginSearch);
    let mut help = state();
    help.update(Action::OpenHelp);

    for view in [state(), searching, form_state(), report, confirmation, help] {
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

#[test]
fn contextual_footer_only_advertises_commands_that_can_run_here() {
    let mut searching = state();
    searching.update(Action::BeginSearch);
    let mut empty_search = searching.clone();
    empty_search.update(Action::Paste("no result".to_owned()));
    let empty = LibraryState::default();

    let cases = [
        (searching, vec![UiCommand::Run, UiCommand::LeaveSearch]),
        (empty_search, vec![UiCommand::LeaveSearch]),
        (
            empty,
            vec![
                UiCommand::Add,
                UiCommand::Presets,
                UiCommand::Search,
                UiCommand::ToggleDetail,
                UiCommand::Preferences,
                UiCommand::Health,
                UiCommand::Help,
                UiCommand::Runners,
                UiCommand::Reload,
                UiCommand::Quit,
            ],
        ),
    ];

    for (view, expected) in cases {
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        let mut geometry = None;
        terminal
            .draw(|frame| geometry = Some(render(frame, &view)))
            .unwrap();
        let actual = geometry
            .unwrap()
            .hits
            .iter()
            .filter_map(|hit| match hit.action {
                HitTarget::Command(command) => Some(command),
                HitTarget::RunFieldCommand { .. } | HitTarget::FocusField(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }
}

#[test]
fn help_and_detail_are_real_serializable_ui_surfaces() {
    let mut view = state();
    view.update(Action::OpenHelp);
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut geometry = None;
    terminal
        .draw(|frame| geometry = Some(render(frame, &view)))
        .unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Rerun"));
    assert!(text.contains("Detail pane"));
    assert_eq!(
        map_event(
            key(KeyCode::Esc, KeyModifiers::NONE),
            &view,
            &ViewGeometry::default()
        ),
        Some(Action::Back)
    );
    assert_eq!(
        map_event(
            key(KeyCode::Char('?'), KeyModifiers::SHIFT),
            &view,
            &ViewGeometry::default()
        ),
        Some(Action::Back)
    );
    let close = geometry
        .unwrap()
        .hits
        .into_iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::CloseModal))
        .unwrap();
    assert_eq!(
        map_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                close.rect.x,
                close.rect.y,
            ),
            &view,
            &ViewGeometry {
                hits: vec![close],
                ..ViewGeometry::default()
            },
        ),
        Some(Action::Back)
    );

    view.update(Action::Back);
    view.update(Action::ToggleDetail {
        currently_visible: true,
    });
    let mut hidden = Terminal::new(TestBackend::new(100, 24)).unwrap();
    hidden
        .draw(|frame| {
            let _ = render(frame, &view);
        })
        .unwrap();
    let hidden_text = hidden
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(!hidden_text.contains("╭ Detail pane"));
    view.update(Action::ToggleDetail {
        currently_visible: false,
    });
    let mut shown = Terminal::new(TestBackend::new(100, 24)).unwrap();
    shown
        .draw(|frame| {
            let _ = render(frame, &view);
        })
        .unwrap();
    let shown_text = shown
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(shown_text.contains("╭ Detail pane"));
}

#[test]
fn help_uses_mature_keyboard_and_mouse_scrolling_for_short_terminals() {
    let mut view = state();
    view.update(Action::OpenHelp);
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(52, 12)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &view, Locale::En, &mut session);
        })
        .unwrap();
    let before = terminal.backend().buffer().clone();

    assert_eq!(
        session.handle_event(key(KeyCode::End, KeyModifiers::NONE), &view, &geometry),
        EventHandling::Consumed
    );
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &view, Locale::En, &mut session);
        })
        .unwrap();
    let at_bottom = terminal.backend().buffer().clone();
    let text = at_bottom
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(text.contains("Ctrl+C Ctrl+C / Esc"), "{text}");
    assert!(text.contains("Quit"), "{text}");
    assert_ne!(before, at_bottom);

    assert_eq!(
        session.handle_event(mouse(MouseEventKind::ScrollUp, 20, 5), &view, &geometry),
        EventHandling::Consumed,
        "the help wheel must route through ScrollableContentState"
    );
}

#[test]
fn narrow_library_footer_scroll_reaches_every_action_and_reserves_the_status_row() {
    let mut view = state();
    view.update(Action::SetStatus("Entry added".to_owned()));
    let (commands, frames) = scroll_footer_commands(&view, Locale::En, 38, 16);

    for command in [
        UiCommand::Run,
        UiCommand::Rerun,
        UiCommand::Add,
        UiCommand::Edit,
        UiCommand::Settings,
        UiCommand::Presets,
        UiCommand::Rename,
        UiCommand::Remove,
        UiCommand::Preferences,
        UiCommand::Health,
        UiCommand::Runners,
        UiCommand::Search,
        UiCommand::ToggleDetail,
        UiCommand::Help,
        UiCommand::Reload,
        UiCommand::Quit,
    ] {
        assert!(
            commands.contains(&command),
            "missing narrow footer command {command:?}"
        );
    }

    let status_row = 14;
    for geometry in frames {
        assert!(
            geometry
                .hits
                .iter()
                .all(|hit| hit.rect.bottom() <= status_row),
            "a visible hit target overlaps the status row: {:?}",
            geometry.hits
        );
    }
}

#[test]
fn narrow_footer_scroll_reaches_all_actions_in_each_supported_locale() {
    let expected = [
        UiCommand::Run,
        UiCommand::Rerun,
        UiCommand::Add,
        UiCommand::Edit,
        UiCommand::Settings,
        UiCommand::Presets,
        UiCommand::Rename,
        UiCommand::Remove,
        UiCommand::Preferences,
        UiCommand::Health,
        UiCommand::Runners,
        UiCommand::Search,
        UiCommand::ToggleDetail,
        UiCommand::Help,
        UiCommand::Reload,
        UiCommand::Quit,
    ];
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let (commands, _) = scroll_footer_commands(&state(), locale, 38, 24);
        for command in expected {
            assert!(
                commands.contains(&command),
                "missing {command:?} for {locale:?}"
            );
        }
    }
}

#[test]
fn every_library_footer_action_has_the_expected_mouse_mapping() {
    let view = state();
    let backend = TestBackend::new(120, 30);
    let mut terminal = Terminal::new(backend).unwrap();
    let mut geometry = None;
    terminal
        .draw(|frame| geometry = Some(render(frame, &view)))
        .unwrap();
    let geometry = geometry.unwrap();
    let expected = [
        (UiCommand::Run, Action::OpenRun),
        (UiCommand::Rerun, Action::Rerun),
        (UiCommand::Add, Action::OpenAdd),
        (UiCommand::Edit, Action::Edit),
        (UiCommand::Settings, Action::OpenSettings),
        (UiCommand::Presets, Action::OpenPresets),
        (UiCommand::Rename, Action::OpenRename),
        (UiCommand::Remove, Action::AskRemove),
        (UiCommand::Preferences, Action::OpenPreferences),
        (UiCommand::Health, Action::OpenHealth),
        (UiCommand::Runners, Action::OpenRunners),
        (UiCommand::Search, Action::BeginSearch),
        (
            UiCommand::ToggleDetail,
            Action::ToggleDetail {
                currently_visible: true,
            },
        ),
        (UiCommand::Help, Action::OpenHelp),
        (UiCommand::Reload, Action::Reload),
        (UiCommand::Quit, Action::Quit),
    ];
    for (command, expected_action) in expected {
        let hit = geometry
            .hits
            .iter()
            .find(|hit| hit.action == HitTarget::Command(command))
            .unwrap();
        assert_eq!(
            map_event(
                mouse(
                    MouseEventKind::Down(MouseButton::Left),
                    hit.rect.x,
                    hit.rect.y,
                ),
                &view,
                &geometry,
            ),
            Some(expected_action),
            "incorrect mouse mapping for {command:?}"
        );
    }
}

#[test]
fn library_footer_keeps_local_and_global_pill_rows_visually_distinct() {
    let mut terminal = Terminal::new(TestBackend::new(180, 30)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_localized(frame, &state(), Locale::En);
        })
        .unwrap();

    let row = |command| {
        geometry
            .hits
            .iter()
            .find(|hit| hit.action == HitTarget::Command(command))
            .unwrap_or_else(|| panic!("missing {command:?}"))
            .rect
            .y
    };
    assert_eq!(row(UiCommand::Run), row(UiCommand::Remove));
    assert_eq!(row(UiCommand::Add), row(UiCommand::Help));
    assert!(
        row(UiCommand::Add) > row(UiCommand::Remove),
        "Library-local and global commands need separate logical rows"
    );

    let run = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::Run))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let key = &buffer[(run.rect.x.saturating_add(1), run.rect.y)];
    let label = &buffer[(run.rect.x.saturating_add(7), run.rect.y)];
    assert_eq!(key.bg, Color::Rgb(0x2a, 0x21, 0x1c));
    assert_eq!(label.bg, Color::Rgb(0x2a, 0x21, 0x1c));
    assert_eq!(key.fg, Color::Rgb(0xd9, 0x77, 0x57));
    assert!(key.modifier.contains(Modifier::BOLD));
}

#[test]
fn raw_titles_report_rows_and_remaining_screen_keys_are_explicit() {
    let geometry = ViewGeometry::default();
    let mut form = state();
    form.update(Action::Present(Screen::Form(FormView {
        purpose: FormPurpose::Add,
        title: "Raw title".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: None,
        fields: vec![FormField::multiline("body", "Body", "")],
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    for (code, action) in [
        (KeyCode::Up, Action::FocusPrevious),
        (KeyCode::Down, Action::FocusNext),
    ] {
        assert_eq!(
            map_event(key(code, KeyModifiers::NONE), &form, &geometry),
            Some(action)
        );
    }
    assert_eq!(
        map_event(key(KeyCode::Enter, KeyModifiers::NONE), &form, &geometry),
        None,
        "the mature textarea owns newline insertion"
    );

    let mut report = state();
    report.update(Action::Present(Screen::Report(ReportView {
        title: "Raw report".to_owned(),
        items: vec![ReportItem {
            status: "ok".to_owned(),
            label: "Raw label".to_owned(),
            translate_label: false,
            detail: "Raw detail".to_owned(),
            translate_detail: false,
        }],
    })));
    assert_eq!(
        map_event(key(KeyCode::Left, KeyModifiers::NONE), &report, &geometry),
        None
    );

    let mut confirm = state();
    confirm.update(Action::AskRemove);
    assert_eq!(
        map_event(key(KeyCode::Left, KeyModifiers::NONE), &confirm, &geometry),
        None
    );

    for view in [form, report] {
        let backend = TestBackend::new(60, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let _ = render(frame, &view);
            })
            .unwrap();
    }
}

#[test]
fn a_terminal_failure_localizes_its_message() {
    use skit_i18n::Localize as _;

    let error = skit_tui::TuiError::Io(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "broken pipe",
    ));
    let message = error.message();
    assert_eq!(error.to_string(), message.localize(Locale::En));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let text = message.localize(locale);
        assert!(text.contains("broken pipe"));
        assert!(!text.contains("{}"));
    }
    assert_eq!(
        message.localize(Locale::ZhCn),
        "终端输入输出失败：broken pipe"
    );
}

#[test]
fn a_terminal_that_is_too_small_still_renders_without_a_panic() {
    // A one-column terminal leaves no inner width, so the footer keeps its smallest height.
    for (width, height) in [(1_u16, 6_u16), (2, 5), (12, 5), (20, 4)] {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_localized(frame, &state(), Locale::En);
            })
            .unwrap();
        assert!(geometry.rows.width <= width);
        terminal
            .draw(|frame| {
                let _ = render(frame, &state());
            })
            .unwrap();
    }
}

#[test]
fn a_short_footer_area_stops_before_it_overflows_its_rows() {
    // Every advertised action cannot fit, so the footer stops instead of drawing outside.
    let mut wide = ViewGeometry::default();
    let mut narrow = ViewGeometry::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
    terminal
        .draw(|frame| {
            wide = render_localized(frame, &state(), Locale::En);
        })
        .unwrap();
    let mut small = Terminal::new(TestBackend::new(24, 6)).unwrap();
    small
        .draw(|frame| {
            narrow = render_localized(frame, &state(), Locale::En);
        })
        .unwrap();
    assert!(
        narrow.hits.len() < wide.hits.len(),
        "a short footer must drop chips it cannot draw"
    );
}
