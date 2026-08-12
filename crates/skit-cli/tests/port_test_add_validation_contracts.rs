//! Public-process ports from Python `tests/test_add_validation_contracts.py` at
//! `main@206f9ef`.
//!
//! Private Python `_validate_python_flags` contracts are intentionally lifted to the real Rust add
//! intake: valid normalized values must reach the stored copy, while invalid values must exit with
//! usage status before an entry or kept-draft fingerprint exists.

use std::{fs, path::{Path, PathBuf}, process::Output};

use assert_cmd::Command;
use skit_application::EntryRepository as _;
use skit_domain::StorageMode;
use skit_store::FileStore;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
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

    fn run_stdin(&self, args: &[&str], input: &str) -> Captured {
        let mut command = self.command();
        let assertion = command.args(args).write_stdin(input).assert();
        let output = assertion.get_output();
        Captured {
            code: output.status.code().unwrap_or(-1),
            stdout: output.stdout.clone(),
            stderr: output.stderr.clone(),
        }
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn draft(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let root = self.data.path().join("drafts");
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn payload_text(&self, selector: &str) -> String {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap()
    }

    fn drafts_empty(&self) -> bool {
        let root = self.data.path().join("drafts");
        !root.exists() || fs::read_dir(root).unwrap().next().is_none()
    }
}

struct Captured {
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl Captured {
    fn text(&self) -> String {
        format!(
            "{}{}",
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
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

fn add_path(sandbox: &Sandbox, source: &Path, tail: &[&str]) -> Output {
    let mut command = sandbox.command();
    command.arg("add").arg(source).args(tail).output().unwrap()
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

#[test]
fn test_validate_python_flags_passes_valid_and_normalizes_the_constraint() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("valid.py", b"print(1)\n");

    let output = add_path(
        &sandbox,
        &source,
        &[
            "-n",
            "valid",
            "--dep",
            "requests",
            "--dep",
            "rich>=13,<16",
            "--python",
            ">=3.11",
            "--no-input",
        ],
    );

    assert_success(&output);
    let stored = sandbox.payload_text("valid");
    assert!(stored.contains("requests"), "{stored}");
    assert!(stored.contains("rich>=13,<16"), "{stored}");
    assert!(stored.contains("requires-python = \">=3.11\""), "{stored}");
}

#[test]
fn test_validate_python_flags_normalizes_dash_and_none_to_empty() {
    let sandbox = Sandbox::new();
    for (index, value) in ["-", "none", "  NONE  "].into_iter().enumerate() {
        let name = format!("auto-{index}");
        let filename = format!("auto-{index}.py");
        let source = sandbox.source(&filename, b"print(1)\n");
        let output = add_path(
            &sandbox,
            &source,
            &["-n", &name, "--python", value, "--no-input"],
        );
        assert_success(&output);
        assert!(
            !sandbox.payload_text(&name).contains("requires-python"),
            "automatic spelling {value:?} produced a pin"
        );
    }
}

#[test]
fn test_validate_python_flags_returns_none_when_no_python_given() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("nopin.py", b"print(1)\n");

    let output = add_path(
        &sandbox,
        &source,
        &["-n", "nopin", "--dep", "requests", "--no-input"],
    );

    assert_success(&output);
    let stored = sandbox.payload_text("nopin");
    assert!(stored.contains("requests"));
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
fn test_validate_python_flags_treats_an_empty_python_as_empty() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("empty.py", b"print(1)\n");

    let output = add_path(
        &sandbox,
        &source,
        &["-n", "empty", "--python", "   ", "--no-input"],
    );

    assert_success(&output);
    assert!(!sandbox.payload_text("empty").contains("requires-python"));
}

#[test]
fn test_validate_python_flags_skips_empty_dep_strings() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("emptydep.py", b"print(1)\n");

    let output = add_path(
        &sandbox,
        &source,
        &["-n", "emptydep", "--dep", "  ", "--no-input"],
    );

    assert_success(&output);
    assert!(!sandbox.payload_text("emptydep").contains("# /// script"));
}

#[test]
fn test_validate_python_flags_exits_2_on_a_bad_dep() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("baddep.py", b"print(1)\n");

    let output = add_path(
        &sandbox,
        &source,
        &["-n", "baddep", "--dep", "@@@", "--no-input"],
    );

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&combined(&output)).contains("isn't a package requirement"));
    assert!(sandbox.store().resolve("baddep").is_err());
}

#[test]
fn test_validate_python_flags_exits_2_on_a_bad_python() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("badpy.py", b"print(1)\n");

    let output = add_path(
        &sandbox,
        &source,
        &[
            "-n",
            "badpy",
            "--python",
            "not-a-version",
            "--no-input",
        ],
    );

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&combined(&output)).contains("isn't a Python version constraint"));
    assert!(sandbox.store().resolve("badpy").is_err());
}

#[test]
fn test_exe_flag_on_a_kept_draft_is_refused_naming_only_exe() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-prog.py", b"print('run me')\n");

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "p1", "--exe", "--no-input"],
    );
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("Drop --exe."), "{shown}");
    assert!(!shown.contains("--ref"), "{shown}");
    assert!(!shown.contains("--kind"), "{shown}");
    assert!(draft.exists());
    assert!(sandbox.store().resolve("p1").is_err());
}

#[test]
fn test_kind_exe_on_a_kept_draft_is_refused_naming_only_kind_exe() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-prog2.py", b"print('run me')\n");

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "p2", "--kind", "exe", "--no-input"],
    );
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("Drop --kind exe."), "{shown}");
    assert!(!shown.contains("--ref"), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(draft.exists());
    assert!(sandbox.store().resolve("p2").is_err());
}

#[cfg(unix)]
#[test]
fn test_inferred_exe_on_a_kept_draft_is_refused_and_keeps_it() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-binish", b"opaque program bytes\n");
    fs::set_permissions(&draft, fs::Permissions::from_mode(0o755)).unwrap();

    let output = add_path(&sandbox, &draft, &["-n", "b1", "--no-input"]);
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("pass --kind <language> to name its language"), "{shown}");
    assert!(!shown.contains("Drop"), "{shown}");
    assert!(draft.exists());
    assert!(sandbox.store().resolve("b1").is_err());
}

#[test]
fn test_ref_flag_on_a_kept_draft_is_refused_naming_only_ref() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-linkme.py", b"print('link me')\n");

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "lk", "--ref", "--no-input"],
    );
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("Drop --ref."), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(!shown.contains("--kind"), "{shown}");
    assert!(draft.exists());
}

#[test]
fn test_a_normal_draft_resume_still_adds_as_a_copy() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-ok.py", b"print('ok')\n");

    let output = add_path(&sandbox, &draft, &["-n", "okentry", "--no-input"]);

    assert_success(&output);
    assert_eq!(sandbox.store().resolve("okentry").unwrap().meta.mode, StorageMode::Copy);
    assert!(!draft.exists());
}

#[test]
fn test_stdin_garbage_python_exits_2_and_leaves_the_drafts_dir_empty() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "-", "-n", "x", "--python", "garbage"],
        "print(1)\n",
    );

    assert_eq!(output.code, 2, "{}", output.text());
    assert!(flat(&output.text()).contains("isn't a Python version constraint"));
    assert!(sandbox.drafts_empty());
    assert!(sandbox.store().resolve("x").is_err());
}

#[test]
fn test_stdin_garbage_dep_exits_2_and_leaves_the_drafts_dir_empty() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "-", "-n", "y", "--dep", "@@@"],
        "print(1)\n",
    );

    assert_eq!(output.code, 2, "{}", output.text());
    assert!(flat(&output.text()).contains("isn't a package requirement"));
    assert!(sandbox.drafts_empty());
    assert!(sandbox.store().resolve("y").is_err());
}

#[test]
fn test_stdin_dash_python_is_automatic() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "-", "-n", "auto", "--python", "-"],
        "print(1)\n",
    );

    assert_eq!(output.code, 0, "{}", output.text());
    assert!(!sandbox.payload_text("auto").contains("requires-python"));
}

#[test]
fn test_stdin_valid_python_lands_in_the_stored_block() {
    let sandbox = Sandbox::new();
    let output = sandbox.run_stdin(
        &["add", "-", "-n", "pinned", "--python", ">=3.11"],
        "print(1)\n",
    );

    assert_eq!(output.code, 0, "{}", output.text());
    assert!(
        sandbox
            .payload_text("pinned")
            .contains("requires-python = \">=3.11\"")
    );
}

#[test]
fn test_prompt_single_extension_draft_resumes_as_prompt_end_to_end() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft(
        "skit-new-summ.prompt",
        b"#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "sumone", "--no-input"],
    );

    assert_success(&output);
    assert_eq!(sandbox.store().resolve("sumone").unwrap().meta.kind.as_str(), "prompt");
    assert!(!draft.exists());
}

#[test]
fn test_nondraft_awk_shebang_refusal_offers_the_exe_escape() {
    let sandbox = Sandbox::new();
    let source = sandbox.source(
        "report.awkish",
        b"#!/usr/bin/awk -f\nBEGIN { print 1 }\n",
    );

    let output = add_path(
        &sandbox,
        &source,
        &["-n", "rep", "--no-input"],
    );
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("names no interpreter skit knows"), "{shown}");
    assert!(shown.contains("--exe to run it directly"), "{shown}");
}

#[test]
fn test_kept_draft_awk_shebang_refusal_offers_only_kind() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft(
        "skit-new-report.py",
        b"#!/usr/bin/awk -f\nBEGIN { print 1 }\n",
    );

    let output = add_path(
        &sandbox,
        &draft,
        &["-n", "drep", "--no-input"],
    );
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("names no interpreter skit knows"), "{shown}");
    assert!(shown.contains("--kind"), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(draft.exists());
}
