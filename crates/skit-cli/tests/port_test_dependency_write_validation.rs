//! Public-process ports from Python `tests/test_dependency_write_validation.py` at
//! `main@206f9ef`.
//!
//! Rust keeps Python dependency authority in the stored PEP 723 block rather than duplicating the
//! same values in raw entry metadata. These tests therefore pin the stronger observable trio:
//! byte-exact stored source, `deps --json`, and (for unpinning) the `run --dry-run` command. A
//! rejected edit must change none of those channels or the independent `needs` axis.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    process::Output,
    thread,
    time::Duration,
};

use assert_cmd::Command;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
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
        self.env_assert(&mut command);
        command
    }

    fn env_assert(&self, command: &mut Command) {
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
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }

    fn create_copy(&self, name: &str, kind: &str, bytes: &[u8]) -> skit_domain::Entry {
        let stored_name = match kind {
            "python" => "script.py",
            "js" => "script.js",
            other => panic!("unsupported fixture kind: {other}"),
        };
        self.store()
            .create(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse(kind).unwrap(),
                mode: StorageMode::Copy,
                source: self
                    .home
                    .path()
                    .join(format!("{name}.{kind}"))
                    .display()
                    .to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: bytes.to_vec(),
                    stored_name: Some(stored_name.to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap()
    }

    fn stored(&self, selector: &str) -> Vec<u8> {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        fs::read(store.payload_path(&entry).unwrap()).unwrap()
    }

    fn deps_json(&self, selector: &str) -> serde_json::Value {
        let output = self.run(&["deps", selector, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn draft(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let drafts = self.data.path().join("drafts");
        fs::create_dir_all(&drafts).unwrap();
        let path = drafts.join(name);
        fs::write(&path, bytes).unwrap();
        path
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

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

fn set_uv(sandbox: &Sandbox, selector: &str, dependency: &str, python: Option<&str>) {
    let mut args = vec!["deps", selector, "--dep", dependency];
    if let Some(python) = python {
        args.extend(["--python", python]);
    }
    assert_success(&sandbox.run(&args));
}

#[test]
fn test_deps_garbage_dep_is_refused_and_nothing_changes() {
    let sandbox = Sandbox::new();
    sandbox.create_copy(
        "a",
        "python",
        b"import requests\n# /// script\n# dependencies = [\"rich\"]\n# ///\nprint(1)\n",
    );
    let before = sandbox.stored("a");
    let before_view = sandbox.deps_json("a");

    let output = sandbox.run(&["deps", "a", "--dep", "@@@"]);
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("isn't a package requirement"), "{shown}");
    assert_eq!(sandbox.stored("a"), before);
    assert_eq!(sandbox.deps_json("a"), before_view);
}

#[test]
fn test_deps_garbage_python_is_refused_and_nothing_changes() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    set_uv(&sandbox, "a", "requests", Some(">=3.11"));
    let before = sandbox.stored("a");
    let before_view = sandbox.deps_json("a");

    let output = sandbox.run(&["deps", "a", "--python", "not-a-version"]);
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("isn't a Python version constraint"), "{shown}");
    assert_eq!(sandbox.stored("a"), before);
    assert_eq!(sandbox.deps_json("a"), before_view);
}

#[test]
fn test_deps_dash_python_clears_meta_and_unpins_the_block() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    set_uv(&sandbox, "a", "requests", Some(">=3.11"));
    assert!(String::from_utf8_lossy(&sandbox.stored("a")).contains("requires-python = \">=3.11\""));

    let output = sandbox.run(&["deps", "a", "--python", "-"]);
    assert_success(&output);

    let stored = String::from_utf8(sandbox.stored("a")).unwrap();
    assert!(!stored.contains("requires-python"), "{stored}");
    assert_eq!(sandbox.deps_json("a")["requires_python"], "");
    let dry = sandbox.run(&["run", "a", "--dry-run", "--no-input"]);
    assert_success(&dry);
    assert!(!flat(&combined(&dry)).contains("--python"), "{}", combined(&dry));
}

#[test]
fn test_deps_only_edit_still_preserves_the_blocks_own_pin() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    set_uv(&sandbox, "a", "requests", Some(">=3.11"));

    let output = sandbox.run(&["deps", "a", "--dep", "rich"]);
    assert_success(&output);

    let stored = String::from_utf8(sandbox.stored("a")).unwrap();
    assert!(stored.contains("requires-python = \">=3.11\""), "{stored}");
    assert!(stored.contains("rich"), "{stored}");
    assert_eq!(sandbox.deps_json("a")["requires_python"], ">=3.11");
}

#[test]
fn test_deps_none_python_clears_meta_when_nothing_to_preserve() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    set_uv(&sandbox, "a", "requests", None);

    let output = sandbox.run(&[
        "deps",
        "a",
        "--dep",
        "requests",
        "--python",
        "none",
    ]);
    assert_success(&output);

    let stored = String::from_utf8(sandbox.stored("a")).unwrap();
    assert!(!stored.contains("requires-python"), "{stored}");
    assert_eq!(sandbox.deps_json("a")["requires_python"], "");
}

#[test]
fn test_deps_valid_dep_and_python_still_write() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&[
        "deps",
        "a",
        "--dep",
        "requests>=2,<3",
        "--python",
        "~=3.12",
    ]);
    assert_success(&output);

    let view = sandbox.deps_json("a");
    assert_eq!(view["dependencies"], serde_json::json!(["requests>=2,<3"]));
    assert_eq!(view["requires_python"], "~=3.12");
    let stored = String::from_utf8(sandbox.stored("a")).unwrap();
    assert!(stored.contains("requests>=2,<3"), "{stored}");
    assert!(stored.contains("requires-python = \"~=3.12\""), "{stored}");
}

#[test]
fn test_deps_refused_write_leaves_needs_untouched() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    assert_success(&sandbox.run(&["deps", "a", "--need", "jq"]));
    let before = sandbox.stored("a");

    let output = sandbox.run(&[
        "deps", "a", "--dep", "@@@", "--need", "ffmpeg",
    ]);

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert_eq!(sandbox.deps_json("a")["needs"], serde_json::json!(["jq"]));
    assert_eq!(sandbox.stored("a"), before);
}

#[test]
fn test_deps_npm_entry_takes_an_npm_shaped_dep_that_fails_pep508() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");

    let output = sandbox.run(&["deps", "jsx", "--dep", "@scope/thing"]);

    assert_success(&output);
    assert_eq!(
        sandbox.deps_json("jsx")["dependencies"],
        serde_json::json!(["@scope/thing"])
    );
}

#[test]
fn test_update_dependencies_uv_invalid_dep_raises_usage_error() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    let before = sandbox.stored("a");

    let output = sandbox.run(&["deps", "a", "--dep", "@@@"]);
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("isn't a package requirement"), "{shown}");
    assert_eq!(sandbox.stored("a"), before);
    assert!(sandbox.deps_json("a")["dependencies"].as_array().unwrap().is_empty());
}

#[test]
fn test_update_dependencies_uv_invalid_python_raises_usage_error() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    let before = sandbox.stored("a");

    let output = sandbox.run(&[
        "deps",
        "a",
        "--dep",
        "requests",
        "--python",
        "not-a-version",
    ]);
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("isn't a Python version constraint"), "{shown}");
    assert_eq!(sandbox.stored("a"), before);
}

#[test]
fn test_update_dependencies_drops_a_whitespace_only_dep_at_the_chokepoint() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&["deps", "a", "--dep", "  ", "--dep", "requests"]);
    assert_success(&output);

    assert_eq!(
        sandbox.deps_json("a")["dependencies"],
        serde_json::json!(["requests"])
    );
    let stored = String::from_utf8(sandbox.stored("a")).unwrap();
    assert!(stored.contains("requests"));
    assert!(!stored.contains("\"  \""), "{stored}");
}

#[test]
fn test_update_dependencies_all_whitespace_list_clears_deps() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    set_uv(&sandbox, "a", "requests", None);

    let output = sandbox.run(&["deps", "a", "--dep", "   ", "--dep", "\t"]);
    assert_success(&output);

    assert!(sandbox.deps_json("a")["dependencies"].as_array().unwrap().is_empty());
    let stored = String::from_utf8(sandbox.stored("a")).unwrap();
    assert!(stored.contains("dependencies = []"), "{stored}");
    assert!(!stored.contains("requests"), "{stored}");
}

#[test]
fn test_update_dependencies_npm_flavor_skips_uv_validation() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");

    let output = sandbox.run(&["deps", "jsx", "--dep", "@scope/thing"]);

    assert_success(&output);
    assert_eq!(
        sandbox.deps_json("jsx")["dependencies"],
        serde_json::json!(["@scope/thing"])
    );
}

#[test]
fn test_update_dependencies_normalizes_dash_python_before_validating() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&[
        "deps",
        "a",
        "--dep",
        "requests",
        "--python",
        "-",
    ]);

    assert_success(&output);
    assert_eq!(sandbox.deps_json("a")["requires_python"], "");
}

#[test]
fn test_no_input_add_of_an_illegally_named_import_writes_no_block() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("cafe.py");
    fs::write(&source, "import café\nprint(café)\n").unwrap();

    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "cafe", "--no-input"])
        .output()
        .unwrap();

    assert_success(&output);
    let stored = String::from_utf8(sandbox.stored("cafe")).unwrap();
    assert!(!stored.contains("# /// script"), "{stored}");
    assert!(sandbox.deps_json("cafe")["dependencies"].as_array().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn test_inferred_exe_draft_gets_the_kind_variant() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-binish", b"opaque program bytes\n");
    fs::set_permissions(&draft, fs::Permissions::from_mode(0o755)).unwrap();

    let output = sandbox
        .command()
        .arg("add")
        .arg(&draft)
        .args(["--name", "b1", "--no-input"])
        .output()
        .unwrap();
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("pass --kind <language> to name its language"), "{shown}");
    assert!(!shown.contains("Drop"), "{shown}");
    assert!(draft.exists());
}

#[test]
fn test_exe_flag_on_the_same_draft_gets_the_drop_variant_naming_only_exe() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-binish2", b"opaque program bytes\n");

    let output = sandbox
        .command()
        .arg("add")
        .arg(&draft)
        .args(["--name", "b2", "--exe", "--no-input"])
        .output()
        .unwrap();
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("Drop --exe."), "{shown}");
    assert!(!shown.contains("--ref"), "{shown}");
    assert!(!shown.contains("--kind"), "{shown}");
    assert!(!shown.contains("to name its language"), "{shown}");
    assert!(draft.exists());
}

#[test]
fn test_shebang_less_unclassifiable_draft_gets_the_classify_variant() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-weird.xyz", b"just some content\n");

    let output = sandbox
        .command()
        .arg("add")
        .arg(&draft)
        .args(["--name", "w1", "--no-input"])
        .output()
        .unwrap();
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("kept draft skit can't classify"), "{shown}");
    assert!(shown.contains("--kind <language> to add it as a script"), "{shown}");
    assert!(shown.contains("--prompt for an AI-agent prompt"), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(!shown.contains("--cmd"), "{shown}");
    assert!(draft.exists());
}

#[test]
fn test_same_unclassifiable_file_outside_drafts_gets_the_full_escape() {
    let sandbox = Sandbox::new();
    let source = sandbox.home.path().join("weird.xyz");
    fs::write(&source, b"just some content\n").unwrap();

    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "w2", "--no-input"])
        .output()
        .unwrap();
    let shown = flat(&combined(&output));

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("isn't a script or an executable"), "{shown}");
    assert!(shown.contains("--exe for a program"), "{shown}");
    assert!(shown.contains("--cmd for a command template"), "{shown}");
    assert!(!shown.contains("kept draft"), "{shown}");
}

#[test]
fn test_ref_on_an_md_draft_is_refused_before_the_prompt_ask() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-note.md", b"# Summarize {{text}}.\n");
    let (code, transcript) = run_pty_add(&sandbox, &draft, &["--name", "md1", "--ref"], b"y\n");
    let shown = flat(&transcript);

    assert_eq!(code, 2, "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("Drop --ref."), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(
        !shown.contains("looks like a prompt") && !shown.contains("AI-agent prompt"),
        "the prompt-kind ask ran before the draft refusal: {shown}"
    );
    assert!(draft.exists());
    assert!(sandbox.store().resolve("md1").is_err());
}

fn run_pty_add(
    sandbox: &Sandbox,
    source: &Path,
    args: &[&str],
    input: &[u8],
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 30,
            cols: 140,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.arg("add");
    command.arg(source);
    for arg in args {
        command.arg(arg);
    }
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", "en");
    command.env("HOME", sandbox.home.path());
    command.env("USERPROFILE", sandbox.home.path());
    command.env("XDG_CONFIG_HOME", sandbox.home.path().join("xdg-config"));
    command.env("XDG_DATA_HOME", sandbox.home.path().join("xdg-data"));
    command.env("XDG_STATE_HOME", sandbox.home.path().join("xdg-state"));

    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(120));
    writer.write_all(input).unwrap();
    writer.flush().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    let transcript = String::from_utf8_lossy(&drain.join().unwrap())
        .replace("\r\n", "\n")
        .replace('\r', "");
    (status.exit_code(), transcript)
}
