#![cfg(unix)]
//! Frozen suite-output evidence contracts from `tests/test_benchmarks_tooling.py`.

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use skit_benchmarks::{
    SuiteKind, SuitePlan,
    dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, generate},
    runner::RunContext,
    stats::{median, nearest_rank_p95, sample_stddev},
    suites,
};
use tempfile::TempDir;

struct Fixture {
    _root: TempDir,
    context: RunContext,
}

impl Fixture {
    fn new(sizes: &[usize]) -> Self {
        let root = TempDir::new().unwrap();
        let mut datasets = BTreeMap::new();
        for &size in sizes {
            datasets.insert(
                size,
                generate(
                    &root.path().join(format!("dataset-{size}")),
                    size,
                    DEFAULT_SEED,
                    DEFAULT_STATE_FRACTION,
                )
                .unwrap(),
            );
        }
        let skit = executable(root.path().join("skit"), "#!/bin/sh\nexit 0\n");
        let counter = root.path().join("tui-count");
        let harness = executable(
            root.path().join("harness"),
            &format!(
                "#!/bin/sh\nset -eu\nCOUNT='{}'\nif [ \"${{2-}}\" = tui ]; then\n  n=0\n  [ ! -f \"$COUNT\" ] || n=$(cat \"$COUNT\")\n  n=$((n + 1))\n  printf '%s' \"$n\" > \"$COUNT\"\n  rss=$((n * 100))\n  printf '{{\"first_idle_ms\":%s,\"select_ms\":%s,\"search_ms\":%s,\"status_text\":\"VmHWM: %s kB\\n\"}}\\n' \"$n\" \"$n\" \"$n\" \"$rss\"\nelse\n  printf '1.25\\n'\nfi\n",
                counter.display()
            ),
        );
        let workdir = root.path().join("work");
        let out_dir = root.path().join("out");
        fs::create_dir_all(&workdir).unwrap();
        fs::create_dir_all(&out_dir).unwrap();
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf();
        let context = RunContext {
            repo_root,
            out_dir,
            workdir,
            datasets,
            skit,
            harness,
            python: None,
            uv: None,
            bash: Some(PathBuf::from("/bin/sh")),
            node: None,
            hyperfine: None,
            strace: None,
            cargo: None,
            rustc: None,
        };
        Self { _root: root, context }
    }
}

fn plan(kind: SuiteKind, sizes: &[usize], samples: usize) -> SuitePlan {
    SuitePlan {
        kind,
        library_sizes: sizes.to_vec(),
        warmup: 0,
        minimum_runs: 1,
        samples,
        fast: true,
        measure_closure: false,
        run_javascript_lane: false,
        run_doctor: false,
        compare_mode: false,
    }
}

fn executable(path: PathBuf, source: &str) -> PathBuf {
    fs::write(&path, source).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    path
}

fn raw_f64(output: &skit_benchmarks::SuiteOutput, key: &str) -> Vec<f64> {
    output.raw[key]
        .as_array()
        .unwrap_or_else(|| panic!("raw {key} is not an array"))
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect()
}

#[test]
fn test_the_library_footprint_metrics_divide_into_each_other() {
    let fixture = Fixture::new(&[0, 3]);
    let output = suites::run(
        &fixture.context,
        &plan(SuiteKind::Footprint, &[0, 3], 1),
    )
    .unwrap();
    assert_eq!(output.metrics["footprint.library.n3.entries"].value, 3.0);
    let total = output.metrics["footprint.library.n3.bytes"].value;
    let per_entry = output.metrics["footprint.library.n3.bytes_per_entry"].value;
    assert_eq!(per_entry, total / 3.0);
    assert_eq!(output.raw["footprint.library.n3.entries"], 3);
    assert_eq!(output.raw["footprint.library.n3.bytes"], total);
}

#[test]
fn test_rss_keeps_samples_and_full_statistics() {
    assert!(Path::new("/usr/bin/time").is_file(), "benchmark RSS contract requires GNU time");
    let fixture = Fixture::new(&[0]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Rss, &[0], 3)).unwrap();
    let samples = raw_f64(&output, "rss.version.samples_kib");
    assert_eq!(samples.len(), 3);
    let metric = &output.metrics["rss.version.peak_kib"];
    assert_eq!(metric.n, 3);
    assert_eq!(metric.value, median(&samples).unwrap());
    assert_eq!(metric.p95, Some(nearest_rank_p95(&samples).unwrap()));
    assert_eq!(metric.stddev, Some(sample_stddev(&samples).unwrap()));
}

#[test]
fn test_tui_keeps_import_and_rss_samples() {
    let fixture = Fixture::new(&[2]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Tui, &[2], 3)).unwrap();
    assert_eq!(raw_f64(&output, "tui.n2.imports"), vec![0.0, 0.0, 0.0]);
    assert_eq!(raw_f64(&output, "tui.n2.rss_samples_kib"), vec![100.0, 200.0, 300.0]);
    let imports = &output.metrics["tui.n2.imports"];
    assert_eq!(imports.value, 0.0);
    assert_eq!(imports.n, 3);
    let rss = &output.metrics["tui.n2.rss_kib"];
    assert_eq!(rss.value, 200.0);
    assert_eq!(rss.p95, Some(300.0));
    assert_eq!(rss.n, 3);
    assert!(
        output.raw["tui.n2.imports_note"]
            .as_str()
            .is_some_and(|note| note.contains("no Python import graph"))
    );
}

#[test]
fn test_tui_records_the_selection_span_when_the_probe_measured_one() {
    let fixture = Fixture::new(&[2]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Tui, &[2], 3)).unwrap();
    assert_eq!(raw_f64(&output, "tui.n2.select_ms"), vec![1.0, 2.0, 3.0]);
    let metric = &output.metrics["tui.n2.select_ms"];
    assert_eq!(metric.value, 2.0);
    assert_eq!(metric.p95, Some(3.0));
    assert_eq!(metric.n, 3);
}

#[test]
fn test_cold_parse_keeps_raw_samples() {
    let fixture = Fixture::new(&[0, 3]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Micro, &[0, 3], 1)).unwrap();
    let samples = raw_f64(&output, "analyze_cold.python.samples_ms");
    assert_eq!(samples, vec![1.25; 5]);
    let metric = &output.metrics["analyze_cold.python.median_ms"];
    assert_eq!(metric.value, 1.25);
    assert_eq!(metric.n, 5);
}
