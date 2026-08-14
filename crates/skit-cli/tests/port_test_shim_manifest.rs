//! Exact-name accounting gate for Python v0.4 `tests/test_shim.py` at `main@206f9ef`.
//!
//! Frozen denominator: 38 `def test_` functions. Thirty-five are executable against Rust's public
//! Python semantic injector / real CLI staging. Three directly test Python-private rewrite/fault
//! helpers and are architecture-closed rather than reimplemented inside the Rust tests.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_const_str_injection_preserves_everything_else",
    "test_const_typed_injection",
    "test_main_guard_const",
    "test_input_queue_by_order",
    "test_input_queue_preamble_is_single_line_after_docstring",
    "test_input_queue_exhaustion_falls_back_to_stdin",
    "test_input_queue_in_loop_consumes_by_call_order",
    "test_input_queue_secret_masks_echo",
    "test_input_queue_with_future_import",
    "test_input_queue_combined_with_const_injection",
    "test_missing_value_leaves_script_untouched",
    "test_shadowed_input_is_not_rewritten_and_surfaces_as_drift",
    "test_shadowed_input_leaves_the_call_site_text_intact_when_only_a_const_is_delivered",
    "test_unshadowed_input_is_rewritten_to_the_wrapper",
    "test_drifted_target_raises",
    "test_bad_type_coercion_raises",
    "test_bad_type_coercion_raises_the_value_subclass_not_plain_shim_error",
    "test_drifted_target_raises_plain_shim_error_not_value_subclass",
    "test_multiline_value_span",
    "test_coerce_bool_invalid_raises",
    "test_inject_syntax_error_raises",
    "test_input_order_beyond_calls_is_drift",
    "test_inject_two_identical_prompts_one_deleted_raises_cleanly_never_corrupts",
    "test_inject_duplicate_prompt_winner_only_still_injects_and_compiles",
    "test_inject_specs_sharing_the_same_order_never_double_bind",
    "test_inject_triple_duplicate_specs_same_order_never_double_bind",
    "test_preamble_inserted_at_end_for_no_docstring_no_future",
    "test_multiline_span_replacement",
    "test_const_injection_survives_form_feed_between_targets",
    "test_const_injection_survives_u2028_inside_earlier_string_literal",
    "test_preamble_insertion_survives_form_feed_inside_docstring",
    "test_input_value_follows_prompt_despite_runtime_call_order_diverging_from_source_order",
    "test_input_value_follows_prompt_after_an_earlier_input_is_deleted",
    "test_write_injected_lands_outside_entry_dir",
    "test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    // Direct unit pin for Python's private physical-line splitter. Public source-edit consequences
    // for form-feed/U+2028 are executable above instead of recreating this helper.
    "test_physical_lines_matches_splitlines_on_ordinary_text",
    // Deterministic monkeypatch faults inside Python's private tempfile writer. Rust's staging
    // writer is private to CLI composition and exposes no fdopen/chmod fault port.
    "test_write_injected_cleanup_on_error",
    "test_write_injected_closes_fd_when_chmod_raises",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("Shim port source must parse")
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
fn test_shim_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 35);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 3);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 35, "duplicate executable names corrupt Shim accounting");
    assert_eq!(closed.len(), 3, "duplicate closed names corrupt Shim accounting");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 38);

    let mut actual = BTreeSet::new();
    for source in [
        include_str!("../../skit-language/tests/port_test_shim_core.rs"),
        include_str!("../../skit-language/tests/port_test_shim_runtime.rs"),
        include_str!("port_test_shim_staging.rs"),
    ] {
        actual.extend(names(source));
    }
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "Shim executable parity is incomplete or mislabeled");
    assert!(actual.is_disjoint(&closed));
}
