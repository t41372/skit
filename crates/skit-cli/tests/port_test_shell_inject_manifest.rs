//! Exact-name completeness gate for Python v0.4 `tests/test_shell_inject.py` at `main@206f9ef`.
//!
//! The frozen module has 87 `def test_` functions. Public behavioral contracts remain executable
//! even when current Rust behavior is red. Only Python-private fault seams or structured helper
//! projections with no equivalent public Rust API are closed here; Rust-additive strengthening does
//! not count toward the executable set.

use std::collections::BTreeSet;
use syn::{Attribute, Item};

const EXECUTABLE: &[&str] = &[
    "test_const_injection_runs_with_the_new_value",
    "test_const_str_is_single_quoted_and_int_is_bare",
    "test_const_rewrites_every_same_name_occurrence",
    "test_const_quoting_is_normalized_not_preserved",
    "test_bad_int_value_raises_the_value_error_not_drift",
    "test_bad_float_and_non_finite_values_are_refused",
    "test_float_const_injects_a_bare_number",
    "test_missing_const_target_is_drift",
    "test_readonly_const_is_never_a_target",
    "test_const_targets_skip_array_and_valueless_assignments",
    "test_no_values_writes_nothing_at_all",
    "test_env_delivery_writes_no_temp_file",
    "test_env_delivery_actually_reaches_the_script",
    "test_mixed_env_and_const_delivery",
    "test_read_interception_echoes_prompt_and_value",
    "test_read_rewrite_keeps_every_flag_and_varname",
    "test_secret_read_masks_the_echo_but_delivers_the_value",
    "test_read_in_a_loop_takes_the_value_once_then_reads_real_stdin",
    "test_function_read_defined_above_invoked_after_keeps_its_value",
    "test_two_specs_claiming_one_read_site_is_drift",
    "test_vanished_read_site_is_drift",
    "test_value_follows_its_prompt_not_its_position",
    "test_multi_variable_read_joins_its_values_on_one_line",
    "test_multi_variable_read_accepts_a_short_prefix",
    "test_multi_variable_read_refuses_a_positional_gap",
    "test_multi_variable_read_refuses_whitespace_in_a_non_last_field",
    "test_read_refuses_a_newline_in_any_field_including_a_single_variable",
    "test_read_refuses_edge_whitespace_that_the_shell_would_strip",
    "test_read_accepts_a_carriage_return_which_the_shell_delivers_intact",
    "test_multi_variable_read_refuses_whitespace_when_a_trailing_var_is_unmanaged",
    "test_multi_variable_read_refuses_a_newline_in_a_non_last_field",
    "test_multi_variable_read_allows_whitespace_in_the_last_field",
    "test_execute_reports_a_whitespace_split_as_a_bad_value",
    "test_builtin_read_spelling_is_rewritten_whole",
    "test_unmanaged_read_still_reads_real_stdin",
    "test_the_preamble_runs_on_every_supported_dialect",
    "test_set_u_and_set_e_survive_the_preamble",
    "test_const_payload_is_inert",
    "test_read_payload_is_inert",
    "test_quote_in_a_read_prompt_survives",
    "test_secret_value_never_reaches_stdout",
    "test_cjk_emoji_const_and_prompt_round_trip",
    "test_crlf_script_injects_and_runs",
    "test_no_trailing_newline_script_injects",
    "test_no_shebang_puts_the_preamble_at_the_very_top",
    "test_preamble_lands_after_the_shebang",
    "test_backslash_values_arrive_byte_identical_raw_or_not",
    "test_reframing_and_custom_ifs_reads_are_never_offered",
    "test_fallthrough_keyword_is_dialect_selected",
    "test_interpreter_gate_refuses_what_the_offline_gate_missed",
    "test_interpreter_gate_is_skipped_when_the_shell_is_not_installed",
    "test_interpreter_gate_reports_an_empty_stderr_without_crashing",
    "test_self_location_warns_when_a_temp_copy_is_written",
    "test_self_location_does_not_warn_for_env_delivery",
    "test_no_self_location_no_warning",
    "test_normalize_rewrites_only_that_assignments_bytes",
    "test_normalize_makes_the_param_an_envdefault",
    "test_normalized_script_still_runs_standalone",
    "test_execute_runs_a_shell_entry_with_injected_values",
    "test_execute_runs_a_managed_read_with_the_block_in_place",
    "test_execute_env_delivery_writes_no_temp_copy",
    "test_run_refuses_a_bad_value_before_it_ever_launches",
    "test_execute_maps_a_drifted_shell_definition_to_drift",
    "test_execute_reports_a_positional_gap_as_a_bad_value",
    "test_execute_surfaces_the_self_location_warning",
    "test_execute_syntax_gate_failure_never_launches",
    "test_execute_without_an_injector_does_not_crash",
    "test_cli_dry_run_shows_the_command",
    "test_cli_normalize_turns_a_const_into_an_env_param",
    "test_cli_normalized_param_runs_through_the_environment",
    "test_cli_normalize_reports_refusals",
    "test_cli_normalize_refuses_a_non_shell_kind",
    "test_cli_normalize_refuses_reference_mode",
    "test_cli_normalize_without_a_stored_copy",
    "test_split_guard_refuses_only_what_the_shell_would_actually_mangle",
    "test_params_warns_when_a_self_locating_script_has_injectable_consts",
    "test_params_does_not_warn_when_the_script_never_self_locates",
    "test_empty_value_in_a_non_last_read_variable_is_a_gap",
    "test_empty_value_in_the_last_read_variable_is_fine",
];

const ARCHITECTURE_CLOSED: &[&str] = &[
    // Python monkeypatches the private quote helper to corrupt generated output. Rust exposes only
    // the real parser-backed injector, so recreating a broken quote function in test code would be
    // a fake implementation seam rather than the frozen public behavior.
    "test_offline_gate_refuses_a_corrupted_injection",
    // Python monkeypatches subprocess.run itself to raise OSError during gate 2. Rust has no public
    // deterministic gate-spawn fault port separate from the later real child launch.
    "test_interpreter_gate_survives_a_spawn_failure",
    // Python's normalizer returns a structured multi-name result carrying exact refusal codes.
    // Rust exposes one-name normalize_shell_default -> Result and intentionally collapses these
    // categories to LanguageError; the full refusal matrix remains covered by rust_additive tests.
    "test_normalize_refuses_and_leaves_the_source_untouched",
    "test_normalize_ignores_array_and_valueless_assignments",
    "test_normalize_on_an_unparseable_script_changes_nothing",
    "test_normalize_mixed_batch_reports_each_name",
    // Direct test of Python cli._render_normalize_warning, a private formatter helper. Rust has no
    // public equivalent renderer; end-to-end normalize/refusal CLI messages are tested separately.
    "test_cli_normalize_warning_renderer_covers_every_code",
    // Same structured `unsafe-literal:NAME` result seam as the normalization batch above. Rust's
    // public one-name API refuses every metacharacter and additive coverage pins the whole matrix,
    // but it does not expose the frozen Python refusal-code projection.
    "test_normalize_refuses_shell_metacharacters",
];

fn has_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| attribute.path().is_ident("test"))
}

fn names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .expect("shell injection port source must parse")
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
fn test_shell_inject_frozen_names_are_exactly_accounted() {
    assert_eq!(EXECUTABLE.len(), 79);
    assert_eq!(ARCHITECTURE_CLOSED.len(), 8);

    let expected = EXECUTABLE.iter().copied().collect::<BTreeSet<_>>();
    let closed = ARCHITECTURE_CLOSED.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(expected.len(), 79, "duplicate executable names make the accounting dishonest");
    assert_eq!(closed.len(), 8, "duplicate closed names make the accounting dishonest");
    assert!(expected.is_disjoint(&closed));
    assert_eq!(expected.len() + closed.len(), 87);

    let mut actual_names = Vec::new();
    for source in [
        include_str!("../../skit-language/tests/port_test_shell_inject_core.rs"),
        include_str!("../../skit-language/tests/port_test_shell_inject_interpreter_absence.rs"),
        include_str!("../../skit-language/tests/port_test_shell_inject_normalize.rs"),
        include_str!("../../skit-language/tests/port_test_shell_inject_split_edges.rs"),
        include_str!("../../skit-language/tests/port_test_shell_normalize_strengthening.rs"),
        include_str!("port_test_shell_inject_env.rs"),
        include_str!("port_test_shell_inject_execute.rs"),
        include_str!("port_test_shell_inject_execute_env_plan.rs"),
        include_str!("port_test_shell_inject_gate2.rs"),
        include_str!("port_test_shell_inject_security.rs"),
        include_str!("port_test_shell_inject_cli.rs"),
        include_str!("port_test_shell_inject_warnings.rs"),
        include_str!("port_test_shell_inject_no_injector.rs"),
    ] {
        actual_names.extend(names(source));
    }
    assert_eq!(
        actual_names.len(),
        79,
        "canonical shell-inject sources contain duplicate or extra frozen-looking test_* functions"
    );
    let actual = actual_names.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "shell injection executable parity is incomplete, duplicated, or mislabeled"
    );
    assert!(actual.is_disjoint(&closed));
}
