//! Exact-name completeness gate for Python v0.4 `tests/test_js_inject.py` at `main@206f9ef`.
use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_int_injects_a_bare_number",
    "test_float_injects_a_bare_number",
    "test_string_injects_a_json_dumps_literal",
    "test_string_json_escapes_quote_backslash_newline",
    "test_cjk_and_emoji_escape_to_valid_js",
    "test_bool_injects_true_or_false_lowercase",
    "test_rewrites_every_same_name_occurrence",
    "test_same_name_nonliteral_declaration_is_not_a_target",
    "test_ts_temp_copy_has_ts_suffix",
    "test_injected_copy_carries_the_origins_module_flavor",
    "test_missing_target_is_drift_not_value_error",
    "test_bad_int_value_raises_value_error",
    "test_bad_float_and_non_finite_are_refused",
    "test_bad_bool_value_raises_value_error",
    "test_no_values_writes_nothing",
    "test_value_for_unmanaged_name_is_ignored",
    "test_mjs_origin_esm_copy_survives_gate2_before_any_package_json",
    "test_gate2_failure_removes_the_temp_copy",
    "test_injected_copy_is_0600",
    "test_injected_const_reaches_the_child",
    "test_injected_string_reaches_the_child",
    "test_run_injects_and_executes_end_to_end",
    "test_execute_runs_a_js_entry_offline_plan",
    "test_execute_maps_a_drifted_js_definition_to_drift",
    "test_execute_refuses_a_bad_value_before_launch",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_offline_gate_refuses_a_corrupted_injection",
    "test_resolve_runner_finds_first_installed",
    "test_resolve_runner_none_when_nothing_installed",
    "test_resolve_runner_respects_pinned_interpreter_and_normalizes",
    "test_gate_node_skips_ts_suffix",
    "test_gate_node_skips_when_runner_is_not_node",
    "test_gate_node_skips_when_no_runner_installed",
    "test_gate_node_passes_on_returncode_zero",
    "test_gate_node_raises_on_nonzero",
    "test_gate_node_raises_on_nonzero_with_empty_stderr",
    "test_gate_node_survives_a_spawn_failure",
    "test_execute_syntax_gate_failure_never_launches",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("JS injection port source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test(&function.attrs) && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn test_js_inject_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 25);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 12);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 25);
    assert_eq!(closed.len(), 12);
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 37);

    let mut actual = names(include_str!("../../skit-language/tests/port_test_js_inject.rs"));
    for source in [
        include_str!("port_test_js_inject_staging_basic.rs"),
        include_str!("port_test_js_inject_offline_plan.rs"),
        include_str!("port_test_js_inject_bad_value.rs"),
        include_str!("port_test_js_inject_drift_case.rs"),
        include_str!("port_test_js_inject_gate2_failure.rs"),
        include_str!("port_test_js_inject_mjs_gate.rs"),
        include_str!("port_test_js_inject_child_const.rs"),
        include_str!("port_test_js_inject_child_string.rs"),
        include_str!("port_test_js_inject_run_e2e.rs"),
    ] {
        actual.extend(names(source));
    }
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "JS injection executable parity is incomplete or mislabeled");
    assert!(actual.is_disjoint(&closed));
}
