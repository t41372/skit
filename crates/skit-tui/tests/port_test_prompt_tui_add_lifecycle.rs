use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::SourcePermissions;
use skit_i18n::Locale;
use skit_tui::{TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, KnownEntryKind, LibraryState,
    ReviewDefaults, ReviewState, Screen, SourceSnapshot,
};

fn snapshot(path: &str, body: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: body.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn inspect(workflow: &mut AddWorkflowState, source: SourceSnapshot) {
    let path = source.path.display().to_string();
    assert!(workflow.reduce(AddAction::SetSourcePath(path.clone())).is_empty());
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, path: requested }] = effects.as_slice() else {
        panic!("add source did not request one inspection: {effects:?}")
    };
    assert_eq!(requested, &source.path);
    assert!(workflow
        .reduce(AddAction::SourceInspected {
            request: *request,
            result: Ok(source),
        })
        .is_empty());
}

fn commit(workflow: &mut AddWorkflowState) -> skit_application::CreateEntry {
    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
        panic!("review save did not emit exactly one commit: {effects:?}")
    };
    (**entry).clone()
}

fn render_review(workflow: AddWorkflowState, width: u16, height: u16) -> String {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow))));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    text(terminal.backend().buffer())
}

fn text(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_tui_add_prompt_opens_the_review_panel() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(
        &mut workflow,
        snapshot(
            "task.prompt.md",
            b"# Task\n\nDo {{a}} and {{b}}\n",
        ),
    );
    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().expect(".prompt.md bypassed review");
    assert_eq!(review.kind(), KnownEntryKind::Prompt);
    assert_eq!(review.name(), "task");
    assert_eq!(review.description(), "Task");
    assert_eq!(review.selected_prompt_names(), ["a", "b"]);

    let entry = commit(&mut workflow);
    assert_eq!(entry.kind.as_str(), "prompt");
    assert_eq!(entry.name, "task");
    assert_eq!(entry.settings.params, ["a", "b"]);
    assert_eq!(entry.workdir, "invoke");
    assert_eq!(entry.settings.runner, "");
}

#[test]
fn test_tui_add_bare_md_asks_before_becoming_a_prompt() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(
        &mut workflow,
        snapshot("notes.md", b"Summarize {{url}}\n"),
    );
    assert_eq!(workflow.stage(), AddStage::Kind, "bare Markdown was silently classified as a prompt");
    let picker = workflow.kind_picker().expect("bare Markdown did not open the kind question");
    assert_eq!(picker.suggested(), Some(KnownEntryKind::Prompt));
    assert!(picker.offers(KnownEntryKind::Prompt));
    assert!(workflow
        .reduce(AddAction::PickKind(Some(KnownEntryKind::Prompt)))
        .is_empty());
    assert_eq!(workflow.stage(), AddStage::Review);
    let entry = commit(&mut workflow);
    assert_eq!(entry.kind.as_str(), "prompt");
    assert_eq!(entry.name, "notes");
    assert_eq!(entry.settings.params, ["url"]);
}

#[test]
fn test_tui_add_bare_md_kind_ask_can_cancel_without_adding() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, snapshot("notes.md", b"ordinary project notes\n"));
    assert_eq!(workflow.stage(), AddStage::Kind);
    let effects = workflow.reduce(AddAction::PickKind(None));
    assert!(effects.is_empty());
    assert_eq!(workflow.stage(), AddStage::Source);
    assert!(workflow.review().is_none());
    assert!(!workflow.commit_pending());
}

#[test]
fn test_review_prompt_without_placeholders_says_so_and_adds_clean() {
    let review = ReviewState::from_source(
        snapshot("plain.prompt.md", b"No holes.\n"),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    assert!(review.prompt_candidates().is_empty());
    assert!(review.prompt_preview().is_empty());
    let rendered = render_review(AddWorkflowState::from_review(review.clone()), 100, 40);
    assert!(
        rendered.contains("No {{name}} placeholders detected"),
        "no-placeholder review lost its explicit explanation:\n{rendered}"
    );
    let mut workflow = AddWorkflowState::from_review(review);
    let entry = commit(&mut workflow);
    assert!(entry.settings.params.is_empty());
    assert!(entry.settings.parameters.is_empty());
}

#[test]
fn test_review_flooded_prompt_previews_capped_and_ticks_nothing() {
    let body = (0..34)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = ReviewState::from_source(
        snapshot("big.prompt.md", body.as_bytes()),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    assert!(review.prompt_is_flooded());
    assert_eq!(review.prompt_preview().len(), 20, "frozen inline preview cap changed");
    assert!(review.selected_prompt_names().is_empty(), "flooded prompt auto-managed variables");
    assert_eq!(review.prompt_candidates().len(), 34, "full searchable candidate set was truncated together with the preview");

    let rendered = render_review(AddWorkflowState::from_review(review.clone()), 105, 46);
    assert!(rendered.contains("probably not written for"), "flood warning disappeared:\n{rendered}");
    assert!(rendered.contains("more"), "flood overflow count/hint disappeared:\n{rendered}");
    let mut workflow = AddWorkflowState::from_review(review);
    let entry = commit(&mut workflow);
    assert!(entry.settings.params.is_empty());
}

#[test]
fn test_review_escape_adds_nothing() {
    let review = ReviewState::from_source(
        snapshot("e.prompt.md", b"{{a}}\n"),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    );
    let mut workflow = AddWorkflowState::from_review(review);
    let effects = workflow.reduce(AddAction::Cancel);
    assert_eq!(workflow.stage(), AddStage::Cancelled);
    assert_eq!(effects, [AddEffect::Cancel]);
    assert!(!effects.iter().any(|effect| matches!(effect, AddEffect::Commit { .. })));
    assert!(!workflow.commit_pending());
}

#[test]
fn test_review_escape_returns_to_the_add_source_screen() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, snapshot("back.prompt.md", b"{{a}}\n"));
    assert_eq!(workflow.stage(), AddStage::Review);
    let effects = workflow.reduce(AddAction::Cancel);
    assert!(effects.is_empty(), "ordinary source cancellation unexpectedly closed the whole add workflow: {effects:?}");
    assert_eq!(workflow.stage(), AddStage::Source);
    assert!(workflow.review().is_none());
    assert!(!workflow.commit_pending());
}
