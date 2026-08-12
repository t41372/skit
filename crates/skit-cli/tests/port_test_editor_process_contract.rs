//! Process-level ports of Python's private `editor.open_in_editor` contracts from
//! `tests/test_editor.py` at `main@206f9ef`.
//!
//! Rust keeps editor launch private inside the CLI composition root, so these use the stronger public
//! boundary: a real `skit edit`, a real compiled editor child, exact argv capture, and the final CLI
//! disposition. In particular Python deliberately treats a non-zero editor exit as a completed edit;
//! changing that to a skit failure is a behavior regression even though the Rust helper has a
//! different return type.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use skit_store::{FileConfigStore, FileStore};
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    capture: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let original = data.path().join("original.py");
        fs::write(&original, b"print(1)\n").unwrap();
        let directory = data.path().join("scripts/a");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("script.py"), b"print(1)\n").unwrap();
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
                    "id = \"6123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"origin\"\n",
                    "description = \"\"\n",
                ),
                source = original.display().to_string(),
            ),
        )
        .unwrap();
        FileStore::new(data.path()).rebuild_registry().unwrap();
        let capture = tools.path().join("argv.txt");
        Self {
            data,
            state,
            config,
            tools,
            capture,
        }
    }

    fn configured_editor(&self, command: &str) {
        FileConfigStore::new(self.config.path())
            .set("editor", command)
            .unwrap();
    }

    fn probe(&self) -> PathBuf {
        let source = self.tools.path().join("editor_process_probe.rs");
        fs::write(
            &source,
            r#"
use std::{env, fs, process};
fn main() {
    let capture = env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path");
    let argv = env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    fs::write(capture, format!("{}\n", argv.join("\n"))).expect("write capture");
    let code = env::var("SKIT_EDITOR_EXIT")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(0);
    process::exit(code);
}
"#,
        )
        .unwrap();
        let executable = self.tools.path().join(probe_executable_name());
        let status = Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success(), "failed to compile editor process probe");
        executable
    }

    fn run(&self, editor_exit: Option<i32>, path: Option<&Path>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("SKIT_EDITOR_CAPTURE", &self.capture)
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .args(["edit", "a", "--no-input"]);
        match editor_exit {
            Some(code) => {
                command.env("SKIT_EDITOR_EXIT", code.to_string());
            }
            None => {
                command.env_remove("SKIT_EDITOR_EXIT");
            }
        }
        if let Some(path) = path {
            command.env("PATH", path);
        }
        command.output().unwrap()
    }

    fn argv(&self) -> Vec<String> {
        fs::read_to_string(&self.capture)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(windows)]
fn probe_executable_name() -> &'static str {
    "editor-process-probe.exe"
}

#[cfg(not(windows))]
fn probe_executable_name() -> &'static str {
    "editor-process-probe"
}

fn quote(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_open_in_editor_appends_path_and_returns_code() {
    let fixture = Fixture::new();
    let probe = fixture.probe();
    fixture.configured_editor(&format!("{} --wait", quote(&probe)));

    let output = fixture.run(Some(0), None);
    assert!(output.status.success(), "{}", combined(&output));
    let argv = fixture.argv();
    assert_eq!(
        argv,
        [
            probe.display().to_string(),
            "--wait".to_owned(),
            fixture.data.path().join("scripts/a/script.py").display().to_string(),
        ],
        "the editor command prefix must be preserved and the edited path appended exactly once"
    );
    assert!(combined(&output).contains("Saved a"), "{}", combined(&output));
}

#[test]
fn test_open_in_editor_returns_nonzero_without_raising() {
    let fixture = Fixture::new();
    let probe = fixture.probe();
    fixture.configured_editor(&quote(&probe));

    let output = fixture.run(Some(3), None);
    // Python's editor helper returns 3 and `skit edit` intentionally ignores that ordinary editor
    // status: some editors use non-zero for an unmodified close. The public operation still reports
    // the edit as completed. Rust must not upgrade that child status into a skit/usage failure.
    assert!(
        output.status.success(),
        "editor exit 3 was incorrectly promoted to a skit failure: {}",
        combined(&output)
    );
    assert!(combined(&output).contains("Saved a"), "{}", combined(&output));
    let argv = fixture.argv();
    assert_eq!(argv.len(), 2, "the non-zero editor must still receive one target: {argv:?}");
    assert_eq!(
        PathBuf::from(&argv[1]),
        fixture.data.path().join("scripts/a/script.py")
    );
}

#[test]
fn test_open_in_editor_launch_failure_message_exact() {
    let fixture = Fixture::new();
    fixture.configured_editor("code --wait");
    // Force the configured command to be absent even on a developer machine that happens to have
    // VS Code installed. The skit binary itself is already selected by an absolute cargo path.
    let empty_path = fixture.tools.path().join("empty-path");
    fs::create_dir(&empty_path).unwrap();

    let output = fixture.run(None, Some(&empty_path));
    assert!(!output.status.success(), "a missing editor unexpectedly succeeded");
    let text = combined(&output);
    assert!(
        text.contains("Could not launch the editor (code --wait):"),
        "the launch failure did not identify the resolved editor command: {text}"
    );
    assert!(
        text.contains("skit config editor <cmd>"),
        "the launch failure lost the Python oracle's recovery instruction: {text}"
    );
    assert!(!text.contains("XX"), "mutation sentinel leaked into the message: {text}");
    assert!(!fixture.capture.exists(), "the missing editor somehow executed");
}
