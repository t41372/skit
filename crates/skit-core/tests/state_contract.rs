use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::{Arc, Barrier};
use std::thread;

use skit_core::{LibraryRoots, StateStore};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(
        root.join("data"),
        root.join("state"),
        root.join("config"),
    )
}

fn values(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn secrets_never_reach_last_values_presets_or_run_history() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let secret_names = BTreeSet::from(["TOKEN".to_owned()]);
    let accepted = values(&[("NAME", "Ada"), ("TOKEN", "sk-secret")]);

    state.save_last("demo", Some(&accepted), Some(&["--fast".to_owned()]), &secret_names)?;
    state.save_preset("demo", "daily", &accepted, &secret_names)?;
    state.record_run(
        "demo",
        0,
        "2026-08-07T20:00:00+00:00",
        Some(&accepted),
        &secret_names,
    )?;

    let loaded = state.load("demo");
    assert_eq!(loaded.values, values(&[("NAME", "Ada")]));
    assert_eq!(loaded.extra_args, vec!["--fast"]);
    assert_eq!(loaded.presets["daily"], values(&[("NAME", "Ada")]));
    assert_eq!(
        loaded.last_run.as_ref().and_then(|run| run.values.get("NAME")),
        Some(&"Ada".to_owned())
    );
    assert!(
        loaded
            .last_run
            .as_ref()
            .is_some_and(|run| !run.values.contains_key("TOKEN"))
    );

    let bytes = fs::read(state.values_path("demo"))?;
    assert!(!String::from_utf8(bytes)?.contains("sk-secret"));
    Ok(())
}

#[test]
fn empty_updates_clear_values_but_none_keeps_them() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let no_secrets = BTreeSet::new();

    state.save_last(
        "demo",
        Some(&values(&[("NAME", "Ada")])),
        Some(&["--old".to_owned()]),
        &no_secrets,
    )?;
    state.save_last("demo", None, None, &no_secrets)?;
    assert_eq!(state.load("demo").values["NAME"], "Ada");
    assert_eq!(state.load("demo").extra_args, vec!["--old"]);

    state.save_last("demo", Some(&BTreeMap::new()), Some(&[]), &no_secrets)?;
    assert!(state.load("demo").values.is_empty());
    assert!(state.load("demo").extra_args.is_empty());
    Ok(())
}

#[test]
fn purge_secret_removes_old_plaintext_from_every_value_surface() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let no_secrets = BTreeSet::new();
    let accepted = values(&[("PUBLIC", "yes"), ("TOKEN", "old-secret")]);

    state.save_last("demo", Some(&accepted), None, &no_secrets)?;
    state.save_preset("demo", "mixed", &accepted, &no_secrets)?;
    state.save_preset(
        "demo",
        "secret-only",
        &values(&[("TOKEN", "old-secret")]),
        &no_secrets,
    )?;
    state.record_run(
        "demo",
        7,
        "2026-08-07T20:00:00+00:00",
        Some(&accepted),
        &no_secrets,
    )?;

    let removed = state.purge_secret("demo", &BTreeSet::from(["TOKEN".to_owned()]))?;
    assert_eq!(removed, BTreeSet::from(["TOKEN".to_owned()]));

    let loaded = state.load("demo");
    assert!(!loaded.values.contains_key("TOKEN"));
    assert_eq!(loaded.presets["mixed"], values(&[("PUBLIC", "yes")]));
    assert!(!loaded.presets.contains_key("secret-only"));
    assert!(
        loaded
            .last_run
            .as_ref()
            .is_some_and(|run| !run.values.contains_key("TOKEN"))
    );
    assert!(!fs::read_to_string(state.values_path("demo"))?.contains("old-secret"));
    Ok(())
}

#[test]
fn corrupt_state_degrades_to_empty_instead_of_breaking_the_library() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let path = state.values_path("demo");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, b"[values\nnot toml")?;

    let loaded = state.load("demo");
    assert!(loaded.values.is_empty());
    assert!(loaded.extra_args.is_empty());
    assert!(loaded.presets.is_empty());
    assert!(loaded.last_run.is_none());
    Ok(())
}

#[test]
fn concurrent_preset_saves_do_not_drop_either_preset() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let barrier = Arc::new(Barrier::new(3));
    let no_secrets = BTreeSet::new();

    let left_store = state.clone();
    let left_barrier = Arc::clone(&barrier);
    let left_secrets = no_secrets.clone();
    let left = thread::spawn(move || {
        left_barrier.wait();
        left_store.save_preset(
            "demo",
            "left",
            &values(&[("VALUE", "left")]),
            &left_secrets,
        )
    });

    let right_store = state.clone();
    let right_barrier = Arc::clone(&barrier);
    let right = thread::spawn(move || {
        right_barrier.wait();
        right_store.save_preset(
            "demo",
            "right",
            &values(&[("VALUE", "right")]),
            &no_secrets,
        )
    });

    barrier.wait();
    left.join().map_err(|_| "left preset thread panicked")??;
    right.join().map_err(|_| "right preset thread panicked")??;

    let loaded = state.load("demo");
    assert_eq!(loaded.presets.len(), 2);
    assert_eq!(loaded.presets["left"]["VALUE"], "left");
    assert_eq!(loaded.presets["right"]["VALUE"], "right");
    Ok(())
}
