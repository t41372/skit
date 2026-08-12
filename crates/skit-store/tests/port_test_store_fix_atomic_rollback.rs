//! Atomic add rollback port for Python `test_add_python_injected_write_failure_rolls_back_entire_entry`.
//!
//! Python injects a failure after the entry directory has begun to materialize. This test creates the
//! same deterministic phase ordering through the public Rust repository: `meta.toml` is a legal safe
//! payload component, so the payload write succeeds inside `.staging`; the mandatory metadata
//! `create_new(meta.toml)` then fails. The uncommitted staged directory must be removed, no entry may
//! appear under `scripts/`, and no registry row may become resolvable.

use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_add_python_injected_write_failure_rolls_back_entire_entry() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let request = CreateEntry {
        name: "boom".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: "/original/boom.py".to_owned(),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: b"print('partial payload')\n".to_vec(),
            stored_name: Some("meta.toml".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    };

    let error = store.create(request).unwrap_err();
    assert!(
        error.to_string().contains("meta.toml") || error.to_string().contains("create"),
        "failure did not happen while materializing the staged entry: {error}"
    );

    assert!(
        store.resolve("boom").is_err(),
        "a failed staged create leaked a resolvable registry row"
    );
    assert!(
        !root.path().join("scripts/boom").exists(),
        "a failed staged create leaked the final entry directory"
    );
    let staging = root.path().join(".staging");
    if staging.exists() {
        assert!(
            fs::read_dir(&staging).unwrap().next().is_none(),
            "a failed post-payload create left user bytes in the staging root"
        );
    }
}
