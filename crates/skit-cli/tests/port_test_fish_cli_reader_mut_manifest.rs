//! Exact-name completeness guard for Python v0.4 `tests/test_fish_cli_reader_mut.py`.
//!
//! The eight behavioral tests live in skit-form. This manifest freezes their Python order so no
//! broader Fish onboarding test can silently substitute for a mutation oracle.

use std::{fs, path::Path};

const EXPECTED: &[&str] = &[
    "test_find_argparse_skips_a_lone_leading_prefix",
    "test_find_argparse_advances_past_every_stacked_prefix",
    "test_flag_spec_binding_and_delivery",
    "test_valueless_flag_is_a_false_default_bool",
    "test_single_required_value_flag_is_not_multiple",
    "test_single_char_short_flag_is_not_degraded",
    "test_plain_long_flag_is_not_degraded",
    "test_validator_is_dropped_from_the_first_bang_forward",
];

#[test]
fn fish_cli_reader_mutation_oracles_match_the_frozen_python_module_exactly() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(
        repo.join("crates/skit-form/tests/port_test_fish_cli_reader_mut_exact.rs"),
    )
    .unwrap();
    let names = syn::parse_file(&source)
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
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        EXPECTED
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
}
