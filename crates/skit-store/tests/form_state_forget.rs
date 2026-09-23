use std::fs;

use skit_application::form_state::{FormStateRepository, FormStateService};
use skit_domain::Slug;
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug() -> Slug {
    Slug::parse("demo").unwrap()
}

#[test]
fn forget_removes_the_values_file_and_is_idempotent() {
    let root = TempDir::new().unwrap();
    let store = FileFormStateStore::new(root.path());
    let slug = slug();

    store
        .update(&slug, |state| {
            state.values.insert("city".to_owned(), "Paris".to_owned());
        })
        .unwrap();
    let path = root.path().join("values/demo.toml");
    assert!(path.is_file());

    let service = FormStateService::new(store);
    service.forget(&slug).unwrap();
    assert!(!path.exists());
    assert!(root.path().join(".locks/demo.values.lock").is_file());

    service.forget(&slug).unwrap();
    assert!(!path.exists());
    assert_eq!(service.load(&slug).values.len(), 0);
}

#[test]
fn forget_write_failures_are_typed_and_do_not_touch_the_blocking_file() {
    let root = TempDir::new().unwrap();
    let blocked = root.path().join("not-a-directory");
    fs::write(&blocked, b"file").unwrap();
    let service = FormStateService::new(FileFormStateStore::new(&blocked));

    let error = service.forget(&slug()).unwrap_err();

    assert!(error.to_string().contains("state"));
    assert_eq!(fs::read(&blocked).unwrap(), b"file");
}

#[test]
fn forget_reports_a_values_path_that_is_not_a_regular_file() {
    let root = TempDir::new().unwrap();
    let path = root.path().join("values/demo.toml");
    fs::create_dir_all(&path).unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));

    assert!(service.forget(&slug()).is_err());
    assert!(path.is_dir());
}
