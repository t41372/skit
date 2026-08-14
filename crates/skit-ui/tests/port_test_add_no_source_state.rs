//! Exact frontend-neutral ports from Python v0.4 `tests/test_add_no_source.py`.
//!
//! Python reached these behaviors by monkeypatching Textual host callbacks. Rust exposes the
//! equivalent add workflow as a public reducer, so the tests pin typed choices/review transitions
//! directly instead of recreating Python callback plumbing.

use skit_application::SourcePermissions;
use skit_ui::{
    AddAction, AddEffect, AddStage, AddWorkflowState, KnownEntryKind, ReviewDefaults, ReviewLane,
    SourceSnapshot,
};

fn snapshot(path: &str, bytes: &[u8], is_directory: bool, is_draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions::default(),
        is_regular: !is_directory,
        is_directory,
        is_draft,
    }
}

fn inspect(workflow: &mut AddWorkflowState, source: SourceSnapshot) {
    let path = source.path.display().to_string();
    let _ = workflow.reduce(AddAction::SetSourcePath(path));
    let effects = workflow.reduce(AddAction::Continue);
    let AddEffect::InspectSource { request, .. } = effects.first().expect("inspect effect") else {
        panic!("source continuation must ask the host for one snapshot: {effects:?}");
    };
    let _ = workflow.reduce(AddAction::SourceInspected {
        request: *request,
        result: Ok(source),
    });
}

fn ambiguous(is_draft: bool) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, snapshot("mystery.xyz", b"opaque text\n", false, is_draft));
    assert_eq!(workflow.stage(), AddStage::Kind);
    workflow
}

#[test]
fn test_ask_kind_plain_lists_sorted_interpreted_plus_exe_and_prompt() {
    let workflow = ambiguous(false);
    assert_eq!(
        workflow.kind_picker().unwrap().choices(),
        [
            KnownEntryKind::Fish,
            KnownEntryKind::JavaScript,
            KnownEntryKind::Lua,
            KnownEntryKind::Perl,
            KnownEntryKind::PowerShell,
            KnownEntryKind::Python,
            KnownEntryKind::R,
            KnownEntryKind::Ruby,
            KnownEntryKind::Shell,
            KnownEntryKind::TypeScript,
            KnownEntryKind::Executable,
            KnownEntryKind::Prompt,
        ]
    );
}

#[test]
fn test_ask_kind_plain_no_exe_when_offer_exe_false() {
    let workflow = ambiguous(true);
    let picker = workflow.kind_picker().unwrap();
    assert!(!picker.offers(KnownEntryKind::Executable));
    assert_eq!(picker.choices().last(), Some(&KnownEntryKind::Prompt));
    assert_eq!(picker.choices().len(), 11);
}

#[test]
fn test_ask_kind_plain_shebang_question_variant() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(
        &mut workflow,
        snapshot(
            "mystery.xyz",
            b"#!/usr/bin/env florblang\necho hi\n",
            false,
            false,
        ),
    );
    assert_eq!(workflow.stage(), AddStage::Kind);
    let picker = workflow.kind_picker().unwrap();
    assert_eq!(picker.filename(), "mystery.xyz");
    assert!(picker.has_shebang());
}

#[test]
fn test_ask_kind_plain_returns_the_picked_language() {
    let mut workflow = ambiguous(false);
    let effects = workflow.reduce(AddAction::PickKind(Some(KnownEntryKind::Shell)));
    assert!(effects.is_empty());
    assert_eq!(workflow.stage(), AddStage::Review);
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
    assert_eq!(workflow.review().unwrap().lane(), ReviewLane::Script);
}

#[test]
fn test_ask_kind_plain_returns_exe_and_prompt() {
    for (kind, lane) in [
        (KnownEntryKind::Executable, ReviewLane::Executable),
        (KnownEntryKind::Prompt, ReviewLane::Prompt),
    ] {
        let mut workflow = ambiguous(false);
        let effects = workflow.reduce(AddAction::PickKind(Some(kind)));
        assert!(effects.is_empty(), "kind={kind:?}: {effects:?}");
        assert_eq!(workflow.stage(), AddStage::Review, "kind={kind:?}");
        let review = workflow.review().unwrap();
        assert_eq!(review.kind(), kind);
        assert_eq!(review.lane(), lane);
    }
}

#[test]
fn test_unknown_tui_form_pick_routes_to_the_kind() {
    let mut workflow = ambiguous(false);
    let picker = workflow.kind_picker().unwrap();
    assert_eq!(picker.filename(), "mystery.xyz");
    assert!(picker.offers(KnownEntryKind::Executable));
    assert!(!picker.has_shebang());
    let _ = workflow.reduce(AddAction::PickKind(Some(KnownEntryKind::Shell)));
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
}

#[test]
fn test_unknown_tui_form_shebang_flag_forwarded() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(
        &mut workflow,
        snapshot(
            "mystery.xyz",
            b"#!/usr/bin/env florblang\necho hi\n",
            false,
            false,
        ),
    );
    assert!(workflow.kind_picker().unwrap().has_shebang());
    let _ = workflow.reduce(AddAction::PickKind(Some(KnownEntryKind::Shell)));
    assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
}

#[test]
fn test_md_tui_form_passes_suggested_prompt() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, snapshot("notes.md", b"hello\n", false, false));
    let picker = workflow.kind_picker().expect("plain .md must ask for a kind");
    assert_eq!(picker.filename(), "notes.md");
    assert_eq!(picker.suggested(), Some(KnownEntryKind::Prompt));
}

#[test]
fn test_unknown_tui_form_pick_exe_hosts_the_review_panel() {
    let mut workflow = ambiguous(false);
    let _ = workflow.reduce(AddAction::PickKind(Some(KnownEntryKind::Executable)));
    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().unwrap();
    assert_eq!(review.kind(), KnownEntryKind::Executable);
    assert_eq!(review.lane(), ReviewLane::Executable);
    assert_eq!(review.source().path.file_name().unwrap().to_string_lossy(), "mystery.xyz");
}

#[test]
fn test_exe_flag_tui_form_hosts_the_panel_and_prefills_flags() {
    let defaults = ReviewDefaults {
        name: Some("given".to_owned()),
        description: Some("prewritten".to_owned()),
        ..ReviewDefaults::default()
    };
    let mut workflow = AddWorkflowState::new(Vec::new()).with_review_defaults(defaults);
    inspect(&mut workflow, snapshot("tool", b"#!/bin/sh\necho hi\n", true, false));
    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().unwrap();
    assert_eq!(review.kind(), KnownEntryKind::Executable);
    assert_eq!(review.lane(), ReviewLane::Executable);
    assert_eq!(review.name(), "given");
    assert_eq!(review.description(), "prewritten");
}

#[test]
fn test_add_unknown_directory_tui_hosts_exe_review_with_no_line_confirm() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    inspect(&mut workflow, snapshot("bundle.dir", b"", true, false));
    assert_eq!(workflow.stage(), AddStage::Review);
    let review = workflow.review().unwrap();
    assert_eq!(review.kind(), KnownEntryKind::Executable);
    assert_eq!(review.lane(), ReviewLane::Executable);
    assert_eq!(review.source().path, std::path::PathBuf::from("bundle.dir"));
}
