//! Completeness guard for Python `tests/test_edit.py` at `main@206f9ef`.
//!
//! Twelve contracts have executable Rust oracles. Two Python-only helper warning contracts are
//! blocked narrowly: Rust has no public in-memory edit operation that returns warning tokens for
//! "already managed / not a candidate" or "not managed". Mapping those names to the Rust CLI's
//! different fatal-error behavior would be dishonest, so this guard forbids impersonating them.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const RECONCILE: &str = include_str!("../../skit-language/tests/port_test_edit_reconcile.rs");
const CLI: &str = include_str!("port_test_edit.rs");

const BLOCKED_NO_PUBLIC_WARNING_SEAM: &[&str] = &[
    "test_add_already_managed_and_not_candidate_warn",
    "test_no_secret_and_missing_name_warns",
];

const EXECUTABLE: &[&str] = &[
    "test_resync_drops_missing_and_keeps_matching",
    "test_resync_updates_changed_type_preserving_customization",
    "test_add_brings_candidate_under_management",
    "test_add_input_candidate_by_display_name",
    "test_remove_and_secret_toggles",
    "test_edit_specs_is_pure_no_mutation_of_input_list",
    "test_cli_resync_prunes_and_persists",
    "test_cli_secret_and_prompt_persist",
    "test_cli_params_view_no_ops",
    "test_cli_bad_prompt_is_warned_not_fatal",
    "test_cli_params_edit_reference_refused",
    "test_cli_edit_command_entry_has_no_source",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("edit parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn actual() -> Vec<String> {
    tests(RECONCILE).into_iter().chain(tests(CLI)).collect()
}

#[test]
fn twelve_reachable_python_edit_contracts_are_executable_once() {
    let actual = actual();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(EXECUTABLE.len(), 12);
    assert_eq!(
        actual.len(),
        12,
        "unexpected extra or missing edit parity tests"
    );
    assert_eq!(
        actual_set.len(),
        actual.len(),
        "duplicate names hide a missing edit contract"
    );
    assert_eq!(actual_set, expected);
}

#[test]
fn helper_warning_contracts_without_a_public_rust_warning_channel_are_not_impersonated() {
    let actual = actual();
    assert_eq!(BLOCKED_NO_PUBLIC_WARNING_SEAM.len(), 2);
    for blocked in BLOCKED_NO_PUBLIC_WARNING_SEAM {
        assert!(
            !actual.iter().any(|name| name == blocked),
            "{blocked} has no equivalent public Rust in-memory warning seam; do not replace it with a different CLI failure"
        );
    }
    assert_eq!(EXECUTABLE.len() + BLOCKED_NO_PUBLIC_WARNING_SEAM.len(), 14);
}
