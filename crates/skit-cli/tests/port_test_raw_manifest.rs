//! Completeness guard for Python v0.4 `tests/test_raw.py` at
//! `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//!
//! Behavioral strength lives in `port_test_raw.rs`; this file only prevents silent loss or rename.

use syn::{Attribute, Item};

const PYTHON_TESTS: &[&str] = &[
    "test_raw_skips_form_and_injection",
    "test_default_run_injects",
    "test_no_values_runs_copy_directly",
    "test_raw_does_not_leave_injected_artifact",
    "test_normal_run_cleans_injected_artifact",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_raw_test_has_the_same_named_executable_rust_oracle() {
    let parsed = syn::parse_file(include_str!("port_test_raw.rs")).unwrap();
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
    assert_eq!(actual, expected);
}
