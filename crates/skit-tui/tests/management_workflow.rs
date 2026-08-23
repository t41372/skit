use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, HealthAction, HealthIssue, HealthIssueKind, HealthSnapshot, HealthView, LibraryState,
    MirrorHealth, ModalState, RunFormView, RunnerEditorAction, RunnerEditorOwner, RunnerEditorView,
    RunnerManagerAction, RunnerManagerView, RunnerRow, RunnerRowIdentity, Screen, UvHealth,
};
use std::collections::BTreeMap;

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(88, 30)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    (terminal, geometry)
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

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
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
    assert_eq!(
        session.handle_event(key(KeyCode::Enter, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Action(Action::Health(HealthAction::Jump))
    );
    let (x, y) = cell_position(buffer, "Ctrl+R Rebuild index");
    assert_eq!(
        session.handle_event(mouse(x, y), &state, &geometry),
        EventHandling::Action(Action::Health(HealthAction::Rebuild))
    );
    assert_eq!(
        session.handle_event(key(KeyCode::Null, KeyModifiers::NONE), &state, &geometry),
        EventHandling::Ignored
    );

    let mut empty = health();
    empty = HealthView::new(HealthSnapshot {
        issues: Vec::new(),
        ..empty.snapshot().clone()
    });
    state.update(Action::Present(Screen::Health(Box::new(empty))));
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            key(KeyCode::PageDown, KeyModifiers::NONE),
            &state,
            &geometry
        ),
        EventHandling::Consumed
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
        session.handle_event(mouse(x, y), &state, &geometry),
        EventHandling::Action(Action::RunnerEditor(RunnerEditorAction::Cancel))
    );

    let mut serialized = serde_json::to_value(&state).unwrap();
    serialized["modal"]["runner_editor"]["view"] =
        serde_json::to_value(RunnerEditorView::edit(&runner("codex"))).unwrap();
    let editing: LibraryState = serde_json::from_value(serialized).unwrap();
    let (terminal, _) = draw(&mut session, &editing);
    assert!(rendered_text(terminal.backend().buffer()).contains("Edit agent (runner)"));
}
