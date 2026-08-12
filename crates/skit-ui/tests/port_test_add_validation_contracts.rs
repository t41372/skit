//! Typed interactive-state ports from Python `tests/test_add_validation_contracts.py` at
//! `main@206f9ef`.
//!
//! Python's private prompt loops become the Rust review state machine: invalid input must leave the
//! same Review open, and corrected input on that state is the only path to a Commit effect.

use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, KnownEntryKind, ReviewDefaults,
    ReviewLane, ReviewState, SourceSnapshot,
};

fn source(path: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(path),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn python_review() -> ReviewState {
    let mut review = ReviewState::from_source(
        source("review.py", b"import requests\nprint(requests)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_name("review");
    review
}

fn resumed(path: &str, bytes: &[u8]) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(vec![DraftSummary {
        path: PathBuf::from(path),
        modified: 1,
    }]);
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, .. }] = effects.as_slice() else {
        panic!("draft resume must inspect the selected source");
    };
    let request = *request;
    let mut snapshot = source(path, bytes);
    snapshot.is_draft = true;
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot),
    });
    workflow
}

#[test]
fn test_interactive_deps_reask_then_python_reask_then_accept() {
    let mut workflow = AddWorkflowState::from_review(python_review());
    let _ = workflow.reduce(AddAction::SetReviewDependencies("@@@".to_owned()));

    assert!(workflow.reduce(AddAction::Save).is_empty());
    assert_eq!(workflow.stage(), AddStage::Review);
    assert!(!workflow.commit_pending());

    // Python's deps prompt uses '-' as the explicit "none" answer. The Rust review must preserve
    // that public meaning rather than send '-' into PEP 508 validation.
    let _ = workflow.reduce(AddAction::SetReviewDependencies("-".to_owned()));
    let _ = workflow.reduce(AddAction::SetReviewPython("not-a-version".to_owned()));
    assert!(workflow.reduce(AddAction::Save).is_empty());
    assert_eq!(workflow.stage(), AddStage::Review);
    assert!(!workflow.commit_pending());

    let _ = workflow.reduce(AddAction::SetReviewPython(">=3.11".to_owned()));
    let effects = workflow.reduce(AddAction::Save);
    let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
        panic!("corrected review must emit exactly one Commit effect: {effects:?}");
    };
    assert!(entry.settings.dependencies.is_empty());
    let stored = String::from_utf8(entry.payload.as_ref().unwrap().bytes.clone()).unwrap();
    assert!(stored.contains("requires-python = \">=3.11\""), "{stored}");
}

#[test]
fn test_interactive_valid_deps_accepted_first_try() {
    let mut workflow = AddWorkflowState::from_review(python_review());
    let _ = workflow.reduce(AddAction::SetReviewDependencies("rich>=13,<16".to_owned()));
    let _ = workflow.reduce(AddAction::SetReviewPython("-".to_owned()));

    let effects = workflow.reduce(AddAction::Save);

    let [AddEffect::Commit { entry, .. }] = effects.as_slice() else {
        panic!("valid review must commit on the first Save: {effects:?}");
    };
    let stored = String::from_utf8(entry.payload.as_ref().unwrap().bytes.clone()).unwrap();
    assert!(stored.contains("rich>=13,<16"), "{stored}");
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
fn test_kind_for_draft_single_prompt_extension_outranks_the_shebang() {
    let workflow = resumed(
        "skit-new-note.prompt",
        b"#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );

    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().expect("prompt draft reached review");
    assert_eq!(review.kind(), KnownEntryKind::Prompt);
    assert_eq!(review.lane(), ReviewLane::Prompt);
}

#[test]
fn test_kind_for_draft_extensionless_falls_through_to_the_shebang() {
    let workflow = resumed("skit-new-plain", b"#!/usr/bin/env bash\necho hi\n");

    assert_eq!(workflow.stage(), AddStage::Review);
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
}

#[test]
fn test_kind_for_draft_script_suffix_stays_shebang_first() {
    let workflow = resumed(
        "skit-new-shellish.py",
        b"#!/usr/bin/env bash\necho drafted\n",
    );

    assert_eq!(workflow.stage(), AddStage::Review);
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
}
