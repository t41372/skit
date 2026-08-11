//! Completeness guard for Python v0.4 `tests/test_source_default_semantics.py`.
//!
//! This manifest is not behavioral coverage. It only prevents executable Python contracts from
//! disappearing or being mapped twice, and prevents the two synthetic-analyzer-only contracts from
//! being faked as Rust tests while no equivalent public reconcile-from-analysis seam exists.

use std::collections::BTreeMap;

use syn::{Attribute, Item};

const FORM_SOURCE: &str = include_str!("../../skit-form/tests/port_test_source_defaults.rs");
const LANGUAGE_SOURCE: &str = include_str!("../../skit-language/tests/port_test_reconcile_defaults.rs");
const STORE_SOURCE: &str = include_str!("../../skit-store/tests/port_test_source_default_pipeline.rs");
const CLI_SOURCE: &str = include_str!("port_test_source_default_semantics.rs");

const EXECUTABLE_PYTHON_TESTS: &[&str] = &[
    "test_plan_refreshes_a_stale_block_default_from_the_python_body",
    "test_plan_refreshes_a_stale_shell_envdefault_from_the_body",
    "test_reconcile_records_current_default_for_an_ok_const",
    "test_reconcile_records_current_default_for_an_ok_envdefault",
    "test_reconcile_omits_current_default_for_a_type_changed_const",
    "test_resync_writes_source_default_into_ok_and_type_changed_specs",
    "test_resync_current_default_and_rebind_and_untouched_input_share_one_pass",
    "test_assemble_injects_a_value_that_equals_the_source_default",
    "test_assemble_injects_the_expansion_of_an_untouched_token_default",
    "test_assemble_inject_delivers_empty_string_when_cleared",
    "test_assemble_env_delivers_empty_string_when_cleared",
    "test_assemble_flag_delivers_empty_string_when_cleared",
    "test_delivers_empty_matrix",
    "test_last_used_drops_values_equal_to_their_default",
    "test_last_used_keeps_a_cleared_empty_only_where_it_was_delivered",
    "test_save_after_run_persists_via_the_remembered_rule",
    "test_input_binding_flag_reflects_the_decl_binding",
];

const BLOCKED_SYNTHETIC_TESTS: &[&str] = &[
    "test_reconcile_ok_const_without_a_default_is_not_recorded",
    "test_reconcile_ok_envdefault_without_a_default_is_not_recorded",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn rust_test_names(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("parity source must parse as Rust");
    file.items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn test_source_default_semantics_python_contracts_have_one_real_rust_oracle_each() {
    let mut counts = BTreeMap::<String, usize>::new();
    for source in [FORM_SOURCE, LANGUAGE_SOURCE, STORE_SOURCE, CLI_SOURCE] {
        for name in rust_test_names(source) {
            *counts.entry(name).or_default() += 1;
        }
    }

    for expected in EXECUTABLE_PYTHON_TESTS {
        assert_eq!(
            counts.get(*expected).copied().unwrap_or(0),
            1,
            "Python source-default contract {expected} must map to exactly one executable Rust test"
        );
    }
    assert_eq!(EXECUTABLE_PYTHON_TESTS.len(), 17);
}

#[test]
fn test_source_default_semantics_synthetic_only_contracts_are_not_faked_as_coverage() {
    let mut names = Vec::new();
    for source in [FORM_SOURCE, LANGUAGE_SOURCE, STORE_SOURCE, CLI_SOURCE] {
        names.extend(rust_test_names(source));
    }
    for blocked in BLOCKED_SYNTHETIC_TESTS {
        assert!(
            !names.iter().any(|name| name == blocked),
            "{blocked} needs an equivalent public Rust reconcile-from-analysis seam; do not fake it with a weaker test"
        );
    }
    assert_eq!(BLOCKED_SYNTHETIC_TESTS.len(), 2);
}
