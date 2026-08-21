//! Batch A accounting for Python v0.4 `tests/test_shim.py` at `main@206f9ef`.
//!
//! This is not the final Shim manifest. CLI staging Batch B owns the three pending writer names.

use std::collections::BTreeSet;

use syn::{Attribute, Item};

const PORTABLE: &[&str] = &[
    "test_const_str_injection_preserves_everything_else",
    "test_const_typed_injection",
    "test_main_guard_const",
    "test_input_queue_preamble_is_single_line_after_docstring",
    "test_missing_value_leaves_script_untouched",
    "test_shadowed_input_is_not_rewritten_and_surfaces_as_drift",
    "test_shadowed_input_leaves_the_call_site_text_intact_when_only_a_const_is_delivered",
    "test_drifted_target_raises",
    "test_bad_type_coercion_raises",
    "test_bad_type_coercion_raises_the_value_subclass_not_plain_shim_error",
    "test_drifted_target_raises_plain_shim_error_not_value_subclass",
    "test_coerce_bool_invalid_raises",
    "test_inject_syntax_error_raises",
    "test_input_order_beyond_calls_is_drift",
    "test_inject_two_identical_prompts_one_deleted_raises_cleanly_never_corrupts",
    "test_inject_specs_sharing_the_same_order_never_double_bind",
    "test_inject_triple_duplicate_specs_same_order_never_double_bind",
    "test_preamble_inserted_at_end_for_no_docstring_no_future",
];

const RUNTIME_REHOMED: &[&str] = &[
    "test_input_queue_by_order",
    "test_input_queue_exhaustion_falls_back_to_stdin",
    "test_input_queue_in_loop_consumes_by_call_order",
    "test_input_queue_secret_masks_echo",
    "test_input_queue_with_future_import",
    "test_input_queue_combined_with_const_injection",
    "test_unshadowed_input_is_rewritten_to_the_wrapper",
    "test_preamble_insertion_survives_form_feed_inside_docstring",
    "test_input_value_follows_prompt_despite_runtime_call_order_diverging_from_source_order",
    "test_input_value_follows_prompt_after_an_earlier_input_is_deleted",
    "test_multiline_value_span",
    "test_inject_duplicate_prompt_winner_only_still_injects_and_compiles",
    "test_multiline_span_replacement",
    "test_const_injection_survives_form_feed_between_targets",
    "test_const_injection_survives_u2028_inside_earlier_string_literal",
];

const PENDING_STAGING: &[&str] = &[
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

#[derive(Debug)]
struct Owner {
    name: String,
    active: bool,
}

fn has(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn owners(source: &str) -> Vec<Owner> {
    syn::parse_file(source)
        .expect("Shim owner source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has(&function.attrs, "test")
                    && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(Owner {
                    name: function.sig.ident.to_string(),
                    active: !has(&function.attrs, "ignore"),
                })
            }
            _ => None,
        })
        .collect()
}

fn set<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeSet<&'a str> {
    names.into_iter().collect()
}

#[test]
fn test_shim_batch_a_accounts_every_owner_without_claiming_staging_complete() {
    assert_eq!(PORTABLE.len(), 18);
    assert_eq!(RUNTIME_REHOMED.len(), 15);
    assert_eq!(PENDING_STAGING.len(), 3);
    assert_eq!(CLOSURES.len(), 2);
    for closure in CLOSURES {
        assert!(!closure.name.is_empty());
        assert!(
            !closure.reason.is_empty(),
            "closure {} has no reason",
            closure.name
        );
        assert!(
            !closure.strong_owner.is_empty(),
            "closure {} has no strong owner",
            closure.name
        );
    }

    let portable = owners(include_str!("../../skit-language/tests/port_test_shim.rs"));
    let runtime = owners(include_str!("port_test_shim_runtime.rs"));
    let all = portable.iter().chain(&runtime).collect::<Vec<_>>();
    let all_names = all
        .iter()
        .map(|owner| owner.name.as_str())
        .collect::<Vec<_>>();
    let all_set = set(all_names.iter().copied());
    assert_eq!(
        all_names.len(),
        all_set.len(),
        "a Set must not hide duplicate exact Shim owners"
    );

    let expected_definitions = set(PORTABLE
        .iter()
        .chain(RUNTIME_REHOMED)
        .chain(PENDING_STAGING)
        .copied());
    assert_eq!(all_set, expected_definitions);

    let portable_active = set(portable
        .iter()
        .filter(|owner| owner.active)
        .map(|owner| owner.name.as_str()));
    let runtime_active = set(runtime
        .iter()
        .filter(|owner| owner.active)
        .map(|owner| owner.name.as_str()));
    let inactive = set(all
        .iter()
        .filter(|owner| !owner.active)
        .map(|owner| owner.name.as_str()));
    assert_eq!(portable_active, set(PORTABLE.iter().copied()));
    assert_eq!(runtime_active, set(RUNTIME_REHOMED.iter().copied()));
    assert_eq!(inactive, set(PENDING_STAGING.iter().copied()));

    let closure_names = set(CLOSURES.iter().map(|closure| closure.name));
    assert!(all_set.is_disjoint(&closure_names));
    let accounted = all_set
        .union(&closure_names)
        .copied()
        .collect::<BTreeSet<_>>();
    assert_eq!(accounted.len(), 38);
    assert_eq!(portable_active.len() + runtime_active.len(), 33);
}
