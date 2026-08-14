//! Exact editor-lane routing contracts from Python v0.4 `tests/test_add_no_source.py`.
//!
//! Rust models editor launch as a typed host effect. These tests verify that effect and the blank
//! review defaults instead of recreating Python's private `_create_*_in_editor` functions.

use skit_application::SourcePermissions;
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, DraftKind, KnownEntryKind, ReviewLane,
    SourceSnapshot,
};

fn edited(path: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: true,
        is_directory: false,
        is_draft: true,
    }
}

#[test]
fn test_plain_menu_choice2_opens_the_python_editor_lane() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Script));
    let [AddEffect::AuthorDraft { request, kind }] = effects.as_slice() else {
        panic!("choice 2 must request exactly one editor-authored script draft: {effects:?}");
    };
    assert_eq!(*kind, DraftKind::Script);
    let request = *request;
    assert!(
        workflow
            .reduce(AddAction::DraftEdited {
                request,
                result: Ok(Some(edited("skit-new.py", b"print('hi')\n"))),
            })
            .is_empty()
    );
    assert_eq!(workflow.stage(), AddStage::Review);
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Python);
    assert_eq!(workflow.review().unwrap().lane(), ReviewLane::Script);
}

#[test]
fn test_plain_menu_choice3_opens_the_prompt_editor_lane() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Prompt));
    let [AddEffect::AuthorDraft { request, kind }] = effects.as_slice() else {
        panic!("choice 3 must request exactly one editor-authored prompt draft: {effects:?}");
    };
    assert_eq!(*kind, DraftKind::Prompt);
    let request = *request;
    assert!(
        workflow
            .reduce(AddAction::DraftEdited {
                request,
                result: Ok(Some(edited("skit-new.prompt.md", b"Review {{thing}}.\n"))),
            })
            .is_empty()
    );
    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().unwrap();
    assert_eq!(review.kind(), KnownEntryKind::Prompt);
    assert_eq!(review.lane(), ReviewLane::Prompt);
    assert!(review.interpolate(), "--no-interpolate was not supplied, so interpolation must default on");
}

#[test]
fn test_ans_choice2_python_lane_uses_blank_defaults() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let defaults = workflow.review_defaults();
    assert_eq!(defaults.name, None);
    assert_eq!(defaults.description, None);
    assert!(defaults.dependencies.is_empty());
    assert_eq!(defaults.requires_python, None);

    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Script));
    assert!(matches!(
        effects.as_slice(),
        [AddEffect::AuthorDraft { kind: DraftKind::Script, .. }]
    ));
}

#[test]
fn test_ans_choice3_prompt_lane_uses_blank_defaults() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let defaults = workflow.review_defaults();
    assert_eq!(defaults.name, None);
    assert_eq!(defaults.description, None);
    assert_eq!(defaults.runner, None);
    assert_eq!(defaults.interpolate, None);

    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Prompt));
    let [AddEffect::AuthorDraft { request, kind }] = effects.as_slice() else {
        panic!("blank prompt lane must request a prompt draft: {effects:?}");
    };
    assert_eq!(*kind, DraftKind::Prompt);
    let request = *request;
    let _ = workflow.reduce(AddAction::DraftEdited {
        request,
        result: Ok(Some(edited("skit-new.prompt.md", b"Review {{thing}}.\n"))),
    });
    assert!(workflow.review().unwrap().interpolate(), "blank defaults must preserve interpolate=true");
}
