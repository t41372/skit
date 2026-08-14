//! Exact frozen-name accounting for `main@206f9ef:tests/test_cli.py`.
//!
//! The executable owners deliberately cross the real CLI, filesystem, PTY, reducer, renderer,
//! editor, and child-process boundaries that replace Python's Typer/Rich/Textual seams. A failing
//! owner is a Rust parity finding. The five closed names are limited to Python-only private helper
//! or arbitrary monkeypatch injection seams; this allowlist is fixed and may not silently grow.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use syn::{Attribute, Item};

const FROZEN: &[&str] = &[
    "test_version_flag_prints_and_exits",
    "test_add_python_copy",
    "test_add_python_reference_skips_onboarding",
    "test_add_rejects_non_py",
    "test_add_needs_path",
    "test_add_exe_needs_path",
    "test_add_exe",
    "test_add_exe_no_input_never_asks",
    "test_add_exe_missing_path_errors_before_any_ask",
    "test_add_cmd_needs_name",
    "test_add_cmd_with_params",
    "test_add_with_explicit_deps_records",
    "test_add_name_conflict_errors",
    "test_add_missing_path_clean_error_not_traceback",
    "test_add_directory_path_clean_error_not_traceback",
    "test_add_unknown_directory_suggests_exe_and_exits_usage",
    "test_add_unknown_directory_with_exe_is_accepted",
    "test_add_onboards_params_non_interactive_skips",
    "test_list_empty",
    "test_list_table",
    "test_list_json",
    "test_list_table_marks_missing_target",
    "test_list_table_does_not_mark_healthy_or_command_entries",
    "test_list_json_missing_field",
    "test_list_and_show_human_faces_use_translated_kind_labels",
    "test_list_table_name_column_escapes_markup",
    "test_remove_not_found",
    "test_remove_with_yes",
    "test_params_not_found",
    "test_params_empty",
    "test_params_command_entry",
    "test_params_command_no_placeholders",
    "test_deps_view",
    "test_deps_not_found",
    "test_deps_not_python",
    "test_deps_set",
    "test_deps_view_with_requires_python",
    "test_deps_command_strips_a_whitespace_only_python_constraint",
    "test_run_not_found_exits_127",
    "test_run_unknown_preset_rejected",
    "test_run_command_reuses_last_extra_args",
    "test_run_nonzero_exit_propagates",
    "test_preset_list_none",
    "test_preset_list_shows",
    "test_preset_list_not_found",
    "test_preset_delete",
    "test_preset_delete_unknown",
    "test_preset_delete_not_found",
    "test_preset_save_not_found",
    "test_preset_save_python_no_params",
    "test_preset_save_command_no_params",
    "test_add_summary_escapes_markup_in_name_and_description",
    "test_add_deps_summary_escapes_markup",
    "test_add_not_py_file_warning_escapes_markup_in_filename",
    "test_remove_escapes_markup_in_name",
    "test_not_found_error_escapes_markup_in_argument",
    "test_params_command_placeholder_line_escapes_markup",
    "test_preset_list_escapes_markup_in_name_and_values",
    "test_preset_delete_unknown_escapes_markup_in_preset_name",
    "test_validate_preset_unknown_escapes_markup",
    "test_deps_view_escapes_markup",
    "test_deps_set_summary_escapes_markup",
    "test_doctor_missing_reference_escapes_markup_in_name",
    "test_config_set_unknown_language_escapes_markup",
    "test_config_set_unknown_mirror_escapes_markup",
    "test_edit_missing_reference_source_escapes_markup_in_path",
    "test_list_table_renders_markup_literally_end_to_end",
    "test_params_python_table_with_secret",
    "test_params_secret_purges_stored_last_value_and_presets",
    "test_params_secret_does_not_purge_other_still_public_params",
    "test_params_edit_without_stored_value_prints_no_purge_message",
    "test_doctor_uv_found",
    "test_doctor_uv_missing",
    "test_doctor_rebuild",
    "test_doctor_reports_missing_reference",
    "test_run_python_with_params_injects",
    "test_run_extra_args_bypass_required_field_validation",
    "test_run_required_field_missing_without_extra_args_exits_125",
    "test_run_raw_skips_form",
    "test_run_passes_and_remembers_extra_args",
    "test_run_bad_typed_value_caught_at_validation",
    "test_run_command_entry_collects_values",
    "test_resolve_metadata_existing_block_not_asked",
    "test_resolve_metadata_explicit_opts",
    "test_resolve_metadata_explicit_opts_strips_and_drops_empties",
    "test_resolve_metadata_no_suggestions",
    "test_resolve_metadata_non_interactive_uses_suggestions",
    "test_prompt_identity_non_interactive_passes_through",
    "test_prompt_identity_explicit_values_skip_prompts",
    "test_onboard_params_no_candidates",
    "test_onboard_params_non_interactive_returns_empty",
    "test_command_placeholders_prefill_from_last",
    "test_command_without_placeholders_has_no_fields",
    "test_params_table_escapes_markup_in_name_and_default",
    "test_doctor_uv_path_escapes_markup",
    "test_edit_params_updated_summary_escapes_markup_in_name",
    "test_edit_params_malformed_prompt_escapes_markup",
    "test_run_reusing_last_arguments_escapes_markup",
    "test_run_raw_passes_argv_genuinely_raw",
    "test_run_cli_argv_not_reexpanded",
    "test_remove_confirm_abort",
    "test_preset_save_command_with_params",
    "test_preset_save_command_escapes_markup_in_preset_name_and_entry_name",
    "test_preset_save_prompt_escapes_markup_in_placeholder_name",
    "test_command_placeholders_collect_interactively",
    "test_collect_param_form_interactive_secret",
    "test_param_form_prefill_uses_definition_default",
    "test_collect_command_values_prompt_escapes_markup_in_placeholder_name",
    "test_collect_param_form_prompt_escapes_markup_in_param_prompt_text",
    "test_edit_reports_escape_markup_in_name",
    "test_edit_reference_mode_escapes_markup_in_name_and_path",
    "test_add_read_error_reports_clean_message",
    "test_add_unreadable_file_clean_error_not_traceback",
    "test_list_description_exact_marker_when_no_description",
    "test_list_description_appends_marker_after_description",
    "test_list_description_healthy_and_command_entries_untouched",
    "test_list_description_escapes_markup_in_description",
    "test_list_description_escapes_markup_in_missing_path",
    "test_run_shim_error",
    "test_run_launch_error",
    "test_add_interactive_tui_form_opens_review_panel",
    "test_no_subcommand_dispatches_to_tui",
    "test_add_interactive_panel_cancel_exits_130",
    "test_add_interactive_plain_form_keeps_line_prompts",
    "test_add_term_dumb_keeps_line_prompts",
    "test_add_exe_interactive_line_asks_name_and_description",
    "test_add_exe_interactive_skips_asks_when_name_and_description_given",
    "test_resolve_metadata_interactive",
    "test_resolve_metadata_interactive_dash_clears_deps",
    "test_resolve_metadata_interactive_none_word_clears_deps",
    "test_prompt_identity_prompts_name_and_description",
    "test_prompt_identity_blank_name_falls_back_to_stem",
    "test_onboard_params_framework_detected",
    "test_onboard_params_interactive_selection",
    "test_paramspec_from_candidate_roundtrip",
    "test_parse_selection_variants",
    "test_parse_selection_ignores_non_ascii_digit_like_chars",
    "test_parse_kv_opts",
    "test_params_candidates_line_escapes_markup_in_name",
    "test_doctor_rebuild_problem_line_escapes_markup",
];

const CLOSED: &[&str] = &[
    // Direct unit tests of Python's line-prompt multi-select parser. Ratatui uses typed review
    // actions rather than parsing comma/range text from a private CLI helper.
    "test_parse_selection_variants",
    "test_parse_selection_ignores_non_ascii_digit_like_chars",
    // Direct unit test of Python's private KEY=VALUE line-prompt parser. Rust forms carry typed
    // fields/values and expose no equivalent textual parser seam.
    "test_parse_kv_opts",
    // Python monkeypatches the analyzer to return an otherwise impossible hostile candidate name
    // solely to exercise a private line-summary printer. Real prompt/parameter labels and markup
    // are covered by executable Ratatui render tests.
    "test_params_candidates_line_escapes_markup_in_name",
    // Python replaces doctor_rebuild with an arbitrary hostile problem string to test the private
    // Rich printer. Rust has no arbitrary problem-injection seam; real doctor names/paths and
    // missing-reference output are executable above.
    "test_doctor_rebuild_problem_line_escapes_markup",
];

const OWNER_FILES: &[&str] = &[
    "crates/skit-cli/tests/port_test_cli_public_core.rs",
    "crates/skit-cli/tests/port_test_cli_run_preset_public.rs",
    "crates/skit-cli/tests/port_test_cli_markup_public.rs",
    "crates/skit-cli/tests/port_test_cli_params_doctor_public.rs",
    "crates/skit-cli/tests/port_test_cli_run_python_public.rs",
    "crates/skit-cli/tests/port_test_cli_metadata_public.rs",
    "crates/skit-cli/tests/port_test_cli_public_edges.rs",
    "crates/skit-cli/tests/port_test_cli_pty_public.rs",
    "crates/skit-ui/tests/port_test_cli_form_collection.rs",
    "crates/skit-tui/tests/port_test_cli_form_markup.rs",
    "crates/skit-cli/tests/port_test_cli_edit_markup_public.rs",
    "crates/skit-cli/tests/port_test_cli_read_list_edges.rs",
    "crates/skit-cli/tests/port_test_cli_run_failures.rs",
    "crates/skit-ui/tests/port_test_cli_add_review_route.rs",
    "crates/skit-cli/tests/port_test_cli_interactive_routes.rs",
    "crates/skit-ui/tests/port_test_cli_review_onboarding.rs",
];

fn has_test_attribute(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("test"))
}

fn parity_tests(path: &Path) -> Vec<String> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let file = syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("could not parse {} as Rust: {error}", path.display()));
    file.items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(function) if has_test_attribute(&function.attrs) => {
                let name = function.sig.ident.to_string();
                name.starts_with("test_").then_some(name)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn frozen_cli_partition_is_exact() {
    let frozen = FROZEN.iter().copied().collect::<BTreeSet<_>>();
    let closed = CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(FROZEN.len(), 140, "frozen test_cli.py denominator drifted");
    assert_eq!(frozen.len(), 140, "duplicate frozen CLI name");
    assert_eq!(
        CLOSED.len(),
        5,
        "CLI architecture-closure allowlist may not expand or shrink silently"
    );
    assert_eq!(closed.len(), 5, "duplicate architecture-closed CLI name");
    assert!(
        closed.is_subset(&frozen),
        "closed CLI names must come from the frozen Python surface"
    );

    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-cli lives at <repo>/crates/skit-cli");
    let mut owners = BTreeMap::<String, String>::new();
    let mut duplicates = Vec::new();
    for relative in OWNER_FILES {
        let path = repo.join(relative);
        for name in parity_tests(&path) {
            if let Some(previous) = owners.insert(name.clone(), (*relative).to_owned()) {
                duplicates.push(format!("{name}: {previous} and {relative}"));
            }
        }
    }
    assert!(
        duplicates.is_empty(),
        "duplicate CLI parity owners:\n{}",
        duplicates.join("\n")
    );

    let expected = frozen
        .difference(&closed)
        .copied()
        .collect::<BTreeSet<_>>();
    let actual = owners.keys().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 135, "executable CLI partition must stay 135/140");
    assert_eq!(
        actual.len(),
        135,
        "canonical CLI owner files must contain exactly 135 parity tests"
    );

    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    let extras = actual.difference(&expected).copied().collect::<Vec<_>>();
    assert!(
        missing.is_empty() && extras.is_empty(),
        "CLI exact-name mismatch; missing={missing:?}, extras={extras:?}"
    );
    assert!(
        closed.iter().all(|name| !actual.contains(name)),
        "an architecture-closed CLI name must not also claim an executable owner"
    );
}
