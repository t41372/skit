//! Completeness guard for Python v0.4 `tests/test_js_inject_mut.py`.
//!
//! Four contracts are public source-injection observables. Three Gate2/tempfile sequencing tests
//! directly patch Python-private runner helpers and have no equivalent public Rust seam. They stay
//! explicitly blocked rather than being replaced by weaker tests. This manifest is not coverage.

use std::collections::BTreeMap;

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-language/tests/port_test_javascript_injection_mut.rs");

const EXECUTABLE: &[&str] = &[
    "test_bad_value_error_carries_the_raw_value_and_type",
    "test_destructuring_binding_is_never_an_injection_target",
    "test_a_spec_without_a_value_does_not_stop_later_injection",
    "test_all_drifted_targets_are_collected_into_one_error",
];

const BLOCKED_PRIVATE: &[&str] = &[
    "test_refused_copy_is_cleaned_even_when_gate_raises",
    "test_accepted_copy_is_gated_after_write_before_return",
    "test_accepted_copy_gate_runs_exactly_once_after_the_write",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names() -> Vec<String> {
    syn::parse_file(SOURCE)
        .expect("JavaScript injection parity source must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn javascript_injection_mut_has_exactly_four_public_python_oracles() {
    let mut counts = BTreeMap::<String, usize>::new();
    for name in names() {
        *counts.entry(name).or_default() += 1;
    }
    for expected in EXECUTABLE {
        assert_eq!(
            counts.get(*expected).copied().unwrap_or_default(),
            1,
            "public JavaScript injection contract {expected} must map exactly once"
        );
    }
    assert_eq!(EXECUTABLE.len(), 4);
}

#[test]
fn javascript_injection_private_gate2_contracts_are_not_faked_as_coverage() {
    let names = names();
    for blocked in BLOCKED_PRIVATE {
        assert!(
            !names.iter().any(|name| name == blocked),
            "{blocked} depends on Python-private Gate2/tempfile sequencing; do not fake it through a weaker Rust test"
        );
    }
    assert_eq!(BLOCKED_PRIVATE.len(), 3);
}
