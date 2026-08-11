//! Mechanical completeness guard for Python v0.4 `tests/test_shell_getopts.py`.
//!
//! This is not behavioral coverage. `port_test_shell_getopts.rs` owns the behavior; this test only
//! prevents a frozen Python oracle from silently disappearing or being renamed.

use syn::{Attribute, Item};

const PYTHON_TESTS: &[&str] = &[
    "test_value_and_bool_flags",
    "test_leading_colon_silent_mode_is_skipped",
    "test_non_alphanumeric_characters_are_skipped",
    "test_repeated_letter_keeps_first",
    "test_dynamic_optstring_degrades_to_dynamic",
    "test_getopts_without_optstring_is_none",
    "test_no_getopts_is_none",
    "test_unparseable_script_is_none",
    "test_secret_letter_is_not_special",
    "test_plan_reads_getopts_and_assembles_flags",
    "test_plan_dynamic_getopts_degrades_with_reason",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_shell_getopts_test_has_the_same_named_executable_rust_oracle() {
    let parsed = syn::parse_file(include_str!("port_test_shell_getopts.rs")).unwrap();
    let actual = parsed
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(PYTHON_TESTS.len(), 11);
    assert_eq!(actual, expected);
}
