use std::fs;

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, RepositoryError, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::{FileStore, stored_filename, stored_filenames};
use tempfile::TempDir;

#[test]
fn stored_filenames_match_the_v040_library_layout() {
    assert_eq!(stored_filename("python"), Some("script.py"));
    assert_eq!(stored_filename("shell"), Some("script.sh"));
    assert_eq!(stored_filename("js"), Some("script.js"));
    assert_eq!(stored_filename("ts"), Some("script.ts"));
    assert_eq!(stored_filename("fish"), Some("script.fish"));
    assert_eq!(stored_filename("powershell"), Some("script.ps1"));
    assert_eq!(stored_filename("ruby"), Some("script.rb"));
    assert_eq!(stored_filename("perl"), Some("script.pl"));
    assert_eq!(stored_filename("lua"), Some("script.lua"));
    assert_eq!(stored_filename("r"), Some("script.r"));
    assert_eq!(stored_filename("prompt"), Some("prompt.md"));
    assert_eq!(stored_filename("exe"), None);
    assert_eq!(stored_filename("command"), None);
    assert_eq!(
        stored_filenames("js"),
        ["script.js", "script.mjs", "script.cjs"]
    );
    assert_eq!(
        stored_filenames("ts"),
        ["script.ts", "script.mts", "script.cts"]
    );
    for (kind, expected) in [
        ("python", &["script.py"][..]),
        ("shell", &["script.sh"][..]),
        ("fish", &["script.fish"][..]),
        ("powershell", &["script.ps1"][..]),
        ("ruby", &["script.rb"][..]),
        ("perl", &["script.pl"][..]),
        ("lua", &["script.lua"][..]),
        ("r", &["script.r"][..]),
        ("prompt", &["prompt.md"][..]),
        ("command", &[][..]),
        ("exe", &[][..]),
        ("future", &["payload"][..]),
    ] {
        assert_eq!(stored_filenames(kind), expected);
    }
}

#[test]
fn payload_fallback_is_deterministic_and_reports_missing_or_ambiguous_copies() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(CreateEntry {
            name: "Fallback".to_owned(),
            kind: EntryKind::parse("future-kind").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/custom".to_owned(),
            description: String::new(),
            workdir: "invoke".to_owned(),
            payload: Some(EntryPayload {
                bytes: b"payload".to_vec(),
                stored_name: Some("custom.bin".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let directory = store.entry_dir_path(&entry.slug);
    for support in [
        "package.json",
        "package-lock.json",
        "bun.lock",
        "bun.lockb",
        "deno.lock",
        ".skit-deps",
        ".skit-deps.tmp-one",
    ] {
        fs::write(directory.join(support), "support").unwrap();
    }
    assert_eq!(
        store.payload_path(&entry).unwrap(),
        directory.join("custom.bin")
    );

    fs::write(directory.join("second.bin"), "second").unwrap();
    assert!(matches!(
        store.payload_path(&entry),
        Err(RepositoryError::InvalidMutation { reason })
            if reason.template().contains("more than one")
    ));
    fs::remove_file(directory.join("second.bin")).unwrap();
    fs::remove_file(directory.join("custom.bin")).unwrap();
    assert!(matches!(
        store.payload_path(&entry),
        Err(RepositoryError::InvalidMutation { reason })
            if reason.template().contains("no stored payload")
    ));
    fs::remove_dir_all(&directory).unwrap();
    assert!(matches!(
        store.payload_path(&entry),
        Err(RepositoryError::Io { .. })
    ));
}

#[test]
fn payload_path_uses_the_original_for_references_and_the_stored_copy_for_copies() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("source.sh");
    fs::write(&source, b"echo ok\n").unwrap();

    let copied = store
        .create(CreateEntry {
            name: "Copied".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: source.display().to_string(),
            description: String::new(),
            workdir: "invoke".to_owned(),
            payload: Some(EntryPayload {
                bytes: fs::read(&source).unwrap(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    assert_eq!(
        store.payload_path(&copied).unwrap(),
        root.path().join("scripts/copied/script.sh")
    );
    assert_eq!(
        store.entry_dir_path(&copied.slug),
        root.path().join("scripts/copied")
    );

    let referenced = store
        .create(CreateEntry {
            name: "Referenced".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Reference,
            source: source.display().to_string(),
            description: String::new(),
            workdir: "origin".to_owned(),
            payload: None,
            settings: EntrySettings::default(),
        })
        .unwrap();
    assert_eq!(store.payload_path(&referenced).unwrap(), source);
}

#[test]
fn payload_path_preserves_javascript_module_extensions_and_ignores_private_support_files() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("module.mjs");
    fs::write(&source, b"export default 1;\n").unwrap();
    let copied = store
        .create(CreateEntry {
            name: "Module".to_owned(),
            kind: EntryKind::parse("js").unwrap(),
            mode: StorageMode::Copy,
            source: source.display().to_string(),
            description: String::new(),
            workdir: "invoke".to_owned(),
            payload: Some(EntryPayload {
                bytes: fs::read(&source).unwrap(),
                stored_name: Some("script.mjs".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let directory = store.entry_dir_path(&copied.slug);
    fs::write(directory.join("package.json"), "{}\n").unwrap();
    fs::write(directory.join("package-lock.json"), "{}\n").unwrap();
    fs::write(directory.join(".skit-deps"), "stamp\n").unwrap();

    assert_eq!(
        store.payload_path(&copied).unwrap(),
        directory.join("script.mjs")
    );
}

#[test]
fn skit_private_files_never_look_like_a_second_payload() {
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(CreateEntry {
            name: "Thing".to_owned(),
            kind: EntryKind::parse("zig").unwrap(),
            mode: StorageMode::Copy,
            source: "/original/thing.zig".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"pub fn main() void {}\n".to_vec(),
                stored_name: Some("thing.zig".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    let directory = root.path().join("scripts/thing");
    let payload = directory.join("thing.zig");
    assert_eq!(store.payload_path(&entry).unwrap(), payload);

    // Each of these is a private file skit writes next to the payload.
    for name in [
        ".run-0123456789abcdef0123456789abcdef.zig",
        ".thing.zig.0123456789abcdef0123456789abcdef.tmp",
        ".skit-deps",
        ".skit-deps.tmp-1-2",
    ] {
        fs::write(directory.join(name), b"private\n").unwrap();
        assert_eq!(
            store.payload_path(&entry).unwrap(),
            payload,
            "{name} was treated as a payload"
        );
    }

    // A second real file is still ambiguous and still refused.
    fs::write(directory.join("other.zig"), b"other\n").unwrap();
    assert!(matches!(
        store.payload_path(&entry),
        Err(RepositoryError::InvalidMutation { .. })
    ));
}
