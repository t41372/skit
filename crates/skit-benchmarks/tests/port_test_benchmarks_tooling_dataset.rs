//! Frozen dataset contracts from `tests/test_benchmarks_tooling.py`.

use std::{collections::BTreeSet, env, fs, path::Path};

use skit_application::{EntryRepository as _, form_state::FormStateService};
use skit_benchmarks::dataset::{
    DEFAULT_SEED, DEFAULT_STATE_FRACTION, DatasetManifest, SEARCH_PROBE_CHAR, dataset_dirs, generate,
    generate_runover,
};
use skit_store::{FileFormStateStore, FileStore};
use tempfile::TempDir;

#[test]
fn test_generate_small_library() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        30,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    assert_eq!(manifest.n, 30);
    assert_eq!(manifest.slugs.len(), 30);
    assert_eq!(
        manifest
            .slugs
            .iter()
            .map(|slug| slug.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        30
    );

    let dirs = dataset_dirs(&manifest.root).unwrap();
    let scan = FileStore::new(dirs.data).scan().unwrap();
    assert_eq!(scan.entries.len(), 30);
    assert_eq!(
        scan.entries
            .iter()
            .map(|entry| entry.slug.as_str())
            .collect::<BTreeSet<_>>(),
        manifest
            .slugs
            .iter()
            .map(|slug| slug.as_str())
            .collect::<BTreeSet<_>>()
    );
    let first = scan
        .entries
        .iter()
        .find(|entry| entry.slug == manifest.slugs[0])
        .unwrap();
    assert!(
        !format!("{} {}", first.name, first.description).contains(SEARCH_PROBE_CHAR),
        "entry zero must remain the search no-match probe"
    );
}

#[test]
fn test_generate_is_deterministic() {
    let root = TempDir::new().unwrap();
    let first = generate(&root.path().join("a"), 15, DEFAULT_SEED, DEFAULT_STATE_FRACTION).unwrap();
    let second =
        generate(&root.path().join("b"), 15, DEFAULT_SEED, DEFAULT_STATE_FRACTION).unwrap();
    assert_eq!(first.slugs, second.slugs);
    assert_eq!(first.kinds, second.kinds);
    let different = generate(&root.path().join("c"), 15, 999, DEFAULT_STATE_FRACTION).unwrap();
    assert_ne!(different.slugs, first.slugs);
}

#[test]
fn test_generate_env_is_restored() {
    let names = ["SKIT_DATA_DIR", "SKIT_STATE_DIR", "SKIT_CONFIG_DIR"];
    let before = names.map(env::var_os);
    let root = TempDir::new().unwrap();
    generate(
        &root.path().join("ds"),
        3,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let after = names.map(env::var_os);
    assert_eq!(after, before, "dataset generation leaked process-global SKIT_* state");
}

#[test]
fn test_kind_mix_and_missing_targets_at_100() {
    let root = TempDir::new().unwrap();
    let manifest = generate(&root.path().join("ds"), 100, DEFAULT_SEED, DEFAULT_STATE_FRACTION)
        .unwrap();
    let kinds = manifest.kinds.values().map(String::as_str).collect::<Vec<_>>();
    assert_eq!(kinds.iter().filter(|kind| **kind == "python").count(), 30);
    assert_eq!(kinds.iter().filter(|kind| **kind == "shell").count(), 20);
    assert_eq!(kinds.iter().filter(|kind| **kind == "prompt").count(), 10);
    assert_eq!(kinds.iter().filter(|kind| **kind == "exe").count(), 6);
    for kind in ["ruby", "perl", "lua", "r"] {
        assert!(kinds.contains(&kind), "missing long-tail kind {kind}");
    }

    let dirs = dataset_dirs(&manifest.root).unwrap();
    let store = FileStore::new(dirs.data);
    let missing = manifest
        .slugs
        .iter()
        .filter(|slug| manifest.kinds[slug.as_str()] == "exe")
        .filter(|slug| {
            let entry = store.resolve(slug.as_str()).unwrap();
            !Path::new(&entry.meta.source).exists()
        })
        .count();
    assert!(
        missing > 0,
        "every tenth reference entry's target is deliberately deleted"
    );
}

#[test]
fn test_state_fraction() {
    let root = TempDir::new().unwrap();
    for (name, fraction, expect_state) in [("none", 0.0, false), ("all", 1.0, true)] {
        let manifest = generate(&root.path().join(name), 8, DEFAULT_SEED, fraction).unwrap();
        let dirs = dataset_dirs(&manifest.root).unwrap();
        let state = FormStateService::new(FileFormStateStore::new(dirs.state));
        let with_state = manifest
            .slugs
            .iter()
            .filter(|slug| state.last_run(slug).at.is_some())
            .count();
        assert_eq!(with_state > 0, expect_state);
        if expect_state {
            assert_eq!(with_state, manifest.n);
        }
    }
}

#[test]
fn test_refuses_non_empty_root() {
    let root = TempDir::new().unwrap();
    let dataset = root.path().join("ds");
    fs::create_dir(&dataset).unwrap();
    fs::write(dataset.join("junk"), "x").unwrap();
    let error = generate(&dataset, 3, DEFAULT_SEED, DEFAULT_STATE_FRACTION)
        .unwrap_err()
        .to_string();
    assert!(error.contains("refusing"));
    assert_eq!(fs::read_to_string(dataset.join("junk")).unwrap(), "x");
}

#[test]
fn test_rejects_bad_inputs() {
    let root = TempDir::new().unwrap();
    let negative = generate(&root.path().join("a"), -1, DEFAULT_SEED, DEFAULT_STATE_FRACTION)
        .unwrap_err()
        .to_string();
    assert!(negative.contains("n must be"));
    let fraction = generate(&root.path().join("b"), 1, DEFAULT_SEED, 1.5)
        .unwrap_err()
        .to_string();
    assert!(fraction.contains("state_fraction"));
}

#[test]
fn test_manifest_round_trip_and_mid_slug() {
    let root = TempDir::new().unwrap();
    let manifest = generate(
        &root.path().join("ds"),
        9,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let loaded = DatasetManifest::load(&manifest.root).unwrap();
    assert_eq!(loaded.slugs, manifest.slugs);
    assert_eq!(loaded.middle_slug().unwrap(), &manifest.slugs[4]);

    let empty = generate(
        &root.path().join("empty"),
        0,
        DEFAULT_SEED,
        DEFAULT_STATE_FRACTION,
    )
    .unwrap();
    let error = empty.middle_slug().unwrap_err().to_string();
    assert!(error.contains("no middle entry"));
}

#[test]
fn test_runover_library() {
    let root = TempDir::new().unwrap();
    let manifest = generate_runover(root.path().join("ro")).unwrap();
    assert_eq!(manifest.n, 3);
    assert_eq!(
        manifest.kinds.values().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["python", "shell", "js"])
    );
    let store = FileStore::new(dataset_dirs(&manifest.root).unwrap().data);
    let names = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        names,
        BTreeSet::from([
            "noop-py".to_owned(),
            "noop-sh".to_owned(),
            "noop-js".to_owned(),
        ])
    );
}
