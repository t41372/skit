//! Post-editor validation port from Python `tests/test_editor.py` at `main@206f9ef`.
//!
//! Python's helper test asserts the typed `FileNotFoundError` cause directly. Rust does not expose
//! that internal error object across the CLI boundary, so this test keeps the observable contract:
//! a real editor deletes the exact path it was given, exits successfully, and skit must then fail
//! cleanly while naming that path and reporting the Python `Can't read` user-facing semantics.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use skit_store::FileStore;
use tempfile::TempDir;

struct RemovedTargetFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    _tools: TempDir,
    capture: PathBuf,
    editor: PathBuf,
}

impl RemovedTargetFixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("removed-target.txt");
        let editor = compile_removing_editor(tools.path());

        let source = data.path().join("review.prompt.md");
        fs::write(&source, b"Review this\n").unwrap();
        let directory = data.path().join("scripts/review");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("prompt.md"), b"Review this\n").unwrap();
        fs::write(
            directory.join("meta.toml"),
            format!(
                concat!(
                    "schema = 1\n",
                    "name = \"review\"\n",
                    "kind = \"prompt\"\n",
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
        FileStore::new(data.path()).rebuild_registry().unwrap();

        Self {
            data,
            state,
            config,
            _tools: tools,
            capture,
            editor,
        }
    }

    fn run(&self) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_skit"))
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("VISUAL", &self.editor)
            .env("EDITOR", &self.editor)
            .env("SKIT_EDITOR_CAPTURE", &self.capture)
            .args(["edit", "review"])
            .output()
            .unwrap()
    }
}

fn compile_removing_editor(root: &Path) -> PathBuf {
    let source = root.join("removing_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("editor target"));
    fs::write(
        env::var_os("SKIT_EDITOR_CAPTURE").expect("capture path"),
        target.to_string_lossy().as_bytes(),
    )
    .expect("capture target");
    fs::remove_file(target).expect("remove editor target");
}
"#,
    )
    .unwrap();
    let output = root.join(editor_name());
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&output)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile removing editor probe");
    output
}

#[cfg(windows)]
fn editor_name() -> &'static str {
    "removing-editor.exe"
}

#[cfg(not(windows))]
fn editor_name() -> &'static str {
    "removing-editor"
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn test_open_entry_prompt_removed_by_editor_is_a_clean_edited_source_error() {
    let fixture = RemovedTargetFixture::new();
    let output = fixture.run();
    let text = combined(&output);
    let removed = fs::read_to_string(&fixture.capture)
        .unwrap_or_else(|error| panic!("editor did not capture the removed target: {error}"));

    assert_eq!(output.status.code(), Some(1), "{text}");
    assert!(
        text.contains("Can't read"),
        "post-edit read failure lost Python user semantics: {text}"
    );
    assert!(
        text.contains(&removed),
        "post-edit read failure did not name the editor target {removed:?}: {text}"
    );
    assert!(
        !Path::new(&removed).exists(),
        "editor target unexpectedly survived deletion"
    );
}
