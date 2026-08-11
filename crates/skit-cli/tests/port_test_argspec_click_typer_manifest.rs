//! Completeness guard for Python v0.4 `tests/test_argspec_click_typer.py`.
//!
//! Sixty-six contracts map to the public parser-owned Click/Typer projection. The one Python test
//! that directly calls private `_decorator_name` has no equivalent Rust public seam and is explicitly
//! blocked rather than being replaced with a weaker fake. This manifest is not behavioral coverage.

use std::collections::BTreeMap;

use syn::{Attribute, Item};

const SOURCE: &str = include_str!("../../skit-form/tests/port_test_argspec_click_typer_exact.rs");

const EXECUTABLE: &[&str] = &[
    "test_click_fields_bottom_up_order_matches_runtime",
    "test_click_argument_variadic_is_multiple_not_required",
    "test_click_is_flag_choice_int_and_required",
    "test_click_plain_argument_is_required",
    "test_click_group_degrades_as_subcommands",
    "test_click_count_option_degrades_field",
    "test_typer_signature_order_and_kinds",
    "test_typer_run_pattern_reads_the_function",
    "test_typer_bool_default_true_degrades_not_guesses",
    "test_typer_underscored_param_gets_kebab_flag",
    "test_typer_two_commands_degrade_as_subcommands",
    "test_argparse_still_wins_when_present",
    "test_read_cli_none_for_plain_scripts",
    "test_click_field_orders_increment_by_one",
    "test_click_from_import_form_is_recognized",
    "test_click_dotted_import_is_recognized",
    "test_click_secret_name_precheck_and_flag_default",
    "test_click_uppercase_type_constants",
    "test_click_non_choice_call_type_degrades_even_with_list_arg",
    "test_click_non_literal_default_degrades",
    "test_click_multiple_option_flag",
    "test_click_short_flag_only_and_help",
    "test_typer_from_import_form_is_recognized",
    "test_typer_orders_match_signature_positions",
    "test_typer_bare_positional_no_default",
    "test_typer_unannotated_param_is_plain_text_not_degraded",
    "test_typer_unmodelable_annotation_degrades",
    "test_typer_option_none_default_is_clean",
    "test_typer_secret_param_name_precheck",
    "test_click_is_flag_defaulting_on_degrades_not_guesses",
    "test_click_dotted_only_import_counts",
    "test_click_from_dotted_module_counts",
    "test_typer_dotted_only_import_counts",
    "test_typer_from_dotted_module_counts",
    "test_click_two_commands_without_group_degrade",
    "test_click_foreign_decorators_between_options_are_skipped_not_fatal",
    "test_click_non_literal_name_skips_that_call_only",
    "test_click_partly_non_literal_names_skip_that_call_only",
    "test_click_short_first_declaration_still_prefers_long_flag",
    "test_click_dest_strips_dashes_not_letters",
    "test_click_default_none_is_clean",
    "test_click_bare_float_and_str_types",
    "test_click_unknown_name_type_degrades",
    "test_click_path_and_file_types",
    "test_typer_option_extra_decl_positions",
    "test_typer_path_annotation_is_path",
    "test_typer_non_constant_decl_is_ignored_not_fatal",
    "test_typer_computed_plain_default_degrades",
    "test_typer_option_computed_first_arg_degrades",
    "test_typer_bool_true_degrade_renders_as_text",
    "test_typer_bool_false_flag_contract_exact",
    "test_click_non_literal_choice_list_degrades",
    "test_typer_unmodelable_annotation_degrades_despite_literal_default",
    "test_typer_option_single_extra_decl_is_read",
    "test_annotated_reads_type_and_metadata",
    "test_annotated_option_without_default_is_required",
    "test_annotated_argument_with_default_is_optional_positional",
    "test_annotated_unmodelable_inner_type_degrades",
    "test_annotated_bool_default_true_degrades",
    "test_annotated_choice_via_typing_qualified_name",
    "test_annotated_help_kwarg_survives_on_degraded_field",
    "test_legacy_typer_style_still_works_after_annotated_refactor",
    "test_annotated_only_recognizes_the_real_annotated_name",
    "test_annotated_without_typer_metadata_reads_as_plain_type",
    "test_annotated_picks_the_typer_call_among_several",
    "test_annotated_option_positional_decl_is_a_flag_not_a_default",
];

const BLOCKED_PRIVATE: &str = "test_decorator_name_unnameable_callable_is_empty";

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names() -> Vec<String> {
    syn::parse_file(SOURCE)
        .expect("Click/Typer parity source must parse as Rust")
        .items
        .into_iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => Some(function.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

#[test]
fn click_typer_has_all_66_public_python_oracles_in_exact_order() {
    let actual = names();
    let expected = EXECUTABLE
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert_eq!(EXECUTABLE.len(), 66);
}

#[test]
fn click_typer_private_decorator_helper_is_not_faked_as_coverage() {
    let mut counts = BTreeMap::<String, usize>::new();
    for name in names() {
        *counts.entry(name).or_default() += 1;
    }
    assert_eq!(counts.get(BLOCKED_PRIVATE).copied().unwrap_or_default(), 0);
}
