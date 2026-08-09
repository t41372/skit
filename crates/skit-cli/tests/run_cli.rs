use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

fn command_library(template: &str, parameters: &str) -> (TempDir, TempDir) {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let dir = data.path().join("scripts").join("demo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("meta.toml"),
        format!(
            r#"
schema = 1
name = "Demo"
kind = "command"
mode = "copy"
source = ""
source_hash = ""
added_at = "2026-08-07T00:00:00Z"
id = "0123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
template = {template:?}
params = ["name"]
{parameters}
"#
        ),
    )
    .unwrap();
    (data, state)
}

fn skit(data: &TempDir, state: &TempDir) -> assert_cmd::Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_LANG", "en");
    command
}

#[test]
fn dry_run_uses_set_values_and_does_not_write_state() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );

    skit(&data, &state)
        .args(["run", "demo", "--set", "name=Ada Lovelace", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Ada Lovelace"));

    assert!(!state.path().join("values/demo.toml").exists());
}

#[test]
fn a_missing_required_value_is_a_skit_input_error_before_spawn() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );

    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .code(125)
        .stderr(predicate::str::contains("required"));
}

#[test]
fn preset_then_set_uses_the_same_precedence_as_the_form_state_service() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );
    fs::create_dir_all(state.path().join("values")).unwrap();
    fs::write(
        state.path().join("values/demo.toml"),
        r#"
[values]
name = "last"

[presets.work]
name = "preset"
"#,
    )
    .unwrap();

    skit(&data, &state)
        .args([
            "run",
            "demo",
            "--preset",
            "work",
            "--set",
            "name=this-run",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("this-run"))
        .stdout(predicate::str::contains("preset").not());
}

#[test]
fn dry_run_masks_secret_placeholder_values() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
secret = true
"#,
    );

    skit(&data, &state)
        .args(["run", "demo", "--set", "name=super-secret", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("super-secret").not())
        .stdout(predicate::str::contains("•••"));
}

#[test]
fn child_exit_status_is_the_skit_process_status() {
    let template = if cfg!(windows) { "exit /b 7" } else { "exit 7" };
    let (data, state) = command_library(template, "");
    fs::write(
        data.path().join("scripts/demo/meta.toml"),
        format!(
            r#"
schema = 1
name = "Demo"
kind = "command"
mode = "copy"
source = ""
source_hash = ""
added_at = "2026-08-07T00:00:00Z"
id = "0123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
template = {template:?}
params = []
"#
        ),
    )
    .unwrap();

    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .code(7)
        .stdout(predicate::str::contains("→ "));

    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("exit = 7"));
}

#[test]
fn malformed_set_and_unknown_preset_are_usage_errors() {
    let (data, state) = command_library("echo {name}", "");

    skit(&data, &state)
        .args(["run", "demo", "--set", "not-an-assignment", "--dry-run"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("NAME=VALUE"));

    skit(&data, &state)
        .args(["run", "demo", "--preset", "missing", "--dry-run"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("preset"));
}

#[test]
fn python_uses_an_existing_verified_private_uv_when_path_has_no_uv() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let entry = data.path().join("scripts/python-demo");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("script.py"), "print('ok')\n").unwrap();
    fs::write(
        entry.join("meta.toml"),
        r#"
schema = 1
name = "Python demo"
kind = "python"
mode = "copy"
source = "/old/tool.py"
source_hash = ""
added_at = "2026-08-08T00:00:00Z"
id = "0123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
"#,
    )
    .unwrap();
    let bin = data.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let uv = bin.join(if cfg!(windows) { "uv.exe" } else { "uv" });
    fs::write(&uv, "private uv").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&uv).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&uv, permissions).unwrap();
    }

    skit(&data, &state)
        .env("PATH", "")
        .env("SKIT_CONFIG_DIR", state.path())
        .args(["run", "python-demo", "--dry-run", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains(uv.display().to_string()));
}

#[test]
fn python_dry_run_is_offline_when_no_uv_is_installed() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let entry = data.path().join("scripts/python-preview");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("script.py"), "print('ok')\n").unwrap();
    fs::write(
        entry.join("meta.toml"),
        r#"
schema = 1
name = "Python preview"
kind = "python"
mode = "copy"
source = "/old/tool.py"
source_hash = ""
added_at = "2026-08-08T00:00:00Z"
id = "1123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
"#,
    )
    .unwrap();

    skit(&data, &state)
        .env("PATH", "")
        .env("SKIT_CONFIG_DIR", state.path())
        .args(["run", "python-preview", "--dry-run", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("uv run --no-project --script "));

    assert!(!data.path().join("bin").exists());
}

#[test]
fn prompt_dry_run_is_offline_complete_and_masked() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let entry = data.path().join("scripts/prompt-preview");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("prompt.md"), "Token {{API_TOKEN}}\n").unwrap();
    fs::write(
        entry.join("meta.toml"),
        r#"
schema = 1
name = "Prompt preview"
kind = "prompt"
mode = "copy"
source = "/old/review.prompt.md"
source_hash = ""
added_at = "2026-08-08T00:00:00Z"
id = "2123456789abcdef0123456789abcdef"
workdir = "invoke"
description = ""
runner = "offline"
params = ["API_TOKEN"]
"#,
    )
    .unwrap();
    fs::write(
        config.path().join("config.toml"),
        concat!(
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"offline\"\n",
            "argv = [\"missing-agent\", \"--prompt\", \"{{prompt}}\", \"--\", \"literal\"]\n",
        ),
    )
    .unwrap();

    let output = skit(&data, &state)
        .env("PATH", "")
        .env("SKIT_CONFIG_DIR", config.path())
        .args([
            "run",
            "prompt-preview",
            "--set",
            "API_TOKEN=actual-secret",
            "--dry-run",
            "--no-input",
            "--",
            "--model",
            "opus",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output).unwrap();
    assert!(output.contains("missing-agent"), "{output}");
    assert!(output.contains("Token •••"), "{output}");
    assert!(!output.contains("actual-secret"), "{output}");
    assert!(!output.contains("<prompt>"), "{output}");
    assert!(output.contains("--model opus -- literal"), "{output}");
}

#[test]
fn dry_run_does_not_materialize_javascript_dependencies() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let entry = data.path().join("scripts/js-demo");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("script.js"), "console.log('ok')\n").unwrap();
    let runtime = data.path().join("node");
    fs::write(&runtime, "runtime").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&runtime).unwrap().permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&runtime, permissions).unwrap();
    }
    fs::write(
        entry.join("meta.toml"),
        format!(
            r#"name = "JavaScript demo"
kind = "js"
mode = "copy"
source = "/deleted/demo.js"
workdir = "invoke"
interpreter = {:?}
dependencies = ["left-pad"]
"#,
            runtime.display().to_string()
        ),
    )
    .unwrap();

    skit(&data, &state)
        .env("PATH", "")
        .args(["run", "js-demo", "--dry-run", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains(runtime.display().to_string()));

    assert!(!entry.join("package.json").exists());
    assert!(!entry.join("node_modules").exists());
    assert!(!entry.join(".skit-deps").exists());
}

#[test]
fn dry_run_applies_explicit_preset_and_forget_state_operations_only() {
    let (data, state) = command_library(
        "echo {name}",
        r#"
[[parameters]]
name = "name"
delivery = "placeholder"
type = "str"
required = true
"#,
    );
    fs::create_dir_all(state.path().join("values")).unwrap();
    fs::write(
        state.path().join("values/demo.toml"),
        "extra_args = [\"old\"]\n",
    )
    .unwrap();

    skit(&data, &state)
        .args([
            "run",
            "demo",
            "--set",
            "name=Ada",
            "--save-preset",
            "review",
            "--forget-args",
            "--dry-run",
        ])
        .assert()
        .success();

    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("[presets.review]"));
    assert!(saved.contains("name = \"Ada\""));
    assert!(!saved.contains("extra_args"));
    assert!(!saved.contains("[values]"));
}

#[cfg(unix)]
#[test]
fn raw_uses_only_the_explicit_tail_and_keeps_form_memory() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let entry = data.path().join("scripts/raw-demo");
    fs::create_dir_all(&entry).unwrap();
    fs::write(entry.join("script.sh"), "printf '%s\\n' \"$@\"\n").unwrap();
    fs::write(
        entry.join("meta.toml"),
        r#"name = "Raw demo"
kind = "shell"
mode = "copy"
source = "/deleted/raw.sh"
workdir = "invoke"
"#,
    )
    .unwrap();
    fs::create_dir_all(state.path().join("values")).unwrap();
    let state_path = state.path().join("values/raw-demo.toml");
    fs::write(
        &state_path,
        "extra_args = [\"remembered\"]\n[values]\nname = \"kept\"\n",
    )
    .unwrap();

    skit(&data, &state)
        .args(["run", "raw-demo", "--raw", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("remembered").not());
    let saved = fs::read_to_string(&state_path).unwrap();
    assert!(saved.contains("remembered"));
    assert!(saved.contains("name = \"kept\""));

    skit(&data, &state)
        .args([
            "run",
            "raw-demo",
            "--forget-args",
            "--no-input",
            "--",
            "explicit",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("explicit"));
    let saved = fs::read_to_string(&state_path).unwrap();
    assert!(saved.contains("explicit"));
    assert!(!saved.contains("remembered"));
}
