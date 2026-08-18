//! Frozen review-fix contracts from `tests/test_benchmarks_tooling.py`.

use std::{collections::BTreeMap, fs};

use serde_json::{Value, json};
use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results, Skip, SuiteKind, SuiteOutput,
    build_plan, merge,
    budget::{evaluate, load_budgets, render_report},
    compare::{compare, render_markdown},
    dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, DatasetManifest, SEARCH_PROBE_CHAR, check_reusable, generate},
    hyperfine::{metrics_from_export, parse_export},
    report::{HEADLINE_METRICS, summarize_directory},
};
use tempfile::TempDir;

fn meta(profile: BenchmarkProfile) -> Meta {
    Meta {
        generated_at: "2026-07-20T12:00:00+00:00".to_owned(),
        profile,
        git: GitInfo { commit: "abcdef1234567890".to_owned(), dirty: false, pr: None },
        skit_version: "0.2.1.dev0".to_owned(),
        host: HostInfo {
            os: "Linux".to_owned(),
            kernel: "6.8.0".to_owned(),
            cpu: "Test CPU".to_owned(),
            cpu_count: 8,
            mem_total_mib: 16_384,
            platform_key: "linux-x86_64".to_owned(),
            ci_runner: Some("ubuntu-24.04".to_owned()),
            ci_image_version: Some("20260719.1".to_owned()),
        },
        python: "3.13.5".to_owned(),
        uv: "0.11.26".to_owned(),
        textual: "8.2.8".to_owned(),
        pyperf: "2.10.0".to_owned(),
    }
}

fn metric(value: f64, unit: &str, n: usize) -> Metric {
    Metric { value, unit: unit.to_owned(), n, p95: None, stddev: None }
}

fn results(profile: BenchmarkProfile, metrics: &[(&str, f64, &str, usize)]) -> Results {
    Results {
        schema_version: 1,
        meta: meta(profile),
        metrics: metrics.iter().map(|(name, value, unit, n)| ((*name).to_owned(), metric(*value, unit, *n))).collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

fn output(suite: SuiteKind, metrics: &[(&str, f64, &str, usize)]) -> SuiteOutput {
    SuiteOutput {
        suite,
        duration_seconds: 0.0,
        metrics: metrics.iter().map(|(name, value, unit, n)| ((*name).to_owned(), metric(*value, unit, *n))).collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn test_compare_excludes_pipeline_self_timings() {
    let base = results(BenchmarkProfile::Pr, &[
        ("pipeline.duration_s", 100.0, "s", 1),
        ("pipeline.suite.tui.duration_s", 10.0, "s", 1),
        ("startup.version.median_ms", 100.0, "ms", 5),
    ]);
    let head = results(BenchmarkProfile::Pr, &[
        ("startup.version.median_ms", 200.0, "ms", 5),
    ]);
    let comparison = compare(&base, &head);
    assert_eq!(comparison.deltas.iter().map(|delta| delta.metric.as_str()).collect::<Vec<_>>(), vec!["startup.version.median_ms"]);
    assert!(comparison.only_base.is_empty());
}

#[test]
fn test_budget_bounds_render_as_plain_numbers() {
    let budgets = load_budgets("[[budget]]\nmetric = 'footprint.wheel_bytes'\nmax = 1048576\ntier = 'target'\n").unwrap();
    let text = render_report(&evaluate(&budgets, &results(BenchmarkProfile::Pr, &[("footprint.wheel_bytes", 461_803.0, "bytes", 1)])));
    assert!(text.contains("1048576"));
    assert!(!text.contains("e+06"));
}

#[test]
fn test_hyperfine_metrics_from_export_mints_ids() {
    let samples = BTreeMap::from([("scale.list.n100".to_owned(), vec![0.1, 0.2, 0.3])]);
    let metrics = metrics_from_export(&samples).unwrap();
    assert_eq!(metrics.keys().map(String::as_str).collect::<Vec<_>>(), vec!["scale.list.n100.median_ms"]);
    assert_eq!(metrics["scale.list.n100.median_ms"].value, 200.0);
}

#[test]
fn test_fractional_bounds_render_compactly() {
    let budgets = load_budgets("[[budget]]\nmetric = 'ratio'\nmax = 0.5\ntier = 'target'\n").unwrap();
    let text = render_report(&evaluate(&budgets, &results(BenchmarkProfile::Pr, &[("ratio", 0.25, "x", 1)])));
    assert!(text.contains("0.25 x ≤ 0.5"), "fractional budget rendering drifted: {text}");
}

#[test]
fn test_micro_deltas_clear_the_us_floor() {
    let base = results(BenchmarkProfile::Pr, &[("micro.store.resolve.n1000.median_us", 100.0, "us", 40)]);
    let head = results(BenchmarkProfile::Pr, &[("micro.store.resolve.n1000.median_us", 300.0, "us", 40)]);
    assert_eq!(compare(&base, &head).notable().iter().map(|delta| delta.metric.as_str()).collect::<Vec<_>>(), vec!["micro.store.resolve.n1000.median_us"]);
    let near = results(BenchmarkProfile::Pr, &[("micro.store.resolve.n1000.median_us", 100.5, "us", 40)]);
    assert!(compare(&base, &near).notable().is_empty());
}

#[test]
fn test_compare_flags_incomparable_sides() {
    let base = results(BenchmarkProfile::Full, &[("x.ms", 100.0, "ms", 5)]);
    let mut head = results(BenchmarkProfile::Pr, &[("x.ms", 100.0, "ms", 5)]);
    head.meta.host.platform_key = "darwin-aarch64".to_owned();
    head.meta.python = "3.14.2".to_owned();
    let comparison = compare(&base, &head);
    assert_eq!(comparison.incomparable, vec![
        "profile: full vs pr".to_owned(),
        "platform: linux-x86_64 vs darwin-aarch64".to_owned(),
        "python: 3.13 vs 3.14".to_owned(),
    ]);
    assert!(render_markdown(&base, &head, &comparison).contains("not directly comparable"));
    let matched = compare(&base, &base);
    assert!(matched.incomparable.is_empty());
    assert!(!render_markdown(&base, &base, &matched).contains("not directly comparable"));
}

#[test]
fn test_compare_flags_harness_provenance_changes() {
    let base = results(BenchmarkProfile::Pr, &[("x.ms", 100.0, "ms", 5)]);
    let mut head = base.clone();
    head.meta.host.ci_image_version = Some("20260726.1".to_owned());
    head.meta.pyperf = "2.11.0".to_owned();
    assert_eq!(compare(&base, &head).incomparable, vec![
        "runner image: 20260719.1 vs 20260726.1".to_owned(),
        "pyperf: 2.10.0 vs 2.11.0".to_owned(),
    ]);
}

#[test]
fn test_old_schema_defaults_missing_harness_provenance() {
    let source = results(BenchmarkProfile::Pr, &[]).to_json().unwrap();
    let mut doc: Value = serde_json::from_str(&source).unwrap();
    doc["meta"]["host"].as_object_mut().unwrap().remove("ci_image_version");
    doc["meta"].as_object_mut().unwrap().remove("pyperf");
    let restored = Results::from_json(&serde_json::to_string(&doc).unwrap()).unwrap();
    assert_eq!(restored.meta.host.ci_image_version, None);
    assert_eq!(restored.meta.pyperf, "unknown");
}

#[test]
fn test_results_rejects_invalid_harness_provenance() {
    let source = results(BenchmarkProfile::Pr, &[]).to_json().unwrap();
    let mut doc: Value = serde_json::from_str(&source).unwrap();
    doc["meta"]["host"]["ci_image_version"] = json!(123);
    let error = Results::from_json(&serde_json::to_string(&doc).unwrap()).unwrap_err().to_string();
    assert!(error.contains("ci_image_version"), "invalid CI image lost field context: {error}");

    doc["meta"]["host"]["ci_image_version"] = Value::Null;
    doc["meta"]["pyperf"] = json!("");
    let error = Results::from_json(&serde_json::to_string(&doc).unwrap()).unwrap_err().to_string();
    assert!(error.contains("meta.pyperf"));
}

#[test]
fn test_budgets_reject_non_finite_bounds() {
    let error = load_budgets("[[budget]]\nmetric = 'm'\nmax = nan\ntier = 'target'\n").unwrap_err().to_string();
    assert!(error.contains("max must be finite"), "non-finite budget diagnostic drifted: {error}");
}

#[test]
fn test_results_reject_non_finite_values() {
    for bad in ["NaN", "Infinity"] {
        let source = results(BenchmarkProfile::Pr, &[("m.x", 1.0, "ms", 1)]).to_json().unwrap();
        let text = source.replace("\"value\": 1.0", &format!("\"value\": {bad}"));
        let error = Results::from_json(&text).unwrap_err().to_string();
        assert!(error.contains("finite"), "non-finite result diagnostic drifted for {bad}: {error}");
    }
}

#[test]
fn test_hyperfine_parser_rejects_malformed_entries() {
    let not_object = parse_export("{\"results\":[1]}").unwrap_err().to_string();
    assert!(not_object.contains("not an object"), "malformed Hyperfine entry lost frozen diagnostic: {not_object}");
    let non_numeric = parse_export("{\"results\":[{\"command\":\"a\",\"times\":[\"x\"],\"exit_codes\":[0]}]}").unwrap_err().to_string();
    assert!(non_numeric.contains("non-numeric time"), "malformed Hyperfine sample lost frozen diagnostic: {non_numeric}");
}

#[test]
fn test_derive_strict_pair_half_present_fails_loud() {
    let error = merge(meta(BenchmarkProfile::Pr), vec![output(SuiteKind::Startup, &[("startup.version.median_ms", 218.0, "ms", 15)])], 1.0).unwrap_err().to_string();
    assert!(error.contains("half-present"));
}

#[test]
fn test_derive_scale_grid_half_is_legitimate() {
    let merged = merge(meta(BenchmarkProfile::Pr), vec![output(SuiteKind::Scale, &[("scale.list_json.n0.median_ms", 220.0, "ms", 15)])], 1.0).unwrap();
    assert!(!merged.metrics.contains_key("scale.list_json.per_entry_us"));
}

#[test]
fn test_merge_rejects_duplicate_suite_outputs() {
    let error = merge(meta(BenchmarkProfile::Pr), vec![
        output(SuiteKind::Micro, &[("m.a", 1.0, "us", 1)]),
        output(SuiteKind::Micro, &[("m.b", 2.0, "us", 1)]),
    ], 1.0).unwrap_err().to_string();
    assert!(error.contains("duplicate suite output"));
}

#[test]
fn test_summarize_dir_rejects_corrupt_run_json() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("run.json"), "{truncated").unwrap();
    let error = summarize_directory(root.path(), None).unwrap_err().to_string();
    assert!(error.contains("not valid JSON"), "corrupt run diagnostic drifted: {error}");
}

#[test]
fn test_summarize_dir_rejects_non_finite_total() {
    let root = TempDir::new().unwrap();
    let meta = serde_json::to_value(meta(BenchmarkProfile::Pr)).unwrap();
    fs::write(root.path().join("run.json"), format!("{{\"meta\":{},\"total_duration_s\":Infinity}}", meta)).unwrap();
    let error = summarize_directory(root.path(), None).unwrap_err().to_string();
    assert!(error.contains("finite"), "non-finite run duration diagnostic drifted: {error}");
}

#[test]
fn test_skip_all_fills_both_suite_fields() {
    let output = SuiteOutput::skip_all(SuiteKind::Rss, "no resource module on Windows");
    assert_eq!(output.suite, SuiteKind::Rss);
    assert_eq!(output.skipped, vec![Skip { suite: SuiteKind::Rss, case: "all".to_owned(), reason: "no resource module on Windows".to_owned() }]);
    assert_eq!(SuiteOutput::from_json(&output.to_json().unwrap()).unwrap(), output);
}

#[test]
fn test_compare_profile_carries_compare_mode() {
    assert!(build_plan(BenchmarkProfile::Compare).iter().all(|plan| plan.compare_mode));
    assert!(build_plan(BenchmarkProfile::Pr).iter().all(|plan| !plan.compare_mode));
    assert!(build_plan(BenchmarkProfile::Full).iter().all(|plan| !plan.compare_mode));
}

#[test]
fn test_suites_needing_a_library_declare_it() {
    for profile in [BenchmarkProfile::Pr, BenchmarkProfile::Full, BenchmarkProfile::Compare] {
        for plan in build_plan(profile) {
            if matches!(plan.kind, SuiteKind::Startup | SuiteKind::Imports | SuiteKind::Footprint) {
                assert!(plan.library_sizes.contains(&0), "{profile:?}/{:?}", plan.kind);
            }
        }
    }
}

#[test]
fn test_imports_also_censuses_a_populated_library() {
    for profile in [BenchmarkProfile::Pr, BenchmarkProfile::Full, BenchmarkProfile::Compare] {
        for plan in build_plan(profile).into_iter().filter(|plan| plan.kind == SuiteKind::Imports) {
            assert!(plan.library_sizes.iter().any(|n| *n > 0), "{profile:?}");
        }
    }
}

#[test]
fn test_headline_keeps_both_census_tiers() {
    for metric in [
        "imports.list_json.n0.modules",
        "imports.list_json.n100.modules",
        "imports.list_json.n100.has_tree_sitter",
    ] {
        assert!(HEADLINE_METRICS.contains(&metric), "headline lost {metric}");
    }
}

#[test]
fn test_corrupt_manifest_reports_the_remedy() {
    for body in ["{\"n\": 3, \"seed\":", "{\"n\": 3}"] {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("manifest.json"), body).unwrap();
        let error = DatasetManifest::load(root.path()).unwrap_err().to_string();
        assert!(error.contains("is unreadable"), "corrupt manifest lost frozen remedy text: {error}");
    }
}

#[test]
fn test_manifest_records_the_probe_char() {
    let root = TempDir::new().unwrap();
    let manifest = generate(&root.path().join("ds"), 3, DEFAULT_SEED, DEFAULT_STATE_FRACTION).unwrap();
    assert_eq!(manifest.probe_char, SEARCH_PROBE_CHAR);
    assert_eq!(DatasetManifest::load(&manifest.root).unwrap().probe_char, SEARCH_PROBE_CHAR);
    let mut stale = manifest.clone();
    stale.probe_char = 'e';
    let error = check_reusable(&stale, 3).unwrap_err().to_string();
    assert!(error.contains("different inputs"));
}
