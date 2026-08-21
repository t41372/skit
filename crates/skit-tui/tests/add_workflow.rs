use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{AddControlId, AddScreenEvent, AddScreenSession, render_add};
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddEffect, AddWorkflowState, DraftSummary, KnownEntryKind, LibraryState,
    ReviewDefaults, ReviewState, Screen, SourceSnapshot,
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

fn scroll_down(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn advertised_key(buffer: &Buffer, area: ratatui_core::layout::Rect) -> Event {
    let rendered = (area.x..area.right())
        .map(|column| buffer[(column, area.y)].symbol())
        .collect::<String>();
    let hint = rendered
        .strip_prefix('[')
        .and_then(|tail| tail.split_once(']'))
        .map(|(hint, _)| hint)
        .unwrap_or_else(|| panic!("footer hit has no advertised key: {rendered:?}"));
    let (code, modifiers) = match hint {
        "Enter" => (KeyCode::Enter, KeyModifiers::NONE),
        "Esc" => (KeyCode::Esc, KeyModifiers::NONE),
        "Space" => (KeyCode::Char(' '), KeyModifiers::NONE),
        "Tab/↓" => (KeyCode::Tab, KeyModifiers::NONE),
        "Shift+Tab/↑" => (KeyCode::BackTab, KeyModifiers::SHIFT),
        "Ctrl+N" => (KeyCode::Char('n'), KeyModifiers::CONTROL),
        "Ctrl+P" => (KeyCode::Char('p'), KeyModifiers::CONTROL),
        "Ctrl+D" => (KeyCode::Char('d'), KeyModifiers::CONTROL),
        "Ctrl+E" => (KeyCode::Char('e'), KeyModifiers::CONTROL),
        "Ctrl+S" => (KeyCode::Char('s'), KeyModifiers::CONTROL),
        _ => panic!("unsupported advertised Add key: {hint}"),
    };
    Event::Key(KeyEvent::new(code, modifiers))
}

fn assert_typed_add_event(
    target: &AddControlId,
    handling: Option<AddScreenEvent>,
    workflow: &AddWorkflowState,
) {
    let matches_target = match (target, &handling) {
        (AddControlId::Continue, Some(AddScreenEvent::Action(AddAction::Continue)))
        | (AddControlId::NewScript, Some(AddScreenEvent::Action(AddAction::NewDraft(_))))
        | (AddControlId::NewPrompt, Some(AddScreenEvent::Action(AddAction::NewDraft(_))))
        | (AddControlId::Save, Some(AddScreenEvent::Action(AddAction::Save)))
        | (AddControlId::EditSource, Some(AddScreenEvent::Action(AddAction::EditSource)))
        | (AddControlId::ToggleFocused, Some(AddScreenEvent::Action(_)))
        | (AddControlId::NextField | AddControlId::PreviousField, Some(AddScreenEvent::Changed))
        | (
            AddControlId::PickFocusedKind,
            Some(AddScreenEvent::Action(AddAction::PickKind(Some(_)))),
        ) => true,
        (AddControlId::DeleteDraft, Some(AddScreenEvent::Action(action))) => matches!(
            (workflow.stage(), action),
            (skit_ui::AddStage::Source, AddAction::DeleteSelectedDraft)
                | (
                    skit_ui::AddStage::ConfirmDraftDelete,
                    AddAction::ConfirmDraftDelete(true)
                )
        ),
        (AddControlId::Cancel, Some(AddScreenEvent::Action(action))) => match workflow.stage() {
            skit_ui::AddStage::Kind => matches!(action, AddAction::PickKind(None)),
            skit_ui::AddStage::ConfirmDraftDelete => {
                matches!(action, AddAction::ConfirmDraftDelete(false))
            }
            _ => matches!(action, AddAction::Cancel),
        },
        _ => false,
    };
    assert!(
        matches_target,
        "advertised {target:?} returned {handling:?} at stage {:?}",
        workflow.stage()
    );
}

#[test]
fn every_advertised_add_action_is_scrollable_and_clickable_at_every_size_tier() {
    let source = AddWorkflowState::new(Vec::new());
    let mut kind = AddWorkflowState::new(Vec::new());
    let _ = kind.reduce(AddAction::SetSourcePath("tool.unknown".into()));
    let request = kind
        .reduce(AddAction::Continue)
        .into_iter()
        .find_map(|effect| match effect {
            AddEffect::InspectSource { request, .. } => Some(request),
            _ => None,
        })
        .unwrap();
    let _ = kind.reduce(AddAction::SourceInspected {
        request,
        result: Ok(SourceSnapshot {
            path: "tool.unknown".into(),
            source_record: "tool.unknown".into(),
            bytes: b"unknown body\n".to_vec(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        }),
    });
    let review = AddWorkflowState::from_review(ReviewState::from_source(
        SourceSnapshot {
            path: "tool.py".into(),
            source_record: "tool.py".into(),
            bytes: b"NAME = 'World'\nprint(NAME)\n".to_vec(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        },
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    ));
    let mut confirm = AddWorkflowState::new(vec![DraftSummary {
        path: "draft.py".into(),
        modified: 1,
        identity: None,
        permissions: SourcePermissions::default(),
    }]);
    let _ = confirm.reduce(AddAction::SelectDraft(0));
    let _ = confirm.reduce(AddAction::DeleteSelectedDraft);

    for workflow in [source, kind, review, confirm] {
        let mut wide_session = AddScreenSession::default();
        let mut wide = Terminal::new(TestBackend::new(200, 40)).unwrap();
        let mut wide_geometry = Default::default();
        wide.draw(|frame| {
            wide_geometry = render_add(
                frame,
                frame.area(),
                &workflow,
                &mut wide_session,
                Locale::En,
            );
        })
        .unwrap();
        let expected = wide_geometry
            .hits
            .iter()
            .filter(|hit| hit.area.y >= 38)
            .map(|hit| hit.target.clone())
            .collect::<Vec<AddControlId>>();
        assert!(!expected.is_empty(), "stage={:?}", workflow.stage());

        for (width, height) in [(120_u16, 30_u16), (46, 9), (24, 3)] {
            let footer_y = height.saturating_sub(if height < 14 { 1 } else { 2 });
            let mut session = AddScreenSession::default();
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut seen = Vec::new();
            for _ in 0..32 {
                let mut geometry = Default::default();
                terminal
                    .draw(|frame| {
                        geometry =
                            render_add(frame, frame.area(), &workflow, &mut session, Locale::En);
                    })
                    .unwrap();
                let footer_hits = geometry
                    .hits
                    .iter()
                    .filter(|hit| hit.area.y >= footer_y)
                    .cloned()
                    .collect::<Vec<_>>();
                for hit in footer_hits {
                    if expected.contains(&hit.target) && !seen.contains(&hit.target) {
                        seen.push(hit.target.clone());
                        if hit.target == AddControlId::ToggleFocused {
                            for _ in 0..64 {
                                if matches!(
                                    session.focused(),
                                    Some(
                                        AddControlId::Candidate(_)
                                            | AddControlId::PromptCandidate(_)
                                            | AddControlId::Interpolate
                                    )
                                ) {
                                    break;
                                }
                                assert_eq!(
                                    session.handle_event(
                                        Event::Key(
                                            KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE,)
                                        ),
                                        &workflow,
                                        &geometry,
                                    ),
                                    Some(AddScreenEvent::Changed)
                                );
                            }
                            assert!(
                                matches!(
                                    session.focused(),
                                    Some(
                                        AddControlId::Candidate(_)
                                            | AddControlId::PromptCandidate(_)
                                            | AddControlId::Interpolate
                                    )
                                ),
                                "Toggle focused needs a checkbox owner"
                            );
                        }
                        let key_handling = session.handle_event(
                            advertised_key(terminal.backend().buffer(), hit.area),
                            &workflow,
                            &geometry,
                        );
                        assert_typed_add_event(&hit.target, key_handling, &workflow);
                        let mouse_handling = session.handle_event(
                            mouse(hit.area.x, hit.area.y),
                            &workflow,
                            &geometry,
                        );
                        assert_typed_add_event(&hit.target, mouse_handling, &workflow);
                    }
                }
                if seen.len() == expected.len() {
                    break;
                }
                assert_eq!(
                    session.handle_event(
                        scroll_down(1, height.saturating_sub(1)),
                        &workflow,
                        &geometry,
                    ),
                    Some(AddScreenEvent::Changed)
                );
            }
            assert_eq!(
                seen,
                expected,
                "stage={:?} size={width}x{height}",
                workflow.stage()
            );
        }
    }
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
