//! Consolidated version 0.4 store-add contracts at the real CLI composition root.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

use skit_application::EntryRepository as _;
use skit_domain::{Entry, EntrySettings, StorageMode};
use skit_store::{FileStore, content_hash};
use tempfile::TempDir;

#[derive(Debug)]
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

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env_remove("EDITOR")
            .env_remove("VISUAL");
        command
    }

    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.scratch.path().join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    fn store(&self) -> FileStore {
        FileStore::new(self.data.path())
    }

    fn entry(&self, selector: &str) -> Entry {
        self.store().resolve(selector).unwrap()
    }

    fn show_json(&self, selector: &str) -> serde_json::Value {
        let output = self
            .command()
            .args(["show", selector, "--json"])
            .output()
            .unwrap();
        assert_success(&output, selector);
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn entry_dir(&self, entry: &Entry) -> PathBuf {
        self.data.path().join("scripts").join(entry.slug.as_str())
    }

    fn root_snapshot(&self) -> Vec<TreeItem> {
        let mut output = tree_snapshot(self.data.path(), "data");
        output.extend(tree_snapshot(self.state.path(), "state"));
        output.extend(tree_snapshot(self.config.path(), "config"));
        output
    }

    fn assert_state_and_config_empty(&self) {
        assert!(tree_snapshot(self.state.path(), "state").is_empty());
        assert!(tree_snapshot(self.config.path(), "config").is_empty());
    }
}

#[derive(Debug, Eq, PartialEq)]
struct TreeItem {
    path: PathBuf,
    kind: &'static str,
    bytes: Vec<u8>,
}

fn tree_snapshot(root: &Path, prefix: &str) -> Vec<TreeItem> {
    fn visit(root: &Path, path: &Path, prefix: &str, output: &mut Vec<TreeItem>) {
        let mut entries = fs::read_dir(path)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            let relative = Path::new(prefix).join(path.strip_prefix(root).unwrap());
            let (kind, bytes) = if metadata.is_dir() {
                ("directory", Vec::new())
            } else if metadata.file_type().is_symlink() {
                (
                    "symlink",
                    fs::read_link(&path)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec(),
                )
            } else {
                ("file", fs::read(&path).unwrap())
            };
            output.push(TreeItem {
                path: relative,
                kind,
                bytes,
            });
            if metadata.is_dir() {
                visit(root, &path, prefix, output);
            }
        }
    }

    let mut output = Vec::new();
    visit(root, root, prefix, &mut output);
    output
}

fn assert_success(output: &Output, receipt: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "stdout={stdout:?}\nstderr={stderr:?}"
    );
    assert!(stderr.is_empty(), "{stderr:?}");
    assert!(stdout.contains(receipt), "{stdout:?}");
}

fn assert_failure(output: &Output, code: i32, message: &str) {
    assert_eq!(output.status.code(), Some(code), "{output:?}");
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(message), "{stderr:?}");
}

fn entry_filenames(sandbox: &Sandbox, entry: &Entry) -> Vec<String> {
    let mut names = fs::read_dir(sandbox.entry_dir(entry))
        .unwrap()
        .map(|item| item.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

#[test]
fn test_add_copy_preserves_original_verbatim() {
    let sandbox = Sandbox::new();
    let source_bytes = concat!(
        "\"\"\"打招呼腳本。\n\n多行 docstring。\"\"\"\n",
        "NAME = \"world\"\n",
        "print(f\"hi {NAME}\")\n",
    )
    .as_bytes();
    let source = sandbox.source("hello.py", source_bytes);

    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "hello");
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    let entry = sandbox.entry("hello");
    let stored = sandbox.entry_dir(&entry).join("script.py");
    assert_eq!(entry.meta.kind.as_str(), "python");
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    assert_eq!(entry.meta.description, "打招呼腳本。");
    assert_eq!(
        entry.meta.source,
        source.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(entry.meta.source_hash, content_hash(source_bytes));
    assert_eq!(fs::read(stored).unwrap(), source_bytes);
    assert_eq!(
        entry_filenames(&sandbox, &entry),
        ["meta.toml", "script.py"]
    );
    let shown = sandbox.show_json("hello");
    assert_eq!(shown["kind"], "python");
    assert_eq!(shown["mode"], "copy");
    assert_eq!(shown["description"], "打招呼腳本。");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_add_command_entry() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "--cmd", "echo {msg}", "--name", "回聲", "--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "回聲");
    let entry = sandbox.entry("回聲");
    let settings = EntrySettings::from_meta(&entry.meta);
    assert_eq!(entry.meta.kind.as_str(), "command");
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.workdir, "invoke");
    assert_eq!(settings.template, "echo {msg}");
    assert_eq!(settings.params, ["msg"]);
    assert_eq!(entry_filenames(&sandbox, &entry), ["meta.toml"]);
    let shown = sandbox.show_json("回聲");
    assert_eq!(shown["template"], "echo {msg}");
    assert_eq!(shown["workdir"], "invoke");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_command_requires_nonempty_template() {
    let sandbox = Sandbox::new();
    let before = sandbox.root_snapshot();
    let output = sandbox
        .command()
        .args(["add", "--cmd", "   ", "--name", "空", "--no-input"])
        .output()
        .unwrap();

    assert_failure(&output, 2, "a command template cannot be empty");
    assert_eq!(sandbox.root_snapshot(), before);
}

#[test]
fn test_syntax_error_script_still_addable() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("bad.py", b"def broken(:\n");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "bad");
    let entry = sandbox.entry("bad");
    assert!(entry.meta.description.is_empty());
    assert_eq!(
        fs::read(sandbox.entry_dir(&entry).join("script.py")).unwrap(),
        b"def broken(:\n"
    );
    assert_eq!(sandbox.show_json("bad")["description"], "");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_add_python_missing_file_raises() {
    let sandbox = Sandbox::new();
    let before = sandbox.root_snapshot();
    let missing = sandbox.scratch.path().join("ghost.py");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&missing)
        .args(["--no-input"])
        .output()
        .unwrap();

    assert_failure(&output, 1, "File not found");
    assert_eq!(sandbox.root_snapshot(), before);
}

#[test]
fn test_add_exe_roundtrip() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("mytool", b"");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args([
            "--exe",
            "--description",
            "a tool",
            "--name",
            "mytool",
            "--no-input",
        ])
        .output()
        .unwrap();

    assert_success(&output, "mytool");
    let entry = sandbox.entry("mytool");
    assert_eq!(entry.meta.kind.as_str(), "exe");
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.description, "a tool");
    assert_eq!(
        entry.meta.source,
        source.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(entry_filenames(&sandbox, &entry), ["meta.toml"]);
    let shown = sandbox.show_json("mytool");
    assert_eq!(shown["kind"], "exe");
    assert_eq!(shown["mode"], "reference");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_add_exe_missing_file_raises() {
    let sandbox = Sandbox::new();
    let before = sandbox.root_snapshot();
    let missing = sandbox.scratch.path().join("no_such_tool");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&missing)
        .args(["--exe", "--no-input"])
        .output()
        .unwrap();

    assert_failure(&output, 1, "File not found");
    assert_eq!(sandbox.root_snapshot(), before);
}

#[test]
fn test_update_dependencies_copy_mode() {
    let sandbox = Sandbox::new();
    let original = b"\"\"\"Sample.\"\"\"\nprint(1)\n";
    let source = sandbox.source("sample.py", original);
    let add = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "sample", "--no-input"])
        .output()
        .unwrap();
    assert_success(&add, "sample");
    let state_before = tree_snapshot(sandbox.state.path(), "state");
    let config_before = tree_snapshot(sandbox.config.path(), "config");

    let output = sandbox
        .command()
        .args(["deps", "sample", "--dep", "httpx", "--python", ">=3.11"])
        .output()
        .unwrap();

    assert_success(&output, "sample");
    assert_eq!(fs::read(&source).unwrap(), original);
    let entry = sandbox.entry("sample");
    let stored = fs::read(sandbox.entry_dir(&entry).join("script.py")).unwrap();
    let text = String::from_utf8(stored.clone()).unwrap();
    assert!(text.contains("httpx"), "{text}");
    assert!(text.contains("requires-python = \">=3.11\""), "{text}");
    assert_eq!(entry.meta.source_hash, content_hash(&stored));
    let shown = sandbox.show_json("sample");
    assert_eq!(shown["dependencies"], serde_json::json!(["httpx"]));
    assert_eq!(shown["requires_python"], ">=3.11");
    assert_eq!(tree_snapshot(sandbox.state.path(), "state"), state_before);
    assert_eq!(
        tree_snapshot(sandbox.config.path(), "config"),
        config_before
    );
}

#[test]
fn test_add_script_copy_is_byte_identical_and_records_hash() {
    let sandbox = Sandbox::new();
    let source_bytes = b"#!/bin/bash\n# Deploy it\necho hi\n";
    let source = sandbox.source("deploy.sh", source_bytes);
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "deploy");
    let entry = sandbox.entry("deploy");
    let stored = sandbox.entry_dir(&entry).join("script.sh");
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert_eq!(entry.meta.mode, StorageMode::Copy);
    assert_eq!(entry.meta.workdir, "invoke");
    assert_eq!(entry.meta.description, "Deploy it");
    assert_eq!(entry.meta.source_hash, content_hash(source_bytes));
    assert_eq!(fs::read(stored).unwrap(), source_bytes);
    assert_eq!(fs::read(source).unwrap(), source_bytes);
    assert_eq!(
        entry_filenames(&sandbox, &entry),
        ["meta.toml", "script.sh"]
    );
    let shown = sandbox.show_json("deploy");
    assert_eq!(shown["kind"], "shell");
    assert_eq!(shown["workdir"], "invoke");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_add_script_reference_points_to_origin() {
    let sandbox = Sandbox::new();
    let source_bytes = b"#!/bin/bash\n# Deploy it\necho hi\n";
    let source = sandbox.source("deploy.sh", source_bytes);
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--ref", "--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "deploy");
    let entry = sandbox.entry("deploy");
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(entry.meta.workdir, "origin");
    assert_eq!(entry.meta.source_hash, content_hash(source_bytes));
    assert_eq!(entry_filenames(&sandbox, &entry), ["meta.toml"]);
    assert_eq!(
        sandbox.store().payload_path(&entry).unwrap(),
        source.canonicalize().unwrap()
    );
    let shown = sandbox.show_json("deploy");
    assert_eq!(shown["mode"], "reference");
    assert_eq!(shown["workdir"], "origin");
    assert_eq!(fs::read(source).unwrap(), source_bytes);
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_add_script_explicit_name_and_description() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("deploy.sh", b"#!/bin/bash\n# Deploy it\necho hi\n");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--name", "ship", "--description", "custom", "--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "ship");
    let entry = sandbox.entry("ship");
    assert_eq!(entry.meta.name, "ship");
    assert_eq!(entry.meta.description, "custom");
    let shown = sandbox.show_json("ship");
    assert_eq!(shown["name"], "ship");
    assert_eq!(shown["description"], "custom");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_add_script_unknown_kind_raises() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("deploy.sh", b"#!/bin/bash\necho hi\n");
    let before = sandbox.root_snapshot();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "martian", "--no-input"])
        .output()
        .unwrap();

    assert_failure(&output, 2, "Unknown kind: martian");
    assert_eq!(sandbox.root_snapshot(), before);
    assert_eq!(fs::read(source).unwrap(), b"#!/bin/bash\necho hi\n");
}

#[test]
fn test_add_script_missing_file_raises() {
    let sandbox = Sandbox::new();
    let missing = sandbox.scratch.path().join("ghost.sh");
    let before = sandbox.root_snapshot();
    let output = sandbox
        .command()
        .arg("add")
        .arg(&missing)
        .args(["--kind", "shell", "--no-input"])
        .output()
        .unwrap();

    assert_failure(&output, 1, "File not found");
    assert_eq!(sandbox.root_snapshot(), before);
}

#[test]
fn test_add_script_lua_uses_double_dash_description() {
    let sandbox = Sandbox::new();
    let source_bytes = b"-- Resize things\nprint('x')\n";
    let source = sandbox.source("tool.lua", source_bytes);
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "tool");
    let entry = sandbox.entry("tool");
    assert_eq!(entry.meta.kind.as_str(), "lua");
    assert_eq!(entry.meta.description, "Resize things");
    assert_eq!(
        fs::read(sandbox.entry_dir(&entry).join("script.lua")).unwrap(),
        source_bytes
    );
    assert_eq!(sandbox.show_json("tool")["description"], "Resize things");
    sandbox.assert_state_and_config_empty();
}

#[test]
fn test_exe_is_always_reference_mode() {
    let sandbox = Sandbox::new();
    let source = sandbox.source("tool", b"program bytes");
    let output = sandbox
        .command()
        .arg("add")
        .arg(&source)
        .args(["--kind", "exe", "--name", "binary", "--no-input"])
        .output()
        .unwrap();

    assert_success(&output, "binary");
    let entry = sandbox.entry("binary");
    let origin = source.canonicalize().unwrap();
    assert_eq!(entry.meta.kind.as_str(), "exe");
    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(Path::new(&entry.meta.source), origin);
    assert_eq!(sandbox.store().payload_path(&entry).unwrap(), origin);
    assert_eq!(entry_filenames(&sandbox, &entry), ["meta.toml"]);
    let shown = sandbox.show_json("binary");
    assert_eq!(shown["kind"], "exe");
    assert_eq!(shown["mode"], "reference");
    assert_eq!(
        shown["source"],
        source.canonicalize().unwrap().display().to_string()
    );
    sandbox.assert_state_and_config_empty();
}
