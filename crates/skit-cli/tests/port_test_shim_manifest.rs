//! Final exact-name accounting for Python v0.4 `tests/test_shim.py` at `main@206f9ef`.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const EXPECTED: &[&str] = &[
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
    "test_physical_lines_matches_splitlines_on_ordinary_text",
    "test_const_injection_survives_form_feed_between_targets",
    "test_const_injection_survives_u2028_inside_earlier_string_literal",
    "test_preamble_insertion_survives_form_feed_inside_docstring",
    "test_input_value_follows_prompt_despite_runtime_call_order_diverging_from_source_order",
    "test_input_value_follows_prompt_after_an_earlier_input_is_deleted",
    "test_write_injected_cleanup_on_error",
    "test_write_injected_closes_fd_when_chmod_raises",
    "test_write_injected_lands_outside_entry_dir",
    "test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable",
];

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
    "test_write_injected_cleanup_on_error",
    "test_write_injected_lands_outside_entry_dir",
    "test_write_injected_falls_back_to_entry_dir_if_os_temp_unavailable",
];

struct Closure {
    name: &'static str,
    reason: &'static str,
    strong_owner: &'static str,
}

const CLOSURES: &[Closure] = &[
    Closure {
        name: "test_physical_lines_matches_splitlines_on_ordinary_text",
        reason: "Rust consumes parser-native byte spans and has no line/column-to-splitlines reconciliation helper.",
        strong_owner: "the exact form-feed, U+2028, and form-feed-docstring runtime owners",
    },
    Closure {
        name: "test_write_injected_closes_fd_when_chmod_raises",
        reason: "NamedTempFile owns the file handle at allocation and has no mkstemp-to-chmod-to-fdopen ownership gap.",
        strong_owner: "command::tests::source_staging_and_prompt_rendering_keep_execution_files_private",
    },
];

struct Owner {
    name: String,
    active: bool,
}

fn has(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn visit(items: Vec<Item>, expected: &BTreeSet<&str>, owners: &mut Vec<Owner>) {
    for item in items {
        match item {
            Item::Fn(function)
                if has(&function.attrs, "test")
                    && expected.contains(function.sig.ident.to_string().as_str()) =>
            {
                owners.push(Owner {
                    name: function.sig.ident.to_string(),
                    active: !has(&function.attrs, "ignore"),
                });
            }
            Item::Mod(module) => {
                if let Some((_, items)) = module.content {
                    visit(items, expected, owners);
                }
            }
            _ => {}
        }
    }
}

fn owners(source: &str, expected: &BTreeSet<&str>) -> Vec<Owner> {
    let mut owners = Vec::new();
    visit(
        syn::parse_file(source)
            .expect("Shim owner source must parse")
            .items,
        expected,
        &mut owners,
    );
    owners
}

fn set<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'a str> {
    names.into_iter().collect()
}

#[test]
fn test_shim_has_36_unique_active_owners_and_two_structured_closures() {
    let expected = set(EXPECTED.iter().copied());
    let executable = set(EXECUTABLE.iter().copied());
    assert_eq!(EXPECTED.len(), 38);
    assert_eq!(
        expected.len(),
        EXPECTED.len(),
        "oracle names contain a duplicate"
    );
    assert_eq!(EXECUTABLE.len(), 36);
    assert_eq!(
        executable.len(),
        EXECUTABLE.len(),
        "executable names contain a duplicate"
    );
    assert_eq!(CLOSURES.len(), 2);
    for closure in CLOSURES {
        assert!(!closure.name.is_empty());
        assert!(!closure.reason.is_empty(), "{} has no reason", closure.name);
        assert!(
            !closure.strong_owner.is_empty(),
            "{} has no strong owner",
            closure.name
        );
    }
    let closures = set(CLOSURES.iter().map(|closure| closure.name));
    assert_eq!(closures.len(), CLOSURES.len());
    assert!(executable.is_disjoint(&closures));
    assert_eq!(
        executable
            .union(&closures)
            .copied()
            .collect::<BTreeSet<_>>(),
        expected
    );

    let mut actual = Vec::new();
    for source in [
        include_str!("../../skit-language/tests/port_test_shim.rs"),
        include_str!("port_test_shim_runtime.rs"),
        include_str!("port_test_shim_staging.rs"),
        include_str!("../src/run/command.rs"),
    ] {
        actual.extend(owners(source, &expected));
    }
    let occurrences = actual
        .iter()
        .map(|owner| owner.name.as_str())
        .collect::<Vec<_>>();
    let occurrence_set = set(occurrences.iter().copied());
    assert_eq!(
        occurrences.len(),
        occurrence_set.len(),
        "a Set must not hide duplicate exact Shim owners"
    );
    assert_eq!(occurrences.len(), 36);
    assert_eq!(occurrence_set, executable);
    assert!(
        actual.iter().all(|owner| owner.active),
        "an executable Shim owner is ignored"
    );
}
