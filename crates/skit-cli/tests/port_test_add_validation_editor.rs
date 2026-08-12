//! Real-PTY validate-before-editor ports from Python `tests/test_add_validation_contracts.py`.

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use tempfile::TempDir;

struct Fixture {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    _tools: TempDir,
    editor: PathBuf,
    capture: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let data = TempDir::new().unwrap();
        let state = TempDir::new().unwrap();
        let config = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let tools = TempDir::new().unwrap();
        let capture = tools.path().join("opened.txt");
        let editor = compile_fail_editor(tools.path());
        Self {
            data,
            state,
            config,
            home,
            _tools: tools,
            editor,
            capture,
        }
    }

    fn run(&self, tail: &[&str]) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 28,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
        command.args(["add", "-e", "-n", "editor-validation"]);
        command.args(tail);
        command.cwd(self.home.path());
        command.env("TERM", "xterm-256color");
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        command.env("SKIT_LANG", "en");
        command.env("HOME", self.home.path());
        command.env("USERPROFILE", self.home.path());
        command.env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"));
        command.env("XDG_DATA_HOME", self.home.path().join("xdg-data"));
        command.env("XDG_STATE_HOME", self.home.path().join("xdg-state"));
        command.env("VISUAL", &self.editor);
        command.env("EDITOR", &self.editor);
        command.env("SKIT_EDITOR_CAPTURE", &self.capture);

        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let status = child.wait().unwrap();
        let output = String::from_utf8_lossy(&drain.join().unwrap())
            .replace("\r\n", "\n")
            .replace('\r', "");
        (status.exit_code(), output)
    }

    fn assert_never_opened(&self) {
        assert!(
            !self.capture.exists(),
            "invalid flags reached the editor before validation"
        );
        let drafts = self.data.path().join("drafts");
        assert!(
            !drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none(),
            "invalid flags materialized a kept draft before validation"
        );
    }
}

fn compile_fail_editor(root: &Path) -> PathBuf {
    let source = root.join("validation_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs};
fn main() {
    fs::write(env::var_os("SKIT_EDITOR_CAPTURE").expect("capture"), b"opened").unwrap();
    std::process::exit(73);
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        "validation-editor.exe"
    } else {
        "validation-editor"
    });
    let status = Command::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(status.success());
    executable
}

fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn test_editor_lane_refuses_bad_python_before_opening_the_editor() {
    let fixture = Fixture::new();

    let (code, output) = fixture.run(&["--python", "garbage"]);

    assert_eq!(code, 2, "{output}");
    assert!(
        flat(&output).contains("isn't a Python version constraint"),
        "{output}"
    );
    fixture.assert_never_opened();
}

#[test]
fn test_editor_lane_refuses_bad_dep_before_opening_the_editor() {
    let fixture = Fixture::new();

    let (code, output) = fixture.run(&["--dep", "@@@"]);

    assert_eq!(code, 2, "{output}");
    assert!(
        flat(&output).contains("isn't a package requirement"),
        "{output}"
    );
    fixture.assert_never_opened();
}
