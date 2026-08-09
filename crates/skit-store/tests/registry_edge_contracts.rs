use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions, UpdateEntry,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;
use toml::{Table, Value};

fn request(
    name: &str,
    kind: &str,
    mode: StorageMode,
    stored_name: Option<&str>,
    bytes: &[u8],
) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        source: format!("/original/{}", stored_name.unwrap_or("metadata")),
        workdir: if mode == StorageMode::Reference {
            "origin"
        } else {
            "invoke"
        }
        .to_owned(),
        description: format!("description for {name}"),
        payload: stored_name.map(|stored_name| EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some(stored_name.to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn read_registry(root: &TempDir) -> Table {
    toml::from_str(
        &fs::read_to_string(root.path().join("registry.toml")).expect("registry should exist"),
    )
    .expect("registry should stay valid TOML")
}

#[cfg(unix)]
#[test]
fn a_late_registry_save_failure_rolls_back_every_mutation_shape() {
    use std::os::unix::fs::PermissionsExt as _;

    struct RestoreMode {
        path: std::path::PathBuf,
        mode: u32,
    }

    impl Drop for RestoreMode {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.path, fs::Permissions::from_mode(self.mode));
        }
    }

    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let describe = store
        .create(request(
            "Describe",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"describe",
        ))
        .unwrap();
    let rename = store
        .create(request(
            "Rename",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"rename",
        ))
        .unwrap();
    let remove = store
        .create(request(
            "Remove",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"remove",
        ))
        .unwrap();
    let edit = store
        .create(request(
            "Edit",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"base",
        ))
        .unwrap();
    let combined = store
        .create(request(
            "Combined",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"combined",
        ))
        .unwrap();

    let describe = store.claim_identity(&describe).unwrap();
    let rename = store.claim_identity(&rename).unwrap();
    let remove = store.claim_identity(&remove).unwrap();
    let edit = store.claim_identity(&edit).unwrap();
    let combined = store.claim_identity(&combined).unwrap();
    fs::create_dir_all(root.path().join(".trash")).unwrap();

    let original_mode = fs::metadata(root.path()).unwrap().permissions().mode() & 0o777;
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o555)).unwrap();
    let restore = RestoreMode {
        path: root.path().to_path_buf(),
        mode: original_mode,
    };

    assert!(
        store
            .create(request(
                "Create",
                "python",
                StorageMode::Copy,
                Some("script.py"),
                b"create",
            ))
            .is_err()
    );
    assert!(!root.path().join("scripts/create").exists());

    assert!(store.describe(&describe, "after").is_err());
    assert_eq!(
        store.resolve("describe").unwrap().meta.description,
        "description for Describe"
    );

    assert!(store.rename(&rename, "After Rename").is_err());
    assert!(root.path().join("scripts/rename").is_dir());
    assert!(!root.path().join("scripts/after-rename").exists());
    assert_eq!(store.resolve("rename").unwrap().meta.name, "Rename");

    assert!(store.remove(&remove).is_err());
    assert!(store.resolve("remove").is_ok());

    assert!(
        store
            .commit_copy_edit(&edit, b"next", &edit.meta.source_hash)
            .is_err()
    );
    assert_eq!(
        fs::read(root.path().join("scripts/edit/script.py")).unwrap(),
        b"base"
    );
    assert_eq!(
        store.resolve("edit").unwrap().meta.source_hash,
        edit.meta.source_hash
    );

    assert!(
        store
            .update_entry(
                &combined,
                UpdateEntry {
                    name: "Combined after".to_owned(),
                    description: "after".to_owned(),
                    settings: EntrySettings::default(),
                    workdir: "store".to_owned(),
                    source: Some(b"after".to_vec()),
                    expected_source_hash: combined.meta.source_hash.clone(),
                },
            )
            .is_err()
    );
    assert_eq!(
        fs::read(root.path().join("scripts/combined/script.py")).unwrap(),
        b"combined"
    );
    let restored = store.resolve("combined").unwrap();
    assert_eq!(restored.meta.name, "Combined");
    assert_eq!(restored.meta.description, "description for Combined");
    assert_eq!(restored.meta.source_hash, combined.meta.source_hash);

    drop(restore);
}

#[cfg(unix)]
#[test]
fn incomplete_remove_is_reported_and_keeps_rebuildable_files() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request(
            "Deferred cleanup",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"payload",
        ))
        .unwrap();
    let directory = root.path().join("scripts/deferred-cleanup");
    let held = directory.join("held");
    fs::create_dir(&held).unwrap();
    fs::write(held.join("item"), b"held").unwrap();
    fs::set_permissions(&held, fs::Permissions::from_mode(0o555)).unwrap();

    let error = store.remove(&entry).unwrap_err();
    assert!(matches!(
        error,
        RepositoryError::RemovalIncomplete { ref name, ref path }
            if name == "Deferred cleanup" && path.ends_with("scripts/deferred-cleanup")
    ));
    assert!(store.resolve("deferred-cleanup").is_err());
    let deferred = root.path().join("scripts/deferred-cleanup");
    assert!(
        deferred.is_dir(),
        "failed cleanup must remain available to doctor --rebuild"
    );
    fs::set_permissions(deferred.join("held"), fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn empty_stale_directories_are_reused_but_regular_files_stay_reserved() {
    let root = TempDir::new().unwrap();
    let scripts = root.path().join("scripts");
    fs::create_dir_all(scripts.join("reuse")).unwrap();
    fs::write(scripts.join("taken"), b"not an entry directory").unwrap();
    let store = FileStore::new(root.path());

    let reused = store
        .create(request(
            "Reuse",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"reused",
        ))
        .unwrap();
    let suffixed = store
        .create(request(
            "Taken",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"suffixed",
        ))
        .unwrap();

    assert_eq!(reused.slug.as_str(), "reuse");
    assert_eq!(suffixed.slug.as_str(), "taken-2");
    assert_eq!(
        fs::read(scripts.join("taken")).unwrap(),
        b"not an entry directory"
    );
}

#[test]
fn every_known_kind_resolves_its_conventional_copy_for_source_cas() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let kinds = [
        ("python", "script.py"),
        ("shell", "script.sh"),
        ("js", "script.js"),
        ("ts", "script.ts"),
        ("fish", "script.fish"),
        ("powershell", "script.ps1"),
        ("ruby", "script.rb"),
        ("perl", "script.pl"),
        ("lua", "script.lua"),
        ("r", "script.R"),
    ];

    for (index, (kind, stored_name)) in kinds.into_iter().enumerate() {
        let name = format!("Kind {index}");
        let entry = store
            .create(request(
                &name,
                kind,
                StorageMode::Copy,
                Some(stored_name),
                b"base",
            ))
            .unwrap();
        let edited = store
            .commit_copy_edit(&entry, b"edited", &entry.meta.source_hash)
            .unwrap();
        assert_eq!(
            fs::read(
                root.path()
                    .join("scripts")
                    .join(edited.slug.as_str())
                    .join(stored_name)
            )
            .unwrap(),
            b"edited"
        );
    }
}

#[test]
fn registry_recovery_handles_read_errors_replaces_backups_and_normalizes_entries() {
    let directory_root = TempDir::new().unwrap();
    fs::create_dir(directory_root.path().join("registry.toml")).unwrap();
    let directory_store = FileStore::new(directory_root.path());
    directory_store
        .create(request(
            "Directory Recovery",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"directory",
        ))
        .unwrap();
    assert!(directory_root.path().join("registry.toml").is_file());
    assert!(directory_root.path().join("registry.toml.corrupt").is_dir());

    let replacement_root = TempDir::new().unwrap();
    let corrupt = b"entries = [invalid TOML";
    fs::write(replacement_root.path().join("registry.toml"), corrupt).unwrap();
    fs::write(
        replacement_root.path().join("registry.toml.corrupt"),
        b"older backup",
    )
    .unwrap();
    FileStore::new(replacement_root.path())
        .create(request(
            "Replacement Recovery",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"replacement",
        ))
        .unwrap();
    assert_eq!(
        fs::read(replacement_root.path().join("registry.toml.corrupt")).unwrap(),
        corrupt
    );

    let normalized_root = TempDir::new().unwrap();
    fs::write(
        normalized_root.path().join("registry.toml"),
        "note = \"keep\"\nentries = [\"not\", \"a\", \"table\"]\n",
    )
    .unwrap();
    FileStore::new(normalized_root.path())
        .create(request(
            "Normalized",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"normalized",
        ))
        .unwrap();
    let document = read_registry(&normalized_root);
    assert_eq!(document.get("note").and_then(Value::as_str), Some("keep"));
    assert!(
        document
            .get("entries")
            .and_then(Value::as_table)
            .unwrap()
            .contains_key("normalized")
    );
}

#[test]
fn metadata_only_requests_keep_a_blank_hash_and_no_payload_file() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request(
            "Metadata Only",
            "command",
            StorageMode::Copy,
            None,
            b"ignored",
        ))
        .unwrap();

    assert!(entry.meta.source_hash.is_empty());
    let files = fs::read_dir(root.path().join("scripts/metadata-only"))
        .unwrap()
        .map(|item| item.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(files, ["meta.toml"]);
}

#[test]
fn reference_rows_keep_the_original_target_without_copying_payload() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request(
            "Reference",
            "future-kind",
            StorageMode::Reference,
            Some("artifact.bin"),
            b"referenced",
        ))
        .unwrap();

    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert!(!root.path().join("scripts/reference/artifact.bin").exists());
    let registry = read_registry(&root);
    let target = registry
        .get("entries")
        .and_then(Value::as_table)
        .and_then(|entries| entries.get("reference"))
        .and_then(Value::as_table)
        .and_then(|row| row.get("target"))
        .and_then(Value::as_str);
    assert_eq!(target, Some("/original/artifact.bin"));
}

#[test]
fn registry_rebuild_handles_missing_noise_invalid_slugs_and_corrupt_entries() {
    let missing = TempDir::new().unwrap();
    assert_eq!(
        FileStore::new(missing.path()).rebuild_registry().unwrap(),
        0
    );

    let root = TempDir::new().unwrap();
    let scripts = root.path().join("scripts");
    fs::create_dir_all(scripts.join("Upper")).unwrap();
    fs::write(scripts.join("README"), "noise").unwrap();
    fs::create_dir_all(scripts.join("broken")).unwrap();
    fs::write(scripts.join("broken/meta.toml"), "not = [toml").unwrap();
    let store = FileStore::new(root.path());
    assert_eq!(store.rebuild_registry().unwrap(), 0);

    fs::create_dir_all(scripts.join("valid")).unwrap();
    fs::write(
        scripts.join("valid/meta.toml"),
        "name = \"Valid\"\nkind = \"command\"\n",
    )
    .unwrap();
    assert_eq!(store.rebuild_registry().unwrap(), 1);
    assert!(
        read_registry(&root)
            .get("entries")
            .and_then(Value::as_table)
            .unwrap()
            .contains_key("valid")
    );

    let blocked = TempDir::new().unwrap();
    fs::write(blocked.path().join("scripts"), "file").unwrap();
    assert!(FileStore::new(blocked.path()).rebuild_registry().is_err());
}
