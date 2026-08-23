use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    process::Command,
};

use skit_benchmarks::{
    BenchmarkProfile, GitInfo, HostInfo, Meta, Metric, Results, SuiteKind, SuiteOutput,
    dataset::{DatasetManifest, dataset_dirs},
    report::RunRecord,
    tui_probe::ProbeResult,
};
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

fn benchmark_binary() -> &'static str {
    env!("CARGO_BIN_EXE_skit-bench")
}

fn repository_budgets() -> &'static str {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmarks/budgets.toml")
}

#[derive(Debug, Default)]
struct WorkflowContract {
    root_environment: BTreeMap<String, String>,
    runners: BTreeSet<String>,
    actions: Vec<String>,
    commands: Vec<String>,
}

fn yaml_scalar(text: &str) -> String {
    text.split(" #")
        .next()
        .unwrap_or(text)
        .trim()
        .trim_matches(['\'', '"'])
        .to_owned()
}

fn workflow_contract(text: &str) -> WorkflowContract {
    let mut contract = WorkflowContract::default();
    let mut root_environment = false;
    let mut command_indent = None;
    for raw in text.lines() {
        let indent = raw.len() - raw.trim_start_matches(' ').len();
        let trimmed = raw.trim();
        if let Some(block_indent) = command_indent {
            if trimmed.is_empty() || indent > block_indent {
                if !trimmed.is_empty() {
                    contract.commands.push(trimmed.to_owned());
                }
                continue;
            }
            command_indent = None;
        }
        if indent == 0 {
            root_environment = trimmed == "env:";
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim_start_matches("- ").trim();
        let value = yaml_scalar(value);
        if root_environment && indent == 2 {
            contract
                .root_environment
                .insert(key.to_owned(), value.clone());
        }
        match key {
            "runs-on" => {
                contract.runners.insert(value);
            }
            "uses" => contract.actions.push(value),
            "run" if value == "|" => command_indent = Some(indent),
            "run" => contract.commands.push(value),
            _ => {}
        }
    }
    contract
}

fn benchmark_workflows() -> [(&'static str, &'static str); 3] {
    [
        (
            "benchmark.yml",
            include_str!("../../../.github/workflows/benchmark.yml"),
        ),
        (
            "benchmark-nightly.yml",
            include_str!("../../../.github/workflows/benchmark-nightly.yml"),
        ),
        (
            "benchmark-compare.yml",
            include_str!("../../../.github/workflows/benchmark-compare.yml"),
        ),
    ]
}

#[test]
fn test_workflows_install_hyperfine_via_the_action() {
    for (name, text) in benchmark_workflows() {
        let workflow = workflow_contract(text);
        assert_eq!(
            workflow
                .actions
                .iter()
                .filter(|action| action.as_str() == "./.github/actions/install-hyperfine")
                .count(),
            1,
            "{name} must use the repository action exactly once"
        );
    }
}

#[test]
fn test_compare_workflow_pins_pyperf_to_the_harness_lock() {
    const V040_HARNESS_PYPERF: &str = "pyperf==2.10.0";
    let workflow = workflow_contract(include_str!(
        "../../../.github/workflows/benchmark-compare.yml"
    ));
    let pins = workflow
        .commands
        .iter()
        .filter_map(|command| shlex::split(command))
        .filter(|words| {
            words.starts_with(&["uv".to_owned(), "pip".to_owned(), "install".to_owned()])
        })
        .flatten()
        .filter(|token| token.starts_with("pyperf=="))
        .collect::<Vec<_>>();
    assert_eq!(
        pins,
        [
            V040_HARNESS_PYPERF.to_owned(),
            V040_HARNESS_PYPERF.to_owned()
        ]
    );
}

#[test]
fn test_ci_runner_label_matches_runs_on() {
    for (name, text) in benchmark_workflows() {
        let workflow = workflow_contract(text);
        let declared = workflow
            .root_environment
            .get(skit_benchmarks::environment::CI_RUNNER_VAR)
            .unwrap_or_else(|| panic!("{name} must export BENCH_CI_RUNNER"));
        assert_eq!(workflow.runners, BTreeSet::from([declared.clone()]));
    }
}

#[test]
fn check_and_compare_commands_use_validated_artifacts_and_stable_exit_codes() {
    let root = TempDir::new().unwrap();
    let results = root.path().join("results.json");
    fs::write(&results, local_result().to_json().unwrap()).unwrap();
    let budgets = repository_budgets();
    let binary = benchmark_binary();

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
fn dataset_and_probe_front_doors_use_real_product_inputs_and_machine_payloads() {
    let root = TempDir::new().unwrap();
    let dataset = root.path().join("dataset");
    let generated = Command::new(benchmark_binary())
        .args([
            "datasets",
            "--n",
            "2",
            "--out",
            dataset.to_str().unwrap(),
            "--seed",
            "19",
            "--state-fraction",
            "1",
        ])
        .output()
        .unwrap();
    assert!(
        generated.status.success(),
        "{}",
        String::from_utf8_lossy(&generated.stderr)
    );
    assert!(generated.stderr.is_empty());
    let manifest = DatasetManifest::load(&dataset).unwrap();
    assert_eq!(manifest.n, 2);
    assert_eq!(manifest.seed, 19);
    assert_eq!(manifest.state_fraction, 1.0);
    assert_eq!(manifest.slugs.len(), 2);
    assert_eq!(
        String::from_utf8(generated.stdout).unwrap(),
        format!("generated 2 entries in {}\n", dataset.display())
    );

    let source = root.path().join("probe.py");
    fs::write(&source, "VALUE = 1\n").unwrap();
    let analyzed = Command::new(benchmark_binary())
        .args([
            "probe",
            "analyze",
            "--kind",
            "python",
            "--source",
            source.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(analyzed.status.success());
    assert!(analyzed.stderr.is_empty());
    let elapsed = String::from_utf8(analyzed.stdout)
        .unwrap()
        .trim()
        .parse::<f64>()
        .unwrap();
    assert!(elapsed.is_finite() && elapsed >= 0.0);

    let dirs = dataset_dirs(&dataset).unwrap();
    let tui = Command::new(benchmark_binary())
        .args(["probe", "tui", "--entries", "2", "--probe-char", "0"])
        .env("SKIT_DATA_DIR", &dirs.data)
        .env("SKIT_STATE_DIR", &dirs.state)
        .env("SKIT_CONFIG_DIR", &dirs.config)
        .output()
        .unwrap();
    assert!(
        tui.status.success(),
        "{}",
        String::from_utf8_lossy(&tui.stderr)
    );
    assert!(tui.stderr.is_empty());
    let probe: ProbeResult = serde_json::from_slice(&tui.stdout).unwrap();
    probe.validate().unwrap();
    assert!(probe.select_ms.is_some());
}

#[test]
fn summarize_front_door_publishes_the_same_validated_json_and_markdown() {
    let root = TempDir::new().unwrap();
    let suites = root.path().join("suites");
    fs::create_dir(&suites).unwrap();
    let output = SuiteOutput {
        suite: SuiteKind::Startup,
        duration_seconds: 0.25,
        metrics: BTreeMap::from([
            (
                "startup.version.median_ms".to_owned(),
                Metric::single(8.0, "ms"),
            ),
            (
                "startup.python.median_ms".to_owned(),
                Metric::single(3.0, "ms"),
            ),
        ]),
        skipped: Vec::new(),
        raw: BTreeMap::new(),
    };
    fs::write(suites.join("startup.json"), output.to_json().unwrap()).unwrap();
    let run = RunRecord {
        meta: local_result().meta,
        total_duration_s: 0.5,
        suites: vec![SuiteKind::Startup],
    };
    fs::write(
        root.path().join("run.json"),
        serde_json::to_string_pretty(&run).unwrap(),
    )
    .unwrap();

    let summarized = Command::new(benchmark_binary())
        .args([
            "summarize",
            root.path().to_str().unwrap(),
            "--budgets",
            repository_budgets(),
        ])
        .output()
        .unwrap();
    assert!(
        summarized.status.success(),
        "{}",
        String::from_utf8_lossy(&summarized.stderr)
    );
    assert!(summarized.stderr.is_empty());
    let results =
        Results::from_json(&fs::read_to_string(root.path().join("results.json")).unwrap()).unwrap();
    assert_eq!(results.metrics["startup.version.over_python_ms"].value, 5.0);
    assert!(root.path().join("results.md").is_file());
    assert_eq!(
        String::from_utf8(summarized.stdout).unwrap(),
        format!(
            "results: {} ({} metrics)\n",
            root.path().join("results.json").display(),
            results.metrics.len()
        )
    );
}

#[test]
fn check_proposal_failure_and_zero_enforcement_have_distinct_exit_contracts() {
    let root = TempDir::new().unwrap();
    let results = root.path().join("results.json");
    let mut artifact = local_result();
    artifact.meta.host.ci_runner = Some("test-runner".to_owned());
    fs::write(&results, artifact.to_json().unwrap()).unwrap();
    let enforced = root.path().join("enforced.toml");
    fs::write(
        &enforced,
        "[[budget]]\nmetric='footprint.wheel_bytes'\nmax=0.5\ntier='enforced'\nratchet=true\ncontext={commit='previous'}\n",
    )
    .unwrap();

    let failed = Command::new(benchmark_binary())
        .args([
            "check",
            results.to_str().unwrap(),
            "--budgets",
            enforced.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&failed.stdout).contains("FAIL"));
    assert!(failed.stderr.is_empty());

    let proposal = Command::new(benchmark_binary())
        .args([
            "check",
            results.to_str().unwrap(),
            "--budgets",
            enforced.to_str().unwrap(),
            "--propose",
            "--allow-regression",
        ])
        .output()
        .unwrap();
    assert!(proposal.status.success());
    assert!(String::from_utf8_lossy(&proposal.stdout).contains("footprint.wheel_bytes"));
    assert!(proposal.stderr.is_empty());

    let target = root.path().join("target.toml");
    fs::write(
        &target,
        "[[budget]]\nmetric='footprint.wheel_bytes'\nmax=2\ntier='target'\n",
    )
    .unwrap();
    let zero = Command::new(benchmark_binary())
        .args([
            "check",
            results.to_str().unwrap(),
            "--budgets",
            target.to_str().unwrap(),
            "--require-enforced",
        ])
        .output()
        .unwrap();
    assert_eq!(zero.status.code(), Some(1));
    let zero_stdout = String::from_utf8(zero.stdout).unwrap();
    assert!(zero_stdout.contains("[target]   ok  footprint.wheel_bytes: passed"));
    assert!(zero_stdout.contains("enforced: 0 rows, 0 evaluated, 0 passed, 0 failed"));
    assert_eq!(
        String::from_utf8(zero.stderr).unwrap(),
        "check: zero applicable enforced rows were evaluated\n"
    );
}

#[test]
fn front_door_errors_name_the_failed_input_without_partial_artifacts() {
    let root = TempDir::new().unwrap();
    let missing = root.path().join("missing.json");
    let compare = Command::new(benchmark_binary())
        .args([
            "compare",
            missing.to_str().unwrap(),
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(compare.status.code(), Some(1));
    assert!(compare.stdout.is_empty());
    assert!(String::from_utf8_lossy(&compare.stderr).contains("could not read result artifact"));

    let analyzed = Command::new(benchmark_binary())
        .args([
            "probe",
            "analyze",
            "--kind",
            "python",
            "--source",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(analyzed.status.code(), Some(1));
    assert!(analyzed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&analyzed.stderr).contains("could not read"));

    let invalid_profile = Command::new(benchmark_binary())
        .args([
            "run",
            "--profile",
            "unknown",
            "--skit-binary",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(invalid_profile.status.code(), Some(1));
    assert!(invalid_profile.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid_profile.stderr).contains("unknown profile"));

    if usize::BITS == 64 {
        let oversized_path = root.path().join("oversized");
        let oversized = Command::new(benchmark_binary())
            .args([
                "datasets",
                "--n",
                "18446744073709551615",
                "--out",
                oversized_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert_eq!(oversized.status.code(), Some(1));
        assert!(oversized.stdout.is_empty());
        assert!(String::from_utf8_lossy(&oversized.stderr).contains("does not fit isize"));
        assert!(!oversized_path.exists());
    }
}

#[test]
fn compare_workflow_keeps_latest_python_main_as_a_schema_v1_side() {
    let workflow = include_str!("../../../.github/workflows/benchmark-compare.yml");
    for required in [
        "pull_request:",
        "ref: ${{ github.event.pull_request.base.sha }}",
        "ref: ${{ github.event.pull_request.head.sha }}",
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
    assert!(!workflow.contains("workflow_dispatch:"));
    assert!(!workflow.contains("inputs.base"));
    assert!(!workflow.contains("inputs.head"));
    assert_eq!(
        workflow
            .matches(".venv/bin/python -m benchmarks run")
            .count(),
        2
    );
    assert!(workflow.contains("schema-v1 Python benchmark harness"));
}
