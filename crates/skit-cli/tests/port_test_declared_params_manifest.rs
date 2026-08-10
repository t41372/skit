//! Mechanical completeness guard for Python v0.4 `tests/test_declared_params.py`.
//!
//! This file is not behavioral coverage. Every target below is a real behavioral test in another
//! integration target; this guard only prevents a Python test from silently disappearing later.

use std::{fs, path::Path};

struct Mapping {
    python: &'static str,
    path: &'static str,
    rust: &'static str,
}

const MAPPINGS: &[Mapping] = &[
    Mapping { python: "test_undeclared_placeholders_synthesize_the_historical_field", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_undeclared_placeholders_synthesize_the_historical_field" },
    Mapping { python: "test_declared_row_overrides_placeholder_schema_including_secret", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_declared_row_overrides_placeholder_schema_including_secret" },
    Mapping { python: "test_declared_env_param_rides_along_after_placeholders", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_declared_env_param_rides_along_after_placeholders" },
    Mapping { python: "test_declared_flag_row_is_dropped_for_templates", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_declared_flag_row_is_dropped_for_templates" },
    Mapping { python: "test_declared_row_with_wrong_delivery_for_its_placeholder_is_replaced_by_synth", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_declared_row_with_wrong_delivery_for_its_placeholder_is_replaced_by_synth" },
    Mapping { python: "test_declared_from_meta_drops_nameless_rows", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_declared_from_meta_drops_nameless_rows" },
    Mapping { python: "test_synthesized_placeholder_shape", path: "crates/skit-domain/tests/port_test_declared_params.rs", rust: "test_synthesized_placeholder_shape" },
    Mapping { python: "test_command_plan_honors_declared_schema", path: "crates/skit-form/tests/port_test_declared_form.rs", rust: "test_command_plan_honors_declared_schema" },
    Mapping { python: "test_exe_with_declared_params_gets_a_form", path: "crates/skit-form/tests/port_test_declared_form.rs", rust: "test_exe_with_declared_params_gets_a_form" },
    Mapping { python: "test_exe_without_declared_params_stays_none_plan", path: "crates/skit-form/tests/port_test_declared_form.rs", rust: "test_exe_without_declared_params_stays_none_plan" },
    Mapping { python: "test_assemble_env_values_masked_and_empty_absent", path: "crates/skit-application/tests/port_test_declared_delivery.rs", rust: "test_assemble_env_values_masked_and_empty_absent" },
    Mapping { python: "test_assemble_mixed_flag_and_env_fields", path: "crates/skit-application/tests/port_test_declared_delivery.rs", rust: "test_assemble_mixed_flag_and_env_fields" },
    Mapping { python: "test_assemble_command_with_env_rider", path: "crates/skit-application/tests/port_test_declared_delivery.rs", rust: "test_assemble_command_with_env_rider" },
    Mapping { python: "test_run_entry_env_overlay_wins_last", path: "crates/skit-runtime/tests/port_test_declared_runtime_wiring.rs", rust: "test_run_entry_env_overlay_wins_last" },
    Mapping { python: "test_transparency_shows_masked_env_prefix", path: "crates/skit-runtime/tests/port_test_declared_runtime_wiring.rs", rust: "test_transparency_shows_masked_env_prefix" },
    Mapping { python: "test_write_read_parameters_roundtrip_and_legacy_params_untouched", path: "crates/skit-domain/tests/port_test_declared_meta.rs", rust: "test_write_read_parameters_roundtrip_and_legacy_params_untouched" },
    Mapping { python: "test_execute_passes_env_values_to_run_entry", path: "crates/skit-runtime/tests/port_test_declared_runtime_wiring.rs", rust: "test_execute_passes_env_values_to_run_entry" },
    Mapping { python: "test_meta_parameters_roundtrip_and_non_dict_rows_dropped", path: "crates/skit-domain/tests/port_test_declared_meta.rs", rust: "test_meta_parameters_roundtrip_and_non_dict_rows_dropped" },
    Mapping { python: "test_declared_plan_secret_placeholder_masks_in_command_values", path: "crates/skit-application/tests/port_test_declared_delivery.rs", rust: "test_declared_plan_secret_placeholder_masks_in_command_values" },
    Mapping { python: "test_unknown_kind_entry_still_gets_none_plan", path: "crates/skit-form/tests/port_test_declared_form_edges.rs", rust: "test_unknown_kind_entry_still_gets_none_plan" },
    Mapping { python: "test_exe_with_only_placeholder_rows_falls_through_to_none", path: "crates/skit-form/tests/port_test_declared_form_edges.rs", rust: "test_exe_with_only_placeholder_rows_falls_through_to_none" },
    Mapping { python: "test_cli_add_flag_param_on_exe_then_run_set", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_add_flag_param_on_exe_then_run_set" },
    Mapping { python: "test_cli_exe_show_table_and_json", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_exe_show_table_and_json" },
    Mapping { python: "test_cli_exe_show_without_declared_is_plain_message", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_exe_show_without_declared_is_plain_message" },
    Mapping { python: "test_cli_declared_edit_with_json_emits_the_final_read_view", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_declared_edit_with_json_emits_the_final_read_view" },
    Mapping { python: "test_cli_env_source_on_non_secret_declared_param_warns", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_env_source_on_non_secret_declared_param_warns" },
    Mapping { python: "test_cli_python_manage_with_json_emits_the_final_read_view", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_python_manage_with_json_emits_the_final_read_view" },
    Mapping { python: "test_cli_add_choice_placeholder_on_command_then_run", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_add_choice_placeholder_on_command_then_run" },
    Mapping { python: "test_cli_command_show_enriched_and_env_rider", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_command_show_enriched_and_env_rider" },
    Mapping { python: "test_cli_command_env_rider_only_no_placeholders", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_command_env_rider_only_no_placeholders" },
    Mapping { python: "test_cli_python_declared_op_is_refused", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_python_declared_op_is_refused" },
    Mapping { python: "test_cli_declared_malformed_value_warns", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_declared_malformed_value_warns" },
    Mapping { python: "test_cli_declared_warning_codes_render", path: "crates/skit-cli/tests/port_test_declared_warning_codes.rs", rust: "test_cli_declared_warning_codes_render" },
    Mapping { python: "test_cli_bad_type_warns_and_skips", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_bad_type_warns_and_skips" },
    Mapping { python: "test_cli_secret_override_persists_value_now_that_it_isnt_secret", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_secret_override_persists_value_now_that_it_isnt_secret" },
    Mapping { python: "test_cli_secret_declared_env_purges_prior_plaintext", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_secret_declared_env_purges_prior_plaintext" },
    Mapping { python: "test_cli_declared_secret_env_source_resolves_without_prompting", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_declared_secret_env_source_resolves_without_prompting" },
    Mapping { python: "test_cli_run_set_env_and_placeholder_dry_run", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_run_set_env_and_placeholder_dry_run" },
    Mapping { python: "test_cli_rm_declared_param", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_rm_declared_param" },
    Mapping { python: "test_cli_exe_declared_show_json_param_origin", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_exe_declared_show_json_param_origin" },
    Mapping { python: "test_cli_exe_no_declared_show_json_param_origin_none", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_cli_exe_no_declared_show_json_param_origin_none" },
    Mapping { python: "test_cli_command_env_show_json_source_env", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_command_env_show_json_source_env" },
    Mapping { python: "test_cli_exe_show_masks_secret_default_and_last_value", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_exe_show_masks_secret_default_and_last_value" },
    Mapping { python: "test_cli_command_show_masks_secret_placeholder_and_undeclared", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_cli_command_show_masks_secret_placeholder_and_undeclared" },
    Mapping { python: "test_declared_add_on_interpreted_meta_kind_defaults_to_deliverable_flag", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_declared_add_on_interpreted_meta_kind_defaults_to_deliverable_flag" },
    Mapping { python: "test_declared_add_on_interpreted_kind_delivers_at_run", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_declared_add_on_interpreted_kind_delivers_at_run" },
    Mapping { python: "test_reader_kind_declared_env_rider_merges_not_erases", path: "crates/skit-form/tests/port_test_declared_form_edges.rs", rust: "test_reader_kind_declared_env_rider_merges_not_erases" },
    Mapping { python: "test_reader_kind_declared_rows_stand_alone_when_no_readable_surface", path: "crates/skit-form/tests/port_test_declared_form_edges.rs", rust: "test_reader_kind_declared_rows_stand_alone_when_no_readable_surface" },
    Mapping { python: "test_declared_table_is_shown_for_an_interpreted_meta_kind", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_declared_table_is_shown_for_an_interpreted_meta_kind" },
    Mapping { python: "test_declared_param_on_an_interpreted_kind_actually_delivers", path: "crates/skit-cli/tests/port_test_declared_params_runtime.rs", rust: "test_declared_param_on_an_interpreted_kind_actually_delivers" },
    Mapping { python: "test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_template_add_of_a_non_placeholder_name_creates_a_deliverable_env_row" },
    Mapping { python: "test_template_add_of_a_real_placeholder_name_still_fills_the_slot", path: "crates/skit-cli/tests/port_test_declared_params_cli.rs", rust: "test_template_add_of_a_real_placeholder_name_still_fills_the_slot" },
];

#[test]
fn every_python_declared_params_test_has_an_executable_rust_oracle() {
    assert_eq!(MAPPINGS.len(), 52, "Python module test count changed in the frozen oracle");
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let mut missing = Vec::new();
    for mapping in MAPPINGS {
        let source = fs::read_to_string(repo.join(mapping.path)).unwrap();
        let needle = format!("fn {}(", mapping.rust);
        if !source.contains("#[test]") || !source.contains(&needle) {
            missing.push(format!(
                "{} -> {}::{}",
                mapping.python, mapping.path, mapping.rust
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "mapped Python declared-parameter tests disappeared:\n{}",
        missing.join("\n")
    );
}
