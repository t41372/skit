//! Exact-name completeness gate for Python v0.4 `tests/test_js_deps.py` at `main@206f9ef`.
//!
//! The frozen module contains 151 `def test_` functions. 136 have executable Rust owners.
//! Fifteen are narrowly architecture-closed private seams whose Python-only parser/monkeypatch
//! injection points do not exist in the Rust rewrite; their reasons are fixed below. This gate
//! rejects missing/duplicate executable owners and any expansion of the closure allowlist.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_external_imports_covers_all_import_forms",
    "test_external_imports_excludes_non_packages",
    "test_external_imports_rejects_malformed_scoped_specifiers",
    "test_external_imports_maps_deep_imports_to_the_package_root",
    "test_external_imports_skips_unreadable_specifiers",
    "test_external_imports_reads_typescript_under_the_ts_grammar",
    "test_external_imports_degrades_to_empty_on_a_parse_error",
    "test_external_imports_ignores_an_import_statement_without_a_string_source",
    "test_split_requirement",
    "test_manifest_text_is_deterministic_and_private",
    "test_manifest_text_skips_an_empty_requirement",
    "test_clean_removes_manifest_lockfiles_and_node_modules",
    "test_clean_on_an_already_clean_dir_is_a_no_op",
    "test_require_installer_maps_runner_to_its_own_installer",
    "test_require_installer_missing_raises_126_family",
    "test_ensure_installed_writes_manifest_runs_installer_and_stamps",
    "test_ensure_installed_uses_the_runners_own_installer",
    "test_ensure_installed_fresh_marker_short_circuits",
    "test_ensure_installed_stale_marker_rebuilds_from_scratch",
    "test_ensure_installed_failure_without_stderr_still_reports",
    "test_ensure_installed_spawn_oserror_is_wrapped",
    "test_ensure_installed_missing_installer_raises_before_touching_the_dir",
    "test_ensure_installed_stamps_even_when_installer_creates_no_node_modules",
    "test_module_type_for",
    "test_manifest_text_carries_the_module_type",
    "test_split_requirement_boundary_shapes",
    "test_module_type_for_multi_dot_sources",
    "test_manifest_text_exact_layout",
    "test_ensure_installed_writes_the_module_type_into_the_manifest",
    "test_module_type_for_a_bare_dotfile_name",
    "test_ensure_module_manifest_writes_the_type",
    "test_ensure_module_manifest_flavorless_writes_nothing",
    "test_ensure_module_manifest_rewrites_a_non_utf8_package_json",
    "test_install_lock_uses_a_persistent_inode_outside_the_entry",
    "test_install_lock_waits_for_a_live_holder",
    "test_ensure_installed_serializes_under_the_entry_lock",
    "test_install_lock_path_survives_entry_directory_removal",
    "test_clear_takes_the_install_lock",
    "test_install_lock_never_unlinks_its_persistent_inode",
    "test_install_lock_reuses_the_same_persistent_inode",
    "test_clean_sweeps_aged_injected_leftovers",
    "test_update_dependencies_js_copy_records_meta_without_touching_the_script",
    "test_update_dependencies_js_reference_is_refused",
    "test_update_dependencies_js_python_constraint_is_refused",
    "test_update_dependencies_js_clearing_sweeps_the_materialized_env",
    "test_update_dependencies_js_reference_clearing_is_allowed",
    "test_add_js_no_input_records_scanned_imports",
    "test_add_js_explicit_dep_flags_win_without_scanning",
    "test_add_js_without_external_imports_records_nothing",
    "test_add_js_reference_mode_asks_no_deps_question",
    "test_deps_command_sets_and_shows_js_dependencies",
    "test_deps_command_python_flag_on_js_is_refused",
    "test_deps_command_dep_on_js_reference_is_refused",
    "test_add_js_ref_with_dep_is_refused_loudly",
    "test_add_js_with_python_flag_is_refused_loudly",
    "test_add_js_empty_dep_records_nothing",
    "test_deps_command_empty_dep_clears_and_sweeps",
    "test_deps_command_write_emits_json_when_asked",
    "test_deps_command_needs_write_emits_json_and_skips_the_human_line",
    "test_deps_command_applies_both_deps_and_needs",
    "test_deps_command_refused_dep_does_not_commit_a_concurrent_need",
    "test_deps_command_drops_empty_and_whitespace_needs",
    "test_add_shell_refuses_unusable_flags_loudly",
    "test_add_cmd_refuses_dep_flag_loudly",
    "test_add_python_still_honors_both_flags",
    "test_add_stdin_honors_explicit_dep_and_python_flags",
    "test_add_stdin_refuses_ref_loudly",
    "test_build_installs_declared_deps_with_the_resolved_runner",
    "test_build_skips_the_engine_without_copy_mode_deps",
    "test_preflight_requires_the_installer_when_deps_are_declared",
    "test_preflight_without_deps_does_not_ask_for_an_installer",
    "test_build_sweeps_aged_injected_leftovers_but_not_fresh_ones",
    "test_write_injected_prefers_entry_dir_when_asked",
    "test_js_injector_honors_prefer_entry_dir",
    "test_flows_marks_prefer_entry_dir_only_for_deps_managed_npm_copies",
    "test_build_passes_the_original_extensions_module_type",
    "test_build_writes_a_module_manifest_for_a_deps_free_module_typed_entry",
    "test_build_writes_no_manifest_for_a_flavorless_deps_free_entry",
    "test_npm_axis_is_independent_of_the_pypi_axis",
    "test_mirror_npm_round_trips_through_save_and_load",
    "test_mirror_env_sets_npm_registry_and_defers_to_the_user",
    "test_mirror_env_without_npm_url_sets_nothing_npm",
    "test_load_mirror_type_hardens_a_hand_edited_npm_value",
    "test_store_remove_waits_for_a_live_js_install_lock",
    "test_store_remove_surfaces_install_lock_failure_without_deleting_entry",
    "test_corrupted_marker_triggers_reinstall_not_a_persistent_crash",
    "test_needs_install_true_without_a_marker",
    "test_needs_install_false_when_the_marker_matches",
    "test_needs_install_true_when_the_declared_deps_changed",
    "test_preflight_skips_the_installer_when_the_marker_is_already_fresh",
    "test_ensure_installed_unknown_runner_falls_back_to_npm_argv",
    "test_write_injected_default_stays_in_the_os_temp_dir",
    "test_settings_js_copy_offers_deps_without_python_constraint",
    "test_settings_js_reference_hides_the_deps_section",
    "test_split_requirements_keeps_scoped_packages_apart",
    "test_settings_save_keeps_scoped_packages_apart",
    "test_tui_direct_add_records_scanned_js_dependencies",
    "test_tui_direct_add_js_without_imports_records_none",
    "test_tui_direct_add_survives_the_source_vanishing_after_the_copy",
    "test_interactive_accept_of_a_scoped_suggestion_round_trips",
    "test_prefs_custom_mirror_saves_the_npm_registry",
    "test_js_and_ts_specs_declare_the_npm_flavor",
    "test_resolve_npm_dependencies_interactive_accepts_the_suggestion",
    "test_resolve_npm_dependencies_interactive_dash_declines",
    "test_resolve_npm_dependencies_interactive_edit_splits_requirements",
    "test_failure_detail_against_real_installer_output",
    "test_failure_detail_names_the_missing_package",
    "test_failure_detail_empty_stderr_degrades",
    "test_failure_detail_drops_bare_paths_even_without_a_cause_line",
    "test_failure_detail_filters_each_noise_marker",
    "test_failure_detail_noise_before_the_cause_still_finds_the_cause",
    "test_failure_detail_drops_every_npm_prefix_noise_shape",
    "test_failure_detail_deno_line_is_reproduced_exactly",
    "test_failure_detail_survives_invalid_utf8_bytes",
    "test_install_announce_line_verbatim",
    "test_install_announces_itself_but_a_fresh_marker_stays_silent",
    "test_install_subprocess_contract_and_marker_dir_reuse",
    "test_dependency_failure_messages_verbatim",
    "test_clean_failure_is_loud_not_silent",
    "test_clean_rmtree_failure_is_loud",
    "test_clean_failure_message_verbatim",
    "test_store_clear_goes_through_the_locked_entry_point",
    "test_update_dependencies_surfaces_clean_failure_as_store_error",
    "test_install_lock_unwritable_dir_raises_126_family_not_a_traceback",
    "test_run_on_unwritable_entry_dir_exits_126_not_1",
    "test_ensure_module_manifest_rewrites_only_on_change",
    "test_clean_unlinks_a_symlinked_node_modules_but_keeps_the_target",
    "test_resolve_npm_dependencies_does_not_prompt_when_stdout_is_piped",
    "test_add_edit_refuses_ref_loudly",
    "test_add_edit_honors_explicit_dep_and_python_flags",
    "test_placeholder_parity_passes_the_shipped_catalogs",
    "test_placeholder_parity_flags_a_swapped_named_placeholder",
    "test_placeholder_parity_flags_a_positional_count_mismatch",
    "test_placeholder_parity_flags_a_positional_conversion_type_swap",
    "test_placeholder_parity_accepts_matching_named_and_plural_forms",
    "test_ensure_installed_installer_failure_carries_its_stderr",
];

const ARCHITECTURE_CLOSED: &[(&str, &str)] = &[
    ("test_resolve_npm_dependencies_without_scanner_suggests_nothing", "Python patched away the dependency scanner; Rust ReviewState only exists after kind/source capture and has no scanner-unavailable state."),
    ("test_resolve_npm_dependencies_unreadable_file_suggests_nothing", "Python's private resolver converted an unreadable path to no suggestions; Rust captures readable source bytes before ReviewState exists, so unreadable-source failure is outside this seam."),
    ("test_settings_save_survives_a_failed_deps_clear", "Python injected a failing persistence callback into the Textual settings screen; Rust exposes frontend-neutral SettingsView save data but no repository-failure injection seam in that view. CLI cleanup atomicity is separately executable."),
    ("test_write_injected_prefer_entry_dir_falls_back_to_os_temp", "Python monkeypatched adjacent tempfile creation to force the private injected-copy fallback. Rust staging has no injectable tempfile-creation seam; adjacent/default placement is covered by executable public launch tests."),
    ("test_sweep_keeps_a_file_exactly_at_the_cutoff", "Python monkeypatched the clock to hit an exact one-hour private sweep cutoff. Rust's private sweeper reads SystemTime::now directly with no injectable clock; aged/fresh public behavior is executable."),
    ("test_sweep_survives_one_failed_unlink_and_still_sweeps_the_rest", "Python monkeypatched unlink to fail for exactly one staged file while later files still sweep. Rust exposes no per-unlink syscall injection seam; public cleanup/failure behavior is executable elsewhere."),
    ("test_clean_tolerates_a_node_modules_symlink_vanishing", "Python monkeypatched a symlink to vanish between lstat and unlink. Rust exposes no deterministic per-syscall race hook; symlink target-safety is executable."),
    ("test_clean_records_a_stuck_symlinked_node_modules", "Python monkeypatched unlink of a symlink to stay stuck and inspected the private cleanup error accumulator. Rust has no public equivalent accumulator or unlink hook."),
    ("test_clean_onexc_treats_an_already_gone_tree_as_success", "Python directly exercised shutil.rmtree(onexc=...) with an already-gone tree. Rust has no shutil/onexc callback seam; public cleanup idempotence is executable."),
    ("test_i18n_gate_catches_an_unquoted_msgstr", "Rust ships compiled static catalog rows rather than parsing gettext .po syntax; an unquoted msgstr parser error is not a Rust product seam."),
    ("test_i18n_gate_passes_the_shipped_catalogs", "Rust has no shipped .po parser gate. The shipped static catalog is covered by executable catalog and placeholder-parity tests."),
    ("test_i18n_gate_catches_an_unquoted_continuation_line", "Rust ships compiled static catalog rows rather than parsing gettext .po continuation syntax."),
    ("test_placeholder_parity_ignores_fuzzy_entries", "Fuzzy is gettext .po metadata; Rust's static catalog has no fuzzy-entry state."),
    ("test_placeholder_parity_skips_an_untranslated_plural_form", "Untranslated plural forms are gettext .po parser/catalog-build metadata; Rust static rows have no untranslated plural-form state."),
    ("test_po_syntax_allows_a_valid_msgctxt_line", "msgctxt line syntax belongs to the removed gettext .po parser; Rust static catalog rows have no msgctxt parser seam."),
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("JS-deps port source must parse")
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
fn test_js_deps_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 136, "the executable JS-deps partition changed");
    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 136, "duplicate expected executable names make accounting dishonest");

    let closed = ARCHITECTURE_CLOSED.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>();
    assert_eq!(ARCHITECTURE_CLOSED.len(), 15, "architecture closure expanded or shrank without review");
    assert_eq!(closed.len(), 15, "duplicate architecture-closed names make accounting dishonest");
    assert!(expected.is_disjoint(&closed), "a JS-deps contract is both executable and architecture-closed");
    assert_eq!(expected.len() + closed.len(), 151, "the frozen test_js_deps.py denominator changed");
    assert!(ARCHITECTURE_CLOSED.iter().all(|(_, reason)| !reason.trim().is_empty()), "every closure needs a concrete architecture reason");

    let mut actual_names = Vec::new();
    for source in [
        include_str!("../../skit-language/tests/port_test_javascript_dependencies.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_core.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_module_type.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_locking.rs"),
        include_str!("port_test_js_deps_exact_cli_store.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_launch.rs"),
        include_str!("port_test_js_deps_exact_run_boundaries.rs"),
        include_str!("../../skit-store/tests/port_test_js_deps_exact_mirror.rs"),
        include_str!("../../skit-store/tests/port_test_js_deps_exact_remove.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_freshness.rs"),
        include_str!("port_test_js_deps_exact_default_injection.rs"),
        include_str!("../../skit-ui/tests/port_test_js_deps_exact_settings.rs"),
        include_str!("../../skit-ui/tests/port_test_js_deps_exact_add_review.rs"),
        include_str!("../../skit-ui/tests/port_test_js_deps_exact_preferences.rs"),
        include_str!("../../skit-ui/tests/port_test_js_deps_exact_review_more.rs"),
        include_str!("port_test_js_deps_exact_diagnostics.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_failures.rs"),
        include_str!("port_test_js_deps_exact_cleanup_store.rs"),
        include_str!("port_test_js_deps_exact_lock_errors.rs"),
        include_str!("../../skit-runtime/tests/port_test_js_deps_exact_tail_public.rs"),
        include_str!("port_test_js_deps_exact_redirected.rs"),
        include_str!("../../skit-i18n/tests/port_test_js_deps_i18n_parity.rs"),
        include_str!("port_test_js_deps_exact_installer_stderr.rs"),
    ] {
        let source_names = names(source);
        for name in &source_names {
            assert!(expected.contains(name.as_str()), "dedicated JS-deps owner file contains an unexpected parity-shaped test name: {name}");
        }
        actual_names.extend(source_names);
    }

    // This shared editor-lane file also owns frozen tests from other Python modules; select only
    // the two test_js_deps.py names instead of misclassifying those legitimate foreign owners.
    actual_names.extend(
        names(include_str!("port_test_add_lane_editor.rs"))
            .into_iter()
            .filter(|name| expected.contains(name.as_str())),
    );

    assert_eq!(actual_names.len(), 136, "JS-deps executable parity has missing or duplicate owners");
    let actual = actual_names.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(actual.len(), 136, "one frozen JS-deps name has more than one executable owner");
    assert_eq!(actual, expected, "JS-deps executable parity is incomplete or mislabeled");
}
