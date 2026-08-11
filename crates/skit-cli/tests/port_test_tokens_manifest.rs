//! Mechanical completeness guard for Python v0.4 `tests/test_tokens.py`.
//!
//! This does not count as behavioral coverage. The behavior lives in the application/runtime
//! tests; this guard only makes the 21 frozen Python test names mechanically non-optional.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const APPLICATION_SOURCE: &str =
    include_str!("../../skit-application/tests/port_test_tokens.rs");
const RUNTIME_SOURCE: &str = include_str!("port_test_tokens_runtime.rs");

const APPLICATION_PYTHON_TESTS: &[&str] = &[
    "test_cwd_token",
    "test_today_token",
    "test_now_token",
    "test_env_token_present",
    "test_env_token_missing_raises_with_names",
    "test_multiple_tokens_in_one_value",
    "test_unknown_braces_pass_through",
    "test_double_brace_escapes",
    "test_brace_escapes_false_keeps_double_braces_byte_identical",
    "test_brace_escapes_true_halves_the_pair",
    "test_named_tokens_expand_in_both_brace_modes",
    "test_preview_threads_brace_escapes",
    "test_tilde_expansion_only_at_start",
    "test_tilde_then_tokens_compose",
    "test_plain_text_unchanged",
    "test_preview_success_and_failure",
    "test_has_tokens",
    "test_escape_sequences_mid_string_exact",
    "test_preview_forwards_every_argument",
    "test_escape_deep_in_string_exact",
];

const RUNTIME_PYTHON_TESTS: &[&str] = &["test_default_env_and_now_paths"];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_python_token_test_has_one_executable_rust_oracle() {
    let application = test_names(APPLICATION_SOURCE);
    let application_python = application
        .iter()
        .filter(|name| name.starts_with("test_"))
        .cloned()
        .collect::<Vec<_>>();
    let expected_application = APPLICATION_PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(application_python, expected_application);

    let additive = application
        .iter()
        .filter(|name| !name.starts_with("test_"))
        .collect::<Vec<_>>();
    assert_eq!(additive.len(), 3, "unexpected Rust-only token test count");
    assert!(
        additive
            .iter()
            .all(|name| name.starts_with("rust_additive_")),
        "Rust-only tests must be explicitly labeled additive: {additive:?}"
    );

    let runtime = test_names(RUNTIME_SOURCE);
    assert_eq!(
        runtime,
        RUNTIME_PYTHON_TESTS
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );

    let all_python = APPLICATION_PYTHON_TESTS
        .iter()
        .chain(RUNTIME_PYTHON_TESTS.iter())
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(all_python.len(), 21, "Python token oracle names must be unique");
}
