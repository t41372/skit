//! Exact completeness guard for Python `tests/test_add_feedback_contracts.py` at `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const LANGUAGE: &str = include_str!("../../skit-language/tests/port_test_add_feedback_contracts.rs");
const TUI: &str = include_str!("../../skit-tui/tests/port_test_add_feedback_contracts.rs");
const CLI: &str = include_str!("port_test_add_feedback_contracts.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_ref_on_kept_draft_is_refused_and_keeps_it",
    "test_ref_on_a_normal_file_still_works",
    "test_prompt_draft_with_shebang_body_resumes_as_prompt",
    "test_py_draft_with_shebang_body_still_resumes_as_shell",
    "test_python_ask_label_names_the_pin_and_enter_records_it",
    "test_python_ask_dash_records_automatic_even_with_a_pin",
    "test_python_ask_label_is_leave_empty_without_a_pin",
    "test_micro_version_pin_unit",
    "test_micro_versioned_shebang_lands_in_stored_pep723",
    "test_shebangless_unknown_uses_the_isnt_a_script_voice",
    "test_shebang_unknown_uses_the_names_no_interpreter_voice",
    "test_add_hints_suppresses_argv_when_a_framework_was_detected",
    "test_add_hints_prints_argv_when_no_framework",
    "test_dynamic_optstring_with_argv_names_extra_arguments_once",
    "test_add_records_only_third_party_deps_not_sibling_modules",
    "test_resolve_python_metadata_without_script_dir_does_not_filter",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn tests(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("add-feedback parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn all_sixteen_python_add_feedback_contracts_are_executable_once() {
    let actual = tests(LANGUAGE)
        .into_iter()
        .chain(tests(TUI))
        .chain(tests(CLI))
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 16);
    assert_eq!(actual.len(), 16, "unexpected extra or missing add-feedback tests");
    assert_eq!(actual_set.len(), actual.len(), "duplicate names hide a missing contract");
    assert_eq!(actual_set, expected);
}
