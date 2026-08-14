use std::{fs, path::{Path, PathBuf}, process::Output};

use assert_cmd::Command;
use skit_application::EntryRepository;
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
            data: TempDir::new().expect("data"),
            state: TempDir::new().expect("state"),
            config: TempDir::new().expect("config"),
            home: TempDir::new().expect("home"),
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
        self.command().args(args).output().expect("run skit")
    }

    fn source(&self, name: &str, body: &str) -> PathBuf {
        let path = self.home.path().join(name);
        fs::write(&path, body).expect("write source");
        path
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn meta(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join("scripts").join(slug).join("meta.toml"))
            .expect("read meta")
    }

    fn payload(&self, slug: &str, name: &str) -> String {
        fs::read_to_string(self.data.path().join("scripts").join(slug).join(name))
            .expect("read payload")
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
    combined(output).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_code(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", combined(output));
}

fn assert_success(output: &Output) {
    assert_code(output, 0);
}

fn add_python(sandbox: &Sandbox, source: &Path, name: &str) -> Output {
    sandbox.run(&[
        "add",
        source.to_str().expect("utf8 source"),
        "--name",
        name,
        "--no-input",
    ])
}

#[test]
fn test_version_flag_prints_and_exits() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["--version"]);
    assert_success(&output);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains(env!("CARGO_PKG_VERSION")),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_add_python_copy() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--name", "hi"]);
    assert_success(&output);
    assert!(sandbox.meta("hi").contains("mode = \"copy\""));
    assert_eq!(sandbox.payload("hi", "script.py"), "print(1)\n");
}

#[test]
fn test_add_python_reference_skips_onboarding() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "CITY = \"x\"\nprint(CITY)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "ref",
        "--ref",
    ]);
    assert_success(&output);
    let meta = sandbox.meta("ref");
    assert!(meta.contains("mode = \"reference\""), "{meta}");
    assert!(meta.contains(&source.to_string_lossy().to_string()), "{meta}");
}

#[test]
fn test_add_rejects_non_py() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("notes.txt", "data");
    let output = sandbox.run(&["add", source.to_str().unwrap()]);
    assert_code(&output, 2);
    assert!(
        flat(&output).contains("pass --kind <language> for an extensionless script"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_add_needs_path() {
    let output = Sandbox::new().run(&["add"]);
    assert_code(&output, 2);
}

#[test]
fn test_add_exe_needs_path() {
    let output = Sandbox::new().run(&["add", "--exe"]);
    assert_code(&output, 2);
}

#[test]
fn test_add_exe() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("tool", "#!/bin/sh\necho hi\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--exe",
        "--name",
        "tool",
    ]);
    assert_success(&output);
    assert!(sandbox.meta("tool").contains("kind = \"exe\""));
}

#[test]
fn test_add_exe_no_input_never_asks() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("archiver", "#!/bin/sh\necho hi\n");
    let output = sandbox.run(&["add", source.to_str().unwrap(), "--exe", "--no-input"]);
    assert_success(&output);
    assert!(sandbox.meta("archiver").contains("kind = \"exe\""));
}

#[test]
fn test_add_exe_missing_path_errors_before_any_ask() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("ghost.bin");
    let output = sandbox.run(&["add", missing.to_str().unwrap(), "--exe"]);
    assert_code(&output, 1);
    assert!(combined(&output).contains("File not found"), "{}", combined(&output));
}

#[test]
fn test_add_cmd_needs_name() {
    let output = Sandbox::new().run(&["add", "--cmd", "echo hi"]);
    assert_code(&output, 2);
}

#[test]
fn test_add_cmd_with_params() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["add", "--cmd", "echo {msg}", "--name", "e"]);
    assert_success(&output);
    let params = sandbox.run(&["params", "e"]);
    assert_success(&params);
    assert!(combined(&params).contains("msg"), "{}", combined(&params));
}

#[test]
fn test_add_with_explicit_deps_records() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "import requests\nprint(requests)\n");
    let output = sandbox.run(&[
        "add",
        source.to_str().unwrap(),
        "--name",
        "r",
        "--dep",
        "requests",
        "--dep",
        "rich",
        "--no-input",
    ]);
    assert_success(&output);
    let deps = sandbox.run(&["deps", "r", "--json"]);
    assert_success(&deps);
    let json: serde_json::Value = serde_json::from_slice(&deps.stdout).expect("deps json");
    assert_eq!(json["dependencies"], serde_json::json!(["requests", "rich"]));
}

#[test]
fn test_add_name_conflict_errors() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "dup"));
    let output = add_python(&sandbox, &source, "dup");
    assert_code(&output, 1);
}

#[test]
fn test_add_missing_path_clean_error_not_traceback() {
    let sandbox = Sandbox::new();
    let missing = sandbox.home.path().join("typo/path.py");
    let output = sandbox.run(&["add", missing.to_str().unwrap()]);
    assert_code(&output, 1);
    let shown = combined(&output);
    assert!(shown.contains("File not found"), "{shown}");
    assert!(!shown.contains("panicked at"), "{shown}");
    assert!(!shown.contains("stack backtrace"), "{shown}");
}

#[test]
fn test_add_directory_path_clean_error_not_traceback() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("adir.py");
    fs::create_dir(&directory).expect("directory");
    let output = sandbox.run(&["add", directory.to_str().unwrap()]);
    assert_code(&output, 1);
    let shown = combined(&output);
    assert!(shown.contains("Not a file"), "{shown}");
    assert!(!shown.contains("--exe"), "{shown}");
    assert!(!shown.contains("panicked at"), "{shown}");
}

#[test]
fn test_add_unknown_directory_suggests_exe_and_exits_usage() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("plainbundle");
    fs::create_dir(&directory).expect("directory");
    let output = sandbox.run(&["add", directory.to_str().unwrap()]);
    assert_code(&output, 2);
    let shown = combined(&output);
    assert!(shown.contains("is a directory"), "{shown}");
    assert!(shown.contains("--exe"), "{shown}");
    assert!(!shown.contains("Not a file"), "{shown}");
}

#[test]
fn test_add_unknown_directory_with_exe_is_accepted() {
    let sandbox = Sandbox::new();
    let directory = sandbox.home.path().join("plainbundle2");
    fs::create_dir(&directory).expect("directory");
    let output = sandbox.run(&[
        "add",
        directory.to_str().unwrap(),
        "--exe",
        "--name",
        "bundled",
    ]);
    assert_success(&output);
    assert!(sandbox.meta("bundled").contains("kind = \"exe\""));
}

#[test]
fn test_add_onboards_params_non_interactive_skips() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "CITY = \"Taipei\"\nprint(CITY)\n");
    let output = add_python(&sandbox, &source, "j");
    assert_success(&output);
    assert!(
        !sandbox.payload("j", "script.py").contains("[tool.skit]"),
        "--no-input must not silently select onboarding candidates"
    );
}

#[test]
fn test_list_empty() {
    assert_success(&Sandbox::new().run(&["list"]));
}

#[test]
fn test_list_table() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    let output = sandbox.run(&["list"]);
    assert_success(&output);
    assert!(combined(&output).contains("a"), "{}", combined(&output));
}

#[test]
fn test_list_json() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    let output = sandbox.run(&["list", "--json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("list json");
    let rows = json.as_array().expect("list array");
    assert!(rows.iter().any(|row| row["slug"] == "a"), "{json}");
}

#[test]
fn test_list_table_marks_missing_target() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "gone"));
    fs::remove_file(sandbox.data.path().join("scripts/gone/script.py")).expect("remove payload");
    let output = sandbox.run(&["list"]);
    assert_success(&output);
    assert!(combined(&output).contains("missing"), "{}", combined(&output));
}

#[test]
fn test_list_table_does_not_mark_healthy_or_command_entries() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "healthy"));
    assert_success(&sandbox.run(&["add", "--cmd", "echo hi", "--name", "cmdok"]));
    let output = sandbox.run(&["list"]);
    assert_success(&output);
    assert!(!combined(&output).contains("missing"), "{}", combined(&output));
}

#[test]
fn test_list_json_missing_field() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "gone"));
    fs::remove_file(sandbox.data.path().join("scripts/gone/script.py")).expect("remove payload");
    let output = sandbox.run(&["list", "--json"]);
    assert_success(&output);
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("list json");
    let row = json
        .as_array()
        .expect("list array")
        .iter()
        .find(|row| row["slug"] == "gone")
        .expect("gone row");
    assert_eq!(row["missing"], true);
}

#[test]
fn test_list_and_show_human_faces_use_translated_kind_labels() {
    let sandbox = Sandbox::new();
    let python = sandbox.source("pyjob.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &python, "pyjob"));
    let prompt = sandbox.source("p.prompt.md", "Do {{a}}\n");
    assert_success(&sandbox.run(&[
        "add",
        prompt.to_str().unwrap(),
        "--prompt",
        "--name",
        "pr",
        "--no-input",
    ]));
    let exe = sandbox.source("tool", "#!/bin/sh\necho hi\n");
    assert_success(&sandbox.run(&[
        "add",
        exe.to_str().unwrap(),
        "--exe",
        "--name",
        "prog",
        "--no-input",
    ]));

    let listed = sandbox.run(&["list"]);
    assert_success(&listed);
    let human = combined(&listed);
    for label in ["Python", "Prompt", "Program"] {
        assert!(human.contains(label), "missing {label:?} in {human}");
    }
    assert!(!human.contains(" python "), "raw python id leaked into human list: {human}");
    for (selector, label) in [("pyjob", "Python ·"), ("pr", "Prompt ·"), ("prog", "Program ·")] {
        let shown = sandbox.run(&["show", selector]);
        assert_success(&shown);
        assert!(combined(&shown).contains(label), "{}", combined(&shown));
    }

    let machine = sandbox.run(&["list", "--json"]);
    assert_success(&machine);
    let rows: serde_json::Value = serde_json::from_slice(&machine.stdout).expect("json");
    let rows = rows.as_array().expect("array");
    let kinds = rows
        .iter()
        .map(|row| (row["name"].as_str().unwrap_or_default(), row["kind"].as_str().unwrap_or_default()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(kinds.get("pyjob"), Some(&"python"));
    assert_eq!(kinds.get("pr"), Some(&"prompt"));
    assert_eq!(kinds.get("prog"), Some(&"exe"));
}

#[test]
fn test_list_table_name_column_escapes_markup() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&[
        "add",
        "--cmd",
        "echo hi",
        "--name",
        "[blue]hi[/blue]",
    ]));
    let output = sandbox.run(&["list"]);
    assert_success(&output);
    assert!(combined(&output).contains("[blue]hi[/blue]"), "{}", combined(&output));
}

#[test]
fn test_remove_not_found() {
    let output = Sandbox::new().run(&["remove", "ghost", "--yes"]);
    assert_code(&output, 1);
}

#[test]
fn test_remove_with_yes() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    let output = sandbox.run(&["remove", "a", "--yes"]);
    assert_success(&output);
    assert!(sandbox.store().resolve("a").is_err());
    assert!(!sandbox.data.path().join("scripts/a").exists());
}

#[test]
fn test_params_not_found() {
    let output = Sandbox::new().run(&["params", "ghost"]);
    assert_code(&output, 1);
}

#[test]
fn test_params_empty() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    assert_success(&sandbox.run(&["params", "a"]));
}

#[test]
fn test_params_command_entry() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&["add", "--cmd", "echo {msg}", "--name", "e"]));
    let output = sandbox.run(&["params", "e"]);
    assert_success(&output);
    assert!(combined(&output).contains("msg"), "{}", combined(&output));
}

#[test]
fn test_params_command_no_placeholders() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&["add", "--cmd", "echo hi", "--name", "e"]));
    assert_success(&sandbox.run(&["params", "e"]));
}

#[test]
fn test_deps_view() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    assert_success(&sandbox.run(&["deps", "a"]));
}

#[test]
fn test_deps_not_found() {
    let output = Sandbox::new().run(&["deps", "ghost"]);
    assert_code(&output, 1);
}

#[test]
fn test_deps_not_python() {
    let sandbox = Sandbox::new();
    assert_success(&sandbox.run(&["add", "--cmd", "echo hi", "--name", "e"]));
    let output = sandbox.run(&["deps", "e", "--dep", "requests"]);
    assert_code(&output, 2);
}

#[test]
fn test_deps_set() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    let output = sandbox.run(&[
        "deps", "a", "--dep", "requests", "--dep", "rich", "--python", ">=3.11",
    ]);
    assert_success(&output);
    let view = sandbox.run(&["deps", "a", "--json"]);
    assert_success(&view);
    let json: serde_json::Value = serde_json::from_slice(&view.stdout).expect("deps json");
    assert_eq!(json["dependencies"], serde_json::json!(["requests", "rich"]));
    assert_eq!(json["requires_python"], ">=3.11");
}

#[test]
fn test_deps_view_with_requires_python() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    assert_success(&sandbox.run(&[
        "deps", "a", "--dep", "requests", "--python", ">=3.12",
    ]));
    let output = sandbox.run(&["deps", "a"]);
    assert_success(&output);
    assert!(combined(&output).contains("3.12"), "{}", combined(&output));
}

#[test]
fn test_deps_command_strips_a_whitespace_only_python_constraint() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("job.py", "print(1)\n");
    assert_success(&add_python(&sandbox, &source, "a"));
    let output = sandbox.run(&["deps", "a", "--python", "   "]);
    assert_success(&output);
    let view = sandbox.run(&["deps", "a", "--json"]);
    assert_success(&view);
    let json: serde_json::Value = serde_json::from_slice(&view.stdout).expect("deps json");
    assert_eq!(json["requires_python"], "");
}
