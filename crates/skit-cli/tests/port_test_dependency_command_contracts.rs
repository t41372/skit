//! Public-process ports from Python `tests/test_dependency_command_contracts.py` at
//! `main@206f9ef`.
//!
//! Python exposed part of this policy through `store.update_dependencies`; Rust owns the validation
//! in the CLI/use-case instead. The same-named Rust tests deliberately cross that stronger public
//! chokepoint and inspect persisted source/JSON/no-entry consequences rather than pretending
//! `FileStore` validates package grammar itself.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
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
                source: self.home.path().join(stored_name).display().to_string(),
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

    fn payload(&self, selector: &str) -> Vec<u8> {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        fs::read(store.payload_path(&entry).unwrap()).unwrap()
    }

    fn deps_json(&self, selector: &str) -> serde_json::Value {
        let output = self.run(&["deps", selector, "--json"]);
        assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn flat(output: &Output) -> String {
    combined(output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

fn run_add(sandbox: &Sandbox, source: &Path, tail: &[&str]) -> Output {
    let mut command = sandbox.command();
    command.arg("add").arg(source).args(tail).output().unwrap()
}

fn pin_python(sandbox: &Sandbox, selector: &str) {
    assert_success(&sandbox.run(&["deps", selector, "--dep", "requests", "--python", ">=3.11"]));
}

#[test]
fn test_two_flags_together_are_both_named_and_joined() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-both.py", b"print('x')\n");

    let output = run_add(
        &sandbox,
        &draft,
        &["--name", "both", "--ref", "--exe", "--no-input"],
    );
    let shown = flat(&output);

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("one of skit's own kept drafts"), "{shown}");
    assert!(shown.contains("Drop --ref/--exe."), "{shown}");
    assert!(!shown.contains("--kind"), "{shown}");
    assert!(draft.exists());
    assert!(sandbox.store().resolve("both").is_err());
}

#[test]
fn test_kind_exe_alone_names_only_kind_exe() {
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-kindonly.py", b"print('x')\n");

    let output = run_add(
        &sandbox,
        &draft,
        &["--name", "kindonly", "--kind", "exe", "--no-input"],
    );
    let shown = flat(&output);

    assert_eq!(output.status.code(), Some(2), "{shown}");
    assert!(shown.contains("Drop --kind exe."), "{shown}");
    assert!(!shown.contains("--ref"), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(draft.exists());
}

#[test]
fn test_js_deps_python_dash_is_refused_as_inapplicable() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");

    let output = sandbox.run(&["deps", "jsx", "--python", "-"]);

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        flat(&output).contains("A Python constraint doesn't apply to js scripts."),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_js_deps_python_none_is_refused_as_inapplicable() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");

    let output = sandbox.run(&["deps", "jsx", "--python", "none"]);

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("A Python constraint doesn't apply to js scripts."));
}

#[test]
fn test_js_deps_python_empty_string_is_refused_as_inapplicable() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");
    assert_success(&sandbox.run(&["deps", "jsx", "--dep", "chalk"]));
    let before = sandbox.deps_json("jsx");

    let output = sandbox.run(&["deps", "jsx", "--python", ""]);

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("A Python constraint doesn't apply to js scripts."));
    assert_eq!(sandbox.deps_json("jsx"), before);
}

#[test]
fn test_python_deps_python_dash_is_still_automatic() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    pin_python(&sandbox, "a");

    let output = sandbox.run(&["deps", "a", "--python", "-"]);

    assert_success(&output);
    assert_eq!(sandbox.deps_json("a")["requires_python"], "");
    assert!(!String::from_utf8_lossy(&sandbox.payload("a")).contains("requires-python"));
}

#[test]
fn test_store_npm_spec_plus_dash_reaches_the_npm_refusal() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");
    let before = sandbox.deps_json("jsx");

    let output = sandbox.run(&["deps", "jsx", "--python", "-"]);

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("doesn't apply"));
    assert_eq!(sandbox.deps_json("jsx"), before);
}

#[test]
fn test_store_uv_spec_plus_dash_normalizes() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&["deps", "a", "--dep", "requests", "--python", "none"]);

    assert_success(&output);
    assert_eq!(sandbox.deps_json("a")["requires_python"], "");
}

#[test]
fn test_store_npm_spec_plus_empty_string_reaches_the_npm_refusal() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");

    let output = sandbox.run(&["deps", "jsx", "--python", ""]);

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("doesn't apply"));
}

#[test]
fn test_store_npm_spec_plus_none_deps_edit_is_not_refused() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");

    let output = sandbox.run(&["deps", "jsx", "--dep", "chalk"]);

    assert_success(&output);
    assert_eq!(
        sandbox.deps_json("jsx")["dependencies"],
        serde_json::json!(["chalk"])
    );
}

#[test]
fn test_add_python_belt_rejects_a_bad_dep_before_any_entry_exists() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("belt.py", b"print(1)\n");

    let output = run_add(
        &sandbox,
        &source,
        &["--name", "belt", "--dep", "@@@", "--no-input"],
    );

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("isn't a package requirement"));
    assert!(sandbox.store().resolve("belt").is_err());
    assert!(!sandbox.data.path().join("scripts/belt").exists());
}

#[test]
fn test_add_python_belt_rejects_a_bad_python_before_any_entry_exists() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("belt.py", b"print(1)\n");

    let output = run_add(
        &sandbox,
        &source,
        &["--name", "belt", "--python", "not-a-version", "--no-input"],
    );

    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(flat(&output).contains("isn't a Python version constraint"));
    assert!(sandbox.store().resolve("belt").is_err());
    assert!(!sandbox.data.path().join("scripts/belt").exists());
}

#[test]
fn test_add_python_belt_drops_a_whitespace_dep_from_the_block() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("belt2.py", b"print(1)\n");

    let output = run_add(
        &sandbox,
        &source,
        &[
            "--name",
            "belt2",
            "--dep",
            "  ",
            "--dep",
            "rich",
            "--no-input",
        ],
    );

    assert_success(&output);
    let stored = String::from_utf8(sandbox.payload("belt2")).unwrap();
    assert!(stored.contains("rich"));
    assert!(!stored.contains("\"  \""), "{stored}");
    assert_eq!(
        sandbox.deps_json("belt2")["dependencies"],
        serde_json::json!(["rich"])
    );
}

#[test]
fn test_add_python_belt_with_no_deps_is_unchanged() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("plain.py", b"print(1)\n");

    let output = run_add(&sandbox, &source, &["--name", "plain", "--no-input"]);

    assert_success(&output);
    let stored = String::from_utf8(sandbox.payload("plain")).unwrap();
    assert!(!stored.contains("# /// script"), "{stored}");
    assert!(
        sandbox.deps_json("plain")["dependencies"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_deps_python_only_prints_the_constraint_line_not_the_deps_line() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&["deps", "a", "--python", ">=3.11"]);
    let shown = flat(&output);

    assert_success(&output);
    assert!(
        shown.contains("Python constraint of a updated: >=3.11"),
        "{shown}"
    );
    assert!(!shown.contains("Dependencies"), "{shown}");
}

#[test]
fn test_deps_python_only_dash_reports_the_dash_placeholder() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    pin_python(&sandbox, "a");

    let output = sandbox.run(&["deps", "a", "--python", "-"]);
    let shown = flat(&output);

    assert_success(&output);
    assert!(
        shown.contains("Python constraint of a updated: —"),
        "{shown}"
    );
    assert!(!shown.contains("Dependencies"), "{shown}");
}

#[test]
fn test_deps_dep_only_prints_the_deps_line() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&["deps", "a", "--dep", "requests"]);
    let shown = flat(&output);

    assert_success(&output);
    assert!(
        shown.contains("Dependencies of a updated: requests"),
        "{shown}"
    );
    assert!(!shown.contains("Python constraint of"), "{shown}");
}

#[test]
fn test_deps_dep_and_python_together_prints_both_axis_lines() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");

    let output = sandbox.run(&["deps", "a", "--dep", "rich", "--python", ">=3.12"]);
    let shown = flat(&output);

    assert_success(&output);
    assert!(shown.contains("Dependencies of a updated: rich"), "{shown}");
    assert!(
        shown.contains("Python constraint of a updated: >=3.12"),
        "{shown}"
    );
    assert_eq!(sandbox.deps_json("a")["requires_python"], ">=3.12");
}

#[test]
fn test_deps_clear_prints_the_deps_line() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("a", "python", b"print(1)\n");
    assert_success(&sandbox.run(&["deps", "a", "--dep", "requests"]));

    let output = sandbox.run(&["deps", "a", "--clear"]);
    let shown = flat(&output);

    assert_success(&output);
    assert!(shown.contains("Dependencies of a updated: —"), "{shown}");
    assert!(!shown.contains("Python constraint of"), "{shown}");
}

#[test]
fn test_js_is_npm_flavor_and_python_is_not() {
    let sandbox = Sandbox::new();
    sandbox.create_copy("jsx", "js", b"console.log(1);\n");
    sandbox.create_copy("py", "python", b"print(1)\n");

    let npm = sandbox.run(&["deps", "jsx", "--dep", "@scope/thing"]);
    let js_python = sandbox.run(&["deps", "jsx", "--python", ">=3.11"]);
    let python = sandbox.run(&["deps", "py", "--python", ">=3.11"]);

    assert_success(&npm);
    assert_eq!(js_python.status.code(), Some(2), "{}", combined(&js_python));
    assert_success(&python);
    assert_eq!(
        sandbox.deps_json("jsx")["dependencies"],
        serde_json::json!(["@scope/thing"])
    );
    assert_eq!(sandbox.deps_json("py")["requires_python"], ">=3.11");
}
