//! Occurrence-level accounting for Python v0.4 `tests/test_benchmarks_tooling.py`.
//!
//! Bare names are not unique: `test_rejects_bad_inputs` occurs in two classes. Keep the raw
//! occurrence list until after multiplicity checks. An invalid PR fixture does not close an oracle
//! contract; every occurrence must have an active exact owner, an explicit host gate, a structured
//! architecture closure, or a named stronger native owner.

use std::{collections::BTreeMap, fs, path::Path};

const ORACLE: &[(&str, &str)] = &[
    ("test_round_trip", "TestResultsModel"),
    ("test_rejects_wrong_schema_version", "TestResultsModel"),
    ("test_rejects_non_json", "TestResultsModel"),
    ("test_rejects_non_object", "TestResultsModel"),
    ("test_rejects_bad_metric_fields", "TestResultsModel"),
    ("test_rejects_bad_skip_entry", "TestResultsModel"),
    ("test_rejects_empty_meta_strings", "TestResultsModel"),
    ("test_ci_runner_null_is_valid", "TestResultsModel"),
    ("test_meta_from_dict_matches_round_trip", "TestResultsModel"),
    ("test_python_major_minor", "TestResultsModel"),
    ("test_suite_output_round_trip", "TestResultsModel"),
    ("test_suite_output_rejects_bad_duration", "TestResultsModel"),
    (
        "test_suite_output_rejects_non_object_raw",
        "TestResultsModel",
    ),
    ("test_rejects_bad_results_structure", "TestResultsModel"),
    ("test_loads_the_real_contract_file", "TestBudgetLoader"),
    ("test_rejects_malformed_rows", "TestBudgetLoader"),
    ("test_pass_and_violation", "TestBudgetEvaluation"),
    ("test_missing_metric_fails_enforced", "TestBudgetEvaluation"),
    (
        "test_missing_metric_reported_not_failed_for_target",
        "TestBudgetEvaluation",
    ),
    ("test_profile_predicate_scopes_row", "TestBudgetEvaluation"),
    ("test_platform_predicate", "TestBudgetEvaluation"),
    (
        "test_empty_platform_key_is_unevaluable",
        "TestBudgetEvaluation",
    ),
    (
        "test_empty_ci_runner_is_unevaluable",
        "TestBudgetEvaluation",
    ),
    (
        "test_ci_only_row_not_applicable_locally",
        "TestBudgetEvaluation",
    ),
    (
        "test_python_mismatch_fails_on_ci_only",
        "TestBudgetEvaluation",
    ),
    (
        "test_stale_ceiling_warns_on_ratchet_rows_only",
        "TestBudgetEvaluation",
    ),
    (
        "test_render_report_tally_and_stale_nudge",
        "TestBudgetEvaluation",
    ),
    (
        "test_enforced_evaluated_counts_verdicts_not_na",
        "TestBudgetEvaluation",
    ),
    ("test_refreshes_ratchet_rows_only", "TestBudgetPropose"),
    (
        "test_propose_anchors_a_pr_artifact_on_the_pr_number",
        "TestBudgetPropose",
    ),
    ("test_propose_requires_the_metric", "TestBudgetPropose"),
    ("test_propose_refuses_a_local_artifact", "TestBudgetPropose"),
    ("test_propose_refuses_a_dirty_tree", "TestBudgetPropose"),
    ("test_propose_refuses_to_widen_a_bound", "TestBudgetPropose"),
    (
        "test_propose_widens_when_the_increase_is_declared",
        "TestBudgetPropose",
    ),
    (
        "test_propose_keeps_a_hand_set_headroom_on_a_non_ratchet_row",
        "TestBudgetPropose",
    ),
    ("test_render_budgets_round_trips", "TestBudgetPropose"),
    ("test_stats", "TestParsers"),
    ("test_census", "TestParsers"),
    ("test_importtime", "TestParsers"),
    ("test_vmhwm", "TestParsers"),
    ("test_maxrss", "TestParsers"),
    ("test_strace", "TestParsers"),
    ("test_pyperf", "TestParsers"),
    ("test_build_argv", "TestHyperfine"),
    ("test_build_argv_quotes_awkward_paths", "TestHyperfine"),
    ("test_parse_export", "TestHyperfine"),
    ("test_metric_from_times", "TestHyperfine"),
    ("test_thresholds", "TestCompare"),
    ("test_exact_units_ignore_the_noise_threshold", "TestCompare"),
    ("test_render_reports_each_side_skips", "TestCompare"),
    ("test_zero_base", "TestCompare"),
    ("test_render", "TestCompare"),
    ("test_render_only_in_sections", "TestCompare"),
    (
        "test_unit_mismatch_is_loud_and_never_mints_a_false_delta",
        "TestCompare",
    ),
    ("test_platform_key", "TestEnvinfo"),
    ("test_ci_runner", "TestEnvinfo"),
    ("test_cpu_model", "TestEnvinfo"),
    ("test_mem_and_git", "TestEnvinfo"),
    ("test_uv_version_from_output", "TestEnvinfo"),
    ("test_installed_uv_version", "TestEnvinfo"),
    ("test_dist_version_fallback", "TestEnvinfo"),
    ("test_build_host_and_meta", "TestEnvinfo"),
    ("test_pull_request_number", "TestEnvinfo"),
    ("test_profiles", "TestPipeline"),
    ("test_dataset_ns", "TestPipeline"),
    ("test_merge_and_derive", "TestPipeline"),
    ("test_merge_rejects_duplicate_ids", "TestPipeline"),
    ("test_merge_rejects_reserved_pipeline_ids", "TestPipeline"),
    ("test_merge_rejects_derived_collision", "TestPipeline"),
    ("test_render_markdown", "TestPipeline"),
    ("test_summarize_dir", "TestPipeline"),
    ("test_summarize_dir_failures", "TestPipeline"),
    ("test_exact_line_counts", "TestSources"),
    ("test_deterministic", "TestSources"),
    ("test_rejects_bad_inputs", "TestSources"),
    ("test_analyzer_constructs_present", "TestSources"),
    ("test_generate_asserts_generator_line_count", "TestSources"),
    ("test_generate_small_library", "TestDatasets"),
    ("test_generate_is_deterministic", "TestDatasets"),
    ("test_generate_env_is_restored", "TestDatasets"),
    ("test_kind_mix_and_missing_targets_at_100", "TestDatasets"),
    ("test_state_fraction", "TestDatasets"),
    ("test_refuses_non_empty_root", "TestDatasets"),
    ("test_rejects_bad_inputs", "TestDatasets"),
    ("test_manifest_round_trip_and_mid_slug", "TestDatasets"),
    ("test_runover_library", "TestDatasets"),
    (
        "test_scoped_skit_dirs_restores_previously_unset_var",
        "TestDatasets",
    ),
    ("test_source_text_rejects_unknown_kind", "TestDatasets"),
    (
        "test_generate_refuses_silent_store_undercount",
        "TestDatasets",
    ),
    (
        "test_runover_refuses_silent_store_undercount",
        "TestDatasets",
    ),
    ("test_datasets_command", "TestFrontDoor"),
    ("test_run_and_summarize_commands", "TestFrontDoor"),
    ("test_check_exit_codes", "TestFrontDoor"),
    ("test_check_require_enforced", "TestFrontDoor"),
    ("test_check_propose_prints_toml", "TestFrontDoor"),
    ("test_cli_formats_os_errors", "TestFrontDoor"),
    ("test_cli_formats_subprocess_errors", "TestFrontDoor"),
    ("test_compare_command", "TestFrontDoor"),
    ("test_budgets_file_is_canonical", "TestContractSync"),
    (
        "test_hyperfine_pin_synced_to_install_action",
        "TestContractSync",
    ),
    (
        "test_the_census_probe_runs_the_real_console_script",
        "TestContractSync",
    ),
    (
        "test_analyzer_source_filenames_share_one_registry",
        "TestContractSync",
    ),
    (
        "test_analyzer_workloads_are_byte_stable",
        "TestContractSync",
    ),
    (
        "test_the_library_footprint_metrics_divide_into_each_other",
        "TestContractSync",
    ),
    (
        "test_broken_lines_constant_is_the_same_in_both_files",
        "TestContractSync",
    ),
    (
        "test_broken_workloads_are_byte_stable_and_actually_broken",
        "TestContractSync",
    ),
    (
        "test_analyzers_survive_a_half_written_source",
        "TestContractSync",
    ),
    ("test_cli_parser_surface_is_stable", "TestContractSync"),
    (
        "test_all_benchmark_subprocesses_are_bounded",
        "TestContractSync",
    ),
    (
        "test_workflows_install_hyperfine_via_the_action",
        "TestContractSync",
    ),
    (
        "test_compare_workflow_pins_pyperf_to_the_harness_lock",
        "TestContractSync",
    ),
    ("test_ci_runner_label_matches_runs_on", "TestContractSync"),
    (
        "test_build_env_composes_path_and_pins_locale",
        "TestEnvspec",
    ),
    (
        "test_build_env_dedupes_and_tolerates_missing_tools",
        "TestEnvspec",
    ),
    ("test_bench_path_is_what_build_env_exports", "TestEnvspec"),
    ("test_build_env_refuses_non_dataset_roots", "TestEnvspec"),
    ("test_pyperf_inherit_covers_the_fixture_vars", "TestEnvspec"),
    (
        "test_check_reusable_accepts_a_fresh_dataset",
        "TestDatasetReuse",
    ),
    ("test_check_reusable_rejects_any_drift", "TestDatasetReuse"),
    ("test_manifest_stamps_the_writing_skit", "TestDatasetReuse"),
    ("test_js_ts_braces_balance", "TestSourceValidity"),
    ("test_python_compiles", "TestSourceValidity"),
    (
        "test_tree_sitter_parses_without_errors",
        "TestSourceValidity",
    ),
    (
        "test_dataset_guarantees_both_sides_of_the_filter_assertion",
        "TestSearchProbeInvariant",
    ),
    (
        "test_compare_excludes_pipeline_self_timings",
        "TestReviewFixes",
    ),
    (
        "test_budget_bounds_render_as_plain_numbers",
        "TestReviewFixes",
    ),
    (
        "test_hyperfine_metrics_from_export_mints_ids",
        "TestReviewFixes",
    ),
    ("test_fractional_bounds_render_compactly", "TestReviewFixes"),
    (
        "test_benchmarks_imports_stay_on_runtime_deps",
        "TestHarnessImportSurface",
    ),
    (
        "test_footprint_closure_bounds_and_isolates_retries",
        "TestCodeReviewFixes",
    ),
    (
        "test_rss_keeps_samples_and_full_statistics",
        "TestCodeReviewFixes",
    ),
    (
        "test_tui_keeps_import_and_rss_samples",
        "TestCodeReviewFixes",
    ),
    (
        "test_tui_records_the_selection_span_when_the_probe_measured_one",
        "TestCodeReviewFixes",
    ),
    ("test_cold_parse_keeps_raw_samples", "TestCodeReviewFixes"),
    (
        "test_micro_deltas_clear_the_us_floor",
        "TestCodeReviewFixes",
    ),
    (
        "test_compare_flags_incomparable_sides",
        "TestCodeReviewFixes",
    ),
    (
        "test_compare_flags_harness_provenance_changes",
        "TestCodeReviewFixes",
    ),
    (
        "test_old_schema_defaults_missing_harness_provenance",
        "TestCodeReviewFixes",
    ),
    (
        "test_results_rejects_invalid_harness_provenance",
        "TestCodeReviewFixes",
    ),
    (
        "test_budgets_reject_non_finite_bounds",
        "TestCodeReviewFixes",
    ),
    (
        "test_results_reject_non_finite_values",
        "TestCodeReviewFixes",
    ),
    (
        "test_pyperf_parser_rejects_malformed_elements",
        "TestCodeReviewFixes",
    ),
    (
        "test_hyperfine_parser_rejects_malformed_entries",
        "TestCodeReviewFixes",
    ),
    (
        "test_derive_strict_pair_half_present_fails_loud",
        "TestCodeReviewFixes",
    ),
    (
        "test_derive_scale_grid_half_is_legitimate",
        "TestCodeReviewFixes",
    ),
    (
        "test_merge_rejects_duplicate_suite_outputs",
        "TestCodeReviewFixes",
    ),
    (
        "test_summarize_dir_rejects_corrupt_run_json",
        "TestCodeReviewFixes",
    ),
    (
        "test_summarize_dir_rejects_non_finite_total",
        "TestCodeReviewFixes",
    ),
    (
        "test_skip_all_fills_both_suite_fields",
        "TestCodeReviewFixes",
    ),
    (
        "test_compare_profile_carries_compare_mode",
        "TestCodeReviewFixes",
    ),
    (
        "test_suites_needing_a_library_declare_it",
        "TestCodeReviewFixes",
    ),
    (
        "test_imports_also_censuses_a_populated_library",
        "TestCodeReviewFixes",
    ),
    (
        "test_headline_keeps_both_census_tiers",
        "TestCodeReviewFixes",
    ),
    (
        "test_corrupt_manifest_reports_the_remedy",
        "TestCodeReviewFixes",
    ),
    (
        "test_manifest_records_the_probe_char",
        "TestCodeReviewFixes",
    ),
];

const EXACT_ACTIVE: &[&str] = &[
    "test_loads_the_real_contract_file",
    "test_budgets_file_is_canonical",
    "test_budget_bounds_render_as_plain_numbers",
    "test_fractional_bounds_render_compactly",
    "test_generate_refuses_silent_store_undercount",
    "test_runover_refuses_silent_store_undercount",
    "test_summarize_dir_rejects_corrupt_run_json",
    "test_analyzer_source_filenames_share_one_registry",
    "test_broken_workloads_are_byte_stable_and_actually_broken",
    "test_workflows_install_hyperfine_via_the_action",
    "test_compare_workflow_pins_pyperf_to_the_harness_lock",
    "test_ci_runner_label_matches_runs_on",
    "test_footprint_closure_bounds_and_isolates_retries",
    "test_the_library_footprint_metrics_divide_into_each_other",
    "test_tui_keeps_import_and_rss_samples",
    "test_tui_records_the_selection_span_when_the_probe_measured_one",
    "test_cold_parse_keeps_raw_samples",
];

const HOST_GATES: &[(&str, &str)] = &[(
    "test_python_compiles",
    "CPython 3.13 compiles every shipped Python benchmark subject on all three CI hosts; CI fails if the exact gate runs zero tests",
)];

const CLOSURES: &[(&str, &str, &str)] = &[
    (
        "test_census",
        "Python sys.modules census parser has no native Rust input surface",
        "crates/skit-benchmarks/src/suites/imports.rs",
    ),
    (
        "test_importtime",
        "Python -X importtime parser has no native Rust input surface",
        "crates/skit-benchmarks/src/suites/imports.rs",
    ),
    (
        "test_pyperf",
        "the native harness never consumes Python pyperf JSON",
        "crates/skit-benchmarks/src/suites/micro.rs",
    ),
    (
        "test_pyperf_parser_rejects_malformed_elements",
        "the native harness never parses Python pyperf elements",
        "crates/skit-benchmarks/src/suites/micro.rs",
    ),
    (
        "test_pyperf_inherit_covers_the_fixture_vars",
        "the native harness has no pyperf worker inheritance seam",
        "crates/skit-benchmarks/src/runner.rs",
    ),
    (
        "test_ci_runner",
        "Rust reads a typed nonempty process variable instead of an injected Python map",
        "crates/skit-benchmarks/src/environment.rs",
    ),
    (
        "test_cpu_model",
        "Rust uses sysinfo and has no /proc text parser seam",
        "crates/skit-benchmarks/src/environment.rs",
    ),
    (
        "test_mem_and_git",
        "Rust uses sysinfo and a bounded git child instead of Python text helpers",
        "crates/skit-benchmarks/src/environment.rs",
    ),
    (
        "test_dist_version_fallback",
        "the native harness has no Python distribution metadata",
        "crates/skit-benchmarks/src/environment.rs",
    ),
    (
        "test_build_host_and_meta",
        "Rust builds host metadata from bounded live probes, not injected Python helpers",
        "crates/skit-benchmarks/src/environment.rs",
    ),
    (
        "test_deterministic",
        "the native source generator owns one fixed seed and no alternate-seed seam",
        "crates/skit-benchmarks/src/sources.rs",
    ),
    (
        "test_generate_asserts_generator_line_count",
        "the native private generator callback cannot be monkeypatched",
        "crates/skit-benchmarks/src/sources.rs",
    ),
    (
        "test_scoped_skit_dirs_restores_previously_unset_var",
        "Rust passes explicit roots and mutates no process-global SKIT directory variables",
        "crates/skit-benchmarks/src/dataset.rs",
    ),
    (
        "test_source_text_rejects_unknown_kind",
        "the private source helper is behind public typed kind validation",
        "crates/skit-benchmarks/src/sources.rs",
    ),
    (
        "test_run_and_summarize_commands",
        "Rust has no replaceable Python dispatch function; public commands have real front-door owners",
        "crates/skit-benchmarks/src/bin/skit-bench.rs",
    ),
    (
        "test_cli_formats_subprocess_errors",
        "Rust has no Python TimeoutExpired wrapper type; real process failures have typed owners",
        "crates/skit-benchmarks/src/bin/skit-bench.rs",
    ),
    (
        "test_hyperfine_pin_synced_to_install_action",
        "Rust removed the duplicate code pin; the installer action is authoritative",
        ".github/actions/install-hyperfine/action.yml",
    ),
    (
        "test_the_census_probe_runs_the_real_console_script",
        "the native imports suite records Python census as not applicable",
        "crates/skit-benchmarks/src/suites/imports.rs",
    ),
    (
        "test_broken_lines_constant_is_the_same_in_both_files",
        "Rust owns one BROKEN_LINES constant",
        "crates/skit-benchmarks/src/suites/micro.rs",
    ),
    (
        "test_cli_parser_surface_is_stable",
        "Rust uses Clap and has no private argparse parser rendering",
        "crates/skit-benchmarks/src/bin/skit-bench.rs",
    ),
    (
        "test_all_benchmark_subprocesses_are_bounded",
        "ProcessSpec requires a timeout by type",
        "crates/skit-benchmarks/src/process.rs",
    ),
    (
        "test_benchmarks_imports_stay_on_runtime_deps",
        "Cargo owns the native dependency graph; no Python import surface remains",
        "crates/skit-benchmarks/Cargo.toml",
    ),
];

fn stronger_owner(class: &str) -> &'static str {
    match class {
        "TestResultsModel" => {
            "crates/skit-benchmarks/src/lib.rs typed Results/SuiteOutput validation and round-trip owners"
        }
        "TestBudgetLoader" | "TestBudgetEvaluation" | "TestBudgetPropose" => {
            "crates/skit-benchmarks/src/budget.rs loader/evaluation/proposal owners plus tests/core_contract.rs"
        }
        "TestParsers" => {
            "crates/skit-benchmarks/src/parsers.rs native parser success and malformed-input owners"
        }
        "TestHyperfine" => "crates/skit-benchmarks/src/hyperfine.rs argv/export/statistics owners",
        "TestCompare" | "TestReviewFixes" => {
            "crates/skit-benchmarks/src/compare.rs comparison and rendering owners"
        }
        "TestEnvinfo" => "crates/skit-benchmarks/src/environment.rs bounded host metadata owners",
        "TestPipeline" => {
            "crates/skit-benchmarks/src/pipeline.rs merge/derive/report/publication owners"
        }
        "TestSources" => {
            "crates/skit-benchmarks/src/sources.rs exact-count/input/construct source owners"
        }
        "TestDatasets" | "TestDatasetReuse" => {
            "crates/skit-benchmarks/src/dataset.rs generation/reuse/manifest/refusal owners"
        }
        "TestFrontDoor" => {
            "crates/skit-benchmarks/tests/front_door_contract.rs real command and typed failure owners"
        }
        "TestContractSync" => {
            "crates/skit-benchmarks/tests/front_door_contract.rs and source_contract.rs workflow/source integrity owners"
        }
        "TestEnvspec" => {
            "crates/skit-benchmarks/src/runner.rs complete child environment and dataset-root owners"
        }
        "TestSourceValidity" => {
            "crates/skit-benchmarks/tests/source_contract.rs real grammar and compiler owners"
        }
        "TestSearchProbeInvariant" => {
            "crates/skit-benchmarks/src/tui_probe.rs generated filter-probe invariant owner"
        }
        "TestCodeReviewFixes" => {
            "crates/skit-benchmarks/src budget/report/pipeline and suites modules own the typed review invariants"
        }
        "TestHarnessImportSurface" => unreachable!("the only HarnessImportSurface row is closed"),
        other => panic!("unclassified benchmark oracle class: {other}"),
    }
}

fn counts<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for name in names {
        *counts.entry(name).or_default() += 1;
    }
    counts
}

#[derive(Debug)]
struct RustTest {
    name: String,
    ignored: bool,
}

fn scan_rust_tests(path: &Path, tests: &mut Vec<RustTest>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            scan_rust_tests(&entry.unwrap().path(), tests);
        }
        return;
    }
    if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
        return;
    }
    let source = fs::read_to_string(path).unwrap();
    let lines = source.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let Some(tail) = line.trim_start().strip_prefix("fn test_") else {
            continue;
        };
        let Some((bare, _)) = tail.split_once('(') else {
            continue;
        };
        let attributes = lines[..index]
            .iter()
            .rev()
            .map(|line| line.trim())
            .take_while(|line| line.is_empty() || line.starts_with("#["))
            .collect::<Vec<_>>();
        if attributes.iter().any(|line| *line == "#[test]") {
            tests.push(RustTest {
                name: format!("test_{bare}"),
                ignored: attributes.iter().any(|line| line.starts_with("#[ignore")),
            });
        }
    }
}

#[test]
fn benchmark_tooling_occurrence_manifest_is_complete() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    assert_eq!(ORACLE.len(), 156);
    let oracle_counts = counts(ORACLE.iter().map(|(name, _)| *name));
    assert_eq!(oracle_counts.len(), 155);
    assert_eq!(oracle_counts["test_rejects_bad_inputs"], 2);

    let exact = counts(EXACT_ACTIVE.iter().copied());
    let gates = counts(HOST_GATES.iter().map(|(name, _)| *name));
    let closures = counts(CLOSURES.iter().map(|(name, _, _)| *name));
    assert_eq!(exact.values().sum::<usize>(), 17);
    assert_eq!(gates.values().sum::<usize>(), 1);
    assert_eq!(closures.values().sum::<usize>(), 22);
    assert!(
        HOST_GATES
            .iter()
            .all(|(_, reason)| !reason.trim().is_empty())
    );
    assert!(CLOSURES.iter().all(|(_, reason, owner)| {
        !reason.trim().is_empty() && !owner.trim().is_empty() && repository.join(owner).exists()
    }));

    let mut category_counts = BTreeMap::<&str, usize>::new();
    for (name, class) in ORACLE {
        let category = if exact.contains_key(name) {
            "exact"
        } else if gates.contains_key(name) {
            "gate"
        } else if closures.contains_key(name) {
            "closure"
        } else {
            let owner = stronger_owner(class);
            assert!(
                !owner.trim().is_empty(),
                "missing stronger owner for {name}"
            );
            "stronger"
        };
        *category_counts.entry(category).or_default() += 1;
    }
    assert_eq!(category_counts["exact"], 17);
    assert_eq!(category_counts["gate"], 1);
    assert_eq!(category_counts["closure"], 22);
    assert_eq!(category_counts["stronger"], 116);

    let mut rust_tests = Vec::new();
    scan_rust_tests(&repository.join("crates/skit-benchmarks"), &mut rust_tests);
    let observed = rust_tests
        .iter()
        .filter(|test| oracle_counts.contains_key(test.name.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        observed.len(),
        18,
        "unexpected exact owner drift: {observed:?}"
    );
    for name in EXACT_ACTIVE {
        let matches = observed
            .iter()
            .filter(|test| test.name == *name)
            .collect::<Vec<_>>();
        assert_eq!(
            matches.len(),
            1,
            "active owner multiplicity changed: {name}"
        );
        assert!(!matches[0].ignored, "exact owner became ignored: {name}");
    }
    for (name, _) in HOST_GATES {
        let matches = observed
            .iter()
            .filter(|test| test.name == *name)
            .collect::<Vec<_>>();
        assert_eq!(matches.len(), 1, "host gate multiplicity changed: {name}");
        assert!(
            matches[0].ignored,
            "host-tool owner must stay an explicit gate: {name}"
        );
    }
    for (name, _, _) in CLOSURES {
        assert!(
            observed.iter().all(|test| test.name != *name),
            "closure also has an exact function owner: {name}"
        );
    }
}
