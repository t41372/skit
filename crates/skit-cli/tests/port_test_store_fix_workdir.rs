//! Workdir-default ports from Python `tests/test_store_fix.py` at `main@206f9ef`.
//!
//! Rust moved add-time default selection out of the filesystem adapter and into the add composition
//! layer. The first two tests therefore cross the real CLI boundary and read authoritative metadata
//! back through `FileStore`. The explicit-override test exercises the repository request directly:
//! once a caller supplies `store`, persistence must not rewrite it to the mode default.

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
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
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
            .current_dir(self.home.path());
        command
    }

    fn source(&self, name: &str) -> std::path::PathBuf {
        let path = self.home.path().join(format!("{name}.py"));
        fs::write(&path, "print(1)\n").unwrap();
        path
    }

    fn resolve(&self, selector: &str) -> skit_domain::Entry {
        FileStore::new(self.data.path()).resolve(selector).unwrap()
    }
}

#[test]
fn test_add_python_copy_mode_defaults_workdir_to_invoke() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("copy");
    sandbox
        .command()
        .arg("add")
        .arg(source)
        .args(["--name", "copy", "--no-input"])
        .assert()
        .success();

    assert_eq!(sandbox.resolve("copy").meta.workdir, "invoke");
}

#[test]
fn test_add_python_reference_mode_still_defaults_workdir_to_origin() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("linked");
    sandbox
        .command()
        .arg("add")
        .arg(source)
        .args(["--name", "linked", "--ref", "--no-input"])
        .assert()
        .success();

    assert_eq!(sandbox.resolve("linked").meta.workdir, "origin");
}

#[test]
fn test_add_python_copy_mode_explicit_workdir_override_still_respected() {
    let data = TempDir::new().unwrap();
    let store = FileStore::new(data.path());
    let request = CreateEntry {
        name: "explicit-workdir".to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: "/original/explicit-workdir.py".to_owned(),
        workdir: "store".to_owned(),
        description: String::new(),
        payload: Some(EntryPayload {
            bytes: b"print(1)\n".to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions::default(),
        }),
        settings: EntrySettings::default(),
    };

    let created = store.create(request).unwrap();
    assert_eq!(created.meta.workdir, "store");
    assert_eq!(
        store.resolve(created.slug.as_str()).unwrap().meta.workdir,
        "store"
    );
}
