//! Rust-additive expansion of the six parametrized hand-edited state shapes from Python v0.4
//! `tests/test_argstate_mut.py`.
//!
//! The original exact Python test remains in `port_test_argstate.rs`; these cases ensure every
//! parametrized row executes independently so one malformed shape cannot hide a failure in another.

use std::{collections::BTreeMap, fs};

use skit_application::form_state::{FormStateRepository, LastRunState};
use skit_domain::Slug;
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug(value: &str) -> Slug {
    Slug::parse(value.to_owned()).unwrap()
}

fn write_state(root: &TempDir, slug: &Slug, body: &str) {
    let values_dir = root.path().join("values");
    fs::create_dir_all(&values_dir).unwrap();
    fs::write(values_dir.join(format!("{}.toml", slug.as_str())), body).unwrap();
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn rust_additive_scalar_values_drops_only_values_section() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let id = slug("scalar-values");
    write_state(&root, &id, "values = 5\nextra_args = [\"--verbose\"]\n");

    let state = store.load(&id);
    assert!(state.values.is_empty());
    assert_eq!(state.extra_args, ["--verbose"]);
    assert_eq!(state.last_run, LastRunState::default());
}

#[test]
fn rust_additive_scalar_extra_args_drops_only_extra_args_section() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let id = slug("scalar-extra-args");
    write_state(
        &root,
        &id,
        "extra_args = \"--verbose\"\n[values]\nCITY = \"Taipei\"\n",
    );

    let state = store.load(&id);
    assert_eq!(state.values, values(&[("CITY", "Taipei")]));
    assert!(state.extra_args.is_empty());
}

#[test]
fn rust_additive_scalar_presets_drops_only_presets_section() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let id = slug("scalar-presets");
    write_state(
        &root,
        &id,
        "presets = 5\n[values]\nCITY = \"Taipei\"\n",
    );

    let state = store.load(&id);
    assert_eq!(state.values, values(&[("CITY", "Taipei")]));
    assert!(state.presets.is_empty());
}

#[test]
fn rust_additive_scalar_preset_row_drops_only_broken_preset() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let id = slug("scalar-preset-row");
    write_state(
        &root,
        &id,
        "[presets]\nbroken = \"not a table\"\n\n[presets.prod]\nCITY = \"Osaka\"\n",
    );

    let state = store.load(&id);
    assert_eq!(
        state.presets,
        BTreeMap::from([("prod".to_owned(), values(&[("CITY", "Osaka")]))])
    );
}

#[test]
fn rust_additive_scalar_last_run_drops_only_last_run_section() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let id = slug("scalar-last-run");
    write_state(
        &root,
        &id,
        "last_run = \"garbage\"\n[values]\nCITY = \"Taipei\"\n",
    );

    let state = store.load(&id);
    assert_eq!(state.values, values(&[("CITY", "Taipei")]));
    assert_eq!(state.last_run, LastRunState::default());
    assert_eq!(store.last_run(&id), LastRunState::default());
}

#[test]
fn rust_additive_scalar_last_run_values_drops_only_nested_values() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let id = slug("scalar-last-run-values");
    write_state(
        &root,
        &id,
        "[last_run]\nat = \"2026-07-25T00:00:00+00:00\"\nexit = 0\nvalues = \"garbage\"\n",
    );

    let expected = LastRunState {
        at: Some("2026-07-25T00:00:00+00:00".to_owned()),
        exit: Some(0),
        values: None,
    };
    assert_eq!(store.load(&id).last_run, expected);
    assert_eq!(store.last_run(&id), expected);
}
