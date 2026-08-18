#![cfg(unix)]
//! Frozen suite-output evidence contracts from `tests/test_benchmarks_tooling.py`.

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use skit_benchmarks::{
    Metric, SuiteKind, SuitePlan,
    dataset::{DEFAULT_SEED, DEFAULT_STATE_FRACTION, generate},
    runner::RunContext,
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
                    isize::try_from(size).expect("benchmark dataset size fits isize"),
                    DEFAULT_SEED,
                    DEFAULT_STATE_FRACTION,
                )
                .unwrap(),
            );
        }

        let tools = root.path().join("tools");
        fs::create_dir_all(&tools).unwrap();
        let skit = executable(
            tools.join("skit"),
            "#!/bin/sh\nprintf 'skit 0.5.0\\n'\n",
        );
        let counter = root.path().join("tui-count");
        let harness_source = r#"#!/bin/sh
set -eu
COUNT='__COUNT__'
if [ "${2-}" = tui ]; then
  n=0
  [ ! -f "$COUNT" ] || n=$(cat "$COUNT")
  n=$((n + 1))
  printf '%s' "$n" > "$COUNT"

  entries=0
  previous=
  for argument do
    if [ "$previous" = --entries ]; then
      entries=$argument
      break
    fi
    previous=$argument
  done
  if [ "$entries" -eq 0 ]; then
    selected=null
  else
    selected=$n
  fi
  search=$((n + 10))
  rss=$((n * 100))
  payload="{\"first_idle_ms\":$n,\"select_ms\":$selected,\"search_ms\":$search,\"status_text\":\"VmHWM: $rss kB\\n\"}"
  printf '%s\n' "$payload"
else
  printf '1.25\n'
fi
"#
        .replace("__COUNT__", &counter.display().to_string());
        let harness = executable(tools.join("harness"), &harness_source);
        let uv = executable(
            tools.join("uv"),
            r#"#!/bin/sh
set -eu
case "$1" in
  build)
    shift
    out=
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --out-dir)
          out=$2
          shift 2
          ;;
        *)
          shift
          ;;
      esac
    done
    [ -n "$out" ]
    mkdir -p "$out"
    printf wheel > "$out/skit_cli-0.5.0-py3-none-any.whl"
    printf source > "$out/skit_cli-0.5.0.tar.gz"
    ;;
  *)
    exit 2
    ;;
esac
"#,
        );

        let workdir = root.path().join("work");
        let out_dir = root.path().join("out");
        fs::create_dir_all(&workdir).unwrap();
        fs::create_dir_all(&out_dir).unwrap();
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
        let context = RunContext {
            repo_root,
            out_dir,
            workdir,
            datasets,
            skit,
            harness,
            python: None,
            uv: Some(uv),
            bash: Some(PathBuf::from("/bin/sh")),
            node: None,
            hyperfine: None,
            strace: None,
            cargo: None,
            rustc: None,
        };
        Self {
            _root: root,
            context,
        }
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

fn f64_array(value: &serde_json::Value) -> Vec<f64> {
    value
        .as_array()
        .expect("raw sample series is an array")
        .iter()
        .map(|value| value.as_f64().expect("raw sample is numeric"))
        .collect()
}

fn sample_stddev(values: &[f64]) -> f64 {
    assert!(values.len() >= 2);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let squared = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>();
    (squared / (values.len() - 1) as f64).sqrt()
}

fn assert_close(actual: f64, expected: f64) {
    let tolerance = expected.abs().max(1.0) * 1.0e-12;
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual}"
    );
}

fn assert_three_sample_metric(metric: &Metric, samples: &[f64], unit: &str) {
    assert_eq!(samples.len(), 3);
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    assert_eq!(metric.unit, unit);
    assert_eq!(metric.n, 3);
    assert_close(metric.value, sorted[1]);
    assert_close(metric.p95.expect("three-sample metric keeps p95"), sorted[2]);
    assert_close(
        metric
            .stddev
            .expect("three-sample metric keeps sample standard deviation"),
        sample_stddev(samples),
    );
}

#[test]
fn test_the_library_footprint_metrics_divide_into_each_other() {
    let fixture = Fixture::new(&[0, 3]);
    let output = suites::run(
        &fixture.context,
        &plan(SuiteKind::Footprint, &[0, 3], 1),
    )
    .unwrap();
    assert!(output.skipped.is_empty());

    let store = &output.metrics["footprint.library_bytes.n3"];
    let state = &output.metrics["footprint.library_state_bytes.n3"];
    let total = &output.metrics["footprint.library_total_bytes.n3"];
    let per_entry = &output.metrics["footprint.library_bytes_per_entry.n3"];
    assert_eq!(store.unit, "bytes");
    assert_eq!(state.unit, "bytes");
    assert_eq!(total.unit, "bytes");
    assert_eq!(per_entry.unit, "bytes");
    assert_eq!(store.n, 1);
    assert_eq!(state.n, 1);
    assert_eq!(total.n, 1);
    assert_eq!(per_entry.n, 1);
    assert_close(total.value, store.value + state.value);
    assert_close(per_entry.value, total.value / 3.0);
}

#[test]
fn test_rss_keeps_samples_and_full_statistics() {
    assert!(
        Path::new("/usr/bin/time").is_file(),
        "benchmark RSS contract requires /usr/bin/time"
    );
    let fixture = Fixture::new(&[0]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Rss, &[0], 3)).unwrap();
    let samples = f64_array(&output.raw["rss.version"]["samples_kib"]);
    assert_three_sample_metric(
        &output.metrics["rss.version.peak_kib"],
        &samples,
        "KiB",
    );
}

#[cfg(target_os = "linux")]
#[test]
fn test_tui_keeps_import_and_rss_samples() {
    let fixture = Fixture::new(&[0]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Tui, &[0], 3)).unwrap();
    assert_eq!(
        output.raw["n0"],
        serde_json::json!({
            "first_idle_ms": [1.0, 2.0, 3.0],
            "select_ms": [],
            "search_ms": [11.0, 12.0, 13.0],
            "rss_kib": [100.0, 200.0, 300.0],
            "import_ms": [0.0, 0.0, 0.0],
        })
    );
    assert!(!output.metrics.contains_key("tui.select.n0.median_ms"));
    assert_three_sample_metric(
        &output.metrics["tui.rss.n0.peak_kib"],
        &[100.0, 200.0, 300.0],
        "KiB",
    );

    let imports = &output.metrics["tui.import.median_ms"];
    assert_eq!(imports.unit, "ms");
    assert_eq!(imports.n, 3);
    assert_eq!(imports.value, 0.0);
    assert_eq!(imports.p95, Some(0.0));
    assert_eq!(imports.stddev, Some(0.0));
    assert_eq!(
        output.raw["native_import_note"].as_str(),
        Some("not applicable: Rust binary has no Python import path")
    );
}

#[test]
fn test_tui_records_the_selection_span_when_the_probe_measured_one() {
    let fixture = Fixture::new(&[2]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Tui, &[2], 3)).unwrap();
    let samples = f64_array(&output.raw["n2"]["select_ms"]);
    assert_eq!(samples, vec![1.0, 2.0, 3.0]);
    assert_three_sample_metric(
        &output.metrics["tui.select.n2.median_ms"],
        &samples,
        "ms",
    );
}

#[test]
fn test_cold_parse_keeps_raw_samples() {
    let fixture = Fixture::new(&[0, 3]);
    let output = suites::run(&fixture.context, &plan(SuiteKind::Micro, &[0, 3], 1)).unwrap();
    let samples = f64_array(&output.raw["analyze_cold"]["python"]["samples_ms"]);
    assert_eq!(samples, vec![1.25; 5]);

    let metric = &output.metrics["micro.analyze_cold.python.median_ms"];
    assert_eq!(metric.unit, "ms");
    assert_eq!(metric.n, 5);
    assert_eq!(metric.value, 1.25);
    assert_eq!(metric.p95, Some(1.25));
    assert_eq!(metric.stddev, Some(0.0));
}
