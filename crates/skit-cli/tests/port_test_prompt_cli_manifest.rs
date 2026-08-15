//! Exact frozen-name accounting for `main@206f9ef:tests/test_prompt_cli.py`.
//!
//! Prompt CLI parity is owned at the strongest Rust boundary available: real `skit` processes,
//! persisted files/config/state, real PTYs, real editor/runner children, deterministic lock/CAS
//! races, Ratatui picker events, and frontend-neutral review state. Failing owners are parity
//! findings. The single closed name is a Python-private read-count injection seam with no
//! deterministic Rust callback/lock boundary in the dry-run path; public dry-run body, limit,
//! masking, runner, and process behavior remains executable. This closure may not silently grow.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::Path};
use syn::{Attribute, Item};

const FROZEN: &[&str] = &[
    "test_add_prompt_missing_file_is_clean_on_the_panel_face",
    "test_add_prompt_unknown_runner_flag_is_usage_error",
    "test_add_runner_flag_without_prompt_is_refused",
    "test_add_prompt_conflicts_with_other_kind_flags",
    "test_add_prompt_flag_forces_the_kind_on_any_extension",
    "test_add_bare_md_no_input_requires_explicit_prompt",
    "test_missing_bare_md_is_refused_before_the_prompt_confirmation",
    "test_executable_lane_preserves_the_existing_non_file_contract",
    "test_add_prompt_file_no_input_manages_everything",
    "test_add_prompt_secret_summary_states_both_sides_of_boundary",
    "test_add_prompt_interactive_tick_subset_and_runner_pick",
    "test_add_prompt_runner_flag_non_interactive",
    "test_add_prompt_from_stdin_needs_a_name",
    "test_add_prompt_from_stdin",
    "test_add_kind_prompt_from_stdin_uses_the_prompt_contract",
    "test_add_prompt_from_stdin_empty_body",
    "test_add_prompt_editor_lane_routes_to_stdin_when_not_interactive",
    "test_add_prompt_ref_mode_keeps_original_and_pins_invoke",
    "test_add_prompt_no_path_with_ref_is_refused",
    "test_run_prompt_no_input_without_pin_is_126",
    "test_run_no_input_is_provably_unaffected_by_last_picked_state",
    "test_run_prompt_unknown_runner_is_126_listing_names",
    "test_run_prompt_pinned_but_removed_runner_is_126",
    "test_run_unpinned_prompt_with_empty_runner_list_teaches_a_copyable_recovery",
    "test_run_runner_flag_on_non_prompt_is_usage_error",
    "test_run_prompt_dry_run_prints_the_resolved_argv",
    "test_run_prompt_dry_run_missing_body_is_127_before_output",
    "test_overlong_prompt_refuses_before_normal_transparency",
    "test_dry_run_refuses_nul_without_looking_up_agent_binary",
    "test_dry_run_refuses_overlong_prompt_without_printing_it",
    "test_prompt_extra_agent_args_do_not_fill_required_placeholders",
    "test_run_prompt_secret_placeholder_masked_in_dry_run",
    "test_run_prompt_runner_flag_threads_through",
    "test_run_prompt_unicode_placeholder_threads_through_set",
    "test_run_prompt_pin_resolves_without_touching_last_picked",
    "test_run_prompt_extra_args_pass_through_after_dashes",
    "test_run_prompt_reuses_last_extra_agent_args",
    "test_normal_prompt_transparency_omits_body_but_keeps_agent_flags",
    "test_runner_list_materializes_the_seeds",
    "test_runner_list_json",
    "test_runner_list_all_json_exposes_stable_raw_indexes_and_reasons",
    "test_runner_list_empty_state",
    "test_runner_list_without_amp_omits_the_one_shot_note",
    "test_runner_add_with_flag_bearing_argv",
    "test_runner_add_preserves_bad_rows_and_force_repairs_matching_name",
    "test_runner_add_blank_name_is_refused_before_seeding",
    "test_runner_add_validation_errors",
    "test_runner_add_duplicate_name_refused",
    "test_runner_add_reports_malformed_config_container",
    "test_runner_remove_and_unknown",
    "test_runner_remove_blank_name_is_usage_error_before_seeding",
    "test_runner_remove_rejects_ambiguous_or_invalid_targets_before_writing",
    "test_removing_every_runner_stays_empty",
    "test_runner_remove_warns_and_preserves_affected_prompt_pins",
    "test_runner_remove_raw_row_is_targeted_and_requires_yes_noninteractively",
    "test_runner_remove_raw_duplicate_has_no_false_pin_warning_or_key_removed_claim",
    "test_runner_remove_raw_valid_row_requires_stable_name_path",
    "test_runner_remove_container_repairs_only_targeted_prompt_value",
    "test_params_read_view_shows_unmanaged_and_gone",
    "test_params_json_carries_runner_and_unmanaged",
    "test_params_add_manages_a_body_placeholder",
    "test_params_rm_unmanages_even_without_a_declared_row",
    "test_params_add_unknown_name_becomes_env_rider",
    "test_params_deliver_placeholder_is_allowed_on_prompts",
    "test_params_runner_pin_and_clear",
    "test_params_runner_pin_with_json_emits_the_read_view",
    "test_params_workdir_with_json_emits_the_read_view",
    "test_params_interpolate_with_json_emits_the_read_view",
    "test_params_runner_pin_validates_the_name",
    "test_params_runner_pin_refused_on_non_prompt",
    "test_show_json_prompt_additions",
    "test_show_json_non_prompt_has_no_runner_keys",
    "test_show_human_prints_the_runner_line",
    "test_show_human_no_fields_names_prompt_and_command_receivers",
    "test_doctor_reports_prompt_drift_and_bad_runner_rows",
    "test_doctor_healthy_prompt_reports_no_drift",
    "test_doctor_skips_a_prompt_whose_body_is_gone",
    "test_add_prompt_interactive_selection",
    "test_add_prompt_plain_identity_defaults_drop_compound_suffix",
    "test_add_prompt_plain_identity_accepts_user_overrides",
    "test_add_prompt_interactive_tui_form_opens_review_panel",
    "test_add_interactive_off_answer_disables_insertion",
    "test_add_interactive_explicit_all_beats_the_flood_cap",
    "test_add_prompt_interactive_runner_pick_pins_and_remembers",
    "test_add_prompt_interactive_panel_cancel_exits_130",
    "test_add_prompt_unknown_runner_refused_before_the_panel",
    "test_add_prompt_term_dumb_keeps_line_prompts",
    "test_add_flood_cap_manages_nothing_and_says_so",
    "test_add_no_interpolate",
    "test_add_no_interpolate_refused_off_the_prompt_lanes",
    "test_add_no_interpolate_refused_up_front_on_non_prompt_path_lane",
    "test_add_no_interpolate_through_stdin_lane",
    "test_params_interpolate_off_and_on",
    "test_params_interpolate_refused_on_non_prompt",
    "test_params_unmanaged_listing_is_flood_capped_and_localizable",
    "test_params_unmanaged_tail_passes_through_the_i18n_boundary",
    "test_show_reports_the_interpolate_switch",
    "test_doctor_skips_drift_for_an_insertion_off_prompt",
    "test_run_insertion_off_prompt_rejects_set_and_sends_verbatim",
    "test_edit_prompt_non_interactive_names_the_unmanaged_variable",
    "test_edit_prompt_non_interactive_flood_previews_with_a_tail",
    "test_edit_prompt_with_no_new_placeholders_is_silent",
    "test_edit_non_prompt_keeps_the_generic_drift_hint",
    "test_edit_prompt_interactive_offers_and_manages_a_new_placeholder",
    "test_edit_prompt_interactive_none_leaves_the_placeholder_literal",
    "test_edit_prompt_interactive_numbers_manage_the_named_ones",
    "test_edit_prompt_preserves_existing_managed_and_adds_the_new_one",
    "test_edit_prompt_interactive_flood_previews_secret_mark_and_tail",
    "test_runner_remove_confirms_unless_yes",
    "test_runner_remove_abort_keeps_the_runner",
    "test_runner_remove_raw_row_refuses_if_index_shifted_during_confirmation",
    "test_runner_remove_name_refuses_if_key_is_replaced_during_confirmation",
    "test_umbrella_cli_help_uses_entry_taxonomy_in_the_requested_locale",
    "test_prompt_only_library_uses_entry_taxonomy_on_dynamic_cli_surfaces",
    "test_empty_library_does_not_claim_it_only_accepts_scripts",
    "test_localized_starter_is_minimal_and_never_creates_its_own_field",
    "test_add_prompt_editor_lane_interactive",
    "test_add_prompt_editor_lane_untouched_starter_adds_nothing",
    "test_add_prompt_editor_lane_asks_for_a_name",
    "test_add_prompt_editor_lane_name_taken_refuses_before_the_editor",
    "test_add_prompt_editor_lane_post_edit_failure_keeps_the_draft",
    "test_add_prompt_editor_lane_deleted_draft_is_a_clean_honest_failure",
    "test_extra_argv_does_not_hide_a_filled_flag_type_error",
    "test_real_prompt_run_warns_before_sending_a_nonempty_secret",
    "test_noninteractive_pi_run_warns_and_uses_lossy_fallback",
    "test_noninteractive_pi_dry_run_warns_and_shows_fallback",
    "test_missing_runner_binary_refuses_before_any_delivery_output",
    "test_add_prompt_unreadable_file_is_a_store_error",
    "test_add_runner_flag_refused_on_cmd_edit_exe_lanes",
    "test_add_prompt_stdin_lane_reports_store_errors",
    "test_params_view_survives_an_unreadable_reference_body",
    "test_params_schema_edits_refused_while_insertion_is_off",
    "test_add_prompt_read_oserror_is_a_clean_store_error",
    "test_complete_runner_names",
    "test_runner_list_all_preserves_anonymous_argv_and_localizes_human_status",
    "test_add_interactive_flood_defaults_to_none_and_caps_the_listing",
    "test_add_interactive_flooded_numbers_address_the_previewed_names_only",
    "test_edit_prompt_tui_reconcile_manages_the_pickers_selection",
    "test_edit_prompt_tui_reconcile_none_manages_nothing",
    "test_edit_prompt_tui_reconcile_flood_preselects_nothing",
    "test_add_bare_md_interactive_ask_yes_and_no",
    "test_add_bare_md_confirm_no_falls_through_to_kind_ask_and_honors_pick",
    "test_run_prompt_interactive_ask_prefilled_from_last_picked",
    "test_run_prompt_inline_stale_pin_prefills_last_configured_pick",
    "test_params_runner_pin_reports_store_errors",
    "test_params_interpolate_reports_store_errors",
    "test_add_prompt_editor_lane_reports_store_errors",
    "test_real_run_spawns_the_same_prompt_snapshot_it_validated",
    "test_real_run_transparency_and_amp_note_use_the_prepared_runner_row",
    "test_dry_run_prints_the_same_prompt_snapshot_it_validated",
];

const CLOSED: &[(&str, &str)] = &[(
    "test_dry_run_prints_the_same_prompt_snapshot_it_validated",
    "Python monkeypatches the private PromptLaunch._read_body loader to return different bytes on consecutive calls and asserts the loader was called exactly once. Rust dry-run reads source_snapshot into an owned String before validation and preview, but exposes no reader callback, prepare/lock boundary, or other deterministic seam between those two internal uses. Public dry-run content, NUL/length refusal, runner selection, and masking remain executable; only the artificial second-read injection/count seam is closed.",
)];

const OWNER_FILES: &[&str] = &[
    "crates/skit-cli/tests/port_test_prompt_cli_public_add.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_add_stdin.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_run_refusals.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_run_process.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_runner_management.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_params_show_doctor.rs",
    "crates/skit-ui/tests/port_test_prompt_cli_add_review.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_interactive_add.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_interpolate_flood.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_edit_reconcile.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_runner_confirmation.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_taxonomy_help.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_editor_lane.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_run_safety.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_public_edges_a.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_public_edges_b.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_read_error.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_completion_locale.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_flood_plain.rs",
    "crates/skit-tui/tests/port_test_prompt_cli_reconcile_picker.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_markdown_interactive.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_interactive_run.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_params_store_errors.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_editor_store_error.rs",
    "crates/skit-cli/tests/port_test_prompt_cli_snapshot_races.rs",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn parity_tests(path: &Path) -> Vec<String> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let file = syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("could not parse {} as Rust: {error}", path.display()));
    file.items.iter().filter_map(|item| match item {
        Item::Fn(function) if has_test_attribute(&function.attrs) => {
            let name = function.sig.ident.to_string();
            name.starts_with("test_").then_some(name)
        }
        _ => None,
    }).collect()
}

#[test]
fn frozen_prompt_cli_partition_is_exact() {
    let frozen = FROZEN.iter().copied().collect::<BTreeSet<_>>();
    let closed = CLOSED.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>();
    assert_eq!(FROZEN.len(), 150, "frozen test_prompt_cli.py denominator drifted");
    assert_eq!(frozen.len(), 150, "duplicate frozen Prompt-CLI name");
    assert_eq!(CLOSED.len(), 1, "Prompt-CLI architecture-closure allowlist may not change silently");
    assert_eq!(closed.len(), 1, "duplicate architecture-closed Prompt-CLI name");
    assert!(CLOSED.iter().all(|(_, reason)| !reason.trim().is_empty()), "closed Prompt-CLI names need concrete reasons");
    assert!(closed.is_subset(&frozen), "closed Prompt-CLI names must be frozen Python names");

    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent)
        .expect("skit-cli lives at <repo>/crates/skit-cli");
    let mut owners = BTreeMap::<String, String>::new();
    let mut duplicates = Vec::new();
    for relative in OWNER_FILES {
        for name in parity_tests(&repo.join(relative)) {
            if let Some(previous) = owners.insert(name.clone(), (*relative).to_owned()) {
                duplicates.push(format!("{name}: {previous} and {relative}"));
            }
        }
    }
    assert!(duplicates.is_empty(), "duplicate Prompt-CLI parity owners:\n{}", duplicates.join("\n"));

    let expected = frozen.difference(&closed).copied().collect::<BTreeSet<_>>();
    let actual = owners.keys().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 149, "executable Prompt-CLI partition must stay 149/150");
    assert_eq!(actual.len(), 149, "canonical Prompt-CLI owner files must contain exactly 149 parity tests");
    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    let extras = actual.difference(&expected).copied().collect::<Vec<_>>();
    assert!(missing.is_empty() && extras.is_empty(), "Prompt-CLI exact-name mismatch; missing={missing:?}, extras={extras:?}");
    assert!(closed.iter().all(|name| !actual.contains(name)), "a closed Prompt-CLI name must not also have an executable owner");
}
