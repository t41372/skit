//! Exact-name completeness gate for Python v0.4 `tests/test_flows.py` at `main@206f9ef`.
//!
//! All 102 frozen functions have executable Rust owners. Rust-only strengthening tests use the
//! `rust_additive_*` prefix and therefore cannot impersonate Python parity in this accounting.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXPECTED: &[&str] = &[
    "test_plan_managed_script_is_inject",
    "test_plan_argparse_script",
    "test_plan_plain_script_is_none",
    "test_plan_command_entry_placeholders",
    "test_plan_managed_wins_over_argparse",
    "test_plan_missing_script_is_none",
    "test_prefill_default_then_last_then_preset",
    "test_prefill_never_offers_secrets",
    "test_validate_required_empty",
    "test_validate_int_error_names_field_and_value",
    "test_validate_choice",
    "test_validate_token_values_deferred",
    "test_assemble_argparse_positionals_then_flags",
    "test_assemble_unchecked_store_true_omits_flag",
    "test_assemble_degraded_empty_omitted_filled_passed",
    "test_assemble_glob_expands_multiple_fields_against_cwd",
    "test_assemble_glob_without_match_keeps_literal",
    "test_assemble_tokens_expand_and_type_check_after_expansion",
    "test_assemble_missing_env_token_is_named_error",
    "test_assemble_inject_values_expanded_and_masked_display",
    "test_assemble_secret_env_source_reads_environment",
    "test_assemble_secret_env_source_missing_is_named_error",
    "test_assemble_typed_secret_beats_env_source",
    "test_assemble_command_values_and_extra_args",
    "test_assemble_extra_args_expand_tokens_and_globs",
    "test_assemble_extra_arg_token_error_forwards_the_token_message",
    "test_assemble_inject_source_forwards_extra_args",
    "test_assemble_field_expands_cwd_and_now_tokens",
    "test_assemble_does_not_retypecheck_plain_values",
    "test_assemble_defaults_env_to_os_environ",
    "test_assemble_flags_tolerates_missing_keys",
    "test_assemble_empty_field_does_not_stop_later_flags",
    "test_split_multi_falls_back_on_unbalanced_quote",
    "test_resolve_secret_empty_when_no_input_and_no_env_source",
    "test_validate_value_accepts_a_valid_choice",
    "test_prefill_drops_a_secret_that_leaked_into_saved_values",
    "test_prefill_preset_drops_leaked_secret",
    "test_prefill_unknown_preset_is_no_op_not_a_crash",
    "test_glob_feedback_counts",
    "test_save_after_run_persists_intent_and_stamps_run",
    "test_record_run_zero_exit_survives_save",
    "test_truthy_accepts_every_truthy_spelling",
    "test_expand_glob_piece_globs_only_when_glob_chars_present",
    "test_expand_glob_piece_supports_recursive_doublestar",
    "test_assemble_tolerates_a_bool_field_missing_from_values",
    "test_assemble_store_false_fires_flag_when_unchecked",
    "test_assemble_repeat_emits_flag_before_each_piece",
    "test_assemble_non_repeat_multi_keeps_one_flag_then_values",
    "test_assemble_repeat_single_piece",
    "test_assemble_repeat_shares_shlex_and_glob_split_with_non_repeat",
    "test_assemble_bool_store_true_fires_only_when_checked",
    "test_assemble_bool_flagless_never_appends_empty_string",
    "test_assemble_bool_empty_action_fires_in_neither_state",
    "test_field_from_arg_maps_every_field",
    "test_field_from_arg_degraded_renders_as_text",
    "test_field_from_arg_copies_repeat",
    "test_field_from_arg_bool_flag_empty_action_defaults_store_true",
    "test_field_from_arg_bool_flag_degraded_stays_text_and_keeps_empty_action",
    "test_field_from_arg_bool_positional_no_flag_keeps_empty_action",
    "test_field_from_arg_bool_flag_explicit_action_preserved",
    "test_render_default_spells_booleans_lowercase",
    "test_plan_sources_are_exact_per_field",
    "test_plan_field_sources_inject_and_flag",
    "test_plan_drift_names_entry_and_keeps_usable_specs",
    "test_plan_subparsers_degrades_with_reason",
    "test_field_from_spec_maps_every_field",
    "test_field_from_spec_unknown_type_falls_back_to_text",
    "test_field_from_spec_maps_numeric_and_bool_kinds",
    "test_type_error_messages_exact",
    "test_assemble_display_order_and_masking",
    "test_assemble_none_plan_only_carries_extras",
    "test_command_placeholders_are_required_and_secret_prechecked",
    "test_save_after_run_clears_cleared_extra_args",
    "test_save_after_run_purges_secret_placeholder_from_presets",
    "test_assemble_expand_extra_false_passes_argv_untouched",
    "test_masked_args_hide_flag_source_secret_values",
    "test_masked_args_still_glob_expand_multiple_fields",
    "test_transparency_lines_inject_source_shows_masked_and_temp_note",
    "test_assemble_display_lists_only_inject_delivered_values",
    "test_transparency_lines_flag_source_is_single_command_line",
    "test_execute_runs_and_returns_the_scripts_exit_code",
    "test_command_template_secret_does_not_get_prompt_agent_warning",
    "test_pinned_amp_prompt_warns_on_runner_none_shared_execution_path",
    "test_execute_injects_then_cleans_up_the_temp_copy",
    "test_execute_classifies_missing_target",
    "test_execute_classifies_not_executable",
    "test_prompt_validation_classifies_missing_body_before_transparency",
    "test_prompt_validation_classifies_empty_runner_config_before_transparency",
    "test_execute_classifies_injection_drift",
    "test_execute_bad_value_reports_value_not_drift",
    "test_transparency_inject_lines_are_exact",
    "test_transparency_shows_the_injected_temp_path",
    "test_transparency_flag_source_masks_secret_in_command",
    "test_transparency_command_source_shows_filled_template",
    "test_transparency_command_source_masks_secret_placeholder",
    "test_normal_prompt_transparency_is_compact_and_never_reads_the_body",
    "test_execute_not_executable_message_carries_the_error",
    "test_execute_launch_error_message_carries_the_error",
    "test_execute_forwards_invoke_cwd",
    "test_execute_inject_falls_back_to_entry_dir",
    "test_typed_multi_value_field_validates_each_piece_not_the_whole_box",
    "test_single_value_field_still_validates_the_whole_string",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("flows port source must parse")
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
fn test_flows_frozen_names_are_exactly_accounted() {
    assert_eq!(EXPECTED.len(), 102, "the frozen test_flows.py denominator changed");
    let expected = EXPECTED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 102, "duplicate frozen Flows names make accounting dishonest");

    let mut actual_names = Vec::new();
    for source in [
        include_str!("../../skit-application/tests/port_test_flows_exact_core.rs"),
        include_str!("../../skit-application/tests/port_test_flows_exact_flags.rs"),
        include_str!("../../skit-application/tests/port_test_flows_exact_stage_boundaries.rs"),
        include_str!("../../skit-application/tests/port_test_flows_exact_state.rs"),
        include_str!("../../skit-application/tests/port_test_flows_exact_transparency.rs"),
        include_str!("../../skit-application/tests/port_test_flows_exact_scalar_contracts.rs"),
        include_str!("../../skit-form/tests/port_test_flows_exact_plan.rs"),
        include_str!("../../skit-form/tests/port_test_flows_exact_plan_more.rs"),
        include_str!("../../skit-store/tests/port_test_flows_exact_more.rs"),
        include_str!("../../skit-store/tests/port_test_flows_exact_drift.rs"),
        include_str!("../../skit-ui/tests/port_test_flows_exact_fields.rs"),
        include_str!("port_test_flows_exact_execute.rs"),
        include_str!("port_test_flows_exact_final_cli.rs"),
        include_str!("../../skit-runtime/tests/port_test_flows_exact_runtime.rs"),
    ] {
        actual_names.extend(names(source));
    }

    assert_eq!(
        actual_names.len(),
        102,
        "canonical Flows sources contain duplicate or extra frozen-looking test_* owners"
    );
    let actual = actual_names.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual.len(), 102, "one frozen Flows name has more than one executable owner");
    assert_eq!(actual, expected, "Flows executable parity is incomplete or mislabeled");
}
