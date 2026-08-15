use std::fs;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use skit_application::{
    EntryMutationRepository as _, EntryRepository as _, SourcePermissions,
};
use skit_i18n::Locale;
use skit_store::FileStore;
use skit_tui::{TuiSession, ViewGeometry, render_with_session};
use skit_ui::{
    Action, AddAction, AddEffect, AddNotice, AddProblem, AddStage, AddWorkflowState, DraftKind,
    KnownEntryKind, LibraryState, ReviewDefaults, ReviewState, Screen, SourceSnapshot,
};
use tempfile::TempDir;

fn source(path: &str, bytes: &[u8], draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: draft,
    }
}

fn render_workflow(workflow: AddWorkflowState) -> String {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow))));
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(110, 40)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    buffer_text(terminal.backend().buffer())
}

fn buffer_text(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_prompt_draft_with_invalid_utf8_reaches_strict_review() {
    let bytes = b"draft:\xff\n";
    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Prompt));
    let [AddEffect::AuthorDraft { request, kind: DraftKind::Prompt }] = effects.as_slice() else {
        panic!("Ctrl+P did not request one prompt draft: {effects:?}")
    };
    let request = *request;
    assert!(workflow
        .reduce(AddAction::DraftEdited {
            request,
            result: Ok(Some(source("skit-new-invalid.prompt.md", bytes, true))),
        })
        .is_empty());
    assert_eq!(workflow.stage(), AddStage::Review, "invalid prompt bytes were rejected before the strict review surface could explain them");
    assert_eq!(workflow.problem(), None, "host-level draft transport failed before prompt validation");
    let review = workflow.review().expect("strict prompt review missing");
    assert_eq!(review.source().bytes, bytes, "invalid UTF-8 draft was decoded/re-encoded before review");
    assert!(matches!(review.create_entry(), Err(AddProblem::InvalidPromptEncoding)));

    let rendered = render_workflow(workflow.clone());
    assert!(rendered.contains("offset 6"), "strict review must name the first invalid UTF-8 byte offset:\n{rendered}");
    assert!(!rendered.contains('�'), "strict review silently replacement-decoded invalid source:\n{rendered}");

    let draft_path = review.source().path.clone();
    let effects = workflow.reduce(AddAction::Cancel);
    assert_eq!(effects, [AddEffect::DraftKept(draft_path.clone())]);
    assert_eq!(workflow.notice(), Some(&AddNotice::DraftKept(draft_path)));
}

#[test]
fn test_prompt_review_surfaces_initial_and_post_editor_os_errors() {
    let mut initial = AddWorkflowState::new(Vec::new());
    initial.reduce(AddAction::SetSourcePath("vanished.prompt.md".to_owned()));
    let effects = initial.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, .. }] = effects.as_slice() else {
        panic!("source intake did not request inspection")
    };
    let request = *request;
    initial.reduce(AddAction::SourceInspected {
        request,
        result: Err("permission changed".to_owned()),
    });
    assert!(matches!(initial.problem(), Some(AddProblem::SourceUnavailable { path, reason }) if path.to_string_lossy()=="vanished.prompt.md" && reason=="permission changed"));
    let rendered = render_workflow(initial);
    assert!(rendered.contains("vanished.prompt.md") && rendered.contains("permission changed"), "initial prompt-open error lost path or OS detail:\n{rendered}");

    let mut edited = AddWorkflowState::from_review(ReviewState::from_source(
        source("edited.prompt.md", b"{{a}}", false),
        KnownEntryKind::Prompt,
        ReviewDefaults::default(),
    ));
    let effects = edited.reduce(AddAction::EditSource);
    let [AddEffect::EditSource { request, path }] = effects.as_slice() else {
        panic!("review edit did not request the source editor")
    };
    assert_eq!(path.to_string_lossy(), "edited.prompt.md");
    let request = *request;
    let before = edited.review().unwrap().source().bytes.clone();
    edited.reduce(AddAction::SourceEdited {
        request,
        result: Err("edited.prompt.md: No such file".to_owned()),
    });
    assert_eq!(edited.stage(), AddStage::Review);
    assert_eq!(edited.review().unwrap().source().bytes, before);
    assert_eq!(edited.problem(), Some(&AddProblem::EditFailed { reason: "edited.prompt.md: No such file".to_owned() }));
    let rendered = render_workflow(edited);
    assert!(rendered.contains("edited.prompt.md") && rendered.contains("No such file"), "post-editor OS error lost path or reason:\n{rendered}");
}

#[test]
fn test_review_duplicate_name_notifies_and_stays() {
    let data = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    let original = ReviewState::from_source(
        source("dup.prompt.md", b"original\n", false),
        KnownEntryKind::Prompt,
        ReviewDefaults { name: Some("dup".to_owned()), ..ReviewDefaults::default() },
    );
    let first = store.create(original.create_entry().unwrap()).unwrap();
    assert_eq!(first.meta.name, "dup");

    let mut workflow = AddWorkflowState::from_review(ReviewState::from_source(
        source("replacement.prompt.md", b"replacement\n", false),
        KnownEntryKind::Prompt,
        ReviewDefaults { name: Some("dup".to_owned()), ..ReviewDefaults::default() },
    ));
    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit { request, entry, .. }] = effects.as_slice() else {
        panic!("duplicate-name review did not emit one repository request: {effects:?}")
    };
    let request_id = *request;
    let error = store
        .create((**entry).clone())
        .expect_err("duplicate entry name was silently accepted");
    let detail = error.to_string();
    assert!(!detail.is_empty());
    assert!(workflow
        .reduce(AddAction::CommitFinished {
            request: request_id,
            result: Err(detail.clone()),
        })
        .is_empty());
    assert_eq!(workflow.stage(), AddStage::Review, "repository refusal closed the review panel");
    assert_eq!(workflow.review().unwrap().name(), "dup");
    assert_eq!(workflow.problem(), Some(&AddProblem::CommitFailed { reason: detail.clone() }));

    let still = store.resolve("dup").unwrap();
    assert_eq!(still.meta.id, first.meta.id, "duplicate refusal replaced the existing entry");
    assert_eq!(fs::read(data.path().join("scripts/dup/prompt.md")).unwrap(), b"original\n");
    let script_dirs = fs::read_dir(data.path().join("scripts"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .count();
    assert_eq!(script_dirs, 1, "duplicate refusal left an extra entry directory behind");

    let rendered = render_workflow(workflow);
    assert!(rendered.contains(&detail), "repository refusal was not surfaced in the still-open review:\n{rendered}");
}
