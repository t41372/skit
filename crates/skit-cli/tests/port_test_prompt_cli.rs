//! Mechanical port of the Python oracle module `tests/test_prompt_cli.py`
//! (`origin/main@206f9ef`): "The prompt kind's CLI surfaces: add lanes, run resolution,
//! params ops, show, the `skit runner` tree, and doctor's prompt sweeps." Each `#[test]`
//! keeps its Python `def test_*` name and its WHY comment, so it traces back to its origin.
//!
//! WHY `skit-cli`: the oracle drives the whole prompt CLI through Typer's `CliRunner`
//! (`skit.cli.app`) — add/run/params/show/runner/doctor/edit. Only the composition-root crate
//! runs the real `skit` binary end to end. These tests drive it through `assert_cmd`, with
//! `SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR` pinned to a per-test `TempDir` and
//! `SKIT_LANG=en` on every invocation.
//!
//! OBSERVABLE MAPPING (the `port_test_run_set.rs` precedent — the oracle's `spawn_spy`
//! monkeypatch of `launcher.run_entry` -> a black-box binary). A black-box port cannot read
//! the launcher kwargs, so each spy field maps to a real, independent witness:
//! - `spawn_spy["values"]` (the substituted values) -> a fake runner on `PATH` whose binary
//!   captures its own argv to `$SKIT_CAP`; the rendered prompt (with the value substituted) is
//!   read back from that capture file, out of band from skit's own stdout.
//! - `spawn_spy["runner"] == config.find_prompt_runner(name)` -> the run actually spawns that
//!   runner's binary; combined with the `state/prompt.toml` `last_runner` witness for a "pick".
//! - `spawn_spy["extra"]` (the argv tail) -> captured argv in `$SKIT_CAP` too.
//! - `argstate.load_last_runner()` -> `state/prompt.toml`'s `last_runner` key, read as text.
//! - `"entry" not in spawn_spy` (a refusal: the launcher was never called) -> a non-zero refusal
//!   exit before any spawn (nothing captured).
//! - the `--dry-run` cases need no binary: the resolved argv / masks / warnings print directly.
//!
//! Output stream: `CliRunner.result.output` merges stdout+stderr, so substring assertions run
//! against the CONCATENATION of both streams (`combined`). The `--json` purity contract
//! (`_json`) is the exception: the whole of STDOUT must parse as exactly one JSON document.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API EXISTS, behavior reachable black-box).
//! - FAILING CONTRACT (divergence): the full asserting body is kept intact and `#[ignore]`d with
//!   the observed-vs-oracle evidence; deleting the `#[ignore]` after the impl is fixed turns it
//!   green. Never softened to match Rust output.
//! - UNMAPPED (cross-crate): the observable itself needs an interactive/internal seam a non-tty
//!   binary cannot drive or intercept — the `Prompt.ask`/`Confirm.ask` answers, the `$EDITOR`
//!   *interactive* reconcile, the `tui_add` panels/pickers, `inlineform.collect`, the
//!   `PromptLaunch._read_body` / `prepare_entry` TOCTOU seams, `config.gettext` wrapping, or a
//!   store-fault injection. Compiling `#[ignore]` stub naming the seam (`src/cli/tests.rs`) and
//!   its non-interactive twin when one exists.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

// AUTO_MANAGE_LIMIT (langs/prompt/analyzer.py:40) — above this many detections nothing is
// auto-managed.
const AUTO_MANAGE_LIMIT: usize = 30;
// LIST_PREVIEW_LIMIT (langs/prompt/analyzer.py:41) — how many candidate names any list prints.
const LIST_PREVIEW_LIMIT: usize = 20;
// render.ARGV_LIMIT on POSIX (langs/prompt/render.py:34).
const ARGV_LIMIT: usize = 100_000;

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

    /// The oracle's `runner.invoke(cli.app, …)`: the real `skit` binary with all three roots
    /// pinned under the sandbox and the locale fixed to English.
    fn command(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// (exit code, stdout+stderr concatenated) — the merged-stream view CliRunner.output gives.
    fn out(&self, args: &[&str]) -> (i32, String) {
        let output = self.command().args(args).output().unwrap();
        let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
        (output.status.code().unwrap_or(-1), combined)
    }

    /// Assert success and return the merged output.
    fn ok(&self, args: &[&str]) -> String {
        let (code, combined) = self.out(args);
        assert_eq!(code, 0, "args={args:?}\n{combined}");
        combined
    }

    /// Whole-STDOUT-as-one-JSON — the `_json` purity contract.
    fn json(&self, args: &[&str]) -> Value {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|error| panic!("args={args:?}: stdout is not one JSON doc: {error}"))
    }

    /// Write a byte-exact input file under the data dir; return its absolute path as a string.
    fn write_file(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.data.path().join(name);
        fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
    }

    fn config_path(&self) -> PathBuf {
        self.config.path().join("config.toml")
    }

    fn set_config(&self, toml: &str) {
        fs::create_dir_all(self.config.path()).unwrap();
        fs::write(self.config_path(), toml).unwrap();
    }

    fn read_config(&self) -> String {
        fs::read_to_string(self.config_path()).unwrap_or_default()
    }

    /// The oracle's `argstate.save_last_runner` — seed `state/prompt.toml`.
    fn set_last_runner(&self, name: &str) {
        fs::create_dir_all(self.state.path()).unwrap();
        fs::write(
            self.state.path().join("prompt.toml"),
            format!("last_runner = {name:?}\n"),
        )
        .unwrap();
    }

    /// The oracle's `argstate.load_last_runner()` — read `state/prompt.toml`'s `last_runner`.
    fn last_runner(&self) -> String {
        let text = fs::read_to_string(self.state.path().join("prompt.toml")).unwrap_or_default();
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("last_runner") {
                let value = rest.trim_start_matches([' ', '=']).trim();
                return value.trim_matches('"').to_owned();
            }
        }
        String::new()
    }

    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    fn meta(&self, slug: &str) -> String {
        fs::read_to_string(self.entry_dir(slug).join("meta.toml")).unwrap_or_default()
    }

    /// The stored body file bytes (the single non-`meta.toml` file in the entry dir).
    fn body_bytes(&self, slug: &str) -> Option<Vec<u8>> {
        let dir = self.entry_dir(slug);
        for entry in fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            let name = entry.file_name();
            if name != std::ffi::OsStr::new("meta.toml") && entry.path().is_file() {
                return fs::read(entry.path()).ok();
            }
        }
        None
    }

    /// The oracle's `_added(text, name)` — a managed prompt entry. Add through the real lane so
    /// the auto-manage happens exactly as the CLI does it. `.prompt.md` infers the prompt kind.
    fn added(&self, text: &str, name: &str) {
        let path = self.write_file(&format!("{name}.prompt.md"), text.as_bytes());
        self.ok(&["add", &path, "-n", name, "--no-input"]);
    }

    /// `_added(..., pin=…)` — a managed prompt entry with a runner pin (a configured seed).
    fn added_pin(&self, text: &str, name: &str, pin: &str) {
        self.added(text, name);
        self.ok(&["params", name, "--runner", pin]);
    }
}

/// A directory of fake agent binaries for every seed's argv[0] (plus a few named extras). Each
/// binary captures its own argv into `$SKIT_CAP` (one line per token) and prints nothing, so a
/// real prompt run can be observed without the rendered body leaking onto skit's own stdout.
fn tools_dir() -> TempDir {
    let dir = TempDir::new().unwrap();
    let script = "#!/bin/sh\nif [ -n \"$SKIT_CAP\" ]; then for a in \"$@\"; do printf '%s\\n' \
                  \"$a\" >> \"$SKIT_CAP\"; done; fi\nexit 0\n";
    for binary in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "agy",
        "copilot",
        "cursor-agent",
        "pi",
        "mycli",
        "mine",
    ] {
        let path = dir.path().join(binary);
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// An executable `$EDITOR` script that appends `text` to the file it is given — the black-box
/// stand-in for the oracle's `_editor_appending` / `open_entry_in_editor` monkeypatch.
fn appending_editor(dir: &Path, text: &str) -> PathBuf {
    let editor = dir.join("skit-fake-editor.sh");
    fs::write(
        &editor,
        format!(
            "#!/bin/sh\nprintf '%s' {} >> \"$1\"\n",
            shell_single_quote(text)
        ),
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    editor
}

fn shell_single_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

// ==========================================================================
// add
// ==========================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): the asserted observable — a monkeypatched `prompt.text.read` raising PermissionError(13, 'permission denied') and the exact 'permission denied' string it injects — needs the read seam intercepted (src/cli/tests.rs). A real chmod-000 read is root-unsafe and yields the OS's own 'Permission denied' wording. The clean-store-error contract for an OSError-on-read that still 'exists' has a black-box twin: test_add_prompt_unreadable_file_is_a_store_error (a directory .prompt.md → 'Not a file')."]
fn test_add_prompt_read_oserror_is_a_clean_store_error() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): asserts the cli-internal `cli._starter_prompt()` localized text per locale — a private composition-root helper with no public seam (src/cli.rs); unit-driven by src/cli/tests.rs. The `placeholder_names` half belongs to skit-language's own port."]
fn test_localized_starter_is_minimal_and_never_creates_its_own_field() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): name derivation. `add p.prompt.md` (no -n): oracle slug 'p' (store.py:571 removesuffix '.prompt'); Rust slug 'p-prompt', so `show p` is 'entry not found'. The 'Managed parameters: target, focus' line itself converges."]
fn test_add_prompt_file_no_input_manages_everything() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file(
        "p.prompt.md",
        b"# Review\n\nCheck {{target}} for {{focus}}\n",
    );
    let combined = sandbox.ok(&["add", &src, "--no-input"]);
    let show = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(show["kind"], "prompt");
    assert_eq!(show["fields"].as_array().unwrap().len(), 2);
    let keys: Vec<&str> = show["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["target", "focus"]);
    assert_eq!(show["runner"], "");
    assert!(
        combined.contains("Managed parameters: target, focus"),
        "{combined}"
    );
}

#[test]
fn test_add_prompt_secret_summary_states_both_sides_of_boundary() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"Use {{api_key}}\n");
    let combined = sandbox.ok(&["add", &src, "--no-input"]);
    assert!(
        combined.contains("never saved by skit: api_key"),
        "{combined}"
    );
    assert!(
        combined.contains("selected agent receives those values as plaintext"),
        "{combined}"
    );
    assert!(combined.contains("may log or sync them"), "{combined}");
}

#[test]
fn test_add_prompt_interactive_tick_subset_and_runner_pick() {
    // The oracle's `if False` branch pins the NON-interactive default (CliRunner has no TTY);
    // the interactive tick path is covered by the cross-crate stubs below.
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"{{a}} {{b}} {{c}}\n");
    sandbox.ok(&["add", &src, "-n", "picky", "--no-input"]);
    let show = sandbox.json(&["show", "picky", "--json"]);
    let keys: Vec<&str> = show["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["a", "b", "c"]);
}

#[test]
#[ignore = "UNMAPPED (cross-crate): drives the interactive line-prompt tick selection via monkeypatched cli.Prompt.ask answers ('', '1,3', '-'); a non-tty binary never opens the picker. Seam: src/cli/tests.rs. Non-interactive default twin: test_add_prompt_interactive_tick_subset_and_runner_pick."]
fn test_add_prompt_interactive_selection() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): interactive identity defaults via monkeypatched cli.Prompt.ask; not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_add_prompt_plain_identity_defaults_drop_compound_suffix() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): interactive identity overrides via monkeypatched cli.Prompt.ask answers; not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_add_prompt_plain_identity_accepts_user_overrides() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): interactive runner pick+remember via monkeypatched cli.Prompt.ask; the pick persists last_runner only through the interactive picker a non-tty binary never opens. Seam: src/cli/tests.rs."]
fn test_add_prompt_interactive_runner_pick_pins_and_remembers() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is that the form=tui add hosts skit.tui_add.run_prompt_review with the flags as prefills; that Textual panel seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_add_prompt_interactive_tui_form_opens_review_panel() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): Esc in the tui_add review panel → exit 130; the panel-cancel seam (skit.tui_add.run_prompt_review) is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_add_prompt_interactive_panel_cancel_exits_130() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the unique observable is that the tui_add panel must NOT open for a bad --runner; the panel seam is not reachable black-box. Non-interactive twin (exit 2 'Unknown runner'): test_add_prompt_unknown_runner_flag_is_usage_error."]
fn test_add_prompt_unknown_runner_refused_before_the_panel() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): asserts TERM=dumb keeps the line-prompt lane instead of the tui_add panel, driven by monkeypatched cli.Prompt.ask; the panel/line-prompt fork is an interactive seam. Seam: src/cli/tests.rs."]
fn test_add_prompt_term_dumb_keeps_line_prompts() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is that the tui_add panel must NOT open for a missing file (pytest.fail inside the panel); the panel seam is not reachable black-box. The clean 'File not found' refusal is a black-box twin of test_missing_bare_md_is_refused_before_the_prompt_confirmation."]
fn test_add_prompt_missing_file_is_clean_on_the_panel_face() {}

#[test]
fn test_add_prompt_runner_flag_non_interactive() {
    let sandbox = Sandbox::new();
    sandbox.set_last_runner("opencode");
    let src = sandbox.write_file("p.prompt.md", b"{{a}}\n");
    sandbox.ok(&[
        "add",
        &src,
        "-n",
        "auto",
        "--runner",
        " claude ",
        "--no-input",
    ]);
    let show = sandbox.json(&["show", "auto", "--json"]);
    assert_eq!(show["runner"], "claude"); // the flag is trimmed
    assert_eq!(sandbox.last_runner(), "opencode"); // an add-time pin is not a picker choice
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches. Oracle prints 'Unknown runner'; Rust prints 'prompt runner \"ghost\" is not configured'."]
fn test_add_prompt_unknown_runner_flag_is_usage_error() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"{{a}}\n");
    let (code, combined) =
        sandbox.out(&["add", &src, "-n", "x", "--runner", "ghost", "--no-input"]);
    assert_eq!(code, 2, "{combined}");
    assert!(combined.contains("Unknown runner"), "{combined}");
}

#[test]
fn test_add_runner_flag_without_prompt_is_refused() {
    let sandbox = Sandbox::new();
    let py = sandbox.write_file("s.py", b"print(1)\n");
    let (code, combined) = sandbox.out(&["add", &py, "--runner", "claude", "--no-input"]);
    assert_eq!(code, 2, "{combined}");
    assert!(
        combined.contains("--runner only applies to prompt entries"),
        "{combined}"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches. Oracle prints 'drop --edit/--exe/--kind/--cmd'; Rust uses a clap conflict ('the argument --prompt cannot be used with --exe')."]
fn test_add_prompt_conflicts_with_other_kind_flags() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"{{a}}\n");
    for flags in [
        vec!["--exe"],
        vec!["--kind", "shell"],
        vec!["--edit"],
        vec!["--cmd", "echo {x}"],
    ] {
        let mut args = vec!["add", &src, "--prompt"];
        args.extend(flags.iter().copied());
        let (code, combined) = sandbox.out(&args);
        assert_eq!(code, 2, "{flags:?}: {combined}");
        assert!(
            combined.contains("drop --edit/--exe/--kind/--cmd"),
            "{flags:?}: {combined}"
        );
    }
}

#[test]
fn test_add_prompt_flag_forces_the_kind_on_any_extension() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("notes.txt", b"Do {{thing}}\n");
    sandbox.ok(&["add", &src, "--prompt", "--no-input"]);
    assert_eq!(sandbox.json(&["show", "notes", "--json"])["kind"], "prompt");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches, wording differs. A bare .md with --no-input: oracle names the fix '--prompt'; Rust prints the generic 'could not infer the entry kind; pass --kind KIND' (no .md-specific --prompt hint)."]
fn test_add_bare_md_no_input_requires_explicit_prompt() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("notes.md", b"hello {{x}}\n");
    let (code, combined) = sandbox.out(&["add", &src, "--no-input"]);
    assert_eq!(code, 2, "{combined}");
    assert!(combined.contains("--prompt"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 1 matches. Oracle prints 'File not found:'; Rust prints the raw 'could not resolve <path>: No such file or directory (os error 2)'."]
fn test_missing_bare_md_is_refused_before_the_prompt_confirmation() {
    // Black-box: a path that does not exist is refused before any kind question at all.
    let sandbox = Sandbox::new();
    let missing = sandbox.data.path().join("missing.md");
    let (code, combined) = sandbox.out(&["add", missing.to_str().unwrap(), "--no-input"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("File not found:"), "{combined}");
    assert!(combined.contains("missing.md"), "{combined}");
}

#[test]
fn test_executable_lane_preserves_the_existing_non_file_contract() {
    let sandbox = Sandbox::new();
    let directory = sandbox.data.path().join("tool-dir");
    fs::create_dir(&directory).unwrap();
    sandbox.ok(&["add", directory.to_str().unwrap(), "--exe", "--no-input"]);
    let show = sandbox.json(&["show", "tool-dir", "--json"]);
    assert_eq!(show["kind"], "exe");
    assert_eq!(show["source_hash"], "");
}

#[test]
#[ignore = "UNMAPPED (cross-crate): drives the interactive .md-prompt Confirm.ask (yes then no) plus the follow-on kind ask, via monkeypatched cli.Confirm.ask / cli.Prompt.ask; a non-tty binary never asks. Seam: src/cli/tests.rs."]
fn test_add_bare_md_interactive_ask_yes_and_no() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): 'No' to the .md question falls through to the interactive kind picker whose choice is honored, via monkeypatched cli.Confirm.ask / cli.Prompt.ask; not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_add_bare_md_confirm_no_falls_through_to_kind_ask_and_honors_pick() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle exit 2 requiring '--name' for a stdin prompt with no name; Rust instead ADDS an entry named 'stdin' (exit 0, 'Added: stdin (copy mode)') rather than refusing."]
fn test_add_prompt_from_stdin_needs_a_name() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "--prompt"])
        .write_stdin("body {{x}}\n")
        .output()
        .unwrap();
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(2), "{combined}");
    assert!(combined.contains("--name"), "{combined}");
}

#[test]
fn test_add_prompt_from_stdin() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "-", "--prompt", "-n", "clip", "--runner", "amp"])
        .write_stdin("Summarize {{url}} briefly.\n")
        .assert()
        .success();
    let show = sandbox.json(&["show", "clip", "--json"]);
    assert_eq!(show["kind"], "prompt");
    assert_eq!(show["runner"], "amp");
    let keys: Vec<&str> = show["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["url"]);
    assert_eq!(
        sandbox.body_bytes("clip").unwrap(),
        b"Summarize {{url}} briefly.\n"
    );
}

#[test]
fn test_add_kind_prompt_from_stdin_uses_the_prompt_contract() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args([
            "add",
            "-",
            "--kind",
            "prompt",
            "-n",
            "kind-clip",
            "--runner",
            "amp",
            "--no-interpolate",
        ])
        .write_stdin("Keep {{url}} literal.\r\n")
        .assert()
        .success();
    let show = sandbox.json(&["show", "kind-clip", "--json"]);
    assert_eq!(show["kind"], "prompt");
    assert_eq!(show["runner"], "amp");
    assert_eq!(show["interpolate"], false);
    assert_eq!(show["workdir"], "invoke");
    assert!(show["fields"].as_array().unwrap().is_empty()); // interpolation off → no fields
    assert_eq!(
        sandbox.body_bytes("kind-clip").unwrap(),
        b"Keep {{url}} literal.\r\n"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle exit 1 'Nothing arrived on stdin' for a whitespace-only stdin body; Rust ADDS the entry (exit 0, 'Added: e (copy mode)') instead of refusing an empty body."]
fn test_add_prompt_from_stdin_empty_body() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "--prompt", "-n", "e"])
        .write_stdin("  \n")
        .output()
        .unwrap();
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    assert_eq!(output.status.code(), Some(1), "{combined}");
    assert!(combined.contains("Nothing arrived on stdin"), "{combined}");
}

#[test]
fn test_add_prompt_editor_lane_routes_to_stdin_when_not_interactive() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "-n", "drafted"])
        .write_stdin("Draft {{a}}\n")
        .assert()
        .success();
    let keys: Vec<String> = sandbox.json(&["show", "drafted", "--json"])["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(keys, ["a"]);
}

#[test]
#[ignore = "UNMAPPED (cross-crate): drives the interactive prompt-editor lane via monkeypatched cli.editor.open_in_editor + cli.Prompt.ask; a non-tty binary routes --prompt to stdin instead of opening $EDITOR. Seam: src/cli/tests.rs. Non-interactive twin: test_add_prompt_editor_lane_routes_to_stdin_when_not_interactive."]
fn test_add_prompt_editor_lane_interactive() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): an untouched $EDITOR starter adds nothing; the interactive editor lane (cli.editor.open_in_editor) is not opened by a non-tty binary. Seam: src/cli/tests.rs."]
fn test_add_prompt_editor_lane_untouched_starter_adds_nothing() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive editor lane asks for a name via cli.Prompt.ask; a non-tty binary never opens it. Seam: src/cli/tests.rs."]
fn test_add_prompt_editor_lane_asks_for_a_name() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): asserts the name-conflict is caught BEFORE $EDITOR opens in the interactive editor lane (monkeypatched cli.editor.open_in_editor must not run); not reachable from a non-tty binary. Non-interactive twin: test_add_prompt_stdin_lane_reports_store_errors ('already taken')."]
fn test_add_prompt_editor_lane_name_taken_refuses_before_the_editor() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): a post-edit failure keeps the temp draft; drives the interactive editor lane plus a monkeypatched cli._onboard_prompt fault. Seam: src/cli/tests.rs."]
fn test_add_prompt_editor_lane_post_edit_failure_keeps_the_draft() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): a deleted draft after the interactive edit is a clean 'Can't read' failure; drives the interactive editor lane (cli.editor.open_in_editor) a non-tty binary never opens. Seam: src/cli/tests.rs."]
fn test_add_prompt_editor_lane_deleted_draft_is_a_clean_honest_failure() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): name derivation. `add p.prompt.md --ref` (no -n): oracle slug 'p' (store.py:571 removesuffix '.prompt'); Rust slug 'p-prompt', so `show p` is 'entry not found'."]
fn test_add_prompt_ref_mode_keeps_original_and_pins_invoke() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"Ref {{x}}\n");
    sandbox.ok(&["add", &src, "--ref", "--no-input"]);
    let show = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(show["mode"], "reference");
    assert_eq!(show["workdir"], "invoke");
    // A reference entry keeps the original in place — no copied body under the entry dir.
    assert!(sandbox.body_bytes("p").is_none(), "{}", sandbox.meta("p"));
    assert!(sandbox.meta("p").contains(&src), "{}", sandbox.meta("p"));
}

#[test]
fn test_add_prompt_no_path_with_ref_is_refused() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "--prompt", "--ref", "-n", "x"])
        .write_stdin("b\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

// ==========================================================================
// run
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the umbrella --help taxonomy wording differs — Rust 'A script, prompt, program, and command library' vs oracle 'scripts, prompts, programs, and commands' — and clap-generated help does not carry the oracle's per-command zh-TW phrasings."]
fn test_umbrella_cli_help_uses_entry_taxonomy_in_the_requested_locale() {
    // Parametrized (en, zh-TW): the umbrella help uses the entry taxonomy in the child's locale.
    let cases: [(&str, [(&str, &str); 9]); 2] = [
        (
            "en",
            [
                ("--help", "scripts, prompts, programs, and commands"),
                ("list", "registered entry"),
                ("show", "one entry"),
                ("remove", "registered entry"),
                ("rename", "Rename an entry"),
                ("describe", "entry's description"),
                ("params", "an entry's managed or declared parameters"),
                ("deps", "an entry's package dependencies"),
                ("doctor", "entry library"),
            ],
        ),
        (
            "zh-TW",
            [
                ("--help", "腳本、提示詞、程式和命令"),
                ("list", "已登記的條目"),
                ("show", "一個條目"),
                ("remove", "已登記的條目"),
                ("rename", "重新命名條目"),
                ("describe", "條目的說明"),
                ("params", "條目的管理參數或宣告參數"),
                ("deps", "條目的套件依賴"),
                ("doctor", "工具庫"),
            ],
        ),
    ];
    for (locale, probes) in cases {
        for (command, phrase) in probes {
            let sandbox = Sandbox::new();
            let args: Vec<&str> = if command == "--help" {
                vec!["--help"]
            } else {
                vec![command, "--help"]
            };
            let output = sandbox
                .command()
                .env("SKIT_LANG", locale)
                .args(&args)
                .output()
                .unwrap();
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                output.status.code(),
                Some(0),
                "{locale} {command}: {combined}"
            );
            let flat = combined.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(flat.contains(phrase), "{locale} {command}: {combined}");
        }
    }
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): Rust human doctor prints 'Entries: 1', never the oracle's taxonomy-aware '1 entry registered'."]
fn test_prompt_only_library_uses_entry_taxonomy_on_dynamic_cli_surfaces() {
    let sandbox = Sandbox::new();
    sandbox.added("Review this\n", "p");
    let combined = sandbox.ok(&["doctor"]);
    assert!(combined.contains("1 entry registered"), "{combined}");
    assert!(!combined.contains("script registered"), "{combined}");
}

#[test]
fn test_empty_library_does_not_claim_it_only_accepts_scripts() {
    let sandbox = Sandbox::new();
    let combined = sandbox.ok(&["list"]);
    assert!(combined.contains("No entries yet"), "{combined}");
    assert!(!combined.contains("No scripts yet"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 126 matches. Oracle says 'No runner selected'; Rust says 'prompt runner is required'."]
fn test_run_prompt_no_input_without_pin_is_126() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let (code, combined) = sandbox.out(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(code, 126, "{combined}");
    assert!(combined.contains("No runner selected"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 126 matches. Oracle says 'No runner selected'; Rust says 'prompt runner is required'."]
fn test_run_no_input_is_provably_unaffected_by_last_picked_state() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    sandbox.set_last_runner("claude");
    let (code, combined) = sandbox.out(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(code, 126, "{combined}");
    assert!(combined.contains("No runner selected"), "{combined}");
}

#[test]
fn test_run_prompt_runner_flag_threads_through() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    let (code, combined) = {
        let output = sandbox
            .command()
            .env("PATH", tools.path())
            .env("SKIT_CAP", &cap)
            .args([
                "run",
                "p",
                "--runner",
                " claude ",
                "--set",
                "a=1",
                "--no-input",
            ])
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    };
    assert_eq!(code, 0, "{combined}");
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(captured.contains("Do 1"), "captured={captured:?}"); // values threaded through
    assert_eq!(sandbox.last_runner(), "claude"); // --runner is a pick
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the Rust prompt analyzer does not detect a unicode placeholder name — `{{目标}}` yields empty fields, so `--set 目标=…` fails 'unknown parameter in --set: 目标'. Ties to task #14 (prompt analyzer defects)."]
fn test_run_prompt_unicode_placeholder_threads_through_set() {
    let sandbox = Sandbox::new();
    sandbox.added("审查 {{目标}}\n", "p");
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    let output = sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_CAP", &cap)
        .args([
            "run",
            "p",
            "--runner",
            "claude",
            "--set",
            "目标=src/app.py",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(captured.contains("src/app.py"), "captured={captured:?}");
}

#[test]
fn test_run_prompt_pin_resolves_without_touching_last_picked() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "codex");
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    let output = sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_CAP", &cap)
        .args(["run", "p", "--set", "a=1", "--no-input"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(captured.contains("Do 1"), "captured={captured:?}");
    assert_eq!(sandbox.last_runner(), ""); // using a pin is not a pick
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 126 matches and 'ghost' is named, but Rust does not list the AVAILABLE runner names (no 'claude') — it prints only 'prompt runner \"ghost\" is not configured'."]
fn test_run_prompt_unknown_runner_is_126_listing_names() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let (code, combined) = sandbox.out(&[
        "run",
        "p",
        "--runner",
        "ghost",
        "--set",
        "a=1",
        "--no-input",
    ]);
    assert_eq!(code, 126, "{combined}");
    assert!(combined.contains("ghost"), "{combined}");
    assert!(combined.contains("claude"), "{combined}");
}

#[test]
fn test_run_prompt_pinned_but_removed_runner_is_126() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    sandbox.ok(&["runner", "add", "mine", "--", "mine", "{{prompt}}"]);
    sandbox.ok(&["params", "p", "--runner", "mine"]);
    // The pin's row is gone.
    sandbox.set_config("[prompt]\nrunners_seeded = true\nrunners = []\n");
    let (code, combined) = sandbox.out(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(code, 126, "{combined}");
    assert!(combined.contains("mine"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 126 matches, but with an empty runner list Rust still prints 'prompt runner is required' instead of the oracle's 'No agents are configured' + the copyable 'skit runner add mycli -- mycli run {{prompt}}' recovery."]
fn test_run_unpinned_prompt_with_empty_runner_list_teaches_a_copyable_recovery() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    sandbox.set_config("[prompt]\nrunners_seeded = true\nrunners = []\n");
    let (code, combined) = sandbox.out(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(code, 126, "{combined}");
    assert!(combined.contains("No agents are configured"), "{combined}");
    let flat = combined.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("skit runner add mycli -- mycli run {{prompt}}"),
        "{combined}"
    );
}

#[test]
fn test_run_runner_flag_on_non_prompt_is_usage_error() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "echo {m}", "-n", "cmd", "--no-input"]);
    let (code, combined) = sandbox.out(&[
        "run",
        "cmd",
        "--runner",
        "claude",
        "--set",
        "m=1",
        "--no-input",
    ]);
    assert_eq!(code, 2, "{combined}");
    assert!(
        combined.contains("--runner only applies to prompt entries"),
        "{combined}"
    );
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive run picker is prefilled from last-picked via monkeypatched cli.Prompt.ask (seen['default']); a non-tty binary never opens the run form. Seam: src/cli/tests.rs (cli.rs run form)."]
fn test_run_prompt_interactive_ask_prefilled_from_last_picked() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the inline run form (skit.inlineform.collect) is prefilled with the last-configured pick when the pin is stale; that interactive seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_run_prompt_inline_stale_pin_prefills_last_configured_pick() {}

#[test]
fn test_run_prompt_dry_run_prints_the_resolved_argv() {
    let sandbox = Sandbox::new();
    sandbox.added("Say {{a}}!\n", "p");
    let (code, combined) = sandbox.out(&[
        "run",
        "p",
        "--runner",
        "claude",
        "--set",
        "a=hello world",
        "--no-input",
        "--dry-run",
    ]);
    assert_eq!(code, 0, "{combined}");
    assert!(combined.contains("claude"), "{combined}");
    assert!(combined.contains("hello world"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle exits 127 'doesn't exist' when the prompt body was deleted; Rust cannot resolve the entry and exits 2 'invalid entry mutation: copy entry has no stored payload'."]
fn test_run_prompt_dry_run_missing_body_is_127_before_output() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Say it\n", "p", "claude");
    // Delete the stored body.
    let dir = sandbox.entry_dir("p");
    for entry in fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != std::ffi::OsStr::new("meta.toml") {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    let (code, combined) = sandbox.out(&["run", "p", "--no-input", "--dry-run"]);
    assert_eq!(code, 127, "{combined}");
    assert!(combined.contains("doesn't exist"), "{combined}");
    assert!(!combined.contains('→'), "{combined}");
}

#[test]
fn test_normal_prompt_transparency_omits_body_but_keeps_agent_flags() {
    let sandbox = Sandbox::new();
    let body = format!(
        "PRIVATE-DOCUMENT-START\n{}{{{{a}}}}\n",
        "detail ".repeat(2_000)
    );
    sandbox.added_pin(&body, "p", "claude");
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    let output = sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_CAP", &cap)
        .args([
            "run",
            "p",
            "--set",
            "a=done",
            "--no-input",
            "--",
            "--model",
            "opus",
        ])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{combined}");
    assert!(!combined.contains("PRIVATE-DOCUMENT-START"), "{combined}");
    assert!(combined.contains("rendered prompt omitted"), "{combined}");
    assert!(combined.contains("claude"), "{combined}");
    assert!(combined.contains("--model"), "{combined}");
    assert!(combined.contains("opus"), "{combined}");
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(
        captured.contains("done"),
        "captured did not carry the value"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 125 matches, marker never leaks. Oracle says 'over this platform'; Rust says 'the rendered prompt makes the command line N bytes; the limit is 100000 bytes'."]
fn test_overlong_prompt_refuses_before_normal_transparency() {
    let sandbox = Sandbox::new();
    let marker = "MUST-NOT-REACH-SCROLLBACK";
    let body = format!("{marker}{}", "x".repeat(ARGV_LIMIT + 1));
    sandbox.added_pin(&body, "p", "claude");
    let (code, combined) = sandbox.out(&["run", "p", "--no-input"]);
    assert_eq!(code, 125, "len={}", combined.len());
    assert!(!combined.contains(marker), "marker leaked");
    assert!(combined.contains("over this platform"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 125 matches. Oracle says 'NUL byte'; Rust says 'the rendered prompt contains a NUL character'."]
fn test_dry_run_refuses_nul_without_looking_up_agent_binary() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("before\u{0}after", "p", "claude");
    // Nothing on PATH — a lookup would fail loudly; the NUL refusal must precede it.
    let output = sandbox
        .command()
        .env("PATH", "")
        .args(["run", "p", "--no-input", "--dry-run"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(125), "{combined}");
    assert!(combined.contains("NUL byte"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 125 matches, marker never leaks. Oracle says 'over this platform'; Rust says 'the rendered prompt makes the command line N bytes; the limit is 100000 bytes'."]
fn test_dry_run_refuses_overlong_prompt_without_printing_it() {
    let sandbox = Sandbox::new();
    let marker = "DRY-RUN-TOO-LONG";
    let body = format!("{marker}{}", "x".repeat(ARGV_LIMIT + 1));
    sandbox.added_pin(&body, "p", "claude");
    let (code, combined) = sandbox.out(&["run", "p", "--no-input", "--dry-run"]);
    assert_eq!(code, 125, "len={}", combined.len());
    assert!(!combined.contains(marker), "marker leaked");
    assert!(combined.contains("over this platform"), "{combined}");
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is that the SAME body snapshot dry-run validated is the one printed, proved by monkeypatching skit.langs.launch.PromptLaunch._read_body to change between reads (reads == 1). That TOCTOU read seam is not interceptable from a black-box binary. Seam: src/cli/tests.rs / launch read path."]
fn test_dry_run_prints_the_same_prompt_snapshot_it_validated() {}

#[test]
fn test_run_prompt_extra_args_pass_through_after_dashes() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "claude");
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_CAP", &cap)
        .args([
            "run",
            "p",
            "--set",
            "a=1",
            "--no-input",
            "--",
            "--model",
            "opus",
        ])
        .assert()
        .success();
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(captured.contains("--model"), "captured={captured:?}");
    assert!(captured.contains("opus"), "captured={captured:?}");
}

#[test]
fn test_run_prompt_reuses_last_extra_agent_args() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "claude");
    let tools = tools_dir();
    let run = |cap: &Path, tail: &[&str]| {
        let mut args = vec!["run", "p", "--set", "a=1", "--no-input"];
        args.extend(tail.iter().copied());
        let output = sandbox
            .command()
            .env("PATH", tools.path())
            .env("SKIT_CAP", cap)
            .args(&args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        fs::read_to_string(cap).unwrap_or_default()
    };
    let first = run(&tools.path().join("c1.txt"), &["--", "--model", "opus"]);
    assert!(
        first.contains("--model") && first.contains("opus"),
        "{first:?}"
    );
    // Next run passes no tail: the remembered agent flags come back.
    let second = run(&tools.path().join("c2.txt"), &[]);
    assert!(
        second.contains("--model") && second.contains("opus"),
        "{second:?}"
    );
    // An explicit tail still wins over the remembered one.
    let third = run(&tools.path().join("c3.txt"), &["--", "--model", "sonnet"]);
    assert!(third.contains("sonnet"), "{third:?}");
    assert!(!third.contains("opus"), "{third:?}");
}

#[test]
fn test_prompt_extra_agent_args_do_not_fill_required_placeholders() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "claude");
    let (code, combined) = sandbox.out(&["run", "p", "--no-input", "--", "--model", "opus"]);
    assert_eq!(code, 125, "{combined}");
    assert!(combined.contains("a is required"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 125 matches. Oracle says 'whole number'; Rust says 'parameter \"count\" has invalid Int value \"nope\"' (same tier as port_test_run_set.rs)."]
fn test_extra_argv_does_not_hide_a_filled_flag_type_error() {
    let sandbox = Sandbox::new();
    let source = sandbox.write_file(
        "count.py",
        b"import argparse\np = argparse.ArgumentParser()\np.add_argument('--count', type=int, required=True)\np.parse_args()\n",
    );
    sandbox.ok(&[
        "add",
        &source,
        "-n",
        "count",
        "--kind",
        "python",
        "--no-input",
    ]);
    let output = sandbox
        .command()
        .env("PATH", "")
        .args([
            "run",
            "count",
            "--set",
            "count=nope",
            "--no-input",
            "--",
            "--count",
            "2",
        ])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(125), "{combined}");
    assert!(combined.contains("whole number"), "{combined}");
}

#[test]
fn test_run_prompt_secret_placeholder_masked_in_dry_run() {
    let sandbox = Sandbox::new();
    sandbox.added("Use {{api_key}}\n", "sec");
    let (code, combined) = sandbox.out(&[
        "run",
        "sec",
        "--runner",
        "claude",
        "--set",
        "api_key=hunter2",
        "--no-input",
        "--dry-run",
    ]);
    assert_eq!(code, 0, "{combined}");
    assert!(!combined.contains("hunter2"), "{combined}");
    assert!(combined.contains("•••"), "{combined}");
    assert!(!combined.contains("receives"), "{combined}"); // dry-run sends nothing
}

#[test]
fn test_real_prompt_run_warns_before_sending_a_nonempty_secret() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Use {{api_key}}\n", "sec", "claude");
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    let output = sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_CAP", &cap)
        .args(["run", "sec", "--set", "api_key=hunter2", "--no-input"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{combined}");
    assert!(combined.contains("never saved by skit"), "{combined}");
    assert!(
        combined.contains("selected agent as plaintext"),
        "{combined}"
    );
    assert!(combined.contains("may log or sync"), "{combined}");
    assert!(!combined.contains("hunter2"), "{combined}");
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(captured.contains("hunter2"), "the secret reaches the agent");
}

#[test]
fn test_noninteractive_pi_run_warns_and_uses_lossy_fallback() {
    // Parametrized over four bodies pi would misinterpret.
    for text in ["--help\nsecond line", "@README.md", "install", "config"] {
        let sandbox = Sandbox::new();
        sandbox.added_pin(text, "p", "pi");
        let tools = tools_dir();
        let cap = tools.path().join("cap.txt");
        let output = sandbox
            .command()
            .env("PATH", tools.path())
            .env("SKIT_CAP", &cap)
            .args(["run", "p", "--no-input"])
            .output()
            .unwrap();
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.status.code(), Some(0), "{text:?}: {combined}");
        assert!(
            combined.contains("Warning: Pi would interpret"),
            "{text:?}: {combined}"
        );
        assert!(
            combined.contains("prepended one newline"),
            "{text:?}: {combined}"
        );
        // The single argv token pi receives is the body with a leading newline.
        let captured = fs::read_to_string(&cap).unwrap_or_default();
        assert!(
            captured.starts_with('\n'),
            "{text:?}: captured={captured:?}"
        );
    }
}

#[test]
fn test_noninteractive_pi_dry_run_warns_and_shows_fallback() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("--help", "p", "pi");
    let (code, combined) = sandbox.out(&["run", "p", "--dry-run", "--no-input"]);
    assert_eq!(code, 0, "{combined}");
    assert!(
        combined.contains("Warning: Pi would interpret"),
        "{combined}"
    );
    assert!(combined.contains("one character longer"), "{combined}");
    assert!(combined.contains("\n--help"), "{combined}");
}

#[test]
fn test_missing_runner_binary_refuses_before_any_delivery_output() {
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\n[[prompt.runners]]\nname = \"missing\"\nargv = [\"definitely-not-installed\", \"{{prompt}}\"]\n",
    );
    sandbox.added("Use {{api_key}}\n", "sec");
    sandbox.ok(&["params", "sec", "--runner", "missing"]);
    let output = sandbox
        .command()
        .env("PATH", "")
        .args(["run", "sec", "--set", "api_key=hunter2", "--no-input"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(126), "{combined}");
    assert!(combined.contains("definitely-not-installed"), "{combined}");
    assert!(
        !combined.contains("selected agent as plaintext"),
        "{combined}"
    );
    assert!(!combined.contains('→'), "{combined}");
    assert!(!combined.contains("hunter2"), "{combined}");
}

#[test]
#[ignore = "UNMAPPED (cross-crate): proves the spawned body is the SAME snapshot dry-run/validate read, by monkeypatching skit.langs.launch.PromptLaunch._read_body to change between reads (reads == 1). The TOCTOU read seam is not interceptable from a black-box binary. Seam: src/cli/tests.rs / launch read path."]
fn test_real_run_spawns_the_same_prompt_snapshot_it_validated() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): monkeypatches cli.launcher.prepare_entry to swap the runner config AFTER prepare returns, proving the transparency + amp note use the PREPARED row, not a re-read. That prepare seam is not interceptable from a black-box binary. Seam: src/cli/tests.rs."]
fn test_real_run_transparency_and_amp_note_use_the_prepared_runner_row() {}

// ==========================================================================
// params
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the Rust human `params` read view omits the unmanaged/gone listing entirely (only 'Parameter: a / Type / Delivery / Interpolation'); the oracle's 'Prompt placeholders', 'Detected but not yet managed: b, c', and 'No longer in the prompt' are absent. The --json 'unmanaged' array carries the data."]
fn test_params_read_view_shows_unmanaged_and_gone() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}} {{b}}\n", "p"); // auto-manages a, b
    sandbox.ok(&["params", "p", "--rm", "b"]); // managed → [a]
    // The stored body file may have any name; overwrite whatever body file exists.
    overwrite_body(&sandbox, "p", "{{b}} {{c}} only\n");
    let combined = sandbox.ok(&["params", "p"]);
    assert!(combined.contains("Prompt placeholders"), "{combined}");
    assert!(
        combined.contains("Detected but not yet managed: b, c"),
        "{combined}"
    );
    assert!(combined.contains("No longer in the prompt"), "{combined}");
    assert!(combined.contains('a'), "{combined}");
}

/// Overwrite the stored body file (whatever its name) under an entry dir.
fn overwrite_body(sandbox: &Sandbox, slug: &str, text: &str) {
    let dir = sandbox.entry_dir(slug);
    for entry in fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != std::ffi::OsStr::new("meta.toml") && entry.path().is_file() {
            fs::write(entry.path(), text).unwrap();
            return;
        }
    }
}

#[test]
fn test_params_json_carries_runner_and_unmanaged() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("{{a}} {{b}}\n", "p", "claude"); // auto-manages a, b
    sandbox.ok(&["params", "p", "--rm", "b"]); // managed → [a]
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], serde_json::json!(["a"]));
    assert_eq!(payload["unmanaged"], serde_json::json!(["b"]));
    assert_eq!(payload["runner"], "claude");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): CLI flag semantics. Oracle `params --add b` MANAGES the body placeholder (placeholders -> [a,b], delivery placeholder). Rust `--add` DECLARES a new param with delivery 'flag' (placeholders stays [a]); managing a placeholder is Rust's separate `--manage`."]
fn test_params_add_manages_a_body_placeholder() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}} {{b}}\n", "p");
    sandbox.ok(&["params", "p", "--rm", "b"]); // managed → [a]
    sandbox.ok(&["params", "p", "--add", "b"]);
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], serde_json::json!(["a", "b"])); // body order
}

#[test]
fn test_params_rm_unmanages_even_without_a_declared_row() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}} {{b}}\n", "p");
    let combined = sandbox.ok(&["params", "p", "--rm", "b"]);
    assert_eq!(
        sandbox.json(&["params", "p", "--json"])["placeholders"],
        serde_json::json!(["a"])
    );
    assert!(!combined.contains("not-declared"), "{combined}");
    assert!(!combined.contains("isn't declared"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle `params --add EXTRA` declares an env rider (delivery 'env'); Rust `--add` declares delivery 'flag' instead."]
fn test_params_add_unknown_name_becomes_env_rider() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}}\n", "p");
    sandbox.ok(&["params", "p", "--add", "EXTRA"]);
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["placeholders"], serde_json::json!(["a"])); // not a body hole
    let deliveries: Vec<&str> = payload["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["delivery"].as_str().unwrap())
        .collect();
    assert_eq!(deliveries, ["env"]);
}

#[test]
fn test_params_deliver_placeholder_is_allowed_on_prompts() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}}\n", "p"); // a is a managed placeholder
    // The oracle seeds a DECLARED row with delivery "env" first, so `--deliver a=placeholder` has
    // real work to do — otherwise it would pass even if --deliver were a no-op (a placeholder's
    // default delivery is already "placeholder").
    let delivery = |sandbox: &Sandbox| -> String {
        sandbox.json(&["params", "p", "--json"])["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"] == "a")
            .expect("a is declared")["delivery"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    sandbox.ok(&["params", "p", "--deliver", "a=env"]);
    assert_eq!(delivery(&sandbox), "env"); // the seed took
    sandbox.ok(&["params", "p", "--deliver", "a=placeholder"]);
    assert_eq!(delivery(&sandbox), "placeholder"); // --deliver actually changed it
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): pin/clear and last_runner preservation converge, but clearing prints the human 'Prompt runner: not set' where the oracle prints 'asks at run time'."]
fn test_params_runner_pin_and_clear() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    sandbox.set_last_runner("opencode");
    sandbox.ok(&["params", "p", "--runner", "claude"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["runner"], "claude");
    assert_eq!(sandbox.last_runner(), "opencode"); // a settings pin is not a run pick
    let combined = sandbox.ok(&["params", "p", "--runner", ""]);
    // The oracle's `meta.runner == ""` (unpinned); an unpinned prompt renders a null runner.
    assert_eq!(
        sandbox.json(&["show", "p", "--json"])["runner"],
        Value::Null
    );
    assert_eq!(sandbox.last_runner(), "opencode");
    assert!(combined.contains("asks at run time"), "{combined}");
}

#[test]
fn test_params_runner_pin_with_json_emits_the_read_view() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let payload = sandbox.json(&["params", "p", "--runner", "claude", "--json"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["runner"], "claude"); // the pin was written
    assert_eq!(payload["runner"], "claude"); // the pin shows in the emitted read view
}

#[test]
fn test_params_workdir_with_json_emits_the_read_view() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let payload = sandbox.json(&["params", "p", "--workdir", "origin", "--json"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["workdir"], "origin"); // written
    assert!(payload.get("params").is_some(), "{payload}"); // the entry's read view
    assert_eq!(payload["runner"], Value::Null); // unpinned prompt renders a null runner
}

#[test]
fn test_params_interpolate_with_json_emits_the_read_view() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let payload = sandbox.json(&["params", "p", "--no-interpolate", "--json"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["interpolate"], false); // flipped
    assert_eq!(payload["interpolate"], false); // shows in the read view
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): Rust exits 2 (oracle exits 1) and prints 'prompt runner \"ghost\" is not configured' where the oracle prints 'isn't configured'. The pin stays cleared in both."]
fn test_params_runner_pin_validates_the_name() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let (code, combined) = sandbox.out(&["params", "p", "--runner", "ghost"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("isn't configured"), "{combined}");
    assert_eq!(sandbox.json(&["show", "p", "--json"])["runner"], "");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): message '--runner only applies to prompt entries' converges, but Rust exits 2 (CliError::Usage) where the oracle exits 1."]
fn test_params_runner_pin_refused_on_non_prompt() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "echo {m}", "-n", "cmd", "--no-input"]);
    let (code, combined) = sandbox.out(&["params", "cmd", "--runner", "claude"]);
    assert_eq!(code, 1, "{combined}");
    assert!(
        combined.contains("--runner only applies to prompt entries"),
        "{combined}"
    );
}

// ==========================================================================
// show
// ==========================================================================

#[test]
fn test_show_json_prompt_additions() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "claude");
    let payload = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(payload["kind"], "prompt");
    assert_eq!(payload["runner"], "claude");
    assert!(
        payload["runners_available"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "claude")
    );
    assert_eq!(payload["workdir"], "invoke");
    let keys: Vec<&str> = payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys, ["a"]);
    let sources: Vec<&str> = payload["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["source"].as_str().unwrap())
        .collect();
    assert_eq!(sources, ["placeholder"]);
}

#[test]
fn test_show_json_non_prompt_has_no_runner_keys() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "echo {m}", "-n", "cmd", "--no-input"]);
    let payload = sandbox.json(&["show", "cmd", "--json"]);
    assert!(payload.get("runner").is_none());
    assert!(payload.get("runners_available").is_none());
}

#[test]
fn test_show_human_prints_the_runner_line() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "claude");
    let combined = sandbox.ok(&["show", "p"]);
    assert!(combined.contains("Runner: claude"), "{combined}");
    sandbox.ok(&["params", "p", "--runner", ""]);
    let combined = sandbox.ok(&["show", "p"]);
    assert!(combined.contains("asks at run time"), "{combined}");
}

#[test]
fn test_show_human_no_fields_names_prompt_and_command_receivers() {
    let sandbox = Sandbox::new();
    sandbox.added("No fields\n", "plain");
    let prompt_view = sandbox.ok(&["show", "plain"]);
    assert!(
        prompt_view.contains("arguments after -- go to the selected agent"),
        "{prompt_view}"
    );
    assert!(
        !prompt_view.contains("pass straight through to the script"),
        "{prompt_view}"
    );
    sandbox.ok(&["add", "--cmd", "echo ready", "-n", "cmd", "--no-input"]);
    let command_view = sandbox.ok(&["show", "cmd"]);
    assert!(
        command_view.contains("arguments after -- are appended to the command"),
        "{command_view}"
    );
}

// ==========================================================================
// skit runner …
// ==========================================================================

#[test]
fn test_runner_list_materializes_the_seeds() {
    let sandbox = Sandbox::new();
    assert!(!sandbox.read_config().contains("runners_seeded = true"));
    let combined = sandbox.ok(&["runner", "list"]);
    assert!(sandbox.read_config().contains("runners_seeded = true")); // first need seeded config
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        assert!(combined.contains(name), "{name}: {combined}");
    }
    assert!(combined.contains("amp -x"), "{combined}");
    let flat = combined.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("does not open an interactive session"),
        "{combined}"
    );
}

#[test]
fn test_runner_list_json() {
    let sandbox = Sandbox::new();
    let payload = sandbox.json(&["runner", "list", "--json"]);
    let rows = payload.as_array().unwrap();
    let has = |name: &str, argv: Value| {
        rows.iter()
            .any(|row| row["name"] == name && row["argv"] == argv)
    };
    assert!(has(
        "claude",
        serde_json::json!(["claude", "--", "{{prompt}}"])
    ));
    assert!(has(
        "opencode",
        serde_json::json!(["opencode", "--prompt={{prompt}}"])
    ));
    assert!(has(
        "copilot",
        serde_json::json!(["copilot", "--interactive={{prompt}}"])
    ));
    assert!(has(
        "cursor",
        serde_json::json!(["cursor-agent", "--", "agent", "{{prompt}}"])
    ));
    assert!(has("pi", serde_json::json!(["pi", "{{prompt}}"])));
}

#[test]
fn test_runner_list_all_json_exposes_stable_raw_indexes_and_reasons() {
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\nrunners = [\n  { name = \"good\", argv = [\"good\", \"{{prompt}}\"] },\n  { name = \"broken\", argv = [\"broken\"] },\n  \"not-a-table\",\n]\n",
    );
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(
        payload,
        serde_json::json!([
            {"row": 0, "name": "good", "argv": ["good", "{{prompt}}"], "reason": null, "descriptor": "good", "valid": true},
            {"row": 1, "name": "broken", "argv": ["broken"], "reason": "prompt-slot-count", "descriptor": "broken", "valid": false},
            {"row": 2, "name": null, "argv": null, "reason": "row-not-table", "descriptor": "not-a-table", "valid": false},
        ])
    );
}

#[test]
fn test_runner_list_all_preserves_anonymous_argv_and_localizes_human_status() {
    // Only the oracle's config.gettext WRAPPING (the XX…XX pseudo that proves the copy is
    // localized) needs the i18n seam and is not black-box; the JSON contract, the rendered-argv
    // human output, AND the underlying localized recovery reasons are all reachable here.
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\nrunners = [\n  { name = \"   \", argv = [\"valuable-agent\", \"--model\", \"x\", \"{{prompt}}\"] },\n  { name = \"broken\", argv = [\"broken\"] },\n]\n",
    );
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(payload[0]["name"], Value::Null);
    assert_eq!(
        payload[0]["argv"],
        serde_json::json!(["valuable-agent", "--model", "x", "{{prompt}}"])
    );
    assert_eq!(payload[0]["reason"], "name"); // JSON keeps the stable machine code
    let human = sandbox.ok(&["runner", "list", "--all"]);
    let flat = human.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("valuable-agent"), "{human}");
    assert!(flat.contains("--model x"), "{human}");
    assert!(flat.contains("'{{prompt}}'"), "{human}");
    // The two malformed rows show their localized human recovery reasons. Split around the em dash
    // the catalog uses, mirroring the oracle's start/end anchoring (robust if the table wraps).
    assert!(flat.contains("A name is required."), "{human}");
    assert!(
        flat.contains("The command needs the {{prompt}} slot exactly once"),
        "{human}"
    );
    assert!(
        flat.contains("that's where the rendered prompt lands."),
        "{human}"
    );
    assert!(!flat.contains("prompt-slot-count"), "{human}"); // the machine code is not shown
}

#[test]
fn test_runner_list_empty_state() {
    let sandbox = Sandbox::new();
    sandbox.set_config("[prompt]\nrunners_seeded = true\nrunners = []\n");
    let combined = sandbox.ok(&["runner", "list"]);
    assert!(combined.contains("No agents are configured"), "{combined}");
    let flat = combined.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("skit runner add mycli -- mycli run {{prompt}}"),
        "{combined}"
    );
    let all_rows = sandbox.ok(&["runner", "list", "--all"]);
    assert!(all_rows.contains("No agents are configured"), "{all_rows}");
}

#[test]
fn test_runner_list_without_amp_omits_the_one_shot_note() {
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\n[[prompt.runners]]\nname = \"mycli\"\nargv = [\"mycli\", \"run\", \"{{prompt}}\"]\n",
    );
    let combined = sandbox.ok(&["runner", "list"]);
    assert!(combined.contains("mycli"), "{combined}");
    assert!(!combined.contains("one-shot"), "{combined}");
}

#[test]
fn test_runner_add_with_flag_bearing_argv() {
    let sandbox = Sandbox::new();
    sandbox.ok(&[
        "runner",
        "add",
        " sonnet ",
        "claude",
        "--model",
        "sonnet",
        "{{prompt}}",
    ]);
    let payload = sandbox.json(&["runner", "list", "--json"]);
    let rows = payload.as_array().unwrap();
    assert!(
        rows.iter().any(|row| row["name"] == "sonnet"
            && row["argv"] == serde_json::json!(["claude", "--model", "sonnet", "{{prompt}}"])),
        "{payload}"
    );
    // The name is trimmed on write; the last stored row is the new one.
    assert_eq!(rows.last().unwrap()["name"], "sonnet");
}

#[test]
fn test_runner_add_preserves_bad_rows_and_force_repairs_matching_name() {
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\nrunners = [\n  { name = \"typo\", argv = [\"old\"] },\n  \"not-a-table\",\n]\n",
    );
    sandbox.ok(&["runner", "add", "new", "new", "{{prompt}}"]);
    // The two bad rows survive an unrelated add.
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(payload[0]["name"], "typo");
    assert_eq!(payload[0]["argv"], serde_json::json!(["old"]));
    assert_eq!(payload[1]["descriptor"], "not-a-table");
    // A plain add on the taken name refuses.
    let (code, _) = sandbox.out(&["runner", "add", "typo", "fixed", "{{prompt}}"]);
    assert_eq!(code, 1);
    // --force repairs the matching name in place.
    sandbox.ok(&[
        "runner",
        "add",
        "typo",
        "--force",
        "--",
        "fixed",
        "{{prompt}}",
    ]);
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(payload[0]["name"], "typo");
    assert_eq!(
        payload[0]["argv"],
        serde_json::json!(["fixed", "{{prompt}}"])
    );
    assert_eq!(payload[1]["descriptor"], "not-a-table");
    assert_eq!(payload[2]["name"], "new");
    assert_eq!(payload[2]["argv"], serde_json::json!(["new", "{{prompt}}"]));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches, config unseeded. Oracle says 'A name is required'; Rust says 'a prompt runner needs a name'."]
fn test_runner_add_blank_name_is_refused_before_seeding() {
    let sandbox = Sandbox::new();
    let (code, combined) = sandbox.out(&["runner", "add", "   ", "x", "{{prompt}}"]);
    assert_eq!(code, 2, "{combined}");
    assert!(combined.contains("A name is required"), "{combined}");
    assert!(!sandbox.read_config().contains("runners_seeded = true"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches on each case, but the sentences differ — e.g. oracle 'first word' vs Rust '{{prompt}} cannot be the prompt runner program'."]
fn test_runner_add_validation_errors() {
    let sandbox = Sandbox::new();
    let cases: [(&[&str], &str); 3] = [
        (&["noslot", "claude"], "exactly once"),
        (&["bin", "{{prompt}}"], "first word"),
        (&["stray", "x", "{{other}}"], "only the {{prompt}} slot"),
    ];
    for (argv, needle) in cases {
        let mut args = vec!["runner", "add"];
        args.extend(argv.iter().copied());
        let (code, combined) = sandbox.out(&args);
        assert_eq!(code, 2, "{argv:?}: {combined}");
        assert!(combined.contains(needle), "{argv:?}: {combined}");
    }
    let (code, combined) = sandbox.out(&["runner", "add", "bare"]);
    assert_eq!(code, 2, "{combined}");
    assert!(combined.contains("needs a command"), "{combined}");
}

#[test]
fn test_runner_add_duplicate_name_refused() {
    let sandbox = Sandbox::new();
    let (code, combined) = sandbox.out(&["runner", "add", "claude", "x", "{{prompt}}"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("already exists"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 1 matches, config preserved. Oracle says 'isn't a table' / 'isn't a list'; Rust says 'configuration section is not a table: prompt' (and the list variant likewise reworded)."]
fn test_runner_add_reports_malformed_config_container() {
    let sandbox = Sandbox::new();
    // prompt is a scalar, not a table.
    sandbox.set_config("prompt = \"broken\"\n");
    let (code, combined) = sandbox.out(&["runner", "add", "new", "new", "{{prompt}}"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("isn't a table"), "{combined}");
    assert_eq!(sandbox.read_config().trim(), "prompt = \"broken\"");

    let sandbox = Sandbox::new();
    // prompt.runners is a scalar, not a list.
    sandbox.set_config("[prompt]\nrunners = \"broken\"\n");
    let (code, combined) = sandbox.out(&["runner", "add", "new", "new", "{{prompt}}"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("isn't a list"), "{combined}");
}

#[test]
fn test_runner_remove_and_unknown() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["runner", "remove", " amp ", "-y"]); // trimmed
    let payload = sandbox.json(&["runner", "list", "--json"]);
    assert!(
        !payload
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "amp")
    );
    let (code, combined) = sandbox.out(&["runner", "remove", "amp", "-y"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("Unknown runner"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches, config unseeded. Oracle says 'A name is required'; Rust says 'a prompt runner needs a name'."]
fn test_runner_remove_blank_name_is_usage_error_before_seeding() {
    let sandbox = Sandbox::new();
    let (code, combined) = sandbox.out(&["runner", "remove", "   ", "--yes"]);
    assert_eq!(code, 2, "{combined}");
    assert!(combined.contains("A name is required"), "{combined}");
    assert!(!sandbox.read_config().contains("runners_seeded = true"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches and nothing is written, but the sentences differ — oracle 'exactly one' / 'non-negative index'; Rust 'runner remove needs a name or --row INDEX' etc."]
fn test_runner_remove_rejects_ambiguous_or_invalid_targets_before_writing() {
    let cases: [(&[&str], &str); 4] = [
        (&[], "exactly one"),
        (&["amp", "--row", "0"], "exactly one"),
        (&["--row", "not-an-index"], "non-negative index"),
        (&["--row", "-1"], "non-negative index"),
    ];
    for (args, needle) in cases {
        let sandbox = Sandbox::new();
        sandbox.set_config("[prompt]\nrunners_seeded = true\nrunners = []\n");
        let before = sandbox.read_config();
        let mut full = vec!["runner", "remove"];
        full.extend(args.iter().copied());
        full.push("--yes");
        let (code, combined) = sandbox.out(&full);
        assert_eq!(code, 2, "{args:?}: {combined}");
        assert!(combined.contains(needle), "{args:?}: {combined}");
        assert_eq!(sandbox.read_config(), before, "{args:?}");
    }
}

#[test]
fn test_removing_every_runner_stays_empty() {
    let sandbox = Sandbox::new();
    for name in [
        "claude",
        "codex",
        "opencode",
        "amp",
        "antigravity",
        "copilot",
        "cursor",
        "pi",
    ] {
        sandbox.ok(&["runner", "remove", name, "--yes"]);
    }
    let payload = sandbox.json(&["runner", "list", "--json"]);
    assert!(payload.as_array().unwrap().is_empty());
    // The seeds must NOT resurrect.
    let combined = sandbox.ok(&["runner", "list"]);
    assert!(!combined.contains("claude"), "{combined}");
}

#[test]
#[ignore = "UNMAPPED (cross-crate): a name remove without -y asks typer.confirm(abort=True) and honors 'y'; the confirmation is an interactive seam a non-tty binary cannot drive (it refuses with 'pass --yes' instead of asking). Seam: src/cli/tests.rs. Non-interactive twin: test_runner_remove_and_unknown (with -y)."]
fn test_runner_remove_confirms_unless_yes() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): answering 'n'/EOF to typer.confirm aborts (exit 1, nothing removed); the interactive confirmation seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_runner_remove_abort_keeps_the_runner() {}

#[test]
fn test_runner_remove_warns_and_preserves_affected_prompt_pins() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "amp");
    let combined = sandbox.ok(&["runner", "remove", "amp", "--yes"]);
    assert!(combined.contains("1 prompt pins this runner"), "{combined}");
    assert!(sandbox.meta("p").contains("amp"), "{}", sandbox.meta("p"));
    let payload = sandbox.json(&["runner", "list", "--json"]);
    assert!(
        !payload
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "amp")
    );
}

#[test]
fn test_runner_remove_raw_row_is_targeted_and_requires_yes_noninteractively() {
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\nrunners = [\n  { name = \"good\", argv = [\"good\", \"{{prompt}}\"] },\n  { name = \"broken\", argv = [\"broken\"] },\n  \"untouched\",\n]\n",
    );
    let (code, combined) = sandbox.out(&["runner", "remove", "--row", "1", "--no-input"]);
    assert_eq!(code, 2, "{combined}");
    assert!(combined.contains("pass --yes"), "{combined}");
    assert_eq!(
        sandbox
            .json(&["runner", "list", "--all", "--json"])
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let removed = sandbox.ok(&["runner", "remove", "--row", "1", "--yes"]);
    assert!(
        removed.contains("Malformed runner row 1 removed"),
        "{removed}"
    );
    assert!(!removed.contains("Runner broken removed"), "{removed}");
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(payload[0]["name"], "good");
    assert_eq!(payload[1]["descriptor"], "untouched");
    assert_eq!(payload.as_array().unwrap().len(), 2);
    let (code, combined) = sandbox.out(&["runner", "remove", "--row", "9", "--yes"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("runner list --all"), "{combined}");
}

#[test]
fn test_runner_remove_raw_duplicate_has_no_false_pin_warning_or_key_removed_claim() {
    let sandbox = Sandbox::new();
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\nrunners = [\n  { name = \"same\", argv = [\"first\", \"{{prompt}}\"] },\n  { name = \"same\", argv = [\"second\", \"{{prompt}}\"] },\n]\n",
    );
    sandbox.added_pin("Do {{a}}\n", "p", "same");
    let combined = sandbox.ok(&["runner", "remove", "--row", "1", "--yes"]);
    assert!(!combined.contains("pins this runner"), "{combined}");
    assert!(!combined.contains("Runner same removed"), "{combined}");
    assert!(
        combined.contains("Malformed runner row 1 removed"),
        "{combined}"
    );
    let payload = sandbox.json(&["runner", "list", "--json"]);
    let row = payload
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "same")
        .unwrap();
    assert_eq!(row["argv"], serde_json::json!(["first", "{{prompt}}"]));
    assert!(sandbox.meta("p").contains("same"), "{}", sandbox.meta("p"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches and rows are preserved, but Rust names the stable path without quotes ('skit runner remove same') where the oracle quotes it ('skit runner remove \"same\"')."]
fn test_runner_remove_raw_valid_row_requires_stable_name_path() {
    let sandbox = Sandbox::new();
    let rows = "[prompt]\nrunners_seeded = true\nrunners = [\n  { name = \"same\", argv = [\"first\", \"{{prompt}}\"] },\n  { name = \"same\", argv = [\"second\", \"{{prompt}}\"] },\n]\n";
    sandbox.set_config(rows);
    let (code, combined) = sandbox.out(&["runner", "remove", "--row", "0", "--yes"]);
    assert_eq!(code, 2, "{combined}");
    let flat = combined.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("skit runner remove \"same\""), "{combined}");
    let payload = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(payload.as_array().unwrap().len(), 2);
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the raw-row remove must refuse if the index SHIFTED during the interactive typer.confirm (a monkeypatched confirm that mutates config mid-prompt). The confirmation seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_runner_remove_raw_row_refuses_if_index_shifted_during_confirmation() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the name remove must refuse if the key was replaced during the interactive typer.confirm (a monkeypatched confirm that swaps the row mid-prompt). The confirmation seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_runner_remove_name_refuses_if_key_is_replaced_during_confirmation() {}

#[test]
fn test_runner_remove_container_repairs_only_targeted_prompt_value() {
    let sandbox = Sandbox::new();
    sandbox.set_config("language = \"zh-TW\"\nprompt = \"garbage\"\n");
    let inspected = sandbox.json(&["runner", "list", "--all", "--json"]);
    assert_eq!(inspected[0]["row"], Value::Null);
    assert_eq!(inspected[0]["reason"], "prompt-section-not-table");
    let combined = sandbox.ok(&["runner", "remove", "--row", "container", "--yes"]);
    assert!(
        combined.contains("Malformed prompt runner container removed"),
        "{combined}"
    );
    assert!(!combined.contains("Runner container removed"), "{combined}");
    let config = sandbox.read_config();
    assert!(config.contains("zh-TW"), "{config}");
    assert!(config.contains("runners_seeded = true"), "{config}");
}

// ==========================================================================
// doctor
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the doctor --json half converges (drift contains 'p', runner_rows_invalid == ['broken']) and the human line names 'broken'; but the oracle's recovery line 'Inspect and repair with: skit runner list --all' is absent from Rust's human doctor (it prints 'WARN malformed prompt runners: broken')."]
fn test_doctor_reports_prompt_drift_and_bad_runner_rows() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}}\n", "p");
    overwrite_body(&sandbox, "p", "no holes\n");
    sandbox.set_config(
        "[prompt]\nrunners_seeded = true\n[[prompt.runners]]\nname = \"broken\"\nargv = [\"x\"]\n",
    );
    let payload = sandbox.json(&["doctor", "--json"]);
    assert!(
        payload["drift"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "p"),
        "{payload}"
    );
    assert_eq!(
        payload["runner_rows_invalid"],
        serde_json::json!(["broken"])
    );
    let human = sandbox.ok(&["doctor"]);
    assert!(human.contains("broken"), "{human}");
    let flat = human.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        flat.contains("Inspect and repair with: skit runner list --all"),
        "{human}"
    );
}

#[test]
fn test_doctor_healthy_prompt_reports_no_drift() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}}\n", "p");
    let payload = sandbox.json(&["doctor", "--json"]);
    assert_eq!(payload["drift"], serde_json::json!([]));
    assert_eq!(payload["runner_rows_invalid"], serde_json::json!([]));
}

// ==========================================================================
// completion
// ==========================================================================

#[test]
fn test_complete_runner_names() {
    // The clap_complete dynamic-completion seam IS black-box reachable: `COMPLETE=bash skit -- skit
    // run --runner <partial>` with the cursor index set drives runner_candidates (cli.rs:107,
    // run/command.rs). The oracle's third assertion (config.load_prompt_runners raising) maps
    // black-box to a malformed config: completion degrades to no candidates and never crashes.
    let sandbox = Sandbox::new();
    let complete = |partial: &str| -> (i32, String) {
        let output = sandbox
            .command()
            .env("COMPLETE", "bash")
            .env("_CLAP_COMPLETE_INDEX", "3") // skit=0 run=1 --runner=2 <value>=3
            .args(["--", "skit", "run", "--runner", partial])
            .output()
            .unwrap();
        (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    };
    // "cl" offers the seeded "claude"; "zz" offers nothing.
    let (code, offered) = complete("cl");
    assert_eq!(code, 0, "{offered}");
    assert!(offered.lines().any(|line| line == "claude"), "{offered}");
    let (code, offered) = complete("zz");
    assert_eq!(code, 0, "{offered}");
    assert!(!offered.contains("claude"), "{offered}");
    // A malformed config degrades to no candidates and never crashes the shell.
    sandbox.set_config("prompt = \"not-a-table\"\n");
    let (code, offered) = complete("");
    assert_eq!(code, 0, "{offered}");
    assert!(!offered.contains("claude"), "{offered}");
}

// ==========================================================================
// edges: unreadable bodies, store failures, refusal lanes
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 1 matches. Oracle prints 'Not a file' for a directory .prompt.md; Rust prints the raw OS error 'could not read <path>: Is a directory (os error 21)'."]
fn test_add_prompt_unreadable_file_is_a_store_error() {
    let sandbox = Sandbox::new();
    let trap = sandbox.data.path().join("dir.prompt.md");
    fs::create_dir(&trap).unwrap(); // a directory that "exists" but is not a file
    let (code, combined) = sandbox.out(&["add", trap.to_str().unwrap(), "--no-input"]);
    assert_eq!(code, 1, "{combined}");
    assert!(combined.contains("Not a file"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches. Oracle prints '--runner only applies to prompt entries' or '--runner can't apply here'; Rust uses a clap conflict ('the argument --cmd cannot be used with --runner')."]
fn test_add_runner_flag_refused_on_cmd_edit_exe_lanes() {
    let sandbox = Sandbox::new();
    let cases: [Vec<&str>; 3] = [
        vec!["add", "--cmd", "echo {x}", "-n", "c", "--runner", "claude"],
        vec!["add", "--edit", "--runner", "claude"],
        vec!["add", "x", "--exe", "--runner", "claude"],
    ];
    for args in cases {
        let (code, combined) = sandbox.out(&args);
        assert_eq!(code, 2, "{args:?}: {combined}");
        assert!(
            combined.contains("--runner only applies to prompt entries")
                || combined.contains("--runner can't apply here"),
            "{args:?}: {combined}"
        );
    }
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 matches. Oracle refuses up front with '--no-interpolate only applies to prompt entries'; Rust routes it through clap conflicts (the argument '--exe' cannot be used with '--no-interpolate') and never emits the prompt-only sentence."]
fn test_add_no_interpolate_refused_up_front_on_non_prompt_path_lane() {
    let sandbox = Sandbox::new();
    let prog = sandbox.write_file("tool", b"#!/bin/sh\necho hi\n");
    for extra in [vec!["--exe"], vec!["--kind", "shell"]] {
        let mut args = vec!["add", &prog];
        args.extend(extra.iter().copied());
        args.extend(["--no-interpolate", "-n", "t", "--no-input"]);
        let (code, combined) = sandbox.out(&args);
        assert_eq!(code, 2, "{extra:?}: {combined}");
        assert!(
            combined.contains("--no-interpolate only applies to prompt entries"),
            "{extra:?}: {combined}"
        );
        assert_eq!(
            sandbox.json(&["list", "--json"]).as_array().unwrap().len(),
            0
        );
    }
}

#[test]
#[ignore = "UNMAPPED (cross-crate): drives the interactive prompt-editor lane (monkeypatched cli.editor.open_in_editor + cli.Prompt.ask) to reach a store-error 'already taken'; a non-tty binary never opens $EDITOR here. Non-interactive twin: test_add_prompt_stdin_lane_reports_store_errors."]
fn test_add_prompt_editor_lane_reports_store_errors() {}

#[test]
fn test_add_prompt_stdin_lane_reports_store_errors() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "echo hi", "-n", "taken", "--no-input"]);
    let output = sandbox
        .command()
        .args(["add", "-", "--prompt", "-n", "taken"])
        .write_stdin("b\n")
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(1), "{combined}");
    assert!(combined.contains("already taken"), "{combined}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): name derivation. `add p.prompt.md --ref` (no -n): oracle slug 'p' (store.py:571 removesuffix '.prompt'); Rust slug 'p-prompt', so `params p` is 'entry not found'."]
fn test_params_view_survives_an_unreadable_reference_body() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"{{a}}\n");
    sandbox.ok(&["add", &src, "--ref", "--no-input"]);
    fs::remove_file(&src).unwrap(); // the original vanished
    let (code, combined) = sandbox.out(&["params", "p"]);
    assert_eq!(code, 0, "{combined}");
    assert!(combined.contains("a = "), "{combined}"); // the managed record still lists
}

#[test]
#[ignore = "UNMAPPED (cross-crate): forces a StoreError from store.write_prompt_runner via monkeypatch and asserts the exact injected message ('disk on fire'); a store-write fault is not deterministically inducible from a black-box binary. Seam: src/cli/tests.rs / store fault injection."]
fn test_params_runner_pin_reports_store_errors() {}

#[test]
fn test_doctor_skips_a_prompt_whose_body_is_gone() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}}\n", "p");
    overwrite_body(&sandbox, "p", ""); // truncate is not enough — remove it
    let dir = sandbox.entry_dir("p");
    for entry in fs::read_dir(&dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != std::ffi::OsStr::new("meta.toml") {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    let payload = sandbox.json(&["doctor", "--json"]);
    assert_eq!(payload["drift"], serde_json::json!([])); // missing is missing's problem
    assert!(
        payload["missing"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "p"),
        "{payload}"
    );
}

// ==========================================================================
// the interpolate switch + flood caps (CLI surfaces)
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): name derivation. `add p.prompt.md` (no -n): oracle slug 'p' (store.py:571 removesuffix '.prompt'); Rust slug 'p-prompt', so `show p` is 'entry not found'."]
fn test_add_no_interpolate() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("p.prompt.md", b"{{a}} {{b}}\n");
    let combined = sandbox.ok(&["add", &src, "--no-interpolate", "--no-input"]);
    let show = sandbox.json(&["show", "p", "--json"]);
    assert_eq!(show["interpolate"], false);
    assert!(show["fields"].as_array().unwrap().is_empty()); // params None → no fields
    assert!(
        combined.to_lowercase().contains("insertion is off"),
        "{combined}"
    );
}

#[test]
fn test_add_no_interpolate_refused_off_the_prompt_lanes() {
    let sandbox = Sandbox::new();
    let py = sandbox.write_file("s.py", b"print(1)\n");
    let (code, combined) = sandbox.out(&["add", &py, "--no-interpolate", "--no-input"]);
    assert_eq!(code, 2, "{combined}");
    assert!(
        combined.contains("--no-interpolate only applies to prompt entries"),
        "{combined}"
    );
    let (code, _) = sandbox.out(&["add", "--cmd", "echo hi", "-n", "c", "--no-interpolate"]);
    assert_eq!(code, 2);
}

#[test]
fn test_add_no_interpolate_through_stdin_lane() {
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "-", "--prompt", "-n", "clip", "--no-interpolate"])
        .write_stdin("Body {{x}}\n")
        .assert()
        .success();
    assert_eq!(
        sandbox.json(&["show", "clip", "--json"])["interpolate"],
        false
    );
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive 'off' answer that disables insertion is given via monkeypatched cli.Prompt.ask; a non-tty binary never asks. Seam: src/cli/tests.rs. Non-interactive twin: test_add_no_interpolate."]
fn test_add_interactive_off_answer_disables_insertion() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): name derivation. `add p.prompt.md` (no -n): oracle derives slug 'p' via source.stem.removesuffix('.prompt') (store.py:571); Rust keeps 'p.prompt' -> slug 'p-prompt', so `show p` is 'entry not found'."]
fn test_add_flood_cap_manages_nothing_and_says_so() {
    let sandbox = Sandbox::new();
    let many = (0..AUTO_MANAGE_LIMIT + 5)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let src = sandbox.write_file("p.prompt.md", format!("{many}\n").as_bytes());
    let combined = sandbox.ok(&["add", &src, "--no-input"]);
    assert!(
        sandbox.json(&["show", "p", "--json"])["fields"]
            .as_array()
            .unwrap()
            .is_empty(),
        "flood must manage nothing"
    );
    assert!(
        combined.contains("too many to manage automatically"),
        "{combined}"
    );
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive flood default flips to 'none' and the listing is capped, observed via monkeypatched cli.Prompt.ask (seen['default']); a non-tty binary never asks. Seam: src/cli/tests.rs. Non-interactive twin: test_add_flood_cap_manages_nothing_and_says_so."]
fn test_add_interactive_flood_defaults_to_none_and_caps_the_listing() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): an explicit interactive 'all' beats the flood cap, via monkeypatched cli.Prompt.ask; a non-tty binary never asks. Seam: src/cli/tests.rs."]
fn test_add_interactive_explicit_all_beats_the_flood_cap() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the --json and re-manage halves converge, but the human params view says 'Interpolation: off', never the oracle's 'Variable insertion is off'."]
fn test_params_interpolate_off_and_on() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    sandbox.ok(&["params", "p", "--no-interpolate"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["interpolate"], false);
    let view = sandbox.ok(&["params", "p"]);
    assert!(view.contains("Variable insertion is off"), "{view}");
    let payload = sandbox.json(&["params", "p", "--json"]);
    assert_eq!(payload["interpolate"], false);
    assert_eq!(payload["unmanaged"], serde_json::json!([])); // no scanning while off
    sandbox.ok(&["params", "p", "--interpolate"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["interpolate"], true);
    assert_eq!(
        sandbox.json(&["params", "p", "--json"])["placeholders"],
        serde_json::json!(["a"])
    );
}

#[test]
#[ignore = "UNMAPPED (cross-crate): forces a StoreError from store.write_prompt_interpolate via monkeypatch and asserts the injected 'disk on fire'; a store-write fault is not deterministically inducible from a black-box binary. Seam: src/cli/tests.rs / store fault injection."]
fn test_params_interpolate_reports_store_errors() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): message '--interpolate only applies to prompt entries' converges, but Rust exits 2 (CliError::Usage) where the oracle exits 1."]
fn test_params_interpolate_refused_on_non_prompt() {
    let sandbox = Sandbox::new();
    sandbox.ok(&["add", "--cmd", "echo {m}", "-n", "cmd", "--no-input"]);
    let (code, combined) = sandbox.out(&["params", "cmd", "--no-interpolate"]);
    assert_eq!(code, 1, "{combined}");
    assert!(
        combined.contains("--interpolate only applies to prompt entries"),
        "{combined}"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the --json 'unmanaged' full-list contract converges, but the human params view omits the capped preview tail ('and N more candidate(s)') — Rust's human read view lists no unmanaged candidates."]
fn test_params_unmanaged_listing_is_flood_capped_and_localizable() {
    // Parametrized over (extra, tail): 1 → "and 1 more candidate", 7 → "and 7 more candidates".
    for (extra, tail) in [
        (1usize, "and 1 more candidate"),
        (7usize, "and 7 more candidates"),
    ] {
        let sandbox = Sandbox::new();
        sandbox.added("Do {{a}}\n", "p");
        let names: Vec<String> = (0..LIST_PREVIEW_LIMIT + extra)
            .map(|index| format!("u{index}"))
            .collect();
        let many = names
            .iter()
            .map(|name| format!("{{{{{name}}}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        overwrite_body(&sandbox, "p", &format!("{{{{a}}}} {many}\n"));
        let combined = sandbox.ok(&["params", "p"]);
        let flat = combined.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(tail), "extra={extra}: {combined}");
        assert!(
            flat.contains(&names[LIST_PREVIEW_LIMIT - 1]),
            "extra={extra}: {combined}"
        );
        assert!(
            !flat.contains(&names[LIST_PREVIEW_LIMIT]),
            "extra={extra}: {combined}"
        );
        let payload = sandbox.json(&["params", "p", "--json"]);
        assert_eq!(payload["unmanaged"], serde_json::json!(names)); // machine contract is full data
    }
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the x-pseudo locale itself works (output is bracketed), but the human params view carries no unmanaged tail, so the pseudo-transformed 'möré' never appears — same missing-listing root as the read-view divergence."]
fn test_params_unmanaged_tail_passes_through_the_i18n_boundary() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    let names: Vec<String> = (0..LIST_PREVIEW_LIMIT + 3)
        .map(|index| format!("u{index}"))
        .collect();
    let many = names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    overwrite_body(&sandbox, "p", &format!("{{{{a}}}} {many}"));
    let output = sandbox
        .command()
        .env("SKIT_LANG", "x-pseudo")
        .args(["params", "p"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{combined}");
    assert!(combined.contains('⟦'), "{combined}");
    assert!(combined.contains("möré"), "{combined}"); // pseudo-transformed tail
    assert!(!combined.contains("and 3 more"), "{combined}");
}

#[test]
fn test_show_reports_the_interpolate_switch() {
    let sandbox = Sandbox::new();
    sandbox.added("Do {{a}}\n", "p");
    sandbox.ok(&["params", "p", "--no-interpolate"]);
    assert_eq!(sandbox.json(&["show", "p", "--json"])["interpolate"], false);
    let human = sandbox.ok(&["show", "p"]);
    assert!(human.contains("Variable insertion: off"), "{human}");
}

#[test]
fn test_doctor_skips_drift_for_an_insertion_off_prompt() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}}\n", "p");
    overwrite_body(&sandbox, "p", "gone\n");
    sandbox.ok(&["params", "p", "--no-interpolate"]);
    let payload = sandbox.json(&["doctor", "--json"]);
    assert_eq!(payload["drift"], serde_json::json!([]));
}

#[test]
fn test_run_insertion_off_prompt_rejects_set_and_sends_verbatim() {
    let sandbox = Sandbox::new();
    sandbox.added_pin("Do {{a}}\n", "p", "claude");
    sandbox.ok(&["params", "p", "--no-interpolate"]);
    let (code, _) = sandbox.out(&["run", "p", "--set", "a=1", "--no-input"]);
    assert_eq!(code, 2); // no fields: --set has nothing to target
    let tools = tools_dir();
    let cap = tools.path().join("cap.txt");
    let output = sandbox
        .command()
        .env("PATH", tools.path())
        .env("SKIT_CAP", &cap)
        .args(["run", "p", "--no-input"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let captured = fs::read_to_string(&cap).unwrap_or_default();
    assert!(captured.contains("Do {{a}}"), "verbatim body: {captured:?}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): Rust has no insertion-off gate on schema edits — with interpolation off, `params --add b` is processed (and errors 'parameter already exists: b') instead of the oracle's exit-1 refusal 'Variable insertion is off'."]
fn test_params_schema_edits_refused_while_insertion_is_off() {
    let sandbox = Sandbox::new();
    sandbox.added("{{a}} {{b}}\n", "p");
    sandbox.ok(&["params", "p", "--no-interpolate"]);
    for flags in [
        vec!["--add", "b"],
        vec!["--rm", "a"],
        vec!["--deliver", "a=placeholder"],
    ] {
        let mut args = vec!["params", "p"];
        args.extend(flags.iter().copied());
        let (code, combined) = sandbox.out(&args);
        assert_eq!(code, 1, "{flags:?}: {combined}");
        assert!(
            combined.contains("Variable insertion is off"),
            "{flags:?}: {combined}"
        );
    }
    sandbox.ok(&["params", "p", "--interpolate"]);
    assert_eq!(
        sandbox.json(&["params", "p", "--json"])["placeholders"],
        serde_json::json!(["a", "b"])
    ); // nothing was mutated while off
    let (code, _) = sandbox.out(&["params", "p", "--rm", "b"]);
    assert_eq!(code, 0);
}

#[test]
#[ignore = "UNMAPPED (cross-crate): a flooded interactive tick that names an index BEYOND the preview must be ignored, observed via monkeypatched cli.Prompt.ask answers; a non-tty binary never opens the picker. Seam: src/cli/tests.rs."]
fn test_add_interactive_flooded_numbers_address_the_previewed_names_only() {}

// ==========================================================================
// edit — the placeholder a body edit introduces is offered for management
// ==========================================================================

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive reconcile answers 'all' via monkeypatched cli.Prompt.ask to MANAGE a body-introduced placeholder; a non-tty binary takes the non-interactive reconcile (manages nothing). Seam: src/cli/tests.rs. Non-interactive twin: test_edit_prompt_non_interactive_names_the_unmanaged_variable."]
fn test_edit_prompt_interactive_offers_and_manages_a_new_placeholder() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive reconcile answers 'none' via monkeypatched cli.Prompt.ask; a non-tty binary cannot drive that picker. Seam: src/cli/tests.rs."]
fn test_edit_prompt_interactive_none_leaves_the_placeholder_literal() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive reconcile answers '1,3' (a tick subset) via monkeypatched cli.Prompt.ask; a non-tty binary cannot drive that picker. Seam: src/cli/tests.rs."]
fn test_edit_prompt_interactive_numbers_manage_the_named_ones() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive reconcile answers 'all', preserving prior managed names and adding the new one, via monkeypatched cli.Prompt.ask; a non-tty binary cannot drive that picker. Seam: src/cli/tests.rs."]
fn test_edit_prompt_preserves_existing_managed_and_adds_the_new_one() {}

#[test]
#[ignore = "FAILING CONTRACT (divergence): Rust `edit` prints only 'Edited: greet (greet)' and never the oracle's 'Detected but not yet managed: username' — the non-interactive edit reconcile does not surface new placeholders."]
fn test_edit_prompt_non_interactive_names_the_unmanaged_variable() {
    let sandbox = Sandbox::new();
    sandbox.added("Say hello.\n", "greet");
    let tools = tools_dir();
    let editor = appending_editor(tools.path(), "\nUser is {{username}}\n");
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["edit", "greet"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{combined}");
    assert!(
        sandbox.json(&["show", "greet", "--json"])["fields"]
            .as_array()
            .unwrap()
            .is_empty(),
        "non-interactive manages nothing"
    );
    assert!(
        combined.contains("Detected but not yet managed: username"),
        "{combined}"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): Rust `edit` does not surface body-introduced placeholders after an edit — it prints only 'Edited: greet (greet)', never the oracle's flood preview 'and 4 more candidates'. Non-interactive reconcile hint is absent."]
fn test_edit_prompt_non_interactive_flood_previews_with_a_tail() {
    let sandbox = Sandbox::new();
    sandbox.added("Base.\n", "greet");
    let holes = (0..LIST_PREVIEW_LIMIT + 4)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let tools = tools_dir();
    let editor = appending_editor(tools.path(), &format!("\n{holes}\n"));
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["edit", "greet"])
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "{combined}");
    assert!(combined.contains("and 4 more candidates"), "{combined}");
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the interactive flood reconcile answers 'all' via monkeypatched cli.Prompt.ask, showing a secret mark and a tail; a non-tty binary cannot drive that picker. Seam: src/cli/tests.rs. Non-interactive tail twin: test_edit_prompt_non_interactive_flood_previews_with_a_tail."]
fn test_edit_prompt_interactive_flood_previews_secret_mark_and_tail() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): under form=tui the reconcile hosts skit.tui_add.run_candidate_picker and manages its returned set; that Textual picker seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_edit_prompt_tui_reconcile_manages_the_pickers_selection() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): cancelling the tui_add candidate picker (None) manages nothing; the Textual picker seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_edit_prompt_tui_reconcile_none_manages_nothing() {}

#[test]
#[ignore = "UNMAPPED (cross-crate): the flooded tui_add picker opens with an EMPTY preselection (seen['selected']); the Textual picker seam is not reachable from a non-tty binary. Seam: src/cli/tests.rs."]
fn test_edit_prompt_tui_reconcile_flood_preselects_nothing() {}

#[test]
fn test_edit_prompt_with_no_new_placeholders_is_silent() {
    let sandbox = Sandbox::new();
    let src = sandbox.write_file("greet.prompt.md", b"{{a}}\n");
    sandbox.ok(&["add", &src, "-n", "greet", "--no-input"]); // auto-manages a
    let tools = tools_dir();
    let editor = appending_editor(tools.path(), "\nmore prose\n");
    let combined = {
        let output = sandbox
            .command()
            .env("EDITOR", &editor)
            .env("VISUAL", &editor)
            .args(["edit", "greet"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };
    assert!(!combined.contains("Now managed"), "{combined}");
    assert!(
        !combined.contains("Detected but not yet managed"),
        "{combined}"
    );
    let keys: Vec<String> = sandbox.json(&["show", "greet", "--json"])["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field["key"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(keys, ["a"]);
}

#[test]
fn test_edit_non_prompt_keeps_the_generic_drift_hint() {
    let sandbox = Sandbox::new();
    let script = sandbox.write_file("s.py", b"print(1)\n");
    sandbox.ok(&["add", &script, "-n", "job", "--no-input"]);
    let tools = tools_dir();
    // A no-op editor: touch nothing, exit 0.
    let editor = tools.path().join("noop-editor.sh");
    fs::write(&editor, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    let combined = {
        let output = sandbox
            .command()
            .env("EDITOR", &editor)
            .env("VISUAL", &editor)
            .args(["edit", "job"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0));
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };
    assert!(
        combined.contains("skit reconciles parameter drift at run time"),
        "{combined}"
    );
}
