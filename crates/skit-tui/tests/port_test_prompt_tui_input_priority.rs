use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{AddScreenEvent, AddScreenGeometry, AddScreenSession, render_add};
use skit_ui::{AddAction, AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn draw(session: &mut AddScreenSession, workflow: &AddWorkflowState) -> AddScreenGeometry {
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            geometry = render_add(frame, area, workflow, session, Locale::En);
        })
        .unwrap();
    geometry
}

#[test]
fn test_review_ctrl_e_in_input_is_end_of_line_not_editor() {
    let mut review = ReviewState::from_source(
        SourceSnapshot {
            path: "e.prompt.md".into(),
            source_record: "e.prompt.md".to_owned(),
            bytes: b"{{a}}\n".to_vec(),
            permissions: SourcePermissions::default(),
            is_regular: true,
            is_directory: false,
            is_draft: false,
        },
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    review.set_name("hello");
    let mut workflow = AddWorkflowState::from_review(review);
    let mut session = AddScreenSession::default();
    let geometry = draw(&mut session, &workflow);

    // ReviewName is the first mature text input. Home moves only its terminal cursor; it must not
    // alter the durable value.
    if let Some(AddScreenEvent::Action(action)) = session.handle_event(
        key(KeyCode::Home, KeyModifiers::NONE),
        &workflow,
        &geometry,
    ) {
        assert!(workflow.reduce(action).is_empty());
    }
    assert_eq!(workflow.review().unwrap().name(), "hello");

    let ctrl_e = session.handle_event(
        key(KeyCode::Char('e'), KeyModifiers::CONTROL),
        &workflow,
        &geometry,
    );
    assert!(
        !matches!(ctrl_e, Some(AddScreenEvent::Action(AddAction::EditSource))),
        "Ctrl+E escaped a focused text input and opened the editor instead of moving to end-of-line"
    );
    if let Some(AddScreenEvent::Action(action)) = ctrl_e {
        assert!(workflow.reduce(action).is_empty());
    }

    let typed = session
        .handle_event(
            key(KeyCode::Char('X'), KeyModifiers::NONE),
            &workflow,
            &geometry,
        )
        .expect("typing after Ctrl+E was ignored");
    let AddScreenEvent::Action(action) = typed else {
        panic!("typing changed only terminal state: {typed:?}")
    };
    assert!(workflow.reduce(action).is_empty());
    assert_eq!(
        workflow.review().unwrap().name(),
        "helloX",
        "Ctrl+E did not leave the cursor at the end of the focused review input"
    );
}
