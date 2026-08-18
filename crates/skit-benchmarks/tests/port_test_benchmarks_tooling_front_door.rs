//! Frozen benchmark front-door contracts from `tests/test_benchmarks_tooling.py`.
//!
//! These owners execute the real `skit-bench` binary and inspect real artifacts/stdout/stderr.

use std::{collections::BTreeMap, fs, process::Command};

use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results,
    budget::load_budgets,
    dataset::DatasetManifest,
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

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_skit-bench")
}

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

fn results(metrics: &[(&str, f64, &str)]) -> Results {
    Results {
        schema_version: 1,
        meta: meta(),
        metrics: metrics
            .iter()
            .map(|(name, value, unit)| {
                (
                    (*name).to_owned(),
                    Metric {
                        value: *value,
                        unit: (*unit).to_owned(),
                        n: 1,
                        p95: None,
                        stddev: None,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>(),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

fn write_results(root: &std::path::Path, name: &str, value: Results) -> std::path::PathBuf {
    let path = root.join(name);
    fs::write(&path, value.to_json().unwrap()).unwrap();
    path
}

fn write_budgets(root: &std::path::Path, text: &str) -> std::path::PathBuf {
    let path = root.join("budgets.toml");
    fs::write(&path, text).unwrap();
    path
}

#[test]
fn test_datasets_command() {
    let root = TempDir::new().unwrap();
    let out = root.path().join("dataset");
    let output = Command::new(binary())
        .args(["datasets", "--n", "3", "--out"])
        .arg(&out)
        .output()
        .expect("run real skit-bench datasets");
    assert!(
        output.status.success(),
        "datasets command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(DatasetManifest::load(&out).unwrap().n, 3);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("generated 3 entries"),
        "datasets command lost its frozen success message: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn test_check_exit_codes() {
    let root = TempDir::new().unwrap();
    let budgets = write_budgets(root.path(), ENFORCED_ROW);
    let good = write_results(
        root.path(),
        "good.json",
        results(&[("imports.version.modules", 300.0, "count")]),
    );
    let bad = write_results(
        root.path(),
        "bad.json",
        results(&[("imports.version.modules", 999.0, "count")]),
    );

    let good_status = Command::new(binary())
        .arg("check")
        .arg(&good)
        .arg("--budgets")
        .arg(&budgets)
        .status()
        .expect("run passing check");
    assert_eq!(good_status.code(), Some(0));

    let bad_status = Command::new(binary())
        .arg("check")
        .arg(&bad)
        .arg("--budgets")
        .arg(&budgets)
        .status()
        .expect("run violating check");
    assert_eq!(bad_status.code(), Some(1));
}

#[test]
fn test_check_require_enforced() {
    let root = TempDir::new().unwrap();
    let budgets = write_budgets(root.path(), &format!("{ENFORCED_ROW}profiles = ['full']\n"));
    let value = write_results(
        root.path(),
        "r.json",
        results(&[("imports.version.modules", 300.0, "count")]),
    );

    let ordinary = Command::new(binary())
        .arg("check")
        .arg(&value)
        .arg("--budgets")
        .arg(&budgets)
        .status()
        .unwrap();
    assert_eq!(ordinary.code(), Some(0));

    let required = Command::new(binary())
        .arg("check")
        .arg(&value)
        .arg("--budgets")
        .arg(&budgets)
        .arg("--require-enforced")
        .status()
        .unwrap();
    assert_eq!(required.code(), Some(1));
}

#[test]
fn test_check_propose_prints_toml() {
    let root = TempDir::new().unwrap();
    let budgets = write_budgets(root.path(), ENFORCED_ROW);
    let value = write_results(
        root.path(),
        "r.json",
        results(&[("imports.version.modules", 291.0, "count")]),
    );
    let output = Command::new(binary())
        .arg("check")
        .arg(&value)
        .arg("--budgets")
        .arg(&budgets)
        .arg("--propose")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "proposal command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(load_budgets(&stdout).unwrap()[0].max_value, 321.0);
}

#[test]
fn test_cli_formats_os_errors() {
    let root = TempDir::new().unwrap();
    let budgets = write_budgets(root.path(), ENFORCED_ROW);
    let missing = root.path().join("missing.json");
    let output = Command::new(binary())
        .arg("check")
        .arg(&missing)
        .arg("--budgets")
        .arg(&budgets)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("benchmarks: "),
        "frozen benchmark CLI error prefix changed: {stderr}"
    );
}

#[test]
fn test_compare_command() {
    let root = TempDir::new().unwrap();
    let base = write_results(root.path(), "a.json", results(&[("x.ms", 100.0, "ms")]));
    let head = write_results(root.path(), "b.json", results(&[("x.ms", 300.0, "ms")]));
    let output = Command::new(binary())
        .arg("compare")
        .arg(&base)
        .arg(&head)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "compare command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("Notable (1)"),
        "compare command lost the notable-delta report: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
