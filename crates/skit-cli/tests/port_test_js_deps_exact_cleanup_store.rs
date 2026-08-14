//! Exact dependency-clear transaction ports from Python v0.4 `tests/test_js_deps.py`.

use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryRepository as _, EntryPayload,
    SourcePermissions, payload_stored_name,
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
        let store = sandbox.store();
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
                    ..EntrySettings::default()
                },
            })
            .unwrap();
        sandbox
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn paths(&self) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        (
            self.data.path().to_path_buf(),
            self.state.path().to_path_buf(),
            self.config.path().to_path_buf(),
            self.home.path().to_path_buf(),
        )
    }

    fn command(&self) -> Command {
        let (data, state, config, home) = self.paths();
        command_for(data, state, config, home)
    }

    fn entry_dir(&self) -> PathBuf {
        let store = self.store();
        let entry = store.resolve("t").unwrap();
        store.entry_dir_path(&entry.slug)
    }
}

fn command_for(data: PathBuf, state: PathBuf, config: PathBuf, home: PathBuf) -> Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data)
        .env("SKIT_STATE_DIR", state)
        .env("SKIT_CONFIG_DIR", config)
        .env("SKIT_LANG", "en")
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .current_dir(home);
    command
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_store_clear_goes_through_the_locked_entry_point() {
    let sandbox = Sandbox::new();
    let lock_path = sandbox.data.path().join(".locks/t.skit-deps.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    lock.lock().unwrap();

    let (data, state, config, home) = sandbox.paths();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let output = command_for(data, state, config, home)
            .args(["deps", "t", "--clear"])
            .output()
            .unwrap();
        done_tx.send(output).unwrap();
    });

    assert!(
        done_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "deps --clear bypassed the live JavaScript dependency lock"
    );
    drop(lock);
    let output = done_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let fresh = sandbox.store().resolve("t").unwrap();
    assert!(EntrySettings::from_meta(&fresh.meta).dependencies.is_empty());
}

#[test]
fn test_update_dependencies_surfaces_clean_failure_as_store_error() {
    let sandbox = Sandbox::new();
    let entry_dir = sandbox.entry_dir();
    fs::write(entry_dir.join(".skit-deps"), "v1\nnode\ndeadbeef\n").unwrap();
    fs::create_dir(entry_dir.join("package.json")).unwrap();

    let output = sandbox
        .command()
        .args(["deps", "t", "--clear"])
        .output()
        .unwrap();
    let rendered = combined(&output);
    assert_ne!(output.status.code(), Some(0), "{rendered}");
    assert!(rendered.contains("package.json"), "{rendered}");
    assert!(rendered.contains("directory"), "{rendered}");

    let fresh = sandbox.store().resolve("t").unwrap();
    assert_eq!(
        EntrySettings::from_meta(&fresh.meta).dependencies,
        ["chalk"],
        "dependency metadata committed even though the old environment could not be cleared"
    );
}