//! Mechanical port of the Python oracle module `tests/test_add_validation_contracts.py`
//! (`origin/main@206f9ef`): "Add validation contracts (exit codes, exact refusal copy,
//! filesystem state, stored PEP 723 text, the two lazy `packaging` validators in isolation)."
//! Each `#[test]` keeps its Python `def test_*` name and its "WHY" comment so it traces back
//! to the oracle.
//!
//! Concept mapping used throughout:
//! - Python `pep723.requires_python_error(v)` -> `skit_language::validate_pep440_specifiers(v)`
//!   (`None` for valid <-> `Ok(())`; an invalid value <-> `Err(PythonMetadataError)`) with the
//!   oracle's localized refusal copy.
//! - Python `pep723.requirement_error(v)` -> `skit_language::validate_pep508_requirement(v)`.
//! - Python `cli._validate_python_flags(deps, python)` has NO public Rust function. The `skit add -`
//!   (stdin) lane runs the same validate-then-normalize contract inside `add_with_config`
//!   (`crates/skit-cli/src/cli.rs`), so these are verified END-TO-END through the composition root.
//! - Python `cli._resolve_python_metadata(...)` interactive re-ask loop -> the real plain-form PTY
//!   path, with dependency partitioning shared with the serializable add review.
//! - Python `runner.invoke(cli.app, ...)` -> the real `skit` binary via `assert_cmd`, sandboxed by
//!   the three `SKIT_*` temp dirs.
//! - Python `registry.kind_for_draft(path)` -> the shared `skit_language::infer_draft_kind` owner;
//!   section 6 keeps only CLI consequences that are not owned more strongly elsewhere.
//! - Python `cli._create_python_in_editor(...)` -> the `skit add --edit` lane (`add_draft`).
//!
//! Bucket disposition (31 Python defs -> 31 `#[test]`; 27 active, 4 ignored):
//! - 27 active contracts include sections 1, 2, 3, and 5 plus all four canonical drafts-boundary
//!   faces and the already-owned classifier complements.
//! - The four drafts-boundary faces are active canonical contracts. The normal resume,
//!   script-suffix, prompt-resume, and unknown-shebang twins remain as semantic-duplicate closures
//!   with their stronger owners named in each reason.
//! - 4 semantic-duplicate closures name their stronger owned-draft owner.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};

use skit_i18n::{Locale, Localize};
use skit_language::{read_uv_metadata, validate_pep440_specifiers, validate_pep508_requirement};
use tempfile::TempDir;

#[cfg(unix)]
#[path = "support/plain_add_pty.rs"]
mod plain_add_pty;
#[cfg(unix)]
use plain_add_pty::PlainAddPty;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    scratch: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            scratch: TempDir::new().unwrap(),
        }
    }

    fn command_in(&self, language: &str) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", language);
        command
    }

    fn show_json(&self, name: &str) -> serde_json::Value {
        let output = self
            .command_in("en")
            .args(["show", name, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

#[cfg(windows)]
#[test]
fn windows_cli_infers_a_real_pathext_source_as_an_executable() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("run.BAT");
    fs::write(&source, "@echo off\r\n").unwrap();

    sandbox
        .command_in("en")
        .env("PATHEXT", ".FOO;.BAT")
        .args(["add"])
        .arg(&source)
        .args(["--name", "run", "--no-input"])
        .assert()
        .success();

    assert_eq!(sandbox.show_json("run")["kind"], "exe");
}

#[cfg(unix)]
fn configure_plain(sandbox: &Sandbox) {
    fs::write(
        sandbox.config.path().join("config.toml"),
        "form = \"plain\"\n",
    )
    .unwrap();
}

#[cfg(unix)]
fn python_source(sandbox: &Sandbox, name: &str, body: &str) -> PathBuf {
    let path = sandbox.scratch.path().join(name);
    fs::write(&path, body).unwrap();
    path
}

#[cfg(unix)]
fn dependencies_question(locale: &str) -> &'static str {
    match locale {
        "zh-CN" => "要安装的依赖(Enter 采用,可自行编辑,或输入 - 表示不需要)",
        "zh-TW" => "要安裝的依賴(Enter 採用,可自行編輯,或輸入 - 表示不需要)",
        _ => "Dependencies to install (Enter to accept, edit the list, or '-' for none)",
    }
}

#[cfg(unix)]
fn automatic_python_question(locale: &str) -> &'static str {
    match locale {
        "zh-CN" => "Python 版本(留空 = 自动)",
        "zh-TW" => "Python 版本(留空 = 自動)",
        _ => "Python version (leave empty for automatic)",
    }
}

#[derive(Debug, Eq, PartialEq)]
struct TreeItem {
    path: PathBuf,
    kind: &'static str,
    bytes: Vec<u8>,
    readonly: bool,
    unix_mode: Option<u32>,
}

#[cfg(unix)]
fn unix_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(metadata.permissions().mode())
}

#[cfg(not(unix))]
fn unix_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

fn snapshot_tree(root: &Path, prefix: &str) -> Vec<TreeItem> {
    fn walk(root: &Path, path: &Path, prefix: &str, items: &mut Vec<TreeItem>) {
        let mut entries = fs::read_dir(path)
            .unwrap_or_else(|error| panic!("read {path:?}: {error}"))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let relative = PathBuf::from(prefix).join(path.strip_prefix(root).unwrap());
            let (kind, bytes) = if metadata.file_type().is_symlink() {
                (
                    "symlink",
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec(),
                )
            } else if metadata.is_dir() {
                ("directory", Vec::new())
            } else {
                ("file", fs::read(&path).unwrap())
            };
            items.push(TreeItem {
                path: relative,
                kind,
                bytes,
                readonly: metadata.permissions().readonly(),
                unix_mode: unix_mode(&metadata),
            });
            if metadata.is_dir() {
                walk(root, &path, prefix, items);
            }
        }
    }

    let mut items = Vec::new();
    walk(root, root, prefix, &mut items);
    items
}

fn snapshot_sandbox(sandbox: &Sandbox) -> Vec<TreeItem> {
    let mut items = snapshot_tree(sandbox.data.path(), "data");
    items.extend(snapshot_tree(sandbox.state.path(), "state"));
    items.extend(snapshot_tree(sandbox.config.path(), "config"));
    items
}

fn explicit_boundary_message(language: &str, file: &str, flags: &str) -> String {
    match language {
        "zh-CN" => format!(
            "{file} 是 skit 自己保留的草稿——恢复草稿一律以副本加入(成功后即消耗),而 reference 或程序项目做不到这点。请去掉 {flags}。"
        ),
        "zh-TW" => format!(
            "{file} 是 skit 自己保留的草稿——恢復草稿一律以副本加入(成功後即消耗),而 reference 或程式項目做不到這點。請拿掉 {flags}。"
        ),
        _ => format!(
            "{file} is one of skit's own kept drafts — a resumed draft is always added as a copy (and consumed on success), which a reference or program entry can't be. Drop {flags}."
        ),
    }
}

fn inferred_boundary_message(language: &str, file: &str) -> String {
    match language {
        "zh-CN" => format!(
            "{file} 是 skit 自己保留的草稿,而草稿一律以脚本或提示词副本加入——请用 --kind <语言> 指定它的语言。"
        ),
        "zh-TW" => format!(
            "{file} 是 skit 自己保留的草稿,而草稿一律以腳本或提示詞副本加入——請用 --kind <語言> 指定它的語言。"
        ),
        _ => format!(
            "{file} is one of skit's own kept drafts, and a draft is always added as a script or prompt copy — pass --kind <language> to name its language."
        ),
    }
}

/// Python `_flat`: collapse every run of whitespace to one space (stdout+stderr concatenated,
/// because a refusal prints to stderr and Python's `result.output` mixes both streams).
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Run `skit <args>` with optional piped stdin and return (exit code, flattened combined output).
fn run(sandbox: &Sandbox, args: &[&str], stdin: Option<&str>) -> (Option<i32>, String) {
    run_in(sandbox, "en", args, stdin)
}

fn run_in(
    sandbox: &Sandbox,
    language: &str,
    args: &[&str],
    stdin: Option<&str>,
) -> (Option<i32>, String) {
    let mut command = sandbox.command_in(language);
    command.args(args);
    if let Some(text) = stdin {
        command.write_stdin(text.to_owned());
    }
    let output = command.output().expect("run skit");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), flat(&combined))
}

/// Run an editor flow with terminal-backed stdin and stdout, as the oracle does.
#[cfg(unix)]
fn run_pty(sandbox: &Sandbox, args: &[&str], editor: &Path) -> (u32, String) {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("EDITOR", editor);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = reader.read_to_end(&mut bytes);
        bytes
    });
    let status = child.wait().unwrap();
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), flat(&output))
}

/// Run an interactive request that must be rejected before it can ask a question.
#[cfg(unix)]
fn run_pty_to_exit(sandbox: &Sandbox, args: &[&str]) -> (u32, String) {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 160,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let captured = Arc::new(Mutex::new(Vec::new()));
    let reader_capture = Arc::clone(&captured);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut chunk = [0_u8; 1024];
        while let Ok(read) = reader.read(&mut chunk) {
            if read == 0 {
                break;
            }
            reader_capture
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..read]);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let (status, timed_out) = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            break (child.wait().unwrap(), true);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    drop(pair.master);
    drain.join().unwrap();
    let output = String::from_utf8_lossy(&captured.lock().unwrap()).into_owned();
    assert!(
        !timed_out,
        "the drafts guard reached an interactive question: {output}"
    );
    (status.exit_code(), flat(&output))
}

/// Python `_draft`: write a body into `<SKIT_DATA_DIR>/drafts/<name>` (the real drafts home).
fn draft(sandbox: &Sandbox, name: &str, body: &str) -> PathBuf {
    let dir = sandbox.data.path().join("drafts");
    fs::create_dir_all(&dir).expect("create drafts dir");
    let path = dir.join(name);
    fs::write(&path, body).expect("write draft");
    path
}

/// Python `_drafts_files() == []`: the drafts dir is absent or holds nothing.
fn drafts_dir_is_empty(sandbox: &Sandbox) -> bool {
    let dir = sandbox.data.path().join("drafts");
    !dir.exists()
        || fs::read_dir(&dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
}

fn entry_dir(sandbox: &Sandbox, slug: &str) -> PathBuf {
    sandbox.data.path().join("scripts").join(slug)
}

/// The stored `meta.toml` text for one slug, empty when the entry was never created.
fn read_meta(sandbox: &Sandbox, slug: &str) -> String {
    fs::read_to_string(entry_dir(sandbox, slug).join("meta.toml")).unwrap_or_default()
}

/// The stored python `script.py` text for one slug, empty when it was never written.
fn read_script(sandbox: &Sandbox, slug: &str) -> String {
    fs::read_to_string(entry_dir(sandbox, slug).join("script.py")).unwrap_or_default()
}

const FORCEABLE_ADD_KINDS: &str =
    "fish, js, lua, perl, powershell, python, r, ruby, shell, ts, exe";

fn unknown_add_kind_message(language: &str, kind: &str) -> String {
    match language {
        "zh-CN" => format!("未知类型：{kind}。可选：{FORCEABLE_ADD_KINDS}"),
        "zh-TW" => format!("未知類型：{kind}。可選：{FORCEABLE_ADD_KINDS}"),
        _ => format!("Unknown kind: {kind}. Choose from: {FORCEABLE_ADD_KINDS}"),
    }
}

fn add_kind_conflict_message(language: &str) -> &'static str {
    match language {
        "zh-CN" => "--kind 与 --exe 只能择一。",
        "zh-TW" => "--kind 與 --exe 只能擇一。",
        _ => "Use --kind or --exe, not both.",
    }
}

// ==========================================================================
// 0. Explicit add kinds use the same closed authoring contract as version 0.4
// ==========================================================================

#[test]
fn test_cli_add_shell_script_records_interpreter() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("deploy.sh");
    fs::write(&source, "#!/usr/bin/env zsh\n# Ship it\necho hi\n").unwrap();

    sandbox
        .command_in("en")
        .arg("add")
        .arg(&source)
        .args(["--name", "deploy", "--no-input"])
        .assert()
        .success();

    let entry = sandbox.show_json("deploy");
    assert_eq!(entry["kind"], "shell");
    assert_eq!(entry["interpreter"], "zsh");
    assert_eq!(entry["description"], "Ship it");
}

#[test]
fn test_cli_add_kind_forces_extensionless_file() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("build");
    fs::write(&source, "echo building\n").unwrap();

    sandbox
        .command_in("en")
        .arg("add")
        .arg(&source)
        .args(["--kind", "shell", "--name", "build", "--no-input"])
        .assert()
        .success();

    assert_eq!(sandbox.show_json("build")["kind"], "shell");
    assert!(entry_dir(&sandbox, "build").join("script.sh").is_file());
}

#[test]
fn test_cli_add_kind_exe() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("thing");
    fs::write(&source, "bytes\n").unwrap();

    sandbox
        .command_in("en")
        .arg("add")
        .arg(&source)
        .args(["--kind", "exe", "--name", "thing", "--no-input"])
        .assert()
        .success();

    let entry = sandbox.show_json("thing");
    assert_eq!(entry["kind"], "exe");
    assert_eq!(entry["mode"], "reference");
}

#[test]
fn test_cli_add_kind_unknown_is_usage_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("unknown");
    fs::write(&source, "bytes\n").unwrap();
    let before = snapshot_sandbox(&sandbox);
    let source_before = fs::read(&source).unwrap();

    for kind in ["cobol", "prompt"] {
        for language in ["en", "zh-CN", "zh-TW"] {
            let output = sandbox
                .command_in(language)
                .arg("add")
                .arg(&source)
                .args(["--kind", kind, "--name", "unknown", "--no-input"])
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(2),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stdout.is_empty());
            assert_eq!(
                String::from_utf8_lossy(&output.stderr).trim(),
                unknown_add_kind_message(language, kind)
            );
            assert_eq!(
                snapshot_sandbox(&sandbox),
                before,
                "kind={kind} language={language}"
            );
            assert_eq!(fs::read(&source).unwrap(), source_before);
        }
    }
}

#[test]
fn test_cli_add_kind_and_exe_conflict() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("conflict");
    fs::write(&source, "echo hi\n").unwrap();
    let before = snapshot_sandbox(&sandbox);
    let source_before = fs::read(&source).unwrap();

    for language in ["en", "zh-CN", "zh-TW"] {
        let output = sandbox
            .command_in(language)
            .arg("add")
            .arg(&source)
            .args([
                "--kind",
                "shell",
                "--exe",
                "--name",
                "conflict",
                "--no-input",
            ])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            add_kind_conflict_message(language)
        );
        assert_eq!(snapshot_sandbox(&sandbox), before, "language={language}");
        assert_eq!(fs::read(&source).unwrap(), source_before);
    }
}

#[test]
fn test_cli_add_command_kind_rejected() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("command");
    fs::write(&source, "echo hi\n").unwrap();
    let before = snapshot_sandbox(&sandbox);
    let source_before = fs::read(&source).unwrap();

    for language in ["en", "zh-CN", "zh-TW"] {
        let output = sandbox
            .command_in(language)
            .arg("add")
            .arg(&source)
            .args(["--kind", "command", "--name", "command", "--no-input"])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            unknown_add_kind_message(language, "command")
        );
        assert_eq!(snapshot_sandbox(&sandbox), before, "language={language}");
        assert_eq!(fs::read(&source).unwrap(), source_before);
    }
}

// ==========================================================================
// 1. The pep723 validators (lazy `packaging` imports), in isolation
// ==========================================================================

#[test]
fn test_requires_python_error_is_none_for_valid_constraints() {
    assert!(validate_pep440_specifiers(">=3.11").is_ok());
    assert!(validate_pep440_specifiers(">=3.12,<3.13").is_ok());
}

#[test]
fn test_requires_python_error_localizes_a_message_for_an_invalid_constraint() {
    let error = validate_pep440_specifiers("not-a-version").unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(
        message.starts_with("not-a-version isn't a Python version constraint"),
        "{message}"
    );
}

#[test]
fn test_requires_python_error_rejects_a_bare_version_without_operator() {
    // `3.11` (no comparison operator) is a real, common mistake — PEP 440 refuses it.
    assert!(validate_pep440_specifiers("3.11").is_err());
}

#[test]
fn test_requirement_error_is_none_for_valid_requirements() {
    assert!(validate_pep508_requirement("requests").is_ok());
    assert!(validate_pep508_requirement("rich>=13,<16").is_ok());
    assert!(validate_pep508_requirement("demo[bold]").is_ok()); // extras are valid PEP 508
}

#[test]
fn test_requirement_error_localizes_a_message_for_an_invalid_requirement() {
    let error = validate_pep508_requirement("@@@").unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(
        message.starts_with("@@@ isn't a package requirement"),
        "{message}"
    );
}

// ==========================================================================
// 2. _validate_python_flags — validate + '-'/'none' normalization
//
// No public Rust `_validate_python_flags`; the same validate-then-normalize contract runs inside
// the `skit add -` lane (`add_with_config`), so each case is observed end-to-end.
// ==========================================================================

#[test]
fn test_validate_python_flags_passes_valid_and_normalizes_the_constraint() {
    // Oracle: _validate_python_flags(["requests", "rich>=13,<16"], ">=3.11") == ">=3.11".
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &[
            "add",
            "-",
            "-n",
            "flagvalid",
            "--dep",
            "requests",
            "--dep",
            "rich>=13,<16",
            "--python",
            ">=3.11",
        ],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    let stored = read_script(&sandbox, "flagvalid");
    assert!(stored.contains("requires-python = \">=3.11\""), "{stored}");
    assert!(stored.contains("\"requests\""), "{stored}");
    assert!(stored.contains("\"rich>=13,<16\""), "{stored}");
}

#[test]
fn test_validate_python_flags_normalizes_dash_and_none_to_empty() {
    // Oracle: _validate_python_flags(None, "-") == "" and "none" == "" and "  NONE  " == "".
    for (value, name) in [
        ("-", "flagdash"),
        ("none", "flagnonelit"),
        ("  NONE  ", "flagupper"),
    ] {
        let sandbox = Sandbox::new();
        let (code, _out) = run(
            &sandbox,
            &["add", "-", "-n", name, "--python", value],
            Some("print(1)\n"),
        );
        assert_eq!(code, Some(0), "value {value:?}");
        assert!(
            !read_script(&sandbox, name).contains("requires-python"),
            "value {value:?} left a requires-python"
        );
    }
}

#[test]
fn test_validate_python_flags_returns_none_when_no_python_given() {
    // Oracle: _validate_python_flags(["requests"], None) is None (nothing to record).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagnone", "--dep", "requests"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    let stored = read_script(&sandbox, "flagnone");
    assert!(stored.contains("\"requests\""), "{stored}");
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
fn test_validate_python_flags_treats_an_empty_python_as_empty() {
    // Oracle: _validate_python_flags(None, "   ") == "" (a blank constraint means automatic).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagempty", "--python", "   "],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    assert!(!read_script(&sandbox, "flagempty").contains("requires-python"));
}

#[test]
fn test_validate_python_flags_skips_empty_dep_strings() {
    // A whitespace-only --dep is dropped (not routed to the validator), matching the block-write.
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagskip", "--dep", "  "],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    assert!(!read_script(&sandbox, "flagskip").contains("dependencies"));
}

#[test]
fn test_validate_python_flags_exits_2_on_a_bad_dep() {
    // Oracle: _validate_python_flags(["@@@"], None) raises typer.Exit(EXIT_USAGE).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagbaddep", "--dep", "@@@"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2));
}

#[test]
fn test_validate_python_flags_exits_2_on_a_bad_python() {
    // Oracle: _validate_python_flags(None, "not-a-version") raises typer.Exit(EXIT_USAGE).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagbadpy", "--python", "not-a-version"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2));
}

// ==========================================================================
// 3. The interactive deps / python asks are re-ask loops on invalid input
//
// The real plain-form PTY drives the same dependency and Python questions a person sees. The
// dependency partition comes from the same parser-backed helper as the serializable add review.
// ==========================================================================

#[test]
#[cfg(unix)]
fn test_interactive_deps_reask_then_python_reask_then_accept() {
    for locale in ["en", "zh-CN", "zh-TW"] {
        let sandbox = Sandbox::new();
        configure_plain(&sandbox);
        let source = python_source(&sandbox, "retry.py", "import requests\nprint(requests)\n");
        let source_before = fs::read(&source).unwrap();
        let config_before = fs::read(sandbox.config.path().join("config.toml")).unwrap();
        let mut pty = PlainAddPty::spawn(
            sandbox.data.path(),
            sandbox.state.path(),
            sandbox.config.path(),
            sandbox.scratch.path(),
            locale,
            &["add", source.to_str().unwrap(), "-n", "retry"],
        );
        pty.wait_for(dependencies_question(locale));
        let checkpoint = pty.checkpoint();
        pty.send_line("@@@");
        pty.wait_for_after(dependencies_question(locale), checkpoint);
        pty.send_line("-");
        pty.wait_for(automatic_python_question(locale));
        let checkpoint = pty.checkpoint();
        pty.send_line("not-a-version");
        pty.wait_for_after(automatic_python_question(locale), checkpoint);
        pty.send_line(">=3.11");
        let (code, output) = pty.finish();
        assert_eq!(code, 0, "locale={locale}: {output}");

        let stored =
            fs::read_to_string(sandbox.data.path().join("scripts/retry/script.py")).unwrap();
        let metadata = read_uv_metadata(&stored).unwrap();
        assert!(
            metadata.dependencies.is_empty(),
            "locale={locale}: {stored}"
        );
        assert_eq!(metadata.requires_python, ">=3.11", "locale={locale}");
        let shown = sandbox.show_json("retry");
        assert_eq!(shown["dependencies"], serde_json::json!([]));
        assert_eq!(shown["requires_python"], ">=3.11");
        assert_eq!(fs::read(&source).unwrap(), source_before, "locale={locale}");
        assert_eq!(
            fs::read(sandbox.config.path().join("config.toml")).unwrap(),
            config_before
        );
        assert!(fs::read_dir(sandbox.state.path()).unwrap().next().is_none());
    }
}

#[test]
#[cfg(unix)]
fn test_interactive_valid_deps_accepted_first_try() {
    for locale in ["en", "zh-CN", "zh-TW"] {
        let sandbox = Sandbox::new();
        configure_plain(&sandbox);
        let source = python_source(
            &sandbox,
            "valid.py",
            "import requests\nimport rich\nprint(requests, rich)\n",
        );
        let source_before = fs::read(&source).unwrap();
        let config_before = fs::read(sandbox.config.path().join("config.toml")).unwrap();
        let mut pty = PlainAddPty::spawn(
            sandbox.data.path(),
            sandbox.state.path(),
            sandbox.config.path(),
            sandbox.scratch.path(),
            locale,
            &["add", source.to_str().unwrap(), "-n", "valid"],
        );
        pty.wait_for(dependencies_question(locale));
        pty.send_line("requests>=2,<3, rich");
        pty.wait_for(automatic_python_question(locale));
        pty.send_line("-");
        let (code, output) = pty.finish();
        assert_eq!(code, 0, "locale={locale}: {output}");

        let stored =
            fs::read_to_string(sandbox.data.path().join("scripts/valid/script.py")).unwrap();
        let metadata = read_uv_metadata(&stored).unwrap();
        assert_eq!(metadata.dependencies, ["requests>=2,<3", "rich"]);
        assert!(metadata.requires_python.is_empty());
        let shown = sandbox.show_json("valid");
        assert_eq!(
            shown["dependencies"],
            serde_json::json!(["requests>=2,<3", "rich"])
        );
        assert_eq!(shown["requires_python"], "");
        assert_eq!(fs::read(&source).unwrap(), source_before);
        assert_eq!(
            fs::read(sandbox.config.path().join("config.toml")).unwrap(),
            config_before
        );
        assert!(fs::read_dir(sandbox.state.path()).unwrap().next().is_none());
    }
}

#[cfg(unix)]
#[test]
fn interactive_python_metadata_cancel_and_retry_refusals_write_nothing() {
    for cancel_at_python in [false, true] {
        let sandbox = Sandbox::new();
        configure_plain(&sandbox);
        let source = python_source(&sandbox, "cancel.py", "import requests\nprint(requests)\n");
        let before = snapshot_sandbox(&sandbox);
        let source_before = fs::read(&source).unwrap();
        let mut pty = PlainAddPty::spawn(
            sandbox.data.path(),
            sandbox.state.path(),
            sandbox.config.path(),
            sandbox.scratch.path(),
            "en",
            &["add", source.to_str().unwrap(), "-n", "cancelled"],
        );
        pty.wait_for(dependencies_question("en"));
        if cancel_at_python {
            pty.send_line("-");
            pty.wait_for(automatic_python_question("en"));
            let checkpoint = pty.checkpoint();
            pty.send_line("not-a-version");
            pty.wait_for_after(automatic_python_question("en"), checkpoint);
        } else {
            let checkpoint = pty.checkpoint();
            pty.send_line("@@@");
            pty.wait_for_after(dependencies_question("en"), checkpoint);
        }
        pty.interrupt();
        let (code, _) = pty.finish();
        assert_ne!(code, 0);
        assert_eq!(snapshot_sandbox(&sandbox), before);
        assert_eq!(fs::read(&source).unwrap(), source_before);
    }
}

// ==========================================================================
// 4. exe / reference can never cross the drafts boundary (every face)
//
// The CLI add path (`add_with_config`) never checks `is_draft`, so none of these refusals fire and
// none of the drafts are kept — every case is a divergence against the oracle's boundary guard
// (cli.py:1894-1933). MUST-FIX tracked as "Refuse the add-lane inputs version 0.4 refuses".
// ==========================================================================

#[test]
fn test_exe_flag_on_a_kept_draft_is_refused_naming_only_exe() {
    // --exe alone → the refusal tells the user to drop --exe and NOTHING else: naming a flag
    // the user never passed (--ref) would be its own small lie. The honest-naming rule is the
    // point, so the other flag names must be absent.
    for language in ["en", "zh-CN", "zh-TW"] {
        let sandbox = Sandbox::new();
        let source = draft(&sandbox, "skit-new-prog.py", "print('run me')\n");
        let before = snapshot_sandbox(&sandbox);
        let (code, out) = run_in(
            &sandbox,
            language,
            &[
                "add",
                source.to_str().unwrap(),
                "-n",
                "p1",
                "--exe",
                "--no-input",
            ],
            None,
        );
        assert_eq!(code, Some(2), "language={language}: {out}");
        assert!(
            out.contains(&explicit_boundary_message(
                language,
                "skit-new-prog.py",
                "--exe"
            )),
            "language={language}: {out}"
        );
        assert!(!out.contains("--ref"), "{out}");
        assert!(!out.contains("--kind"), "{out}");
        assert_eq!(snapshot_sandbox(&sandbox), before, "language={language}");
        assert!(!entry_dir(&sandbox, "p1").exists());
    }
}

#[test]
fn test_kind_exe_on_a_kept_draft_is_refused_naming_only_kind_exe() {
    // --kind exe alone → the refusal names only "--kind exe"; --ref and --exe (neither passed)
    // stay out of the message.
    for language in ["en", "zh-CN", "zh-TW"] {
        let sandbox = Sandbox::new();
        let source = draft(&sandbox, "skit-new-prog2.py", "print('run me')\n");
        let before = snapshot_sandbox(&sandbox);
        let (code, out) = run_in(
            &sandbox,
            language,
            &[
                "add",
                source.to_str().unwrap(),
                "-n",
                "p2",
                "--kind",
                "exe",
                "--no-input",
            ],
            None,
        );
        assert_eq!(code, Some(2), "language={language}: {out}");
        assert!(
            out.contains(&explicit_boundary_message(
                language,
                "skit-new-prog2.py",
                "--kind exe"
            )),
            "language={language}: {out}"
        );
        assert!(!out.contains("--ref"), "{out}");
        assert!(!out.contains("--exe"), "{out}");
        assert_eq!(snapshot_sandbox(&sandbox), before, "language={language}");
        assert!(!entry_dir(&sandbox, "p2").exists());
    }
}

#[cfg(unix)]
#[test]
fn test_inferred_exe_on_a_kept_draft_is_refused_and_keeps_it() {
    use std::os::unix::fs::PermissionsExt as _;
    // A hand-planted +x bit on an extensionless draft INFERS exe — the widened guard covers the
    // inferred route just like the explicit flags. The INFERRED route (the user passed no flag)
    // gets the --kind message, not the Drop-flags one: there is no flag to drop, so it points at
    // the escape a draft can actually take.
    for language in ["en", "zh-CN", "zh-TW"] {
        let sandbox = Sandbox::new();
        let source = draft(&sandbox, "skit-new-binish", "opaque program bytes\n");
        let mut permissions = fs::metadata(&source).unwrap().permissions();
        permissions.set_mode(0o755); // POSIX infer_kind classifies +x as exe
        fs::set_permissions(&source, permissions).unwrap();
        let before = snapshot_sandbox(&sandbox);
        let (code, out) = run_in(
            &sandbox,
            language,
            &["add", source.to_str().unwrap(), "-n", "b1", "--no-input"],
            None,
        );
        assert_eq!(code, Some(2), "language={language}: {out}");
        assert!(
            out.contains(&inferred_boundary_message(language, "skit-new-binish")),
            "language={language}: {out}"
        );
        assert!(!out.contains("Drop"), "{out}");
        assert_eq!(snapshot_sandbox(&sandbox), before, "language={language}");
        assert!(!entry_dir(&sandbox, "b1").exists());
    }
}

#[cfg(unix)]
#[test]
fn rust_additive_explicit_language_overrides_inferred_draft_executable() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-typed",
        "#!/usr/bin/env bash\necho typed\n",
    );
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&source, permissions).unwrap();
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "typed",
            "--kind",
            "shell",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(!out.contains("one of skit's own kept drafts"), "{out}");
    assert!(read_meta(&sandbox, "typed").contains("kind = \"shell\""));
    assert!(read_meta(&sandbox, "typed").contains("mode = \"copy\""));
    assert!(!source.exists(), "the successful copy consumes the draft");
}

#[cfg(unix)]
#[test]
fn rust_additive_prompt_flag_overrides_inferred_draft_executable() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-prompt", "Summarize {{text}}.\n");
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&source, permissions).unwrap();
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "typed-prompt",
            "--prompt",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(!out.contains("one of skit's own kept drafts"), "{out}");
    assert!(read_meta(&sandbox, "typed-prompt").contains("kind = \"prompt\""));
    assert!(read_meta(&sandbox, "typed-prompt").contains("mode = \"copy\""));
    assert!(
        !source.exists(),
        "the successful prompt copy consumes the draft"
    );
}

#[test]
fn test_ref_flag_on_a_kept_draft_is_refused_naming_only_ref() {
    // --ref alone keeps refusing, naming ONLY --ref — --exe (never passed) stays out of the
    // message.
    for language in ["en", "zh-CN", "zh-TW"] {
        let sandbox = Sandbox::new();
        let source = draft(&sandbox, "skit-new-linkme.py", "print('link me')\n");
        let before = snapshot_sandbox(&sandbox);
        let (code, out) = run_in(
            &sandbox,
            language,
            &[
                "add",
                source.to_str().unwrap(),
                "-n",
                "lk",
                "--ref",
                "--no-input",
            ],
            None,
        );
        assert_eq!(code, Some(2), "language={language}: {out}");
        assert!(
            out.contains(&explicit_boundary_message(
                language,
                "skit-new-linkme.py",
                "--ref"
            )),
            "language={language}: {out}"
        );
        assert!(!out.contains("--exe"), "{out}");
        assert!(!out.contains("--kind"), "{out}");
        assert_eq!(snapshot_sandbox(&sandbox), before, "language={language}");
    }

    // The guard also owns ordering. A `.md` draft must not reach the interactive "looks like a
    // prompt" question when `--ref` already makes the request invalid.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-note.md", "# Summarize {{text}}.\n");
    let before = snapshot_sandbox(&sandbox);
    #[cfg(unix)]
    let (code, out) = run_pty_to_exit(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "md1", "--ref"],
    );
    #[cfg(not(unix))]
    let (code, out) = {
        let (code, out) = run(
            &sandbox,
            &["add", source.to_str().unwrap(), "-n", "md1", "--ref"],
            None,
        );
        (code.unwrap_or_default() as u32, out)
    };
    assert_eq!(code, 2, "{out}");
    assert!(
        out.contains(&explicit_boundary_message(
            "en",
            "skit-new-note.md",
            "--ref"
        )),
        "{out}"
    );
    assert!(!out.contains("looks like"), "{out}");
    assert_eq!(snapshot_sandbox(&sandbox), before);
}

#[test]
#[ignore = "SEMANTIC DUPLICATE (owned-draft root): the stronger canonical path-copy success and post-commit consume contract is port_test_add_lane_contracts::test_path_add_of_a_drafts_home_file_unlinks_it_on_copy. Keep this frozen body for oracle accounting."]
fn test_a_normal_draft_resume_still_adds_as_a_copy() {
    // The complement: a draft added with NO exe/ref flag resumes normally (copy, consumed on
    // success) — the guard fires only for the two forbidden shapes.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-ok.py", "print('ok')\n");
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "okentry",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    // Oracle: `store.resolve("okentry").meta.mode == "copy"`. The reachable Rust equivalent is the
    // stored `meta.toml`, whose `mode` field serializes StorageMode::Copy as the lowercase "copy".
    assert!(read_meta(&sandbox, "okentry").contains("mode = \"copy\""));
    assert!(!source.exists()); // consumed on success
}

// ==========================================================================
// 5. --dep / --python validated BEFORE the pipe is read or a draft materializes
// ==========================================================================

#[test]
fn test_stdin_garbage_python_exits_2_and_leaves_the_drafts_dir_empty() {
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "x", "--python", "garbage"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("isn't a Python version constraint"), "{out}");
    assert!(drafts_dir_is_empty(&sandbox)); // refused before mkstemp — no kept-draft fingerprint
    assert!(!entry_dir(&sandbox, "x").exists());
}

#[test]
fn test_stdin_garbage_dep_exits_2_and_leaves_the_drafts_dir_empty() {
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "y", "--dep", "@@@"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("isn't a package requirement"), "{out}");
    assert!(drafts_dir_is_empty(&sandbox));
}

#[test]
fn test_stdin_dash_python_is_automatic() {
    // '-' at --python means automatic: the add succeeds and the stored block carries no
    // requires-python.
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "auto", "--python", "-"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(!read_script(&sandbox, "auto").contains("requires-python"));
}

#[test]
fn test_stdin_valid_python_lands_in_the_stored_block() {
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "pinned", "--python", ">=3.11"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_script(&sandbox, "pinned").contains("requires-python = \">=3.11\""));
}

#[cfg(unix)]
#[test]
fn test_editor_lane_refuses_bad_python_before_opening_the_editor() {
    // The editor lane validates BEFORE the editor opens (the name-conflict precedent): a bad
    // --python is refused and open_in_editor is never called (no authoring session cost).
    let sandbox = Sandbox::new();
    let (editor, marker) = with_sentinel_editor(&sandbox);
    let (code, out) = run_pty(
        &sandbox,
        &["add", "--edit", "-n", "edX", "--python", "garbage"],
        &editor,
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("isn't a Python version constraint"), "{out}");
    assert!(!marker.exists(), "the editor never opened"); // opened == []
    assert!(drafts_dir_is_empty(&sandbox)); // no draft was materialized
}

#[cfg(unix)]
#[test]
fn test_editor_lane_refuses_bad_dep_before_opening_the_editor() {
    let sandbox = Sandbox::new();
    let (editor, marker) = with_sentinel_editor(&sandbox);
    let (code, out) = run_pty(
        &sandbox,
        &["add", "--edit", "-n", "edY", "--dep", "@@@"],
        &editor,
    );
    assert_eq!(code, 2, "{out}");
    assert!(out.contains("isn't a package requirement"), "{out}");
    assert!(!marker.exists(), "the editor never opened");
    assert!(drafts_dir_is_empty(&sandbox));
}

/// Configure a fake editor that (a) touches a marker so a run is observable and (b) writes valid
/// python so a materialized draft is non-empty. The oracle asserts the marker stays absent.
#[cfg(unix)]
fn with_sentinel_editor(sandbox: &Sandbox) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt as _;
    let marker = sandbox.config.path().join("editor-ran");
    let editor = sandbox.config.path().join("editor.sh");
    fs::write(
        &editor,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf 'print(1)\\n' > \"$1\"\n",
            marker.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&editor, permissions).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();
    (editor, marker)
}

// ==========================================================================
// 6. kind_for_draft: the exception keys on the rationale (placeholder-bodied kinds)
//
// No Rust kind_for_draft; the draft's kind is decided by the extension-first `infer_kind` in the
// add path, observed here through the stored `meta.toml` kind.
// ==========================================================================

#[test]
fn test_kind_for_draft_single_prompt_extension_outranks_the_shebang() {
    // A `.prompt` (single-extension) draft whose body opens with a #! resumes as a PROMPT: the
    // exception is keyed on placeholder_params, not on compound-suffix shape.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-note.prompt",
        "#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "psingle",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "psingle").contains("kind = \"prompt\""));
}

#[test]
fn test_kind_for_draft_extensionless_falls_through_to_the_shebang() {
    // No registered extension at all → by_ext is None → the shebang decides (here: shell).
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-plain", "#!/usr/bin/env bash\necho hi\n");
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "plain", "--no-input"],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "plain").contains("kind = \"shell\""));
}

#[test]
#[ignore = "SEMANTIC DUPLICATE (owned-draft root): shared classifier precedence is owned by skit-language::test_kind_for_draft_shebang_first and its real CLI consequence by port_test_draft_inference_and_reader_cli::test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks. Keep this frozen body for oracle accounting."]
fn test_kind_for_draft_script_suffix_stays_shebang_first() {
    // A `.py` script suffix is NOT placeholder-bodied, so the shebang still outranks it.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-shellish.py",
        "#!/usr/bin/env bash\necho drafted\n",
    );
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "shellish",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "shellish").contains("kind = \"shell\""));
}

#[test]
#[ignore = "SEMANTIC DUPLICATE (owned-draft root): the stronger single+compound suffix, exact stored bytes, and post-commit cleanup owner is port_test_add_feedback_contracts::test_prompt_draft_with_shebang_body_resumes_as_prompt. Keep this frozen body for oracle accounting."]
fn test_prompt_single_extension_draft_resumes_as_prompt_end_to_end() {
    // The CLI face of the single-extension prompt rule: the draft resumes as a prompt entry and
    // is consumed on success.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-summ.prompt",
        "#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "psumm", "--no-input"],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "psumm").contains("kind = \"prompt\""));
    assert!(!source.exists()); // consumed on success
}

// ==========================================================================
// 7. The unknown+shebang refusal: --exe for an on-disk file, --kind-only for a draft
// ==========================================================================

#[test]
fn test_nondraft_awk_shebang_refusal_offers_the_exe_escape() {
    let sandbox = Sandbox::new();
    let file = sandbox.data.path().join("report.awkish");
    fs::write(&file, "#!/usr/bin/awk -f\nBEGIN { print 1 }\n").unwrap();
    let (code, out) = run(
        &sandbox,
        &["add", file.to_str().unwrap(), "-n", "rep", "--no-input"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("names no interpreter skit knows"), "{out}");
    assert!(out.contains("--exe to run it directly"), "{out}"); // an on-disk file gets the escape
}

#[test]
#[ignore = "SEMANTIC DUPLICATE (owned-draft root): the stronger exact shebang voice, keep/no-write, and no-entry owner is port_test_draft_inference_and_reader_cli::test_cli_add_awk_shebang_draft_is_unknown_kept_with_kind_escape. Keep this frozen body for oracle accounting."]
fn test_kept_draft_awk_shebang_refusal_offers_only_kind() {
    // The same awk shebang, but as a KEPT DRAFT: --exe is refused at the boundary, so the hint
    // must NOT offer it — only --kind.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-report.py",
        "#!/usr/bin/awk -f\nBEGIN { print 1 }\n",
    );
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "repd", "--no-input"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("names no interpreter skit knows"), "{out}");
    assert!(out.contains("--kind <language> to choose one"), "{out}");
    assert!(!out.contains("--exe"), "{out}"); // the draft variant never offers the program escape
    assert!(source.exists());
}
