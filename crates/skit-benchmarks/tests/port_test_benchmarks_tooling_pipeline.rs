//! Frozen benchmark pipeline contracts from `tests/test_benchmarks_tooling.py`.

use std::{collections::{BTreeMap, BTreeSet}, fs};

use serde_json::json;
use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results, Skip, SuiteKind, SuiteOutput,
    build_plan, dataset_sizes, merge,
    budget::{evaluate, load_budgets},
    report::{render_results_markdown, summarize_directory},
};
use tempfile::TempDir;

const ENFORCED_ROW: &str = r#"
[[budget]]
metric = "imports.version.modules"
max = 400
tier = "enforced"
ratchet = true
context = { python = "3.13", commit = "abc", date = "2026-07-20" }
"#;

fn meta() -> Meta {
    Meta {
        generated_at: "2026-07-20T12:00:00+00:00".to_owned(),
        profile: BenchmarkProfile::Pr,
        git: GitInfo {
            commit: "abcdef1234567890".to_owned(),
            dirty: false,
            pr: None,
        },
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
    Metric {
        value,
        unit: unit.to_owned(),
        n,
        p95: None,
        stddev: None,
    }
}

fn output(suite: SuiteKind, metrics: &[(&str, f64, &str, usize)]) -> SuiteOutput {
    SuiteOutput {
        suite,
        duration_seconds: 0.0,
        metrics: metrics
            .iter()
            .map(|(name, value, unit, n)| ((*name).to_owned(), metric(*value, unit, *n)))
            .collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

fn results(metrics: &[(&str, f64, &str, usize)]) -> Results {
    Results {
        schema_version: 1,
        meta: meta(),
        metrics: metrics
            .iter()
            .map(|(name, value, unit, n)| ((*name).to_owned(), metric(*value, unit, *n)))
            .collect(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn test_profiles() {
    let pr = build_plan(BenchmarkProfile::Pr);
    let full = build_plan(BenchmarkProfile::Full);
    let compare_plan = build_plan(BenchmarkProfile::Compare);
    assert_eq!(
        pr.iter().map(|plan| plan.kind).collect::<BTreeSet<_>>(),
        BTreeSet::from([
            SuiteKind::Imports,
            SuiteKind::Footprint,
            SuiteKind::Rss,
            SuiteKind::Startup,
            SuiteKind::Scale,
            SuiteKind::RunOverhead,
            SuiteKind::Micro,
            SuiteKind::Tui,
        ])
    );
    assert!(full.iter().any(|plan| plan.kind == SuiteKind::Syscalls));
    assert!(!compare_plan.iter().any(|plan| plan.kind == SuiteKind::Footprint));
    assert!(
        !pr.iter()
            .find(|plan| plan.kind == SuiteKind::RunOverhead)
            .unwrap()
            .run_javascript_lane
    );
    assert!(
        full.iter()
            .find(|plan| plan.kind == SuiteKind::RunOverhead)
            .unwrap()
            .run_javascript_lane
    );
    assert_eq!(
        full.iter()
            .find(|plan| plan.kind == SuiteKind::Scale)
            .unwrap()
            .library_sizes,
        vec![0, 10, 100, 1_000]
    );
    let error = "nightly".parse::<BenchmarkProfile>().unwrap_err().to_string();
    assert!(error.contains("unknown profile"));
}

#[test]
fn test_dataset_ns() {
    assert_eq!(dataset_sizes(&build_plan(BenchmarkProfile::Pr)), vec![0, 100, 1_000]);
    assert_eq!(
        dataset_sizes(&build_plan(BenchmarkProfile::Full)),
        vec![0, 10, 100, 1_000]
    );
}

#[test]
fn test_merge_and_derive() {
    let mut startup = output(
        SuiteKind::Startup,
        &[
            ("startup.python.median_ms", 35.0, "ms", 15),
            ("startup.version.median_ms", 218.0, "ms", 15),
        ],
    );
    startup.duration_seconds = 10.0;
    let mut scale = output(
        SuiteKind::Scale,
        &[
            ("scale.list_json.n0.median_ms", 220.0, "ms", 15),
            ("scale.list_json.n1000.median_ms", 720.0, "ms", 15),
        ],
    );
    scale.skipped.push(Skip {
        suite: SuiteKind::Scale,
        case: "x".to_owned(),
        reason: "y".to_owned(),
    });

    let merged = merge(meta(), vec![startup.clone(), scale], 100.0).unwrap();
    assert_eq!(merged.metrics["startup.version.over_python_ms"].value, 183.0);
    assert_eq!(merged.metrics["scale.list_json.per_entry_us"].value, 500.0);
    assert_eq!(merged.metrics["pipeline.skipped_count"].value, 1.0);
    assert_eq!(merged.metrics["pipeline.duration_s"].value, 100.0);
    assert_eq!(merged.metrics["pipeline.suite.startup.duration_s"].value, 10.0);

    let partial = merge(meta(), vec![startup], 1.0).unwrap();
    assert!(!partial.metrics.contains_key("scale.list_json.per_entry_us"));
}

#[test]
fn test_merge_rejects_duplicate_ids() {
    let first = output(SuiteKind::Startup, &[("m.x", 1.0, "ms", 1)]);
    let second = output(SuiteKind::Scale, &[("m.x", 2.0, "ms", 1)]);
    let error = merge(meta(), vec![first, second], 1.0)
        .unwrap_err()
        .to_string();
    assert!(error.contains("duplicate metric id"));
}

#[test]
fn test_merge_rejects_reserved_pipeline_ids() {
    for metric_id in [
        "pipeline.duration_s",
        "pipeline.skipped_count",
        "pipeline.suite.demo.duration_s",
    ] {
        let value = output(SuiteKind::Startup, &[(metric_id, 999.0, "s", 1)]);
        let error = merge(meta(), vec![value], 1.0).unwrap_err().to_string();
        assert!(
            error.contains("reserved pipeline metric id"),
            "reserved id {metric_id:?} was not rejected loudly: {error}"
        );
    }
}

#[test]
fn test_merge_rejects_derived_collision() {
    let clash = output(
        SuiteKind::Startup,
        &[
            ("startup.python.median_ms", 35.0, "ms", 1),
            ("startup.version.median_ms", 218.0, "ms", 1),
            ("startup.version.over_python_ms", 1.0, "ms", 1),
        ],
    );
    let error = merge(meta(), vec![clash], 1.0).unwrap_err().to_string();
    assert!(
        error.contains("already present"),
        "frozen derived-collision diagnostic changed: {error}"
    );
}

#[test]
fn test_render_markdown() {
    let mut value = results(&[
        ("startup.version.median_ms", 218.0, "ms", 15),
        ("imports.version.modules", 291.0, "count", 1),
    ]);
    value.metrics.get_mut("startup.version.median_ms").unwrap().p95 = Some(230.0);
    value.skipped.push(Skip {
        suite: SuiteKind::RunOverhead,
        case: "js".to_owned(),
        reason: "node not found".to_owned(),
    });
    let budget_report = evaluate(&load_budgets(ENFORCED_ROW).unwrap(), &value);
    let text = render_results_markdown(&value, Some(&budget_report));
    assert!(text.contains("`startup.version.median_ms` | 218 ms | 230 | 15"));
    assert!(text.contains("run_overhead/js"));
    assert!(text.contains("### Budgets"));
    assert!(render_results_markdown(&results(&[]), None).contains("No skipped cases."));
}

#[test]
fn test_summarize_dir() {
    let bench = TempDir::new().unwrap();
    fs::create_dir(bench.path().join("suites")).unwrap();
    fs::write(
        bench.path().join("run.json"),
        serde_json::to_string(&json!({"meta": meta(), "total_duration_s": 12.5})).unwrap(),
    )
    .unwrap();
    let startup = output(
        SuiteKind::Startup,
        &[
            ("startup.python.median_ms", 35.0, "ms", 15),
            ("startup.version.median_ms", 218.0, "ms", 15),
        ],
    );
    fs::write(
        bench.path().join("suites/startup.json"),
        startup.to_json().unwrap(),
    )
    .unwrap();

    let summarized = summarize_directory(bench.path(), None).unwrap();
    assert!(bench.path().join("results.json").exists());
    assert!(bench.path().join("results.md").exists());
    assert_eq!(summarized.metrics["pipeline.duration_s"].value, 12.5);
    let again = Results::from_json(&fs::read_to_string(bench.path().join("results.json")).unwrap())
        .unwrap();
    assert_eq!(again, summarized);
}

#[test]
fn test_summarize_dir_failures() {
    let bench = TempDir::new().unwrap();
    let error = summarize_directory(bench.path(), None).unwrap_err().to_string();
    assert!(error.contains("no run.json"));

    fs::write(
        bench.path().join("run.json"),
        serde_json::to_string(&json!({"meta": meta(), "total_duration_s": "slow"})).unwrap(),
    )
    .unwrap();
    let error = summarize_directory(bench.path(), None).unwrap_err().to_string();
    assert!(
        error.contains("total_duration_s"),
        "bad run duration lost the frozen field diagnostic: {error}"
    );

    fs::write(
        bench.path().join("run.json"),
        serde_json::to_string(&json!({"meta": meta(), "total_duration_s": 1.0})).unwrap(),
    )
    .unwrap();
    fs::create_dir(bench.path().join("suites")).unwrap();
    let error = summarize_directory(bench.path(), None).unwrap_err().to_string();
    assert!(error.contains("no suite outputs"));
}
