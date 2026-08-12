//! Exact-name completeness guard for Python v0.4 `tests/test_js_cli_reader_mut.py`.
//!
//! This is not behavioral coverage. The ten real tests live in skit-form and must remain in the
//! same frozen Python order; Rust-additive tests cannot substitute for a missing Python oracle.

use std::{fs, path::Path};

const EXPECTED: &[&str] = &[
    "test_option_spec_skips_computed_pair_then_keeps_reading_the_real_type",
    "test_option_spec_skips_non_pair_then_keeps_reading_the_real_type",
    "test_string_type_yields_a_clean_str_field_not_a_degraded_one",
    "test_identifier_call_that_is_not_parseargs_is_not_read_as_a_surface",
    "test_member_call_that_is_not_parseargs_is_not_read_as_a_surface",
    "test_numeric_option_key_names_no_field",
    "test_read_option_defaults_binding_none_delivery_flag",
    "test_multiple_true_option_sets_both_multiple_and_repeat",
    "test_no_multiple_key_leaves_both_off",
    "test_multiple_false_option_leaves_both_off",
];

fn test_names(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).unwrap();
    file.items
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
        .filter(|name| !name.starts_with("rust_additive_"))
        .collect()
}

#[test]
fn javascript_parseargs_mutation_oracles_match_the_frozen_python_module_exactly() {
    assert_eq!(EXPECTED.len(), 10);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(
        repo.join("crates/skit-form/tests/port_test_js_cli_reader_mut_exact.rs"),
    )
    .unwrap();
    assert_eq!(
        test_names(&source),
        EXPECTED
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
}
