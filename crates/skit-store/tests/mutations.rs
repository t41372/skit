use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::mpsc,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use serde_json::json;
use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions, UpdateEntry,
};
use skit_domain::{
    EntryKind, EntrySettings, StorageMode,
    parameters::{ParamDecl, ParameterDelivery, ParameterType},
};
use skit_store::{FileStore, content_hash};
use tempfile::TempDir;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn request(name: &str, bytes: &[u8]) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.py"),
        workdir: "invoke".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions {
                readonly: false,
                unix_mode: Some(0o755),
            },
        }),
        settings: EntrySettings::default(),
    }
}

fn javascript_request(name: &str, bytes: &[u8]) -> CreateEntry {
    let mut request = request(name, bytes);
    request.kind = EntryKind::parse("js").unwrap();
    request.source = format!("/original/{name}.js");
    request.payload.as_mut().unwrap().stored_name = Some("script.js".to_owned());
    request.settings.dependencies = vec!["chalk".to_owned()];
    request
}

fn directory_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(directory)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, files);
            } else {
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }

    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

fn write_legacy_meta(root: &TempDir, slug: &str, name: &str) {
    let dir = root.path().join("scripts").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("meta.toml"),
        format!(
            "name = {name:?}\nkind = \"python\"\nmode = \"copy\"\nsource = \"/old.py\"\nsource_hash = \"\"\n"
        ),
    )
    .unwrap();
    fs::write(dir.join("script.py"), b"print('old')\n").unwrap();
}

#[test]
fn content_hash_is_the_existing_sha256_contract() {
    assert_eq!(
        content_hash(b""),
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        content_hash(b"abc"),
        "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn create_is_atomic_mints_identity_and_preserves_payload_bytes() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let bytes = b"#!/usr/bin/env python3\r\nprint('hello')\r\n";

    let mut create = request("Hello", bytes);
    create.settings.dependencies = vec!["requests>=2".to_owned()];
    create.settings.interpreter = "python3.14".to_owned();
    let entry = store.create(create).unwrap();

    assert_eq!(entry.slug.as_str(), "hello");
    assert!(entry.meta.id.is_some());
    let added_at = OffsetDateTime::parse(&entry.meta.added_at, &Rfc3339).unwrap();
    assert!(added_at <= OffsetDateTime::now_utc());
    assert_eq!(entry.meta.source_hash, content_hash(bytes));
    let settings = EntrySettings::from_meta(&entry.meta);
    assert_eq!(settings.dependencies, ["requests>=2"]);
    assert_eq!(settings.interpreter, "python3.14");
    assert_eq!(
        fs::read(root.path().join("scripts/hello/script.py")).unwrap(),
        bytes
    );
    assert!(
        fs::read_to_string(root.path().join("scripts/hello/meta.toml"))
            .unwrap()
            .contains("id = ")
    );
    let staging = root.path().join(".staging");
    assert!(
        !staging.exists() || fs::read_dir(staging).unwrap().next().is_none(),
        "successful create must not leave a staged directory"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.path().join("scripts/hello/script.py"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }
}

#[test]
fn test_write_read_parameters_roundtrip_and_legacy_params_untouched() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let mut create = request("rt", b"");
    create.kind = EntryKind::parse("command").unwrap();
    create.mode = StorageMode::Reference;
    create.source.clear();
    create.payload = None;
    create.settings = EntrySettings {
        template: "run {a} {b}".to_owned(),
        params: vec!["a".to_owned(), "b".to_owned()],
        ..EntrySettings::default()
    };
    let created = store.create(create).unwrap();
    let entry_dir = root.path().join("scripts/rt");
    let meta_path = entry_dir.join("meta.toml");

    // A future root field is not part of EntrySettings, but every settings write must retain it.
    let mut source = fs::read_to_string(&meta_path).unwrap();
    source.push_str("\n[future]\nkeep = true\n");
    fs::write(&meta_path, source).unwrap();
    let held = store.resolve(created.slug.as_str()).unwrap();
    assert_eq!(held.meta.extra["future"], json!({"keep": true}));
    assert!(!held.meta.extra.contains_key("parameters"));
    let registry_before_write = fs::read(root.path().join("registry.toml")).unwrap();

    let mut a = ParamDecl::new("a");
    a.delivery = ParameterDelivery::Placeholder;
    a.parameter_type = ParameterType::Int;
    a.required = false;
    let mut settings = EntrySettings::from_meta(&held.meta);
    assert_eq!(settings.params, ["a", "b"]);
    assert_eq!(settings.template, "run {a} {b}");
    assert!(settings.parameters.is_empty());
    settings.parameters = vec![a.clone()];

    let updated = store
        .update_settings(&held, &settings, &held.meta.workdir)
        .unwrap();
    let back = EntrySettings::from_meta(&updated.meta);
    assert_eq!(back.parameters, [a.clone()]);
    assert_eq!(back.params, ["a", "b"]);
    assert_eq!(back.template, "run {a} {b}");
    assert_eq!(updated.meta.extra["future"], json!({"keep": true}));

    let resolved = store.resolve(created.slug.as_str()).unwrap();
    assert_eq!(EntrySettings::from_meta(&resolved.meta), back);
    assert_eq!(resolved.meta.extra["future"], json!({"keep": true}));
    let document = fs::read_to_string(&meta_path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert_eq!(
        document["params"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let rows = document["parameters"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"].as_str(), Some("a"));
    assert_eq!(rows[0]["delivery"].as_str(), Some("placeholder"));
    assert_eq!(rows[0]["type"].as_str(), Some("int"));
    assert!(rows[0].get("required").is_none());
    assert_eq!(document["future"]["keep"].as_bool(), Some(true));

    let registry_after_write = fs::read(root.path().join("registry.toml")).unwrap();
    assert_ne!(registry_after_write, registry_before_write);
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert!(scan.diagnostics.is_empty());
    assert_eq!(
        fs::read(root.path().join("registry.toml")).unwrap(),
        registry_after_write,
        "the settings write must publish a fresh registry projection"
    );

    let mut cleared = EntrySettings::from_meta(&resolved.meta);
    cleared.parameters.clear();
    let cleared = store
        .update_settings(&resolved, &cleared, &resolved.meta.workdir)
        .unwrap();
    let expected_after = EntrySettings::from_meta(&cleared.meta);
    assert!(expected_after.parameters.is_empty());
    assert_eq!(expected_after.params, ["a", "b"]);
    assert_eq!(expected_after.template, "run {a} {b}");
    assert_eq!(cleared.meta.extra["future"], json!({"keep": true}));
    let resolved_after = store.resolve(created.slug.as_str()).unwrap();
    assert_eq!(
        EntrySettings::from_meta(&resolved_after.meta),
        expected_after
    );
    assert_eq!(resolved_after.meta.extra["future"], json!({"keep": true}));
    let document = fs::read_to_string(&meta_path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    assert!(!document.contains_key("parameters"));
    assert_eq!(
        document["params"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert_eq!(document["future"]["keep"].as_bool(), Some(true));
    let registry_after_clear = fs::read(root.path().join("registry.toml")).unwrap();
    assert_ne!(registry_after_clear, registry_after_write);
    let scan = store.scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert!(scan.diagnostics.is_empty());
    assert_eq!(
        fs::read(root.path().join("registry.toml")).unwrap(),
        registry_after_clear,
        "the clear must publish a fresh registry projection"
    );

    let mut files = fs::read_dir(entry_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    files.sort();
    assert_eq!(files, ["meta.toml".to_owned()]);
}

#[test]
fn create_sweeps_files_and_directories_left_in_the_private_staging_root() {
    let root = TempDir::new().unwrap();
    let staging = root.path().join(".staging");
    fs::create_dir_all(staging.join("abandoned/nested")).unwrap();
    fs::write(staging.join("abandoned/nested/payload"), b"partial").unwrap();
    fs::write(staging.join("abandoned-file"), b"partial").unwrap();
    let store = FileStore::new(root.path());

    store.create(request("Fresh", b"complete")).unwrap();

    assert!(fs::read_dir(staging).unwrap().next().is_none());
    assert_eq!(
        fs::read(root.path().join("scripts/fresh/script.py")).unwrap(),
        b"complete"
    );
}

#[test]
fn create_refuses_conflicts_and_path_traversal_without_partial_entries() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(request("Hello", b"first")).unwrap();

    let conflict = store.create(request("Hello", b"second")).unwrap_err();
    assert!(matches!(conflict, RepositoryError::Conflict { .. }));
    assert_eq!(
        fs::read(root.path().join("scripts/hello/script.py")).unwrap(),
        b"first"
    );

    let mut invalid = request("Escape", b"payload");
    invalid.payload.as_mut().unwrap().stored_name = Some("../outside".to_owned());
    let error = store.create(invalid).unwrap_err();
    assert!(matches!(error, RepositoryError::InvalidMutation { .. }));
    assert!(!root.path().join("scripts/escape").exists());
    assert!(!root.path().join("outside").exists());
}

#[test]
fn legacy_claim_stamps_once_and_old_handles_cannot_touch_a_reincarnation() {
    let root = TempDir::new().unwrap();
    write_legacy_meta(&root, "legacy", "Legacy");
    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();
    let held = store.resolve("legacy").unwrap();
    assert!(held.meta.id.is_none());

    let claimed = store.claim_identity(&held).unwrap();
    let old_id = claimed.meta.id.clone().unwrap();
    assert_eq!(
        store.resolve("legacy").unwrap().meta.id,
        Some(old_id.clone())
    );

    store.remove(&claimed).unwrap();
    let replacement = store.create(request("Legacy", b"replacement")).unwrap();
    assert_ne!(replacement.meta.id, Some(old_id));

    let error = store.describe(&claimed, "must not land").unwrap_err();
    assert!(matches!(error, RepositoryError::StaleEntry { .. }));
    assert_eq!(store.resolve("legacy").unwrap().meta.description, "");
}

#[test]
fn a_legacy_mutation_claims_identity_before_it_writes() {
    let root = TempDir::new().unwrap();
    write_legacy_meta(&root, "legacy", "Legacy");
    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();
    let held = store.resolve("legacy").unwrap();
    assert!(held.meta.id.is_none());

    let described = store.describe(&held, "claimed during mutation").unwrap();

    assert!(described.meta.id.is_some());
    assert_eq!(described.meta.description, "claimed during mutation");
    assert_eq!(store.resolve("legacy").unwrap().meta.id, described.meta.id);
}

#[test]
fn metadata_mutations_preserve_every_unknown_toml_value() {
    let root = TempDir::new().unwrap();
    let directory = root.path().join("scripts/extensions");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        r#"# Keep this file header.
name = "Extensions" # Keep the display-name note.
kind = "future-kind"
mode = "reference"
source = "/tmp/tool"
release_at = 2026-08-08T12:34:56Z
limit = inf
# Keep the future section note.
future = { enabled = true, values = [1, 2, 3] } # Keep the inline future note.
"#,
    )
    .unwrap();
    let store = FileStore::new(root.path());
    store.rebuild_registry().unwrap();

    let held = store.resolve("extensions").unwrap();
    store.describe(&held, "updated").unwrap();

    let text = fs::read_to_string(directory.join("meta.toml")).unwrap();
    for comment in [
        "# Keep this file header.",
        "# Keep the display-name note.",
        "# Keep the future section note.",
        "# Keep the inline future note.",
    ] {
        assert!(text.contains(comment), "lost {comment}:\n{text}");
    }
    let document = text.parse::<toml::Table>().unwrap();
    assert_eq!(
        document["release_at"].as_datetime().unwrap().to_string(),
        "2026-08-08T12:34:56Z"
    );
    assert!(document["limit"].as_float().unwrap().is_infinite());
    assert_eq!(document["future"]["enabled"].as_bool(), Some(true));
    assert_eq!(document["future"]["values"].as_array().unwrap().len(), 3);
}

#[test]
fn rename_describe_and_remove_preserve_identity_and_payload() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let created = store.create(request("Before", b"payload")).unwrap();
    let claimed = store.claim_identity(&created).unwrap();

    let described = store.describe(&claimed, "useful").unwrap();
    assert_eq!(described.meta.description, "useful");
    assert_eq!(described.meta.id, claimed.meta.id);

    let renamed = store.rename(&described, "After Name").unwrap();
    assert_eq!(renamed.slug.as_str(), "before");
    assert_eq!(renamed.meta.name, "After Name");
    assert_eq!(renamed.meta.id, claimed.meta.id);
    assert_eq!(
        fs::read(root.path().join("scripts/before/script.py")).unwrap(),
        b"payload"
    );
    assert_eq!(store.resolve("before").unwrap().meta.name, "After Name");
    assert_eq!(store.resolve("After Name").unwrap().slug.as_str(), "before");

    assert_eq!(store.remove(&renamed).unwrap(), "After Name");
    assert!(!root.path().join("scripts/before").exists());
}

#[test]
fn copy_edit_is_identity_and_source_compare_and_swap() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let created = store.create(request("Edit", b"base")).unwrap();
    let claimed = store.claim_identity(&created).unwrap();

    let stale = store
        .commit_copy_edit(&claimed, b"wrong", "sha256:not-the-base")
        .unwrap_err();
    assert!(matches!(stale, RepositoryError::SourceChanged { .. }));
    assert_eq!(
        fs::read(root.path().join("scripts/edit/script.py")).unwrap(),
        b"base"
    );

    let edited = store
        .commit_copy_edit(&claimed, b"next", &claimed.meta.source_hash)
        .unwrap();
    assert_eq!(edited.meta.source_hash, content_hash(b"next"));
    assert_eq!(
        fs::read(root.path().join("scripts/edit/script.py")).unwrap(),
        b"next"
    );
}

#[test]
fn combined_update_commits_metadata_and_source_under_one_identity_check() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let claimed = store
        .claim_identity(&store.create(request("Combined", b"before")).unwrap())
        .unwrap();
    let settings = EntrySettings {
        dependencies: vec!["requests".to_owned()],
        ..EntrySettings::default()
    };

    let updated = store
        .update_entry(
            &claimed,
            UpdateEntry {
                name: "After".to_owned(),
                description: "complete".to_owned(),
                settings,
                workdir: "store".to_owned(),
                source: Some(b"after".to_vec()),
                expected_source_hash: claimed.meta.source_hash.clone(),
            },
        )
        .unwrap();

    assert_eq!(updated.slug, claimed.slug);
    assert_eq!(updated.meta.name, "After");
    assert_eq!(updated.meta.description, "complete");
    assert_eq!(updated.meta.workdir, "store");
    assert_eq!(
        fs::read(root.path().join("scripts/combined/script.py")).unwrap(),
        b"after"
    );
    assert_eq!(store.resolve("After").unwrap(), updated);

    let before_meta = fs::read(root.path().join("scripts/combined/meta.toml")).unwrap();
    let error = store
        .update_entry(
            &updated,
            UpdateEntry {
                name: "Never".to_owned(),
                description: "must not land".to_owned(),
                settings: EntrySettings::default(),
                workdir: "invoke".to_owned(),
                source: Some(b"never".to_vec()),
                expected_source_hash: "sha256:stale".to_owned(),
            },
        )
        .unwrap_err();
    assert!(matches!(error, RepositoryError::SourceChanged { .. }));
    assert_eq!(
        fs::read(root.path().join("scripts/combined/meta.toml")).unwrap(),
        before_meta
    );
    assert_eq!(
        fs::read(root.path().join("scripts/combined/script.py")).unwrap(),
        b"after"
    );
}

#[test]
fn module_typed_copy_edits_ignore_dependency_support_files() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let mut create = request("Module", b"export const value = 1;\n");
    create.kind = EntryKind::parse("js").unwrap();
    create.source = "/original/module.mjs".to_owned();
    create.payload.as_mut().unwrap().stored_name = Some("script.mjs".to_owned());
    let entry = store.create(create).unwrap();
    let directory = root.path().join("scripts/module");
    fs::write(directory.join("package.json"), "{}\n").unwrap();
    fs::write(directory.join("package-lock.json"), "{}\n").unwrap();
    fs::write(directory.join(".skit-deps"), "stamp\n").unwrap();

    let edited = store
        .commit_copy_edit(
            &entry,
            b"export const value = 2;\n",
            &entry.meta.source_hash,
        )
        .unwrap();
    assert_eq!(
        fs::read(directory.join("script.mjs")).unwrap(),
        b"export const value = 2;\n"
    );

    let updated = store
        .update_entry(
            &edited,
            UpdateEntry {
                name: "Renamed module".to_owned(),
                description: "updated".to_owned(),
                settings: EntrySettings::default(),
                workdir: "invoke".to_owned(),
                source: Some(b"export const value = 3;\n".to_vec()),
                expected_source_hash: edited.meta.source_hash.clone(),
            },
        )
        .unwrap();
    assert_eq!(updated.slug, entry.slug);
    assert_eq!(
        fs::read(directory.join("script.mjs")).unwrap(),
        b"export const value = 3;\n"
    );
}

#[test]
fn concurrent_copy_edits_allow_exactly_one_source_cas_winner() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let claimed = store
        .claim_identity(&store.create(request("Race", b"base")).unwrap())
        .unwrap();
    let expected = claimed.meta.source_hash.clone();
    let barrier = Arc::new(Barrier::new(3));

    let handles = [b"left".as_slice(), b"right".as_slice()].map(|payload| {
        let store = store.clone();
        let held = claimed.clone();
        let expected = expected.clone();
        let barrier = Arc::clone(&barrier);
        let payload = payload.to_vec();
        thread::spawn(move || {
            barrier.wait();
            store.commit_copy_edit(&held, &payload, &expected)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().unwrap());

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(RepositoryError::SourceChanged { .. })))
            .count(),
        1
    );
    let bytes = fs::read(root.path().join("scripts/race/script.py")).unwrap();
    assert!(bytes == b"left" || bytes == b"right");
}

#[test]
fn test_store_remove_waits_for_a_live_js_install_lock() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let claimed = store
        .create(javascript_request("Locked", b"console.log(1);\n"))
        .unwrap();
    let entry_dir = store.entry_dir_path(&claimed.slug);
    let lock_path = root.path().join(".locks/locked.skit-deps.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.lock().unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx.send(store.remove(&claimed)).unwrap();
    });
    started_rx.recv().unwrap();

    assert!(done_rx.recv_timeout(Duration::from_millis(100)).is_err());
    drop(lock);
    assert_eq!(
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap(),
        "Locked"
    );
    worker.join().unwrap();
    assert!(!entry_dir.exists());
    assert!(
        lock_path.is_file(),
        "the persistent dependency lock inode was removed with the entry"
    );
}

#[test]
fn test_store_remove_surfaces_install_lock_failure_without_deleting_entry() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(javascript_request("Locked", b"console.log(1);\n"))
        .unwrap();
    let entry_dir = store.entry_dir_path(&entry.slug);
    let meta = entry_dir.join("meta.toml");
    let payload = store.payload_path(&entry).unwrap();
    let registry = root.path().join("registry.toml");
    let lock_path = root.path().join(".locks/locked.skit-deps.lock");
    fs::create_dir_all(&lock_path).unwrap();

    let entry_before = directory_bytes(&entry_dir);
    let meta_before = fs::read(&meta).unwrap();
    let payload_before = fs::read(&payload).unwrap();
    let registry_before = fs::read(&registry).unwrap();

    let error = store.remove(&entry).unwrap_err();

    assert!(error.to_string().contains("skit-deps.lock"), "{error}");
    assert!(entry_dir.is_dir());
    assert_eq!(directory_bytes(&entry_dir), entry_before);
    assert_eq!(fs::read(&meta).unwrap(), meta_before);
    assert_eq!(fs::read(&payload).unwrap(), payload_before);
    assert_eq!(fs::read(&registry).unwrap(), registry_before);
    assert!(
        lock_path.is_dir(),
        "the refusing dependency-lock directory was replaced"
    );
    let fresh = store.resolve(entry.slug.as_str()).unwrap();
    assert_eq!(fresh.meta, entry.meta);
}

#[test]
fn a_rename_that_keeps_the_same_slug_reuses_its_own_directory() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(request("Report", b"print(1)\n")).unwrap();
    assert_eq!(entry.slug.as_str(), "report");

    // The new display name derives the same slug, so the entry keeps its address.
    let renamed = store.rename(&entry, "  Report  ").unwrap();

    assert_eq!(renamed.slug.as_str(), "report");
    assert_eq!(renamed.meta.name, "Report");
    assert!(root.path().join("scripts/report").is_dir());
    assert_eq!(store.scan().unwrap().entries.len(), 1);
}
