//! Exact TUI lifecycle contracts from Python v0.4 `tests/test_add_no_source.py`, mapped to Rust's
//! public frontend-neutral add reducer. Red expectations are intentional parity findings.

use skit_application::SourcePermissions;
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewLane,
    ReviewState, SourceSnapshot,
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

fn inspect(workflow: &mut AddWorkflowState, snapshot: SourceSnapshot) {
    let _ = workflow.reduce(AddAction::SetSourcePath(snapshot.path.display().to_string()));
    let effects = workflow.reduce(AddAction::Continue);
    let AddEffect::InspectSource { request, .. } = effects.first().expect("inspect effect") else {
        panic!("expected source inspection, got {effects:?}");
    };
    let _ = workflow.reduce(AddAction::SourceInspected {
        request: *request,
        result: Ok(snapshot),
    });
}

#[test]
fn test_bare_add_tui_form_summary_on_success() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetCommandTemplate("echo {msg}".into()));
    let _ = workflow.reduce(AddAction::SetCommandName("viatui".into()));
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::Commit { request, entry, source }] = effects.as_slice() else {
        panic!("a valid TUI command door must request exactly one repository create: {effects:?}");
    };
    assert_eq!(entry.name, "viatui");
    assert_eq!(entry.kind.as_str(), "command");
    assert_eq!(entry.settings.params, ["msg"]);
    assert!(entry.payload.is_none());
    assert!(source.is_none());

    let done = workflow.reduce(AddAction::CommitFinished {
        request: *request,
        result: Ok("viatui".into()),
    });
    assert_eq!(done, [AddEffect::Complete("viatui".into())]);
    assert_eq!(workflow.stage(), AddStage::Complete);
}

#[test]
fn test_bare_add_tui_form_cancel_exits_130() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    assert_eq!(workflow.reduce(AddAction::Cancel), [AddEffect::Cancel]);
    assert_eq!(workflow.stage(), AddStage::Cancelled);
}

#[test]
fn test_unknown_plain_pick_prompt_runs_prompt_onboarding() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, source("mystery.xyz", b"do {{thing}}\n"));
    assert_eq!(workflow.stage(), AddStage::Kind);
    let _ = workflow.reduce(AddAction::PickKind(Some(KnownEntryKind::Prompt)));
    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().unwrap();
    assert_eq!(review.kind(), KnownEntryKind::Prompt);
    assert_eq!(review.lane(), ReviewLane::Prompt);
    assert_eq!(
        review
            .prompt_candidates()
            .iter()
            .map(|candidate| candidate.name.as_str())
            .collect::<Vec<_>>(),
        ["thing"]
    );
}

#[test]
fn test_unknown_tui_form_cancel_exits_130() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, source("mystery.xyz", b"opaque text\n"));
    assert_eq!(workflow.stage(), AddStage::Kind);
    let effects = workflow.reduce(AddAction::PickKind(None));
    assert_eq!(
        effects,
        [AddEffect::Cancel],
        "frozen hosted kind-modal cancellation exits the add instead of silently returning to Source"
    );
    assert_eq!(workflow.stage(), AddStage::Cancelled);
}

#[test]
fn test_unknown_tui_form_pick_exe_cancel_exits_130() {
    let review = ReviewState::from_source(
        source("mystery.xyz", b"opaque text\n"),
        KnownEntryKind::Executable,
        ReviewDefaults::default(),
    );
    let mut hosted = AddWorkflowState::from_review(review);
    assert_eq!(hosted.stage(), AddStage::Review);
    assert_eq!(hosted.reduce(AddAction::Cancel), [AddEffect::Cancel]);
    assert_eq!(hosted.stage(), AddStage::Cancelled);
}
