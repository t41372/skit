use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions,
    library_detail::{LibraryDetailRepository as _, LibraryTargetState},
};
use skit_domain::{EntryKind, EntrySettings, Slug, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

fn write_meta(root: &TempDir, slug: &str, body: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("meta.toml"), body).unwrap();
}

fn command(name: &str, description: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Reference,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: description.to_owned(),
        payload: None,
        settings: EntrySettings {
            template: "printf ok".to_owned(),
            ..EntrySettings::default()
        },
    }
}

#[test]
fn refresh_entries_keep_source_bytes_and_storage_states_adapter_owned() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "copy",
        concat!(
            "schema = 1\nname = \"Copy\"\nkind = \"shell\"\nmode = \"copy\"\n",
            "source = \"/missing/original.sh\"\nworkdir = \"invoke\"\ndescription = \"\"\n",
        ),
    );
    let source = b"printf '\xff'\r\n";
    fs::write(root.path().join("scripts/copy/script.sh"), source).unwrap();

    write_meta(
        &root,
        "missing",
        concat!(
            "schema = 1\nname = \"Missing\"\nkind = \"shell\"\nmode = \"reference\"\n",
            "source = \"/definitely/missing/skit-library.sh\"\nworkdir = \"origin\"\n",
            "description = \"\"\n",
        ),
    );
    write_meta(
        &root,
        "future",
        concat!(
            "schema = 1\nname = \"Future\"\nkind = \"martian\"\nmode = \"reference\"\n",
            "source = \"/definitely/missing/future\"\nworkdir = \"origin\"\ndescription = \"\"\n",
        ),
    );

    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();
    let refresh = store.library_refresh().unwrap();
    let snapshots = refresh.entries;
    let by_slug = |slug: &str| {
        snapshots
            .iter()
            .find(|snapshot| snapshot.entry.slug == Slug::parse(slug).unwrap())
            .unwrap()
    };

    let copy = by_slug("copy");
    assert_eq!(copy.source.as_deref(), Some(source.as_slice()));
    assert_eq!(copy.target, LibraryTargetState::Present);
    assert!(!copy.original_source_exists);

    let missing = by_slug("missing");
    assert!(missing.source.is_none());
    assert_eq!(
        missing.target,
        LibraryTargetState::Missing("/definitely/missing/skit-library.sh".into())
    );

    let future = by_slug("future");
    assert_eq!(future.target, LibraryTargetState::NotApplicable);
}

#[test]
fn refresh_uses_registry_membership_and_excludes_a_valid_orphan_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(command("Member", "indexed")).unwrap();
    write_meta(
        &root,
        "orphan",
        concat!(
            "schema = 1\nname = \"Orphan\"\nkind = \"command\"\nmode = \"reference\"\n",
            "source = \"\"\nworkdir = \"invoke\"\ndescription = \"valid but not indexed\"\n",
            "template = \"printf orphan\"\n",
        ),
    );

    let refresh = store.library_refresh().unwrap();
    let scan_slugs = refresh
        .scan
        .entries
        .iter()
        .map(|entry| entry.slug.as_str())
        .collect::<Vec<_>>();
    let detail_slugs = refresh
        .entries
        .iter()
        .map(|entry| entry.entry.slug.as_str())
        .collect::<Vec<_>>();
    assert_eq!(scan_slugs, ["member"]);
    assert_eq!(detail_slugs, scan_slugs);
}

#[test]
fn corrupt_membership_keeps_valid_scan_and_detail_sets_equal_with_a_diagnostic() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(command("Good", "healthy")).unwrap();
    let corrupt = store.create(command("Broken", "before")).unwrap();
    fs::write(
        root.path()
            .join("scripts")
            .join(corrupt.slug.as_str())
            .join("meta.toml"),
        "[[[broken",
    )
    .unwrap();

    let refresh = store.library_refresh().unwrap();
    let scan_slugs = refresh
        .scan
        .entries
        .iter()
        .map(|entry| entry.slug.clone())
        .collect::<Vec<_>>();
    let detail_slugs = refresh
        .entries
        .iter()
        .map(|entry| entry.entry.slug.clone())
        .collect::<Vec<_>>();
    assert_eq!(scan_slugs, detail_slugs);
    assert_eq!(scan_slugs, [Slug::parse("good").unwrap()]);
    assert_eq!(refresh.scan.diagnostics.len(), 1);
    assert_eq!(
        refresh.scan.diagnostics[0].slug.as_deref(),
        Some(corrupt.slug.as_str())
    );
}

#[test]
fn a_stale_row_uses_one_current_entry_for_both_faces_then_repairs() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(command("Current", "before")).unwrap();
    let meta_path = root
        .path()
        .join("scripts")
        .join(entry.slug.as_str())
        .join("meta.toml");
    let meta = fs::read_to_string(&meta_path).unwrap();
    fs::write(
        &meta_path,
        meta.replace("description = \"before\"", "description = \"after\""),
    )
    .unwrap();

    let refresh = store.library_refresh().unwrap();
    assert_eq!(refresh.scan.entries[0].description, "after");
    assert_eq!(refresh.entries[0].entry.meta.description, "after");
    assert!(
        fs::read_to_string(root.path().join("registry.toml"))
            .unwrap()
            .contains("description = \"after\"")
    );
}

#[test]
fn a_refresh_never_waits_for_the_registry_repair_lock() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(command("Current", "before")).unwrap();
    let meta_path = root
        .path()
        .join("scripts")
        .join(entry.slug.as_str())
        .join("meta.toml");
    let meta = fs::read_to_string(&meta_path).unwrap();
    fs::write(
        &meta_path,
        meta.replace("description = \"before\"", "description = \"after\""),
    )
    .unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.path().join("registry.native.lock"))
        .unwrap();
    lock.set_len(1).unwrap();
    lock.lock().unwrap();
    let registry_before = fs::read(root.path().join("registry.toml")).unwrap();

    let refresh = store.library_refresh().unwrap();

    assert_eq!(refresh.scan.entries[0].description, "after");
    assert_eq!(refresh.entries[0].entry.meta.description, "after");
    assert_eq!(
        fs::read(root.path().join("registry.toml")).unwrap(),
        registry_before
    );
    drop(lock);
    store.library_refresh().unwrap();
    assert!(
        fs::read_to_string(root.path().join("registry.toml"))
            .unwrap()
            .contains("description = \"after\"")
    );
}

#[test]
fn a_fresh_refresh_preserves_metadata_payload_and_registry_bytes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = b"printf '\xff'\r\n";
    let entry = store
        .create(CreateEntry {
            name: "Exact".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/exact.sh".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: source.to_vec(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let entry_dir = root.path().join("scripts").join(entry.slug.as_str());
    let paths = [
        entry_dir.join("meta.toml"),
        entry_dir.join("script.sh"),
        root.path().join("registry.toml"),
    ];
    let before = paths
        .iter()
        .map(|path| fs::read(path).unwrap())
        .collect::<Vec<_>>();

    let refresh = store.library_refresh().unwrap();
    assert_eq!(
        refresh.entries[0].source.as_deref(),
        Some(source.as_slice())
    );
    for (path, before) in paths.iter().zip(before) {
        assert_eq!(
            fs::read(path).unwrap(),
            before,
            "{} changed",
            path.display()
        );
    }
}
