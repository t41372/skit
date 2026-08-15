use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;
use tempfile::TempDir;

const FROZEN_LIST_PREVIEW_LIMIT: usize = 20;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
    editor: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tools = TempDir::new().unwrap();
        let editor = compile_editor(tools.path());
        let this = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools,
            editor,
        };
        let command = quote(&this.editor);
        this.command().args(["config", "editor", &command]).assert().success();
        this
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

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_prompt(&self, name: &str, body: &str) {
        let source = self.source(&format!("{name}.prompt.md"), body);
        let output = self.run(&["add", source.to_str().unwrap(), "--name", name, "--no-input"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn add_python(&self, name: &str, body: &str) {
        let source = self.source(&format!("{name}.py"), body);
        let output = self.run(&["add", source.to_str().unwrap(), "--name", name, "--no-input"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    }

    fn fields(&self, name: &str) -> Vec<String> {
        let output = self.run(&["show", name, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
        payload["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|field| field["key"].as_str().unwrap().to_owned())
            .collect()
    }

    fn set_form_plain(&self) {
        self.command().args(["config", "form", "plain"]).assert().success();
    }

    fn edit(&self, name: &str, append: &str) -> Output {
        self.command()
            .env("SKIT_EDITOR_APPEND", append)
            .args(["edit", name])
            .output()
            .unwrap()
    }
}

fn quote(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("prompt_edit_probe.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("target"));
    let append = env::var("SKIT_EDITOR_APPEND").unwrap_or_default();
    if !append.is_empty() {
        let mut bytes = fs::read(&target).expect("read target");
        bytes.extend_from_slice(append.as_bytes());
        fs::write(&target, bytes).expect("append target");
    }
}
"#,
    ).unwrap();
    let executable = root.join(if cfg!(windows) { "prompt-edit-probe.exe" } else { "prompt-edit-probe" });
    let status = ProcessCommand::new("rustc").arg(source).arg("-o").arg(&executable).status().unwrap();
    assert!(status.success());
    executable
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

fn flat(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run_edit_pty(sandbox: &Sandbox, name: &str, append: &str, chunks: &[&[u8]]) -> (u32, String) {
    let pair = native_pty_system().openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 }).unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(["edit", name]);
    command.cwd(sandbox.home.path());
    command.env("TERM", "xterm-256color");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.env("SKIT_EDITOR_APPEND", append);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(140));
    let _ = writer.write_all(b"\x1b[1;1R");
    let _ = writer.flush();
    for chunk in chunks {
        thread::sleep(Duration::from_millis(180));
        if writer.write_all(chunk).is_err() { break; }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).replace("\r\n", "\n").replace('\r', "");
    (status.exit_code(), output)
}

#[test]
fn test_edit_prompt_non_interactive_names_the_unmanaged_variable() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("greet", "Say hello.\n");
    let output = sandbox.edit("greet", "\n{{username}}\n");
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(sandbox.fields("greet"), Vec::<String>::new());
    assert!(combined(&output).contains("Detected but not yet managed: username"), "{}", combined(&output));
}

#[test]
fn test_edit_prompt_non_interactive_flood_previews_with_a_tail() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("greet", "Base.\n");
    let holes = (0..FROZEN_LIST_PREVIEW_LIMIT + 4)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let output = sandbox.edit("greet", &format!("\n{holes}\n"));
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(flat(&combined(&output)).contains("and 4 more candidates"), "{}", combined(&output));
    assert_eq!(sandbox.fields("greet"), Vec::<String>::new());
}

#[test]
fn test_edit_prompt_with_no_new_placeholders_is_silent() {
    let sandbox = Sandbox::new();
    sandbox.add_prompt("greet", "{{a}}\n");
    let output = sandbox.edit("greet", "\nmore prose\n");
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    let shown = combined(&output);
    assert!(!shown.contains("Now managed"), "{shown}");
    assert!(!shown.contains("Detected but not yet managed"), "{shown}");
    assert_eq!(sandbox.fields("greet"), ["a"]);
}

#[test]
fn test_edit_non_prompt_keeps_the_generic_drift_hint() {
    let sandbox = Sandbox::new();
    sandbox.add_python("job", "print(1)\n");
    let output = sandbox.edit("job", "");
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(combined(&output).contains("skit reconciles parameter drift at run time"), "{}", combined(&output));
}

#[test]
fn test_edit_prompt_interactive_offers_and_manages_a_new_placeholder() {
    let sandbox = Sandbox::new();
    sandbox.set_form_plain();
    sandbox.add_prompt("greet", "Say hello.\n");
    let (code, output) = run_edit_pty(&sandbox, "greet", "\nUser is {{username}}\n", &[b"all\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.fields("greet"), ["username"]);
    assert!(flat(&output).contains("Now managed: username"), "{output}");
}

#[test]
fn test_edit_prompt_interactive_none_leaves_the_placeholder_literal() {
    let sandbox = Sandbox::new();
    sandbox.set_form_plain();
    sandbox.add_prompt("greet", "Say hello.\n");
    let (code, output) = run_edit_pty(&sandbox, "greet", "\n{{username}}\n", &[b"none\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.fields("greet"), Vec::<String>::new());
    assert!(!output.contains("Now managed"), "{output}");
}

#[test]
fn test_edit_prompt_interactive_numbers_manage_the_named_ones() {
    let sandbox = Sandbox::new();
    sandbox.set_form_plain();
    sandbox.add_prompt("greet", "Base.\n");
    let (code, output) = run_edit_pty(&sandbox, "greet", "\n{{a}} {{b}} {{c}}\n", &[b"1,3\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.fields("greet"), ["a", "c"]);
}

#[test]
fn test_edit_prompt_preserves_existing_managed_and_adds_the_new_one() {
    let sandbox = Sandbox::new();
    sandbox.set_form_plain();
    sandbox.add_prompt("greet", "{{kept}}\n");
    let (code, output) = run_edit_pty(&sandbox, "greet", "\n{{added}}\n", &[b"all\r"]);
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.fields("greet"), ["kept", "added"]);
}

#[test]
fn test_edit_prompt_interactive_flood_previews_secret_mark_and_tail() {
    let sandbox = Sandbox::new();
    sandbox.set_form_plain();
    sandbox.add_prompt("greet", "Base.\n");
    let holes = std::iter::once("{{token}}".to_owned())
        .chain((0..FROZEN_LIST_PREVIEW_LIMIT + 3).map(|index| format!("{{{{h{index}}}}}")))
        .collect::<Vec<_>>()
        .join(" ");
    let (code, output) = run_edit_pty(&sandbox, "greet", &format!("\n{holes}\n"), &[b"all\r"]);
    assert_eq!(code, 0, "{output}");
    let shown = flat(&output);
    assert!(shown.contains("token (secret)"), "{shown}");
    assert!(shown.contains("…and 4 more"), "{shown}");
    let expected = std::iter::once("token".to_owned())
        .chain((0..FROZEN_LIST_PREVIEW_LIMIT + 3).map(|index| format!("h{index}")))
        .collect::<Vec<_>>();
    assert_eq!(sandbox.fields("greet"), expected);
}
