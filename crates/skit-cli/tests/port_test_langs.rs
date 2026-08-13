//! Executable public-surface ports for Python `tests/test_langs.py` at `main@206f9ef`.
//!
//! Forward-compatible unknown kinds are created through the real `FileStore` and exercised through
//! the real CLI. Python-only lazy registry/capability internals are classified by the companion
//! manifest instead of being reimplemented in tests.

use std::{fs, path::{Path, PathBuf}};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository as _, EntryPayload, EntryRepository as _, SourcePermissions,
    canonical_stored_filename,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    empty_path: TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            empty_path: TempDir::new().unwrap(),
        };
        fs::write(
            fixture.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
        fixture
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
            .env("PATH", self.empty_path.path())
            .env("EDITOR", "__skit_test_no_editor__")
            .env("VISUAL", "__skit_test_no_editor__")
            .env_remove("FORCE_COLOR")
            .env_remove("NO_COLOR")
            .env_remove("CLICOLOR")
            .env_remove("CLICOLOR_FORCE")
            .current_dir(self.home.path());
        command
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn create_unknown(&self, name: &str, workdir: &str) -> PathBuf {
        let marker = self.home.path().join(format!("{name}-spawned"));
        let source = self.home.path().join(format!("{name}.src"));
        let body = format!(
            "#!/bin/sh\nprintf spawned > {}\n",
            marker.display()
        );
        fs::write(&source, body.as_bytes()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let permissions = permissions(&source);
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("martian").unwrap(),
                mode: StorageMode::Copy,
                source: source.display().to_string(),
                workdir: workdir.to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: body.into_bytes(),
                    stored_name: Some("payload".to_owned()),
                    permissions,
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
        marker
    }

    fn create_exe(&self, name: &str) {
        let source = PathBuf::from(env!("CARGO_BIN_EXE_skit"));
        let bytes = fs::read(&source).unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("exe").unwrap(),
                mode: StorageMode::Reference,
                source: source.display().to_string(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes,
                    stored_name: None,
                    permissions: permissions(&source),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
    }

    fn create_python(&self, name: &str) {
        let source = self.home.path().join(format!("{name}.py"));
        let bytes = b"print(1)\n".to_vec();
        fs::write(&source, &bytes).unwrap();
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                source: source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes,
                    stored_name: Some("script.py".to_owned()),
                    permissions: permissions(&source),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
    }
}

fn permissions(path: &Path) -> SourcePermissions {
    let metadata = fs::metadata(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        SourcePermissions {
            readonly: metadata.permissions().readonly(),
            unix_mode: Some(metadata.permissions().mode()),
        }
    }
    #[cfg(not(unix))]
    {
        SourcePermissions {
            readonly: metadata.permissions().readonly(),
            unix_mode: None,
        }
    }
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_stored_name_unknown_kind_falls_back_to_payload() {
    assert_eq!(canonical_stored_filename("martian"), Some("payload"));
    assert_eq!(canonical_stored_filename("python"), Some("script.py"));
    assert_eq!(canonical_stored_filename("exe"), None);
    assert_eq!(canonical_stored_filename("command"), None);
}

#[test]
fn test_unknown_kind_build_command_raises_clean_launch_error() {
    let fixture = Fixture::new();
    fixture.create_unknown("thing", "invoke");
    let output = fixture
        .command()
        .args(["run", "thing", "--dry-run", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(126), "{text}");
    assert!(text.contains("martian"), "unknown kind was not named: {text}");
    assert!(text.to_lowercase().contains("unknown entry kind"), "{text}");
}

#[test]
fn test_unknown_kind_run_entry_raises_before_spawning() {
    let fixture = Fixture::new();
    let marker = fixture.create_unknown("thing", "invoke");
    let output = fixture
        .command()
        .args(["run", "thing", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(126), "{text}");
    assert!(text.contains("martian"), "{text}");
    assert!(!marker.exists(), "unknown-kind payload was spawned before refusal");
}

#[test]
fn test_unknown_kind_never_reports_missing() {
    let fixture = Fixture::new();
    fixture.create_unknown("thing", "invoke");
    let entry = fixture.store().resolve("thing").unwrap();
    let payload = fixture.store().payload_path(&entry).unwrap();
    fs::remove_file(&payload).unwrap();
    assert!(!payload.exists());

    let output = fixture.command().arg("doctor").output().unwrap();
    let text = combined(&output);
    assert!(
        !text.contains("launch target is gone") && !text.contains("missing target"),
        "unknown kind was falsely reported as a missing launch target: {text}"
    );
}

#[test]
fn test_unknown_kind_preflight_still_checks_workdir() {
    let fixture = Fixture::new();
    let missing = fixture.home.path().join("no-such-workdir");
    fixture.create_unknown("thing", missing.to_str().unwrap());
    let output = fixture
        .command()
        .args(["run", "thing", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert!(!output.status.success(), "{text}");
    assert!(
        text.to_lowercase().contains("working directory") && text.contains("no-such-workdir"),
        "unknown-kind preflight skipped the workdir contract: {text}"
    );
}

#[test]
fn test_unknown_kind_script_path_uses_payload_fallback() {
    let fixture = Fixture::new();
    fixture.create_unknown("thing", "invoke");
    let entry = fixture.store().resolve("thing").unwrap();
    let path = fixture.store().payload_path(&entry).unwrap();
    assert_eq!(path.file_name().and_then(|name| name.to_str()), Some("payload"));
    assert!(path.is_file());
}

#[test]
fn test_params_exe_prints_plain_message_without_manage_dead_end() {
    let fixture = Fixture::new();
    fixture.create_exe("prog");
    let output = fixture.command().args(["params", "prog"]).output().unwrap();
    let text = combined(&output);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("has no managed parameters"), "{text}");
    assert!(!text.contains("--manage"), "exe guidance advertised a dead --manage path: {text}");
}

#[test]
fn test_doctor_missing_uv_pure_exe_library_exits_zero() {
    let fixture = Fixture::new();
    fixture.create_exe("prog");
    let output = fixture.command().arg("doctor").output().unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(0), "{text}");
    assert!(text.to_lowercase().contains("uv"), "doctor omitted uv status: {text}");
}

#[test]
fn test_doctor_missing_uv_with_python_entry_exits_one() {
    let fixture = Fixture::new();
    fixture.create_python("a");
    let output = fixture.command().arg("doctor").output().unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(text.to_lowercase().contains("uv"), "doctor omitted the missing uv problem: {text}");
}

#[test]
fn test_doctor_json_missing_uv_pure_exe_library_exits_zero() {
    let fixture = Fixture::new();
    fixture.create_exe("prog");
    let output = fixture
        .command()
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(0), "{text}");
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("doctor --json was not JSON: {error}; {text}"));
    assert!(payload.is_object() || payload.is_array(), "unexpected doctor JSON shape: {payload}");
}
