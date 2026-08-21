use std::{cell::Cell, collections::BTreeMap, path::PathBuf};

use skit_benchmarks::{GitInfo, HostInfo, Meta, Metric, Results, pipeline::ExecutionRequest};
use tempfile::TempDir;

use super::{BenchmarkProfile, Cli, Command, dispatch_with};

fn results() -> Results {
    Results {
        schema_version: 1,
        meta: Meta {
            generated_at: "2026-08-20T00:00:00Z".to_owned(),
            profile: BenchmarkProfile::Compare,
            git: GitInfo {
                commit: "abcdef1234567890".to_owned(),
                dirty: false,
                pr: None,
            },
            skit_version: "0.5.0".to_owned(),
            host: HostInfo {
                os: "Linux".to_owned(),
                kernel: "test".to_owned(),
                cpu: "Test CPU".to_owned(),
                cpu_count: 1,
                mem_total_mib: 1,
                platform_key: "linux-x86_64".to_owned(),
                ci_runner: None,
                ci_image_version: None,
            },
            python: "unknown".to_owned(),
            uv: "unknown".to_owned(),
            textual: "not-applicable".to_owned(),
            pyperf: "rust-harness-v1".to_owned(),
        },
        metrics: BTreeMap::from([(
            "imports.version.modules".to_owned(),
            Metric::single(0.0, "count"),
        )]),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    }
}

#[test]
fn run_adapter_passes_every_typed_input_and_reports_the_published_result() {
    let root = TempDir::new().unwrap();
    let out = root.path().join("out");
    let budgets = root.path().join("budgets.toml");
    let repo = root.path().join("repo");
    let measured = root.path().join("measured");
    let skit = root.path().join("skit");
    std::fs::write(
        &budgets,
        "[[budget]]\nmetric='imports.version.modules'\nmax=1\ntier='target'\n",
    )
    .unwrap();
    let called = Cell::new(false);

    let code = dispatch_with(
        Cli {
            command: Command::Run {
                profile: "compare".to_owned(),
                out: out.clone(),
                budgets: budgets.clone(),
                repo: repo.clone(),
                measured_repo: Some(measured.clone()),
                skit_binary: skit.clone(),
            },
        },
        |request: ExecutionRequest<'_>| {
            called.set(true);
            assert_eq!(request.profile, BenchmarkProfile::Compare);
            assert_eq!(request.bench_dir, out);
            assert_eq!(request.repo_root, repo);
            assert_eq!(request.measured_repo, Some(measured.as_path()));
            assert_eq!(request.skit, skit);
            assert_eq!(request.harness, PathBuf::from("/test-harness"));
            assert_eq!(request.budgets.unwrap().len(), 1);
            Ok(results())
        },
        || Ok(PathBuf::from("/test-harness")),
    )
    .unwrap();

    assert!(called.get());
    assert_eq!(code, 0);
}
