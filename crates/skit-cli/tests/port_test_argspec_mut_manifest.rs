//! Exact-name completeness guard for Python v0.4 `tests/test_argspec_mut.py`.
//!
//! This file is not behavioral coverage. The three executable tests remain independently named so
//! broader argparse/Click/Typer suites cannot silently substitute for this mutation contract.

use std::{fs, path::Path};

const EXPECTED: &[&str] = &[
    "test_argparse_field_binding_is_none_and_delivery_is_flag",
    "test_click_field_binding_is_none_and_delivery_is_flag",
    "test_typer_field_binding_is_none_and_delivery_is_flag",
];

#[test]
fn argspec_axis_mutation_oracles_match_the_frozen_python_module_exactly() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source =
        fs::read_to_string(repo.join("crates/skit-form/tests/port_test_argspec_mut_exact.rs"))
            .unwrap();
    let file = syn::parse_file(&source).unwrap();
    let names = file
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
