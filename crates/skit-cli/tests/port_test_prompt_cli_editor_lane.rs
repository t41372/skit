use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_language::placeholder_params;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
    editor: PathBuf,
    capture: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tools = TempDir::new().unwrap();
        let editor = compile_editor(tools.path());
        let capture = tools.path().join("editor-capture.txt");
        let this = Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools,
            editor,
            capture,
        };
        let quoted = format!("\"{}\"", this.editor.display());
        this.command("en")
            .args(["config", "editor", &quoted])
            .assert()
            .success();
        this
    }

    fn command(&self, lang: &str) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", lang)
            .env("HOME", self.home.path())
            .env("USERPROFILE", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path().join("xdg-config"))
            .env("XDG_DATA_HOME", self.home.path().join("xdg-data"))
            .env("XDG_STATE_HOME", self.home.path().join("xdg-state"))
            .env("SKIT_EDITOR_CAPTURE", &self.capture)
            .current_dir(self.home.path());
        command
    }

    fn entry_exists(&self, slug: &str) -> bool {
        self.data.path().join("scripts").join(slug).join("meta.toml").is_file()
    }

    fn show_json(&self, slug: &str) -> serde_json::Value {
        let output = self.command("en").args(["show", slug, "--json"]).output().unwrap();
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output.stdout, &output.stderr));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("prompt_draft_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("draft path"));
    let initial = fs::read(&target).unwrap_or_default();
    if let Some(capture) = env::var_os("SKIT_EDITOR_CAPTURE") {
        let mut payload = target.to_string_lossy().as_bytes().to_vec();
        payload.push(b'\n');
        payload.extend_from_slice(&initial);
        fs::write(capture, payload).unwrap();
    }
    if env::var_os("SKIT_EDITOR_FAIL_IF_CALLED").is_some() {
        std::process::exit(73);
    }
    if env::var_os("SKIT_EDITOR_DELETE").is_some() {
        let _ = fs::remove_file(&target);
        return;
    }
    if let Some(content) = env::var_os("SKIT_EDITOR_CONTENT") {
        fs::write(&target, content.to_string_lossy().as_bytes()).unwrap();
    }
    if let Some(slug) = env::var_os("SKIT_EDITOR_COLLIDE") {
        let data = PathBuf::from(env::var_os("SKIT_DATA_DIR").expect("data dir"));
        let dir = data.join("scripts").join(slug);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("collision-sentinel"), b"occupied").unwrap();
    }
}
"#,
    ).unwrap();
    let executable = root.join(if cfg!(windows) { "prompt-draft-editor.exe" } else { "prompt-draft-editor" });
    let status = ProcessCommand::new("rustc").arg(source).arg("-o").arg(&executable).status().unwrap();
    assert!(status.success());
    executable
}

fn combined(stdout: &[u8], stderr: &[u8]) -> String {
    format!("{}{}", String::from_utf8_lossy(stdout), String::from_utf8_lossy(stderr))
}

fn run_pty(
    sandbox: &Sandbox,
    args: &[&str],
    lang: &str,
    envs: &[(&str, &str)],
    chunks: &[&[u8]],
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize { rows: 30, cols: 120, pixel_width: 0, pixel_height: 0 })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.cwd(sandbox.home.path());
    command.env("TERM", "xterm-256color");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", lang);
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.env("SKIT_EDITOR_CAPTURE", &sandbox.capture);
    for (key, value) in envs {
        command.env(key, value);
    }
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

fn captured(sandbox: &Sandbox) -> (PathBuf, Vec<u8>) {
    let bytes = fs::read(&sandbox.capture).unwrap();
    let split = bytes.iter().position(|byte| *byte == b'\n').unwrap();
    (
        PathBuf::from(String::from_utf8(bytes[..split].to_vec()).unwrap()),
        bytes[split + 1..].to_vec(),
    )
}

#[test]
fn test_localized_starter_is_minimal_and_never_creates_its_own_field() {
    for (locale, expected) in [
        ("en", "# New prompt\n\n"),
        ("zh-CN", "# 新提示词\n\n"),
        ("zh-TW", "# 新提示詞\n\n"),
    ] {
        let sandbox = Sandbox::new();
        let (code, output) = run_pty(
            &sandbox,
            &["add", "--prompt", "-n", "starter"],
            locale,
            &[],
            &[],
        );
        assert_eq!(code, 0, "locale={locale}\n{output}");
        let (_, initial) = captured(&sandbox);
        assert_eq!(initial, expected.as_bytes(), "locale={locale}");
        assert!(placeholder_params("prompt", expected).is_empty());
        assert_eq!(
            placeholder_params("prompt", &(expected.to_owned() + "Review {{目标}}\n"))
                .into_iter()
                .map(|field| field.name)
                .collect::<Vec<_>>(),
            ["目标"]
        );
        assert!(!sandbox.entry_exists("starter"));
    }
}

#[test]
fn test_add_prompt_editor_lane_interactive() {
    let sandbox = Sandbox::new();
    let (code, output) = run_pty(
        &sandbox,
        &["add", "--prompt", "-n", "note"],
        "en",
        &[("SKIT_EDITOR_CONTENT", "Edited body {{v}}\n")],
        &[b"all\r", b"-\r"],
    );
    assert_eq!(code, 0, "{output}");
    assert_eq!(fs::read(sandbox.data.path().join("scripts/note/prompt.md")).unwrap(), b"Edited body {{v}}\n");
    assert_eq!(
        sandbox.show_json("note")["fields"].as_array().unwrap().iter().map(|field| field["key"].as_str().unwrap()).collect::<Vec<_>>(),
        ["v"]
    );
}

#[test]
fn test_add_prompt_editor_lane_untouched_starter_adds_nothing() {
    let sandbox = Sandbox::new();
    let (code, output) = run_pty(
        &sandbox,
        &["add", "--prompt", "-n", "empty"],
        "en",
        &[],
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Nothing was written"), "{output}");
    assert!(!sandbox.entry_exists("empty"));
}

#[test]
fn test_add_prompt_editor_lane_asks_for_a_name() {
    let sandbox = Sandbox::new();
    let (code, output) = run_pty(
        &sandbox,
        &["add", "--prompt"],
        "en",
        &[],
        &[b"\r"],
    );
    assert_eq!(code, 2, "{output}");
    assert!(output.contains("A name is required"), "{output}");
}

#[test]
fn test_add_prompt_editor_lane_name_taken_refuses_before_the_editor() {
    let sandbox = Sandbox::new();
    sandbox.command("en").args(["add", "--cmd", "echo hi", "--name", "taken"]).assert().success();
    let (code, output) = run_pty(
        &sandbox,
        &["add", "--prompt", "-n", "taken"],
        "en",
        &[("SKIT_EDITOR_FAIL_IF_CALLED", "1")],
        &[],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("already taken"), "{output}");
    assert!(!sandbox.capture.exists(), "editor was launched before name-conflict refusal");
}

#[test]
fn test_add_prompt_editor_lane_post_edit_failure_keeps_the_draft() {
    let sandbox = Sandbox::new();
    let (code, output) = run_pty(
        &sandbox,
        &["add", "--prompt", "-n", "keptprompt"],
        "en",
        &[
            ("SKIT_EDITOR_CONTENT", "Drafted prompt {{v}}\n"),
            ("SKIT_EDITOR_COLLIDE", "keptprompt"),
        ],
        &[b"all\r", b"-\r"],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("Your draft was kept at"), "{output}");
    let (draft, _) = captured(&sandbox);
    assert!(draft.exists(), "post-edit store failure destroyed the user's draft: {output}");
    assert!(!sandbox.entry_exists("keptprompt"));
    assert_eq!(fs::read(sandbox.data.path().join("scripts/keptprompt/collision-sentinel")).unwrap(), b"occupied");
}

#[test]
fn test_add_prompt_editor_lane_deleted_draft_is_a_clean_honest_failure() {
    let sandbox = Sandbox::new();
    let (code, output) = run_pty(
        &sandbox,
        &["add", "--prompt", "-n", "gone"],
        "en",
        &[("SKIT_EDITOR_DELETE", "1")],
        &[],
    );
    assert_eq!(code, 1, "{output}");
    assert!(output.contains("Can't read"), "{output}");
    assert!(output.contains("The draft is no longer at"), "{output}");
    assert!(!output.contains("Your draft was kept at"), "{output}");
    let (draft, _) = captured(&sandbox);
    assert!(!draft.exists());
    assert!(!sandbox.entry_exists("gone"));
}
