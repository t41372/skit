//! Mechanical port of the Python oracle module `tests/test_add_no_source.py`
//! (`origin/main@206f9ef`): issue #10, "make adding weird stuff intuitive" — a bare
//! `skit add` (no path) and an unclassifiable file ASK in a terminal instead of lecturing
//! about CLI flags, and still refuse honestly in a pipe / under `--no-input`. Each `#[test]`
//! keeps its Python `def test_*` name and its WHY comment, so it traces back to its origin.
//!
//! The oracle drives its private CLI helpers directly (`cli._add_no_source_ask`,
//! `cli._ask_kind_plain`, `cli._hosted_add_summary`, …) and monkeypatches `Prompt.ask`,
//! `Confirm.ask`, `skit.tui_add.run_*`, and the `_create_*`/`_print_add_summary` seams. None
//! of that is reachable from a black-box integration test. So this port observes the same
//! behavior end to end through the real `skit` binary:
//!
//! Concept mapping:
//! - Python `runner.invoke(cli.app, ["add", …])` with a NON-terminal stream -> `Sandbox::bin()`
//!   (a piped `std::process::Command` — its streams are never a tty, which IS the oracle's pipe).
//! - Python `monkeypatch.setattr(cli, "_is_interactive", lambda: True)` + monkeypatched
//!   `Prompt.ask`/`Confirm.ask` answers -> a real pty (`Sandbox::pty`) whose stdin bytes are the
//!   canned line answers; Rust's interactive gate is the actual `io::stdin().is_terminal()`
//!   (cli.rs:1059), so only a pty reaches `refuse_bare_add_flags` (cli.rs:1176) and
//!   `bare_add_plain` (cli.rs:1248).
//! - Python `capsys`/`result.output` -> the control-stripped pty stream (keep `\n`, drop ESC/`\r`),
//!   asserted with `contains`, not `in _lines(out)` line-vector equality.
//! - Python `config.save_form("plain")` -> `Sandbox::form("plain")` (Rust defaults to tui).
//! - Python `store.resolve(name).meta` -> `Sandbox::show_json(name)` (`skit show NAME --json`);
//!   `entry.meta.params` for a command -> `params --json` `"placeholders"`.
//! - Python `store.list_entries()` -> `skit list --json` (empty array == nothing landed).
//! - `_interpreted()` (the sorted interpreted kinds, prompt excluded) is baked in from
//!   `skit-oracle/src/skit/langs/registry.py` + `kindnames.kind_choices`:
//!   [fish, js, lua, perl, powershell, python, r, ruby, shell, ts] (10), so shell -> menu index 9,
//!   exe -> 11, prompt -> 12.
//!
//! Bucket disposition (68 Python defs -> 68 exact names across the workspace, plus 5 Rust
//! regressions for non-interactive refusal, invalid input, terminal cancellation, snapshot order,
//! and TUI separation):
//! - 36 REAL owners: 31 public integration tests in this file, including the eight plain
//!   unknown-kind PTY contracts, plus five typed selector owners in `src/cli/tests.rs`.
//! - 1 FRAMEWORK-CALL closure: Python captures private Typer `Prompt.ask` kwargs and console
//!   identity. Rust owns the observable question, legal choices, bracket, cancellation, and typed
//!   return mapping instead; it does not recreate Typer callback data.
//! - 31 cross-crate closures: the oracle calls a private skit-cli helper directly, or asserts the
//!   captured kwargs of a monkeypatched internal (`_create_*`, `_print_add_summary`,
//!   `_hosted_add_summary`, `_command_secret_names`, `_wants_tui_form`, `_cancelled_add`) or a
//!   skit-tui review/kind-pick seam (`run_kind_pick`/`run_exe_review`/`run_add_review`/
//!   `run_add_source`). No black-box call compiles against those; the owning tier is named per stub.

use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use serde_json::Value;

#[path = "support/temp_root.rs"]
mod temp_root;

use temp_root::TempRoot;

/// A fresh four-directory sandbox so skit writes only inside temp dirs, never the repo or cwd.
struct Sandbox {
    data: TempRoot,
    state: TempRoot,
    config: TempRoot,
    scratch: TempRoot,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempRoot::new(),
            state: TempRoot::new(),
            config: TempRoot::new(),
            scratch: TempRoot::new(),
        }
    }

    /// Python `config.save_form(value)` — Rust defaults to tui, so plain-branch tests set it.
    fn form(&self, value: &str) {
        fs::write(
            self.config.path().join("config.toml"),
            format!("form = {value:?}\n"),
        )
        .unwrap();
    }

    /// A non-terminal `skit` invocation — the oracle's pipe / `--no-input` lane.
    fn bin(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_skit"));
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// A `skit` invocation on a real pty — the oracle's forced-interactive terminal. `inputs` are
    /// the canned line answers the oracle fed through monkeypatched `Prompt.ask`/`Confirm.ask`.
    /// `answer_cursor` replies to a Ratatui cursor-position query (needed only for tui-form paths).
    fn pty(&self, args: &[&str], inputs: &[&[u8]], answer_cursor: bool) -> (u32, String) {
        self.pty_in_locale(args, inputs, answer_cursor, "en")
    }

    fn pty_in_locale(
        &self,
        args: &[&str],
        inputs: &[&[u8]],
        answer_cursor: bool,
        locale: &str,
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
        command.env("SKIT_LANG", locale);
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = reader.read_to_end(&mut bytes);
            bytes
        });
        let mut writer = pair.master.take_writer().unwrap();
        thread::sleep(Duration::from_millis(60));
        if answer_cursor {
            let _ = writer.write_all(b"\x1b[1;1R");
            let _ = writer.flush();
        }
        for bytes in inputs {
            thread::sleep(Duration::from_millis(140));
            if writer.write_all(bytes).is_err() {
                break;
            }
            let _ = writer.flush();
        }
        let status = child.wait().unwrap();
        drop(writer);
        let raw = drain.join().unwrap();
        // Python `capsys`/`result.output`: keep newlines, drop ESC/`\r` and other control bytes so
        // SGR/cursor residue cannot masquerade as text. `contains` on full phrases is unaffected.
        (status.exit_code(), terminal_text(&raw))
    }

    /// Wait for one real rendered prompt, change external state, then answer it.
    ///
    /// The synchronization makes a source-snapshot race deterministic: the source has already
    /// been inspected when the kind question appears, and the mutation happens before the answer
    /// can reach the add continuation.
    fn pty_after_output<F>(
        &self,
        args: &[&str],
        wait_for: &str,
        inputs: &[&[u8]],
        answer_cursor: bool,
        before_input: F,
    ) -> (portable_pty::ExitStatus, String)
    where
        F: FnOnce(),
    {
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
        command.env("SKIT_DATA_DIR", self.data.path());
        command.env("SKIT_STATE_DIR", self.state.path());
        command.env("SKIT_CONFIG_DIR", self.config.path());
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);

        let captured = Arc::new(Mutex::new(Vec::new()));
        let reader_capture = Arc::clone(&captured);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let drain = thread::spawn(move || {
            let mut chunk = [0_u8; 1024];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => reader_capture
                        .lock()
                        .unwrap()
                        .extend_from_slice(&chunk[..read]),
                }
            }
        });
        let mut writer = pair.master.take_writer().unwrap();
        if answer_cursor {
            writer.write_all(b"\x1b[1;1R").unwrap();
            writer.flush().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let early_status = loop {
            let shown = {
                let bytes = captured.lock().unwrap();
                terminal_text(bytes.as_slice())
            };
            if shown.contains(wait_for) {
                break None;
            }
            if let Some(status) = child.try_wait().unwrap() {
                break Some(status);
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {wait_for:?}: {shown}"
            );
            thread::sleep(Duration::from_millis(10));
        };

        if early_status.is_none() {
            before_input();
            for input in inputs {
                writer.write_all(input).unwrap();
                writer.flush().unwrap();
                thread::sleep(Duration::from_millis(80));
            }
        }
        let mut timed_out = false;
        let status = match early_status {
            Some(status) => status,
            None => {
                let exit_deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        break status;
                    }
                    if Instant::now() >= exit_deadline {
                        timed_out = true;
                        child.kill().unwrap();
                        break child.wait().unwrap();
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
        };
        drop(writer);
        drain.join().unwrap();
        let raw = captured.lock().unwrap().clone();
        let shown = terminal_text(&raw);
        assert!(!timed_out, "child did not exit after input: {shown}");
        (status, shown)
    }

    /// Python `store.resolve(name)` via `skit show NAME --json`.
    fn show_json(&self, name: &str) -> Value {
        let output = self
            .bin()
            .args(["show", name, "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "show --json failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document")
    }

    /// Python `entry.meta.params` for a command / prompt -> `params --json` `"placeholders"`.
    fn placeholders(&self, name: &str) -> Value {
        let output = self
            .bin()
            .args(["params", name, "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "params --json failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let document: Value =
            serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document");
        document["placeholders"].clone()
    }

    /// Python `store.list_entries()` via `skit list --json`.
    fn list_entries(&self) -> Vec<Value> {
        let output = self
            .bin()
            .args(["list", "--json"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success(), "list --json failed");
        serde_json::from_slice::<Value>(&output.stdout)
            .expect("stdout is exactly one JSON document")
            .as_array()
            .expect("list --json is an array")
            .clone()
    }
}

fn terminal_text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .collect()
}

/// Python `result.output` — the merged streams a CliRunner user sees (stdout then stderr).
fn combined(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::from_utf8_lossy(stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(stderr));
    text
}

/// Collapse whitespace so an assertion on a phrase is not broken by wrapping.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn snapshot_tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn visit(root: &Path, directory: &Path, output: &mut Vec<(PathBuf, Vec<u8>)>) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        for entry in entries {
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
    output.sort_by(|left, right| left.0.cmp(&right.0));
    output
}

type RootSnapshots = (
    Vec<(PathBuf, Vec<u8>)>,
    Vec<(PathBuf, Vec<u8>)>,
    Vec<(PathBuf, Vec<u8>)>,
);

fn root_snapshots(sandbox: &Sandbox) -> RootSnapshots {
    (
        snapshot_tree(sandbox.data.path()),
        snapshot_tree(sandbox.state.path()),
        snapshot_tree(sandbox.config.path()),
    )
}

// ---------------------------------------------------------------------------
// 1. Bare add, non-interactive: the honest lane list, never an ask.
// ---------------------------------------------------------------------------

#[test]
fn test_bare_add_no_input_lists_the_lanes() {
    // The message names ONLY the lanes that work under --no-input / in a pipe: the stdin
    // spellings and --cmd. It no longer recommends --edit/--prompt-with-editor.
    let sandbox = Sandbox::new();
    let output = sandbox
        .bin()
        .args(["add", "--no-input"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let out = combined(&output.stdout, &output.stderr);
    assert!(out.contains("Provide a source path"), "{out}");
    assert!(!out.contains("--edit"), "{out}");
    assert!(out.contains("--prompt"), "{out}");
    assert!(out.contains("--cmd"), "{out}");
    assert!(out.contains("skit add -"), "{out}");
    assert!(out.contains("-n NAME"), "{out}");
}

#[test]
fn test_bare_add_piped_lists_the_lanes() {
    // A pipe (no TTY) is non-interactive even without --no-input; `Sandbox::bin` IS that pipe,
    // so this needs no monkeypatch analogue.
    let sandbox = Sandbox::new();
    let output = sandbox
        .bin()
        .args(["add"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        combined(&output.stdout, &output.stderr).contains("Provide a source path"),
        "{}",
        combined(&output.stdout, &output.stderr)
    );
}

// ---------------------------------------------------------------------------
// 2. Bare add, interactive, with a flag that has nothing to attach to -> refused.
// ---------------------------------------------------------------------------

#[test]
fn test_bare_add_interactive_refuses_each_orphan_flag() {
    // On a terminal, a bare add carrying a flag that has nothing to attach to is a usage error
    // that names the withheld flag — `refuse_bare_add_flags` (cli.rs:1176), reached only when both
    // streams are terminals (a pty), which is the oracle's forced `_is_interactive` -> True.
    let cases: [(&[&str], &str); 9] = [
        (&["--name", "x"], "--name"),
        (&["--description", "d"], "--description"),
        (&["--ref"], "--ref"),
        (&["--exe"], "--exe"),
        (&["--kind", "shell"], "--kind"),
        (&["--runner", "claude"], "--runner"),
        (&["--dep", "rich"], "--dep"),
        (&["--python", ">=3.11"], "--python"),
        (&["--no-interpolate"], "--no-interpolate"),
    ];
    for (flag, shown) in cases {
        let sandbox = Sandbox::new();
        let mut args = vec!["add"];
        args.extend_from_slice(flag);
        let (code, output) = sandbox.pty(&args, &[], false);
        assert_eq!(code, 2, "{flag:?}: {output}");
        assert!(output.contains("need a source"), "{flag:?}: {output}");
        assert!(output.contains(shown), "{flag:?}: {output}");
    }
}

// ---------------------------------------------------------------------------
// 3. Bare add, plain menu (form=plain / TERM=dumb): the four lanes.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate: asserts the exact kwargs the choice-2 editor lane forwards (name=None, description=None, deps_opt=None, python_opt=None, no_input=False) by monkeypatching `cli._create_python_in_editor`. That blank-default wiring is a private skit-cli seam (bare_add_plain -> add_plain_draft, cli.rs:1294/1320); a black-box add cannot observe the forwarded kwargs."]
fn test_plain_menu_choice2_opens_the_python_editor_lane() {
    // Choice 2 routes to the editor lane with blank forwarded values (no flag can ride along).
    // The kwarg contract is unobservable end to end.
}

#[test]
#[ignore = "cross-crate: asserts the exact kwargs the choice-3 prompt-editor lane forwards (interpolate=True default, runner=None, no_input=False) by monkeypatching `cli._create_prompt_in_editor`. Private skit-cli seam (bare_add_plain -> add_plain_draft with DraftKind::Prompt, cli.rs:1295/1320); unobservable black-box."]
fn test_plain_menu_choice3_opens_the_prompt_editor_lane() {
    // Choice 3 routes to the prompt-editor lane with interpolate=True; the wiring is private.
}

#[test]
fn test_plain_menu_choice4_command_template_happy_path() {
    // Choice 4: template, name, description (the retry loop is gone) -> a command entry whose
    // {holes} are detected.
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(
        &["add"],
        &[b"4\n", b"ffmpeg -i {input}\n", b"encode\n", b"\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.show_json("encode")["kind"], "command");
    assert_eq!(sandbox.placeholders("encode"), serde_json::json!(["input"]));
    assert!(output.contains("Detected parameters: input"), "{output}");
}

#[test]
fn test_plain_menu_choice4_empty_template_cancels() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(&["add"], &[b"4\n", b"   \n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.to_lowercase().contains("nothing was added"),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
}

#[test]
fn test_plain_menu_choice4_empty_name_cancels() {
    // One cancellation rule: an empty NAME cancels (130), no retry loop.
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(&["add"], &[b"4\n", b"echo {x}\n", b"   \n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.to_lowercase().contains("nothing was added"),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
}

#[test]
fn test_plain_menu_choice4_stores_the_description() {
    // The Description (optional) ask lands on the command entry.
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(
        &["add"],
        &[b"4\n", b"echo {x}\n", b"shout\n", b"say it loud\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    let entry = sandbox.show_json("shout");
    assert_eq!(entry["kind"], "command");
    assert_eq!(entry["description"], "say it loud");
}

#[cfg(unix)]
#[test]
fn test_plain_menu_choice1_path_continues_into_a_real_add() {
    // Choice 1 hands the typed path back into a real add. An extensionless file with the exec bit
    // infers as exe (the slug is the stem), so "tool" resolves.
    use std::os::unix::fs::PermissionsExt as _;
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let exe = sandbox.scratch.path().join("tool");
    fs::write(&exe, "opaque bytes\n").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    let path_line = format!("{}\n", exe.display());
    let (code, output) = sandbox.pty(&["add"], &[b"1\n", path_line.as_bytes()], false);
    assert_eq!(code, 0, "{output}");
    assert_eq!(sandbox.show_json("tool")["kind"], "exe");
}

#[test]
fn test_plain_menu_choice1_empty_path_cancels() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(&["add"], &[b"1\n", b"\n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.to_lowercase().contains("nothing was added"),
        "{output}"
    );
}

// ---------------------------------------------------------------------------
// 4. Bare add, TUI form: the hosted source step.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate: monkeypatches `skit.tui_add.run_add_source` to return a pre-made slug, then asserts the summary prints its name. The success path drives the skit-tui add-source workflow (run_add_workflow, cli.rs:1116-1143) to completion; the summary-prints-name seam is exercised by the real pty workflow tests in terminal_pty.rs (bare_add_uses_the_typed_workflow_and_returns_the_created_entry). No stub-return analogue exists black-box."]
fn test_bare_add_tui_form_summary_on_success() {
    // A successful hosted source step is summarized by name; unobservable via a stub return here.
}

#[test]
fn test_bare_add_tui_form_cancel_exits_130() {
    // form=tui (default) bare add: cancelling the hosted source step (Esc, the oracle's
    // run_add_source -> None) exits 130 and stores nothing.
    let sandbox = Sandbox::new();
    let (code, output) = sandbox.pty(&["add"], &[b"\x1b"], true);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.to_lowercase().contains("nothing was added"),
        "{output}"
    );
}

// ---------------------------------------------------------------------------
// 5. Unknown-kind plain ask (_ask_kind_plain): contents, order, routing, cancel.
// ---------------------------------------------------------------------------

// The five direct helper/model owners live beside the private production
// `PlainKindSelector` in `src/cli/tests.rs`. The public layout and routing remain here.

// ---------------------------------------------------------------------------
// 6. Unknown-kind ask end to end: routing + picked-kind-rejoins-dispatch.
// ---------------------------------------------------------------------------

#[test]
fn test_unknown_plain_pick_language_adds_it() {
    // shell is menu index 9 among [fish, js, lua, perl, powershell, python, r, ruby, shell, ts].
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"echo hi\n";
    fs::write(&source, source_bytes).unwrap();
    let (_, state_before, config_before) = root_snapshots(&sandbox);
    let (code, output) = sandbox.pty(&["add", &source.to_string_lossy()], &[b"9\n"], false);
    assert_eq!(code, 0, "{output}");
    assert!(
        output.contains("What is mystery.xyz? skit can't tell from the name."),
        "{output}"
    );
    let shown = sandbox.show_json("mystery");
    assert_eq!(shown["kind"], "shell");
    assert_eq!(shown["mode"], "copy");
    assert_eq!(shown["source"], source.display().to_string());
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/mystery/script.sh")).unwrap(),
        source_bytes
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
}

#[test]
fn test_unknown_plain_pick_exe_adds_it() {
    // exe is menu index 11 (n=10 interpreted + 1).
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"some opaque text\n";
    fs::write(&source, source_bytes).unwrap();
    let (_, state_before, config_before) = root_snapshots(&sandbox);
    let (code, output) = sandbox.pty(&["add", &source.to_string_lossy()], &[b"11\n"], false);
    assert_eq!(code, 0, "{output}");
    let shown = sandbox.show_json("mystery");
    assert_eq!(shown["kind"], "exe");
    assert_eq!(shown["mode"], "reference");
    assert_eq!(shown["source"], source.display().to_string());
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    let entry_files = fs::read_dir(sandbox.data.path().join("scripts/mystery"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect::<Vec<_>>();
    assert_eq!(entry_files, [std::ffi::OsString::from("meta.toml")]);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
}

#[test]
fn test_unknown_plain_cancel_exits_130() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"some opaque text\n";
    fs::write(&source, source_bytes).unwrap();
    let roots_before = root_snapshots(&sandbox);
    let (code, output) = sandbox.pty(&["add", &source.to_string_lossy()], &[b"-\n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(root_snapshots(&sandbox), roots_before);
}

#[test]
fn test_unknown_plain_pick_language_with_runner_hits_prompt_only_refusal() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"echo hi\n";
    fs::write(&source, source_bytes).unwrap();
    let roots_before = root_snapshots(&sandbox);
    let (code, output) = sandbox.pty(
        &["add", &source.to_string_lossy(), "--runner", "claude"],
        &[b"9\n"],
        false,
    );
    assert_eq!(code, 2, "{output}");
    assert!(
        output.contains("--runner only applies to prompt entries"),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(root_snapshots(&sandbox), roots_before);
}

#[test]
fn test_unknown_plain_pick_prompt_runs_prompt_onboarding() {
    // prompt is menu index 12 (n=10 + 2); the runner ask answers '-' (no pin).
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"do {{thing}}\n";
    fs::write(&source, source_bytes).unwrap();
    let (_, state_before, config_before) = root_snapshots(&sandbox);
    let (code, output) = sandbox.pty(
        &["add", &source.to_string_lossy()],
        &[b"12\n", b"-\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    let shown = sandbox.show_json("mystery");
    assert_eq!(shown["kind"], "prompt");
    assert_eq!(shown["mode"], "copy");
    assert_eq!(shown["runner"], Value::Null);
    assert_eq!(shown["fields"][0]["key"], "thing");
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/mystery/prompt.md")).unwrap(),
        source_bytes
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
}

#[test]
fn test_unknown_plain_kept_draft_offers_no_program_option() {
    // A kept draft lives under skit's OWN drafts home; the drafts boundary forbids exe.
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let drafts = sandbox.data.path().join("drafts");
    fs::create_dir_all(&drafts).unwrap();
    let draft = drafts.join("skit-new-mystery");
    let draft_bytes = b"some opaque text\n";
    fs::write(&draft, draft_bytes).unwrap();
    let roots_before = root_snapshots(&sandbox);
    let (code, output) = sandbox.pty(&["add", &draft.to_string_lossy()], &[b"-\n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(!output.contains("A program (run it directly)"), "{output}");
    assert!(output.contains("A prompt for an AI agent"), "{output}");
    assert_eq!(fs::read(&draft).unwrap(), draft_bytes);
    assert_eq!(root_snapshots(&sandbox), roots_before);
}

// ---------------------------------------------------------------------------
// 7. Unknown-kind TUI form: the hosted kind ask.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate: monkeypatches `skit.tui_add.run_kind_pick` to return \"shell\" and captures its filename/has_shebang/offer_exe kwargs. That modal is skit-tui's KindPickModal (skit-tui/src/screens/add.rs); the CLI forwards to it through the hosted review workflow (cli.rs:1082-1092). The captured-kwarg contract is white-box to the skit-tui seam."]
fn test_unknown_tui_form_pick_routes_to_the_kind() {
    // The picked kind rejoins dispatch; the forwarded filename/offer_exe/has_shebang are private.
}

#[test]
#[ignore = "cross-crate: monkeypatches `skit.tui_add.run_kind_pick` -> None (cancel). The real cancel is an Esc in skit-tui's hosted review/kind-pick workflow (cli.rs:1082); a faithful drive needs the full Ratatui panel, exercised by terminal_pty.rs. The stubbed-return isolation has no black-box analogue."]
fn test_unknown_tui_form_cancel_exits_130() {
    // Cancelling the hosted kind pick exits 130 and stores nothing.
}

#[test]
#[ignore = "cross-crate: monkeypatches `skit.tui_add.run_kind_pick` and captures has_shebang=True for a shebang'd unknown file. The shebang-forwarding into the modal is a skit-tui seam (screens/add.rs:905-910); captured kwargs are unobservable black-box."]
fn test_unknown_tui_form_shebang_flag_forwarded() {
    // A #! body forwards has_shebang=True into the modal.
}

#[test]
#[ignore = "cross-crate: a bare .md under form=tui goes straight to `skit.tui_add.run_kind_pick` with suggested='prompt' and NO line Confirm. The suggested pre-highlight is a skit-tui KindPickModal seam; the captured `suggested` kwarg and the \"Confirm must not fire\" guard are white-box to that tier (cli.rs:1082 hosted workflow)."]
fn test_md_tui_form_passes_suggested_prompt() {
    // .md pre-highlights the prompt option in the modal, no line Confirm.
}

#[test]
#[ignore = "cross-crate: picking \"A program\" from the tui kind modal must host the SAME ExeReviewScreen the Library's `a` opens (never a line prompt). Monkeypatches `skit.tui_add.run_exe_review` and asserts the resolved source path reaches the panel. That panel and its routing are skit-tui (screens/add.rs); the captured path is white-box."]
fn test_unknown_tui_form_pick_exe_hosts_the_review_panel() {
    // The exe pick hosts the review panel with the resolved source forwarded intact.
}

#[test]
#[ignore = "cross-crate: monkeypatches `skit.tui_add.run_exe_review` -> None (Esc). Cancelling the hosted ExeReviewScreen exits 130; the panel is skit-tui, the stubbed-return isolation has no black-box analogue."]
fn test_unknown_tui_form_pick_exe_cancel_exits_130() {
    // Cancelling the hosted exe review exits 130 and stores nothing.
}

#[test]
#[ignore = "cross-crate: the explicit --exe lane under form=tui also hosts the review panel and prefills --name/--description. Monkeypatches `skit.tui_add.run_exe_review` and captures those prefills. The panel + prefill wiring is skit-tui (screens/add.rs) fed by add_review_defaults (cli.rs:1150); captured kwargs are white-box."]
fn test_exe_flag_tui_form_hosts_the_panel_and_prefills_flags() {
    // --exe under form=tui hosts the panel with name/description prefilled.
}

#[test]
#[ignore = "cross-crate: drives the hosted interpreted branch by monkeypatching `skit.tui_add.run_kind_pick` + `run_add_review` to return a shell entry whose stored copy carries a secret-marked decl, then asserts the summary prints \"Managed parameters\"/\"city\"/\"Secret parameter values are never saved\". The summary is skit-cli's private print_add_summary (cli.rs:3130-3141) over a skit-tui-produced entry; the stubbed review that plants the decl has no black-box analogue."]
fn test_hosted_interpreted_branch_prints_managed_and_secret_lines() {
    // The interpreted hosted branch reports managed decls + the secret subset.
}

#[test]
#[ignore = "cross-crate: the python hosted branch reports managed decls + secret subset + deps (via effective_uv_metadata). Monkeypatches `skit.tui_add.run_add_review` to return a pre-planted python entry. Same private print_add_summary seam (cli.rs:3130) over a stubbed skit-tui review; unobservable black-box."]
fn test_hosted_python_branch_prints_managed_and_secret_lines() {
    // The python hosted branch reports managed decls, secrets, and dependencies.
}

// ===========================================================================
// Mutation-kill battery: the extracted helpers are standalone and mutated.
// ===========================================================================

#[test]
#[ignore = "cross-crate: calls the private helper `cli._add_no_source_ask()` directly and captures `_create_python_in_editor` kwargs. The function is skit-cli-private (bare_add_plain, cli.rs:1248); its choice-2 blank-default wiring is not on the public binary surface."]
fn test_ans_choice2_python_lane_uses_blank_defaults() {
    // Choice 2 forwards blank defaults into the python editor lane.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` directly and captures `_create_prompt_in_editor` kwargs (interpolate=True). Private skit-cli helper (cli.rs:1248/1295); unobservable black-box."]
fn test_ans_choice3_prompt_lane_uses_blank_defaults() {
    // Choice 3 forwards blank defaults (interpolate=True) into the prompt editor lane.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and captures the args handed to `cli._print_add_summary` (resolved slug, deps from effective_uv_metadata, managed decls, decl.secret subset). Both the tui source step and the summary are private skit-cli/skit-tui seams (cli.rs:1116-1143, print_add_summary cli.rs:3095)."]
fn test_ans_tui_summary_receives_deps_params_and_secrets() {
    // The tui branch hands the summary deps/managed/secrets from the resolved entry.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._hosted_add_summary(entry)` directly and asserts (deps, managed, secrets) read from a script's stored [tool.skit] block (decl.secret honored, not is_secret_name). No public function returns this triple; the logic is skit-cli-private (cli.rs:1422)."]
fn test_hosted_add_summary_script_reads_decls_and_honors_decl_secret() {
    // params_io path: managed = every stored decl, secrets = the decl.secret subset.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._hosted_add_summary(entry)` on a prompt entry and asserts the meta.params + is_secret_name fallback. Private skit-cli helper (cli.rs:1436-1437); no public triple-returning surface."]
fn test_hosted_add_summary_prompt_falls_back_to_meta_and_name_heuristic() {
    // A prompt has no params_io, so managed mirrors meta.params and secrets = is_secret_name subset.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._hosted_add_summary(entry)` on a command entry and asserts the meta.params + name-heuristic fallback. Private skit-cli helper (cli.rs:1436-1437)."]
fn test_hosted_add_summary_command_uses_meta_fallback() {
    // A command reports meta.params holes as managed, name-heuristic subset as secrets.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and asserts the tui cancel prints exactly \"Cancelled — nothing was added.\" then raises Exit(130). The message+code is CliError::AddCancelled (cli.rs:7646); the outcome is covered end to end by test_bare_add_tui_form_cancel_exits_130, but this direct-call isolation is skit-cli-private."]
fn test_ans_tui_cancel_prints_exact_message_and_exits_130() {
    // The tui cancel prints the one cancel line and exits 130.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` with TERM=dumb + form=tui and asserts the AND short-circuits to the plain menu. The TERM=dumb rule is `wants_tui_form` (cli.rs:1241-1246); this direct-call mutation-kill is skit-cli-private."]
fn test_ans_term_dumb_forces_the_plain_menu_even_with_form_tui() {
    // TERM=dumb forces the plain line menu even under form=tui.
}

#[test]
fn test_ans_plain_menu_lines_are_exact() {
    // The plain menu's four printed lines, verbatim (a choice-1 + empty path cancels out).
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (_code, output) = sandbox.pty(&["add"], &[b"1\n", b"\n"], false);
    assert!(output.contains("What would you like to add?"), "{output}");
    assert!(
        output.contains("  1. A file you already have — a script, program, or prompt"),
        "{output}"
    );
    assert!(
        output.contains("  2. A new script, written in your editor"),
        "{output}"
    );
    assert!(
        output.contains("  3. A new AI-agent prompt, written in your editor"),
        "{output}"
    );
    assert!(
        output.contains("  4. A command template (e.g. ffmpeg -i {input})"),
        "{output}"
    );
}

#[test]
fn test_ans_choice4_reports_params_and_stores_description() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(
        &["add"],
        &[b"4\n", b"tpl {a} {b}\n", b"cmd4\n", b"a fine command\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    let entry = sandbox.show_json("cmd4");
    assert_eq!(entry["kind"], "command");
    assert_eq!(sandbox.placeholders("cmd4"), serde_json::json!(["a", "b"]));
    assert_eq!(entry["description"], "a fine command");
    assert!(
        output.contains(
            "Detected parameters: a, b (the run form asks for them; your last values are remembered)"
        ),
        "{output}"
    );
}

#[test]
fn test_ans_choice4_empty_template_cancels_with_exact_message() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(&["add"], &[b"4\n", b"   \n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
}

#[test]
fn test_ans_choice4_empty_name_cancels_with_exact_message() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(&["add"], &[b"4\n", b"echo {x}\n", b"  \n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
}

#[test]
fn test_ans_choice1_empty_path_cancels_with_exact_message() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(&["add"], &[b"1\n", b"  \n"], false);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and asserts its RETURN value is the stripped typed path (\"~/tool.py\"), which the caller continues the path lane with. The return value is an internal handoff (bare_add_plain calls add() itself, cli.rs:1282-1293); no public surface exposes it."]
fn test_ans_choice1_returns_the_typed_path() {
    // Choice 1 returns the stripped typed path for the caller's path lane.
}

// --- Real-prompt (rich) CLI tests: the prompt LABELS and choice lists print. ---

#[test]
fn test_cli_plain_choice4_prompt_labels_and_choices() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(
        &["add"],
        &[b"4\n", b"tpl {a} {b}\n", b"enc\n", b"\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    let stored = sandbox.show_json("enc");
    assert_eq!(stored["kind"], "command");
    assert_eq!(stored["template"], "tpl {a} {b}");
    let joined = flat(&output);
    assert!(joined.contains("Which one? [1/2/3/4] (1):"), "{joined}");
    assert!(joined.contains("Command template:"), "{joined}");
    assert!(joined.contains("Name for the command:"), "{joined}");
    assert!(joined.contains("Description (optional)"), "{joined}");
}

#[cfg(unix)]
#[test]
fn test_cli_plain_choice1_path_label() {
    // The "Path to the file:" label prints, and an inferred exe lands.
    use std::os::unix::fs::PermissionsExt as _;
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let exe = sandbox.scratch.path().join("tool");
    fs::write(&exe, "bytes\n").unwrap();
    fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    let path_line = format!("{}\n", exe.display());
    let (code, output) = sandbox.pty(
        &["add"],
        &[b"1\n", path_line.as_bytes(), b"\n", b"\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    assert!(flat(&output).contains("Path to the file:"), "{output}");
    assert_eq!(sandbox.show_json("tool")["kind"], "exe");
}

// --- _ask_kind_plain: exact question, options, cancel hint, choice list. ---

#[test]
fn test_cli_ask_kind_plain_full_layout() {
    let interpreted = [
        "fish",
        "JavaScript",
        "Lua",
        "Perl",
        "PowerShell",
        "Python",
        "R",
        "Ruby",
        "Shell",
        "TypeScript",
    ];
    for (locale, question, executable, prompt, cancel, which) in [
        (
            "en",
            "What is mystery.xyz? skit can't tell from the name.",
            "A program (run it directly)",
            "A prompt for an AI agent",
            "- = cancel",
            "Which one?",
        ),
        (
            "zh-CN",
            "mystery.xyz 是什么？skit 无法从名称判断。",
            "一个程序（直接运行）",
            "给 AI agent 的提示词",
            "- = 取消",
            "选哪个？",
        ),
        (
            "zh-TW",
            "mystery.xyz 是什麼？skit 無法從名稱判斷。",
            "一個程式（直接執行）",
            "給 AI agent 的提示詞",
            "- = 取消",
            "選哪個？",
        ),
    ] {
        let sandbox = Sandbox::new();
        sandbox.form("plain");
        let source = sandbox.scratch.path().join("mystery.xyz");
        fs::write(&source, "opaque text\n").unwrap();
        let roots_before = root_snapshots(&sandbox);
        let (code, output) = sandbox.pty_in_locale(
            &["add", &source.to_string_lossy()],
            &[b"-\n"],
            false,
            locale,
        );
        assert_eq!(code, 130, "locale={locale}: {output}");
        assert!(
            output.lines().any(|line| line.trim() == question),
            "locale={locale}: {output}"
        );
        for (index, label) in interpreted.iter().enumerate() {
            assert!(
                output
                    .lines()
                    .any(|line| line.trim() == format!("{}. {label}", index + 1)),
                "locale={locale}, label={label}: {output}"
            );
        }
        for (index, label) in [(11, executable), (12, prompt)] {
            assert!(
                output
                    .lines()
                    .any(|line| line.trim() == format!("{index}. {label}")),
                "locale={locale}, label={label}: {output}"
            );
        }
        assert!(
            output.lines().any(|line| line.trim() == cancel),
            "locale={locale}: {output}"
        );
        let flat = flat(&output);
        assert!(flat.contains(which), "locale={locale}: {output}");
        assert!(
            flat.contains("[1/2/3/4/5/6/7/8/9/10/11/12/-]"),
            "locale={locale}: {output}"
        );
        assert_eq!(root_snapshots(&sandbox), roots_before, "locale={locale}");
    }
}

#[test]
fn test_cli_ask_kind_plain_shebang_question() {
    for (locale, question) in [
        (
            "en",
            "The #! in mystery.xyz names no interpreter skit knows. What is it?",
        ),
        (
            "zh-CN",
            "mystery.xyz 的 #! 指定了 skit 不认识的解释器。这是什么?",
        ),
        (
            "zh-TW",
            "mystery.xyz 的 #! 指定了 skit 不認識的直譯器。這是什麼?",
        ),
    ] {
        let sandbox = Sandbox::new();
        sandbox.form("plain");
        let source = sandbox.scratch.path().join("mystery.xyz");
        fs::write(&source, "#!/usr/bin/env florblang\ncode\n").unwrap();
        let roots_before = root_snapshots(&sandbox);
        let (code, output) = sandbox.pty_in_locale(
            &["add", &source.to_string_lossy()],
            &[b"-\n"],
            false,
            locale,
        );
        assert_eq!(code, 130, "locale={locale}: {output}");
        assert!(
            output.lines().any(|line| line.trim() == question),
            "locale={locale}: {output}"
        );
        assert_eq!(root_snapshots(&sandbox), roots_before, "locale={locale}");
    }
}

#[test]
fn unknown_plain_kind_selector_reprompts_invalid_answers_without_writing() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"opaque text\n";
    fs::write(&source, source_bytes).unwrap();
    let roots_before = root_snapshots(&sandbox);

    let (code, output) = sandbox.pty(
        &["add", &source.to_string_lossy()],
        &[b"0\n", b"13\n", b"not-a-number\n", b"-\n"],
        false,
    );

    assert_eq!(code, 130, "{output}");
    assert!(
        output.matches("Choose a number from 1 to 12.").count() >= 3,
        "{output}"
    );
    assert!(
        output.contains("Cancelled — nothing was added."),
        "{output}"
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(root_snapshots(&sandbox), roots_before);
}

#[test]
fn unknown_kind_noninteractive_paths_refuse_without_rendering_a_selector_or_writing() {
    let run = |terminal: bool, no_input: bool| {
        let sandbox = Sandbox::new();
        sandbox.form("plain");
        let source = sandbox.scratch.path().join("mystery.xyz");
        let source_bytes = b"opaque text\n";
        fs::write(&source, source_bytes).unwrap();
        let roots_before = root_snapshots(&sandbox);
        let mut args = vec!["add", source.to_str().unwrap()];
        if no_input {
            args.push("--no-input");
        }
        let (code, output) = if terminal {
            sandbox.pty(&args, &[], false)
        } else {
            let output = sandbox
                .bin()
                .args(&args)
                .stdin(Stdio::null())
                .output()
                .unwrap();
            (
                output.status.code().unwrap_or_default() as u32,
                combined(&output.stdout, &output.stderr),
            )
        };

        assert_eq!(
            code, 2,
            "terminal={terminal}, no_input={no_input}: {output}"
        );
        assert!(
            output.contains("isn't a script or an executable"),
            "terminal={terminal}, no_input={no_input}: {output}"
        );
        assert!(
            !output.contains("Which one?") && !output.contains("- = cancel"),
            "terminal={terminal}, no_input={no_input}: {output}"
        );
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        assert_eq!(root_snapshots(&sandbox), roots_before);
    };

    run(false, false); // redirected/non-TTY streams
    run(true, true); // a real terminal whose --no-input contract forbids questions
}

#[cfg(unix)]
#[test]
fn unknown_plain_kind_selector_ctrl_c_cancels_without_writing() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"opaque text\n";
    fs::write(&source, source_bytes).unwrap();
    let roots_before = root_snapshots(&sandbox);

    let (status, output) = sandbox.pty_after_output(
        &["add", &source.to_string_lossy()],
        "What is mystery.xyz? skit can't tell from the name.",
        &[b"\x03"],
        false,
        || {},
    );

    // The host names the signal, and the two spellings differ: glibc gives "Interrupt", macOS gives
    // "Interrupt: 2". Both must still prove that SIGINT ended the child, not a normal exit.
    assert!(
        status
            .signal()
            .is_some_and(|name| name == "Interrupt" || name.starts_with("Interrupt:")),
        "{status:?}: {output}"
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(root_snapshots(&sandbox), roots_before);
}

#[test]
fn unknown_plain_kind_selector_uses_the_pre_question_source_snapshot() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let source = sandbox.scratch.path().join("mystery.xyz");
    let inspected = b"echo before\n";
    let changed = b"echo after\n";
    fs::write(&source, inspected).unwrap();
    let (_, state_before, config_before) = root_snapshots(&sandbox);

    let (status, output) = sandbox.pty_after_output(
        &["add", &source.to_string_lossy()],
        "What is mystery.xyz? skit can't tell from the name.",
        &[b"9\n"],
        false,
        || fs::write(&source, changed).unwrap(),
    );

    assert!(status.success(), "{status:?}: {output}");
    assert!(
        output.contains("What is mystery.xyz? skit can't tell from the name."),
        "{output}"
    );
    assert_eq!(
        fs::read(sandbox.data.path().join("scripts/mystery/script.sh")).unwrap(),
        inspected
    );
    assert_eq!(fs::read(&source).unwrap(), changed);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
}

#[test]
fn unknown_tui_form_keeps_kind_selection_inside_the_hosted_workflow() {
    let sandbox = Sandbox::new();
    let source = sandbox.scratch.path().join("mystery.xyz");
    let source_bytes = b"opaque text\n";
    fs::write(&source, source_bytes).unwrap();
    let roots_before = root_snapshots(&sandbox);

    let (status, output) = sandbox.pty_after_output(
        &["add", &source.to_string_lossy()],
        "What is mystery.xyz? skit can't tell from the name.",
        &[b"\x1b", b"\x1b"],
        true,
        || {},
    );

    assert_eq!(status.exit_code(), 130, "{status:?}: {output}");
    assert!(
        output.contains("What is mystery.xyz? skit can't tell from the name."),
        "the hosted kind stage never rendered: {output}"
    );
    assert!(
        !output.contains("- = cancel"),
        "line selector leaked into TUI: {output}"
    );
    assert!(
        !flat(&output).contains("[1/2/3/4/5/6/7/8/9/10/11/12/-]"),
        "line selector leaked into TUI: {output}"
    );
    assert_eq!(fs::read(&source).unwrap(), source_bytes);
    assert_eq!(root_snapshots(&sandbox), roots_before);
}

// --- Call contracts: capture Prompt.ask / store.add_command / _print_add_summary kwargs. ---

#[test]
#[ignore = "FRAMEWORK-CALL CLOSURE: the frozen body captures private Typer `Prompt.ask` kwargs and asserts `console is cli.console`. Rust uses dialoguer, so those callback values do not exist. The active public PTY layout owner pins `Which one?`, the exact `[1/../-]` choices and cancellation; the internal `PlainKindSelector` owners pin the same typed legal inputs and return mapping. Do not recreate Typer call data."]
fn test_ask_kind_plain_prompt_call_contract() {
    // Typer's callback identity is closed; public layout and typed parsing remain executable.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and captures the \"Which one?\" `Prompt.ask` args (choices=[1..4], default=1, console). Rust's dialoguer Input (cli.rs:1270-1278) has no captured-kwarg surface, and the rendered bracket already diverges (see test_cli_plain_choice4_prompt_labels_and_choices). Private skit-cli helper."]
fn test_ans_which_one_prompt_call_contract() {
    // The which-one prompt carries choices=[1..4], default="1".
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and captures the Command-template/Name/Description prompt args, the `store.add_command` args, and the `_print_add_summary(entry, [], [], secrets)` empty-list args. All are private skit-cli seams (cli.rs:1296-1315, print_add_summary cli.rs:3095); no captured-kwarg black-box surface."]
fn test_ans_choice4_call_contracts() {
    // The choice-4 lane's prompt/add_command/summary calls carry their exact args.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and captures the \"Path to the file\" `Prompt.ask` args. Private skit-cli helper (add_plain_text, cli.rs:1360); no captured-kwarg surface."]
fn test_ans_path_prompt_call_contract() {
    // The path prompt carries console=console.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and asserts no \"XX\" string-mutation marker leaks. A belt-and-braces mutation check on the private helper's rendered output; the plain menu render itself is asserted verbatim by test_ans_plain_menu_lines_are_exact."]
fn test_ans_no_stray_markup_tokens_in_output() {
    // No XX mutation marker leaks into the plain menu render.
}

// ---------------------------------------------------------------------------
// 9. Interactive directory adds: the --exe escape is COLLECTED, not taught.
// ---------------------------------------------------------------------------

#[test]
fn test_add_unknown_directory_plain_confirm_yes_adds_program() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let dir = sandbox.scratch.path().join("bundle.dir");
    fs::create_dir(&dir).unwrap();
    let directory_before = snapshot_tree(&dir);
    let (code, output) = sandbox.pty(
        &["add", &dir.to_string_lossy()],
        &[b"\n", b"toolname\n", b"a dir-shaped tool\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    let entry = sandbox.show_json("toolname");
    assert_eq!(entry["kind"], "exe");
    assert_eq!(entry["mode"], "reference");
    assert_eq!(
        entry["source"],
        dir.canonicalize().unwrap().display().to_string()
    );
    assert_eq!(entry["description"], "a dir-shaped tool");
    assert!(dir.is_dir());
    assert_eq!(snapshot_tree(&dir), directory_before);
    assert_eq!(
        snapshot_tree(&sandbox.data.path().join("scripts/toolname"))
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>(),
        [PathBuf::from("meta.toml")],
        "a reference directory entry must not acquire a stored payload",
    );
}

#[test]
fn test_add_unknown_directory_plain_confirm_no_cancels() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let dir = sandbox.scratch.path().join("bundle.dir");
    fs::create_dir(&dir).unwrap();
    let data_before = snapshot_tree(sandbox.data.path());
    let state_before = snapshot_tree(sandbox.state.path());
    let config_before = snapshot_tree(sandbox.config.path());
    let directory_before = snapshot_tree(&dir);
    let (code, output) = sandbox.pty(&["add", &dir.to_string_lossy()], &[b"n\n"], false);
    assert_eq!(snapshot_tree(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    assert_eq!(snapshot_tree(&dir), directory_before);
    assert_eq!(code, 130, "{output}");
    assert!(
        output.to_lowercase().contains("nothing was added"),
        "{output}"
    );
    assert!(sandbox.list_entries().is_empty());
}

#[test]
fn test_add_unknown_directory_plain_confirm_call_contract() {
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let dir = sandbox.scratch.path().join("bundle.dir");
    fs::create_dir(&dir).unwrap();
    let data_before = snapshot_tree(sandbox.data.path());
    let state_before = snapshot_tree(sandbox.state.path());
    let config_before = snapshot_tree(sandbox.config.path());
    let directory_before = snapshot_tree(&dir);

    let (code, output) = sandbox.pty(&["add", &dir.to_string_lossy()], &[b"n\n"], false);

    assert_eq!(snapshot_tree(sandbox.data.path()), data_before);
    assert_eq!(snapshot_tree(sandbox.state.path()), state_before);
    assert_eq!(snapshot_tree(sandbox.config.path()), config_before);
    assert_eq!(snapshot_tree(&dir), directory_before);
    assert_eq!(code, 130, "{output}");
    let shown = flat(&output);
    assert!(
        shown.contains("bundle.dir is a directory. Add it as a program that runs directly?"),
        "{shown}"
    );
    assert!(shown.contains("[Y/n]"), "{shown}");
    assert!(sandbox.list_entries().is_empty());
}

#[test]
#[ignore = "cross-crate: under form=tui the exe review panel IS the directory consent (Esc cancels), never a line Confirm. Monkeypatches `skit.tui_add.run_exe_review` and asserts the resolved directory reaches the panel. The panel and its no-line-Confirm routing are skit-tui (screens/add.rs) via the hosted workflow (cli.rs:1082); captured path is white-box."]
fn test_add_unknown_directory_tui_hosts_exe_review_with_no_line_confirm() {
    // form=tui routes a directory into the hosted exe review, not a line Confirm.
}

// ---------------------------------------------------------------------------
// 10. Command-template adds report the SAME trace through every door.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate: calls the private helper `cli._command_secret_names(entry)` directly on a stored command and asserts it returns the secret-looking {holes} ([\"API_KEY\"]). No public function returns that list; the never-saved caveat it feeds is observed end to end by test_cmd_flag_secret_hole_gets_never_saved_note. Private skit-cli helper (cli.rs:1414)."]
fn test_command_secret_names_picks_the_secret_holes() {
    // A template's secret-looking holes are its is_secret_name subset.
}

#[test]
fn test_cmd_flag_secret_hole_gets_never_saved_note() {
    // The --cmd door reports detected params AND the never-saved secrets caveat (a pipe lane).
    let sandbox = Sandbox::new();
    let output = sandbox
        .bin()
        .args(["add", "--cmd", "curl -H {API_KEY} {url}", "-n", "curler"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        combined(&output.stdout, &output.stderr)
    );
    let out = combined(&output.stdout, &output.stderr);
    assert!(out.contains("Detected parameters"), "{out}");
    assert!(
        out.contains("Secret parameter values are never saved"),
        "{out}"
    );
}

#[test]
fn test_plain_menu_choice4_secret_hole_gets_never_saved_note() {
    // The plain menu's choice-4 door reports the same detected-params + never-saved caveat.
    let sandbox = Sandbox::new();
    sandbox.form("plain");
    let (code, output) = sandbox.pty(
        &["add"],
        &[b"4\n", b"deploy {AUTH_TOKEN}\n", b"deployer\n", b"\n"],
        false,
    );
    assert_eq!(code, 0, "{output}");
    assert!(output.contains("Detected parameters"), "{output}");
    assert!(
        output.contains("Secret parameter values are never saved"),
        "{output}"
    );
}

#[test]
#[ignore = "cross-crate: monkeypatches `skit.tui_add.run_add_source` to return a pre-made command slug, then asserts the tui bare-add door reports it exactly like --cmd (\"Detected parameters\" + never-saved, and NEVER a \"Managed parameters\" spelling). The tui source step is skit-tui (cli.rs:1116); the stubbed-return isolation has no black-box analogue."]
fn test_bare_add_tui_command_door_matches_the_cmd_door() {
    // The tui command door reports like the --cmd door, never a second Managed-parameters line.
}

#[test]
fn test_bare_add_refusal_names_only_lanes_that_honor_the_flag() {
    // The lane advice never teaches a guaranteed second refusal: a recommended lane honors EVERY
    // withheld flag (--ref/--exe/--kind fit none; --dep/--python only --edit; --runner/
    // --no-interpolate only --prompt; -n/-d fit all three).
    let cases: [(&[&str], Option<&str>); 9] = [
        (&["--ref"], None),
        (&["--exe"], None),
        (&["--kind", "shell"], None),
        (&["--dep", "rich"], Some("--edit")),
        (&["--python", ">=3.11"], Some("--edit")),
        (&["--runner", "claude"], Some("--prompt")),
        (&["--no-interpolate"], Some("--prompt")),
        (&["--name", "x"], Some("--edit, --prompt, --cmd")),
        (&["--description", "d"], Some("--edit, --prompt, --cmd")),
    ];
    for (flag, advice) in cases {
        let sandbox = Sandbox::new();
        let mut args = vec!["add"];
        args.extend_from_slice(flag);
        let (code, output) = sandbox.pty(&args, &[], false);
        assert_eq!(code, 2, "{flag:?}: {output}");
        let joined = flat(&output);
        match advice {
            None => assert!(!joined.contains("pick a lane"), "{flag:?}: {joined}"),
            Some(advice) => assert!(
                joined.contains(&format!(
                    "pick a lane outright with {advice} (nothing was added)"
                )),
                "{flag:?}: {joined}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// 11. The shared helpers, pinned directly.
// ---------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate: calls the private `cli._wants_tui_form()` directly across the (form, TERM) matrix. Rust's twin is `wants_tui_form(config_dir)` (cli.rs:1241-1246), private to skit-cli; TERM=dumb forcing plain is observed indirectly by the plain-menu pty tests. No public surface returns the boolean."]
fn test_wants_tui_form_matrix() {
    // TERM=dumb forces plain regardless of form; otherwise the config decides.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._cancelled_add()` directly and asserts it prints exactly \"Cancelled — nothing was added.\" then Exit(130). Rust's twin is CliError::AddCancelled (cli.rs:7646/7678); the line+code is observed end to end by every pty cancel test above. No public callable surface."]
fn test_cancelled_add_exact_line_and_exit_code() {
    // The one cancel exit: the dim note then 130.
}

#[test]
#[ignore = "cross-crate: calls the private `cli._add_no_source_ask()` and captures the tui command door's `_print_add_summary(entry, [], [], secrets)` args (empty deps/managed, exactly the secret holes). Private skit-cli/skit-tui seam (cli.rs:1461-1462, print_add_summary cli.rs:3095); no captured-kwarg black-box surface."]
fn test_bare_add_tui_command_door_summary_call_contract() {
    // The command door hands the summary empty deps/managed and exactly the secret holes.
}
