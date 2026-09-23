//! Completeness guard for Python v0.4 `tests/test_shell_analyzer_mut.py`.
//!
//! Thirty tests have equivalent public Rust analyzer observables. Four Python tests inspect the
//! private `ReadFlags` helper's internal booleans/string/list shape; they stay explicitly blocked
//! rather than being represented by partial candidate assertions.

use std::{fs, path::Path};

const EXECUTABLE: &[&str] = &[
    "test_const_candidate_carries_exact_lineno_and_secret",
    "test_const_non_secret_lineno_deeper_in_file",
    "test_envdefault_candidate_carries_exact_lineno_and_secret",
    "test_read_candidate_carries_exact_lineno_and_str_type",
    "test_demoted_const_reports_accumulator_reason",
    "test_plain_loop_body_reassignment_is_the_only_demotion_path",
    "test_plain_while_loop_reassignment_demotes",
    "test_bare_assigned_skips_subscript_then_records_later_clobber",
    "test_bare_assigned_self_read_continues_to_later_clobber",
    "test_const_scan_skips_plus_equals_then_finds_later_const",
    "test_envdefault_scan_skips_nondefault_operator_then_finds_default",
    "test_envdefault_scan_skips_subscript_then_finds_scalar",
    "test_toplevel_skips_local_then_yields_later_const",
    "test_injectable_reads_skip_reframing_then_include_normal_read",
    "test_dash_p_prompt_consumes_only_a_real_next_arg",
    "test_dash_p_prompt_then_varname",
    "test_command_prefix_without_read_is_not_a_read",
    "test_builtin_prefix_without_read_is_not_a_read",
    "test_builtin_read_still_recognized",
    "test_non_ifs_var_prefix_does_not_exclude_the_read",
    "test_ifs_prefix_still_excludes_the_read",
    "test_non_let_command_arguments_are_not_scanned_for_targets",
    "test_let_command_targets_are_demoted",
    "test_long_option_containing_r_is_not_treated_as_readonly",
    "test_short_r_flag_is_readonly",
    "test_uses_argv_at_alone",
    "test_uses_argv_star_alone",
    "test_uses_argv_hash_alone",
    "test_secret_flag_after_another_cluster_letter_terminates",
    "test_unknown_flag_letter_after_another_terminates",
];

const BLOCKED: &[&str] = &[
    "test_plain_read_flag_shape",
    "test_secret_read_flag_shape",
    "test_raw_read_flag_shape",
    "test_scan_cluster_no_prompt_leaves_prompt_empty",
];

fn test_names(source: &str) -> Vec<String> {
    syn::parse_file(source)
        .unwrap()
        .items
        .into_iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function)
                if function
                    .attrs
                    .iter()
                    .any(|attribute| attribute.path().is_ident("test")) =>
            {
                Some(function.sig.ident.to_string())
            }
            _ => None,
        })
        .collect()
}

#[test]
fn shell_analyzer_mutation_coverage_is_thirty_executable_four_blocked() {
    assert_eq!(EXECUTABLE.len() + BLOCKED.len(), 34);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let source = fs::read_to_string(
        repo.join("crates/skit-form/tests/port_test_shell_analyzer_mut_exact.rs"),
    )
    .unwrap();
    assert_eq!(
        test_names(&source),
        EXECUTABLE
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    );
    for blocked in BLOCKED {
        assert!(
            !source.contains(&format!("fn {blocked}(")),
            "blocked private ReadFlags contract {blocked} must not be faked as executable coverage"
        );
    }
}
