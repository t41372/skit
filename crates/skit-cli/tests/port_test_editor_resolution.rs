//! Public-process ports of the editor-resolution slice in Python `tests/test_editor.py` at
//! `main@206f9ef`.
//!
//! The Python module tests the private `resolve_editor()` helper directly. Rust does not expose an
//! equivalent helper, so these tests prove the same contract through `skit edit`: a real executable
//! probe records which editor won precedence and exactly which arguments reached the process.
//!
//! Thirteen of the fourteen Python resolution contracts have an observable process-level seam.
//! `test_resolve_editor_windows_empty_quoted_token_strips_to_empty` is intentionally not mapped:
//! once `argv[0]` is either `""` or `""`, the public Rust edit boundary exposes only launch failure,
//! so it cannot distinguish the Python helper's empty-token shape without inventing a test-only
//! production API. Do not replace that blocked contract with a weaker launch-error assertion.

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use skit_store::{FileConfigStore, FileStore};
use tempfile::TempDir;

struct EditFixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    tools: TempDir,
    capture: PathBuf,
}

impl EditFixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let source = data.path().join("source.py");
        fs::write(&source, b"print('source')\n").unwrap();
        let directory = data.path().join("scripts/a");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("script.py"), b"print('stored')\n").unwrap();
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
        FileStore::new(data.path()).rebuild_registry().unwrap();
        let capture = tools.path().join("capture.txt");
        Self {
            data,
            state,
            config,
            tools,
            capture,
        }
    }

    fn set_editor(&self, value: &str) {
        FileConfigStore::new(self.config.path())
            .set("editor", value)
            .unwrap();
    }

    fn hand_edit_editor(&self, value: &str) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(
            self.config.path().join("config.toml"),
            format!("editor = {value:?}\n"),
        )
        .unwrap();
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("SKIT_EDITOR_CAPTURE", &self.capture)
            .arg("edit")
            .arg("a")
            .arg("--no-input");
        command
    }

    fn run(&self, visual: Option<&OsStr>, editor: Option<&OsStr>, path: Option<&OsStr>) -> Output {
        let mut command = self.command();
        match visual {
            Some(value) => {
                command.env("VISUAL", value);
            }
            None => {
                command.env_remove("VISUAL");
            }
        }
        match editor {
            Some(value) => {
                command.env("EDITOR", value);
            }
            None => {
                command.env_remove("EDITOR");
            }
        }
        if let Some(value) = path {
            command.env("PATH", value);
        }
        command.output().unwrap()
    }

    fn captured(&self) -> Vec<String> {
        fs::read_to_string(&self.capture)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

fn compile_probe(root: &Path, output: &Path) {
    let source = root.join("editor_probe.rs");
    if !source.exists() {
        fs::write(
            &source,
            r#"
use std::{env, fs};
fn main() {
    let capture = env::var_os("SKIT_EDITOR_CAPTURE").expect("SKIT_EDITOR_CAPTURE");
    let text = env::args_os()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(capture, format!("{text}\n")).expect("write editor capture");
}
"#,
        )
        .unwrap();
    }
    let status = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(output)
        .status()
        .unwrap();
    assert!(status.success(), "failed to compile editor probe");
}

fn probe(fixture: &EditFixture, name: &str) -> PathBuf {
    let path = fixture.tools.path().join(name);
    compile_probe(fixture.tools.path(), &path);
    path
}

fn probe_with_spaces(fixture: &EditFixture, name: &str) -> PathBuf {
    let directory = fixture.tools.path().join("directory with spaces");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(name);
    compile_probe(fixture.tools.path(), &path);
    path
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status={:?}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn captured_program_name(fixture: &EditFixture) -> String {
    Path::new(&fixture.captured()[0])
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn test_resolve_editor_config_wins_over_env() {
    let fixture = EditFixture::new();
    let configured = probe(&fixture, executable_name("configured"));
    let visual = probe(&fixture, executable_name("visual"));
    let editor = probe(&fixture, executable_name("editor"));
    fixture.set_editor(&format!("{} --wait", quote_path(&configured)));

    let output = fixture.run(Some(visual.as_os_str()), Some(editor.as_os_str()), None);
    assert_success(&output);
    let captured = fixture.captured();
    assert_eq!(Path::new(&captured[0]), configured);
    assert_eq!(captured[1], "--wait");
}

#[test]
fn test_resolve_editor_visual_over_editor() {
    let fixture = EditFixture::new();
    let visual = probe(&fixture, executable_name("visual"));
    let editor = probe(&fixture, executable_name("editor"));

    let output = fixture.run(Some(visual.as_os_str()), Some(editor.as_os_str()), None);
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), visual);
}

#[test]
fn test_resolve_editor_editor_env_when_no_visual() {
    let fixture = EditFixture::new();
    let editor = probe(&fixture, executable_name("editor"));

    let output = fixture.run(None, Some(editor.as_os_str()), None);
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), editor);
}

#[cfg(unix)]
#[test]
fn test_resolve_editor_platform_default_unix() {
    let fixture = EditFixture::new();
    let default = probe(&fixture, "vi");

    let output = fixture.run(None, None, Some(fixture.tools.path().as_os_str()));
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), default);
}

#[cfg(windows)]
#[test]
fn test_resolve_editor_platform_default_windows() {
    let fixture = EditFixture::new();
    let default = probe(&fixture, "notepad.exe");
    fixture.set_editor("");

    let output = fixture.run(None, None, Some(fixture.tools.path().as_os_str()));
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), default);
}

#[cfg(not(windows))]
#[test]
fn test_resolve_editor_quoted_value_uses_posix_split_off_windows() {
    let fixture = EditFixture::new();
    let configured = probe_with_spaces(&fixture, executable_name("my editor"));
    fixture.set_editor(&format!("{} --wait", quote_path(&configured)));

    let output = fixture.run(None, None, None);
    assert_success(&output);
    let captured = fixture.captured();
    assert_eq!(Path::new(&captured[0]), configured);
    assert_eq!(captured[1], "--wait");
}

#[cfg(windows)]
#[test]
fn test_resolve_editor_quoted_value_non_posix_on_windows() {
    let fixture = EditFixture::new();
    let configured = probe(&fixture, "edit.exe");
    fixture.set_editor(&format!("{} --wait", configured.display()));

    let output = fixture.run(None, None, None);
    assert_success(&output);
    let captured = fixture.captured();
    assert_eq!(Path::new(&captured[0]), configured);
    assert_eq!(captured[1], "--wait");
}

#[cfg(windows)]
#[test]
fn test_resolve_editor_quoted_spaced_path_on_windows() {
    let fixture = EditFixture::new();
    let configured = probe_with_spaces(&fixture, "Code.exe");
    fixture.set_editor(&format!("\"{}\" --wait", configured.display()));

    let output = fixture.run(None, None, None);
    assert_success(&output);
    let captured = fixture.captured();
    assert_eq!(Path::new(&captured[0]), configured);
    assert_eq!(captured[1], "--wait");
}

#[cfg(windows)]
#[test]
fn test_resolve_editor_unquoted_windows_path_untouched() {
    let fixture = EditFixture::new();
    let configured = probe(&fixture, "edit.exe");
    fixture.set_editor(&format!("{} --wait", configured.display()));

    let output = fixture.run(None, None, None);
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), configured);
}

#[test]
fn test_resolve_editor_whitespace_visual_falls_through_to_editor() {
    let fixture = EditFixture::new();
    let editor = probe(&fixture, executable_name("editor"));

    let output = fixture.run(Some(OsStr::new("   ")), Some(editor.as_os_str()), None);
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), editor);
}

#[test]
fn test_resolve_editor_whitespace_config_falls_through_to_visual() {
    let fixture = EditFixture::new();
    let visual = probe(&fixture, executable_name("visual"));
    let editor = probe(&fixture, executable_name("editor"));
    fixture.hand_edit_editor("   ");

    let output = fixture.run(Some(visual.as_os_str()), Some(editor.as_os_str()), None);
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), visual);
}

#[cfg(unix)]
#[test]
fn test_resolve_editor_all_whitespace_candidates_use_platform_default() {
    let fixture = EditFixture::new();
    let default = probe(&fixture, "vi");
    fixture.hand_edit_editor("  ");

    let output = fixture.run(
        Some(OsStr::new(" ")),
        Some(OsStr::new("")),
        Some(fixture.tools.path().as_os_str()),
    );
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), default);
}

#[cfg(windows)]
#[test]
fn test_resolve_editor_all_whitespace_candidates_use_platform_default() {
    let fixture = EditFixture::new();
    let default = probe(&fixture, "notepad.exe");
    fixture.hand_edit_editor("  ");

    let output = fixture.run(
        Some(OsStr::new(" ")),
        Some(OsStr::new("")),
        Some(fixture.tools.path().as_os_str()),
    );
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), default);
}

#[cfg(unix)]
#[test]
fn test_resolve_editor_unbalanced_quotes_falls_back_to_raw() {
    let fixture = EditFixture::new();
    let raw_name = "weird \"editor";
    let expected = probe(&fixture, raw_name);

    let output = fixture.run(None, Some(OsStr::new(raw_name)), Some(fixture.tools.path().as_os_str()));
    assert_success(&output);
    assert_eq!(Path::new(&fixture.captured()[0]), expected);
}

#[cfg(windows)]
#[test]
fn test_resolve_editor_unbalanced_quotes_falls_back_to_raw() {
    // Windows file names cannot contain a literal quote, so the public launch seam cannot create the
    // exact raw fallback executable used by the Python helper test. Keep the contract visibly red
    // rather than claiming a weaker launch-error mapping is equivalent.
    panic!("BLOCKED parity contract: public editor launch cannot distinguish the raw quoted token on Windows");
}

#[cfg(windows)]
const _: () = {
    // Python `test_resolve_editor_windows_empty_quoted_token_strips_to_empty` remains intentionally
    // unmapped. Public process launch cannot distinguish empty argv[0] from a literal quoted token.
};

#[cfg(windows)]
fn executable_name(stem: &str) -> &str {
    match stem {
        "configured" => "configured.exe",
        "visual" => "visual.exe",
        "editor" => "editor.exe",
        "my editor" => "my editor.exe",
        other => other,
    }
}

#[cfg(not(windows))]
fn executable_name(stem: &str) -> &str {
    stem
}

fn quote_path(path: &Path) -> String {
    format!("\"{}\"", path.display())
}
