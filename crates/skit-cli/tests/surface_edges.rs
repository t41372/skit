//! Reporting, refusal, and management paths that the main lanes do not reach.

use std::fs;

use predicates::prelude::*;
use tempfile::TempDir;

#[cfg(unix)]
use std::{
    io::{self, Read as _},
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::Duration,
};

#[cfg(unix)]
use skit_application::form_state::FormStateRepository as _;
#[cfg(unix)]
use skit_domain::Slug;
#[cfg(unix)]
use skit_runtime::{ProgramProbe as _, SystemProbe};
#[cfg(unix)]
use skit_store::FileFormStateStore;

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

#[cfg(unix)]
fn shell_fixture(sandbox: &Sandbox, name: &str, body: &str) -> (PathBuf, PathBuf) {
    let source = sandbox.data.path().join(format!("{name}.sh"));
    let bin = sandbox.data.path().join(format!("{name}-bin"));
    fs::create_dir(&bin).unwrap();
    let bash = bin.join("bash");
    fs::write(&bash, "#!/bin/sh\nexec /bin/sh \"$@\"\n").unwrap();
    fs::set_permissions(&bash, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(&source, body).unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--kind", "shell", "--name", name, "--no-input"])
        .assert()
        .success();
    (source, bin)
}

#[cfg(unix)]
fn add_real_shell(sandbox: &Sandbox, name: &str, body: &str) -> PathBuf {
    let source = sandbox.data.path().join(format!("{name}.sh"));
    fs::write(&source, body).unwrap();
    sandbox
        .command()
        .args(["add"])
        .arg(&source)
        .args(["--kind", "shell", "--name", name, "--no-input"])
        .assert()
        .success();
    source
}

#[cfg(unix)]
fn manage_shell_params(sandbox: &Sandbox, name: &str, params: &[&str]) {
    let mut command = sandbox.command();
    command.args(["params", name]);
    for param in params {
        command.args(["--manage", param]);
    }
    command.assert().success();
}

#[cfg(unix)]
fn find_program(name: &str) -> PathBuf {
    SystemProbe
        .find_program(name)
        .unwrap_or_else(|| panic!("the shell runtime parity owner requires {name} on PATH"))
}

#[cfg(unix)]
fn output_text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[cfg(unix)]
fn wait_for_file(path: &Path) -> bool {
    (0..500).any(|_| {
        if path.is_file() {
            true
        } else {
            thread::sleep(Duration::from_millis(10));
            false
        }
    })
}

#[cfg(unix)]
fn read_complete_staged_path(report: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(report).ok()?;
    let path = text.strip_suffix('\n')?;
    (!path.is_empty() && !path.contains('\n')).then(|| PathBuf::from(path))
}

#[cfg(unix)]
fn wait_for_complete_staged_path(report: &Path) -> Option<PathBuf> {
    (0..500).find_map(|_| {
        let path = read_complete_staged_path(report);
        if path.is_none() {
            thread::sleep(Duration::from_millis(10));
        }
        path
    })
}

#[cfg(unix)]
fn wait_for_child(child: &mut Child) -> io::Result<std::process::ExitStatus> {
    for _ in 0..500 {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
    }
    let kill_error = child.kill().err();
    match child.wait() {
        Ok(status) => Ok(status),
        Err(error) => Err(kill_error.unwrap_or(error)),
    }
}

#[cfg(unix)]
struct ChildCleanupGuard {
    child: Option<Child>,
    release: PathBuf,
}

#[cfg(unix)]
impl ChildCleanupGuard {
    fn new(child: Child, release: PathBuf) -> Self {
        Self {
            child: Some(child),
            release,
        }
    }

    fn finish(mut self) -> io::Result<Output> {
        fs::write(&self.release, [])?;
        let child = self.child.as_mut().expect("child guard owns one process");
        let status = wait_for_child(child)?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        if let Some(mut pipe) = child.stdout.take() {
            pipe.read_to_end(&mut stdout)?;
        }
        if let Some(mut pipe) = child.stderr.take() {
            pipe.read_to_end(&mut stderr)?;
        }
        self.child.take();
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }
}

#[cfg(unix)]
impl Drop for ChildCleanupGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        let _ = fs::write(&self.release, []);
        let _ = wait_for_child(child);
    }
}

#[cfg(unix)]
#[test]
fn staged_report_checkpoint_and_child_cleanup_are_panic_safe() {
    let signals = TempDir::new().unwrap();
    let report = signals.path().join("report");
    fs::write(&report, []).unwrap();
    assert_eq!(read_complete_staged_path(&report), None);
    fs::write(&report, "/tmp/.injected-checkpoint.sh").unwrap();
    assert_eq!(read_complete_staged_path(&report), None);
    fs::write(&report, "/tmp/.injected-checkpoint.sh\n").unwrap();
    assert_eq!(
        read_complete_staged_path(&report),
        Some(PathBuf::from("/tmp/.injected-checkpoint.sh"))
    );

    let ready = signals.path().join("ready");
    let release = signals.path().join("release");
    let completed = signals.path().join("completed");
    let mut ready_seen = false;
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg(": > \"$READY\"; while [ ! -f \"$RELEASE\" ]; do sleep 0.01; done; : > \"$COMPLETED\"")
            .env("READY", &ready)
            .env("RELEASE", &release)
            .env("COMPLETED", &completed)
            .spawn()
            .unwrap();
        let _child = ChildCleanupGuard::new(child, release.clone());
        ready_seen = wait_for_file(&ready);
        panic!("exercise the staged-child unwind checkpoint");
    }));

    assert!(unwind.is_err());
    assert!(ready_seen);
    assert!(release.is_file());
    assert!(completed.is_file());
}

#[cfg(unix)]
fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(root, &path, output);
            } else {
                output.push((
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                ));
            }
        }
    }

    let mut output = Vec::new();
    visit(root, root, &mut output);
    output.sort();
    output
}

#[cfg(unix)]
fn snapshot_user_data(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    snapshot_tree(root)
        .into_iter()
        .filter(|(path, _)| !path.starts_with(".locks"))
        .collect()
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
fn editing_without_a_configured_editor_falls_back_to_vi_and_reports_a_launch_failure() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    use std::io::Read as _;

    // v0.4: nothing configured and no env editor resolves the platform default `vi`
    // (editor.py:30-46). With an empty PATH the launch itself fails, which is a failed
    // operation (exit 1) that names the command and teaches the config key.
    let sandbox = Sandbox::new();
    sandbox.python("Sample", "print(1)\n");
    let empty_path = TempDir::new().unwrap();

    sandbox
        .command()
        .env_remove("VISUAL")
        .env_remove("EDITOR")
        .env("PATH", empty_path.path())
        .args(["edit", "sample", "--no-input"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("Could not launch the editor (vi)"))
        .stderr(predicate::str::contains("skit config editor"));

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(std::path::PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(["add", "--edit", "--name", "Draft"]);
    command.env("SKIT_DATA_DIR", sandbox.data.path());
    command.env("SKIT_STATE_DIR", sandbox.state.path());
    command.env("SKIT_CONFIG_DIR", sandbox.config.path());
    command.env("SKIT_LANG", "en");
    command.env_remove("VISUAL");
    command.env_remove("EDITOR");
    command.env("PATH", empty_path.path());
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = std::thread::spawn(move || {
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        output
    });
    let status = child.wait().unwrap();
    drop(pair.master);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    assert_eq!(status.exit_code(), 1, "{output}");
    assert!(
        output.contains("Could not launch the editor (vi)"),
        "{output}"
    );
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
fn a_declared_row_without_a_source_binding_warns_and_skips_source_only_edits() {
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
    let stored = sandbox.data.path().join("scripts/shell/script.sh");
    let source_before = fs::read(&stored).unwrap();
    let meta_before = fs::read(&meta).unwrap();

    for (arguments, warning) in [
        (
            vec!["params", "shell", "--prompt", "extra=Label"],
            "extra isn't a managed parameter; skipped.",
        ),
        (
            vec!["params", "shell", "--env-source", "extra=VAR"],
            "extra isn't a managed parameter; --env-source skipped.",
        ),
        (
            vec!["params", "shell", "--secret", "extra"],
            "extra isn't a managed parameter; skipped.",
        ),
    ] {
        sandbox
            .command()
            .args(&arguments)
            .assert()
            .success()
            .stderr(predicate::str::contains(warning));
    }

    sandbox
        .command()
        .args(["params", "shell", "--prompt", "missing=Label"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "missing isn't a managed parameter; skipped.",
        ));
    assert_eq!(fs::read(stored).unwrap(), source_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
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
            "Pass exactly one runner name or --row INDEX.",
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

    // Without a terminal there is nobody to ask, so skit refuses before it looks at any
    // directory — however many it would have found (`src/skit/cli.py:5112-5116`).
    sandbox
        .command()
        .env("HOME", home.path())
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Nothing installed: name a target (claude, codex, agents) or pass --to DIR.",
        ));

    let project = TempDir::new().unwrap();
    fs::create_dir(project.path().join(".agents")).unwrap();
    // `--project` selects a scope, not a target, so it reaches the same refusal.
    sandbox
        .command()
        .current_dir(project.path())
        .args(["agent", "install", "--project"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Nothing installed: name a target (claude, codex, agents) or pass --to DIR.",
        ));
    assert!(!project.path().join(".agents/skills/skit/SKILL.md").exists());

    // A named target is consent by itself. `agents` is a project convention, so it lands in
    // ./.agents whatever --project says (`src/skit/agentskill.py:69-70`).
    sandbox
        .command()
        .current_dir(project.path())
        .args(["agent", "install", "agents", "--project"])
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
fn agent_skill_installation_refuses_before_it_resolves_the_user_directory() {
    let sandbox = Sandbox::new();

    // The non-interactive refusal comes first, so an unresolvable home never changes it
    // (`src/skit/cli.py:5112-5116` runs before `agentskill.default_roots`).
    sandbox
        .command()
        .env_remove("HOME")
        .args(["agent", "install"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Nothing installed: name a target (claude, codex, agents) or pass --to DIR.",
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

#[cfg(unix)]
#[test]
fn test_secret_read_masks_the_echo_but_delivers_the_value() {
    let sandbox = Sandbox::new();
    add_real_shell(
        &sandbox,
        "secret-read",
        "#!/usr/bin/env bash\nread -s -p \"Password: \" PW\necho \"len=${#PW}\"\n",
    );
    manage_shell_params(&sandbox, "secret-read", &["input-1"]);
    sandbox.ok(&["params", "secret-read", "--secret", "input-1"]);

    let output = sandbox
        .command()
        .args([
            "run",
            "secret-read",
            "--no-input",
            "--set",
            "input-1=hunter2",
        ])
        .output()
        .unwrap();
    let text = output_text(&output);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("Password: ***\nlen=7\n"), "{text}");
    assert!(
        !text.contains("hunter2"),
        "secret leaked to visible output:\n{text}"
    );
}

#[cfg(unix)]
#[test]
fn test_read_in_a_loop_takes_the_value_once_then_reads_real_stdin() {
    let sandbox = Sandbox::new();
    add_real_shell(
        &sandbox,
        "loop-read",
        concat!(
            "#!/usr/bin/env bash\n",
            "for i in 1 2 3; do\n",
            "  read -p \"Item: \" it\n",
            "  echo \"item=$it\"\n",
            "done\n",
        ),
    );
    manage_shell_params(&sandbox, "loop-read", &["input-1"]);

    sandbox
        .command()
        .args(["run", "loop-read", "--no-input", "--set", "input-1=first"])
        .write_stdin("second\nthird\n")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Item: first\nitem=first\nitem=second\nitem=third\n",
        ));
}

#[cfg(unix)]
#[test]
#[ignore = "host-tool gate: requires bash, sh, zsh, and dash on PATH; run with cargo test -p skit-cli-rs --test surface_edges test_the_preamble_runs_on_every_supported_dialect -- --ignored --exact"]
fn test_the_preamble_runs_on_every_supported_dialect() {
    for shell in ["bash", "sh", "zsh", "dash"] {
        let sandbox = Sandbox::new();
        let name = format!("dialect-{shell}");
        let interpreter = find_program(shell);
        add_real_shell(
            &sandbox,
            &name,
            concat!(
                "#!/bin/sh\n",
                "NAME=x\n",
                "read who\n",
                "echo \"hi $who / $NAME\"\n",
                "read it\n",
                "echo \"it=$it\"\n",
            ),
        );
        manage_shell_params(&sandbox, &name, &["NAME", "input-1"]);
        sandbox.ok(&[
            "params",
            &name,
            "--interpreter",
            interpreter.to_str().unwrap(),
        ]);

        sandbox
            .command()
            .args([
                "run",
                &name,
                "--no-input",
                "--set",
                "NAME=y",
                "--set",
                "input-1=Ada",
            ])
            .write_stdin("typed\n")
            .assert()
            .success()
            .stdout(predicate::str::contains("Ada\nhi Ada / y\nit=typed\n"));
    }
}

#[cfg(unix)]
#[test]
fn test_set_u_and_set_e_survive_the_preamble() {
    let sandbox = Sandbox::new();
    add_real_shell(
        &sandbox,
        "strict-shell",
        concat!(
            "#!/usr/bin/env bash\n",
            "set -euo pipefail\n",
            "OUT=/tmp/out\n",
            "read -p \"Deploy? \" confirm\n",
            "echo \"$OUT $confirm\"\n",
        ),
    );
    manage_shell_params(&sandbox, "strict-shell", &["OUT", "input-1"]);

    sandbox
        .command()
        .args([
            "run",
            "strict-shell",
            "--no-input",
            "--set",
            "OUT=/tmp/x",
            "--set",
            "input-1=yes",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("/tmp/x yes\n"));
}

#[cfg(unix)]
#[test]
fn test_secret_value_never_reaches_stdout() {
    let sandbox = Sandbox::new();
    let report = sandbox.state.path().join("staged-path");
    let release = sandbox.state.path().join("release");
    let source = add_real_shell(
        &sandbox,
        "secret-file",
        concat!(
            "#!/usr/bin/env bash\n",
            "API_KEY=changeme\n",
            "printf '%s\\n' \"$0\" > \"${SKIT_STAGE_REPORT}.pending\"\n",
            "mv \"${SKIT_STAGE_REPORT}.pending\" \"$SKIT_STAGE_REPORT\"\n",
            "while [ ! -f \"$SKIT_STAGE_RELEASE\" ]; do sleep 0.01; done\n",
            "printf 'done\\n'\n",
        ),
    );
    manage_shell_params(&sandbox, "secret-file", &["API_KEY"]);
    sandbox.ok(&["params", "secret-file", "--secret", "API_KEY"]);
    let source_before = fs::read(&source).unwrap();
    let data_before = snapshot_user_data(sandbox.data.path());

    let child = Command::new(env!("CARGO_BIN_EXE_skit"))
        .env("SKIT_DATA_DIR", sandbox.data.path())
        .env("SKIT_STATE_DIR", sandbox.state.path())
        .env("SKIT_CONFIG_DIR", sandbox.config.path())
        .env("SKIT_LANG", "en")
        .env("SKIT_STAGE_REPORT", &report)
        .env("SKIT_STAGE_RELEASE", &release)
        .args([
            "run",
            "secret-file",
            "--no-input",
            "--set",
            "API_KEY=s3cr3t",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let child = ChildCleanupGuard::new(child, release);
    let staged = wait_for_complete_staged_path(&report);
    let observation = staged.as_ref().map(|path| {
        (
            path.is_file(),
            fs::metadata(path).map(|metadata| metadata.permissions().mode() & 0o777),
            fs::read_to_string(path).map(|text| text.contains("s3cr3t")),
        )
    });
    let output = child.finish();

    assert!(
        staged.is_some(),
        "shell child did not publish a complete staged path"
    );
    let staged = staged.unwrap();
    let (was_file, mode, contained_secret) = observation.unwrap();
    assert!(
        was_file,
        "staged source vanished before the child completed"
    );
    assert_eq!(mode.unwrap(), 0o600);
    assert!(
        contained_secret.unwrap(),
        "the real secret must reach the private staged source"
    );
    let output = output.unwrap();
    let text = output_text(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.lines().any(|line| line == "done"), "{text}");
    assert!(
        !text.contains("s3cr3t"),
        "secret leaked to visible output:\n{text}"
    );
    assert!(
        !staged.exists(),
        "secret-bearing staged source survived the run"
    );
    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(snapshot_user_data(sandbox.data.path()), data_before);
    let state_bytes = snapshot_tree(sandbox.state.path())
        .into_iter()
        .flat_map(|(_, bytes)| bytes)
        .collect::<Vec<_>>();
    assert!(
        !String::from_utf8_lossy(&state_bytes).contains("s3cr3t"),
        "secret leaked into state"
    );
}

#[cfg(unix)]
#[test]
fn test_execute_runs_a_managed_read_with_the_block_in_place() {
    let sandbox = Sandbox::new();
    add_real_shell(
        &sandbox,
        "managed-read",
        "#!/usr/bin/env bash\nread -s -p \"Password: \" PW\necho \"len=${#PW}\"\n",
    );
    manage_shell_params(&sandbox, "managed-read", &["input-1"]);
    sandbox.ok(&["params", "managed-read", "--secret", "input-1"]);
    let stored = sandbox.data.path().join("scripts/managed-read/script.sh");
    let stored_before = fs::read(&stored).unwrap();
    assert!(
        String::from_utf8_lossy(&stored_before).contains("# /// script"),
        "the managed block must be physically present"
    );

    let output = sandbox
        .command()
        .args([
            "run",
            "managed-read",
            "--no-input",
            "--set",
            "input-1=hunter2",
        ])
        .output()
        .unwrap();
    let text = output_text(&output);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("Password: ***\nlen=7\n"), "{text}");
    assert!(
        !text.contains("hunter2"),
        "secret leaked to visible output:\n{text}"
    );
    assert_eq!(fs::read(&stored).unwrap(), stored_before);
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
        // The exact sentence version 0.4 ships (`src/skit/langs/launch.py:762-767`,
        // catalogued at `locales/zh_CN/LC_MESSAGES/skit.po:2227`).
        .stderr(predicate::str::contains(
            "Warning: Pi would interpret the beginning of this prompt as a CLI option, file, or package command. skit prepended one newline and is continuing; the prompt delivered to Pi is one character longer than the rendered text.",
        ));
}

#[test]
fn an_amp_prompt_reports_that_the_builtin_runner_is_one_shot() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "--name", "Review", "--no-input"])
        .write_stdin("Review this change.\n")
        .assert()
        .success();

    sandbox
        .command()
        .args([
            "run",
            "review",
            "--no-input",
            "--runner",
            "amp",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("amp -x"))
        .stderr(predicate::str::contains(
            "The built-in amp runner is one-shot: amp -x runs this prompt once and does not open an interactive session.",
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
fn test_e2e_run_shell_script() {
    let sandbox = Sandbox::new();
    let (source, bin) = shell_fixture(&sandbox, "hi", "#!/bin/bash\necho \"shell-ran-ok\"\n");
    let source_before = fs::read(&source).unwrap();
    let data_before = snapshot_user_data(sandbox.data.path());
    let config_before = snapshot_tree(sandbox.config.path());

    sandbox
        .command()
        .env("PATH", bin)
        .args(["run", "hi", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("shell-ran-ok"));

    assert_eq!(fs::read(source).unwrap(), source_before);
    assert_eq!(snapshot_user_data(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    let state = FileFormStateStore::new(sandbox.state.path()).load(&Slug::parse("hi").unwrap());
    assert_eq!(state.last_run.exit, Some(0));
    assert!(
        state
            .last_run
            .at
            .as_deref()
            .is_some_and(|at| !at.is_empty())
    );
    assert_eq!(state.last_run.values, Some(Default::default()));
    assert!(state.values.is_empty());
}

#[cfg(unix)]
#[test]
fn test_e2e_run_shell_env_param_reaches_child() {
    let sandbox = Sandbox::new();
    let (source, bin) = shell_fixture(
        &sandbox,
        "width",
        "#!/bin/bash\n: \"${WIDTH:=640}\"\necho \"w=$WIDTH\"\n",
    );
    sandbox.ok(&["params", "width", "--manage", "WIDTH"]);
    let source_before = fs::read(&source).unwrap();
    let data_before = snapshot_user_data(sandbox.data.path());
    let config_before = snapshot_tree(sandbox.config.path());

    sandbox
        .command()
        .env("PATH", bin)
        .args(["run", "width", "--set", "WIDTH=800", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("w=800"));

    assert_eq!(fs::read(source).unwrap(), source_before);
    assert_eq!(snapshot_user_data(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    let state = FileFormStateStore::new(sandbox.state.path()).load(&Slug::parse("width").unwrap());
    assert_eq!(state.values.get("WIDTH").map(String::as_str), Some("800"));
    assert_eq!(state.last_run.exit, Some(0));
    assert!(
        state
            .last_run
            .at
            .as_deref()
            .is_some_and(|at| !at.is_empty())
    );
    assert_eq!(
        state
            .last_run
            .values
            .as_ref()
            .and_then(|values| values.get("WIDTH"))
            .map(String::as_str),
        Some("800")
    );
}

#[cfg(unix)]
#[test]
fn test_e2e_dry_run_shows_interpreter_and_script() {
    let sandbox = Sandbox::new();
    let (_source, bin) = shell_fixture(&sandbox, "dry", "#!/bin/bash\necho hi\n");
    let data_before = snapshot_tree(sandbox.data.path());
    let state_before = snapshot_tree(sandbox.state.path());
    let config_before = snapshot_tree(sandbox.config.path());

    sandbox
        .command()
        .env("PATH", bin)
        .args(["run", "dry", "--dry-run", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("bash"))
        .stdout(predicate::str::contains("script.sh"));

    assert_eq!(snapshot_tree(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
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
            "Unknown runner: nosuch. Configured runners: claude, codex, opencode, amp, antigravity, copilot, cursor, pi",
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
