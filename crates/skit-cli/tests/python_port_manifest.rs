//! Enforce the Python-oracle inventory and the executable Rust port map.
//!
//! A green Rust test file is not parity by itself. This guard makes a `done` row mean that every
//! Python test name from the pinned oracle exists as a non-ignored Rust test. Partial rows must name
//! every deferred contract explicitly, and `port_test_*.rs` files cannot hide unmapped or ignored
//! tests.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

const ORACLE_COMMIT: &str = "206f9ef946fc45835cb2479593794431f2620c32";

#[derive(Debug, Deserialize)]
struct OracleInventory {
    schema: u64,
    oracle: OracleReference,
    summary: OracleSummary,
    modules: Vec<OracleModule>,
}

#[derive(Debug, Deserialize)]
struct OracleReference {
    commit: String,
}

#[derive(Debug, Deserialize)]
struct OracleSummary {
    behavior: ClassSummary,
    mutation: ClassSummary,
    coverage: ClassSummary,
}

#[derive(Debug, Deserialize)]
struct ClassSummary {
    modules: usize,
    tests: usize,
}

#[derive(Debug, Deserialize)]
struct OracleModule {
    path: String,
    class: String,
    source_sha256: String,
    test_count: usize,
    test_names_fnv1a128: String,
}

#[derive(Debug, Deserialize)]
struct PortMap {
    schema: u64,
    oracle_commit: String,
    modules: Vec<PortModule>,
}

#[derive(Debug, Deserialize)]
struct PortModule {
    source: String,
    status: String,
    targets: Vec<String>,
    ported: Vec<String>,
    deferred: Vec<DeferredTest>,
}

#[derive(Debug, Deserialize)]
struct DeferredTest {
    name: String,
    reason: String,
}

fn workspace_root() -> PathBuf {
    fs::canonicalize(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("the workspace root exists")
}

fn read_json<T: for<'de> Deserialize<'de>>(root: &Path, path: &str) -> T {
    let bytes = fs::read(root.join(path)).unwrap_or_else(|error| panic!("read {path}: {error}"));
    serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("parse {path}: {error}"))
}

fn test_names_fnv1a128<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

    let mut names = names.into_iter().collect::<Vec<_>>();
    names.sort_unstable();
    let mut hash = OFFSET;
    for name in names {
        for byte in name.bytes().chain(std::iter::once(0)) {
            hash ^= u128::from(byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("{hash:032x}")
}

fn rust_tests(source: &str) -> BTreeMap<String, bool> {
    let mut tests = BTreeMap::new();
    let mut has_test_attribute = false;
    let mut ignored = false;

    for line in source.lines() {
        let line = line.trim();
        if line.starts_with("#[test") {
            has_test_attribute = true;
            continue;
        }
        if line.starts_with("#[ignore") {
            ignored = true;
            continue;
        }
        if line.is_empty() || line.starts_with("//") || line.starts_with("#[") {
            continue;
        }

        let function = line
            .strip_prefix("fn test_")
            .or_else(|| line.strip_prefix("async fn test_"));
        if has_test_attribute
            && let Some(function) = function
            && let Some(suffix) = function.split_once('(').map(|(name, _)| name)
        {
            let name = format!("test_{suffix}");
            assert!(
                tests.insert(name.clone(), ignored).is_none(),
                "duplicate Rust test function {name}"
            );
        }
        has_test_attribute = false;
        ignored = false;
    }
    tests
}

fn collect_port_files(root: &Path, directory: &Path, output: &mut BTreeSet<String>) {
    for entry in fs::read_dir(directory).unwrap_or_else(|error| {
        panic!("scan {}: {error}", directory.display());
    }) {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_port_files(root, &path, output);
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name.starts_with("port_test_") && name.ends_with(".rs") {
            output.insert(
                path.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

#[test]
fn python_oracle_inventory_and_port_status_are_machine_checked() {
    let root = workspace_root();
    let oracle: OracleInventory = read_json(&root, "docs/design/python-test-oracle.json");
    let port_map: PortMap = read_json(&root, "docs/design/python-test-port-map.json");

    assert_eq!(oracle.schema, 1);
    assert_eq!(port_map.schema, 1);
    assert_eq!(oracle.oracle.commit, ORACLE_COMMIT);
    assert_eq!(port_map.oracle_commit, ORACLE_COMMIT);
    assert_eq!(
        (
            oracle.summary.behavior.modules,
            oracle.summary.behavior.tests
        ),
        (84, 3_018)
    );
    assert_eq!(
        (
            oracle.summary.mutation.modules,
            oracle.summary.mutation.tests
        ),
        (72, 1_010)
    );
    assert_eq!(
        (
            oracle.summary.coverage.modules,
            oracle.summary.coverage.tests
        ),
        (19, 578)
    );
    assert_eq!(oracle.modules.len(), 175);

    let mut oracle_by_path = BTreeMap::new();
    let mut observed_summary: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for module in &oracle.modules {
        assert!(
            oracle_by_path
                .insert(module.path.as_str(), module)
                .is_none(),
            "duplicate oracle module {}",
            module.path
        );
        let summary = observed_summary
            .entry(module.class.as_str())
            .or_insert((0, 0));
        summary.0 += 1;
        summary.1 += module.test_count;
        assert_eq!(
            module.source_sha256.len(),
            64,
            "invalid source digest in {}",
            module.path
        );
        assert_eq!(
            module.test_names_fnv1a128.len(),
            32,
            "invalid test-name digest in {}",
            module.path
        );
    }
    assert_eq!(observed_summary.get("behavior"), Some(&(84, 3_018)));
    assert_eq!(observed_summary.get("mutation"), Some(&(72, 1_010)));
    assert_eq!(observed_summary.get("coverage"), Some(&(19, 578)));

    let mut mapped_sources = BTreeSet::new();
    let mut mapped_targets = BTreeSet::new();
    let mut ported_by_target: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for module in &port_map.modules {
        assert!(
            mapped_sources.insert(module.source.as_str()),
            "duplicate port-map source {}",
            module.source
        );
        let oracle_module = oracle_by_path
            .get(module.source.as_str())
            .unwrap_or_else(|| panic!("port map references unknown source {}", module.source));
        assert_eq!(
            oracle_module.class, "behavior",
            "only behavior modules may claim direct parity: {}",
            module.source
        );
        assert!(
            !module.targets.is_empty(),
            "no Rust target for {}",
            module.source
        );

        let ported = module
            .ported
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let deferred = module
            .deferred
            .iter()
            .map(|test| test.name.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            ported.len(),
            module.ported.len(),
            "duplicate ported name in {}",
            module.source
        );
        assert_eq!(
            deferred.len(),
            module.deferred.len(),
            "duplicate deferred name in {}",
            module.source
        );
        assert!(
            ported.is_disjoint(&deferred),
            "ported and deferred overlap in {}",
            module.source
        );
        assert!(
            module
                .deferred
                .iter()
                .all(|test| !test.reason.trim().is_empty()),
            "blank deferral reason in {}",
            module.source
        );

        match module.status.as_str() {
            "done" => {
                assert!(
                    deferred.is_empty(),
                    "done row defers tests in {}",
                    module.source
                );
            }
            "partial" => {
                assert!(
                    !ported.is_empty(),
                    "partial row ports nothing in {}",
                    module.source
                );
                assert!(
                    !deferred.is_empty(),
                    "partial row defers nothing in {}",
                    module.source
                );
            }
            status => panic!("unknown port status {status:?} for {}", module.source),
        }
        let accounted = ported.union(&deferred).copied().collect::<BTreeSet<_>>();
        assert_eq!(
            accounted.len(),
            oracle_module.test_count,
            "wrong test count for {}",
            module.source
        );
        assert_eq!(
            test_names_fnv1a128(accounted.iter().copied()),
            oracle_module.test_names_fnv1a128,
            "ported/deferred names do not match the pinned oracle for {}",
            module.source
        );

        let mut target_tests = BTreeMap::new();
        let mut target_count_by_test = BTreeMap::<String, usize>::new();
        for target in &module.targets {
            let path = root.join(target);
            assert!(path.is_file(), "missing Rust target {target}");
            mapped_targets.insert(target.as_str());
            let source =
                fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {target}: {error}"));
            for (name, ignored) in rust_tests(&source) {
                if ported.contains(name.as_str()) {
                    ported_by_target
                        .entry(target.clone())
                        .or_default()
                        .insert(name.clone());
                    *target_count_by_test.entry(name.clone()).or_default() += 1;
                }
                target_tests
                    .entry(name)
                    .and_modify(|seen_ignored| *seen_ignored &= ignored)
                    .or_insert(ignored);
            }
        }
        for name in &module.ported {
            let ignored = target_tests.get(name).unwrap_or_else(|| {
                panic!(
                    "ported test {name} has no Rust function in targets for {}",
                    module.source
                )
            });
            assert!(
                !ignored,
                "ported test {name} is ignored in {}",
                module.source
            );
            assert_eq!(
                target_count_by_test.get(name),
                Some(&1),
                "ported test {name} must exist in exactly one Rust target for {}",
                module.source
            );
        }
    }

    let mut port_files = BTreeSet::new();
    collect_port_files(&root, &root.join("crates"), &mut port_files);
    assert_eq!(
        port_files,
        mapped_targets
            .iter()
            .filter(|path| path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.starts_with("port_test_")))
            .copied()
            .map(str::to_owned)
            .collect(),
        "every port_test_*.rs file must be present in the machine-readable map"
    );

    for path in &port_files {
        let source = fs::read_to_string(root.join(path)).unwrap();
        assert!(
            !source.contains("#[ignore"),
            "ignored tests cannot count as a port: {path}"
        );
        let actual = rust_tests(&source).into_keys().collect::<BTreeSet<_>>();
        let expected = ported_by_target
            .get(path)
            .unwrap_or_else(|| panic!("port file has no mapped tests: {path}"))
            .clone();
        assert_eq!(actual, expected, "unmapped or missing tests in {path}");
    }
}
