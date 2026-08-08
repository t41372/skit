use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }
}

#[test]
fn prompt_without_a_path_reads_a_pipe_in_no_input_mode() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/review/prompt.md")).unwrap(),
        b"Review {{subject}}.\n"
    );
}

#[test]
fn edit_and_no_input_are_an_explicit_usage_conflict() {
    Sandbox::new()
        .command()
        .args(["add", "--edit", "--name", "Draft", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("standard input"));
}

#[cfg(unix)]
#[test]
fn edit_creates_a_python_draft_and_removes_it_after_copying() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let editor = sandbox.config.path().join("editor.sh");
    fs::write(&editor, "#!/bin/sh\nprintf 'print(42)\\n' > \"$1\"\n").unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&editor, permissions).unwrap();
    fs::create_dir_all(sandbox.config.path()).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", "--edit", "--name", "Draft"])
        .assert()
        .success();

    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/draft/script.py")).unwrap(),
        b"print(42)\n"
    );
    let drafts = sandbox.data.path().join("drafts");
    assert!(
        !drafts.exists() || fs::read_dir(drafts).unwrap().next().is_none(),
        "a copied draft must not remain after success"
    );
}

#[test]
fn add_lane_selectors_are_mutually_exclusive() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("source.py");
    fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--cmd", "echo {name}"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--edit"])
        .assert()
        .code(2);
}

#[test]
fn copied_javascript_adds_static_external_imports_as_private_dependencies() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.mjs");
    fs::write(
        &source,
        "import chalk from 'chalk';\nimport local from './local.mjs';\n",
    )
    .unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Color"])
        .assert()
        .success();

    let meta = fs::read_to_string(sandbox.data.path().join("scripts/color/meta.toml")).unwrap();
    assert!(meta.contains("dependencies = [\"chalk\"]"));
    assert!(!meta.contains("./local.mjs"));
}

#[test]
fn copied_python_excludes_modules_next_to_the_original_source() {
    let sandbox = Sandbox::new();
    let source_dir = sandbox.data.path().join("source");
    fs::create_dir(&source_dir).unwrap();
    fs::write(source_dir.join("helpers.py"), "VALUE = 1\n").unwrap();
    let source = source_dir.join("tool.py");
    fs::write(&source, "import helpers\nimport requests\n").unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Local import"])
        .assert()
        .success();

    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/local-import/script.py")).unwrap();
    assert!(stored.contains("dependencies = [\"requests\"]"));
    assert!(!stored.contains("\"helpers\""));
}

#[test]
fn copied_module_extensions_resolve_as_present_in_list_and_show() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.mjs");
    fs::write(&source, "export default 1;\n").unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Module"])
        .assert()
        .success();

    sandbox
        .command()
        .args(["list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"missing\":false"));
    sandbox
        .command()
        .args(["show", "module", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"missing\":false"));
}

#[cfg(unix)]
#[test]
fn executable_sources_are_references_and_permission_inference_is_supported() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("hello");
    fs::copy("/bin/dash", &source).unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Executable"])
        .assert()
        .success();
    let meta =
        fs::read_to_string(sandbox.data.path().join("scripts/executable/meta.toml")).unwrap();
    assert!(meta.contains("kind = \"exe\""));
    assert!(meta.contains("mode = \"reference\""));
    assert!(
        !sandbox
            .data
            .path()
            .join("scripts/executable/script")
            .exists()
    );
    sandbox
        .command()
        .args(["run", "executable", "--no-input"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn add_refuses_empty_or_conflicting_lane_controls() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("source.py");
    fs::write(&source, "print(1)\n").unwrap();

    sandbox
        .command()
        .args(["add", "--cmd", "", "--name", "Empty"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["add", "--cmd", "true"])
        .assert()
        .code(2);
    for arguments in [
        vec!["add", source.to_str().unwrap(), "--prompt", "--exe"],
        vec![
            "add",
            source.to_str().unwrap(),
            "--prompt",
            "--kind",
            "python",
        ],
        vec!["add", "--cmd", "true", "--prompt"],
        vec!["add", "--cmd", "true", "--exe"],
    ] {
        sandbox.command().args(arguments).assert().code(2);
    }
}

#[test]
fn python_metadata_is_validated_before_add_and_written_to_the_stored_copy() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.py");
    fs::write(&source, "#!/usr/bin/env python3.12.1\nprint('ok')\n").unwrap();

    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "Python tool",
            "--dep",
            "requests>=2,<3",
        ])
        .assert()
        .success();
    let stored =
        fs::read_to_string(sandbox.data.path().join("scripts/python-tool/script.py")).unwrap();
    assert!(stored.contains("dependencies = [\"requests>=2,<3\"]"));
    assert!(stored.contains("requires-python = \">=3.12.1,<3.13\""));
    let meta =
        fs::read_to_string(sandbox.data.path().join("scripts/python-tool/meta.toml")).unwrap();
    assert!(!meta.contains("dependencies ="));
    assert!(!meta.contains("requires_python ="));

    for arguments in [
        vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            "Bad requirement",
            "--dep",
            "requests=>2",
        ],
        vec![
            "add",
            source.to_str().unwrap(),
            "--name",
            "Bad version",
            "--python",
            "not-a-version",
        ],
    ] {
        sandbox.command().args(arguments).assert().code(2);
    }
    assert!(!sandbox.data.path().join("scripts/bad-requirement").exists());
    assert!(!sandbox.data.path().join("scripts/bad-version").exists());

    let stored_path = sandbox.data.path().join("scripts/python-tool/script.py");
    let before = fs::read(&stored_path).unwrap();
    sandbox
        .command()
        .args(["deps", "python-tool", "--dep", "requests=>2"])
        .assert()
        .code(2);
    sandbox
        .command()
        .args(["deps", "python-tool", "--python", "not-a-version"])
        .assert()
        .code(2);
    assert_eq!(fs::read(stored_path).unwrap(), before);
}

#[test]
fn add_keeps_settings_that_do_not_belong_in_the_source() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("reference.py");
    fs::write(&source, "print('ok')\n").unwrap();

    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Reference", "--ref", "--python", ">=3.12"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["show", "reference", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"requires_python\":\">=3.12\""));

    sandbox
        .command()
        .args([
            "add",
            "--prompt",
            "--name",
            "Literal prompt",
            "--no-interpolate",
            "--no-input",
        ])
        .write_stdin("Keep {{subject}} literal.\n")
        .assert()
        .success();
    sandbox
        .command()
        .args(["show", "literal-prompt", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"interpolate\":false"));
}

#[test]
fn a_registered_shebang_pins_the_non_python_interpreter() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool");
    fs::write(&source, "#!/usr/bin/env -S deno run\nconsole.log('ok');\n").unwrap();

    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "Deno tool"])
        .assert()
        .success();
    let meta = fs::read_to_string(sandbox.data.path().join("scripts/deno-tool/meta.toml")).unwrap();
    assert!(meta.contains("kind = \"js\""));
    assert!(meta.contains("interpreter = \"deno\""));
}
