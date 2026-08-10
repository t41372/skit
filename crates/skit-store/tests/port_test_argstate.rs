//! Mechanical behavioral port of state persistence cases from
//! `origin/main@206f9ef:tests/test_argstate_mut.py`.
//!
//! These tests deliberately exercise only the public Rust repository/service boundaries. The
//! Python suite is the oracle; a red assertion is a parity finding, not permission to patch the
//! store implementation in this branch.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_application::form_state::{
    FormStateRepository, FormStateService, LastRunState, PersistedFormState,
};
use skit_domain::{Slug, parameters::ParamDecl};
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug(value: &str) -> Slug {
    Slug::parse(value.to_owned()).unwrap()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn public(name: &str) -> ParamDecl {
    ParamDecl::new(name)
}

fn secret(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.secret = true;
    declaration
}

fn write_state(root: &TempDir, slug: &Slug, body: &str) {
    let values_dir = root.path().join("values");
    fs::create_dir_all(&values_dir).unwrap();
    fs::write(values_dir.join(format!("{}.toml", slug.as_str())), body).unwrap();
}

#[test]
fn test_a_missing_state_file_is_empty_not_an_error() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("absent");

    assert_eq!(store.load(&slug), PersistedFormState::default());
    assert_eq!(store.last_run(&slug), LastRunState::default());
}

#[test]
fn test_values_lock_path_shape() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("my-slug");

    store.update(&slug, |_| ()).unwrap();

    let lock = root.path().join(".locks").join("my-slug.values.lock");
    assert!(lock.is_file());
    assert_eq!(lock.parent().unwrap().file_name().unwrap(), ".locks");
    assert_eq!(lock.parent().unwrap().parent().unwrap(), root.path());
}

#[test]
fn test_concurrent_save_preset_from_many_threads_loses_no_preset() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileFormStateStore::new(root.path()));
    let slug = slug("many-threads");
    let barrier = Arc::new(Barrier::new(8));

    let handles = (0..8)
        .map(|index| {
            let store = Arc::clone(&store);
            let slug = slug.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                let name = format!("p{index}");
                barrier.wait();
                store
                    .update(&slug, |state| {
                        state
                            .presets
                            .insert(name.clone(), values(&[(&name, "v")]));
                    })
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().unwrap();
    }

    let state = store.load(&slug);
    assert_eq!(state.presets.len(), 8);
    for index in 0..8 {
        let name = format!("p{index}");
        assert_eq!(state.presets.get(&name), Some(&values(&[(&name, "v")])));
    }
}

#[test]
fn test_last_run_matches_load_state_before_and_after_a_run() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("last-run");

    assert_eq!(store.last_run(&slug), store.load(&slug).last_run);

    store
        .update(&slug, |state| {
            state.values = values(&[("a", "1")]);
            state.extra_args = vec!["--x".to_owned()];
            state
                .presets
                .insert("prod".to_owned(), values(&[("a", "2")]));
        })
        .unwrap();
    assert_eq!(store.last_run(&slug), LastRunState::default());

    store
        .update(&slug, |state| {
            state.last_run.at = Some("2026-07-25T00:00:00+00:00".to_owned());
            state.last_run.exit = Some(3);
        })
        .unwrap();

    assert_eq!(store.last_run(&slug), store.load(&slug).last_run);
    assert_eq!(store.last_run(&slug).exit, Some(3));
}

#[test]
fn test_last_run_is_a_copy_not_the_stored_mapping() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("copy-run");
    store
        .update(&slug, |state| {
            state.last_run.at = Some("2026-07-25T00:00:00+00:00".to_owned());
            state.last_run.exit = Some(0);
        })
        .unwrap();

    let mut first = store.last_run(&slug);
    first.exit = Some(99);

    assert_eq!(store.last_run(&slug).exit, Some(0));
}

#[test]
fn test_a_hand_edited_state_file_drops_only_the_malformed_section() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());

    let scalar_values = slug("scalar-values");
    write_state(
        &root,
        &scalar_values,
        "values = 5\nextra_args = [\"--verbose\"]\n",
    );
    let state = store.load(&scalar_values);
    assert!(state.values.is_empty());
    assert_eq!(state.extra_args, ["--verbose"]);

    let scalar_extra = slug("scalar-extra-args");
    write_state(
        &root,
        &scalar_extra,
        "extra_args = \"--verbose\"\n[values]\nCITY = \"Taipei\"\n",
    );
    let state = store.load(&scalar_extra);
    assert_eq!(state.values, values(&[("CITY", "Taipei")]));
    assert!(state.extra_args.is_empty());

    let scalar_presets = slug("scalar-presets");
    write_state(
        &root,
        &scalar_presets,
        "presets = 5\n[values]\nCITY = \"Taipei\"\n",
    );
    let state = store.load(&scalar_presets);
    assert_eq!(state.values, values(&[("CITY", "Taipei")]));
    assert!(state.presets.is_empty());

    let scalar_preset_row = slug("scalar-preset-row");
    write_state(
        &root,
        &scalar_preset_row,
        "[presets]\nbroken = \"not a table\"\n\n[presets.prod]\nCITY = \"Osaka\"\n",
    );
    let state = store.load(&scalar_preset_row);
    assert_eq!(
        state.presets,
        BTreeMap::from([("prod".to_owned(), values(&[("CITY", "Osaka")]))])
    );

    let scalar_last_run = slug("scalar-last-run");
    write_state(
        &root,
        &scalar_last_run,
        "last_run = \"garbage\"\n[values]\nCITY = \"Taipei\"\n",
    );
    let state = store.load(&scalar_last_run);
    assert_eq!(state.values, values(&[("CITY", "Taipei")]));
    assert_eq!(state.last_run, LastRunState::default());
    assert_eq!(store.last_run(&scalar_last_run), LastRunState::default());

    let scalar_last_run_values = slug("scalar-last-run-values");
    write_state(
        &root,
        &scalar_last_run_values,
        "[last_run]\nat = \"2026-07-25T00:00:00+00:00\"\nexit = 0\nvalues = \"garbage\"\n",
    );
    let state = store.load(&scalar_last_run_values);
    assert_eq!(
        state.last_run,
        LastRunState {
            at: Some("2026-07-25T00:00:00+00:00".to_owned()),
            exit: Some(0),
            values: None,
        }
    );
    assert_eq!(
        store.last_run(&scalar_last_run_values),
        LastRunState {
            at: Some("2026-07-25T00:00:00+00:00".to_owned()),
            exit: Some(0),
            values: None,
        }
    );
}

#[test]
fn test_purge_secret_reports_names_removed_across_values_and_presets() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let slug = slug("purge-demo");
    repository
        .update(&slug, |state| {
            state.values = values(&[("API_TOKEN", "abc"), ("REGION", "us")]);
            state
                .presets
                .insert("prod".to_owned(), values(&[("REGION", "eu")]));
        })
        .unwrap();
    let service = FormStateService::new(repository.clone());
    let declarations = [secret("API_TOKEN"), public("REGION")];

    let removed = service.purge_secrets(&slug, &declarations).unwrap();

    assert_eq!(removed, BTreeSet::from(["API_TOKEN".to_owned()]));
    let state = repository.load(&slug);
    assert_eq!(state.values, values(&[("REGION", "us")]));
    assert_eq!(
        state.presets,
        BTreeMap::from([("prod".to_owned(), values(&[("REGION", "eu")]))])
    );
}

#[test]
fn test_save_last_drops_secret_with_no_stored_values_table() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let slug = slug("no-values-table");
    repository
        .update(&slug, |state| {
            state.extra_args = vec!["--verbose".to_owned()];
        })
        .unwrap();
    let service = FormStateService::new(repository.clone());

    service
        .save_last(&slug, &[secret("SECRET")], None, None, false)
        .unwrap();

    let state = repository.load(&slug);
    assert!(state.values.is_empty());
    assert_eq!(state.extra_args, ["--verbose"]);
}

#[test]
fn test_last_run_snapshot_strips_and_retroactively_purges_secrets() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let slug = slug("run-snapshot");
    repository
        .update(&slug, |state| {
            state.last_run.at = Some("2026-07-09T00:00:00+00:00".to_owned());
            state.last_run.exit = Some(0);
            state.last_run.values = Some(values(&[("TOKEN", "plaintext"), ("CITY", "Taipei")]));
        })
        .unwrap();
    let service = FormStateService::new(repository.clone());
    let declarations = [secret("TOKEN"), public("CITY")];

    let removed = service.purge_secrets(&slug, &declarations).unwrap();

    assert_eq!(removed, BTreeSet::from(["TOKEN".to_owned()]));
    assert_eq!(
        repository.load(&slug).last_run.values,
        Some(values(&[("CITY", "Taipei")]))
    );

    let submitted = values(&[("TOKEN", "new-secret"), ("CITY", "Osaka")]);
    service
        .record_run(
            &slug,
            0,
            "2026-07-10T00:00:00+00:00",
            &declarations,
            Some(&submitted),
        )
        .unwrap();
    assert_eq!(
        repository.load(&slug).last_run.values,
        Some(values(&[("CITY", "Osaka")]))
    );
}

#[test]
fn test_purge_secret_survives_a_last_run_values_that_is_not_a_table() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let slug = slug("broken-snapshot");
    write_state(
        &root,
        &slug,
        "[values]\nAPI_KEY = \"plaintext\"\n[last_run]\nat = \"2026-07-25T00:00:00+00:00\"\nexit = 0\nvalues = \"garbage\"\n",
    );
    let service = FormStateService::new(repository.clone());

    let removed = service.purge_secrets(&slug, &[secret("API_KEY")]).unwrap();

    assert_eq!(removed, BTreeSet::from(["API_KEY".to_owned()]));
    let state = repository.load(&slug);
    assert!(state.values.is_empty());
    assert_eq!(
        state.last_run,
        LastRunState {
            at: Some("2026-07-25T00:00:00+00:00".to_owned()),
            exit: Some(0),
            values: None,
        }
    );
}
