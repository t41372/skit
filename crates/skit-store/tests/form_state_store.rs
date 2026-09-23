use std::{
    collections::BTreeMap,
    fs,
    sync::{Arc, Barrier},
    thread,
};

use skit_application::form_state::{FormStateRepository, LastRunState, PersistedFormState};
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
fn narrow_last_run_read_returns_only_the_listing_stamp() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(
        state_path(&root),
        r#"
[values]
city = "Taipei"

[presets.large]
first = "one"
second = "two"

[last_run]
at = "2026-08-08T00:00:00Z"
exit = 9

[last_run.values]
city = "Tainan"
"#,
    )
    .unwrap();

    assert_eq!(
        FileFormStateStore::new(root.path()).last_run(&slug()),
        LastRunState {
            at: Some("2026-08-08T00:00:00Z".to_owned()),
            exit: Some(9),
            values: None,
        }
    );
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
extra_args = "wrong-shape"
extra_args_raw = "yes"

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
    assert!(state.extra_args.is_empty());
    assert!(!state.extra_args_raw);
    assert_eq!(
        state.presets,
        BTreeMap::from([(
            "good".to_owned(),
            BTreeMap::from([("city".to_owned(), "Paris".to_owned())]),
        )])
    );
    assert_eq!(state.last_run.at.as_deref(), Some("2026-08-07T17:22:00Z"));
    assert_eq!(state.last_run.exit, Some(0));
    assert!(state.last_run.values.is_none());
}

#[test]
fn update_owns_the_complete_schema_and_preserves_only_unknown_extension_fields() {
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
            state.extra_args = vec!["--fresh".to_owned(), "two words".to_owned()];
            state.extra_args_raw = false;
            state
                .presets
                .insert("travel".to_owned(), state.values.clone());
            state.last_run.at = Some("2026-08-08T01:30:00Z".to_owned());
            state.last_run.exit = Some(0);
            state.last_run.values = Some(state.values.clone());
        })
        .unwrap();

    let text = fs::read_to_string(state_path(&root)).unwrap();
    let doc: toml::Table = toml::from_str(&text).unwrap();
    assert_eq!(doc["future"].as_str(), Some("keep-me"));
    assert!(doc.get("extra_args_raw").is_none());
    assert_eq!(doc["extra_args"].as_array().unwrap().len(), 2);
    assert_eq!(
        doc["extra_args"].as_array().unwrap()[0].as_str(),
        Some("--fresh")
    );
    assert_eq!(
        doc["extra_args"].as_array().unwrap()[1].as_str(),
        Some("two words")
    );
    assert_eq!(doc["values"]["city"].as_str(), Some("Tokyo"));
    assert_eq!(doc["presets"]["travel"]["city"].as_str(), Some("Tokyo"));
    assert_eq!(doc["last_run"]["at"].as_str(), Some("2026-08-08T01:30:00Z"));
    assert_eq!(doc["last_run"]["exit"].as_integer(), Some(0));
    assert_eq!(doc["last_run"]["values"]["city"].as_str(), Some("Tokyo"));
    assert_eq!(doc["last_run"]["future_nested"].as_str(), Some("keep-too"));

    let round_trip = store.load(&slug());
    assert_eq!(round_trip.values["city"], "Tokyo");
    assert_eq!(round_trip.extra_args, ["--fresh", "two words"]);
    assert!(!round_trip.extra_args_raw);
    assert_eq!(round_trip.presets["travel"]["city"], "Tokyo");
    assert_eq!(
        round_trip.last_run.at.as_deref(),
        Some("2026-08-08T01:30:00Z")
    );
    assert_eq!(round_trip.last_run.exit, Some(0));
    assert_eq!(
        round_trip.last_run.values.as_ref().unwrap()["city"],
        "Tokyo"
    );
}

#[test]
fn read_modify_write_preserves_unrelated_future_leaf_values() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(
        state_path(&root),
        r#"
extra_args = ["--known", 17, true]

[values]
city = "Paris"
future_number = 42
future_array = [1, 2]

[presets.existing]
city = "Paris"
future_flag = true

[last_run]
at = "2026-08-07T17:22:00Z"
exit = 7
future_stamp = 99

[last_run.values]
city = "Paris"
future_table = { enabled = true }
"#,
    )
    .unwrap();

    let store = FileFormStateStore::new(root.path());
    store
        .update(&slug(), |state| {
            state.presets.insert(
                "new".to_owned(),
                BTreeMap::from([("city".to_owned(), "Tokyo".to_owned())]),
            );
        })
        .unwrap();

    let document: toml::Table =
        toml::from_str(&fs::read_to_string(state_path(&root)).unwrap()).unwrap();
    assert_eq!(document["extra_args"].as_array().unwrap().len(), 3);
    assert_eq!(document["values"]["future_number"].as_integer(), Some(42));
    assert!(document["values"]["future_array"].is_array());
    assert_eq!(
        document["presets"]["existing"]["future_flag"].as_bool(),
        Some(true)
    );
    assert_eq!(document["last_run"]["future_stamp"].as_integer(), Some(99));
    assert_eq!(
        document["last_run"]["values"]["future_table"]["enabled"].as_bool(),
        Some(true)
    );
    assert_eq!(document["presets"]["new"]["city"].as_str(), Some("Tokyo"));
}

#[test]
fn changing_known_keys_keeps_unknown_sibling_leaves() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(
        state_path(&root),
        r#"
[values]
city = "Paris"
future_number = 42

[presets.travel]
city = "Paris"
future_flag = true

[last_run]
at = "old"
exit = 7

[last_run.values]
city = "Paris"
future_array = [1, 2]
"#,
    )
    .unwrap();

    FileFormStateStore::new(root.path())
        .update(&slug(), |state| {
            state.values.insert("city".to_owned(), "Tokyo".to_owned());
            state.presets.get_mut("travel").unwrap().remove("city");
            state.last_run.at = Some("new".to_owned());
            state
                .last_run
                .values
                .as_mut()
                .unwrap()
                .insert("city".to_owned(), "Tokyo".to_owned());
        })
        .unwrap();

    let document: toml::Table =
        toml::from_str(&fs::read_to_string(state_path(&root)).unwrap()).unwrap();
    assert_eq!(document["values"]["city"].as_str(), Some("Tokyo"));
    assert_eq!(document["values"]["future_number"].as_integer(), Some(42));
    assert!(document["presets"]["travel"].get("city").is_none());
    assert_eq!(
        document["presets"]["travel"]["future_flag"].as_bool(),
        Some(true)
    );
    assert_eq!(document["last_run"]["at"].as_str(), Some("new"));
    assert_eq!(
        document["last_run"]["values"]["future_array"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn clearing_the_tail_also_clears_its_raw_provenance_marker() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    store
        .update(&slug(), |state| {
            state.extra_args = vec!["{today}".to_owned()];
            state.extra_args_raw = true;
        })
        .unwrap();
    store
        .update(&slug(), |state| {
            state.extra_args.clear();
        })
        .unwrap();

    let doc: toml::Table = toml::from_str(&fs::read_to_string(state_path(&root)).unwrap()).unwrap();
    assert!(doc.get("extra_args").is_none());
    assert!(doc.get("extra_args_raw").is_none());
    assert!(store.load(&slug()).extra_args.is_empty());
    assert!(!store.load(&slug()).extra_args_raw);
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
        assert_eq!(
            state.presets[&format!("preset-{index}")]["value"],
            index.to_string()
        );
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

#[test]
fn clearing_last_run_fields_removes_known_rows_and_an_empty_table() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("values")).unwrap();
    fs::write(
        state_path(&root),
        "[last_run]\nat = \"now\"\nexit = 7\n[last_run.values]\nname = \"Ada\"\n",
    )
    .unwrap();
    let store = FileFormStateStore::new(root.path());
    store
        .update(&slug(), |state| {
            state.last_run.at = None;
            state.last_run.exit = None;
            state.last_run.values = None;
        })
        .unwrap();

    let document: toml::Table =
        toml::from_str(&fs::read_to_string(state_path(&root)).unwrap()).unwrap();
    assert!(!document.contains_key("last_run"));
}
