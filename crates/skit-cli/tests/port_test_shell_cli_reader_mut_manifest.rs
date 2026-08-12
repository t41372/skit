//! Completeness guard for Python v0.4 `tests/test_shell_cli_reader_mut.py`.
//!
//! Five tests have equivalent public Rust reader observables. Two Python tests probe private helper
//! states that cannot be reached through the real getopts grammar and are therefore explicitly
//! blocked rather than backfilled with weaker public behavior.

use std::{fs, path::Path};

const EXECUTABLE: &[&str] = &[
    "test_getopts_found_after_an_earlier_non_getopts_command",
    "test_trailing_value_marker_makes_a_str_flag",
    "test_repeated_letter_emits_exactly_one_field",
    "test_option_binding_and_delivery_and_flag",
    "test_bool_flag_shape_from_a_bare_letter",
];

const BLOCKED: &[&str] = &[
    "test_option_carries_secret_from_the_name",
    "test_find_getopts_dynamic_optstring_returns_empty_literal_marker",
];

fn test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function)
                if function
                    .attrs
                    .iter()
                    .any(|attribute| attribute.path().is_ident("test")) =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn shell_cli_reader_mutation_coverage_is_five_executable_two_blocked() {
    assert_eq!(EXECUTABLE.len() + BLOCKED.len(), 7);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(
        repo.join("crates/skit-form/tests/port_test_shell_cli_reader_mut_exact.rs"),
    )
    .unwrap();
    assert_eq!(
        test_names(&source),
        EXECUTABLE
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );

    for blocked in BLOCKED {
        assert!(
            !source.contains(&format!("fn {blocked}(")),
            "blocked private-helper contract {blocked} must not be faked as executable coverage"
        );
    }
}
