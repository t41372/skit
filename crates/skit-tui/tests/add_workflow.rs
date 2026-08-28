use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_i18n::Locale;
use skit_tui::{AddControlId, AddScreenEvent, AddScreenSession, render_add};
use skit_tui::{EventHandling, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddEffect, AddWorkflowState, DraftSummary, KnownEntryKind, LibraryState,
    PROMPT_LIST_PREVIEW_LIMIT, ReviewDefaults, ReviewState, Screen, SourceSnapshot,
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
    mouse_with_kind(MouseEventKind::Down(MouseButton::Left), column, row)
}

fn mouse_with_kind(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn add_click(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
    geometry: &skit_tui::AddScreenGeometry,
    column: u16,
    row: u16,
) -> Option<AddScreenEvent> {
    assert_eq!(
        session.handle_event(mouse(column, row), state, geometry),
        Some(AddScreenEvent::Changed)
    );
    session.handle_event(
        mouse_with_kind(MouseEventKind::Up(MouseButton::Left), column, row),
        state,
        geometry,
    )
}

fn session_click(
    session: &mut TuiSession,
    state: &LibraryState,
    geometry: &ViewGeometry,
    column: u16,
    row: u16,
) -> EventHandling {
    assert_eq!(
        session.handle_event(mouse(column, row), state, geometry),
        EventHandling::Consumed
    );
    session.handle_event(
        mouse_with_kind(MouseEventKind::Up(MouseButton::Left), column, row),
        state,
        geometry,
    )
}

fn snapshot(path: &str, bytes: &[u8], is_draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        executable: None,
        is_regular: true,
        is_directory: false,
        is_draft,
        identity: None,
    }
}

fn review(
    path: &str,
    bytes: &[u8],
    kind: KnownEntryKind,
    is_draft: bool,
    defaults: ReviewDefaults,
) -> AddWorkflowState {
    AddWorkflowState::from_review(ReviewState::from_source(
        snapshot(path, bytes, is_draft),
        kind,
        defaults,
    ))
}

fn draw_add(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
) -> (Terminal<TestBackend>, skit_tui::AddScreenGeometry) {
    draw_add_sized(session, state, 100, 60)
}

fn draw_add_sized(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, skit_tui::AddScreenGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = skit_tui::AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), state, session, Locale::En);
        })
        .unwrap();
    (terminal, geometry)
}

fn assert_focus_cycle_has_only_visible_controls(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
) -> Vec<AddControlId> {
    let (_, mut geometry) = draw_add(session, state);
    let first = session
        .focused()
        .cloned()
        .expect("the rendered Add stage must own a focus target");
    let mut visited = Vec::new();
    let attempts = geometry.hits.len().saturating_add(1);
    for _ in 0..attempts {
        let focused = session
            .focused()
            .cloned()
            .expect("Tab left the rendered Add stage without focus");
        assert!(
            geometry.hits.iter().any(|hit| hit.target == focused),
            "focus reached a control with no rendered pointer twin: {focused:?}; stage={:?}, review={:?}",
            state.stage(),
            state
                .review()
                .map(|review| (review.kind(), review.is_fresh(), review.storage()))
        );
        if !visited.contains(&focused) {
            visited.push(focused);
        }
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
                state,
                &geometry,
            ),
            Some(AddScreenEvent::Changed)
        );
        (_, geometry) = draw_add(session, state);
        if session.focused() == Some(&first) {
            return visited;
        }
    }
    panic!("Tab did not complete one bounded Add focus traversal");
}

fn prompt_review(
    candidate_count: usize,
    interpolate: bool,
    defaults: ReviewDefaults,
) -> AddWorkflowState {
    let body = (0..candidate_count)
        .map(|index| format!("{{{{field{index:02}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    review(
        "task.prompt.md",
        body.as_bytes(),
        KnownEntryKind::Prompt,
        false,
        ReviewDefaults {
            interpolate: Some(interpolate),
            ..defaults
        },
    )
}

fn tab_until_add_focus(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
    geometry: &skit_tui::AddScreenGeometry,
    target: &AddControlId,
) {
    for _ in 0..32 {
        if session.focused() == Some(target) {
            return;
        }
        let _ = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            state,
            geometry,
        );
    }
    assert_eq!(
        session.focused(),
        Some(target),
        "focus did not reach {target:?} in one bounded traversal"
    );
}

fn ambiguous_kind(path: &str) -> AddWorkflowState {
    let mut state = AddWorkflowState::new(Vec::new());
    let _ = state.reduce(AddAction::SetSourcePath(path.to_owned()));
    let request = state
        .reduce(AddAction::Continue)
        .into_iter()
        .find_map(|effect| match effect {
            AddEffect::InspectSource { request, .. } => Some(request),
            _ => None,
        })
        .expect("source inspection request");
    let _ = state.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot(path, b"plain body\n", false)),
    });
    assert!(
        state.kind_picker().is_some(),
        "the source must be ambiguous"
    );
    state
}

#[test]
fn add_session_syncs_external_source_and_review_values_without_resetting_equal_input() {
    let mut source = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let (_, initial_geometry) = draw_add(&mut session, &source);
    assert_eq!(
        session.focused(),
        Some(&AddControlId::Text(skit_tui::AddTextField::SourcePath))
    );
    assert!(
        initial_geometry
            .hits
            .iter()
            .any(|hit| { hit.target == AddControlId::Text(skit_tui::AddTextField::SourcePath) })
    );

    let _ = source.reduce(AddAction::SetSourcePath("picked/external.js".to_owned()));
    let _ = source.reduce(AddAction::SetCommandTemplate("node {file}".to_owned()));
    let _ = source.reduce(AddAction::SetCommandName("external-name".to_owned()));
    let _ = source.reduce(AddAction::SetCommandDescription(
        "external-description".to_owned(),
    ));
    let (terminal, geometry) = draw_add(&mut session, &source);
    let rendered = text(terminal.backend().buffer());
    for value in [
        "picked/external.js",
        "node {file}",
        "external-name",
        "external-description",
    ] {
        assert!(
            rendered.contains(value),
            "stale source input {value:?}:\n{rendered}"
        );
    }
    assert!(geometry.hits.iter().any(|hit| {
        hit.target == AddControlId::Text(skit_tui::AddTextField::CommandDescription)
    }));

    let mut javascript_review = review(
        "tool.js",
        b"import chalk from 'chalk';\nconsole.log(chalk);\n",
        KnownEntryKind::JavaScript,
        false,
        ReviewDefaults::default(),
    );
    let (_, geometry) = draw_add(&mut session, &javascript_review);
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| { session.focused() == Some(&hit.target) }),
        "a stage change left focus outside the visible control geometry"
    );

    let _ = javascript_review.reduce(AddAction::SetReviewName(
        "renamed-outside-the-widget".to_owned(),
    ));
    let _ = javascript_review.reduce(AddAction::SetReviewDescription(
        "description-from-the-reducer".to_owned(),
    ));
    let _ = javascript_review.reduce(AddAction::SetReviewDependencies("chalk@5".to_owned()));
    let (terminal, _) = draw_add(&mut session, &javascript_review);
    let rendered = text(terminal.backend().buffer());
    for value in [
        "renamed-outside-the-widget",
        "description-from-the-reducer",
        "chalk@5",
    ] {
        assert!(
            rendered.contains(value),
            "stale review input {value:?}:\n{rendered}"
        );
    }

    let mut python = review(
        "tool.py",
        b"print('ok')\n",
        KnownEntryKind::Python,
        false,
        ReviewDefaults::default(),
    );
    let _ = draw_add(&mut session, &python);
    let _ = python.reduce(AddAction::SetReviewPython(">=3.12".to_owned()));
    let (terminal, geometry) = draw_add(&mut session, &python);
    assert!(
        text(terminal.backend().buffer()).contains(">=3.12"),
        "the external Python constraint did not replace the widget value"
    );
    assert!(
        geometry.hits.iter().any(|hit| {
            hit.target == AddControlId::Text(skit_tui::AddTextField::PythonConstraint)
        })
    );
}

#[test]
fn add_equal_state_rerender_preserves_the_clicked_input_cursor() {
    let mut state = AddWorkflowState::new(Vec::new());
    let _ = state.reduce(AddAction::SetSourcePath("abcdef".to_owned()));
    let mut session = AddScreenSession::default();
    let (_, geometry) = draw_add(&mut session, &state);
    let source = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Text(skit_tui::AddTextField::SourcePath))
        .expect("source input hit")
        .area;
    assert_eq!(
        add_click(
            &mut session,
            &state,
            &geometry,
            source.x.saturating_add(3),
            source.y.saturating_add(1),
        ),
        Some(AddScreenEvent::Changed)
    );

    let (_, rerendered_geometry) = draw_add(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE)),
            &state,
            &rerendered_geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::SetSourcePath(
            "abXcdef".to_owned()
        )))
    );
}

#[test]
fn add_sync_rebuilds_exact_source_kind_and_storage_control_shapes() {
    let mut session = AddScreenSession::default();

    let empty_source = AddWorkflowState::new(Vec::new());
    let (_, empty_geometry) = draw_add(&mut session, &empty_source);
    assert!(
        empty_geometry
            .hits
            .iter()
            .all(|hit| hit.target != AddControlId::DeleteDraft),
        "an empty draft inventory exposed Delete"
    );
    assert_focus_cycle_has_only_visible_controls(&mut session, &empty_source);

    let draft_source = AddWorkflowState::new(vec![DraftSummary {
        path: "kept.py".into(),
        modified: 1,
        identity: None,
        permissions: SourcePermissions::default(),
        content_hash: None,
    }]);
    let (_, draft_geometry) = draw_add(&mut session, &draft_source);
    for target in [AddControlId::Draft(0), AddControlId::DeleteDraft] {
        assert!(
            draft_geometry.hits.iter().any(|hit| hit.target == target),
            "a nonempty draft inventory lost {target:?}"
        );
    }

    let kind = ambiguous_kind("likely.md");
    let suggested = kind
        .kind_picker()
        .and_then(|picker| picker.suggested())
        .expect("likely.md must have a suggested kind");
    let (_, kind_geometry) = draw_add(&mut session, &kind);
    assert!(
        kind_geometry
            .hits
            .iter()
            .any(|hit| { session.focused() == Some(&hit.target) }),
        "kind focus is not a visible typed hit"
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &kind,
            &kind_geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::PickKind(Some(suggested))))
    );

    for (label, state, storage_visible, edit_visible) in [
        (
            "fresh script",
            review(
                "draft.js",
                b"console.log(1);\n",
                KnownEntryKind::JavaScript,
                true,
                ReviewDefaults::default(),
            ),
            false,
            true,
        ),
        (
            "external executable",
            review(
                "tool",
                b"binary bytes",
                KnownEntryKind::Executable,
                false,
                ReviewDefaults::default(),
            ),
            false,
            false,
        ),
        (
            "external script",
            review(
                "tool.js",
                b"console.log(1);\n",
                KnownEntryKind::JavaScript,
                false,
                ReviewDefaults::default(),
            ),
            true,
            true,
        ),
    ] {
        let (_, geometry) = draw_add(&mut session, &state);
        assert_eq!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.target == AddControlId::Storage),
            storage_visible,
            "wrong Storage control shape for {label}"
        );
        assert_eq!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.target == AddControlId::EditSource),
            edit_visible,
            "wrong Edit control shape for {label}"
        );
        assert!(
            geometry
                .hits
                .iter()
                .any(|hit| { session.focused() == Some(&hit.target) }),
            "{label} left focus outside visible geometry"
        );
        assert_focus_cycle_has_only_visible_controls(&mut session, &state);
    }
}

#[test]
fn add_storage_sync_keeps_copy_and_reference_controls_disjoint() {
    let mut state = review(
        "tool.js",
        b"import chalk from 'chalk';\nconst OUTPUT = 'out.txt';\nconsole.log(chalk, OUTPUT);\n",
        KnownEntryKind::JavaScript,
        false,
        ReviewDefaults::default(),
    );
    let mut session = AddScreenSession::default();
    let (copy_terminal, copy_geometry) = draw_add(&mut session, &state);
    let copy = text(copy_terminal.backend().buffer());
    assert!(
        copy.contains("Keep a copy"),
        "copy selection is not rendered:\n{copy}"
    );
    assert!(
        copy.contains("Package dependencies"),
        "npm copy lost its dependency editor"
    );
    assert!(
        copy_geometry
            .hits
            .iter()
            .any(|hit| { hit.target == AddControlId::Text(skit_tui::AddTextField::Dependencies) })
    );
    assert!(
        copy_geometry
            .hits
            .iter()
            .any(|hit| { matches!(hit.target, AddControlId::Candidate(_)) })
    );

    tab_until_add_focus(
        &mut session,
        &state,
        &copy_geometry,
        &AddControlId::Text(skit_tui::AddTextField::Dependencies),
    );
    let _ = state.reduce(AddAction::SetReviewStorage(StorageMode::Reference));
    let (reference_terminal, reference_geometry) = draw_add(&mut session, &state);
    let reference = text(reference_terminal.backend().buffer());
    assert!(
        reference.contains("Link the original"),
        "reference selection is not rendered:\n{reference}"
    );
    assert!(!reference.contains("Package dependencies"));
    assert!(reference_geometry.hits.iter().all(|hit| {
        hit.target != AddControlId::Text(skit_tui::AddTextField::Dependencies)
            && !matches!(hit.target, AddControlId::Candidate(_))
    }));
    assert!(
        reference_geometry
            .hits
            .iter()
            .any(|hit| { session.focused() == Some(&hit.target) }),
        "focus stayed on a control removed by reference mode"
    );

    let _ = state.reduce(AddAction::SetReviewStorage(StorageMode::Copy));
    let (copy_again_terminal, copy_again_geometry) = draw_add(&mut session, &state);
    assert!(text(copy_again_terminal.backend().buffer()).contains("Keep a copy"));
    assert!(
        copy_again_geometry
            .hits
            .iter()
            .any(|hit| { hit.target == AddControlId::Text(skit_tui::AddTextField::Dependencies) })
    );

    let reference_first = review(
        "linked.js",
        b"import chalk from 'chalk';\n",
        KnownEntryKind::JavaScript,
        false,
        ReviewDefaults {
            reference: true,
            ..ReviewDefaults::default()
        },
    );
    let (terminal, geometry) = draw_add(&mut AddScreenSession::default(), &reference_first);
    let rendered = text(terminal.backend().buffer());
    assert!(rendered.contains("Link the original"));
    assert!(
        geometry
            .hits
            .iter()
            .all(|hit| { hit.target != AddControlId::Text(skit_tui::AddTextField::Dependencies) })
    );
    assert_focus_cycle_has_only_visible_controls(
        &mut AddScreenSession::default(),
        &reference_first,
    );
}

#[test]
fn add_prompt_candidate_boundaries_keep_focus_hits_and_ctrl_o_in_lockstep() {
    for (count, interpolate, has_picker) in [
        (PROMPT_LIST_PREVIEW_LIMIT - 1, true, false),
        (PROMPT_LIST_PREVIEW_LIMIT, true, false),
        (PROMPT_LIST_PREVIEW_LIMIT + 1, true, true),
        (PROMPT_LIST_PREVIEW_LIMIT + 1, false, false),
    ] {
        let state = prompt_review(count, interpolate, ReviewDefaults::default());
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw_add(&mut session, &state);
        assert_eq!(
            geometry
                .hits
                .iter()
                .any(|hit| hit.target == AddControlId::Continue),
            has_picker,
            "wrong Choose variables control at count={count}, interpolate={interpolate}"
        );
        let visited = assert_focus_cycle_has_only_visible_controls(&mut session, &state);
        assert_eq!(
            visited.contains(&AddControlId::Continue),
            has_picker,
            "focus shape disagrees with the rendered picker control"
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL,)),
                &state,
                &geometry,
            ),
            has_picker.then_some(AddScreenEvent::OpenPromptCandidates),
            "Ctrl+O disagrees with the visible picker control"
        );
    }
}

#[test]
fn add_runner_selection_and_close_report_the_exact_visible_state_change() {
    let state = prompt_review(
        1,
        true,
        ReviewDefaults {
            runner: Some("beta".to_owned()),
            runner_names: vec!["alpha".to_owned(), "beta".to_owned()],
            ..ReviewDefaults::default()
        },
    );
    let mut session = AddScreenSession::default();
    let (terminal, geometry) = draw_add(&mut session, &state);
    let runner = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Runner)
        .expect("Prompt runner control")
        .area;
    let buffer = terminal.backend().buffer();
    let runner_text = (runner.y..runner.bottom())
        .flat_map(|row| {
            (runner.x..runner.right()).map(move |column| buffer[(column, row)].symbol().to_owned())
        })
        .collect::<String>();
    assert!(
        runner_text.contains("beta"),
        "wrong selected runner: {runner_text}"
    );
    assert_eq!(
        add_click(
            &mut session,
            &state,
            &geometry,
            runner.x.saturating_add(1),
            runner.y.saturating_add(1),
        ),
        Some(AddScreenEvent::Changed)
    );
    let (_, open) = draw_add(&mut session, &state);
    assert!(
        open.hits
            .iter()
            .any(|hit| matches!(hit.target, AddControlId::RunnerOption(_)))
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &open,
        ),
        Some(AddScreenEvent::Changed),
        "closing an open runner select must request a redraw"
    );
    let (_, closed) = draw_add(&mut session, &state);
    assert!(
        closed
            .hits
            .iter()
            .all(|hit| !matches!(hit.target, AddControlId::RunnerOption(_)))
    );
}

#[test]
fn add_kind_home_and_end_choose_the_first_and_last_exact_kind() {
    for (code, take_choice) in [(KeyCode::Home, 0_usize), (KeyCode::End, usize::MAX)] {
        let state = ambiguous_kind("likely.md");
        let choices = state.kind_picker().expect("kind picker").choices();
        let expected = if take_choice == usize::MAX {
            *choices.last().expect("last kind")
        } else {
            choices[take_choice]
        };
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw_add(&mut session, &state);
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
                &state,
                &geometry,
            ),
            Some(AddScreenEvent::Changed)
        );
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
                &state,
                &geometry,
            ),
            Some(AddScreenEvent::Action(AddAction::PickKind(Some(expected))))
        );
    }
}

#[test]
fn add_source_ctrl_d_is_distinct_from_bare_d_and_other_control_keys() {
    let mut state = AddWorkflowState::new(Vec::new());
    let _ = state.reduce(AddAction::SetSourcePath("abc".to_owned()));
    let mut session = AddScreenSession::default();
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::SetSourcePath(
            "abcd".to_owned()
        )))
    );

    let mut session = AddScreenSession::default();
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
            &state,
            &geometry,
        ),
        None,
        "an unrelated Ctrl key must not become Delete-next-character"
    );

    let mut session = AddScreenSession::default();
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::SetSourcePath(
            "abc".to_owned()
        )))
    );
    assert_eq!(
        session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL,)),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::SetSourcePath(
            "ab".to_owned()
        )))
    );
}

#[test]
fn add_storage_dropdown_options_keep_typed_indices_and_actions() {
    let state = review(
        "tool.py",
        b"print('ok')\n",
        KnownEntryKind::Python,
        false,
        ReviewDefaults::default(),
    );
    for (index, expected) in [(0, StorageMode::Copy), (1, StorageMode::Reference)] {
        let mut session = AddScreenSession::default();
        let (_, geometry) = draw_add(&mut session, &state);
        let storage = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Storage)
            .expect("Storage control")
            .area;
        assert_eq!(
            add_click(
                &mut session,
                &state,
                &geometry,
                storage.x.saturating_add(1),
                storage.y.saturating_add(1),
            ),
            Some(AddScreenEvent::Changed)
        );
        let (_, open) = draw_add(&mut session, &state);
        let option = open
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::StorageOption(index))
            .unwrap_or_else(|| panic!("Storage option {index}"))
            .area;
        assert_eq!(
            add_click(
                &mut session,
                &state,
                &open,
                option.x.saturating_add(1),
                option.y,
            ),
            Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                expected
            )))
        );
    }
}

#[test]
fn add_review_notes_distinguish_empty_owned_metadata_and_modeled_boundaries() {
    for (body, absent) in [
        (
            b"# /// script\n# requires-python = \">=3.12\"\n# dependencies = []\n# ///\nprint(1)\n"
                .as_slice(),
            "needs Python >=3.12",
        ),
        (
            b"# /// script\n# dependencies = [\"requests\"]\n# ///\nprint(1)\n".as_slice(),
            "installs requests",
        ),
    ] {
        let state = review(
            "owned.py",
            body,
            KnownEntryKind::Python,
            false,
            ReviewDefaults::default(),
        );
        let (terminal, _) = draw_add(&mut AddScreenSession::default(), &state);
        let rendered = text(terminal.backend().buffer());
        assert!(
            rendered.contains(absent),
            "missing owned-metadata fact: {rendered}"
        );
        assert!(
            !rendered.contains("(none declared)"),
            "a partially populated metadata block was labeled empty: {rendered}"
        );
    }

    let zero = review(
        "zero.py",
        b"P = 'x'\np.add_argument('--help', action='help')\n",
        KnownEntryKind::Python,
        false,
        ReviewDefaults::default(),
    );
    let (terminal, _) = draw_add(&mut AddScreenSession::default(), &zero);
    let zero_rendered = text(terminal.backend().buffer());
    assert!(
        !zero_rendered.contains("skit read this script's own arguments"),
        "a zero-field parser must not claim a run form was modeled"
    );
    assert!(
        !zero_rendered.contains("couldn't model them statically"),
        "a modeled zero-field parser must not get the unmodeled-framework notice"
    );

    let dynamic = review(
        "dynamic.sh",
        b"#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n",
        KnownEntryKind::Shell,
        false,
        ReviewDefaults::default(),
    );
    let review = dynamic.review().expect("dynamic review");
    assert!(review.onboarding().uses_cli_framework());
    assert!(review.modeled_cli_field_count().is_none());
    let (terminal, _) = draw_add_sized(&mut AddScreenSession::default(), &dynamic, 200, 100);
    assert!(
        text(terminal.backend().buffer())
            .contains("so the run form offers an extra-arguments field"),
        "the unmodeled CLI-framework notice is missing"
    );
}

#[test]
fn add_review_candidate_and_reference_copy_are_semantically_exact() {
    let input = review(
        "input.py",
        b"NAME = input('Your name?')\nprint(NAME)\n",
        KnownEntryKind::Python,
        false,
        ReviewDefaults::default(),
    );
    let (terminal, _) = draw_add(&mut AddScreenSession::default(), &input);
    assert!(
        text(terminal.backend().buffer()).contains("input() #1: Your name?"),
        "an input-bound parameter lost its prompt label"
    );

    for (body, expected, rejected) in [
        (
            b"import argparse\np = argparse.ArgumentParser()\np.add_argument('--name')\n"
                .as_slice(),
            "Link the original: skit never writes to the file.",
            "parameter setup is skipped",
        ),
        (
            b"P = 'x'\np.add_argument('--help', action='help')\n".as_slice(),
            "parameter setup is skipped",
            "Link the original: skit never writes to the file.",
        ),
    ] {
        let state = review(
            "linked.py",
            body,
            KnownEntryKind::Python,
            false,
            ReviewDefaults {
                reference: true,
                ..ReviewDefaults::default()
            },
        );
        let (terminal, _) = draw_add(&mut AddScreenSession::default(), &state);
        let rendered = text(terminal.backend().buffer());
        assert!(
            rendered.contains(expected),
            "missing reference note: {rendered}"
        );
        assert!(
            !rendered.contains(rejected),
            "wrong reference note: {rendered}"
        );
    }
}

#[test]
fn add_source_draft_overflow_note_has_an_exact_zero_to_one_boundary() {
    let drafts = (0..=PROMPT_LIST_PREVIEW_LIMIT)
        .map(|index| DraftSummary {
            path: format!("draft-{index:02}.py").into(),
            modified: u64::try_from(index).unwrap(),
            identity: None,
            permissions: SourcePermissions::default(),
            content_hash: None,
        })
        .collect::<Vec<_>>();
    for (count, note) in [
        (PROMPT_LIST_PREVIEW_LIMIT, None),
        (PROMPT_LIST_PREVIEW_LIMIT + 1, Some("…and 1 more")),
    ] {
        let state = AddWorkflowState::new(drafts[..count].to_vec());
        let (terminal, _) = draw_add(&mut AddScreenSession::default(), &state);
        let rendered = text(terminal.backend().buffer());
        assert_eq!(
            rendered.contains("…and"),
            note.is_some(),
            "wrong overflow boundary for {count} drafts: {rendered}"
        );
        if let Some(note) = note {
            assert!(rendered.contains(note), "wrong overflow count: {rendered}");
        }
    }
}

#[test]
fn add_footer_labels_and_select_options_follow_the_review_lane() {
    for (state, edit_label, reference_label) in [
        (
            prompt_review(1, true, ReviewDefaults::default()),
            "Edit prompt",
            "Link the original — edits take effect immediately; skit never writes",
        ),
        (
            review(
                "tool.py",
                b"print(1)\n",
                KnownEntryKind::Python,
                false,
                ReviewDefaults::default(),
            ),
            "Edit script",
            "Link the original — edits take effect immediately, but skit won't write",
        ),
    ] {
        let mut session = AddScreenSession::default();
        let (terminal, geometry) = draw_add(&mut session, &state);
        assert!(
            text(terminal.backend().buffer()).contains(edit_label),
            "wrong localized edit footer label"
        );
        let storage = geometry
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::Storage)
            .expect("external review Storage control")
            .area;
        assert_eq!(
            add_click(
                &mut session,
                &state,
                &geometry,
                storage.x.saturating_add(1),
                storage.y.saturating_add(1),
            ),
            Some(AddScreenEvent::Changed)
        );
        let (terminal, open) = draw_add(&mut session, &state);
        let option = open
            .hits
            .iter()
            .find(|hit| hit.target == AddControlId::StorageOption(1))
            .expect("visible reference option after opening Storage")
            .area;
        let option_text = (option.x..option.right())
            .map(|column| terminal.backend().buffer()[(column, option.y)].symbol())
            .collect::<String>();
        assert!(
            option_text.contains(reference_label),
            "the typed reference hit does not own its visible lane-specific label: {option_text}"
        );
        assert_eq!(
            add_click(
                &mut session,
                &state,
                &open,
                option.x.saturating_add(1),
                option.y,
            ),
            Some(AddScreenEvent::Action(AddAction::SetReviewStorage(
                StorageMode::Reference
            )))
        );
    }
}

#[test]
fn add_footer_wraps_only_past_its_exact_measured_width() {
    let state = AddWorkflowState::new(Vec::new());
    let expected = [
        AddControlId::Continue,
        AddControlId::Cancel,
        AddControlId::NewScript,
        AddControlId::NewPrompt,
        AddControlId::NextField,
        AddControlId::PreviousField,
    ];
    let (wide, wide_geometry) = draw_add_sized(&mut AddScreenSession::default(), &state, 200, 3);
    let footer_y = 2;
    let exact_width = wide_geometry
        .hits
        .iter()
        .filter(|hit| hit.area.y == footer_y && expected.contains(&hit.target))
        .map(|hit| hit.area.right())
        .max()
        .expect("wide footer command hits");
    assert_ne!(wide.backend().buffer()[(199, footer_y)].symbol(), "↓");

    for (width, all_visible) in [
        (exact_width, true),
        (exact_width.saturating_sub(1), false),
        (2, false),
        (1, false),
    ] {
        let (terminal, geometry) =
            draw_add_sized(&mut AddScreenSession::default(), &state, width, 3);
        let footer_hits = geometry
            .hits
            .iter()
            .filter(|hit| hit.area.y == footer_y && expected.contains(&hit.target))
            .collect::<Vec<_>>();
        assert_eq!(
            footer_hits.len() == expected.len(),
            all_visible,
            "wrong footer wrap at exact measured width {width}"
        );
        assert!(
            footer_hits.iter().all(|hit| hit.area.right() <= width),
            "a footer hit overlaps the reserved indicator column or exits the viewport"
        );
        if width == 1 {
            assert!(!footer_hits.is_empty(), "width one lost its first action");
            assert_eq!(
                terminal.backend().buffer()[(0, footer_y)].symbol(),
                "[",
                "a scroll indicator replaced the only visible action cell"
            );
            let mut session = AddScreenSession::default();
            let (_, geometry) = draw_add_sized(&mut session, &state, width, 3);
            assert_eq!(
                add_click(&mut session, &state, &geometry, 0, footer_y),
                Some(AddScreenEvent::Action(AddAction::Continue)),
                "the visible width-one action cell must activate Continue"
            );
        } else if !all_visible {
            assert_eq!(
                terminal.backend().buffer()[(width - 1, footer_y)].symbol(),
                "↓",
                "an overflowing footer needs a separate scroll indicator cell"
            );
            assert!(
                footer_hits.iter().all(|hit| hit.area.right() < width),
                "a footer hit overlaps its scroll indicator"
            );
        }
    }
}

#[test]
fn add_local_footer_activates_continue_on_primary_release() {
    let state = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
        })
        .unwrap();
    let area = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Continue)
        .expect("Continue is visible in the Add footer")
        .area;

    assert_eq!(
        session.handle_event(
            mouse_with_kind(MouseEventKind::Down(MouseButton::Left), area.x, area.y),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Changed),
        "primary Down must arm Continue without continuing"
    );
    assert_eq!(
        session.handle_event(
            mouse_with_kind(MouseEventKind::Up(MouseButton::Left), area.x, area.y),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::Continue))
    );
}

#[test]
fn add_release_outside_cancels_the_footer_arm_before_a_late_release() {
    let state = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
        })
        .unwrap();
    let continue_area = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Continue)
        .expect("Continue is visible in the Add footer")
        .area;
    let outside = (0..20)
        .flat_map(|row| (0..80).map(move |column| (column, row)))
        .find(|(column, row)| {
            geometry
                .hits
                .iter()
                .all(|hit| !hit.area.contains((*column, *row).into()))
        })
        .expect("the Add frame must contain one non-widget cell");

    assert_eq!(
        session.handle_event(
            mouse_with_kind(
                MouseEventKind::Down(MouseButton::Left),
                continue_area.x,
                continue_area.y,
            ),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Changed)
    );
    assert_eq!(
        session.handle_event(
            mouse_with_kind(MouseEventKind::Up(MouseButton::Left), outside.0, outside.1),
            &state,
            &geometry,
        ),
        None,
        "release outside must cancel the armed footer target"
    );
    assert_eq!(
        session.handle_event(
            mouse_with_kind(
                MouseEventKind::Up(MouseButton::Left),
                continue_area.x,
                continue_area.y,
            ),
            &state,
            &geometry,
        ),
        None,
        "a late release must not resurrect the cancelled footer target"
    );
}

#[test]
fn add_remaining_controls_activate_only_on_matching_primary_release() {
    let state = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
    let mut geometry = Default::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &state, &mut session, Locale::En);
        })
        .unwrap();
    let cancel = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Cancel)
        .expect("Cancel is visible in the Add footer")
        .area;
    assert_eq!(
        session.handle_event(
            mouse_with_kind(MouseEventKind::Down(MouseButton::Left), cancel.x, cancel.y,),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Changed)
    );
    assert_eq!(
        session.handle_event(
            mouse_with_kind(MouseEventKind::Up(MouseButton::Left), cancel.x, cancel.y),
            &state,
            &geometry,
        ),
        Some(AddScreenEvent::Action(AddAction::Cancel))
    );
}

fn scroll_down(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

#[test]
fn add_render_clamps_stale_scroll_and_never_registers_zero_height_rows() {
    let long = AddWorkflowState::new(
        (0..8)
            .map(|index| DraftSummary {
                path: format!("draft-{index}.py").into(),
                modified: u64::try_from(index).unwrap(),
                identity: None,
                permissions: SourcePermissions::default(),
                content_hash: None,
            })
            .collect(),
    );
    let mut session = AddScreenSession::default();
    let (_, mut geometry) = draw_add_sized(&mut session, &long, 48, 5);
    assert_eq!(
        geometry.first_visible, 2,
        "fresh render must not jump to the end"
    );
    for _ in 0..8 {
        if geometry.first_visible >= 4 {
            break;
        }
        assert_eq!(
            session.handle_event(
                scroll_down(geometry.body.x, geometry.body.y),
                &long,
                &geometry,
            ),
            Some(AddScreenEvent::Changed)
        );
        (_, geometry) = draw_add_sized(&mut session, &long, 48, 5);
    }
    assert_eq!(
        geometry.first_visible, 5,
        "the test needs an exact row edge"
    );
    assert!(
        geometry.hits.iter().all(|hit| hit.area.height > 0),
        "a row ending at the viewport start registered a zero-height hit"
    );

    let short = prompt_review(0, false, ReviewDefaults::default());
    let (_, clamped) = draw_add(&mut session, &short);
    assert_eq!(
        clamped.first_visible, 0,
        "a shorter Add shape retained an unreachable stale offset"
    );
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
        content_hash: None,
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
                        let mouse_handling =
                            add_click(&mut session, &workflow, &geometry, hit.area.x, hit.area.y);
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
        session_click(&mut session, &state, &geometry, 3, select_row),
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
        session_click(&mut session, &state, &geometry, cancel_x, cancel_y),
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
        session.handle_event(
            mouse_with_kind(MouseEventKind::Down(MouseButton::Left), 79, 1),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "an active picker must own blank pointer cells instead of falling through to Add",
    );
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
        session_click(&mut session, &state, &geometry, runner_x, runner_y),
        EventHandling::Action(Action::OpenAddRunnerEditor)
    );
}
