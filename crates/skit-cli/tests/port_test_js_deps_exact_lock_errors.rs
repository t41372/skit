//! Exact dependency-lock refusal ports from Python v0.4 `tests/test_js_deps.py`.
#![cfg(unix)]

use std::{fs, path::Path};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, SourcePermissions, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    bin: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt as _;
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            bin: TempDir::new().unwrap(),
        };
        let node = sandbox.bin.path().join("node");
        fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
        let mut permissions = fs::metadata(&node).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&node, permissions).unwrap();

        let store = FileStore::new(sandbox.data.path());
        let kind = EntryKind::parse("js").unwrap();
        store
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
                    dependencies: vec!["chalk".to_owned()],
                    interpreter: "node".to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap();
        fs::create_dir_all(sandbox.data.path().join(".locks/t.skit-deps.lock")).unwrap();
        sandbox
    }

    fn run(&self) -> std::process::Output {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .env("PATH", self.bin.path())
            .current_dir(self.home.path())
            .args(["run", "t", "--no-input"])
            .output()
            .unwrap()
    }
}

fn text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_install_lock_unwritable_dir_raises_126_family_not_a_traceback() {
    let output = Sandbox::new().run();
    let rendered = text(&output);
    assert_eq!(output.status.code(), Some(126), "{rendered}");
    assert!(rendered.contains("skit-deps.lock"), "{rendered}");
    assert!(!rendered.contains("traceback"), "{rendered}");
    assert!(!rendered.contains("panicked at"), "{rendered}");
}

#[test]
fn test_run_on_unwritable_entry_dir_exits_126_not_1() {
    let output = Sandbox::new().run();
    let rendered = text(&output);
    assert_eq!(output.status.code(), Some(126), "{rendered}");
    assert_ne!(output.status.code(), Some(1), "prerequisite refusal collapsed into generic failure:\n{rendered}");
    assert!(!rendered.contains("traceback"), "{rendered}");
    assert!(!rendered.contains("panicked at"), "{rendered}");
}