use std::fs;

use predicates::prelude::*;
use serde_json::Value;
use skit_i18n::{Locale, text};
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
}

#[test]
fn new_refusal_messages_have_complete_chinese_rows() {
    // Each refusal is a whole typed template, so an exact catalog lookup must translate it.
    for english in [
        "standard input cannot be an executable entry",
        "--no-interpolate only applies to prompt entries",
        "cannot save a preset because the entry has no form fields",
        "invalid Boolean value {}; use true or false",
        "{} is not valid UTF-8",
        "invalid PEP 508 requirement {}: {}",
    ] {
        assert_ne!(text(Locale::ZhCn, english), english);
        assert_ne!(text(Locale::ZhTw, english), english);
    }
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

    fn add_command(&self, name: &str, template: &str) {
        self.command()
            .args(["add", "--cmd", template, "--name", name])
            .assert()
            .success();
    }

    fn meta_path(&self, slug: &str) -> std::path::PathBuf {
        self.data
            .path()
            .join("scripts")
            .join(slug)
            .join("meta.toml")
    }
}

#[test]
fn command_template_updates_reconcile_the_placeholder_schema() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Template", "echo {old}");

    sandbox
        .command()
        .args(["params", "template", "--template", "echo {new}"])
        .assert()
        .success();

    let output = sandbox
        .command()
        .args(["params", "template", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["placeholders"], serde_json::json!(["new"]));
    assert_eq!(record["parameters"][0]["name"], "new");
    assert!(
        record["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|field| field["name"] != "old")
    );
}

#[test]
fn required_commands_trim_values_and_drop_empty_occurrences() {
    let sandbox = Sandbox::new();
    sandbox.add_command("Needs", "true");

    let output = sandbox
        .command()
        .args([
            "deps",
            "needs",
            "--need",
            "  printf  ",
            "--need",
            "",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["needs"], serde_json::json!(["printf"]));
}

#[test]
fn standard_input_cannot_create_an_executable_reference() {
    for kind_args in [vec!["--exe"], vec!["--kind", "exe"]] {
        let sandbox = Sandbox::new();
        let mut args = vec!["add", "-", "--name", "Pipe executable"];
        args.extend(kind_args);
        sandbox
            .command()
            .args(args)
            .write_stdin("not an executable path\n")
            .assert()
            .code(2)
            .stderr(predicate::str::contains("standard input"));
        assert!(!sandbox.data.path().join("scripts").exists());
    }
}

#[test]
fn empty_dependency_is_explicit_and_no_interpolate_is_prompt_only() {
    let sandbox = Sandbox::new();
    let javascript = sandbox.data.path().join("scan.js");
    fs::write(&javascript, "import 'chalk';\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            javascript.to_str().unwrap(),
            "--name",
            "No packages",
            "--dep",
            "",
        ])
        .assert()
        .success();
    let metadata = fs::read_to_string(sandbox.meta_path("no-packages")).unwrap();
    assert!(!metadata.contains("chalk"));

    let shell = sandbox.data.path().join("tool.sh");
    fs::write(&shell, "echo ok\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            shell.to_str().unwrap(),
            "--kind",
            "shell",
            "--name",
            "Bad dependency lane",
            "--dep",
            "",
        ])
        .assert()
        .code(2);

    let python = sandbox.data.path().join("tool.py");
    fs::write(&python, "print('ok')\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            python.to_str().unwrap(),
            "--name",
            "Bad interpolation lane",
            "--no-interpolate",
        ])
        .assert()
        .code(2);
    assert!(
        !sandbox
            .data
            .path()
            .join("scripts/bad-interpolation-lane")
            .exists()
    );
}

#[test]
fn an_empty_prompt_runner_clears_the_pin() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["runner", "add", "agent", "agent", "{{prompt}}"])
        .assert()
        .success();
    let prompt = sandbox.data.path().join("clear.prompt.md");
    fs::write(&prompt, "Review this.\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            prompt.to_str().unwrap(),
            "--prompt",
            "--name",
            "Clear runner",
            "--runner",
            "agent",
        ])
        .assert()
        .success();

    sandbox
        .command()
        .args(["params", "clear-runner", "--runner", ""])
        .assert()
        .success();
    sandbox
        .command()
        .args(["show", "clear-runner", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"runner\":null"));

    sandbox
        .command()
        .args(["params", "clear-runner", "--runner", "agent"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "clear-runner", "--runner", "   "])
        .assert()
        .success();
    sandbox
        .command()
        .args(["show", "clear-runner", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"runner\":null"));
}

#[test]
fn prompt_show_reports_source_read_and_utf8_errors() {
    let sandbox = Sandbox::new();
    let prompt = sandbox.data.path().join("strict.prompt.md");
    fs::write(&prompt, "Review this.\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            prompt.to_str().unwrap(),
            "--prompt",
            "--name",
            "Strict prompt",
        ])
        .assert()
        .success();
    let stored = sandbox.data.path().join("scripts/strict-prompt/prompt.md");

    fs::write(&stored, [0xff]).unwrap();
    sandbox
        .command()
        .args(["show", "strict-prompt", "--json"])
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("UTF-8"));
    sandbox
        .command()
        .args(["show", "strict-prompt"])
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("UTF-8"));

    fs::remove_file(stored).unwrap();
    sandbox
        .command()
        .args(["show", "strict-prompt", "--json"])
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("read"));
}

fn write_legacy_command(sandbox: &Sandbox, slug: &str, template: &str, required: bool) {
    let entry = sandbox.data.path().join("scripts").join(slug);
    fs::create_dir_all(&entry).unwrap();
    fs::write(
        entry.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = \"Legacy\"\n",
                "kind = \"command\"\n",
                "mode = \"reference\"\n",
                "source = \"\"\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-01-01T00:00:00Z\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
                "template = {:?}\n",
                "params = [\"name\"]\n",
                "[[parameters]]\n",
                "name = \"name\"\n",
                "delivery = \"placeholder\"\n",
                "required = {}\n",
            ),
            template, required
        ),
    )
    .unwrap();
}

#[test]
fn invalid_run_inputs_do_not_stamp_a_legacy_identity() {
    for args in [
        vec![
            "run",
            "legacy",
            "--preset",
            "missing",
            "--dry-run",
            "--no-input",
        ],
        vec![
            "run",
            "legacy",
            "--set",
            "missing=value",
            "--dry-run",
            "--no-input",
        ],
        vec!["run", "legacy", "--dry-run", "--no-input"],
    ] {
        let sandbox = Sandbox::new();
        write_legacy_command(&sandbox, "legacy", "echo {name}", true);
        let path = sandbox.meta_path("legacy");
        let before = fs::read(&path).unwrap();
        sandbox.command().args(args).assert().failure();
        assert_eq!(fs::read(path).unwrap(), before);
    }

    let sandbox = Sandbox::new();
    let entry = sandbox.data.path().join("scripts/legacy-prompt");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("prompt.md"), "Review this.\n").unwrap();
    fs::write(
        entry.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Legacy prompt\"\n",
            "kind = \"prompt\"\n",
            "mode = \"copy\"\n",
            "source = \"old.prompt.md\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-01-01T00:00:00Z\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
        ),
    )
    .unwrap();
    let meta = entry.join("meta.toml");
    let before = fs::read(&meta).unwrap();
    sandbox
        .command()
        .args(["run", "legacy-prompt", "--runner", "missing", "--no-input"])
        .assert()
        .code(126);
    assert_eq!(fs::read(meta).unwrap(), before);
}

#[test]
fn saving_a_preset_requires_at_least_one_form_field() {
    let sandbox = Sandbox::new();
    sandbox.add_command("No fields", "true");

    sandbox
        .command()
        .args([
            "run",
            "no-fields",
            "--save-preset",
            "empty",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .code(2);
    assert!(!sandbox.state.path().join("values/no-fields.toml").exists());

    sandbox
        .command()
        .args(["preset", "save", "no-fields", "empty"])
        .assert()
        .code(2);
    assert!(!sandbox.state.path().join("values/no-fields.toml").exists());
}

#[cfg(unix)]
#[test]
fn invalid_javascript_inputs_do_not_install_or_stamp_identity() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let entry = sandbox.data.path().join("scripts/legacy-js");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("script.js"), "console.log('ok');\n").unwrap();
    fs::write(
        entry.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Legacy JS\"\n",
            "kind = \"js\"\n",
            "mode = \"copy\"\n",
            "source = \"old.js\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-01-01T00:00:00Z\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
            "dependencies = [\"chalk\"]\n",
        ),
    )
    .unwrap();
    let tools = TempDir::new().unwrap();
    let marker = tools.path().join("installed");
    for (name, body) in [
        ("node", "#!/bin/sh\nexit 0\n"),
        ("npm", "#!/bin/sh\ntouch \"$SKIT_TEST_MARKER\"\nexit 0\n"),
    ] {
        let path = tools.path().join(name);
        fs::write(&path, body).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let meta = entry.join("meta.toml");
    let before = fs::read(&meta).unwrap();

    sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_TEST_MARKER", &marker)
        .args(["run", "legacy-js", "--set", "missing=value", "--no-input"])
        .assert()
        .code(2);
    assert!(!marker.exists());
    assert_eq!(fs::read(meta).unwrap(), before);
}

#[cfg(unix)]
#[test]
fn invalid_reference_dependencies_do_not_stamp_identity() {
    use std::os::unix::fs::PermissionsExt as _;

    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("reference.js");
    fs::write(&source, "console.log('ok');\n").unwrap();
    let entry = sandbox.data.path().join("scripts/legacy-reference");
    fs::create_dir_all(&entry).unwrap();
    fs::write(
        entry.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = \"Legacy reference\"\n",
                "kind = \"js\"\n",
                "mode = \"reference\"\n",
                "source = {:?}\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-01-01T00:00:00Z\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
                "dependencies = [\"chalk\"]\n",
            ),
            source.display().to_string()
        ),
    )
    .unwrap();
    let tools = TempDir::new().unwrap();
    let node = tools.path().join("node");
    fs::write(&node, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(node, fs::Permissions::from_mode(0o755)).unwrap();
    let meta = entry.join("meta.toml");
    let before = fs::read(&meta).unwrap();

    sandbox
        .command()
        .env("PATH", tools.path())
        .args(["run", "legacy-reference", "--no-input"])
        .assert()
        .code(125);
    assert_eq!(fs::read(meta).unwrap(), before);
}
