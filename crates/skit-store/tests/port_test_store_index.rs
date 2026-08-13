//! Registry/list-summary ports from Python v0.4 `tests/test_store.py`.
//!
//! Hand-edited registry fixtures are intentional user-input fixtures, not replicas of Rust private
//! helpers. All behavior is observed through public `FileStore::scan`, `scan_entries`, `resolve`,
//! and mutation traits. Python's read-side repair and mtime-only freshness contracts stay frozen;
//! a stronger/different Rust cache proof is an implementation difference, not a reason to weaken
//! these tests.

use std::{fs, path::Path};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(name: &str, kind: &str, mode: StorageMode, source: &str, description: &str) -> CreateEntry {
    let stored_name = match kind {
        "python" => Some("script.py"),
        "shell" => Some("script.sh"),
        _ => None,
    };
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        source: source.to_owned(),
        workdir: if mode == StorageMode::Reference { "origin" } else { "invoke" }.to_owned(),
        description: description.to_owned(),
        payload: stored_name.map(|stored_name| EntryPayload {
            bytes: b"print('payload')\n".to_vec(),
            stored_name: Some(stored_name.to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn registry_path(root: &TempDir) -> std::path::PathBuf {
    root.path().join("registry.toml")
}

fn registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(registry_path(root)).unwrap()).unwrap()
}

fn write_registry(root: &TempDir, document: &Table) {
    fs::write(registry_path(root), toml::to_string_pretty(document).unwrap()).unwrap();
}

fn rows_mut(document: &mut Table) -> &mut Table {
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .expect("registry entries table")
}

fn legacy_row(name: &str, kind: &str, description: &str) -> Value {
    Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String(name.to_owned())),
        ("kind".to_owned(), Value::String(kind.to_owned())),
        ("description".to_owned(), Value::String(description.to_owned())),
    ]))
}

#[test]
fn test_summaries_match_full_entries_field_for_field() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let reference = root.path().join("linked.py");
    fs::write(&reference, b"print('linked')\n").unwrap();
    store.create(request("copied", "python", StorageMode::Copy, "/origin/copied.py", "a copy")).unwrap();
    store.create(request("linked", "python", StorageMode::Reference, &reference.display().to_string(), "linked")).unwrap();
    store.create(request("templated", "command", StorageMode::Reference, "", "no file")).unwrap();

    let by_slug = store
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.slug.clone(), entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut summaries = store.scan().unwrap().entries;
    summaries.sort_by(|left, right| left.slug.cmp(&right.slug));
    assert_eq!(summaries.iter().map(|summary| &summary.slug).collect::<Vec<_>>(), by_slug.keys().collect::<Vec<_>>());
    for summary in summaries {
        let entry = &by_slug[&summary.slug];
        assert_eq!(summary.name, entry.meta.name);
        assert_eq!(summary.kind, entry.meta.kind);
        assert_eq!(summary.mode, entry.meta.mode);
        assert_eq!(summary.description, entry.meta.description);
        assert_eq!(
            summary.target.as_deref(),
            (entry.meta.mode == StorageMode::Reference).then_some(entry.meta.source.as_str())
        );
    }
}

#[test]
fn test_summaries_serve_from_the_index_without_parsing_metas() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("one", "python", StorageMode::Copy, "/origin/one.py", "first")).unwrap();
    store.create(request("two", "command", StorageMode::Reference, "", "second")).unwrap();
    let expected = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|summary| (summary.slug, summary.name, summary.description))
        .collect::<Vec<_>>();

    for entry in store.scan_entries().unwrap() {
        let meta = root.path().join("scripts").join(entry.slug.as_str()).join("meta.toml");
        let metadata = fs::metadata(&meta).unwrap();
        let modified = metadata.modified().unwrap();
        fs::write(&meta, "not [ toml").unwrap();
        fs::File::open(&meta)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
    }

    let actual = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|summary| (summary.slug, summary.name, summary.description))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "a steady-state listing parsed metadata despite the frozen mtime");
    assert!(store.scan_entries().unwrap().is_empty(), "the authoritative meta reader should see only corruption");
}

#[test]
fn test_a_row_an_older_skit_wrote_falls_back_to_its_meta() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("old.py");
    fs::write(&source, "print(1)\n").unwrap();
    let entry = store.create(request("old", "python", StorageMode::Reference, &source.display().to_string(), "")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc).insert(entry.slug.as_str().to_owned(), legacy_row("old", "python", ""));
    write_registry(&root, &doc);

    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.mode, StorageMode::Reference);
    assert_eq!(summary.target.as_deref(), Some(source.to_str().unwrap()));

    let repaired = registry(&root);
    let row = repaired["entries"][entry.slug.as_str()].as_table().unwrap();
    assert_eq!(row.get("mode").and_then(Value::as_str), Some("reference"));
    assert_eq!(row.get("target").and_then(Value::as_str), Some(source.to_str().unwrap()));
    assert!(row.get("mtime_ns").and_then(Value::as_integer).is_some());
}

#[test]
fn test_a_hand_broken_row_falls_back_instead_of_inventing_a_summary() {
    for broken in [
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::String("x".to_owned())),
            ("kind".to_owned(), Value::String("python".to_owned())),
            ("description".to_owned(), Value::Integer(7)),
        ])),
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::String("x".to_owned())),
            ("kind".to_owned(), Value::String("python".to_owned())),
            ("mode".to_owned(), Value::String("sideways".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
        ])),
        Value::Table(Table::from_iter([
            ("kind".to_owned(), Value::String("python".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
        ])),
        Value::Table(Table::from_iter([
            ("name".to_owned(), Value::String("x".to_owned())),
            ("kind".to_owned(), Value::String("python".to_owned())),
            ("mode".to_owned(), Value::String("reference".to_owned())),
            ("description".to_owned(), Value::String(String::new())),
            ("target".to_owned(), Value::Integer(7)),
        ])),
    ] {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store.create(request("real", "python", StorageMode::Copy, "/origin/real.py", "the truth")).unwrap();
        let mut doc = registry(&root);
        rows_mut(&mut doc).insert(entry.slug.as_str().to_owned(), broken);
        write_registry(&root, &doc);
        let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
        assert_eq!(summary.name, "real");
        assert_eq!(summary.description, "the truth");
    }
}

#[test]
fn rust_additive_store_broken_row_non_string_field_falls_back() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("real", "python", StorageMode::Copy, "/origin/real.py", "the truth")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc).insert(entry.slug.as_str().to_owned(), Value::Table(Table::from_iter([
        ("name".to_owned(), Value::String("x".to_owned())),
        ("kind".to_owned(), Value::String("python".to_owned())),
        ("description".to_owned(), Value::Integer(7)),
    ])));
    write_registry(&root, &doc);
    assert_eq!(store.scan().unwrap().entries[0].description, "the truth");
}

#[test]
fn test_a_broken_row_over_a_corrupt_meta_is_skipped_like_list_entries() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("doomed", "python", StorageMode::Copy, "/origin/doomed.py", "")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc).insert(entry.slug.as_str().to_owned(), legacy_row("doomed", "python", ""));
    write_registry(&root, &doc);
    fs::write(root.path().join("scripts/doomed/meta.toml"), "not [ toml").unwrap();
    assert!(store.scan().unwrap().entries.is_empty());
    assert!(store.scan_entries().unwrap().is_empty());
}

#[test]
fn test_rename_and_describe_keep_the_index_in_step() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("before", "python", StorageMode::Copy, "/origin/before.py", "old text")).unwrap();
    let described = store.describe(&entry, "new text").unwrap();
    let renamed = store.rename(&described, "after").unwrap();
    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!((summary.name.as_str(), summary.description.as_str()), ("after", "new text"));
    assert_eq!(summary.slug, renamed.slug);
}

#[test]
fn test_an_older_registry_is_widened_the_first_time_it_is_listed() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("legacy", "python", StorageMode::Copy, "/origin/legacy.py", "old row")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc).insert(entry.slug.as_str().to_owned(), legacy_row("legacy", "python", "old row"));
    write_registry(&root, &doc);

    assert_eq!(store.scan().unwrap().entries.len(), 1);

    let after = registry(&root);
    let row = after["entries"][entry.slug.as_str()].as_table().unwrap();
    assert_eq!(row.get("mode").and_then(Value::as_str), Some("copy"));
    assert!(row.get("mtime_ns").and_then(Value::as_integer).is_some());
}

#[test]
fn test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("doomed", "python", StorageMode::Copy, "/origin/doomed.py", "")).unwrap();
    let bad = b"entries = [ this is not toml";
    fs::write(registry_path(&root), bad).unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    let quarantine = root.path().join("registry.toml.corrupt");
    assert!(quarantine.is_file(), "corrupt registry was not quarantined by the read path");
    assert_eq!(fs::read(quarantine).unwrap(), bad);
    assert!(root.path().join("scripts/doomed/meta.toml").is_file());
}

#[test]
fn test_a_corrupted_meta_drops_out_of_the_listing_like_every_other_face() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("broken", "python", StorageMode::Copy, "/origin/broken.py", "")).unwrap();
    store.create(request("fine", "command", StorageMode::Reference, "", "")).unwrap();
    fs::write(root.path().join("scripts/broken/meta.toml"), "not [ toml").unwrap();
    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.iter().map(|summary| summary.name.as_str()).collect::<Vec<_>>(), ["fine"]);
    assert_eq!(store.scan_entries().unwrap().iter().map(|entry| entry.meta.name.as_str()).collect::<Vec<_>>(), ["fine"]);
}

#[test]
fn test_a_non_mapping_row_falls_back_instead_of_crashing() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("real", "python", StorageMode::Copy, "/origin/real.py", "the truth")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc).insert(entry.slug.as_str().to_owned(), Value::String("oops".to_owned()));
    write_registry(&root, &doc);
    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!((summary.name.as_str(), summary.description.as_str()), ("real", "the truth"));
}

#[test]
fn test_resolve_survives_a_hand_broken_row() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("real", "python", StorageMode::Copy, "/origin/real.py", "")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc).insert("stray".to_owned(), Value::String("oops".to_owned()));
    write_registry(&root, &doc);
    assert_eq!(store.resolve("real").unwrap().slug, entry.slug);
    assert_eq!(store.resolve(entry.slug.as_str()).unwrap().slug, entry.slug);
    assert!(store.resolve("stray").is_err());
}

#[test]
fn test_a_hand_edited_meta_shows_up_on_the_next_listing() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("job", "python", StorageMode::Copy, "/origin/job.py", "the old text")).unwrap();
    assert_eq!(store.scan().unwrap().entries[0].description, "the old text");

    let meta_path = root.path().join("scripts/job/meta.toml");
    let mut meta: Table = toml::from_str(&fs::read_to_string(&meta_path).unwrap()).unwrap();
    meta.insert("description".to_owned(), Value::String("edited by hand".to_owned()));
    fs::write(&meta_path, toml::to_string_pretty(&meta).unwrap()).unwrap();

    assert_eq!(store.scan().unwrap().entries[0].description, "edited by hand");
    let after = registry(&root);
    assert_eq!(
        after["entries"][entry.slug.as_str()]["description"].as_str(),
        Some("edited by hand")
    );
    assert_eq!(store.scan().unwrap().entries[0].description, "edited by hand");
}

#[test]
fn test_a_reference_row_that_lost_its_target_is_repaired_once() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("linked.py");
    fs::write(&source, "print(1)\n").unwrap();
    let entry = store.create(request("linked", "python", StorageMode::Reference, &source.display().to_string(), "")).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc)
        .get_mut(entry.slug.as_str())
        .and_then(Value::as_table_mut)
        .unwrap()
        .remove("target");
    write_registry(&root, &doc);

    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.target.as_deref(), Some(source.to_str().unwrap()));
    let repaired = registry(&root);
    assert_eq!(repaired["entries"][entry.slug.as_str()]["target"].as_str(), Some(source.to_str().unwrap()));
    let before = fs::read(registry_path(&root)).unwrap();
    assert_eq!(store.scan().unwrap().entries[0].target.as_deref(), Some(source.to_str().unwrap()));
    assert_eq!(fs::read(registry_path(&root)).unwrap(), before, "a converged listing rewrote the index again");
}

#[test]
fn test_an_emptied_target_on_a_file_kind_falls_back_to_the_meta() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("orig.py");
    fs::write(&source, "print(1)\n").unwrap();
    let entry = store.create(request("linked", "python", StorageMode::Reference, &source.display().to_string(), "")).unwrap();
    fs::remove_file(&source).unwrap();
    let mut doc = registry(&root);
    rows_mut(&mut doc)
        .get_mut(entry.slug.as_str())
        .and_then(Value::as_table_mut)
        .unwrap()
        .insert("target".to_owned(), Value::String(String::new()));
    write_registry(&root, &doc);

    let summary = store.scan().unwrap().entries.into_iter().next().unwrap();
    assert_eq!(summary.target.as_deref(), Some(source.to_str().unwrap()));
    assert!(!Path::new(summary.target.as_deref().unwrap()).exists());
}
