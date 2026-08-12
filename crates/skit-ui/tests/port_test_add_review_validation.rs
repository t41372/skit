//! Frontend-neutral ports from Python `tests/test_add_review_validation.py` at `main@206f9ef`.
//!
//! The two contracts whose observable result is whether an owned draft file is physically unlinked
//! live in the CLI composition-root test module. The eight contracts here stay at the typed review
//! / reducer boundary: validation must stop before a Commit effect exists, and rescan must preserve
//! the user's candidate decisions rather than merely recomputing a plausible panel.

use std::path::PathBuf;

use skit_application::SourcePermissions;
use skit_ui::{
    AddAction, AddEffect, AddProblem, AddStage, AddWorkflowState, DraftSummary, KnownEntryKind,
    ReviewDefaults, ReviewState, SourceSnapshot,
};

fn source(path: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(path),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(0o644),
        },
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn draft(path: &str, bytes: &[u8]) -> SourceSnapshot {
    let mut source = source(path, bytes);
    source.permissions.unix_mode = Some(0o755);
    source.is_draft = true;
    source
}

fn candidate_selected(review: &ReviewState, name: &str) -> bool {
    review
        .candidates()
        .iter()
        .find(|candidate| candidate.declaration.name == name)
        .unwrap_or_else(|| panic!("candidate {name} is missing"))
        .selected
}

#[test]
fn test_draft_resume_inferred_exe_routes_to_ask_without_program_option() {
    let summary = DraftSummary {
        path: PathBuf::from("skit-new-binish"),
        modified: 1,
    };
    let mut workflow = AddWorkflowState::new(vec![summary]);
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let effects = workflow.reduce(AddAction::Continue);
    let [AddEffect::InspectSource { request, .. }] = effects.as_slice() else {
        panic!("resumed draft must ask the host to inspect its source");
    };
    let request = *request;

    let effects = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(draft("skit-new-binish", b"opaque program bytes\n")),
    });

    assert!(effects.is_empty());
    assert_eq!(workflow.stage(), AddStage::Kind);
    let picker = workflow.kind_picker().expect("draft executable inference must route to kind ask");
    assert!(!picker.offers(KnownEntryKind::Executable));
    assert!(
        picker
            .choices()
            .iter()
            .all(|kind| *kind != KnownEntryKind::Executable)
    );
}

#[test]
fn test_candidate_tick_survives_a_noop_edit_rescan() {
    let bytes = b"CITY = \"Taipei\"\nprint(CITY)\n";
    let mut review = ReviewState::from_source(
        source("cand.py", bytes),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert!(candidate_selected(&review, "CITY"));
    review.set_candidate_selected("CITY", false);

    review.rescan(bytes.to_vec());

    assert!(!candidate_selected(&review, "CITY"));
}

#[test]
fn test_edit_source_capture_skips_a_candidate_with_no_checkbox() {
    let bytes = concat!(
        "#!/usr/bin/env bash\n",
        "REGION=us-east-1\n",
        "while getopts \"n:\" o; do case $o in n) NAME=$OPTARG;; esac; done\n",
        "echo \"$REGION $NAME\"\n",
    )
    .as_bytes();
    let mut review = ReviewState::from_source(
        source("opt.sh", bytes),
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );

    assert_eq!(review.modeled_cli_field_count(), Some(1));
    assert!(
        review.candidates().is_empty(),
        "modeled getopts form must suppress source-management checkboxes"
    );

    review.rescan(bytes.to_vec());

    assert_eq!(review.modeled_cli_field_count(), Some(1));
    assert!(review.candidates().is_empty());
}

#[test]
fn test_new_candidate_after_a_real_edit_takes_its_default() {
    let mut review = ReviewState::from_source(
        source("cand2.py", b"CITY = \"Taipei\"\nprint(CITY)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_candidate_selected("CITY", false);

    review.rescan(
        b"CITY = \"Taipei\"\nREGION = \"us-east-1\"\nprint(CITY, REGION)\n".to_vec(),
    );

    assert!(!candidate_selected(&review, "CITY"));
    assert!(candidate_selected(&review, "REGION"));
}

#[test]
fn test_review_dash_python_is_stored_as_automatic() {
    let mut review = ReviewState::from_source(
        source("auto.py", b"print(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_name("autoentry");
    review.set_requires_python("-");

    let entry = review.create_entry().unwrap();

    assert!(entry.settings.requires_python.is_empty());
    let payload = entry.payload.expect("python copy keeps a payload");
    let stored = String::from_utf8(payload.bytes).unwrap();
    assert!(!stored.contains("requires-python"));
}

#[test]
fn test_review_rejects_a_bad_uv_dep_and_keeps_the_panel_open() {
    let mut review = ReviewState::from_source(
        source("baddep.py", b"print(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_name("baddep");
    review.set_dependencies_text("@@@");
    let mut workflow = AddWorkflowState::from_review(review);

    let effects = workflow.reduce(AddAction::Save);

    assert!(effects.is_empty(), "invalid PEP 508 input reached a repository Commit effect");
    assert_eq!(workflow.stage(), AddStage::Review);
    assert!(matches!(
        workflow.problem(),
        Some(AddProblem::InvalidDependency { value }) if value == "@@@"
    ));
    assert!(!workflow.commit_pending());
}

#[test]
fn test_review_rejects_a_bad_python_constraint_and_keeps_the_panel_open() {
    let mut review = ReviewState::from_source(
        source("badpy.py", b"print(1)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_name("badpy");
    review.set_requires_python("not-a-version");
    let mut workflow = AddWorkflowState::from_review(review);

    let effects = workflow.reduce(AddAction::Save);

    assert!(effects.is_empty(), "invalid PEP 440 input reached a repository Commit effect");
    assert_eq!(workflow.stage(), AddStage::Review);
    assert!(matches!(
        workflow.problem(),
        Some(AddProblem::InvalidPythonConstraint { value }) if value == "not-a-version"
    ));
    assert!(!workflow.commit_pending());
}

#[test]
fn test_review_does_not_validate_npm_deps() {
    let mut review = ReviewState::from_source(
        source(
            "tool.js",
            b"import thing from \"@scope/thing\";\nconsole.log(thing);\n",
        ),
        KnownEntryKind::JavaScript,
        ReviewDefaults::default(),
    );
    review.set_name("jstool");
    review.set_dependencies_text("@scope/thing");

    let entry = review.create_entry().unwrap();

    assert_eq!(entry.kind.as_str(), "js");
    assert_eq!(entry.settings.dependencies, ["@scope/thing"]);
}
