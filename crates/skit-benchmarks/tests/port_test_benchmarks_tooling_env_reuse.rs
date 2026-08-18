//! Frozen environment, dataset-reuse, and search-probe contracts.

use std::collections::BTreeSet;

use skit_benchmarks::{
    dataset::{
        DEFAULT_SEED, DEFAULT_STATE_FRACTION, DatasetManifest, SEARCH_PROBE_CHAR, check_reusable,
        dataset_dirs, generate,
    },
    environment::{bench_path, build_environment},
};
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_build_env_composes_path_and_pins_locale() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        3,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let work = root.path().join("work");
    let env = build_environment(
        "/venv/bin/skit",
        Some("/tools/bin/uv"),
        Some("/node/bin/node"),
        &work,
        &manifest.root,
    )
    .unwrap();
    assert_eq!(
        env["PATH"],
        "/venv/bin:/tools/bin:/node/bin:/usr/bin:/bin"
    );
    assert_eq!(env["LC_ALL"], "C.UTF-8");
    assert_eq!(env["PYTHONUTF8"], "1");
    assert_eq!(env["SKIT_LANG"], "en");
    assert_eq!(env["TERM"], "dumb");
    assert_eq!(env["COLUMNS"], "100");
    assert_eq!(env["LINES"], "40");
    assert_eq!(
        env["SKIT_DATA_DIR"],
        manifest.root.join("data").display().to_string()
    );
    assert_eq!(
        env["SKIT_STATE_DIR"],
        manifest.root.join("state").display().to_string()
    );
    assert_eq!(
        env["SKIT_CONFIG_DIR"],
        manifest.root.join("config").display().to_string()
    );
    assert!(work.join("home").is_dir());
    assert!(!env.contains_key("PYTHONPATH"));
    assert!(!env.contains_key("FORCE_COLOR"));
}

#[test]
fn test_build_env_dedupes_and_tolerates_missing_tools() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("n0"),
        0,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let env = build_environment(
        "/usr/bin/skit",
        Some("/usr/bin/uv"),
        None,
        &root.path().join("work"),
        &manifest.root,
    )
    .unwrap();
    assert_eq!(env["PATH"], "/usr/bin:/bin");
    assert_eq!(
        env["SKIT_DATA_DIR"],
        manifest.root.join("data").display().to_string()
    );
}

#[test]
fn test_bench_path_is_what_build_env_exports() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        0,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let expected = bench_path("/venv/bin/skit", Some("/tools/bin/uv"), Some("/node/bin/node"));
    let env = build_environment(
        "/venv/bin/skit",
        Some("/tools/bin/uv"),
        Some("/node/bin/node"),
        &root.path().join("work"),
        &manifest.root,
    )
    .unwrap();
    assert_eq!(env["PATH"], expected);
}

#[test]
fn test_build_env_refuses_non_dataset_roots() {
    let root = TempDir::new().unwrap();
    let bogus = root.path().join("not-a-dataset");
    std::fs::create_dir(&bogus).unwrap();
    let error = build_environment(
        "/usr/bin/skit",
        None,
        None,
        &root.path().join("work"),
        &bogus,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("not a generated dataset"));
}

#[test]
fn test_check_reusable_accepts_a_fresh_dataset() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        3,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let loaded = DatasetManifest::load(&manifest.root).unwrap();
    check_reusable(&loaded, 3).unwrap();
}

#[test]
fn test_check_reusable_rejects_any_drift() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        3,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let mut loaded = DatasetManifest::load(&manifest.root).unwrap();
    assert!(check_reusable(&loaded, 4).unwrap_err().to_string().contains("different inputs"));
    loaded.skit_version = "0.0.0+other".to_owned();
    assert!(check_reusable(&loaded, 3).unwrap_err().to_string().contains("different inputs"));
}

#[test]
fn test_manifest_stamps_the_writing_skit() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        3,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    assert_eq!(manifest.skit_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        DatasetManifest::load(&manifest.root).unwrap().skit_version,
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn test_dataset_guarantees_both_sides_of_the_filter_assertion() {
    for n in [3, 9] {
        let root = TempDir::new().unwrap();
        let manifest = generate(
            &root.path().join(format!("n{n}")),
            n,
            DEFAULT_SEED,
            DEFAULT_STATE_FRACTION,
        )
        .unwrap();
        let dirs = dataset_dirs(&manifest.root).unwrap();
        let entries = FileStore::new(dirs.data).scan().unwrap().entries;
        let matches = entries
            .iter()
            .filter(|entry| {
                format!("{} {}", entry.name, entry.description).contains(SEARCH_PROBE_CHAR)
            })
            .map(|entry| entry.slug.to_string())
            .collect::<BTreeSet<_>>();
        assert!(
            !matches.is_empty() && matches.len() < entries.len(),
            "n={n} must preserve both matching and filtered-out rows"
        );
    }
}
