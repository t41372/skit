use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use skit_application::EntryRepository as _;
use skit_store::FileStore;
use tempfile::TempDir;

fn write_command_entry(data: &Path, with_parameter: bool) {
    let directory = data.join("scripts/demo");
    fs::create_dir_all(&directory).unwrap();
    let parameter = if with_parameter {
        concat!(
            "params = [\"name\"]\n",
            "[[parameters]]\n",
            "name = \"name\"\n",
            "delivery = \"placeholder\"\n",
            "required = true\n",
        )
    } else {
        "params = []\n"
    };
    let template = if with_parameter {
        "echo {name}"
    } else {
        "echo done"
    };
    fs::write(
        directory.join("meta.toml"),
        format!(
            concat!(
                "schema = 1\n",
                "name = \"Demo\"\n",
                "kind = \"command\"\n",
                "mode = \"copy\"\n",
                "source = \"\"\n",
                "source_hash = \"\"\n",
                "added_at = \"2026-08-08T00:00:00Z\"\n",
                "id = \"0123456789abcdef0123456789abcdef\"\n",
                "workdir = \"invoke\"\n",
                "description = \"\"\n",
                "template = {template:?}\n",
                "{parameter}",
            ),
            parameter = parameter,
            template = template,
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn write_secret_command_entry(data: &Path) {
    let directory = data.join("scripts/secret");
    fs::create_dir_all(&directory).unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Secret\"\n",
            "kind = \"command\"\n",
            "mode = \"copy\"\n",
            "source = \"\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"1123456789abcdef0123456789abcdef\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
            "template = \"echo {name}\"\n",
            "params = [\"name\", \"token\"]\n",
            "[[parameters]]\n",
            "name = \"name\"\n",
            "delivery = \"placeholder\"\n",
            "required = true\n",
            "[[parameters]]\n",
            "name = \"token\"\n",
            "delivery = \"placeholder\"\n",
            "required = true\n",
            "secret = true\n",
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn write_pinned_prompt_entry(data: &Path) {
    let directory = data.join("scripts/pinned-prompt");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("prompt.md"), "Hello\n").unwrap();
    fs::write(
        directory.join("meta.toml"),
        concat!(
            "schema = 1\n",
            "name = \"Pinned prompt\"\n",
            "kind = \"prompt\"\n",
            "mode = \"copy\"\n",
            "source = \"prompt.md\"\n",
            "source_hash = \"\"\n",
            "added_at = \"2026-08-08T00:00:00Z\"\n",
            "id = \"2123456789abcdef0123456789abcdef\"\n",
            "workdir = \"invoke\"\n",
            "description = \"\"\n",
            "runner = \"local\"\n",
            "interpolate = true\n",
        ),
    )
    .unwrap();
    FileStore::new(data).rebuild_registry().unwrap();
}

fn run_in_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
) -> (u32, String) {
    run_pty(args, data, state, config, input, true)
}

fn run_plain_in_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
) -> (u32, String) {
    run_pty(args, data, state, config, input, false)
}

fn run_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
    answer_cursor_query: bool,
) -> (u32, String) {
    run_pty_configured(
        args,
        data,
        state,
        config,
        input,
        answer_cursor_query,
        |_| {},
    )
}

fn run_pty_configured(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
    input: &[&[u8]],
    answer_cursor_query: bool,
    configure: impl FnOnce(&mut CommandBuilder),
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    configure(&mut command);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let mut writer = pair.master.take_writer().unwrap();
    thread::sleep(Duration::from_millis(40));
    if answer_cursor_query {
        let _ = writer.write_all(b"\x1b[1;1R");
        let _ = writer.flush();
    }
    for bytes in input {
        thread::sleep(Duration::from_millis(120));
        if writer.write_all(bytes).is_err() {
            break;
        }
        let _ = writer.flush();
    }
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}

#[test]
fn bare_agent_install_lists_existing_targets_and_writes_only_the_confirmed_choice() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join(".agents")).unwrap();

    let (code, output) = run_pty_configured(
        &["agent", "install"],
        data.path(),
        state.path(),
        config.path(),
        &[b"2\n", b"y\n"],
        false,
        |command| {
            command.env("HOME", home.path());
            command.env("USERPROFILE", home.path());
            command.cwd(project.path());
        },
    );

    assert_eq!(code, 0, "{output}");
    assert!(output.contains("1. claude (user)"), "{output}");
    assert!(output.contains("2. agents (project)"), "{output}");
    assert!(output.contains("Write the skill into"), "{output}");
    assert!(
        project
            .path()
            .join(".agents/skills/skit/SKILL.md")
            .is_file(),
        "{output}"
    );
    assert!(!home.path().join(".claude/skills").exists());
}

#[cfg(unix)]
fn run_with_null_stdin_in_pty(
    args: &[&str],
    data: &Path,
    state: &Path,
    config: &Path,
) -> (u32, String) {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new("sh");
    command.args(["-c", "exec \"$@\" < /dev/null", "sh"]);
    command.arg(PathBuf::from(env!("CARGO_BIN_EXE_skit")));
    command.args(args);
    command.env("TERM", "xterm-256color");
    command.env("SKIT_LANG", "en");
    command.env("SKIT_DATA_DIR", data);
    command.env("SKIT_STATE_DIR", state);
    command.env("SKIT_CONFIG_DIR", config);
    let mut child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().unwrap();
    let drain = thread::spawn(move || {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let writer = pair.master.take_writer().unwrap();
    let status = child.wait().unwrap();
    drop(writer);
    let output = String::from_utf8_lossy(&drain.join().unwrap()).into_owned();
    (status.exit_code(), output)
}

#[test]
fn terminal_detection_keeps_automation_flags_noninteractive() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), false);

    let (code, output) = run_in_pty(
        &["add", "--no-input"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 2, "{output}");

    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run", "--no-input"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 0, "{output}");

    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run", "--raw"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 2, "{output}");
}

#[test]
fn bare_add_uses_the_typed_workflow_and_returns_the_created_entry() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("typed-add.sh");
    fs::write(&source, b"#!/bin/sh\necho typed\n").unwrap();
    let path_input = format!("{}", source.display());

    let (code, output) = run_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[path_input.as_bytes(), b"\r", b"\x1b[1;1R", b"\x13"],
    );

    assert_eq!(code, 0, "{output}");
    let entry = FileStore::new(data.path()).resolve("typed-add").unwrap();
    assert_eq!(entry.meta.kind.as_str(), "shell");
    assert!(output.contains("Added: typed-add"), "{output}");
}

#[test]
fn bare_add_plain_menu_and_typed_cancel_keep_the_latest_main_contract() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();
    let source = data.path().join("plain-add.sh");
    fs::write(&source, b"#!/bin/sh\necho plain\n").unwrap();
    let path_input = format!("{}\n", source.display());

    let (code, output) = run_plain_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[b"1\n", path_input.as_bytes()],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("What would you like to add?"), "{output}");
    assert!(output.contains("Which one?"), "{output}");
    FileStore::new(data.path()).resolve("plain-add").unwrap();

    fs::remove_file(config.path().join("config.toml")).unwrap();
    let (code, output) = run_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
}

#[cfg(unix)]
#[test]
fn one_nonterminal_standard_stream_disables_interactive_forms() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), false);

    let (code, output) =
        run_with_null_stdin_in_pty(&["add"], data.path(), state.path(), config.path());
    assert_eq!(code, 2, "{output}");

    let (code, output) = run_with_null_stdin_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
    );
    assert_eq!(code, 0, "{output}");
}

#[test]
fn terminal_browser_runs_host_success_error_and_host_quit_paths() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), false);

    let (code, output) = run_in_pty(
        &["tui"],
        data.path(),
        state.path(),
        config.path(),
        &[b"x", b"\x1b[A", b"h", b"\x1b[1;1R", b"\x1b", b"q"],
    );
    assert_eq!(code, 0, "{output}");

    let invalid_config = config.path().join("not-a-directory");
    fs::write(&invalid_config, "file").unwrap();
    // Reads follow version 0.4 and project I/O failures to defaults. The first q
    // must reach the focused Preferences input. Escape returns to the library,
    // where the final q exits.
    let (code, output) = run_in_pty(
        &["tui"],
        data.path(),
        state.path(),
        &invalid_config,
        &[b",", b"\x1b[1;1R", b"q", b"\x1b", b"q"],
    );
    assert_eq!(code, 0, "{output}");

    // Enter opens the run form; Ctrl+R is the run form's explicit run chord
    // (`src/skit/tui_form.py:555` `Binding("ctrl+r", "submit", …)`). Ctrl+S there means
    // "Save as preset" (tui_form.py:548), so it would open a modal and wait for a name.
    fs::write(config.path().join("config.toml"), "after_run = \"exit\"\n").unwrap();
    let (code, output) = run_in_pty(
        &["tui"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\r", b"\x1b[1;1R", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");
}

#[test]
fn terminal_run_form_can_submit_or_cancel_without_plain_input() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);

    // Ctrl+R is the run form's explicit run chord (`src/skit/tui_form.py:555`).
    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");

    // Ctrl+S opens the preset name modal (`tui_form.py:548`). Enter saves the typed name and
    // returns to the form (`tui_form.py:363-366`), so the run still happens afterwards.
    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada", b"\x13", b"nightly", b"\r", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Preset \"nightly\" saved."), "{output}");
    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("nightly"), "{saved}");
    assert!(saved.contains("Ada"), "{saved}");

    // Escape inside the modal dismisses only the modal (`tui_form.py:376-377`), so the form
    // survives and the following Ctrl+R still runs.
    let (code, output) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada", b"\x13", b"\x1b", b"\x12"],
    );
    assert_eq!(code, 0, "{output}");

    let (code, _) = run_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x1b"],
    );
    assert_eq!(code, 130);
}

#[test]
fn terminal_authoring_and_confirmation_paths_need_no_hidden_cli_knowledge() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("source.sh");
    fs::write(&source, "echo ok\n").unwrap();
    let source_input = source.display().to_string().into_bytes();

    // The source step advertises Enter Continue; the review panel that follows advertises
    // Ctrl+S Add. The panel arrives after a host round trip, which re-enters the terminal and
    // asks for the cursor position again.
    let (code, output) = run_in_pty(
        &["add"],
        data.path(),
        state.path(),
        config.path(),
        &[source_input.as_slice(), b"\r", b"\x1b[1;1R", b"\x13"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(data.path().join("scripts/source/meta.toml").is_file());

    let editor = data.path().join("editor.sh");
    fs::write(
        &editor,
        "#!/bin/sh\nsleep 0.2\nprintf '#!/usr/bin/env python3\\nprint(1)\\n' > \"$1\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();

    let (code, output) = run_plain_in_pty(
        &["add", "--prompt", "--name", "Prompt"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["add", "--prompt", "--name", "No Body", "--no-input"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 2, "{output}");

    let (code, output) = run_plain_in_pty(
        &["edit", "Declined"],
        data.path(),
        state.path(),
        config.path(),
        &[b"n\n"],
    );
    assert_eq!(code, 130, "{output}");
    let (code, output) = run_plain_in_pty(
        &["edit", "New Script"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(data.path().join("scripts/new-script/meta.toml").is_file());

    let (code, output) = run_plain_in_pty(
        &["remove", "new-script"],
        data.path(),
        state.path(),
        config.path(),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["remove", "prompt"],
        data.path(),
        state.path(),
        config.path(),
        &[b"n\n"],
    );
    assert_eq!(code, 130, "{output}");
    let (code, output) = run_plain_in_pty(
        &["remove", "prompt"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x04"],
    );
    assert_eq!(code, 130, "{output}");

    let (code, output) = run_plain_in_pty(
        &["runner", "remove", "codex"],
        data.path(),
        state.path(),
        config.path(),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["runner", "remove", "claude"],
        data.path(),
        state.path(),
        config.path(),
        &[b"n\n"],
    );
    assert_eq!(code, 130, "{output}");

    let values = state.path().join("values");
    fs::create_dir_all(&values).unwrap();
    fs::write(
        values.join("prompt.toml"),
        "[presets.old]\n[ presets.keep ]\n",
    )
    .unwrap();
    let (code, output) = run_plain_in_pty(
        &["preset", "delete", "prompt", "old"],
        data.path(),
        state.path(),
        config.path(),
        &[b"y\n"],
    );
    assert_eq!(code, 0, "{output}");
    let (code, output) = run_plain_in_pty(
        &["preset", "delete", "prompt", "keep"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
}

#[test]
fn interactive_preset_save_collects_current_values_instead_of_saving_the_prefill_unasked() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);
    let _ = run_plain_in_pty(
        &["doctor", "--rebuild"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );

    let (code, output) = run_plain_in_pty(
        &["preset", "save", "demo", "favorite"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Grace\n"],
    );
    assert_eq!(code, 0, "{output}");
    let saved = fs::read_to_string(state.path().join("values/demo.toml")).unwrap();
    assert!(saved.contains("[presets.favorite]"), "{saved}");
    assert!(saved.contains("name = \"Grace\""), "{saved}");
}

#[test]
fn add_onboarding_accepts_clean_defaults_and_leaves_demoted_candidates_unmanaged() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("parameters.py");
    fs::write(
        &source,
        "KEEP = 1\nCOUNT = 0\nCOUNT += 1\nprint(KEEP, COUNT)\n",
    )
    .unwrap();
    let source = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source, "--name", "Parameters"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\r"],
    );
    assert_eq!(code, 0, "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/parameters/script.py")).unwrap();
    assert!(stored.contains("name = \"KEEP\""), "{stored}");
    assert!(!stored.contains("name = \"COUNT\""), "{stored}");
}

#[test]
fn add_onboarding_space_toggles_the_focused_checkbox() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("toggle.py");
    fs::write(&source, "VALUE = 1\nprint(VALUE)\n").unwrap();
    let source = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source, "--name", "Toggle"],
        data.path(),
        state.path(),
        config.path(),
        &[b" \r"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Space toggles"), "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/toggle/script.py")).unwrap();
    assert!(!stored.contains("[tool.skit]"), "{stored}");
}

#[test]
fn an_empty_onboarding_selection_does_not_delete_existing_managed_metadata() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("existing.py");
    let original = concat!(
        "# /// script\n",
        "# dependencies = []\n",
        "# [tool.skit]\n",
        "# schema = 1\n",
        "#\n",
        "# [[tool.skit.params]]\n",
        "# name = \"VALUE\"\n",
        "# kind = \"const\"\n",
        "# type = \"int\"\n",
        "# default = 1\n",
        "# ///\n",
        "VALUE = 1\n",
        "print(VALUE)\n",
    );
    fs::write(&source, original).unwrap();
    let source = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source, "--name", "Existing"],
        data.path(),
        state.path(),
        config.path(),
        &[b" \r"],
    );
    assert_eq!(code, 0, "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/existing/script.py")).unwrap();
    assert_eq!(stored, original);
}

#[test]
fn add_onboarding_distinguishes_modeled_and_dynamic_cli_surfaces() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let modeled = data.path().join("modeled.py");
    fs::write(
        &modeled,
        concat!(
            "VALUE = 1\n",
            "import argparse\n",
            "p = argparse.ArgumentParser()\n",
            "p.add_argument('--name')\n",
        ),
    )
    .unwrap();
    let modeled = modeled.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &modeled, "--name", "Modeled"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("skit read this script's own arguments"),
        "{output}"
    );
    let stored = fs::read_to_string(data.path().join("scripts/modeled/script.py")).unwrap();
    assert!(!stored.contains("[tool.skit]"), "{stored}");

    let dynamic = data.path().join("dynamic.py");
    fs::write(
        &dynamic,
        concat!(
            "VALUE = 1\n",
            "import argparse\n",
            "p = argparse.ArgumentParser()\n",
            "p.add_subparsers()\n",
        ),
    )
    .unwrap();
    let dynamic = dynamic.to_string_lossy().into_owned();
    let (code, output) = run_plain_in_pty(
        &["add", &dynamic, "--name", "Dynamic"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\r"],
    );
    assert_eq!(code, 0, "{output}");
    let stored = fs::read_to_string(data.path().join("scripts/dynamic/script.py")).unwrap();
    assert!(stored.contains("name = \"VALUE\""), "{stored}");
}

#[test]
fn reference_add_reports_onboarding_but_never_writes_the_original() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    let source = data.path().join("reference.py");
    let original = b"VALUE = 1\n";
    fs::write(&source, original).unwrap();
    let source_arg = source.to_string_lossy().into_owned();

    let (code, output) = run_plain_in_pty(
        &["add", &source_arg, "--name", "Reference", "--ref"],
        data.path(),
        state.path(),
        config.path(),
        &[],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("parameter setup was skipped"), "{output}");
    assert_eq!(fs::read(source).unwrap(), original);
}

#[test]
fn terminal_plain_launch_menu_uses_the_same_prefill_and_argument_contract() {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let config = TempDir::new().unwrap();
    write_command_entry(data.path(), true);
    fs::write(config.path().join("config.toml"), "form = \"plain\"\n").unwrap();

    let (code, output) = run_plain_in_pty(
        &[
            "run",
            "demo",
            "--set",
            "name=Grace",
            "--",
            "two words",
            "single",
        ],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Grace"), "{output}");

    let (code, output) = run_plain_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("two words"), "{output}");
    assert!(output.contains("single"), "{output}");

    let (code, output) = run_plain_in_pty(
        &["run", "demo", "--dry-run", "--forget-args"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(!output.contains("two words"), "{output}");
    assert!(!output.contains("single"), "{output}");

    let (code, output) = run_plain_in_pty(
        &["run", "demo", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\x04"],
    );
    assert_eq!(code, 130, "{output}");

    write_secret_command_entry(data.path());
    let (code, output) = run_plain_in_pty(
        &["run", "secret", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"Ada\n", b"private-token\n", b"\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(!output.contains("private-token"), "{output}");

    write_pinned_prompt_entry(data.path());
    fs::write(
        config.path().join("config.toml"),
        concat!(
            "form = \"plain\"\n",
            "[prompt]\n",
            "runners_seeded = true\n",
            "[[prompt.runners]]\n",
            "name = \"backup\"\n",
            "argv = [\"printf\", \"{{prompt}}\"]\n",
            "[[prompt.runners]]\n",
            "name = \"local\"\n",
            "argv = [\"sh\", \"-c\", \"printf %s\", \"{{prompt}}\"]\n",
        ),
    )
    .unwrap();
    let (code, output) = run_plain_in_pty(
        &["run", "pinned-prompt", "--dry-run"],
        data.path(),
        state.path(),
        config.path(),
        &[b"\n\n\n\n\n"],
    );
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("Prompt runner choices: backup, local [local]:"),
        "{output}"
    );
}
