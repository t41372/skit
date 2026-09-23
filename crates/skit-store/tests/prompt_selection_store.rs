use std::{fs, sync::Arc, thread};

use skit_application::prompt_selection::PromptSelectionStore;
use skit_store::FilePromptSelectionStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn path(root: &TempDir) -> std::path::PathBuf {
    root.path().join("prompt.toml")
}

#[test]
fn absent_corrupt_and_wrong_typed_state_degrade_without_rewriting() {
    let root = TempDir::new().unwrap();
    let store = FilePromptSelectionStore::new(root.path());
    assert_eq!(store.load_last_runner(), "");
    assert!(!path(&root).exists());

    fs::write(path(&root), b"not = [toml").unwrap();
    let corrupt = fs::read(path(&root)).unwrap();
    assert_eq!(store.load_last_runner(), "");
    assert_eq!(fs::read(path(&root)).unwrap(), corrupt);

    fs::write(path(&root), b"last_runner = 3\n").unwrap();
    assert_eq!(store.load_last_runner(), "");
}

#[test]
fn save_is_atomic_and_preserves_unknown_toml() {
    let root = TempDir::new().unwrap();
    fs::write(
        path(&root),
        b"future = \"keep\"\nlast_runner = \"old\"\n\n[nested]\nvalue = 7\n",
    )
    .unwrap();
    let store = FilePromptSelectionStore::new(root.path());

    store.save_last_runner("codex").unwrap();

    assert_eq!(store.load_last_runner(), "codex");
    let document = fs::read_to_string(path(&root))
        .unwrap()
        .parse::<Table>()
        .unwrap();
    assert_eq!(document.get("future").and_then(Value::as_str), Some("keep"));
    assert_eq!(
        document
            .get("nested")
            .and_then(Value::as_table)
            .and_then(|table| table.get("value"))
            .and_then(Value::as_integer),
        Some(7)
    );
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn lock_or_replace_failure_leaves_existing_state_intact_and_cleans_temporary_files() {
    let lock_failure = TempDir::new().unwrap();
    fs::write(path(&lock_failure), b"last_runner = \"old\"\n").unwrap();
    fs::write(lock_failure.path().join(".locks"), b"not a directory").unwrap();
    let store = FilePromptSelectionStore::new(lock_failure.path());
    assert!(store.save_last_runner("new").is_err());
    assert_eq!(
        fs::read(path(&lock_failure)).unwrap(),
        b"last_runner = \"old\"\n"
    );

    let replace_failure = TempDir::new().unwrap();
    fs::create_dir(path(&replace_failure)).unwrap();
    fs::write(path(&replace_failure).join("sentinel"), b"keep").unwrap();
    let store = FilePromptSelectionStore::new(replace_failure.path());
    assert!(store.save_last_runner("new").is_err());
    assert_eq!(
        fs::read(path(&replace_failure).join("sentinel")).unwrap(),
        b"keep"
    );
    assert!(fs::read_dir(replace_failure.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn concurrent_saves_publish_one_complete_parseable_document() {
    let root = TempDir::new().unwrap();
    let store = Arc::new(FilePromptSelectionStore::new(root.path()));
    let threads = (0..8)
        .map(|index| {
            let store = Arc::clone(&store);
            thread::spawn(move || store.save_last_runner(&format!("runner-{index}")).unwrap())
        })
        .collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }

    let value = store.load_last_runner();
    assert!((0..8).any(|index| value == format!("runner-{index}")));
    fs::read_to_string(path(&root))
        .unwrap()
        .parse::<Table>()
        .unwrap();
}
