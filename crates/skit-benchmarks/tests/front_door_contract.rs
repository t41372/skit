use std::{collections::BTreeMap, fs, process::Command};

use skit_benchmarks::{BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results};
use tempfile::TempDir;

fn local_result() -> Results {
    let metrics = [
        ("footprint.wheel_bytes", 1.0, "bytes"),
        ("binary.release_bytes", 1.0, "bytes"),
        ("repository.python_implementation_files", 0.0, "count"),
        ("imports.version.modules", 0.0, "count"),
        ("imports.list_json.n0.modules", 0.0, "count"),
        ("imports.list_json.n100.modules", 0.0, "count"),
        ("imports.list_json.n100.has_tree_sitter", 0.0, "bool"),
        ("imports.version.has_typer", 0.0, "bool"),
        ("imports.version.has_rich", 0.0, "bool"),
        ("imports.version.has_textual", 0.0, "bool"),
        ("imports.version.has_tree_sitter", 0.0, "bool"),
    ]
    .into_iter()
    .map(|(id, value, unit)| (id.to_owned(), Metric::single(value, unit)))
    .collect();
    Results {
        schema_version: 1,
        meta: Meta {
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
            python: "3.13.7".to_owned(),
            uv: "0.12.3".to_owned(),
            textual: "not-applicable".to_owned(),
            pyperf: "rust-harness-v1".to_owned(),
        },
        metrics,
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn check_and_compare_commands_use_validated_artifacts_and_stable_exit_codes() {
    let root = TempDir::new().unwrap();
    let results = root.path().join("results.json");
    fs::write(&results, local_result().to_json().unwrap()).unwrap();
    let budgets = concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/budgets.toml");
    let binary = env!("CARGO_BIN_EXE_skit-bench");

    let check = Command::new(binary)
        .args([
            "check",
            results.to_str().unwrap(),
            "--budgets",
            budgets,
            "--require-enforced",
        ])
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(String::from_utf8_lossy(&check.stdout).contains("enforced:"));

    let compare = Command::new(binary)
        .args([
            "compare",
            results.to_str().unwrap(),
            results.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(compare.status.success());
    assert!(String::from_utf8_lossy(&compare.stdout).contains("Notable (none)"));
}

#[test]
fn compare_workflow_keeps_latest_python_main_as_a_schema_v1_side() {
    let workflow = include_str!("../../../.github/workflows/benchmark-compare.yml");
    for required in [
        "$side/pyproject.toml",
        "$side/crates/skit-benchmarks/Cargo.toml",
        "side-base/crates/skit-benchmarks/Cargo.toml",
        "side-head/crates/skit-benchmarks/Cargo.toml",
        "working-directory: side-base",
        "working-directory: side-head",
        ".venv/bin/python -m benchmarks run",
        "target/release/skit-bench compare",
    ] {
        assert!(
            workflow.contains(required),
            "missing compare lane {required}"
        );
    }
    assert_eq!(
        workflow
            .matches(".venv/bin/python -m benchmarks run")
            .count(),
        2
    );
    assert!(workflow.contains("schema-v1 Python benchmark harness"));
}
