use std::{fs, path::{Path, PathBuf}, process::{Command, Output}};

use skit_store::FileConfigStore;
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
            tools: TempDir::new().expect("tools"),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_skit"))
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .current_dir(self.home.path())
            .args(args)
            .output()
            .expect("run skit")
    }

    fn source(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("source parent");
        }
        fs::write(&path, body).expect("source");
        path
    }

    fn editor(&self) -> PathBuf {
        let source = self.tools.path().join("editor_probe.rs");
        fs::write(&source, "fn main() {}\n").expect("editor source");
        let executable = self.tools.path().join(if cfg!(windows) {
            "editor-probe.exe"
        } else {
            "editor-probe"
        });
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let status = Command::new(rustc)
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .expect("compile editor probe");
        assert!(status.success(), "editor probe compile failed: {status}");
        executable
    }

    fn configure_editor(&self) {
        let editor = self.editor();
        FileConfigStore::new(self.config.path())
            .set("editor", &format!("\"{}\"", editor.display()))
            .expect("configure editor");
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

#[test]
fn test_edit_reports_escape_markup_in_name() {
    let fixture = Fixture::new();
    let source = fixture.source("job.py", "print(1)\n");
    let added = fixture.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "[blue]a[/blue]",
        "--no-input",
    ]);
    assert_success(&added);
    fixture.configure_editor();

    let output = fixture.run(&["edit", "[blue]a[/blue]"]);
    assert_success(&output);
    assert!(combined(&output).contains("[blue]a[/blue]"), "{}", combined(&output));
}

#[test]
fn test_edit_reference_mode_escapes_markup_in_name_and_path() {
    let fixture = Fixture::new();
    let source = fixture.source("[red]weird[bold]/job.py", "print(1)\n");
    let added = fixture.run(&[
        "add",
        source.to_str().unwrap(),
        "--ref",
        "--name",
        "refjob",
        "--no-input",
    ]);
    assert_success(&added);
    fixture.configure_editor();

    let output = fixture.run(&["edit", "refjob"]);
    assert_success(&output);
    assert!(combined(&output).contains("[red]weird[bold]"), "{}", combined(&output));
}
