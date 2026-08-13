//! Public behavior ports from Python v0.4 `tests/test_store.py` at `main@206f9ef`.
//!
//! Python's old store module combined application composition with filesystem persistence. Rust
//! splits those layers, so user-facing add/remove/deps/doctor contracts go through the real CLI and
//! are read back through `FileStore`; repository-only resolve/scan contracts use `FileStore`
//! directly. Assertions retain the Python v0.4 contract even when current Rust is red.

use std::{fs, path::PathBuf};

use assert_cmd::Command;
use skit_application::{EntryRepository as _, RepositoryError};
use skit_domain::{EntrySettings, StorageMode};
use skit_store::{FileStore, content_hash};
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
        fs::write(
            sandbox.config.path().join("config.toml"),
            "[mirror]\nenabled = false\n",
        )
        .unwrap();
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

    fn sample_python(&self, name: &str) -> PathBuf {
        let path = self.home.path().join(format!("{name}.py"));
        fs::write(
            &path,
            "\"\"\"打招呼腳本。\n\n多行 docstring。\"\"\"\nNAME = \"world\"\nprint(f\"hi {NAME}\")\n",
        )
        .unwrap();
        path
    }

    fn add(&self, source: &std::path::Path, args: &[&str]) -> std::process::Output {
        self.command()
            .arg("add")
            .arg(source)
            .args(args)
            .arg("--no-input")
            .output()
            .unwrap()
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
fn test_add_copy_preserves_original_verbatim() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("hello");
    let original = fs::read(&source).unwrap();
    let output = sandbox.add(&source, &["--name", "hello"]);
    assert!(output.status.success(), "{}", combined(&output));

    let entry = sandbox.store().resolve("hello").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "python");
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    let stored = sandbox.data.path().join("scripts/hello/script.py");
    assert_eq!(fs::read(stored).unwrap(), original);
    assert_eq!(entry.meta.description, "打招呼腳本。");
    assert_eq!(entry.meta.source, source.canonicalize().unwrap().display().to_string());
    assert_eq!(entry.meta.source_hash, content_hash(&original));
}

#[test]
fn test_add_reference_points_to_origin() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("hello");
    let output = sandbox.add(&source, &["--name", "hello", "--ref"]);
    assert!(output.status.success(), "{}", combined(&output));
    let entry = sandbox.store().resolve("hello").unwrap();
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.source, source.canonicalize().unwrap().display().to_string());
    assert!(!sandbox.data.path().join("scripts/hello/script.py").exists());
}

#[test]
fn test_name_conflict_rejected() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("hello");
    assert!(sandbox.add(&source, &["--name", "hello"]).status.success());
    let second = sandbox.add(&source, &["--name", "hello"]);
    assert_eq!(second.status.code(), Some(2), "{}", combined(&second));
    assert!(matches!(
        sandbox.store().scan().unwrap().entries.as_slice(),
        [_]
    ));
}

#[test]
fn test_slug_dedup() {
    let sandbox = Sandbox::new();
    let first = sandbox.sample_python("one");
    let second = sandbox.sample_python("two");
    assert!(sandbox.add(&first, &["--name", "任務A"]).status.success());
    assert!(sandbox.add(&second, &["--name", "任務B"]).status.success());
    let entries = sandbox.store().scan().unwrap().entries;
    assert_eq!(entries.len(), 2);
    let slugs = entries.iter().map(|entry| entry.slug.as_str()).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(slugs.len(), 2);
    assert!(entries.iter().all(|entry| !entry.slug.as_str().is_empty()));
}

#[test]
fn test_resolve_and_remove() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("hello");
    assert!(sandbox.add(&source, &["--name", "hi"]).status.success());
    let entry = sandbox.store().resolve("hi").unwrap();
    assert_eq!(sandbox.store().resolve(entry.slug.as_str()).unwrap().meta.name, "hi");
    sandbox.command().args(["remove", "hi", "--yes", "--no-input"]).assert().success();
    assert!(matches!(sandbox.store().resolve("hi"), Err(RepositoryError::NotFound { .. })));
    assert!(!sandbox.data.path().join("scripts").join(entry.slug.as_str()).exists());
}

#[test]
fn test_remove_copy_does_not_touch_original() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("hello");
    assert!(sandbox.add(&source, &["--name", "hi"]).status.success());
    sandbox.command().args(["remove", "hi", "--yes", "--no-input"]).assert().success();
    assert!(source.exists());
}

#[test]
fn test_add_command_entry() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--cmd", "echo {msg}", "--name", "回聲", "--no-input"])
        .assert()
        .success();
    let entry = sandbox.store().resolve("回聲").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "command");
    assert_eq!(EntrySettings::from_meta(&entry.meta).template, "echo {msg}");
    assert_eq!(entry.meta.workdir, "invoke");
}

#[test]
fn test_command_requires_nonempty_template() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "--cmd", "   ", "--name", "空", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_doctor_rebuild_from_meta() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("a");
    assert!(sandbox.add(&source, &["--name", "a"]).status.success());
    sandbox
        .command()
        .args(["add", "--cmd", "echo hi", "--name", "b", "--no-input"])
        .assert()
        .success();
    fs::remove_file(sandbox.data.path().join("registry.toml")).unwrap();
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
    sandbox.command().args(["doctor", "--rebuild"]).assert().success();
    let names = sandbox
        .store()
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|entry| entry.name)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names, ["a".to_owned(), "b".to_owned()].into_iter().collect());
}

#[test]
fn test_doctor_reports_missing_reference() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("ref");
    assert!(sandbox.add(&source, &["--name", "ref", "--ref"]).status.success());
    fs::remove_file(&source).unwrap();
    let output = sandbox.command().args(["doctor", "--rebuild"]).output().unwrap();
    let text = combined(&output);
    assert!(text.contains("ref"), "{text}");
    assert!(text.contains(&source.display().to_string()), "{text}");
}

#[test]
fn test_syntax_error_script_still_addable() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("bad.py");
    fs::write(&source, "def broken(:\n").unwrap();
    let output = sandbox.add(&source, &["--name", "bad"]);
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(sandbox.store().resolve("bad").unwrap().meta.description, "");
}

#[test]
fn test_add_python_missing_file_raises() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("ghost.py");
    let output = sandbox.add(&missing, &["--name", "ghost"]);
    assert_ne!(output.status.code(), Some(0), "missing source unexpectedly registered");
    assert!(combined(&output).to_ascii_lowercase().contains("not found"), "{}", combined(&output));
}

#[test]
fn test_add_exe_roundtrip() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("mytool");
    fs::write(&source, b"payload").unwrap();
    let output = sandbox.add(&source, &["--name", "mytool", "--exe", "--description", "a tool"]);
    assert!(output.status.success(), "{}", combined(&output));
    let entry = sandbox.store().resolve("mytool").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "exe");
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.description, "a tool");
}

#[test]
fn test_add_exe_missing_file_raises() {
    let sandbox = Sandbox::new();
    let output = sandbox.add(&sandbox.home.path().join("no_such_tool"), &["--name", "tool", "--exe"]);
    assert_ne!(output.status.code(), Some(0));
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_list_entries_skips_corrupt_meta() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--cmd", "echo hi", "--name", "good", "--no-input"])
        .assert()
        .success();
    let bad = sandbox.data.path().join("scripts/bad-slug");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("meta.toml"), "not valid toml [[[").unwrap();
    let scan = sandbox.store().scan().unwrap();
    assert_eq!(scan.entries.len(), 1);
    assert_eq!(scan.entries[0].name, "good");
}

#[test]
fn test_doctor_rebuild_corrupt_meta() {
    let sandbox = Sandbox::new();
    let missing = sandbox.data.path().join("scripts/orphan");
    fs::create_dir_all(&missing).unwrap();
    let corrupt = sandbox.data.path().join("scripts/corrupt");
    fs::create_dir_all(&corrupt).unwrap();
    fs::write(corrupt.join("meta.toml"), "[[[bad").unwrap();
    let output = sandbox.command().args(["doctor", "--rebuild"]).output().unwrap();
    let text = combined(&output);
    assert!(text.contains("orphan"), "{text}");
    assert!(text.contains("corrupt"), "{text}");
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_update_dependencies_copy_mode() {
    let sandbox = Sandbox::new();
    let source = sandbox.sample_python("deps");
    assert!(sandbox.add(&source, &["--name", "deps"]).status.success());
    sandbox
        .command()
        .args(["deps", "deps", "--dep", "httpx", "--python", ">=3.11"])
        .assert()
        .success();
    let text = fs::read_to_string(sandbox.data.path().join("scripts/deps/script.py")).unwrap();
    assert!(text.contains("httpx"), "{text}");
    assert!(text.contains(">=3.11"), "{text}");
}

#[test]
fn test_resolve_not_found_raises() {
    let sandbox = Sandbox::new();
    assert!(matches!(
        sandbox.store().resolve("nonexistent"),
        Err(RepositoryError::NotFound { .. })
    ));
}
