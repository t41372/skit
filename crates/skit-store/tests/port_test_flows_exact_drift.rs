//! Exact launch-drift port from Python v0.4 `tests/test_flows.py`.
//!
//! Rust closes the injection race at the store launch transaction. The frozen behavior still
//! requires a drift classification that points the user at resynchronization.

use std::{fs, path::Path};

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, ExitClass,
    RepositoryError, RepositoryOperation, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

const MANAGED: &str = concat!(
    "# /// script\n",
    "# [tool.skit]\n",
    "# schema = 1\n",
    "# [[tool.skit.params]]\n",
    "# name = \"WIDTH\"\n",
    "# kind = \"const\"\n",
    "# type = \"int\"\n",
    "# default = 800\n",
    "# ///\n",
    "WIDTH = 800\n",
    "print(WIDTH)\n",
);

#[test]
fn test_execute_classifies_injection_drift() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let kind = EntryKind::parse("python").unwrap();
    let entry = store
        .create(CreateEntry {
            name: "drift-run".to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: "drift.py".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: MANAGED.as_bytes().to_vec(),
                stored_name: Some(payload_stored_name(&kind, Path::new("drift.py"))),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let expected = entry.meta.source_hash.clone();
    assert!(!expected.is_empty(), "the launch transaction fixture must start from a pinned source hash");
    let payload = store.payload_path(&entry).unwrap();
    fs::write(&payload, MANAGED.replace("WIDTH = 800", "WIDTH = 900")).unwrap();

    let error = match store.prepare_launch(&entry, Some(&expected)) {
        Ok(_) => panic!("a source mutation after form assembly was allowed to launch"),
        Err(error) => error,
    };
    assert!(
        matches!(&error, RepositoryError::SourceChanged { slug, .. } if slug == "drift-run"),
        "launch drift was classified as the wrong repository error: {error}"
    );
    assert_eq!(error.exit_class(RepositoryOperation::Launch), ExitClass::Skit);
    assert!(
        error.to_string().contains("resync"),
        "the frozen drift outcome must point at the fix instead of reporting a generic launch failure: {error}"
    );
}
