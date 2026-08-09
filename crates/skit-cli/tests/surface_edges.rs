//! Reporting, refusal, and management paths that the main lanes do not reach.

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
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .env("SKIT_LANG", "en");
        command
    }

    fn ok(&self, args: &[&str]) -> String {
        let output = self.command().args(args).assert().success();
        String::from_utf8(output.get_output().stdout.clone()).unwrap()
    }

    fn python(&self, name: &str, body: &str) -> std::path::PathBuf {
        let source = self.data.path().join(format!("{name}.py"));
        fs::write(&source, body).unwrap();
        self.command()
            .args(["add"])
            .arg(&source)
            .args(["--name", name])
            .assert()
            .success();
        source
    }
}

#[test]
fn show_reports_every_optional_axis_for_a_python_entry() {
    let sandbox = Sandbox::new();
    sandbox.python("Sample", "#!/usr/bin/env python3\nprint(1)\n");
    sandbox.ok(&[
        "deps", "sample", "--dep", "rich", "--python", ">=3.11", "--need", "git",
    ]);

    let report = sandbox.ok(&["show", "sample"]);

    assert!(report.contains("Dependencies: rich"), "{report}");
    assert!(report.contains("Python constraint: >=3.11"), "{report}");
    assert!(report.contains("Needs: git"), "{report}");
    assert!(!report.contains("⚠ missing:"), "{report}");
    assert!(!report.contains("drifted from the script"), "{report}");
}

#[test]
fn show_reports_the_template_and_prompt_axes_for_their_own_kinds() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "printf {value}", "--name", "Command"]);
    let report = sandbox.ok(&["show", "command"]);
    assert!(
        report.contains("Command template: printf {value}"),
        "{report}"
    );

    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .success();

    let unset = sandbox.ok(&["show", "review"]);
    assert!(unset.contains("Runner: (asks at run time)"), "{unset}");
    assert!(!unset.contains("Variable insertion: off"), "{unset}");

    sandbox.ok(&["params", "review", "--no-interpolate"]);
    let disabled = sandbox.ok(&["show", "review"]);
    assert!(
        disabled.contains("Variable insertion: off (the body travels as written)"),
        "{disabled}"
    );

    sandbox.ok(&["params", "review", "--runner", "claude"]);
    let pinned = sandbox.ok(&["show", "review"]);
    assert!(pinned.contains("Runner: claude"), "{pinned}");
}

#[test]
fn show_reads_a_referenced_prompt_from_its_original_path() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("brief.prompt.md");
    fs::write(&source, "Draft {{topic}}.\n").unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Brief", "--ref"])
        .assert()
        .success();

    let report = sandbox.ok(&["show", "brief", "--json"]);

    assert!(report.contains("topic"), "{report}");
}

#[test]
fn show_source_is_empty_for_an_entry_whose_target_is_gone() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("gone.py");
    fs::write(&source, "print(1)\n").unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Gone", "--ref"])
        .assert()
        .success();
    fs::remove_file(&source).unwrap();

    let report = sandbox.ok(&["show", "gone", "--json"]);

    assert!(report.contains("\"missing\":true"), "{report}");
    assert!(report.contains("\"fields\":[]"), "{report}");
}

#[test]
fn show_degrades_for_prompt_payload_damage_and_uses_the_single_file_fallback() {
    let missing = Sandbox::new();
    missing
        .command()
        .args(["add", "--prompt", "--name", "Missing", "--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .success();
    fs::remove_file(missing.data.path().join("scripts/missing/prompt.md")).unwrap();
    missing
        .command()
        .args(["show", "missing", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"missing\":true"));

    let damaged = Sandbox::new();
    damaged
        .command()
        .args(["add", "--prompt", "--name", "Damaged", "--no-input"])
        .write_stdin("Review.\n")
        .assert()
        .success();
    fs::write(
        damaged.data.path().join("scripts/damaged/prompt.md"),
        [0xff, 0xfe],
    )
    .unwrap();
    damaged
        .command()
        .args(["show", "damaged", "--json"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("isn't valid UTF-8"))
        .stderr(predicate::str::contains("offset 0"));

    let fallback = Sandbox::new();
    fallback
        .command()
        .args(["add", "--prompt", "--name", "Fallback", "--no-input"])
        .write_stdin("Review {{topic}}.\n")
        .assert()
        .success();
    fs::rename(
        fallback.data.path().join("scripts/fallback/prompt.md"),
        fallback.data.path().join("scripts/fallback/prompt.txt"),
    )
    .unwrap();
    fallback
        .command()
        .args(["show", "fallback", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"key\":\"topic\""));
}

#[test]
fn an_explicit_executable_add_overrides_kind_inference() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.py");
    fs::write(&source, "print(1)\n").unwrap();

    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Tool", "--exe"])
        .assert()
        .success();

    let report = sandbox.ok(&["show", "tool", "--json"]);
    assert!(report.contains("\"kind\":\"exe\""), "{report}");
}

#[test]
fn an_explicit_none_constraint_clears_an_inherited_python_pin() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("pinned.py");
    fs::write(&source, "#!/usr/bin/env python3.12\nprint(1)\n").unwrap();

    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Pinned", "--python", "none"])
        .assert()
        .success();

    let report = sandbox.ok(&["show", "pinned", "--json"]);
    assert!(report.contains("\"requires_python\":\"\""), "{report}");
}

#[test]
fn editing_without_a_configured_editor_is_a_typed_refusal() {
    let sandbox = Sandbox::new();
    sandbox.python("Sample", "print(1)\n");

    sandbox
        .command()
        .args(["edit", "sample", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "configure an editor before you use edit",
        ));

    sandbox
        .command()
        .args(["add", "--edit", "--name", "Draft"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "configure an editor before you use --edit",
        ));
}

#[test]
fn params_refuses_operations_that_must_stay_separate() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("shell.sh");
    fs::write(
        &source,
        "#!/usr/bin/env bash\nTARGET=\"out\"\necho \"$TARGET\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Shell"])
        .assert()
        .success();

    sandbox
        .command()
        .args(["params", "shell", "--normalize", "TARGET", "--resync"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "--normalize must be a separate params operation",
        ));
}

#[test]
fn template_changes_apply_only_to_command_entries_and_need_a_value() {
    let sandbox = Sandbox::new();
    sandbox.python("Sample", "print(1)\n");
    sandbox.ok(&["add", "--cmd", "printf ok", "--name", "Command"]);

    sandbox
        .command()
        .args(["params", "sample", "--template", "printf ok"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "--template only applies to command entries",
        ));

    sandbox
        .command()
        .args(["params", "command", "--template", ""])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "a command template cannot be empty",
        ));
}

#[test]
fn a_declared_row_without_a_source_binding_refuses_source_only_edits() {
    // v0.4 data can hold a declared row that the stored source does not manage.
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("shell.sh");
    fs::write(
        &source,
        "#!/usr/bin/env bash\nTARGET=\"out\"\necho \"$TARGET\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Shell"])
        .assert()
        .success();
    let meta = sandbox.data.path().join("scripts/shell/meta.toml");
    let mut text = fs::read_to_string(&meta).unwrap();
    text.push_str("\n[[parameters]]\nname = \"extra\"\nkind = \"none\"\ntype = \"str\"\n");
    fs::write(&meta, text).unwrap();

    for arguments in [
        vec!["params", "shell", "--prompt", "extra=Label"],
        vec!["params", "shell", "--env-source", "extra=VAR"],
        vec!["params", "shell", "--secret", "extra"],
    ] {
        sandbox
            .command()
            .args(&arguments)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(
                "parameter extra is not managed in the stored source",
            ));
    }

    sandbox
        .command()
        .args(["params", "shell", "--prompt", "missing=Label"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown parameter: missing"));
}

#[test]
fn an_unknown_parameter_binding_is_a_typed_refusal() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("tool.rb");
    fs::write(&source, "puts 1\n").unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Tool"])
        .assert()
        .success();
    sandbox.ok(&["params", "tool", "--add", "target"]);

    sandbox
        .command()
        .args(["params", "tool", "--binding", "target=guess"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown parameter binding: guess"));
}

#[test]
fn prompt_params_json_reports_the_runner_interpolation_and_placeholders() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--no-input"])
        .write_stdin("Review {{subject}} for {{team}}.\n")
        .assert()
        .success();

    let report = sandbox.ok(&["params", "review", "--json"]);

    assert!(report.contains("\"interpolate\":true"), "{report}");
    assert!(report.contains("\"runner\":null"), "{report}");
    assert!(report.contains("subject"), "{report}");
    assert!(report.contains("team"), "{report}");
}

#[test]
fn runner_rows_list_their_status_and_removal_needs_a_target() {
    let sandbox = Sandbox::new();

    let rows = sandbox.ok(&["runner", "list", "--all"]);
    assert!(rows.contains("valid"), "{rows}");
    assert!(rows.lines().count() >= 1, "{rows}");

    sandbox
        .command()
        .args(["runner", "remove", "--yes"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "runner remove needs a name or --row INDEX",
        ));

    fs::write(
        sandbox.config.path().join("config.toml"),
        "[prompt]\nrunners = [{ name = \"broken\", argv = [\"agent\"] }]\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["runner", "remove", "--row", "0", "--yes"])
        .assert()
        .success();
}

#[test]
fn agent_skill_installation_needs_one_convention() {
    let sandbox = Sandbox::new();
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    fs::create_dir(home.path().join(".codex")).unwrap();

    sandbox
        .command()
        .env("HOME", home.path())
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("more than one agent directory"));

    let project = TempDir::new().unwrap();
    fs::create_dir(project.path().join(".agents")).unwrap();
    sandbox
        .command()
        .current_dir(project.path())
        .args(["agent", "install", "--project"])
        .assert()
        .success();
    assert!(
        project
            .path()
            .join(".agents/skills/skit/SKILL.md")
            .is_file()
    );
}

#[test]
fn agent_skill_installation_needs_a_user_directory() {
    let sandbox = Sandbox::new();

    sandbox
        .command()
        .env_remove("HOME")
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "could not determine the user directory",
        ));
}

#[cfg(unix)]
#[test]
fn a_run_with_injected_values_uses_a_private_staged_source() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("greet.sh");
    fs::write(
        &source,
        "#!/usr/bin/env bash\nNAME=\"world\"\necho \"hello $NAME\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Greet"])
        .assert()
        .success();
    sandbox.ok(&["params", "greet", "--manage", "NAME"]);

    sandbox
        .command()
        .args(["run", "greet", "--no-input", "--set", "NAME=skit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello skit"));

    // The stored copy keeps its own value, and no staged file survives the run.
    let stored = fs::read_to_string(sandbox.data.path().join("scripts/greet/script.sh")).unwrap();
    assert!(stored.contains("NAME=\"world\""), "{stored}");
    let leftovers = fs::read_dir(sandbox.data.path().join("scripts/greet"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|item| item.file_name().to_string_lossy().starts_with(".run-"))
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn a_pi_prompt_that_starts_with_a_flag_reports_the_added_newline() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Flagged", "--no-input"])
        .write_stdin("--version please\n")
        .assert()
        .success();

    sandbox
        .command()
        .args([
            "run",
            "flagged",
            "--no-input",
            "--runner",
            "pi",
            "--dry-run",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "Added a newline to keep the Pi prompt in message mode",
        ));
}

#[cfg(unix)]
#[test]
fn a_python_run_reuses_a_private_uv_that_is_already_installed() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("hello.py");
    fs::write(&source, "print('from python')\n").unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Hello"])
        .assert()
        .success();

    // A private uv already exists, so the first run must not start a download.
    let uv = sandbox.data.path().join("bin/uv");
    fs::create_dir_all(uv.parent().unwrap()).unwrap();
    fs::write(&uv, "#!/bin/sh\nprintf 'stub uv %s\\n' \"$*\"\n").unwrap();
    let mut mode = fs::metadata(&uv).unwrap().permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut mode, 0o755);
    fs::set_permissions(&uv, mode).unwrap();

    sandbox
        .command()
        .env("PATH", "/nonexistent")
        .args(["run", "hello", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("stub uv run --no-project"));
}

#[test]
fn show_reports_a_missing_target_and_a_drifted_copy() {
    let sandbox = Sandbox::new();
    let gone = sandbox.data.path().join("gone.py");
    fs::write(&gone, "print(1)\n").unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&gone)
        .args(["--name", "Gone", "--ref"])
        .assert()
        .success();
    fs::remove_file(&gone).unwrap();

    let missing = sandbox.ok(&["show", "gone"]);
    assert!(missing.contains("⚠ missing:"), "{missing}");

    // A managed declaration whose source constant is gone reports drift.
    let drifted = sandbox.data.path().join("drift.sh");
    fs::write(
        &drifted,
        "#!/usr/bin/env bash\nNAME=\"world\"\necho \"$NAME\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&drifted)
        .args(["--name", "Drift"])
        .assert()
        .success();
    sandbox.ok(&["params", "drift", "--manage", "NAME"]);
    let stored = sandbox.data.path().join("scripts/drift/script.sh");
    let text = fs::read_to_string(&stored).unwrap();
    fs::write(&stored, text.replace("NAME=\"world\"", "OTHER=\"world\"")).unwrap();

    let report = sandbox.ok(&["show", "drift"]);
    assert!(report.contains("drifted from the script"), "{report}");
}

#[test]
fn show_reports_an_entry_whose_stored_payload_was_deleted() {
    let sandbox = Sandbox::new();
    sandbox.python("Sample", "print(1)\n");
    fs::remove_file(sandbox.data.path().join("scripts/sample/script.py")).unwrap();

    let report = sandbox.ok(&["show", "sample"]);

    assert!(report.contains("⚠ missing:"), "{report}");
    assert!(report.contains("No form fields"), "{report}");
}

#[test]
fn params_json_reports_no_unmanaged_names_for_a_reader_driven_source() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("reader.py");
    fs::write(
        &source,
        concat!(
            "import argparse\n",
            "parser = argparse.ArgumentParser()\n",
            "parser.add_argument('--target')\n",
            "args = parser.parse_args()\n",
        ),
    )
    .unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Reader"])
        .assert()
        .success();

    let report = sandbox.ok(&["params", "reader", "--json"]);

    assert!(report.contains("\"unmanaged\":[]"), "{report}");
    assert!(report.contains("target"), "{report}");
}

#[cfg(unix)]
#[test]
fn a_referenced_entry_runs_from_its_original_path_and_keeps_its_bytes() {
    let sandbox = Sandbox::new();
    let source = sandbox.data.path().join("outside.sh");
    fs::write(
        &source,
        "#!/usr/bin/env bash\nNAME=\"world\"\necho \"hello $NAME\"\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--name", "Outside", "--ref"])
        .assert()
        .success();
    // A reference entry keeps its original bytes, so the run needs no staged rewrite.
    sandbox
        .command()
        .args(["run", "outside", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello world"));
    assert_eq!(
        fs::read_to_string(&source).unwrap(),
        "#!/usr/bin/env bash\nNAME=\"world\"\necho \"hello $NAME\"\n"
    );
}

#[test]
fn adding_a_prompt_refuses_a_runner_that_is_not_configured() {
    let sandbox = Sandbox::new();

    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--runner", "nosuch"])
        .args(["--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "prompt runner \"nosuch\" is not configured",
        ));
    assert!(!sandbox.data.path().join("scripts/review").exists());

    // A configured runner is accepted and recorded.
    sandbox.ok(&["runner", "add", "mycli", "--", "mycli", "{{prompt}}"]);
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--runner", "mycli"])
        .args(["--no-input"])
        .write_stdin("Review {{subject}}.\n")
        .assert()
        .success();
    let report = sandbox.ok(&["show", "review", "--json"]);
    assert!(report.contains("\"runner\":\"mycli\""), "{report}");
}
