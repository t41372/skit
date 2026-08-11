use std::{
    collections::BTreeMap,
    fs, thread,
    time::{Duration, Instant},
};

use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Skip, SuiteKind, SuiteOutput,
    budget::load_budgets,
    environment::{
        bench_path, build_environment, platform_key, pull_request_number, version_from_output,
    },
    process::{ProcessError, ProcessSpec, run},
    report::{HEADLINE_METRICS, RunRecord, render_results_markdown, summarize_directory},
};
use tempfile::TempDir;

fn meta() -> Meta {
    Meta {
        generated_at: "2026-08-08T12:00:00Z".to_owned(),
        profile: BenchmarkProfile::Pr,
        git: GitInfo {
            commit: "abcdef1234567890".to_owned(),
            dirty: false,
            pr: None,
        },
        skit_version: "0.5.0".to_owned(),
        host: HostInfo {
            os: "Linux".to_owned(),
            kernel: "6.8.0".to_owned(),
            cpu: "Test CPU".to_owned(),
            cpu_count: 8,
            mem_total_mib: 16_384,
            platform_key: "linux-x86_64".to_owned(),
            ci_runner: None,
            ci_image_version: None,
        },
        python: "3.13.5".to_owned(),
        uv: "0.11.26".to_owned(),
        textual: "not-applicable".to_owned(),
        pyperf: "rust-harness-v1".to_owned(),
    }
}

fn metric(value: f64, unit: &str) -> Metric {
    Metric {
        value,
        unit: unit.to_owned(),
        n: 1,
        p95: None,
        stddev: None,
    }
}

#[test]
fn constructed_environment_is_dataset_scoped_absolute_and_not_ambient() {
    let root = TempDir::new().unwrap();
    let dataset = root.path().join("dataset");
    fs::create_dir(&dataset).unwrap();
    fs::write(dataset.join("manifest.json"), "{}").unwrap();
    let work = root.path().join("work");
    let env = build_environment(
        "/opt/skit/bin/skit",
        Some("/opt/uv/bin/uv"),
        Some("/opt/node/bin/node"),
        &work,
        &dataset,
    )
    .unwrap();
    assert_eq!(
        env["PATH"],
        "/opt/skit/bin:/opt/uv/bin:/opt/node/bin:/usr/bin:/bin"
    );
    assert_eq!(env["SKIT_LANG"], "en");
    assert_eq!(env["LC_ALL"], "C.UTF-8");
    assert!(std::path::Path::new(&env["SKIT_DATA_DIR"]).is_absolute());
    assert!(!env.contains_key("RUST_LOG"));
    assert_eq!(
        bench_path(
            "/opt/skit/bin/skit",
            Some("/opt/skit/bin/uv"),
            Some("/opt/node/bin/node")
        ),
        "/opt/skit/bin:/opt/node/bin:/usr/bin:/bin"
    );
    assert!(build_environment("skit", None, None, &work, &root.path().join("missing")).is_err());
}

#[test]
fn environment_provenance_normalizes_platform_and_pr_anchor() {
    assert_eq!(platform_key("Linux", "AMD64"), "linux-x86_64");
    assert_eq!(platform_key("Darwin", "arm64"), "darwin-aarch64");
    assert_eq!(
        pull_request_number("refs/pull/29/merge"),
        Some("29".to_owned())
    );
    assert_eq!(pull_request_number("refs/heads/main"), None);
    assert_eq!(version_from_output("Python 3.13.7\n"), "3.13.7");
    assert_eq!(version_from_output("skit 0.5.0\n"), "0.5.0");
    assert_eq!(version_from_output("0.5.0\n"), "0.5.0");
    assert_eq!(version_from_output("\n"), "unknown");
}

#[test]
fn process_runner_builds_a_complete_environment_and_enforces_timeouts() {
    let output = run(&ProcessSpec {
        argv: vec!["/usr/bin/env".to_owned()],
        cwd: "/".into(),
        env: BTreeMap::from([("ONLY_THIS".to_owned(), "yes".to_owned())]),
        timeout: Duration::from_secs(2),
        check: true,
    })
    .unwrap();
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "ONLY_THIS=yes"
    );

    let timeout = run(&ProcessSpec {
        argv: vec!["/bin/sh".to_owned(), "-c".to_owned(), "sleep 1".to_owned()],
        cwd: "/".into(),
        env: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
        timeout: Duration::from_millis(20),
        check: true,
    });
    assert!(matches!(timeout, Err(ProcessError::Timeout { .. })));
}

#[cfg(unix)]
#[test]
fn process_timeout_terminates_the_complete_descendant_tree() {
    let root = TempDir::new().unwrap();
    let marker = root.path().join("orphan-ran");
    let started = Instant::now();
    let timeout = run(&ProcessSpec {
        argv: vec![
            "/bin/sh".to_owned(),
            "-c".to_owned(),
            "(sleep 0.25; printf orphan > \"$MARKER\") & wait".to_owned(),
        ],
        cwd: root.path().to_path_buf(),
        env: BTreeMap::from([
            ("MARKER".to_owned(), marker.display().to_string()),
            ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ]),
        timeout: Duration::from_millis(20),
        check: true,
    });
    assert!(matches!(timeout, Err(ProcessError::Timeout { .. })));
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "timeout waited for a descendant that inherited a capture pipe"
    );

    thread::sleep(Duration::from_millis(400));
    assert!(
        !marker.exists(),
        "a descendant survived the benchmark process deadline"
    );
}

#[test]
fn summary_front_door_rejects_stale_outputs_and_renders_budget_context() {
    let root = TempDir::new().unwrap();
    let suites = root.path().join("suites");
    fs::create_dir(&suites).unwrap();
    let output = SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 1.25,
        metrics: BTreeMap::from([
            ("startup.version.median_ms".to_owned(), metric(18.0, "ms")),
            ("startup.python.median_ms".to_owned(), metric(7.0, "ms")),
        ]),
        skipped: vec![Skip {
            suite: SuiteKind::Startup,
            case: "optional".to_owned(),
            reason: "not installed".to_owned(),
        }],
        raw: BTreeMap::new(),
    };
    fs::write(suites.join("startup.json"), output.to_json().unwrap()).unwrap();
    let run = RunRecord {
        meta: meta(),
        total_duration_s: 2.0,
        suites: vec![SuiteKind::Startup],
    };
    fs::write(
        root.path().join("run.json"),
        serde_json::to_string_pretty(&run).unwrap(),
    )
    .unwrap();
    let budgets =
        load_budgets("[[budget]]\nmetric='startup.version.over_python_ms'\nmax=75\ntier='target'")
            .unwrap();
    let results = summarize_directory(root.path(), Some(&budgets)).unwrap();
    assert_eq!(
        results.metrics["startup.version.over_python_ms"].value,
        11.0
    );
    let markdown = fs::read_to_string(root.path().join("results.md")).unwrap();
    assert!(markdown.contains("startup.version.median_ms"));
    assert!(markdown.contains("| 18 ms | — | 1 |"));
    assert!(markdown.contains("profile **pr** · 2026-08-08T12:00:00Z"));
    assert!(markdown.contains("Skipped (1)"));
    assert!(markdown.contains("Budgets"));
    assert_eq!(
        markdown,
        render_results_markdown(
            &results,
            Some(&skit_benchmarks::budget::evaluate(&budgets, &results))
        )
    );

    fs::write(suites.join("scale.json"), output.to_json().unwrap()).unwrap();
    assert!(summarize_directory(root.path(), None).is_err());
    assert!(HEADLINE_METRICS.contains(&"imports.list_json.n100.has_tree_sitter"));
}

#[test]
fn summary_names_missing_incomplete_and_legacy_run_directories() {
    let root = TempDir::new().unwrap();
    let missing = summarize_directory(root.path(), None)
        .unwrap_err()
        .to_string();
    assert!(missing.contains("no run.json"));

    fs::write(
        root.path().join("run.json"),
        serde_json::json!({
            "meta": meta(),
            "total_duration_s": -1.0,
            "suites": []
        })
        .to_string(),
    )
    .unwrap();
    assert!(
        summarize_directory(root.path(), None)
            .unwrap_err()
            .to_string()
            .contains("finite and non-negative")
    );

    let legacy = RunRecord {
        meta: meta(),
        total_duration_s: 1.0,
        suites: Vec::new(),
    };
    fs::write(
        root.path().join("run.json"),
        serde_json::to_string(&legacy).unwrap(),
    )
    .unwrap();
    fs::create_dir(root.path().join("suites")).unwrap();
    assert!(
        summarize_directory(root.path(), None)
            .unwrap_err()
            .to_string()
            .contains("no suite outputs")
    );

    let output = SuiteOutput {
        suite: SuiteKind::Imports,
        duration_seconds: 0.25,
        metrics: BTreeMap::from([(
            "imports.version.modules".to_owned(),
            Metric {
                value: 0.0,
                unit: "count".to_owned(),
                n: 2,
                p95: Some(0.0),
                stddev: Some(0.0),
            },
        )]),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    fs::write(
        root.path().join("suites/imports.json"),
        output.to_json().unwrap(),
    )
    .unwrap();
    let mut results = summarize_directory(root.path(), None).unwrap();
    results.meta.git.dirty = true;
    results.meta.host.ci_image_version = Some("image-1".to_owned());
    let markdown = render_results_markdown(&results, None);
    assert!(markdown.contains("(dirty)"));
    assert!(markdown.contains("image image-1"));
    assert!(markdown.contains("No skipped cases."));
    assert!(markdown.contains("| 0 count | 0 | 2 |"));
}

#[cfg(unix)]
#[test]
fn summary_front_door_refuses_symlinked_input_artifacts() {
    use std::os::unix::fs::symlink;

    let root = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let run = RunRecord {
        meta: meta(),
        total_duration_s: 1.0,
        suites: vec![SuiteKind::Startup],
    };
    fs::write(
        root.path().join("run.json"),
        serde_json::to_string(&run).unwrap(),
    )
    .unwrap();
    fs::create_dir(outside.path().join("suites")).unwrap();
    symlink(outside.path().join("suites"), root.path().join("suites")).unwrap();
    assert!(summarize_directory(root.path(), None).is_err());

    fs::remove_file(root.path().join("suites")).unwrap();
    fs::create_dir(root.path().join("suites")).unwrap();
    let output = SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 0.1,
        metrics: BTreeMap::new(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    let outside_suite = outside.path().join("startup.json");
    fs::write(&outside_suite, output.to_json().unwrap()).unwrap();
    symlink(&outside_suite, root.path().join("suites/startup.json")).unwrap();
    assert!(summarize_directory(root.path(), None).is_err());
}
