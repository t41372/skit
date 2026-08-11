//! Mechanical completeness guard for Python v0.4 `tests/test_run_set.py`.
//!
//! This file is deliberately not behavioral coverage. The behavioral oracle is
//! `port_test_run_set.rs`; this guard only prevents one of the frozen Python tests from silently
//! disappearing or being renamed later.

use syn::{Attribute, Item};

const PYTHON_TESTS: &[&str] = &[
    "test_set_inject_values_non_interactive",
    "test_set_makes_command_placeholders_runnable",
    "test_set_wins_over_preset",
    "test_set_satisfies_required_argparse_field",
    "test_set_saves_preset_with_dry_run_without_running",
    "test_save_preset_on_field_less_entry_refused_saves_nothing",
    "test_save_preset_deferred_until_a_real_run_is_accepted",
    "test_save_preset_not_written_when_launch_is_refused",
    "test_save_preset_dry_run_validation_failure_writes_nothing",
    "test_set_secret_never_persisted_and_masked_in_dry_run",
    "test_set_token_values_expand_at_assembly",
    "test_set_malformed_exits_2_with_exact_message",
    "test_set_value_may_contain_equals_signs",
    "test_set_key_is_stripped",
    "test_set_unknown_name_exits_2_and_lists_valid",
    "test_set_on_entry_without_fields_lists_a_dash",
    "test_set_with_raw_is_a_usage_conflict",
    "test_preset_with_raw_is_a_usage_conflict",
    "test_save_preset_with_raw_is_a_usage_conflict",
    "test_raw_never_replays_last_extra_args",
    "test_set_bad_typed_value_exits_125",
    "test_set_bad_value_fails_before_the_form_opens",
    "test_set_empty_value_on_required_placeholder_exits_125",
    "test_interactive_form_skips_set_fields",
    "test_interactive_all_fields_set_skips_the_form_entirely",
    "test_save_preset_no_fields_refused_before_any_form",
    "test_save_preset_persists_when_ctrl_c_ends_an_accepted_run",
];

fn is_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn every_python_run_set_test_has_the_same_named_executable_rust_oracle() {
    let parsed = syn::parse_file(include_str!("port_test_run_set.rs")).unwrap();
    let actual = parsed
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if is_test(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(PYTHON_TESTS.len(), 27, "frozen Python module has 27 tests");
    assert_eq!(actual, expected, "Python run-set oracle inventory drifted");
}
