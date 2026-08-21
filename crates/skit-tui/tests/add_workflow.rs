use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddWorkflowState, KnownEntryKind, LibraryState, ReviewDefaults, ReviewState,
    Screen, SourceSnapshot,
};

fn draw(session: &mut TuiSession, state: &LibraryState) -> (Terminal<TestBackend>, ViewGeometry) {
    draw_sized(session, state, 80, 34)
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

fn text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn row_containing(buffer: &Buffer, needle: &str) -> u16 {
    (0..buffer.area.height)
        .find(|row| {
            (0..buffer.area.width)
                .map(|column| buffer[(column, *row)].symbol())
                .collect::<String>()
                .contains(needle)
        })
        .expect("expected rendered row")
}

fn position_of(buffer: &Buffer, needle: &str) -> (u16, u16) {
    for row in 0..buffer.area.height {
        let line = (0..buffer.area.width)
            .map(|column| buffer[(column, row)].symbol())
            .collect::<String>();
        if let Some(column) = line.find(needle) {
            return (u16::try_from(column).unwrap(), row);
        }
    }
    panic!("expected rendered text: {needle}");
}

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn typed_add_screen_uses_mature_input_and_mouse_opened_file_explorer() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &state);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Add an entry"));
    assert!(rendered.contains("Path to a script, executable, or prompt:"));
    assert!(rendered.contains("Write a script…"));
    assert!(terminal.backend().cursor_position().y > 2);

    let handling = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('界'), KeyModifiers::NONE)),
        &state,
        &geometry,
    );
    assert_eq!(
        handling,
        EventHandling::Action(Action::Add(AddAction::SetSourcePath("界".to_owned())))
    );
    if let EventHandling::Action(action) = handling {
        state.update(action);
    }

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );

    let (terminal, geometry) = draw(&mut session, &state);
    assert!(
        text(terminal.backend().buffer()).contains("[Ctrl+O] Select"),
        "the visible Browse button must advertise its independent keyboard path"
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (terminal, geometry) = draw(&mut session, &state);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Source path"), "{rendered}");
    assert!(rendered.contains("Search"), "{rendered}");
    assert!(rendered.contains("Cancel"), "{rendered}");
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );

    let (terminal, geometry) = draw(&mut session, &state);
    assert!(
        text(terminal.backend().buffer()).contains("[Ctrl+O] Select"),
        "Esc must return from the picker to Add Source"
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Add(AddAction::Continue))
    );
    let select_row = row_containing(terminal.backend().buffer(), "Select");
    assert_eq!(
        session.handle_event(mouse(3, select_row), &state, &geometry),
        EventHandling::Consumed
    );

    let (terminal, geometry) = draw(&mut session, &state);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Source path"));
    assert!(rendered.contains("Search"));
    assert!(rendered.contains("Cancel"));
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );

    let (terminal, _) = draw(&mut session, &state);
    assert!(text(terminal.backend().buffer()).contains("Add an entry"));

    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (terminal, geometry) = draw(&mut session, &state);
    let (cancel_x, cancel_y) = position_of(terminal.backend().buffer(), "[Esc] Cancel");
    assert_eq!(
        session.handle_event(mouse(cancel_x, cancel_y), &state, &geometry),
        EventHandling::Consumed
    );
    let (terminal, _) = draw(&mut session, &state);
    assert!(text(terminal.backend().buffer()).contains("Add an entry"));
}

#[test]
fn add_file_overlay_accepts_a_real_path_and_ignores_non_widget_events() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        session.handle_event(Event::FocusGained, &state, &geometry),
        EventHandling::Ignored
    );
    for character in "Cargo.toml".chars() {
        let _ = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)),
            &state,
            &geometry,
        );
    }
    let (_, geometry) = draw(&mut session, &state);
    assert!(matches!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Add(AddAction::SetSourcePath(path)))
            if path.ends_with("Cargo.toml")
    ));
}

#[test]
fn add_prompt_review_routes_the_complete_picker_and_runner_editor_seam() {
    // Ctrl+O opens the searchable candidate picker only when the detected list is capped
    // (more placeholders than the inline preview shows), so this seam needs a capped prompt:
    // 21 holes exceeds the preview limit (20) but stays under the auto-manage flood limit (30),
    // so every candidate ticks on by default and a Space untick narrows the set. A taller
    // backend keeps the runner section on screen past the 20-row preview.
    let body = (0..21)
        .map(|index| format!("{{{{h{index:02}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = ReviewState::from_source(
        SourceSnapshot {
            path: "task.prompt.md".into(),
            source_record: "task.prompt.md".to_owned(),
            bytes: body.into_bytes(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw_sized(&mut session, &state, 80, 60);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Prompt runner"));
    assert!(rendered.contains("Add Runner"));

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let (terminal, geometry) = draw_sized(&mut session, &state, 80, 60);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Choose prompt variables"));
    assert!(rendered.contains("Select all variables"));

    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    // Space unticked the focused first row (h00); Ctrl+S commits the rest in source order.
    let expected = (1..21)
        .map(|index| format!("h{index:02}"))
        .collect::<Vec<_>>();
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::Add(AddAction::SetPromptCandidates(expected)))
    );

    let (terminal, geometry) = draw_sized(&mut session, &state, 80, 60);
    let (runner_x, runner_y) = position_of(terminal.backend().buffer(), "Add Runner");
    assert_eq!(
        session.handle_event(mouse(runner_x, runner_y), &state, &geometry),
        EventHandling::Action(Action::OpenAddRunnerEditor)
    );
}
