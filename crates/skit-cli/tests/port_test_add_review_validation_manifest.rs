//! Completeness guard for Python `tests/test_add_review_validation.py` at `main@206f9ef`.
//!
//! Nine contracts have executable Rust behavior tests. Python's artificial non-copy fresh-draft
//! branch is architecture-closed: Rust's typed review forces every owned draft to Copy both when
//! storage is edited and again when the CreateEntry is built. The guard proves that invariant and
//! forbids a fake same-named test for an unreachable state.

use std::{collections::BTreeSet, path::PathBuf};

use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_ui::{KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot};
use syn::{Attribute, Item};

const UI_SOURCE: &str = include_str!("../../skit-ui/tests/port_test_add_review_validation.rs");
const HOST_SOURCE: &str = include_str!("port_test_add_review_validation_draft.rs");
const ARCHITECTURE_CLOSED: &str = "test_fresh_draft_keeps_the_file_when_the_entry_is_not_a_copy";
const EXECUTABLE: &[&str] = &[
    "test_draft_resume_inferred_exe_routes_to_ask_without_program_option",
    "test_fresh_draft_copy_flow_unlinks_the_file",
    "test_candidate_tick_survives_a_noop_edit_rescan",
    "test_edit_source_capture_skips_a_candidate_with_no_checkbox",
    "test_new_candidate_after_a_real_edit_takes_its_default",
    "test_review_dash_python_is_stored_as_automatic",
    "test_review_rejects_a_bad_uv_dep_and_keeps_the_panel_open",
    "test_review_rejects_a_bad_python_constraint_and_keeps_the_panel_open",
    "test_review_does_not_validate_npm_deps",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("add-review validation parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn actual_tests() -> Vec<String> {
    tests(UI_SOURCE)
        .into_iter()
        .chain(tests(HOST_SOURCE))
        .collect()
}

#[test]
fn nine_reachable_python_add_review_validation_contracts_are_executable_once() {
    let actual = actual_tests();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(EXECUTABLE.len(), 9);
    assert_eq!(
        actual.len(),
        9,
        "unexpected extra or missing add-review tests"
    );
    assert_eq!(
        actual_set.len(),
        actual.len(),
        "duplicate names hide a missing contract"
    );
    assert_eq!(actual_set, expected);
    assert!(!actual.iter().any(|name| name == ARCHITECTURE_CLOSED));
}

#[test]
fn fresh_owned_drafts_cannot_reach_a_non_copy_commit_in_rust() {
    let source = SourceSnapshot {
        path: PathBuf::from("skit-new-draft"),
        source_record: "skit-new-draft".to_owned(),
        bytes: b"print(1)\n".to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(0o644),
        },
        is_regular: true,
        is_directory: false,
        is_draft: true,
    };
    let mut review =
        ReviewState::from_source(source, KnownEntryKind::Python, ReviewDefaults::default());

    review.set_storage(StorageMode::Reference);
    assert_eq!(review.storage(), StorageMode::Copy);
    assert_eq!(review.create_entry().unwrap().mode, StorageMode::Copy);
    assert!(
        !actual_tests()
            .iter()
            .any(|name| name == ARCHITECTURE_CLOSED)
    );
}
