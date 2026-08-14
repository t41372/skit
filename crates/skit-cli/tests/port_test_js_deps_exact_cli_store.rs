//! Exact public-process/store ports from Python v0.4 `tests/test_js_deps.py`.
//!
//! Python exposed several policies through `store.update_dependencies`; Rust moves part of that
//! policy into the CLI/use-case boundary. These tests keep the frozen user-visible transaction,
//! refusal, JSON, and filesystem results. Current Rust wording may differ and is allowed to fail.

use std::{fs, path::{Path, PathBuf}, process::Output};

use assert_cmd::Command;
use skit_application::EntryRepository as _;
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

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    fn add_js(&self, name: &str, body: &str, extra: &[&str]) -> Output {
        let path = self.source(&format!("{name}.mjs"), body);
        let mut command = self.command();
        command
            .arg("add")
            .arg(path)
            .args(["--name", name])
            .args(extra)
            .arg("--no-input")
            .output()
            .unwrap()
    }

    fn entry_dir(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.entry_dir_path(&entry.slug)
    }

    fn payload(&self, selector: &str) -> PathBuf {
        let store = self.store();
        let entry = store.resolve(selector).unwrap();
        store.payload_path(&entry).unwrap()
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

fn assert_success(output: &Output) {
    assert_eq!(output.status.code(), Some(0), "{}", combined(output));
}

#[test]
fn test_update_dependencies_js_copy_records_meta_without_touching_the_script() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let path = sandbox.payload("t");
    let before = fs::read(&path).unwrap();

    let output = sandbox.run(&["deps", "t", "--dep", "chalk@^5"]);
    assert_success(&output);

    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!(["chalk@^5"]));
    assert_eq!(fs::read(path).unwrap(), before, "JS dependency metadata rewrote the stored script");
}

#[test]
fn test_update_dependencies_js_reference_is_refused() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &["--ref"]));
    let output = sandbox.run(&["deps", "t", "--dep", "chalk"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("reference-mode"), "{text}");
}

#[test]
fn test_update_dependencies_js_python_constraint_is_refused() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&["deps", "t", "--dep", "chalk", "--python", ">=3.11"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("Python constraint"), "{text}");
}

#[test]
fn test_update_dependencies_js_clearing_sweeps_the_materialized_env() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    assert_success(&sandbox.run(&["deps", "t", "--dep", "chalk"]));
    let entry_dir = sandbox.entry_dir("t");
    fs::write(entry_dir.join("package.json"), "{}\n").unwrap();
    fs::create_dir(entry_dir.join("node_modules")).unwrap();

    let output = sandbox.run(&["deps", "t", "--clear"]);
    assert_success(&output);
    assert!(!entry_dir.join("package.json").exists());
    assert!(!entry_dir.join("node_modules").exists());
    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!([]));
}

#[test]
fn test_update_dependencies_js_reference_clearing_is_allowed() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &["--ref"]));
    let output = sandbox.run(&["deps", "t", "--clear"]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!([]));
}

#[test]
fn test_add_js_no_input_records_scanned_imports() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js(
        "t",
        "import chalk from \"chalk\";\nimport { z } from \"zod\";\n",
        &[],
    );
    let text = combined(&output);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!(["chalk", "zod"]));
    assert!(text.contains("chalk, zod"), "{text}");
}

#[test]
fn test_add_js_explicit_dep_flags_win_without_scanning() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js(
        "t",
        "import chalk from \"chalk\";\n",
        &["--dep", "zod@3", "--dep", "execa"],
    );
    assert_success(&output);
    assert_eq!(
        sandbox.deps_json("t")["dependencies"],
        serde_json::json!(["zod@3", "execa"])
    );
}

#[test]
fn test_add_js_without_external_imports_records_nothing() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js("t", "import fs from \"node:fs\";\nconsole.log(1);\n", &[]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!([]));
}

#[test]
fn test_add_js_reference_mode_asks_no_deps_question() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js("t", "import chalk from \"chalk\";\n", &["--ref"]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!([]));
}

#[test]
fn test_deps_command_sets_and_shows_js_dependencies() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    assert_success(&sandbox.run(&["deps", "t", "--dep", "chalk@^5"]));

    let human = sandbox.run(&["deps", "t"]);
    assert_success(&human);
    assert!(combined(&human).contains("chalk@^5"));

    let json = sandbox.run(&["deps", "t", "--json"]);
    assert_success(&json);
    assert!(String::from_utf8_lossy(&json.stdout).contains("\"chalk@^5\""));
}

#[test]
fn test_deps_command_python_flag_on_js_is_refused() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&["deps", "t", "--python", ">=3.11"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("Python constraint"), "{text}");
}

#[test]
fn test_deps_command_dep_on_js_reference_is_refused() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &["--ref"]));
    let output = sandbox.run(&["deps", "t", "--dep", "chalk"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("reference-mode"), "{text}");
}

#[test]
fn test_add_js_ref_with_dep_is_refused_loudly() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js("t", "import chalk from \"chalk\";\n", &["--ref", "--dep", "chalk"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("Reference-mode"), "{text}");
    assert!(sandbox.store().resolve("t").is_err());
}

#[test]
fn test_add_js_with_python_flag_is_refused_loudly() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js("t", "console.log(1);\n", &["--python", ">=3.11"]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("Python constraint"), "{text}");
    assert!(sandbox.store().resolve("t").is_err());
}

#[test]
fn test_add_js_empty_dep_records_nothing() {
    let sandbox = Sandbox::new();
    let output = sandbox.add_js("j", "console.log(1);\n", &["--dep", "  "]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("j")["dependencies"], serde_json::json!([]));
}

#[test]
fn test_deps_command_empty_dep_clears_and_sweeps() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    assert_success(&sandbox.run(&["deps", "t", "--dep", "chalk"]));
    let entry_dir = sandbox.entry_dir("t");
    fs::create_dir(entry_dir.join("node_modules")).unwrap();

    let output = sandbox.run(&["deps", "t", "--dep", ""]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("t")["dependencies"], serde_json::json!([]));
    assert!(!entry_dir.join("node_modules").exists());
}

#[test]
fn test_deps_command_write_emits_json_when_asked() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&["deps", "t", "--dep", "chalk@^5", "--json"]);
    assert_success(&output);
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["dependencies"], serde_json::json!(["chalk@^5"]));
}

#[test]
fn test_deps_command_needs_write_emits_json_and_skips_the_human_line() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&["deps", "t", "--need", "jq", "--json"]);
    assert_success(&output);
    let payload: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["needs"], serde_json::json!(["jq"]));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("updated"));
}

#[test]
fn test_deps_command_applies_both_deps_and_needs() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&["deps", "t", "--dep", "chalk", "--need", "jq"]);
    assert_success(&output);
    let payload = sandbox.deps_json("t");
    assert_eq!(payload["dependencies"], serde_json::json!(["chalk"]));
    assert_eq!(payload["needs"], serde_json::json!(["jq"]));
}

#[test]
fn test_deps_command_refused_dep_does_not_commit_a_concurrent_need() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&["deps", "t", "--need", "jq", "--python", ">=3.11"]);
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert_eq!(sandbox.deps_json("t")["needs"], serde_json::json!([]));
}

#[test]
fn test_deps_command_drops_empty_and_whitespace_needs() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.add_js("t", "console.log(1);\n", &[]));
    let output = sandbox.run(&[
        "deps", "t", "--need", "  ", "--need", " jq ", "--need", "",
    ]);
    assert_success(&output);
    assert_eq!(sandbox.deps_json("t")["needs"], serde_json::json!(["jq"]));
}

#[test]
fn test_add_shell_refuses_unusable_flags_loudly() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("d.sh", "#!/bin/sh\necho hi\n");
    for (args, fragment) in [
        (vec!["--dep", "requests"], "don't take package dependencies"),
        (vec!["--python", ">=3.11"], "Python constraint"),
    ] {
        let mut command = sandbox.command();
        let output = command
            .arg("add")
            .arg(&source)
            .args(&args)
            .arg("--no-input")
            .output()
            .unwrap();
        let text = combined(&output);
        assert_eq!(output.status.code(), Some(2), "args={args:?}\n{text}");
        assert!(text.contains(fragment), "args={args:?}\n{text}");
    }
}

#[test]
fn test_add_cmd_refuses_dep_flag_loudly() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&[
        "add", "--cmd", "echo {x}", "--name", "e", "--dep", "requests",
    ]);
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(
        text.contains("don't take package dependencies") || text.contains("--dep can't apply here"),
        "{text}"
    );
    assert!(sandbox.store().resolve("e").is_err());
}

#[test]
fn test_add_python_still_honors_both_flags() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("j.py", "print(1)\n");
    let mut command = sandbox.command();
    let output = command
        .arg("add")
        .arg(source)
        .args([
            "--dep", "requests", "--python", ">=3.11", "--no-input",
        ])
        .env("SKIT_FORM", "plain")
        .output()
        .unwrap();
    assert_success(&output);
    let store = sandbox.store();
    let entry = store.resolve("j").unwrap();
    let copy = fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap();
    assert!(copy.contains("\"requests\""), "{copy}");
    assert!(copy.contains("requires-python = \">=3.11\""), "{copy}");
}

#[test]
fn test_add_stdin_honors_explicit_dep_and_python_flags() {
    let sandbox = Sandbox::new();
    let mut command = sandbox.command();
    let output = command
        .args([
            "add", "-", "--name", "clip", "--dep", "requests>=2,<3", "--python", ">=3.11",
        ])
        .write_stdin("print(\"hi\")\n")
        .output()
        .unwrap();
    assert_success(&output);
    let store = sandbox.store();
    let entry = store.resolve("clip").unwrap();
    let copy = fs::read_to_string(store.payload_path(&entry).unwrap()).unwrap();
    assert!(copy.contains("\"requests>=2,<3\""), "{copy}");
    assert!(copy.contains("requires-python = \">=3.11\""), "{copy}");
}

#[test]
fn test_add_stdin_refuses_ref_loudly() {
    let sandbox = Sandbox::new();
    let mut command = sandbox.command();
    let output = command
        .args(["add", "-", "--name", "clip", "--ref"])
        .write_stdin("print(\"hi\")\n")
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{text}");
    assert!(text.contains("existing file") || text.contains("--ref can't apply here"), "{text}");
    assert!(sandbox.store().resolve("clip").is_err());
}
