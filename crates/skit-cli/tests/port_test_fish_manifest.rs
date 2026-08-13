//! Exact-name completeness gate for Python v0.4 `tests/test_fish.py` at `main@206f9ef`.
//!
//! Rust's Fish adapter is tree-sitter owned, so the old Python hand-scanner internals are not
//! recreated in tests. Every public analyzer/reader/registry/corpus/flow contract remains executable;
//! only the explicitly named Python-private scanner/helper seams are architecture-closed.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_oneline_idiom_int",
    "test_newline_continued_or",
    "test_float_and_string_defaults",
    "test_guarded_set_may_carry_scope_flags",
    "test_secret_name_flagged",
    "test_suppressed_by_plain_clobber_anywhere",
    "test_clobber_before_the_idiom_also_suppresses",
    "test_unrelated_clobber_does_not_suppress",
    "test_underscore_name_skipped",
    "test_first_occurrence_wins_on_duplicate_idiom",
    "test_query_without_following_set_is_not_a_candidate",
    "test_query_with_no_name_is_ignored",
    "test_conditional_set_without_value_is_not_a_candidate",
    "test_mismatched_names_are_not_an_idiom",
    "test_unconditional_set_after_query_is_not_an_idiom",
    "test_idiom_inside_function_is_not_toplevel",
    "test_idiom_inside_every_block_kind_is_ignored",
    "test_toplevel_after_a_closed_block_is_detected",
    "test_nested_clobber_does_not_suppress_toplevel_idiom",
    "test_stray_end_clamps_depth_at_zero",
    "test_argv_hint",
    "test_self_location_hints",
    "test_hint_ignores_commented_argv",
    "test_reconcile_ok_then_drift",
    "test_argparse_short_long_and_valueless_bool",
    "test_argparse_value_suffixes",
    "test_argparse_long_only_and_short_only",
    "test_argparse_dummy_short_yields_long_only",
    "test_argparse_numeric_hash_degrades",
    "test_argparse_validator_is_stripped",
    "test_argparse_secret_name",
    "test_argparse_skips_own_options",
    "test_argparse_attached_own_option_does_not_consume",
    "test_argparse_after_conditional_prefix_is_found",
    "test_argparse_empty_specs_is_zero_field_surface",
    "test_no_argparse_returns_none",
    "test_argparse_variable_specs_degrade_to_dynamic",
    "test_argparse_command_substitution_specs_degrade_to_dynamic",
    "test_argparse_garbage_specs_are_skipped",
    "test_argparse_empty_long_falls_back_to_short",
    "test_registry_capabilities",
    "test_corpus_analyze_is_total_and_reads_back",
    "test_corpus_expected_detections",
    "test_manage_then_plan_and_assemble_env_delivery",
    "test_env_overlay_overrides_default_in_real_fish",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_tokenize_semicolon_and_words",
    "test_tokenize_quotes_hold_separators",
    "test_tokenize_escaped_quote_does_not_close",
    "test_tokenize_comment_ends_line",
    "test_tokenize_hash_midword_is_literal",
    "test_tokenize_backslash_escape_outside_quote",
    "test_tokenize_unterminated_quote_is_total",
    "test_statements_drop_empty_runs_between_semicolons",
    "test_logical_lines_join_continuation",
    "test_logical_lines_even_backslashes_are_not_a_continuation",
    "test_logical_lines_trailing_continuation_flushes",
    "test_dequote_single_quote_escapes",
    "test_dequote_double_quote_escapes",
    "test_dequote_backslash_outside_and_at_end",
    "test_dequote_unterminated_quotes_are_total",
    "test_strip_comment_paths",
    "test_classify_set_matrix",
    "test_is_query_matrix",
    "test_spec_tokens_all_own_options_no_specs",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("ported Fish Rust test source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test(&function.attrs)
                    && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn test_fish_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 45);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 19);
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), EXECUTABLE.len(), "duplicate executable Fish oracle name");
    assert_eq!(closed.len(), ARCHITECTURE_CLOSED.len(), "duplicate closed Fish oracle name");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 64, "frozen Fish denominator changed");

    let mut actual = parity_names(include_str!("../../skit-language/tests/port_test_fish.rs"));
    actual.extend(parity_names(include_str!(
        "../../skit-language/tests/port_test_fish_registry.rs"
    )));
    actual.extend(parity_names(include_str!("port_test_fish_e2e.rs")));
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();

    assert_eq!(
        actual, expected,
        "Fish executable parity is incomplete, duplicated, or mislabeled"
    );
    assert!(
        actual.is_disjoint(&closed),
        "a Python-private Fish scanner helper was falsely presented as executable parity"
    );
}
