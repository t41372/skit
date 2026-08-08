use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_application::form_state::{FormStateRepository, PersistedFormState};
use skit_domain::Slug;
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug() -> Slug {
    Slug::parse("demo").unwrap()
}

fn state_path(root: &TempDir) -> std::path::PathBuf {
    root.path().join("values/demo.toml")
}

#[test]
fn missing_and_corrupt_state_degrade_to_empty_without_rewriting_the_file() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    assert_eq!(store.load(&slug()), PersistedFormState::default());

    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(state_path(&root), b"[values\nnot valid toml").unwrap();
    let before = fs::read(state_path(&root)).unwrap();

    assert_eq!(store.load(&slug()), PersistedFormState::default());
    assert_eq!(fs::read(state_path(&root)).unwrap(), before);
}

#[test]
fn load_isolates_malformed_sections_and_keeps_valid_siblings() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(
        state_path(&root),
        r#"
future = "keep"
values = "wrong-shape"

[presets]
bad = "wrong-shape"

[presets.good]
city = "Paris"

[last_run]
at = "2026-08-07T17:22:00Z"
exit = 0
values = "wrong-shape"
"#,
    )
    .unwrap();

    let state = FileFormStateStore::new(root.path()).load(&slug());

    assert!(state.values.is_empty());
    assert_eq!(
        state.presets,
        BTreeMap::from([(
            "good".to_owned(),
            BTreeMap::from([("city".to_owned(), "Paris".to_owned())]),
        )])
    );
    assert!(state.last_run_values.is_empty());
}

#[test]
fn update_preserves_unknown_fields_and_unported_last_run_metadata() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(
        state_path(&root),
        r#"
future = "keep-me"
extra_args = ["--old"]
extra_args_raw = true

[values]
city = "Paris"

[last_run]
at = "2026-08-07T17:22:00Z"
exit = 7
future_nested = "keep-too"

[last_run.values]
city = "Paris"
"#,
    )
    .unwrap();

    let store = FileFormStateStore::new(root.path());
    store
        .update(&slug(), |state| {
            state.values.insert("city".to_owned(), "Tokyo".to_owned());
            state
                .presets
                .insert("travel".to_owned(), state.values.clone());
            state.last_run_values.clear();
        })
        .unwrap();

    let text = fs::read_to_string(state_path(&root)).unwrap();
    let doc: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(doc["future"].as_str(), Some("keep-me"));
    assert_eq!(doc["extra_args_raw"].as_bool(), Some(true));
    assert_eq!(doc["extra_args"].as_array().unwrap()[0].as_str(), Some("--old"));
    assert_eq!(doc["values"]["city"].as_str(), Some("Tokyo"));
    assert_eq!(doc["presets"]["travel"]["city"].as_str(), Some("Tokyo"));
    assert_eq!(doc["last_run"]["at"].as_str(), Some("2026-08-07T17:22:00Z"));
    assert_eq!(doc["last_run"]["exit"].as_integer(), Some(7));
    assert_eq!(doc["last_run"]["future_nested"].as_str(), Some("keep-too"));
    assert!(doc["last_run"].get("values").is_none());
}

#[test]
fn concurrent_read_modify_write_updates_do_not_drop_sibling_presets() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FileFormStateStore::new(root.path()));
    let barrier = Arc::new(Barrier::new(16));
    let slug = slug();

    let handles = (0..16)
        .map(|index| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            let slug = slug.clone();
            thread::spawn(move || {
                barrier.wait();
                store
                    .update(&slug, |state| {
                        state.presets.insert(
                            format!("preset-{index}"),
                            BTreeMap::from([("value".to_owned(), index.to_string())]),
                        );
                    })
                    .unwrap();
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        handle.join().unwrap();
    }

    let state = store.load(&slug);
    assert_eq!(state.presets.len(), 16);
    for index in 0..16 {
        assert_eq!(state.presets[&format!("preset-{index}")]["value"], index.to_string());
    }
}

#[test]
fn state_write_failures_are_typed_and_leave_no_partial_document() {
    let root = TempDir::new().unwrap();
    let blocked = root.path().join("not-a-directory");
    fs::write(&blocked, b"file").unwrap();
    let store = FileFormStateStore::new(&blocked);

    let error = store
        .update(&slug(), |state| {
            state.values.insert("city".to_owned(), "Paris".to_owned());
        })
        .unwrap_err();

    assert!(error.to_string().contains("state"));
    assert_eq!(fs::read(&blocked).unwrap(), b"file");
}
