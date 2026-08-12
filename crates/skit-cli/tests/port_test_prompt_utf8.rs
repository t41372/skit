//! Public-process ports from Python `tests/test_prompt_utf8.py` at `main@206f9ef`.
//!
//! Every invalid prompt assertion uses the actual byte offset, never a character index. The tests
//! also require failed add/edit/read surfaces to leave entry/source state untouched. Red behavior is
//! a Rust parity finding; production code is not changed in this branch.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Output},
};

use assert_cmd::Command;
use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, SourcePermissions,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode};
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
    tools: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
            tools: TempDir::new().unwrap(),
        }
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
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

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn create_prompt_copy(&self, name: &str, bytes: &[u8]) -> skit_domain::Entry {
        let source = self.source(&format!("{name}.prompt.md"), bytes);
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("prompt").unwrap(),
                mode: StorageMode::Copy,
                source: source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: bytes.to_vec(),
                    stored_name: Some("prompt.md".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap()
    }

    fn payload(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.payload_path(&entry).unwrap()
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn byte_offset(prefix: &str) -> usize {
    prefix.as_bytes().len()
}

fn assert_invalid_utf8(text: &str, offset: usize) {
    assert!(text.contains("UTF-8"), "{text}");
    assert!(
        text.contains(&format!("offset {offset}")) || text.contains(&format!("byte {offset}")),
        "invalid prompt reported the wrong/missing byte offset {offset}: {text}"
    );
    assert!(
        !text.contains('\u{fffd}'),
        "invalid prompt was replacement-decoded: {text}"
    );
}

#[test]
fn test_store_rejects_invalid_utf8_prompt_and_reports_byte_offset() {
    let sandbox = Sandbox::new();
    let prefix = "多字节";
    let mut invalid = prefix.as_bytes().to_vec();
    invalid.extend_from_slice(b"\xff tail");
    let source = sandbox.source("bad.prompt.md", &invalid);

    let mut command = sandbox.command();
    let output = command
        .arg("add")
        .arg(&source)
        .args(["--prompt", "--name", "bad", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(2), "{text}");
    assert_invalid_utf8(&text, byte_offset(prefix));
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
    assert!(!sandbox.data.path().join("scripts/bad").exists());
}

#[test]
fn test_prompt_snapshot_read_error_is_not_ambiguous() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("gone.prompt.md");

    let mut command = sandbox.command();
    let output = command
        .arg("add")
        .arg(&missing)
        .args(["--prompt", "--ref", "--name", "gone", "--no-input"])
        .output()
        .unwrap();
    let text = combined(&output);

    assert_ne!(output.status.code(), Some(0), "{text}");
    assert!(text.contains("gone.prompt.md"), "{text}");
    assert!(
        !text.contains("UTF-8"),
        "missing source was misclassified as encoding: {text}"
    );
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[cfg(unix)]
#[test]
fn test_prompt_copy_keeps_the_snapshot_permissions() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let source = sandbox.source("mode.prompt.md", b"hello {{name}}\n");
    fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();

    let mut command = sandbox.command();
    let output = command
        .arg("add")
        .arg(&source)
        .args(["--prompt", "--name", "mode", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));

    let stored = sandbox.payload("mode");
    assert_eq!(
        fs::metadata(&stored).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[test]
fn test_stdin_prompt_cli_rejects_invalid_utf8_with_real_byte_offset() {
    let sandbox = Sandbox::new();
    let invalid = b"bad \xff prompt\n".to_vec();
    let mut command = sandbox.command();
    let assertion = command
        .args(["add", "-", "--prompt", "--name", "stdin-bad", "--no-input"])
        .write_stdin(invalid)
        .assert();
    let output = assertion.get_output();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(output.status.code(), Some(2), "{text}");
    assert_invalid_utf8(&text, 4);
    assert!(sandbox.store().scan().unwrap().entries.is_empty());
}

#[test]
fn test_runtime_prompt_payload_seam_refuses_invalid_bytes() {
    let sandbox = Sandbox::new();
    let prefix = "前缀";
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.extend_from_slice(b"\xff runtime\n");
    sandbox.create_prompt_copy("runtime-bad", &bytes);

    let output = sandbox.run(&["run", "runtime-bad", "--dry-run", "--no-input"]);
    let text = combined(&output);

    assert_ne!(output.status.code(), Some(0), "{text}");
    assert_invalid_utf8(&text, byte_offset(prefix));
}

#[test]
fn test_changed_prompt_is_rejected_by_launch_and_health_with_byte_offset() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt_copy("live", b"hello {{name}}\n");
    let payload = sandbox.payload("live");
    let prefix = "多字节";
    let mut changed = prefix.as_bytes().to_vec();
    changed.extend_from_slice(b"\xff changed\n");
    fs::write(&payload, &changed).unwrap();
    let offset = byte_offset(prefix);

    let run = sandbox.run(&["run", "live", "--dry-run", "--no-input"]);
    assert_ne!(run.status.code(), Some(0), "{}", combined(&run));
    assert_invalid_utf8(&combined(&run), offset);

    let doctor = sandbox.run(&["doctor"]);
    assert_invalid_utf8(&combined(&doctor), offset);
    assert_eq!(fs::read(&payload).unwrap(), changed);
}

#[test]
fn test_cli_edit_invalid_prompt_refuses_then_reedit_recovers_without_replacement() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt_copy("edit-bad", b"hello {{name}}\n");
    let payload = sandbox.payload("edit-bad");
    let prefix = "多字节";
    let mut invalid = prefix.as_bytes().to_vec();
    invalid.extend_from_slice(b"\xff broken\n");
    fs::write(&payload, &invalid).unwrap();
    let capture = sandbox.tools.path().join("editor-called");
    let editor = compile_editor(sandbox.tools.path());

    let first = sandbox
        .command()
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .env("SKIT_PROMPT_EDITOR_CAPTURE", &capture)
        .env("SKIT_PROMPT_EDITOR_FAIL", "1")
        .args(["edit", "edit-bad"])
        .output()
        .unwrap();
    let first_text = combined(&first);
    assert_ne!(first.status.code(), Some(0), "{first_text}");
    assert_invalid_utf8(&first_text, byte_offset(prefix));
    assert!(
        !capture.exists(),
        "invalid prompt reached the editor before strict validation"
    );
    assert_eq!(fs::read(&payload).unwrap(), invalid);

    fs::write(&payload, "repaired {{name}}\n".as_bytes()).unwrap();
    let second = sandbox
        .command()
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .env("SKIT_PROMPT_EDITOR_CAPTURE", &capture)
        .env("SKIT_PROMPT_EDITOR_CONTENT", "edited 你好 {{name}}\n")
        .env_remove("SKIT_PROMPT_EDITOR_FAIL")
        .args(["edit", "edit-bad"])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", combined(&second));
    assert_eq!(
        fs::read(&payload).unwrap(),
        "edited 你好 {{name}}\n".as_bytes()
    );
    assert!(!String::from_utf8_lossy(&fs::read(&payload).unwrap()).contains('\u{fffd}'));
}

#[test]
fn test_library_edit_preserves_valid_prompt_utf8_and_refreshes_placeholders() {
    let sandbox = Sandbox::new();
    sandbox.create_prompt_copy("edit-good", "hello {{name}}\n".as_bytes());
    let payload = sandbox.payload("edit-good");
    let capture = sandbox.tools.path().join("editor-good");
    let editor = compile_editor(sandbox.tools.path());

    let output = sandbox
        .command()
        .env("VISUAL", &editor)
        .env("EDITOR", &editor)
        .env("SKIT_PROMPT_EDITOR_CAPTURE", &capture)
        .env("SKIT_PROMPT_EDITOR_CONTENT", "你好 {{city}} café\n")
        .args(["edit", "edit-good"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(
        fs::read(&payload).unwrap(),
        "你好 {{city}} café\n".as_bytes()
    );
    let params = sandbox.run(&["params", "edit-good", "--json"]);
    assert_eq!(params.status.code(), Some(0), "{}", combined(&params));
    let value: serde_json::Value = serde_json::from_slice(&params.stdout).unwrap();
    let all = serde_json::to_string(&value).unwrap();
    assert!(
        all.contains("city"),
        "edited placeholder was not refreshed: {all}"
    );
    assert!(
        !all.contains("name"),
        "stale placeholder survived edit: {all}"
    );
}

#[test]
fn test_cli_add_params_run_doctor_share_the_strict_prompt_contract() {
    let sandbox = Sandbox::new();
    let prefix = "前缀";
    let mut invalid = prefix.as_bytes().to_vec();
    invalid.extend_from_slice(b"\xff tail\n");
    let offset = byte_offset(prefix);
    let source = sandbox.source("all.prompt.md", &invalid);
    let mut add_command = sandbox.command();
    let add = add_command
        .arg("add")
        .arg(&source)
        .args(["--prompt", "--name", "all-bad", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(add.status.code(), Some(2), "{}", combined(&add));
    assert_invalid_utf8(&combined(&add), offset);
    assert!(sandbox.store().resolve("all-bad").is_err());

    sandbox.create_prompt_copy("surfaces", b"hello {{name}}\n");
    let payload = sandbox.payload("surfaces");
    fs::write(&payload, &invalid).unwrap();
    for args in [
        vec!["show", "surfaces", "--json"],
        vec!["params", "surfaces", "--json"],
        vec!["run", "surfaces", "--dry-run", "--no-input"],
        vec!["doctor"],
    ] {
        let output = sandbox.run(&args);
        let text = combined(&output);
        assert_invalid_utf8(&text, offset);
        assert_eq!(
            fs::read(&payload).unwrap(),
            invalid,
            "surface {args:?} mutated the prompt"
        );
    }
}

fn compile_editor(root: &Path) -> PathBuf {
    let source = root.join("prompt_utf8_editor.rs");
    fs::write(
        &source,
        r#"
use std::{env, fs, path::PathBuf};
fn main() {
    let target = PathBuf::from(env::args_os().last().expect("editor target"));
    let capture = env::var_os("SKIT_PROMPT_EDITOR_CAPTURE").expect("capture");
    fs::write(&capture, target.to_string_lossy().as_bytes()).unwrap();
    if env::var_os("SKIT_PROMPT_EDITOR_FAIL").is_some() {
        std::process::exit(73);
    }
    if let Some(content) = env::var_os("SKIT_PROMPT_EDITOR_CONTENT") {
        fs::write(&target, content.to_string_lossy().as_bytes()).unwrap();
    }
}
"#,
    )
    .unwrap();
    let executable = root.join(if cfg!(windows) {
        "prompt-utf8-editor.exe"
    } else {
        "prompt-utf8-editor"
    });
    let status = ProcessCommand::new("rustc")
        .arg(source)
        .arg("-o")
        .arg(&executable)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "failed to compile prompt UTF-8 editor probe"
    );
    executable
}
