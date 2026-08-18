//! Live occurrence audit for frozen `tests/test_benchmarks_tooling.py`.
//!
//! This guard is intentionally not attached to the master behavior inventory until every frozen
//! benchmark-tooling test-function occurrence has an executable owner or one narrowly justified
//! architecture closure. Bare Python test names are a multiset: different classes can reuse one.

use std::{collections::BTreeMap, fs, path::Path};

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
    (
        "test_scoped_skit_dirs_restores_previously_unset_var",
        "Python temporarily rewrote process-global SKIT_* variables through scoped_skit_dirs; the Rust dataset generator passes explicit store/state/config paths and has no process-environment scope to restore",
    ),
    (
        "test_source_text_rejects_unknown_kind",
        "Python directly called private _source_text; the Rust source_text helper is private and its unsupported-kind branch has an in-module unit test, while public dataset generation validates kinds before dispatch",
    ),
    (
        "test_generate_refuses_silent_store_undercount",
        "Python injected a lying global store.list_entries; Rust generate performs the same post-generate count check through a concrete LibraryService<FileStore> but exposes no repository injection seam for an integration owner",
    ),
];
const OWNERS: &[&str] = &[
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_results.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_budget.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_parsers_compare.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_environment.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_pipeline.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_sources.rs",
    "crates/skit-benchmarks/tests/port_test_benchmarks_tooling_dataset.rs",
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

fn count_names(names: impl IntoIterator<Item = String>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for name in names {
        *counts.entry(name).or_default() += 1;
    }
    counts
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
    assert_eq!(
        frozen_list.len(),
        156,
        "frozen benchmark-tooling function-occurrence denominator changed"
    );
    let frozen = count_names(frozen_list);
    assert_eq!(
        frozen.get("test_rejects_bad_inputs"),
        Some(&2),
        "the frozen suite's cross-class duplicate-name sentinel changed"
    );
    for sentinel in [
        "test_round_trip",
        "test_loads_the_real_contract_file",
        "test_stats",
        "test_thresholds",
        "test_platform_key",
        "test_profiles",
        "test_exact_line_counts",
        "test_generate_small_library",
    ] {
        assert!(
            frozen.contains_key(sentinel),
            "preserved benchmark-tooling source lost sentinel {sentinel}"
        );
    }

    assert!(CLOSED.iter().all(|(_, reason)| !reason.trim().is_empty()));
    let closed = count_names(CLOSED.iter().map(|(name, _)| (*name).to_owned()));
    for (name, count) in &closed {
        assert!(
            frozen.get(name).is_some_and(|frozen_count| count <= frozen_count),
            "benchmark-tooling closure over-accounts non-frozen or duplicate occurrence {name:?}: closed={count}, frozen={:?}",
            frozen.get(name)
        );
    }

    let mut owner_names = Vec::new();
    for relative in OWNERS {
        owner_names.extend(executable_rust_tests(&repo.join(relative)));
    }
    let owners = count_names(owner_names);

    let mut missing = Vec::new();
    let mut extras = Vec::new();
    for (name, frozen_count) in &frozen {
        let closed_count = closed.get(name).copied().unwrap_or(0);
        let expected = frozen_count - closed_count;
        let actual = owners.get(name).copied().unwrap_or(0);
        if actual < expected {
            missing.push(format!("{name} x{}", expected - actual));
        } else if actual > expected {
            extras.push(format!("{name} x{}", actual - expected));
        }
    }
    for (name, count) in &owners {
        if !frozen.contains_key(name) {
            extras.push(format!("{name} x{count} (non-frozen)"));
        }
    }

    let executable_count = owners.values().sum::<usize>();
    let closed_count = closed.values().sum::<usize>();
    assert!(
        missing.is_empty() && extras.is_empty(),
        "benchmark-tooling live occurrence audit incomplete: executable={executable_count}/156 closed={closed_count} missing={missing:?} extras={extras:?}"
    );
    assert_eq!(
        executable_count + closed_count,
        156,
        "benchmark-tooling occurrence accounting must cover the frozen denominator exactly"
    );
}
