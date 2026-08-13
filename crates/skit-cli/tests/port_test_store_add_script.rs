//! Generic interpreted-add ports from Python v0.4 `tests/test_store.py`.

use std::fs;

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let sandbox = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        };
        fs::write(sandbox.config.path().join("config.toml"), "[mirror]\nenabled = false\n").unwrap();
        sandbox
    }

    fn command(&self) -> Command {
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
            .current_dir(self.home.path());
        command
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn shell(&self, name: &str, body: &str) -> std::path::PathBuf {
        let path = self.home.path().join(format!("{name}.sh"));
        fs::write(&path, body).unwrap();
        path
    }
}

fn combined(output: &std::process::Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn test_add_script_copy_is_byte_identical_and_records_hash() {
    let sandbox = Sandbox::new();
    let source = sandbox.shell("deploy", "#!/bin/bash\n# Deploy it\necho hi\n");
    let bytes = fs::read(&source).unwrap();
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--name", "deploy", "--no-input"])
        .assert()
        .success();
    let entry = sandbox.store().resolve("deploy").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    assert_eq!(fs::read(sandbox.data.path().join("scripts/deploy/script.sh")).unwrap(), bytes);
    assert!(entry.meta.source_hash.starts_with("sha256:"));
    assert_eq!(entry.meta.workdir, "invoke");
    assert_eq!(entry.meta.description, "Deploy it");
}

#[test]
fn test_add_script_reference_points_to_origin() {
    let sandbox = Sandbox::new();
    let source = sandbox.shell("deploy", "#!/bin/bash\necho hi\n");
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--ref", "--name", "deploy", "--no-input"])
        .assert()
        .success();
    let entry = sandbox.store().resolve("deploy").unwrap();
    assert_eq!(entry.meta.workdir, "origin");
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.source, source.canonicalize().unwrap().display().to_string());
    assert!(!sandbox.data.path().join("scripts/deploy/script.sh").exists());
}

#[test]
fn test_add_script_explicit_workdir_override() {
    let sandbox = Sandbox::new();
    let store = sandbox.store();
    let source = sandbox.shell("deploy", "#!/bin/bash\necho hi\n");
    let bytes = fs::read(&source).unwrap();
    let entry = store
        .create(CreateEntry {
            name: "deploy".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: source.canonicalize().unwrap().display().to_string(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes,
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        })
        .unwrap();
    assert_eq!(entry.meta.workdir, "store");
    assert_eq!(store.resolve("deploy").unwrap().meta.workdir, "store");
}

#[test]
fn test_add_script_explicit_name_and_description() {
    let sandbox = Sandbox::new();
    let source = sandbox.shell("deploy", "#!/bin/bash\n# inferred\necho hi\n");
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--name", "ship", "--description", "custom", "--no-input"])
        .assert()
        .success();
    let entry = sandbox.store().resolve("ship").unwrap();
    assert_eq!(entry.meta.name, "ship");
    assert_eq!(entry.meta.description, "custom");
}

#[test]
fn test_add_script_records_interpreter() {
    let sandbox = Sandbox::new();
    let source = sandbox.shell("deploy", "#!/usr/bin/env zsh\necho hi\n");
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--name", "deploy", "--no-input"])
        .assert()
        .success();
    assert_eq!(EntrySettings::from_meta(&sandbox.store().resolve("deploy").unwrap().meta).interpreter, "zsh");
}

#[test]
fn test_add_script_unknown_kind_raises() {
    let sandbox = Sandbox::new();
    let source = sandbox.shell("deploy", "echo hi\n");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "martian", "--name", "deploy", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(combined(&output).contains("martian"), "{}", combined(&output));
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_add_script_non_interpreted_kind_raises() {
    let sandbox = Sandbox::new();
    let source = sandbox.shell("deploy", "echo hi\n");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "exe", "--name", "deploy", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_add_script_missing_file_raises() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("ghost.sh");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&missing)
        .args(["--kind", "shell", "--name", "ghost", "--no-input"])
        .output()
        .unwrap();
    assert_ne!(output.status.code(), Some(0));
    assert!(combined(&output).to_ascii_lowercase().contains("not found"), "{}", combined(&output));
}

#[test]
fn test_add_script_lua_uses_double_dash_description() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("tool.lua");
    fs::write(&source, "-- Resize things\nprint('x')\n").unwrap();
    sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "lua", "--name", "tool", "--no-input"])
        .assert()
        .success();
    let entry = sandbox.store().resolve("tool").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "lua");
    assert_eq!(entry.meta.description, "Resize things");
    assert!(sandbox.data.path().join("scripts/tool/script.lua").is_file());
}
