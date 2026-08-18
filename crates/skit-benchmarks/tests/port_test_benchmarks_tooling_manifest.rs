//! Live exact-name audit for frozen `tests/test_benchmarks_tooling.py`.
//!
//! This guard is intentionally not attached to the master behavior inventory until every frozen
//! benchmark-tooling name has an executable owner or one narrowly justified architecture closure.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

const CLOSED: &[(&str, &str)] = &[
    (
        "test_census",
        "Python-only sys.modules census parser; the native Rust imports suite records a zero Python-module census and has no module-graph parser input surface",
    ),
    (
        "test_importtime",
        "Python-only -X importtime tree parser; the native Rust imports suite emits an explicit not-applicable import-time artifact because no Python import tree exists",
    ),
    (
        "test_pyperf",
        "Python-only pyperf JSON parser; the Rust microbenchmark lane uses the native Criterion/CodSpeed harness and never consumes a pyperf artifact",
    ),
    (
        "test_ci_runner",
        "Python pure helper accepted an injected environment mapping; Rust collect_meta reads BENCH_CI_RUNNER/ImageVersion through a private non-empty process-environment helper with no map-injection seam",
    ),
    (
        "test_cpu_model",
        "Python parsed injected /proc/cpuinfo text; Rust collect_meta gets the CPU brand from sysinfo and exposes no text-parser seam",
    ),
    (
        "test_mem_and_git",
        "Python exposed pure memory-page and git-status text helpers; Rust collect_meta uses sysinfo total memory plus a bounded live git status process and exposes neither parser seam",
    ),
    (
        "test_dist_version_fallback",
        "Python queried importlib.metadata distribution versions; the native Rust benchmark harness has no Python distribution-metadata lookup and records native harness provenance instead",
    ),
    (
        "test_build_host_and_meta",
        "Python exposed pure injected HostInfo/Meta builder helpers; Rust collect_meta intentionally constructs them from bounded live probes and sysinfo, while typed Meta round-trip remains executable",
    ),
    (
        "test_deterministic",
        "Python benchmark source generation exposed a per-call seed injection; the Rust-only benchmark generator owns a fixed stable seed derived from language and line count and exposes no alternate-seed seam",
    ),
    (
        "test_generate_asserts_generator_line_count",
        "Python monkeypatched the private _shell generator to make it lie about line count; Rust language generators are private functions with no injectable generator callback, while public exact-line-count invariants remain executable",
    ),
];
const OWNERS: &[&str] = &[
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_results.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_budget.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_parsers_compare.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_environment.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_pipeline.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_sources.rs",
];

fn frozen_names(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let tail = line.strip_prefix("def test_")?;
            let name = tail.split_once('(')?.0;
            Some(format!("test_{name}"))
        })
        .collect()
}

fn executable_rust_tests(path: &Path) -> Vec<String> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
    let lines = source.lines().collect::<Vec<_>>();
    let mut names = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let Some(tail) = trimmed.strip_prefix("fn test_") else {
            continue;
        };
        let Some(name) = tail.split_once('(').map(|(name, _)| format!("test_{name}")) else {
            continue;
        };
        let has_test_attribute = lines[..index]
            .iter()
            .rev()
            .map(|line| line.trim())
            .take_while(|line| line.is_empty() || line.starts_with("#["))
            .any(|line| line == "#[test]");
        if has_test_attribute {
            names.push(name);
        }
    }
    names
}

#[test]
fn benchmarks_tooling_frozen_name_audit_is_live() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-benchmarks lives under <repo>/crates/skit-benchmarks");
    let python = fs::read_to_string(repo.join("tests/test_benchmarks_tooling.py"))
        .expect("preserved benchmark-tooling source");
    let frozen_list = frozen_names(&python);
    let frozen = frozen_list.iter().map(String::as_str).collect::<BTreeSet<_>>();
    assert_eq!(frozen_list.len(), 156, "frozen benchmark-tooling denominator changed");
    assert_eq!(frozen.len(), 156, "duplicate frozen benchmark-tooling test name");
    for sentinel in [
        "test_round_trip",
        "test_loads_the_real_contract_file",
        "test_stats",
        "test_thresholds",
        "test_platform_key",
        "test_profiles",
        "test_exact_line_counts",
    ] {
        assert!(
            frozen.contains(sentinel),
            "preserved benchmark-tooling source lost sentinel {sentinel}"
        );
    }

    let closed = CLOSED.iter().map(|(name, _)| *name).collect::<BTreeSet<_>>();
    assert_eq!(closed.len(), CLOSED.len(), "duplicate benchmark-tooling closure name");
    assert!(CLOSED.iter().all(|(_, reason)| !reason.trim().is_empty()));
    assert!(
        closed.is_subset(&frozen),
        "benchmark-tooling closure includes a non-frozen name"
    );

    let mut owners = BTreeMap::<String, String>::new();
    let mut duplicates = Vec::new();
    for relative in OWNERS {
        for name in executable_rust_tests(&repo.join(relative)) {
            if let Some(previous) = owners.insert(name.clone(), (*relative).to_owned()) {
                duplicates.push(format!("{name}: {previous} and {relative}"));
            }
        }
    }
    assert!(
        duplicates.is_empty(),
        "duplicate benchmark-tooling owners:\n{}",
        duplicates.join("\n")
    );

    let expected = frozen.difference(&closed).copied().collect::<BTreeSet<_>>();
    let actual = owners.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let missing = expected.difference(&actual).copied().collect::<Vec<_>>();
    let extras = actual.difference(&expected).copied().collect::<Vec<_>>();
    assert!(
        missing.is_empty() && extras.is_empty(),
        "benchmark-tooling live exact-name audit incomplete: executable={}/{} closed={} missing={missing:?} extras={extras:?}",
        actual.len(),
        frozen.len(),
        closed.len()
    );
}
