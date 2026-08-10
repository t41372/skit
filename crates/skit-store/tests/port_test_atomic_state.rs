//! Public-surface ports of atomic persistence consequences from
//! `origin/main@206f9ef:tests/test_atomic.py`.
//!
//! Private syscall-spy tests are intentionally not recreated by exposing Rust internals. These
//! cases assert the same externally observable filesystem guarantees through `FileFormStateStore`.

use std::fs;

use skit_application::form_state::FormStateRepository;
use skit_domain::Slug;
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug(value: &str) -> Slug {
    Slug::parse(value.to_owned()).unwrap()
}

#[test]
fn test_atomic_write_replace_failure_removes_its_temporary_file() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("replace-fails");
    let values = root.path().join("values");
    let target = values.join("replace-fails.toml");
    fs::create_dir_all(&target).unwrap();

    assert!(
        store
            .update(&slug, |state| {
                state.values.insert("A".to_owned(), "1".to_owned());
            })
            .is_err()
    );

    let names = fs::read_dir(&values)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(names, ["replace-fails.toml"]);
    assert!(target.is_dir());
}

#[test]
fn test_failed_atomic_write_releases_the_per_slug_lock_for_the_next_update() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("retry-after-failure");
    let values = root.path().join("values");
    let target = values.join("retry-after-failure.toml");
    fs::create_dir_all(&target).unwrap();

    assert!(store.update(&slug, |_| ()).is_err());
    assert!(root
        .path()
        .join(".locks")
        .join("retry-after-failure.values.lock")
        .is_file());

    fs::remove_dir(&target).unwrap();
    store
        .update(&slug, |state| {
            state.values.insert("A".to_owned(), "1".to_owned());
        })
        .unwrap();

    assert_eq!(store.load(&slug).values.get("A").map(String::as_str), Some("1"));
}

#[cfg(unix)]
#[test]
fn test_atomic_write_preserves_existing_state_file_mode() {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug("mode-preserved");
    let values = root.path().join("values");
    fs::create_dir_all(&values).unwrap();
    let target = values.join("mode-preserved.toml");
    fs::write(&target, "[values]\nA = \"1\"\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    let before = fs::metadata(&target).unwrap().mode() & 0o777;

    store
        .update(&slug, |state| {
            state.values.insert("B".to_owned(), "2".to_owned());
        })
        .unwrap();

    assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, before);
    assert_eq!(store.load(&slug).values.get("B").map(String::as_str), Some("2"));
}
