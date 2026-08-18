//! Frozen repository contract-sync owners from `tests/test_benchmarks_tooling.py`.

use std::{fs, path::Path};

use skit_benchmarks::budget::{load_budgets, render_budgets};

fn repo() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-benchmarks lives under <repo>/crates/skit-benchmarks")
}

#[test]
fn test_budgets_file_is_canonical() {
    let path = repo().join("benchmarks/budgets.toml");
    let text = fs::read_to_string(&path).expect("read benchmark budget contract");
    assert_eq!(
        render_budgets(&load_budgets(&text).unwrap()).unwrap(),
        text,
        "the committed budget contract is not byte-canonical under its own renderer"
    );
}

#[test]
fn test_analyzer_source_filenames_share_one_registry() {
    let source = fs::read_to_string(repo().join("crates/skit-benchmarks/src/suites/micro.rs"))
        .expect("read native microbenchmark source");
    assert!(
        source.contains("sources::{LANGUAGES, extension, generate, generate_broken}"),
        "micro suite stopped importing the shared language/extension registry"
    );
    assert!(
        source.contains("format!(\"{language}_{lines}.{}\", extension(language))"),
        "normal analyzer filenames stopped using the shared extension registry"
    );
    assert!(
        source.contains("format!(\"{language}_{BROKEN_LINES}_broken.{}\", extension(language))"),
        "broken analyzer filenames stopped using the shared extension registry"
    );
}

#[test]
fn test_workflows_install_hyperfine_via_the_action() {
    for workflow in [
        "benchmark.yml",
        "benchmark-nightly.yml",
        "benchmark-compare.yml",
    ] {
        let text = fs::read_to_string(repo().join(".github/workflows").join(workflow))
            .unwrap_or_else(|error| panic!("could not read {workflow}: {error}"));
        assert!(
            text.contains("uses: ./.github/actions/install-hyperfine"),
            "{workflow} bypasses the single Hyperfine installer action"
        );
    }
}

#[test]
fn test_compare_workflow_pins_pyperf_to_the_harness_lock() {
    let workflow = fs::read_to_string(repo().join(".github/workflows/benchmark-compare.yml"))
        .expect("read benchmark compare workflow");
    assert_eq!(
        workflow.matches("pyperf==2.10.0").count(),
        2,
        "legacy Python base/head environments must use the same frozen pyperf distribution"
    );
}

#[test]
fn test_ci_runner_label_matches_runs_on() {
    for workflow in [
        "benchmark.yml",
        "benchmark-nightly.yml",
        "benchmark-compare.yml",
    ] {
        let text = fs::read_to_string(repo().join(".github/workflows").join(workflow))
            .unwrap_or_else(|error| panic!("could not read {workflow}: {error}"));
        let mut runs_on = text
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix("runs-on:"))
            .map(str::trim)
            .collect::<Vec<_>>();
        let mut declared = text
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix("BENCH_CI_RUNNER:"))
            .map(str::trim)
            .collect::<Vec<_>>();
        runs_on.sort_unstable();
        runs_on.dedup();
        declared.sort_unstable();
        declared.dedup();
        assert!(!declared.is_empty(), "{workflow} does not export BENCH_CI_RUNNER");
        assert_eq!(
            runs_on, declared,
            "{workflow} stamps a runner label different from the runner it actually uses"
        );
    }
}
