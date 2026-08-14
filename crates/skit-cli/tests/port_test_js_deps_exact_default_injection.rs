//! Exact default injected-copy location port from Python v0.4 `tests/test_js_deps.py`.
#![cfg(unix)]

use std::{fs, path::Path};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _,
    SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_language::{ParseOutcome, parse_document, write_managed_params};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_write_injected_default_stays_in_the_os_temp_dir() {
    use std::os::unix::fs::PermissionsExt as _;

    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let node = bin.path().join("node");
    let marker = home.path().join("launched-path");
    fs::write(
        &node,
        "#!/bin/sh\nprintf '%s' \"$1\" > \"$SKIT_TEST_NODE_PATH\"\nexit 0\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&node).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&node, permissions).unwrap();

    let source = "const M = \"x\";\nconsole.log(M);\n";
    let ParseOutcome::Parsed(document) = parse_document("js", source) else {
        panic!("JS fixture must parse");
    };
    let declarations = document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect::<Vec<_>>();
    let managed = write_managed_params("js", source, &declarations).unwrap();

    let store = FileStore::new(data.path());
    let kind = EntryKind::parse("js").unwrap();
    let entry = store
        .create(CreateEntry {
            name: "t".to_owned(),
            kind: kind.clone(),
            mode: StorageMode::Copy,
            source: "t.js".to_owned(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: managed.into_bytes(),
                stored_name: Some(payload_stored_name(&kind, Path::new("t.js"))),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                interpreter: "node".to_owned(),
                ..EntrySettings::default()
            },
        })
        .unwrap();
    let entry_dir = store.entry_dir_path(&entry.slug);

    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    let output = command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", config.path())
        .env("SKIT_LANG", "en")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("XDG_CONFIG_HOME", home.path().join("xdg-config"))
        .env("XDG_DATA_HOME", home.path().join("xdg-data"))
        .env("XDG_STATE_HOME", home.path().join("xdg-state"))
        .env("PATH", bin.path())
        .env("SKIT_TEST_NODE_PATH", &marker)
        .args(["run", "t", "--set", "M=y", "--no-input"])
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{text}");
    let launched = fs::read_to_string(marker).unwrap();
    let launched = Path::new(&launched);
    assert_ne!(
        launched.parent(),
        Some(entry_dir.as_path()),
        "default injected copies must stay out of the persistent store: {}\n{text}",
        launched.display()
    );
    assert!(!launched.exists(), "default injected copy survived launch cleanup: {}", launched.display());
}