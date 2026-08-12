//! Exact completeness guard for Python `tests/test_healthcheck.py` at `main@206f9ef`.

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("port_test_healthcheck.rs");
const PYTHON_TESTS: &[&str] = &[
    "test_entry_drifted_true_for_managed_placeholder_gone_from_prompt",
    "test_entry_drifted_false_when_prompt_body_unreadable",
    "test_entry_drifted_false_for_insertion_off_prompt",
    "test_collect_reports_every_category_and_excludes_double_reports",
    "test_collect_double_report_exclusion_continues_not_breaks",
    "test_collect_clean_library_reports_nothing",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_healthcheck_contract_has_the_same_named_executable_rust_oracle_in_order() {
    let actual = syn::parse_file(SOURCE)
        .expect("healthcheck parity target must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(PYTHON_TESTS.len(), 6);
    assert_eq!(actual, expected);
}
