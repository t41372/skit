//! Parameter-resync refusal ports from Python `tests/test_editor.py` at `main@206f9ef`.

use std::{fs, path::Path, process::Command};

use skit_store::FileStore;
use tempfile::TempDir;

struct CliRoots {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl CliRoots {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
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
fn test_params_edit_command_entry_refused() {
    let roots = CliRoots::new();
    let added = roots
        .command()
        .args(["add", "--cmd", "echo {x}", "--name", "ec", "--no-input"])
        .output()
        .unwrap();
    assert!(added.status.success(), "{}", combined(&added));

    let output = roots
        .command()
        .args(["params", "ec", "--resync"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
}

#[test]
fn test_params_edit_missing_copy_refused() {
    let roots = CliRoots::new();
    let source = roots.data.path().join("source.py");
    fs::write(&source, b"CITY = \"x\"\nprint(CITY)\n").unwrap();
    let directory = roots.data.path().join("scripts/a");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("script.py"), b"CITY = \"x\"\nprint(CITY)\n").unwrap();
    fs::write(
        directory.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = \"a\"\n",
                "kind = \"python\"\n",
                "mode = \"copy\"\n",
                "source = {source:?}\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-12T00:00:00Z\"\n",
                "id = \"0123456789abcdef0123456789abcdef\"\n",
                "workdir = \"origin\"\n",
                "description = \"\"\n",
            ),
            source = source.display().to_string(),
        ),
    )
    .unwrap();
    FileStore::new(roots.data.path()).rebuild_registry().unwrap();
    fs::remove_file(directory.join("script.py")).unwrap();

    let output = roots
        .command()
        .args(["params", "a", "--resync"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("no stored copy"), "{}", combined(&output));
}

#[allow(dead_code)]
fn _assert_path_type(_: &Path) {}
