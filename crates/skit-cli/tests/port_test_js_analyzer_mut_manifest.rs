//! Exact-name completeness guard for Python v0.4 `tests/test_js_analyzer_mut.py`.
//!
//! This is not behavioral coverage. The five exact tests remain separately executable even though
//! broader onboarding tests exercise related source-analysis behavior.

use std::{fs, path::Path};

const EXPECTED: &[&str] = &[
    "test_destructuring_with_literal_value_is_not_a_candidate",
    "test_non_literal_const_is_skipped_but_later_literals_still_land",
    "test_reassigned_const_carries_the_accumulator_demotion_marker",
    "test_augmented_reassigned_const_demotion_marker",
    "test_external_imports_skips_sourceless_export_statements",
];

#[test]
fn javascript_analyzer_mutation_oracles_match_the_frozen_python_module_exactly() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source =
        fs::read_to_string(repo.join("crates/skit-form/tests/port_test_js_analyzer_mut_exact.rs"))
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
