//! Exact-name/order completeness guard for Python v0.4 `tests/test_argspec.py`.
//!
//! The 34 behavioral tests live in `skit-form` and use the public parser-owned CLI projection. This
//! manifest is not coverage; it prevents omissions, aggregate renames, reorder drift, or extra fake
//! Python-parity tests.

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-form/tests/port_test_argspec_exact.rs");

const PYTHON_TESTS: &[&str] = &[
    "test_no_argparse_returns_none",
    "test_syntax_error_returns_none",
    "test_stitch_reads_eight_fields_in_source_order",
    "test_stitch_positional_multiple_required",
    "test_argparse_path_type_spellings",
    "test_argparse_choices_beat_path_type",
    "test_stitch_required_flag_and_long_name_preferred",
    "test_stitch_choices_with_default",
    "test_stitch_int_field",
    "test_stitch_custom_type_degrades_field",
    "test_stitch_store_true_checkbox",
    "test_store_false_defaults_on",
    "test_subparsers_degrade_whole_spec",
    "test_loop_generated_arguments_degrade_whole_spec",
    "test_append_action_degrades_field_only",
    "test_non_literal_choices_degrade_field",
    "test_help_and_version_actions_are_not_fields",
    "test_secret_name_precheck",
    "test_optional_positional_star_not_required",
    "test_dest_override_wins",
    "test_type_float_and_str_map_to_kinds",
    "test_default_none_literal_does_not_degrade",
    "test_non_literal_argument_name_skips_that_field_only",
    "test_short_flag_only_keeps_short_name",
    "test_field_order_matches_source_order",
    "test_choices_win_over_type_for_kind",
    "test_required_false_literal_is_not_required",
    "test_partly_non_literal_name_list_skips_that_field_only",
    "test_flag_dest_only_strips_dashes_not_letters",
    "test_computed_default_degrades_field",
    "test_argparse_fixed_nargs_is_a_multi_value_field",
    "test_click_fixed_nargs_is_a_multi_value_field",
    "test_click_multiple_with_fixed_nargs_is_not_modelled_at_all",
    "test_click_multiple_with_nargs_one_is_still_modelled",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

#[test]
fn argspec_port_has_all_34_python_tests_in_exact_order() {
    let actual = syn::parse_file(SOURCE)
        .expect("argspec parity source must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let expected = PYTHON_TESTS
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();

    assert_eq!(actual, expected);
    assert_eq!(PYTHON_TESTS.len(), 34);
}
