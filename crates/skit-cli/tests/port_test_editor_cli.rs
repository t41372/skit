//! Existing-entry `skit edit` ports from Python `tests/test_editor.py` at `main@206f9ef`.
//!
//! These tests use a real child editor probe instead of mocking the Rust host. This preserves the
//! Python contract at a stronger public boundary: the exact path passed to the editor, pre-launch
//! refusal for a missing reference, user-visible success copy, and launch-failure propagation.

#![cfg(unix)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use skit_store::FileStore;
use tempfile::TempDir;

struct EditFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    source: PathBuf,
    capture: PathBuf,
}

impl EditFixture {
    fn new(mode: &str) -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let source = data.path().join("orig.py");
        fs::write(&source, b"print(1)\n").unwrap();
        let directory = data.path().join("scripts/a");
        fs::create_dir_all(&directory).unwrap();
        if mode == "copy" {
            fs::write(directory.join("script.py"), b"print(1)\n").unwrap();
        }
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = \"a\"\n",
                    "kind = \"python\"\n",
                    "mode = {mode:?}\n",
                    "source = {source:?}\n",
                    "source_hash = \"\"\n",
                    "added_at = \"2026-08-12T00:00:00Z\"\n",
                    "id = \"0123456789abcdef0123456789abcdef\"\n",
                    "workdir = \"origin\"\n",
                    "description = \"\"\n",
                ),
                mode = mode,
                source = source.display().to_string(),
            ),
        )
        .unwrap();
        FileStore::new(data.path()).rebuild_registry().unwrap();
        let capture = tools.path().join("capture.txt");
        Self {
            data,
            state,
            config,
            tools,
            source,
            capture,
        }
    }

    fn probe(&self) -> PathBuf {
        let source = self.tools.path().join("probe.rs");
        fs::write(
            &source,
            r#"
use std::{env, fs};
fn main() {
    let capture = env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path");
    let text = env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(capture, format!("{text}\n")).expect("write capture");
}
"#,
        )
        .unwrap();
        let executable = self.tools.path().join("editor-probe");
        let status = Command::new("rustc")
            .arg(source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap();
        assert!(status.success(), "failed to compile editor probe");
        executable
    }

    fn run(&self, editor: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_skit"))
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env_remove("VISUAL")
            .env("EDITOR", editor)
            .env("SKIT_EDITOR_CAPTURE", &self.capture)
            .args(["edit", "a"])
            .output()
            .unwrap()
    }

    fn captured_target(&self) -> PathBuf {
        let lines = fs::read_to_string(&self.capture).unwrap();
        let argv = lines.lines().collect::<Vec<_>>();
        assert_eq!(argv.len(), 2, "editor must receive only its argv[0] and the source path: {argv:?}");
        PathBuf::from(argv[1])
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_edit_opens_copy_source() {
    let fixture = EditFixture::new("copy");
    let editor = fixture.probe();

    let output = fixture.run(&editor);
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(
        fixture.captured_target(),
        fixture.data.path().join("scripts/a/script.py"),
        "copy-mode edit must open the stored copy itself"
    );
    assert!(combined(&output).contains("Saved a"), "{}", combined(&output));
}

#[test]
fn test_edit_opens_reference_original() {
    let fixture = EditFixture::new("reference");
    let editor = fixture.probe();

    let output = fixture.run(&editor);
    assert!(output.status.success(), "{}", combined(&output));
    assert_eq!(
        fixture.captured_target(),
        fixture.source.canonicalize().unwrap(),
        "reference-mode edit must open the original source"
    );
}

#[test]
fn test_edit_reference_source_gone() {
    let fixture = EditFixture::new("reference");
    let editor = fixture.probe();
    fs::remove_file(&fixture.source).unwrap();

    let output = fixture.run(&editor);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(combined(&output).contains("gone"), "{}", combined(&output));
    assert!(
        !fixture.capture.exists(),
        "the editor was launched even though the reference source was already gone"
    );
}

#[test]
fn test_edit_reports_editor_launch_failure() {
    let fixture = EditFixture::new("copy");
    let missing_editor = fixture.tools.path().join("editor-does-not-exist");

    let output = fixture.run(&missing_editor);
    assert_eq!(output.status.code(), Some(1), "{}", combined(&output));
    assert!(
        combined(&output).contains("could not launch"),
        "the public CLI must surface the editor launch failure: {}",
        combined(&output)
    );
    assert!(!fixture.capture.exists());
}
