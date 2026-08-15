use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenEvent, AddScreenGeometry, AddScreenSession, EventHandling, TuiSession,
    ViewGeometry, render_add, render_with_session,
};
use skit_ui::{
    Action, AddAction, AddEffect, AddProblem, AddStage, AddWorkflowState, KnownEntryKind,
    LibraryState, ReviewDefaults, ReviewState, Screen, SourceSnapshot,
};

fn source(path: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn review(bytes: &[u8]) -> ReviewState {
    ReviewState::from_source(
        source("review.prompt.md", bytes),
        KnownEntryKind::Prompt,
        ReviewDefaults {
            runner_names: vec!["claude".to_owned(), "codex".to_owned()],
            ..ReviewDefaults::default()
        },
    )
}

fn workflow(bytes: &[u8]) -> AddWorkflowState {
    AddWorkflowState::from_review(review(bytes))
}

fn draw_add(
    session: &mut AddScreenSession,
    view: &AddWorkflowState,
    width: u16,
    height: u16,
) -> AddScreenGeometry {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            let area = frame.area();
            geometry = render_add(frame, area, view, session, Locale::En);
        })
        .unwrap();
    geometry
}

fn draw_state(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> ViewGeometry {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    geometry
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

#[test]
fn test_review_choose_variables_key_is_harmless_for_a_short_prompt() {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow(b"{{a}} {{b}}")))));
    let mut session = TuiSession::default();
    let geometry = draw_state(&mut session, &state, 100, 40);

    let handling = session.handle_event(
        key(KeyCode::Char('o'), KeyModifiers::CONTROL),
        &state,
        &geometry,
    );
    assert_eq!(
        handling,
        EventHandling::Ignored,
        "Ctrl+O must not open the full candidate picker when the complete prompt already fits the inline preview"
    );

    // If Ctrl+O secretly opened an overlay, Esc would merely consume/cancel that overlay. With no
    // picker door on a short prompt, Esc belongs to the review and must request the normal cancel.
    let after = draw_state(&mut session, &state, 100, 40);
    assert_eq!(
        session.handle_event(key(KeyCode::Esc, KeyModifiers::NONE), &state, &after),
        EventHandling::Action(Action::Add(AddAction::Cancel))
    );
}

#[test]
fn test_review_space_untick_keeps_a_subset() {
    let mut workflow = workflow(b"{{a}} {{b}}\n");
    let mut session = AddScreenSession::default();
    let geometry = draw_add(&mut session, &workflow, 100, 40);

    for _ in 0..32 {
        if session.focused() == Some(&AddControlId::PromptCandidate("b".to_owned())) {
            break;
        }
        assert_eq!(
            session.handle_event(key(KeyCode::Tab, KeyModifiers::NONE), &workflow, &geometry),
            Some(AddScreenEvent::Changed)
        );
    }
    assert_eq!(session.focused(), Some(&AddControlId::PromptCandidate("b".to_owned())));
    let event = session
        .handle_event(key(KeyCode::Char(' '), KeyModifiers::NONE), &workflow, &geometry)
        .expect("Space on a focused prompt checkbox was ignored");
    let AddScreenEvent::Action(action) = event else {
        panic!("Space changed only terminal state instead of the prompt selection: {event:?}")
    };
    assert!(workflow.reduce(action).is_empty());
    assert_eq!(workflow.review().unwrap().selected_prompt_names(), ["a"]);

    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
        panic!("review did not commit after keyboard untick: {effects:?}")
    };
    assert_eq!(entry.settings.params, ["a"]);
}

#[test]
fn test_review_ctrl_e_rescans_and_keeps_edits() {
    let mut workflow = workflow(b"{{a}}\n");
    assert!(workflow.reduce(AddAction::SetReviewName("renamed".to_owned())).is_empty());
    let effects = workflow.reduce(AddAction::EditSource);
    let [AddEffect::EditSource { request, path }] = effects.as_slice() else {
        panic!("Ctrl+E path did not ask the host to edit exactly one source: {effects:?}")
    };
    assert_eq!(path.to_string_lossy(), "review.prompt.md");
    let request = *request;
    assert!(workflow
        .reduce(AddAction::SourceEdited {
            request,
            result: Ok(source("review.prompt.md", b"{{a}} {{b}}\n")),
        })
        .is_empty());

    let review = workflow.review().unwrap();
    assert_eq!(review.name(), "renamed", "source rescan discarded an unsaved name edit");
    assert_eq!(review.selected_prompt_names(), ["a", "b"], "source rescan did not discover/select the new placeholder");
    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
        panic!("edited review did not commit: {effects:?}")
    };
    assert_eq!(entry.name, "renamed");
    assert_eq!(entry.settings.params, ["a", "b"]);
}

#[test]
fn test_review_ctrl_e_keeps_the_runner_pick_and_reports_editor_errors() {
    let mut workflow = workflow(b"{{a}}\n");
    assert!(workflow
        .reduce(AddAction::SetPromptRunner {
            name: "codex".to_owned(),
            picked: true,
        })
        .is_empty());
    assert_eq!(workflow.review().unwrap().runner(), "codex");
    assert!(workflow.review().unwrap().runner_was_picked());

    let first = workflow.reduce(AddAction::EditSource);
    let [AddEffect::EditSource { request, .. }] = first.as_slice() else {
        panic!("first edit did not ask the host")
    };
    let request = *request;
    workflow.reduce(AddAction::SourceEdited {
        request,
        result: Ok(source("review.prompt.md", b"{{a}} {{b}}\n")),
    });
    assert_eq!(workflow.review().unwrap().runner(), "codex", "successful rescan lost the runner pick");
    assert!(workflow.review().unwrap().runner_was_picked());

    let second = workflow.reduce(AddAction::EditSource);
    let [AddEffect::EditSource { request, .. }] = second.as_slice() else {
        panic!("second edit did not ask the host")
    };
    let request = *request;
    let before = workflow.review().unwrap().source().bytes.clone();
    assert!(workflow
        .reduce(AddAction::SourceEdited {
            request,
            result: Err("no editor".to_owned()),
        })
        .is_empty());
    assert_eq!(workflow.problem(), Some(&AddProblem::EditFailed { reason: "no editor".to_owned() }));
    assert_eq!(workflow.stage(), AddStage::Review, "editor failure threw the user out of review");
    assert_eq!(workflow.review().unwrap().runner(), "codex", "editor failure cleared the runner pick");
    assert_eq!(workflow.review().unwrap().source().bytes, before, "failed editor replaced the source snapshot");
}

#[test]
fn test_review_edit_tolerates_a_placeholder_checkbox_unmounted_during_recompose() {
    let mut workflow = workflow(b"{{old}}\n");
    let mut session = AddScreenSession::default();
    let first = draw_add(&mut session, &workflow, 110, 40);
    assert!(first.hits.iter().any(|hit| hit.target == AddControlId::PromptCandidate("old".to_owned())));

    let effects = workflow.reduce(AddAction::EditSource);
    let [AddEffect::EditSource { request, .. }] = effects.as_slice() else {
        panic!("edit did not produce a host request")
    };
    let request = *request;
    workflow.reduce(AddAction::SourceEdited {
        request,
        result: Ok(source("review.prompt.md", b"{{new}}\n")),
    });

    // Reuse the exact same terminal session. This is the Ratatui equivalent of Textual having an
    // old checkbox unmounted while a queued edit/recompose completes: no stale widget/index state
    // may survive the model signature change.
    let second = draw_add(&mut session, &workflow, 110, 40);
    assert!(!second.hits.iter().any(|hit| hit.target == AddControlId::PromptCandidate("old".to_owned())));
    assert!(second.hits.iter().any(|hit| hit.target == AddControlId::PromptCandidate("new".to_owned())));
    let review = workflow.review().unwrap();
    assert_eq!(review.selected_prompt_names(), ["new"]);
    assert!(!review.prompt_is_flooded());
}
