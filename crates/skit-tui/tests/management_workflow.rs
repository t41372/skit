use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, HealthAction, HealthIssue, HealthIssueKind, HealthRebuildOutcome, HealthSnapshot,
    HealthView, LibraryState, MirrorHealth, ModalState, ReportItem, ReportView, RunFormView,
    RunnerEditorAction, RunnerEditorField, RunnerEditorOwner, RunnerEditorView,
    RunnerManagerAction, RunnerManagerView, RunnerRow, RunnerRowIdentity, Screen, UvHealth,
};
use std::collections::BTreeMap;

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Terminal<TestBackend>, ViewGeometry) {
    draw_sized(session, state, 88, 30)
}

fn draw_sized(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
}

#[test]
fn long_report_reaches_its_end_by_keyboard_and_wheel() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Report(ReportView {
        title: "Long report".to_owned(),
        items: (0..24)
            .map(|index| ReportItem {
                status: "ok".to_owned(),
                label: format!("Check {index}"),
                translate_label: false,
                detail: if index == 23 {
                    "REPORT-END-SENTINEL".to_owned()
                } else {
                    format!("detail {index}")
                },
                translate_detail: false,
            })
            .collect(),
    })));

    let mut keyboard = TuiSession::default();
    let (initial, geometry) = draw_sized(&mut keyboard, &state, 44, 10);
    assert!(!rendered_text(initial.backend().buffer()).contains("REPORT-END-SENTINEL"));
    assert_eq!(
        keyboard.handle_event(key(KeyCode::End, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Consumed,
        "Report End must use the report scroll owner"
    );
    let (at_end, geometry) = draw_sized(&mut keyboard, &state, 44, 10);
    let at_end_text = rendered_text(at_end.backend().buffer());
    assert!(at_end_text.contains("REPORT-END-SENTINEL"), "{at_end_text}");
    assert!(
        !geometry.rows.is_empty(),
        "Report must publish its scroll viewport"
    );
    assert!(
        at_end_text.contains('▲') || at_end_text.contains('█'),
        "Report overflow has no visible scroll affordance: {at_end_text}"
    );

    let mut wheel = TuiSession::default();
    let (_, mut geometry) = draw_sized(&mut wheel, &state, 44, 10);
    let mut consumed = false;
    let mut last = String::new();
    for _ in 0..32 {
        consumed |= wheel.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: geometry.rows.x,
                row: geometry.rows.y,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ) == EventHandling::Consumed;
        let (terminal, next) = draw_sized(&mut wheel, &state, 44, 10);
        last = rendered_text(terminal.backend().buffer());
        geometry = next;
    }
    assert!(consumed, "Report wheel input was never consumed");
    assert!(last.contains("REPORT-END-SENTINEL"), "{last}");
}

#[test]
fn narrow_health_summary_wheel_reaches_the_wrapped_sentinel() {
    let mut snapshot = health().snapshot().clone();
    snapshot.diagnostics = vec![format!(
        "{}HEALTH-END-SENTINEL",
        "a long wrapped diagnostic ".repeat(18)
    )];
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Health(Box::new(HealthView::new(
        snapshot,
    )))));
    let mut session = TuiSession::default();
    let (initial, mut geometry) = draw_sized(&mut session, &state, 24, 14);
    assert!(!rendered_text(initial.backend().buffer()).contains("HEALTH-END-SENTINEL"));

    let mut last = String::new();
    let mut consumed = false;
    for _ in 0..40 {
        consumed |= session.handle_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 3,
                row: 3,
                modifiers: KeyModifiers::NONE,
            }),
            &state,
            &geometry,
        ) == EventHandling::Consumed;
        let (terminal, next) = draw_sized(&mut session, &state, 24, 14);
        last = rendered_text(terminal.backend().buffer());
        geometry = next;
    }
    assert!(consumed, "Health summary wheel input was never consumed");
    assert!(last.contains("HEALTH-END-SENTINEL"), "{last}");
}

fn rendered_text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn cell_position(buffer: &Buffer, needle: &str) -> (u16, u16) {
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
    panic!("expected rendered text: {needle}");
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn key_with_kind(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Event {
    Event::Key(KeyEvent::new_with_kind(code, modifiers, kind))
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn health() -> HealthView {
    HealthView::new(HealthSnapshot {
        uv: UvHealth::NotRequired,
        entry_count: 2,
        issues: vec![HealthIssue {
            slug: "missing".to_owned(),
            name: "Missing".to_owned(),
            kind: HealthIssueKind::MissingTarget,
        }],
        invalid_runner_rows: Vec::new(),
        mirror: MirrorHealth::Off,
        library_path: "/data/scripts".to_owned(),
        library_size: "2 KiB".to_owned(),
        diagnostics: Vec::new(),
    })
}

fn runner(name: &str) -> RunnerRow {
    let identity = RunnerRowIdentity {
        index: Some(0),
        snapshot_token: "snapshot".to_owned(),
    };
    RunnerRow {
        identity: identity.clone(),
        name: Some(name.to_owned()),
        argv: Some(vec!["agent".to_owned(), "{{prompt}}".to_owned()]),
        reason: None,
        descriptor: name.to_owned(),
        key_identities: vec![identity],
        pinned_count: 0,
    }
}

#[test]
fn health_uses_singular_copy_only_for_exactly_one_entry() {
    for (count, registered, rebuilt) in [
        (1, "1 entry registered", "Index rebuilt: 1 entry"),
        (2, "2 entries registered", "Index rebuilt: 2 entries"),
    ] {
        let mut snapshot = health().snapshot().clone();
        snapshot.entry_count = count;
        let mut view = HealthView::new(snapshot.clone());
        view.reduce(HealthAction::Rebuilt {
            snapshot: Box::new(snapshot),
            outcome: HealthRebuildOutcome {
                entry_count: count,
                problems: Vec::new(),
            },
        });
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Health(Box::new(view))));
        let mut session = TuiSession::default();
        let (terminal, _) = draw(&mut session, &state);
        let screen = rendered_text(terminal.backend().buffer());
        assert!(screen.contains(registered), "{screen}");
        assert!(screen.contains(rebuilt), "{screen}");
    }
}

#[test]
fn health_screen_routes_every_advertised_keyboard_and_mouse_action() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Health(Box::new(health()))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let buffer = terminal.backend().buffer();
    let rendered = rendered_text(buffer);
    assert!(rendered.contains("Health check"));
    assert!(rendered.contains("Enter Jump to entry"));
    assert!(rendered.contains("Ctrl+R Rebuild index"));
    assert!(rendered.contains("Esc Back"));
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Health(HealthAction::Rebuild))
    );
    for event in [
        key(KeyCode::Char('r'), KeyModifiers::NONE),
        key(KeyCode::Char('x'), KeyModifiers::CONTROL),
        key_with_kind(
            KeyCode::Char('r'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        ),
    ] {
        assert_eq!(
            session.handle_event(event, &state, &geometry),
            EventHandling::Ignored,
            "only a pressed Ctrl+R may rebuild Health"
        );
    }
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Action(Action::Health(HealthAction::Jump))
    );
    let (x, y) = cell_position(buffer, "Ctrl+R Rebuild index");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), x, y),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), x, y),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Health(HealthAction::Rebuild))
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Null, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Ignored
    );
    let (issue_x, issue_y) = cell_position(buffer, "Missing");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Moved, issue_x, issue_y),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "pointer motion over an issue must not move the Health selection"
    );

    let mut empty = health();
    empty = HealthView::new(HealthSnapshot {
        issues: Vec::new(),
        ..empty.snapshot().clone()
    });
    state.update(Action::Present(Screen::Health(Box::new(empty))));
    let (_, geometry) = draw(&mut session, &state);
    for code in [
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
    ] {
        assert_eq!(
            session.handle_event(key(code, KeyModifiers::NONE), &state, &geometry),
            EventHandling::Consumed,
            "Health without issues must keep {code:?} in the summary viewport"
        );
    }
}

#[test]
fn unpinned_runner_removal_does_not_fabricate_a_pin_warning() {
    let mut manager = RunnerManagerView::new(vec![runner("codex")]);
    manager.reduce(RunnerManagerAction::RemoveSelected);
    assert!(manager.removal().is_some());
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Runners(Box::new(manager))));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state);
    let screen = rendered_text(terminal.backend().buffer());
    assert!(screen.contains("Confirm removal"), "{screen}");
    assert!(
        !screen.contains("0 prompt") && !screen.contains("prompts pin"),
        "an unpinned runner fabricated a dependency warning: {screen}"
    );
}

#[test]
fn runner_manager_and_shared_modal_use_typed_mature_widget_sessions() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![runner("codex")]),
    ))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let rendered = rendered_text(terminal.backend().buffer());
    assert!(rendered.contains("Agents (prompt runners)"));
    assert!(rendered.contains("Ctrl+N New agent"));
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('n'), KeyModifiers::CONTROL),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Runners(RunnerManagerAction::New))
    );
    for event in [
        key(KeyCode::Char('n'), KeyModifiers::NONE),
        key(KeyCode::Char('x'), KeyModifiers::CONTROL),
        key_with_kind(
            KeyCode::Char('n'),
            KeyModifiers::CONTROL,
            KeyEventKind::Release,
        ),
    ] {
        assert_eq!(
            session.handle_event(event, &state, &geometry),
            EventHandling::Ignored,
            "only a pressed Ctrl+N may open a new runner"
        );
    }
    let (row_x, row_y) = cell_position(terminal.backend().buffer(), "codex");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Moved, row_x, row_y),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "pointer motion over a runner must not move the selection"
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::ScrollDown, row_x, row_y),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Runners(RunnerManagerAction::Next))
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Null, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Ignored
    );
    state.update(Action::Runners(RunnerManagerAction::New));
    let (_, geometry) = draw(&mut session, &state);
    state.update(Action::Runners(RunnerManagerAction::Editor(
        RunnerEditorAction::SetName("x".to_owned()),
    )));
    assert_eq!(
        session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Consumed
    );
    state.update(Action::Runners(RunnerManagerAction::CancelEditor));

    let run = RunFormView::from_declarations(
        "prompt",
        "Prompt",
        &[],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    );
    state.update(Action::Present(Screen::Run(Box::new(run))));
    state.update(Action::OpenRunRunnerEditor);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor {
            owner: RunnerEditorOwner::Run { selector },
            ..
        }) if selector == "prompt"
    ));
    let (terminal, geometry) = draw(&mut session, &state);
    let buffer = terminal.backend().buffer();
    let rendered = rendered_text(buffer);
    assert!(rendered.contains("New agent (runner)"));
    assert!(rendered.contains("Tab/↓ Next field"));
    assert!(rendered.contains("Enter Save"));
    let mut narrow_session = TuiSession::default();
    let (narrow_terminal, narrow_geometry) = draw_sized(&mut narrow_session, &state, 24, 18);
    let (footer_x, footer_y) = cell_position(narrow_terminal.backend().buffer(), "Tab/↓");
    assert_eq!(
        narrow_session.handle_event(
            mouse(MouseEventKind::ScrollDown, footer_x, footer_y),
            &state,
            &narrow_geometry,
        ),
        EventHandling::Consumed,
        "the narrow RunnerEditor footer must own its wheel input"
    );
    let (_, name_label_row) = cell_position(buffer, "Name, e.g. aider");
    let (command_x, command_label_row) = cell_position(buffer, "Command, e.g. aider");
    assert_eq!(
        terminal.backend().cursor_position().y,
        name_label_row.saturating_add(1),
        "the Name-focused editor placed its caret in the wrong input"
    );
    assert_ne!(
        terminal.backend().cursor_position().y,
        command_label_row.saturating_add(1),
        "the unfocused Command input also placed a caret"
    );
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                command_x,
                command_label_row
            ),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                command_x,
                command_label_row
            ),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::RunnerEditor(RunnerEditorAction::Focus(
            RunnerEditorField::Command,
        )))
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Moved, command_x, command_label_row),
            &state,
            &geometry,
        ),
        EventHandling::Ignored
    );
    assert_eq!(
        session.handle_event(
            key_with_kind(KeyCode::Esc, KeyModifiers::NONE, KeyEventKind::Release),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a released RunnerEditor key must not activate its action"
    );
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('界'), KeyModifiers::NONE),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::RunnerEditor(RunnerEditorAction::SetName(
            "界".to_owned(),
        )))
    );
    state.update(Action::RunnerEditor(RunnerEditorAction::SetName(
        "界".to_owned(),
    )));
    assert_eq!(
        session.handle_event(key(KeyCode::Left, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Null, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Ignored
    );
    let (x, y) = cell_position(buffer, "Esc Cancel");
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), x, y),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), x, y),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::RunnerEditor(RunnerEditorAction::Cancel))
    );

    let mut serialized = serde_json::to_value(&state).unwrap();
    serialized["modal"]["runner_editor"]["view"] =
        serde_json::to_value(RunnerEditorView::edit(&runner("codex"))).unwrap();
    let editing: LibraryState = serde_json::from_value(serialized).unwrap();
    let (terminal, _) = draw(&mut session, &editing);
    assert!(rendered_text(terminal.backend().buffer()).contains("Edit agent (runner)"));
}
