//! Exact installer-stderr propagation port from Python v0.4 `tests/test_js_deps.py`.
#![cfg(unix)]

use std::{fs, path::Path};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

#[test]
fn test_ensure_installed_installer_failure_carries_its_stderr() {
    use std::os::unix::fs::PermissionsExt as _;

    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    for (name, body) in [
        ("node", "#!/bin/sh\nexit 0\n"),
        (
            "npm",
            concat!(
                "#!/bin/sh\n",
                "printf '%s\\n' 'npm error code E404' >&2\n",
                "printf '%s\\n' 'npm error 404 Not Found - GET https://registry.npmjs.org/skit-no-such-pkg-e2e-xyz - Not found' >&2\n",
                "printf '%s\\n' 'npm error A complete log of this run can be found in: /tmp/debug.log' >&2\n",
                "exit 1\n",
            ),
        ),
    ] {
        let path = bin.path().join(name);
        fs::write(&path, body).unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).unwrap();
    }

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
                bytes: b"console.log(1);\n".to_vec(),
                stored_name: Some(payload_stored_name(&kind, Path::new("t.js"))),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings {
                dependencies: vec!["skit-no-such-pkg-e2e-xyz".to_owned()],
                interpreter: "node".to_owned(),
                ..EntrySettings::default()
            },
        })
        .unwrap();

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
        .current_dir(home.path())
        .args(["run", "t", "--no-input"])
        .output()
        .unwrap();
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(output.status.code(), Some(0), "{rendered}");
    assert!(rendered.contains("Not Found - GET"), "useful installer stderr was lost:\n{rendered}");
    assert!(rendered.contains("skit-no-such-pkg-e2e-xyz"), "package name disappeared:\n{rendered}");
    assert!(!rendered.contains("A complete log"), "npm noise leaked into the user-facing reason:\n{rendered}");

    let entry_dir = store.entry_dir_path(&entry.slug);
    assert!(!entry_dir.join("node_modules/.skit-deps-ok").exists());
    assert!(!entry_dir.join(".skit-deps").exists(), "failed install was marked fresh");
}