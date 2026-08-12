use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions, UpdateEntry,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

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
        source: format!("/original/{name}"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: stored_name.map(str::to_owned),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn write_legacy(root: &TempDir, description: &str) {
    let directory = root.path().join("scripts").join("legacy");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        format!(
            "name = \"Legacy\"\nkind = \"python\"\nmode = \"copy\"\ndescription = {description:?}\n"
        ),
    )
    .unwrap();
    fs::write(directory.join("script.py"), b"legacy").unwrap();
}

#[test]
fn same_slug_renames_succeed_while_name_conflicts_and_blank_names_refuse() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let first = store
        .create(request(
            "Alpha Tool",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"alpha",
        ))
        .unwrap();
    let first = store.claim_identity(&first).unwrap();

    let renamed = store.rename(&first, "Alpha-Tool").unwrap();
    assert_eq!(renamed.slug.as_str(), "alpha-tool");
    assert_eq!(renamed.meta.id, first.meta.id);
    assert_eq!(
        fs::read(root.path().join("scripts/alpha-tool/script.py")).unwrap(),
        b"alpha"
    );

    let renamed_again = store.rename(&renamed, "New display name").unwrap();
    assert_eq!(renamed_again.slug.as_str(), "alpha-tool");
    assert_eq!(renamed_again.meta.name, "New display name");

    let second = store
        .create(request(
            "Beta",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"beta",
        ))
        .unwrap();
    let second = store.claim_identity(&second).unwrap();
    assert!(matches!(
        store.rename(&second, "New display name").unwrap_err(),
        RepositoryError::RenameConflict { .. }
    ));
    assert!(matches!(
        store.rename(&second, "   ").unwrap_err(),
        RepositoryError::InvalidMutation { .. }
    ));
}

#[test]
fn another_entry_slug_is_also_a_reserved_display_name() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let alpha = store
        .create(request(
            "Alpha Name",
            "command",
            StorageMode::Reference,
            None,
            b"",
        ))
        .unwrap();
    let beta = store
        .create(request(
            "Beta",
            "command",
            StorageMode::Reference,
            None,
            b"",
        ))
        .unwrap();

    assert!(matches!(
        store.rename(&beta, alpha.slug.as_str()).unwrap_err(),
        RepositoryError::RenameConflict { name } if name == alpha.slug.as_str()
    ));
    let renamed = store.rename(&alpha, "alpha-name").unwrap();
    assert_eq!(renamed.meta.name, "alpha-name");
    assert_eq!(renamed.slug, alpha.slug);
}

#[test]
fn colliding_slug_bases_receive_deterministic_numeric_suffixes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    let slugs = ["A B", "A-B", "A_B"].map(|name| {
        store
            .create(request(
                name,
                "python",
                StorageMode::Copy,
                Some("script.py"),
                name.as_bytes(),
            ))
            .unwrap()
            .slug
            .to_string()
    });

    assert_eq!(slugs, ["a-b", "a-b-2", "a-b-3"]);
}

#[test]
fn reference_and_payloadless_entries_refuse_copy_editing_cleanly() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let reference = store
        .create(request(
            "Linked",
            "future-kind",
            StorageMode::Reference,
            Some("linked.bin"),
            b"linked",
        ))
        .unwrap();
    assert!(!root.path().join("scripts/linked/linked.bin").exists());
    assert!(matches!(
        store
            .commit_copy_edit(&reference, b"edit", &reference.meta.source_hash)
            .unwrap_err(),
        RepositoryError::InvalidMutation { .. }
    ));

    let payloadless = store
        .create(CreateEntry {
            name: "Metadata Only".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    assert!(payloadless.meta.source_hash.is_empty());
    assert!(matches!(
        store
            .commit_copy_edit(&payloadless, b"edit", "")
            .unwrap_err(),
        RepositoryError::InvalidMutation { .. }
    ));

    let missing_name = request(
        "Missing Stored Name",
        "python",
        StorageMode::Copy,
        None,
        b"payload",
    );
    assert!(matches!(
        store.create(missing_name).unwrap_err(),
        RepositoryError::InvalidMutation { .. }
    ));
}

#[test]
fn unknown_kinds_use_the_single_payload_and_refuse_an_ambiguous_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(request(
            "Opaque",
            "future-kind",
            StorageMode::Copy,
            Some("artifact.custom"),
            b"base",
        ))
        .unwrap();
    let entry = store.claim_identity(&entry).unwrap();

    let edited = store
        .commit_copy_edit(&entry, b"next", &entry.meta.source_hash)
        .unwrap();
    assert_eq!(
        fs::read(root.path().join("scripts/opaque/artifact.custom")).unwrap(),
        b"next"
    );

    fs::create_dir(root.path().join("scripts/opaque/ignored-directory")).unwrap();
    fs::write(root.path().join("scripts/opaque/second.bin"), b"second").unwrap();
    assert!(matches!(
        store
            .commit_copy_edit(&edited, b"never", &edited.meta.source_hash)
            .unwrap_err(),
        RepositoryError::InvalidMutation { .. }
    ));
}

#[test]
fn claims_refuse_missing_entries_and_content_changed_legacy_handles() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let created = store
        .create(request(
            "Gone",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"gone",
        ))
        .unwrap();
    let claimed = store.claim_identity(&created).unwrap();
    store.remove(&claimed).unwrap();
    assert!(matches!(
        store.claim_identity(&claimed).unwrap_err(),
        RepositoryError::StaleEntry { .. }
    ));

    write_legacy(&root, "before");
    store.rebuild_registry().unwrap();
    let held = store.resolve("legacy").unwrap();
    write_legacy(&root, "after");
    assert!(matches!(
        store.claim_identity(&held).unwrap_err(),
        RepositoryError::StaleEntry { .. }
    ));
}

#[test]
fn stored_payload_names_must_be_one_safe_path_component() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    for (index, name) in ["", ".", "..", "a/b", "a\\b", "a\0b", "/absolute"]
        .into_iter()
        .enumerate()
    {
        assert!(matches!(
            store.create(request(
                &format!("Unsafe {index}"),
                "future-kind",
                StorageMode::Copy,
                Some(name),
                b"payload",
            )),
            Err(RepositoryError::InvalidMutation { .. })
        ));
    }
}

#[test]
fn an_unindexed_legacy_name_still_blocks_a_duplicate_create() {
    let root = TempDir::new().unwrap();
    write_legacy(&root, "legacy");
    let store = FileStore::new(root.path());

    assert!(matches!(
        store.create(request(
            "Legacy",
            "python",
            StorageMode::Copy,
            Some("script.py"),
            b"duplicate",
        )),
        Err(RepositoryError::Conflict { .. })
    ));
}

#[test]
fn a_reference_entry_refuses_a_staged_source_replacement() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let original = root.path().join("original.py");
    fs::write(&original, b"print(1)\n").unwrap();
    let entry = store
        .create(CreateEntry {
            name: "Referenced".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Reference,
            source: original.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();

    let error = store
        .update_entry(
            &entry,
            UpdateEntry {
                name: entry.meta.name.clone(),
                description: String::new(),
                settings: EntrySettings::from_meta(&entry.meta),
                workdir: entry.meta.workdir.clone(),
                source: Some(b"print(2)\n".to_vec()),
                expected_source_hash: entry.meta.source_hash.clone(),
            },
        )
        .unwrap_err();

    assert!(matches!(error, RepositoryError::InvalidMutation { .. }));
    assert!(
        error
            .to_string()
            .contains("reference entries are edited at their original path")
    );
    assert_eq!(fs::read(&original).unwrap(), b"print(1)\n");
}

#[test]
fn a_copy_payload_without_a_stored_name_is_refused_before_any_write() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    let error = store
        .create(CreateEntry {
            name: "Nameless".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/nameless.py".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"print(1)\n".to_vec(),
                stored_name: None,
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap_err();

    assert!(matches!(error, RepositoryError::InvalidMutation { .. }));
    assert!(!root.path().join("scripts/nameless").exists());
}
