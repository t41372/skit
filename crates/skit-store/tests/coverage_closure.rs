use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, Slug, StorageMode};
use skit_store::{FileStore, library_surface};
use tempfile::TempDir;

fn copied_shell(name: &str, bytes: &[u8]) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("shell").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.sh"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some("script.sh".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    }
}

fn write_meta(root: &TempDir, slug: &str, body: &str) {
    let directory = root.path().join("scripts").join(slug);
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("meta.toml"), body).unwrap();
}

#[test]
fn library_reference_targets_keep_empty_and_explicit_source_meanings() {
    let root = TempDir::new().unwrap();
    write_meta(
        &root,
        "empty-exe",
        concat!(
            "schema = 1\nname = \"Empty exe\"\nkind = \"exe\"\nmode = \"reference\"\n",
            "source = \"\"\nworkdir = \"invoke\"\ndescription = \"\"\n",
        ),
    );
    write_meta(
        &root,
        "missing-shell",
        concat!(
            "schema = 1\nname = \"Missing shell\"\nkind = \"shell\"\nmode = \"reference\"\n",
            "source = \"/definitely/missing/skit-coverage.sh\"\nworkdir = \"origin\"\n",
            "description = \"\"\n",
        ),
    );
    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();

    let surface = library_surface(
        &store,
        &root.path().join("state"),
        &root.path().join("config"),
    )
    .unwrap();

    assert_eq!(
        surface.details[&Slug::parse("empty-exe").unwrap()].missing_target,
        None
    );
    assert_eq!(
        surface.details[&Slug::parse("missing-shell").unwrap()].missing_target,
        Some("/definitely/missing/skit-coverage.sh".to_owned())
    );
}

#[test]
fn copy_launch_without_a_prior_hash_and_empty_slug_reuse_are_publicly_safe() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("scripts/reuse")).unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(copied_shell("Reuse", b"printf ok\n")).unwrap();
    assert_eq!(entry.slug.as_str(), "reuse");

    let prepared = store.prepare_launch(&entry, None).unwrap();
    assert_eq!(
        fs::read(prepared.payload_path().unwrap()).unwrap(),
        b"printf ok\n"
    );
}
