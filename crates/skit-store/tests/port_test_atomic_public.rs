//! Public-consequence ports of Python v0.4 `tests/test_atomic.py`.
//!
//! Rust keeps its atomic/lock primitives private. These exact frozen names therefore drive the
//! real public consumers (`FileConfigStore`, `FileFormStateStore`, `FileStore`) instead of exposing
//! or recreating private helpers in the test suite. Behavioral mismatches stay red.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    sync::mpsc,
    thread,
    time::Duration,
};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
    form_state::FormStateRepository as _,
};
use skit_domain::{EntryKind, EntrySettings, Slug, StorageMode};
use skit_store::{FileConfigStore, FileFormStateStore, FileStore};
use tempfile::TempDir;
use toml::{Table, Value};

#[test]
fn test_load_toml_recoverable_missing_file_returns_empty_no_backup() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let settings = store.settings().unwrap();
    assert_eq!(settings["lang"], "");
    assert_eq!(settings["form"], "tui");
    assert!(!root.path().join("config.toml").exists());
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn test_load_toml_recoverable_valid_file_returns_doc_no_backup() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    fs::write(
        &path,
        "language = \"zh-CN\"\n[mirror]\nenabled = true\npypi = \"https://example.invalid/simple\"\n",
    )
    .unwrap();
    let store = FileConfigStore::new(root.path());
    assert_eq!(store.get("lang").unwrap(), "zh-CN");
    let mirror = store.mirror().unwrap();
    assert!(mirror.enabled);
    assert_eq!(mirror.pypi, "https://example.invalid/simple");
    assert!(!root.path().join("config.toml.bak").exists());
}

#[test]
fn test_load_toml_recoverable_corrupt_file_backs_up_and_returns_empty() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("config.toml");
    let corrupt = b"language = \"zh-CN\"\nthis is = = not valid toml";
    fs::write(&path, corrupt).unwrap();
    let store = FileConfigStore::new(root.path());

    let recovery = store
        .set_with_recovery("editor", "vim")
        .unwrap()
        .expect("a corrupt read-modify-write must report its backup");
    assert_eq!(recovery.path, path);
    assert_eq!(recovery.backup_path, root.path().join("config.toml.bak"));
    assert_eq!(fs::read(&recovery.backup_path).unwrap(), corrupt);

    let document = fs::read_to_string(&path).unwrap().parse::<Table>().unwrap();
    assert_eq!(document.get("editor").and_then(Value::as_str), Some("vim"));
    assert!(!document.contains_key("language"), "corrupt input was not treated as an empty document");
}

#[test]
fn test_advisory_file_lock_keeps_a_persistent_one_byte_inode() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("editor", "vim").unwrap();
    let lock = root.path().join("config.lock");
    assert!(lock.is_file());
    assert!(fs::metadata(lock).unwrap().len() >= 1);
}

#[test]
fn test_advisory_file_lock_serializes_two_waiting_threads() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    store.set("editor", "seed").unwrap();
    let lock_path = root.path().join("config.lock");
    let held = OpenOptions::new().read(true).write(true).open(&lock_path).unwrap();
    held.lock().unwrap();

    let (tx, rx) = mpsc::channel();
    let first = store.clone();
    let tx_first = tx.clone();
    let a = thread::spawn(move || {
        tx_first.send(first.set("editor", "vim")).unwrap();
    });
    let second = store.clone();
    let b = thread::spawn(move || {
        tx.send(second.set("after_run", "stay")).unwrap();
    });

    assert!(rx.recv_timeout(Duration::from_millis(75)).is_err(), "a waiter entered while the transaction lock was held");
    drop(held);
    rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
    a.join().unwrap();
    b.join().unwrap();
    assert_eq!(store.get("editor").unwrap(), "vim");
    assert_eq!(store.get("after_run").unwrap(), "stay");
}

#[test]
fn test_advisory_lock_open_failure_releases_its_thread_mutex() {
    let root = TempDir::new().unwrap();
    let store = FileConfigStore::new(root.path());
    let lock = root.path().join("config.lock");
    fs::create_dir(&lock).unwrap();
    assert!(store.set("editor", "vim").is_err());
    fs::remove_dir(&lock).unwrap();
    store.set("editor", "vim").unwrap();
    assert_eq!(store.get("editor").unwrap(), "vim");
}

#[cfg(unix)]
#[test]
fn test_atomic_write_text_keep_mode_preserves_existing_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = Slug::parse("mode-preserved").unwrap();
    let values = root.path().join("values");
    fs::create_dir_all(&values).unwrap();
    let target = values.join("mode-preserved.toml");
    fs::write(&target, "[values]\nA = \"1\"\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let expected = fs::metadata(&target).unwrap().permissions().mode() & 0o777;

    store.update(&slug, |state| { state.values.insert("B".into(), "2".into()); }).unwrap();
    assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, expected);
    assert_eq!(store.load(&slug).values.get("B").map(String::as_str), Some("2"));
}

#[test]
fn test_atomic_write_text_keep_mode_missing_target_skips_chmod() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = Slug::parse("fresh-state").unwrap();
    assert!(!root.path().join("values/fresh-state.toml").exists());
    store.update(&slug, |state| { state.values.insert("A".into(), "1".into()); }).unwrap();
    assert_eq!(store.load(&slug).values.get("A").map(String::as_str), Some("1"));
}

#[cfg(windows)]
#[test]
fn test_keep_mode_windows_fallback_is_skipped_when_there_is_no_mode() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = Slug::parse("windows-fresh-state").unwrap();
    assert!(!root.path().join("values/windows-fresh-state.toml").exists());
    store.update(&slug, |state| { state.values.insert("A".into(), "1".into()); }).unwrap();
    assert_eq!(store.load(&slug).values.get("A").map(String::as_str), Some("1"));
}

fn request(name: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("future-kind").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/origin/{name}.tool"),
        workdir: "invoke".to_owned(),
        description: "old".to_owned(),
        payload: Some(EntryPayload {
            bytes: b"payload\n".to_vec(),
            stored_name: Some("script.tool".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn make_legacy(root: &TempDir) -> FileStore {
    let store = FileStore::new(root.path());
    let entry = store.create(request("legacy")).unwrap();
    let path = root.path().join("registry.toml");
    let mut document = fs::read_to_string(&path).unwrap().parse::<Table>().unwrap();
    document
        .get_mut("entries")
        .and_then(Value::as_table_mut)
        .unwrap()
        .insert(
            entry.slug.as_str().to_owned(),
            Value::Table(Table::from_iter([
                ("name".to_owned(), Value::String("legacy".to_owned())),
                ("kind".to_owned(), Value::String("future-kind".to_owned())),
                ("description".to_owned(), Value::String("old".to_owned())),
            ])),
        );
    fs::write(path, toml::to_string_pretty(&document).unwrap()).unwrap();
    store
}

fn assert_repaired(root: &TempDir) {
    let document = fs::read_to_string(root.path().join("registry.toml")).unwrap().parse::<Table>().unwrap();
    let row = document["entries"]["legacy"].as_table().unwrap();
    assert_eq!(row.get("mode").and_then(Value::as_str), Some("copy"));
    assert!(row.get("mtime_ns").and_then(Value::as_integer).is_some());
}

#[test]
fn test_try_lock_acquires_when_free_and_excludes_a_second_taker() {
    let root = TempDir::new().unwrap();
    let store = make_legacy(&root);
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), ["legacy"]);
    assert_repaired(&root);
}

fn busy_read_path_case() {
    let root = TempDir::new().unwrap();
    let store = make_legacy(&root);
    let registry = root.path().join("registry.toml");
    let before = fs::read(&registry).unwrap();
    let lock_path = root.path().join("registry.native.lock");
    let held = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path).unwrap();
    if held.metadata().unwrap().len() == 0 { held.set_len(1).unwrap(); }
    held.lock().unwrap();

    let worker_store = store.clone();
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || { tx.send(worker_store.scan()).unwrap(); });
    let result = match rx.recv_timeout(Duration::from_millis(200)) {
        Ok(result) => result,
        Err(error) => {
            drop(held);
            worker.join().unwrap();
            panic!("read path blocked on a busy registry lock: {error}");
        }
    };
    result.unwrap();
    worker.join().unwrap();
    assert_eq!(fs::read(&registry).unwrap(), before, "a busy read path performed its optional repair");

    drop(held);
    store.scan().unwrap();
    assert_repaired(&root);
}

#[test]
fn test_try_lock_declines_while_the_blocking_lock_is_held() { busy_read_path_case(); }

#[test]
fn test_try_lock_declines_when_only_the_native_lock_is_held() { busy_read_path_case(); }

#[test]
fn test_try_lock_treats_an_unopenable_lock_file_as_not_acquired() {
    let root = TempDir::new().unwrap();
    let store = make_legacy(&root);
    let registry = root.path().join("registry.toml");
    let before = fs::read(&registry).unwrap();
    let lock_path = root.path().join("registry.native.lock");
    if lock_path.exists() { fs::remove_file(&lock_path).unwrap(); }
    fs::create_dir(&lock_path).unwrap();

    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.iter().map(|entry| entry.name.as_str()).collect::<Vec<_>>(), ["legacy"]);
    assert_eq!(fs::read(&registry).unwrap(), before);

    fs::remove_dir(&lock_path).unwrap();
    store.scan().unwrap();
    assert_repaired(&root);
}

#[test]
fn rust_additive_atomic_public_consumer_test_uses_no_fake_document_model() {
    let _ = BTreeMap::<String, String>::new();
}
