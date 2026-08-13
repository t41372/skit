//! Exact-name completeness gate for Python v0.4 `tests/test_js_analyzer.py` at `main@206f9ef`.
//!
//! The two Python lazy-import fault-injection contracts have no Rust architectural equivalent.
//! Every other frozen Python test name must exist as a real Rust `#[test] fn test_*` in the
//! parser/reconcile/block/parseArgs port or the real CLI/store integration port. Rust-additive
//! row splits are excluded from parity accounting.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXPECTED: &[&str] = &[
    "test_const_number_string_bool",
    "test_template_string_excluded",
    "test_object_and_array_excluded",
    "test_destructuring_excluded",
    "test_bare_declaration_without_value_skipped",
    "test_leading_underscore_skipped",
    "test_last_write_wins_keeps_first_slot",
    "test_multiple_declarators_in_one_statement",
    "test_comment_between_keyword_and_declarator_is_skipped",
    "test_secret_by_name",
    "test_lineno_recorded",
    "test_let_and_var_demoted",
    "test_const_reassigned_is_demoted",
    "test_const_augmented_assign_is_demoted",
    "test_const_update_expression_is_demoted",
    "test_plain_const_not_demoted",
    "test_member_reassignment_does_not_demote",
    "test_negative_int_is_a_unary_expression_not_a_number_literal",
    "test_exotic_number_literals_are_float_with_source_text_default",
    "test_simple_decimal_float",
    "test_empty_and_escaped_string_values",
    "test_ts_annotation_value_still_found",
    "test_ts_only_constructs_parse_under_the_typescript_grammar",
    "test_js_grammar_errors_on_typescript_only_syntax",
    "test_tsx_grammar_branch",
    "test_unknown_lang_falls_back_to_javascript",
    "test_has_error_returns_empty_syntax_error",
    "test_empty_script",
    "test_reconcile_const_ok",
    "test_reconcile_const_gone_is_missing",
    "test_reconcile_type_change_is_flagged",
    "test_reconcile_ts_lang_threaded",
    "test_block_roundtrip_on_ts_file",
    "test_block_lands_after_a_node_shebang",
    "test_block_at_top_when_no_shebang",
    "test_write_empty_params_is_identity",
    "test_parseargs_util_member_inline_options",
    "test_parseargs_bare_call",
    "test_parseargs_nested_member",
    "test_parseargs_all_option_features",
    "test_parseargs_boolean_default_true_applies_literally",
    "test_parseargs_string_key_option",
    "test_parseargs_secret_option_name",
    "test_parseargs_identifier_options_whole_spec_degrade",
    "test_parseargs_spread_in_options_whole_spec_degrade",
    "test_parseargs_computed_key_skips_just_that_field",
    "test_parseargs_empty_string_key_is_skipped",
    "test_parseargs_non_object_option_value_degrades_field",
    "test_parseargs_unknown_type_string_degrades_field",
    "test_parseargs_non_literal_type_value_degrades_field",
    "test_parseargs_non_literal_default_degrades_field",
    "test_parseargs_ignores_spread_computed_and_numeric_keys_in_spec",
    "test_parseargs_option_spec_without_type_keeps_str_and_reads_default",
    "test_parseargs_shorthand_property_in_options_is_skipped",
    "test_parseargs_finds_options_past_a_spread_and_another_key",
    "test_parseargs_empty_options_object_is_a_readable_zero_field_surface",
    "test_no_parseargs_surface_returns_none",
    "test_parseargs_member_call_that_is_not_parseargs_is_ignored",
    "test_parseargs_with_no_config_object_returns_none",
    "test_parseargs_non_object_config_returns_none",
    "test_parseargs_config_without_options_key_returns_none",
    "test_reader_on_syntax_error_returns_none",
    "test_reader_threads_lang_for_typescript",
    "test_params_manage_writes_block_into_js_copy",
    "test_params_show_lists_ts_const",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    "test_import_guard_degrades_analysis_capabilities_to_none",
    "test_plan_degrades_to_none_when_analyzer_missing",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_names(source: &str) -> BTreeSet<String> {
    syn::parse_file(source)
        .expect("ported Rust test source must parse")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function)
                if has_test_attribute(&function.attrs)
                    && function.sig.ident.to_string().starts_with("test_") =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn test_js_analyzer_frozen_names_are_exactly_accounted() {
    assert_eq!(EXPECTED.len(), 65);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 2);
    let expected = EXPECTED.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), EXPECTED.len(), "duplicate executable oracle name");
    assert_eq!(closed.len(), ARCHITECTURE_CLOSED.len(), "duplicate closed oracle name");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 67, "frozen Python denominator changed");

    let mut actual = parity_names(include_str!("../../skit-language/tests/port_test_js_analyzer.rs"));
    actual.extend(parity_names(include_str!("port_test_js_analyzer_cli.rs")));
    let actual = actual.iter().map(String::as_str).collect::<BTreeSet<_>>();

    assert_eq!(actual, expected, "JS analyzer parity names drifted from the frozen Python oracle");
    assert!(actual.is_disjoint(&closed), "a Python-private import seam was falsely presented as executable parity");
}
