//! Exact completeness guard for Python `tests/test_dependency_command_contracts.py` at
//! `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("port_test_dependency_command_contracts.rs");
const PYTHON_TESTS: &[&str] = &[
    "test_two_flags_together_are_both_named_and_joined",
    "test_kind_exe_alone_names_only_kind_exe",
    "test_js_deps_python_dash_is_refused_as_inapplicable",
    "test_js_deps_python_none_is_refused_as_inapplicable",
    "test_js_deps_python_empty_string_is_refused_as_inapplicable",
    "test_python_deps_python_dash_is_still_automatic",
    "test_store_npm_spec_plus_dash_reaches_the_npm_refusal",
    "test_store_uv_spec_plus_dash_normalizes",
    "test_store_npm_spec_plus_empty_string_reaches_the_npm_refusal",
    "test_store_npm_spec_plus_none_deps_edit_is_not_refused",
    "test_add_python_belt_rejects_a_bad_dep_before_any_entry_exists",
    "test_add_python_belt_rejects_a_bad_python_before_any_entry_exists",
    "test_add_python_belt_drops_a_whitespace_dep_from_the_block",
    "test_add_python_belt_with_no_deps_is_unchanged",
    "test_deps_python_only_prints_the_constraint_line_not_the_deps_line",
    "test_deps_python_only_dash_reports_the_dash_placeholder",
    "test_deps_dep_only_prints_the_deps_line",
    "test_deps_dep_and_python_together_prints_both_axis_lines",
    "test_deps_clear_prints_the_deps_line",
    "test_js_is_npm_flavor_and_python_is_not",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn all_twenty_python_dependency_command_contracts_are_executable_once() {
    let actual = syn::parse_file(SOURCE)
        .expect("dependency-command parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let actual_set = actual.iter().cloned().collect::<BTreeSet<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 20);
    assert_eq!(
        actual.len(),
        20,
        "unexpected extra or missing dependency-command tests"
    );
    assert_eq!(
        actual_set.len(),
        actual.len(),
        "duplicate names hide a missing contract"
    );
    assert_eq!(actual_set, expected);
}
