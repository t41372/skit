//! Mechanical completeness guard for Python v0.4 `tests/test_default_semantics_review_fixes.py`.
//!
//! This is not behavioral coverage. Seventeen Python contracts have equal-or-stronger executable
//! public-surface Rust oracles. One Python test deliberately constructs an impossible parser state
//! through a private synthetic analyzer; Rust's equivalent `reconcile_analysis` is private and this
//! test-only branch will not expose production internals or reimplement the algorithm in test code.
//! That blocked name is therefore required to remain *absent* rather than being faked by a stub.

use std::collections::{BTreeMap, BTreeSet};

use syn::{Attribute, Item};

const EXECUTABLE_PYTHON_TESTS: &[&str] = &[
    "test_secret_with_an_empty_source_literal_is_still_delivered",
    "test_secret_field_never_delivers_empty",
    "test_input_binding_with_a_default_is_delivered",
    "test_main_guard_override_receives_the_unchanged_default",
    "test_envdefault_default_that_no_longer_fits_the_type_is_not_published",
    "test_int_shaped_literal_still_refreshes_a_str_envdefault",
    "test_secret_source_literal_is_absent_from_reconcile_and_json",
    "test_preset_from_last_saves_effective_values_after_an_all_defaults_run",
    "test_preset_from_last_still_refuses_an_entry_that_never_ran",
    "test_preset_from_last_pins_the_default_that_actually_ran",
    "test_preset_from_legacy_run_without_snapshot_refuses_to_guess",
    "test_last_used_filters_the_default_but_keeps_a_delivered_empty",
    "test_run_save_preset_stores_a_default_equal_value_verbatim",
    "test_resync_and_secret_in_one_edit_drops_the_refreshed_literal",
    "test_final_no_secret_in_same_edit_keeps_the_public_default",
    "test_shell_colon_envdefaults_do_not_claim_to_deliver_empty",
    "test_shell_noncolon_envdefaults_genuinely_deliver_empty",
];

const BLOCKED_PRIVATE_SYNTHETIC_TEST: &str =
    "test_const_default_that_no_longer_fits_the_declared_type_is_not_published";

const SOURCES: &[&str] = &[
    include_str!("../../skit-store/tests/port_test_default_review_pipeline.rs"),
    include_str!("../../skit-language/tests/port_test_reconcile_defaults.rs"),
    include_str!("../../skit-store/tests/port_test_default_delivery_bridge.rs"),
    include_str!("port_test_default_review_cli.rs"),
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_executable_default_review_python_contract_has_one_real_rust_oracle() {
    let expected = EXECUTABLE_PYTHON_TESTS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 17, "the executable Python names must be unique");

    let mut counts = BTreeMap::<String, usize>::new();
    for source in SOURCES {
        let parsed = syn::parse_file(source).unwrap();
        for item in parsed.items {
            if let Item::Fn(function) = item
                && is_test(&function.attrs)
            {
                let name = function.sig.ident.to_string();
                if expected.contains(name.as_str()) || name == BLOCKED_PRIVATE_SYNTHETIC_TEST {
                    *counts.entry(name).or_default() += 1;
                }
            }
        }
    }

    for name in EXECUTABLE_PYTHON_TESTS {
        assert_eq!(
            counts.get(*name),
            Some(&1),
            "Python oracle {name} must have exactly one executable Rust test"
        );
    }
    assert_eq!(
        counts.get(BLOCKED_PRIVATE_SYNTHETIC_TEST),
        None,
        "the private synthetic branch must not be papered over with a fake Rust test"
    );
}

#[test]
fn blocked_default_review_contract_is_not_counted_as_coverage() {
    assert_eq!(BLOCKED_PRIVATE_SYNTHETIC_TEST, "test_const_default_that_no_longer_fits_the_declared_type_is_not_published");
    assert_eq!(EXECUTABLE_PYTHON_TESTS.len(), 17);
    assert_eq!(EXECUTABLE_PYTHON_TESTS.len() + 1, 18);
}
