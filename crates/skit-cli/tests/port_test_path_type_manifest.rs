//! Mechanical completeness guard for Python v0.4 `tests/test_path_type.py`.
//!
//! Behavioral coverage lives in the five layer-specific files. This guard only proves that the 14
//! frozen Python test names remain executable exactly once; Rust-only strengthening is excluded.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const DOMAIN: &str = include_str!("../../skit-domain/tests/port_test_path_type.rs");
const APPLICATION: &str = include_str!("../../skit-application/tests/port_test_path_type.rs");
const LANGUAGE: &str = include_str!("../../skit-language/tests/port_test_path_reconcile.rs");
const UI: &str = include_str!("../../skit-ui/tests/port_test_path_type.rs");
const CLI: &str = include_str!("port_test_path_type.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_path_is_an_allowed_type",
    "test_unknown_type_still_degrades_to_str",
    "test_block_round_trip_carries_path",
    "test_meta_round_trip_carries_path",
    "test_coerce_default_path_keeps_raw_string",
    "test_edit_declared_accepts_path_type",
    "test_reconcile_path_over_str_const_is_refinement",
    "test_reconcile_path_over_int_const_is_drift",
    "test_resync_preserves_declared_path",
    "test_resync_still_corrects_real_type_drift",
    "test_formfield_carries_path_for_every_delivery",
    "test_degraded_flag_field_still_renders_free_text",
    "test_validate_value_path_is_free_text",
    "test_type_label_path",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
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
fn every_python_path_type_test_has_exactly_one_executable_rust_oracle() {
    let all = [DOMAIN, APPLICATION, LANGUAGE, UI, CLI]
        .into_iter()
        .flat_map(names)
        .collect::<Vec<_>>();
    let python = all
        .iter()
        .filter(|name| name.starts_with("test_"))
        .cloned()
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<_>>();
    let actual = python.iter().cloned().collect::<BTreeSet<_>>();

    assert_eq!(PYTHON_TESTS.len(), 14);
    assert_eq!(python.len(), 14, "duplicate or missing Python path-type oracle: {python:?}");
    assert_eq!(actual, expected);

    let additive = all
        .iter()
        .filter(|name| !name.starts_with("test_"))
        .collect::<Vec<_>>();
    assert!(
        additive
            .iter()
            .all(|name| name.starts_with("rust_additive_")),
        "non-Python tests must be explicitly labeled additive: {additive:?}"
    );
}
