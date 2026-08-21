//! Mechanical port of the Python oracle module `tests/test_cli.py`
//! (`origin/main@206f9ef`, version 0.4.1.dev0): "CLI end-to-end (Typer `CliRunner`) + direct
//! unit tests for interactive helper functions." Each `#[test]` keeps its Python `def test_*`
//! name and its WHY comment so it traces back to its origin.
//!
//! WHY `skit-cli` (package `skit-cli-rs`): the oracle drives `skit.cli.app` through Typer's
//! `CliRunner` and inspects exit codes, on-disk results, and printed output. Only the
//! composition-root crate can run the real `skit` binary end to end. These tests drive the
//! `skit` binary through `assert_cmd`, with `SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR`
//! pinned to a per-test `TempDir` and `SKIT_LANG=en` on every invocation.
//!
//! OBSERVABLE MAPPING (`CliRunner` -> a black-box binary):
//! - `result.exit_code` -> the process exit code.
//! - `result.output` (CliRunner merges stdout+stderr) -> `run()` returns the two streams
//!   concatenated, so a substring may land on either.
//! - `store.resolve(slug).meta.{mode,kind,description}` -> the compact `list --json` row (the
//!   machine contract keeps the RAW registry ids `python`/`prompt`/`exe`, no spaces after the
//!   `:` — a serializer artifact, so `"missing":true` is matched compact, not `"missing": true`).
//! - `store.add_python/add_command/add_exe(...)` fixtures -> the real `skit add` lanes, or a
//!   hand-built entry directory + `registry.toml` for the uv-free shell vehicle.
//! - `argstate.save_last / save_preset` -> a seeded `state/values/<slug>.toml`
//!   (top-level `extra_args`, `[values]`, `[presets.<name>]`); `argstate.load_state` -> reading
//!   that file back.
//! - the `run_entry_spy` (monkeypatched `launcher.run_entry`, reading its `values`/`extra_args`/
//!   `script_override`) -> real, independent black-box witnesses: `script_override is not None`
//!   -> the `→ inject:` transparency line; `extra` -> a `printf '%s\n' "$@"` shell body echoing
//!   the tail; `values` -> the value reaching the command/script body on stdout.
//!
//! KIND SUBSTITUTION (uv-free vehicles, the `port_test_run_set.rs` precedent). A python entry can
//! only launch through `uv run --script` (a uv download + network); the run behaviors under test
//! are kind-agnostic, so an INJECT-const python fixture becomes a SHELL entry built with
//! `write_managed_params` ("const" -> Inject delivery), and the argparse fixture stays python but
//! is observed through the offline `--dry-run` preview (`PATH=""`, no uv).
//!
//! Buckets:
//! - REAL asserting `#[test]` (API EXISTS, behavior converges): the bulk — add/list/remove/run/
//!   preset/params/deps/doctor/config lanes, and every markup case that Rust renders as literal
//!   text (Rust has no Rich markup layer, so "escaping" reduces to "the datum is printed"), plus
//!   typed-value validation before launch.
//! - FAILING CONTRACT (divergence): the Rust message/exit differs from the oracle's. The full
//!   asserting body is kept and `#[ignore]`d with the exact oracle-vs-Rust evidence; deleting the
//!   `#[ignore]` line after the impl is fixed turns it green.
//! - cross-crate / white-box `#[ignore]` stub: the direct unit tests of CLI-PRIVATE helpers
//!   (`_parse_selection`, `_parse_kv_opts`, `_resolve_python_metadata`, `_prompt_identity`,
//!   `_onboard_params`, `_list_description` — none `pub` in `skit_cli`), the interactive
//!   tty/Prompt.ask + Textual-panel seams (`src/cli.rs` inline forms + `skit-tui`), and the
//!   un-injectable internal faults (`Path.read_text` failure, `shim.inject`/`launcher.run_entry`
//!   monkeypatch, `store.doctor_rebuild` monkeypatch, a hostile analyzer candidate). Each names
//!   its owning seam.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{detect_candidates, write_managed_params};
use tempfile::TempDir;

/// The distinctive prefix skit prints iff an injected temp copy is made — the black-box witness
/// for the oracle's `script_override is not None`.
const INJECT_MARKER: &str = "→ inject:";

/// The oracle's module-level `ARGPARSE_REQUIRED` fixture.
const ARGPARSE_REQUIRED: &str = "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('-o', '--output', required=True)\nap.parse_args()\n";

/// A fresh sandbox root holding `data/`, `state/`, and `config/` subtrees.
fn sandbox() -> TempDir {
    TempDir::new().unwrap()
}

/// The oracle's `runner.invoke(cli.app, ...)`: the real `skit` binary with all three roots pinned
/// under the sandbox and the locale fixed to English.
fn skit(root: &TempDir) -> Command {
    let mut command = Command::cargo_bin("skit").expect("skit binary builds");
    command
        .env("SKIT_DATA_DIR", root.path().join("data"))
        .env("SKIT_STATE_DIR", root.path().join("state"))
        .env("SKIT_CONFIG_DIR", root.path().join("config"))
        .env("SKIT_LANG", "en");
    command
}

/// Run one command; return `(exit_code, stdout+stderr)` — the CliRunner `result.output` semantics.
fn run(command: &mut Command) -> (i32, String) {
    let output = command.output().expect("skit runs");
    let mut merged = String::from_utf8_lossy(&output.stdout).into_owned();
    merged.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), merged)
}

/// The oracle's `" ".join(result.output.split())`: collapse every run of whitespace so a
/// Rich-wrapped sentence matches.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Write a source file under the sandbox and return its path (the oracle's `_py` / `tmp_path`).
fn write_src(root: &TempDir, name: &str, body: &str) -> PathBuf {
    let path = root.path().join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, body).unwrap();
    path
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

/// Register one hand-built entry directory in the authoritative membership index.
fn register(root: &TempDir, slug: &str) {
    let data = root.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let registry = data.join("registry.toml");
    let mut body = fs::read_to_string(&registry).unwrap_or_default();
    body.push_str(&format!("[entries.{slug}]\n"));
    fs::write(&registry, body).unwrap();
}

/// A hand-built shell entry (the uv-free launch vehicle). `extra_meta` appends optional metadata
/// lines (e.g. a pinned interpreter).
fn shell_entry(root: &TempDir, slug: &str, name: &str, source: &str, extra_meta: &str) {
    let dir = root.path().join("data/scripts").join(slug);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("script.sh"), source).unwrap();
    fs::write(
        dir.join("meta.toml"),
        format!(
            "name = {name:?}\nkind = \"shell\"\nmode = \"copy\"\nsource = \"/deleted/{slug}.sh\"\nworkdir = \"invoke\"\n{extra_meta}"
        ),
    )
    .unwrap();
    register(root, slug);
}

/// A shell entry whose managed params are injected (the uv-free twin of a python INJECT-const
/// fixture built by the oracle's `metawriter.write_params`).
fn inject_shell(root: &TempDir, slug: &str, name: &str, body: &str, params: &[ParamDecl]) {
    let source = write_managed_params("shell", body, params).unwrap();
    shell_entry(root, slug, name, &source, "");
}

/// Read `state/values/<slug>.toml` back as text (the oracle's `argstate.load_state(slug)`).
fn state_text(root: &TempDir, slug: &str) -> Option<String> {
    fs::read_to_string(
        root.path()
            .join("state/values")
            .join(format!("{slug}.toml")),
    )
    .ok()
}

/// Seed a state file directly (the oracle's `argstate.save_last` / `save_preset`).
fn seed_state(root: &TempDir, slug: &str, body: &str) {
    let values = root.path().join("state/values");
    fs::create_dir_all(&values).unwrap();
    fs::write(values.join(format!("{slug}.toml")), body).unwrap();
}

/// Install a fake `uv` on a bin dir under the sandbox; return the bin dir for `PATH`
/// (the black-box twin of the oracle's `monkeypatch(find_uv, lambda: "/usr/bin/uv")`).
fn install_uv(root: &TempDir) -> PathBuf {
    uv_in(root, "uvbin")
}

/// Install a fake `uv` inside a named bin dir under the sandbox.
fn uv_in(root: &TempDir, dirname: &str) -> PathBuf {
    let bin = root.path().join(dirname);
    fs::create_dir_all(&bin).unwrap();
    let uv = bin.join("uv");
    fs::write(&uv, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&uv, fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

/// A `PATH` that finds our fake `uv` plus the real shells.
fn path_with(bin: &std::path::Path) -> String {
    format!("{}:/usr/bin:/bin", bin.display())
}

/// True when the effective user bypasses POSIX permission bits (root reads a `0o000` file). Mirrors
/// the oracle's `skipif(os.geteuid() == 0)`.
fn bypasses_permissions(root: &TempDir) -> bool {
    let probe = root.path().join(".permcheck");
    fs::write(&probe, "x").unwrap();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o000)).unwrap();
    let readable = fs::read(&probe).is_ok();
    fs::set_permissions(&probe, fs::Permissions::from_mode(0o644)).unwrap();
    readable
}

/// An inject-const string parameter (oracle `ParamDecl(name, binding="const", type="str", ...)`).
fn const_str(name: &str, default: Option<&str>) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    if let Some(value) = default {
        declaration.default = Some(ParameterValue::String(value.to_owned()));
    }
    declaration
}

/// An inject-const integer parameter (oracle `ParamDecl(name, binding="const", type="int", ...)`).
fn const_int(name: &str, default: Option<i64>) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Int;
    if let Some(value) = default {
        declaration.default = Some(ParameterValue::Integer(value));
    }
    declaration
}

// --------------------------------------------------------------------------
// main callback
// --------------------------------------------------------------------------

#[test]
fn test_version_flag_prints_and_exits() {
    // The integration test shares the `skit-cli-rs` package, so CARGO_PKG_VERSION equals the
    // binary's own version — the black-box twin of the oracle's `skit.__version__`.
    let root = sandbox();
    let (code, out) = run(skit(&root).arg("--version"));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(env!("CARGO_PKG_VERSION")), "{out}");
}

#[test]
#[ignore = "cross-crate: the oracle monkeypatches `skit.tui.run_menu` and asserts a bare `skit` (no subcommand) dispatched to it. The Rust dispatch is the composition root (cli.rs -> `skit tui`) wired to skit-tui's `run_menu`; a non-tty black-box harness cannot witness that the menu function was CALLED, and driving the real Ratatui menu needs a live terminal. Owning seam: skit-tui menu + cli.rs no-subcommand branch."]
fn test_no_subcommand_dispatches_to_tui() {
    // Bare `skit` runs the TUI menu.
}

// --------------------------------------------------------------------------
// add
// --------------------------------------------------------------------------

#[test]
fn test_add_python_copy() {
    let root = sandbox();
    let path = write_src(&root, "job.py", "print(1)\n");
    let (code, out) = run(skit(&root).arg("add").arg(&path).args(["--name", "hi"]));
    assert_eq!(code, 0, "{out}");
    let (_c, json) = run(skit(&root).args(["list", "--json"]));
    assert!(json.contains("\"name\":\"hi\""), "{json}");
    assert!(json.contains("\"mode\":\"copy\""), "{json}");
}

#[test]
#[ignore = "cross-crate: in a real terminal with form=tui, `skit add x.py` hosts the SAME review panel the TUI's `a` opens (oracle monkeypatches `skit.tui_add.run_add_review` and reads the ride-along prefills). The panel + prefill wiring is skit-tui (screens/add.rs) fed by add_review_defaults (cli.rs); a non-tty binary harness cannot open or observe it."]
fn test_add_interactive_tui_form_opens_review_panel() {
    // The flags ride along as prefills; the panel's slug feeds the printed summary.
}

#[test]
#[ignore = "cross-crate: Esc in the tui review panel = exit 130, nothing added. The panel is skit-tui (needs a live terminal); the cancel message/code is CliError::AddCancelled (cli.rs). The end-to-end 130 cancel is covered by the pty suite; this one drives the panel seam."]
fn test_add_interactive_panel_cancel_exits_130() {
    // Cancelled — nothing was added.
}

#[test]
#[ignore = "cross-crate: form=plain opts out of the panel and runs the interactive LINE prompts (\"Description (optional)\"). Line prompts need a tty + Prompt.ask; the seam is cli.rs's inline plain-form path, unit-driven by src/cli/tests.rs, not a non-tty binary."]
fn test_add_interactive_plain_form_keeps_line_prompts() {
    // Line-prompt path runs instead of the panel.
}

#[test]
#[ignore = "cross-crate: TERM=dumb can't host a Textual panel, so the line-prompt path runs — same interactive tty seam as test_add_interactive_plain_form_keeps_line_prompts."]
fn test_add_term_dumb_keeps_line_prompts() {
    // TERM=dumb -> line prompts, not the panel.
}

#[test]
fn test_add_python_reference_skips_onboarding() {
    let root = sandbox();
    let path = write_src(&root, "ref.py", "CITY = \"x\"\nprint(CITY)\n");
    let (code, out) = run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "ref", "--ref"]));
    assert_eq!(code, 0, "{out}");
    let (_c, json) = run(skit(&root).args(["list", "--json"]));
    assert!(json.contains("\"mode\":\"reference\""), "{json}");
}

#[test]
fn test_add_rejects_non_py() {
    let root = sandbox();
    let path = write_src(&root, "notes.txt", "data");
    let (code, out) = run(skit(&root).arg("add").arg(&path));
    assert_eq!(code, 2);
    let flat = flat(&out);
    assert!(
        flat.contains("pass --kind <language> for an extensionless script"),
        "{flat}"
    );
}

#[test]
fn test_add_needs_path() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).arg("add"));
    assert_eq!(code, 2);
}

#[test]
fn test_add_exe_needs_path() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["add", "--exe"]));
    assert_eq!(code, 2);
}

#[test]
fn test_add_exe() {
    let root = sandbox();
    let exe = write_src(&root, "tool", "#!/bin/sh\necho hi\n");
    let (code, out) = run(skit(&root)
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--name", "tool"]));
    assert_eq!(code, 0, "{out}");
    let (_c, json) = run(skit(&root).args(["list", "--json"]));
    assert!(json.contains("\"kind\":\"exe\""), "{json}");
}

#[test]
#[ignore = "cross-crate: the exe add lane, in a terminal under form=plain, line-asks the name (default: the file stem) and a description via Prompt.ask (oracle stubs cli.Prompt.ask and reads which prompts fired). Line prompts are the interactive tty seam (cli.rs inline plain form); a non-tty binary never asks."]
fn test_add_exe_interactive_line_asks_name_and_description() {
    // Name in skit + Description (optional).
}

#[test]
fn test_add_exe_interactive_skips_asks_when_name_and_description_given() {
    // Both --name and --description supplied: each ask is skipped and both flags stand. The
    // non-tty harness never asks regardless; the OUTCOME (the flags win) is what converges.
    let root = sandbox();
    let exe = write_src(&root, "backup", "#!/bin/sh\necho hi\n");
    let (code, out) = run(skit(&root).arg("add").arg(&exe).args([
        "--exe",
        "--name",
        "given",
        "--description",
        "prewritten",
    ]));
    assert_eq!(code, 0, "{out}");
    let (_c, json) = run(skit(&root).args(["list", "--json"]));
    assert!(json.contains("\"kind\":\"exe\""), "{json}");
    assert!(json.contains("\"description\":\"prewritten\""), "{json}");
}

#[test]
fn test_add_exe_no_input_never_asks() {
    // --no-input keeps the deterministic contract: no prompts, so the file stem becomes the name.
    let root = sandbox();
    let exe = write_src(&root, "archiver", "#!/bin/sh\necho hi\n");
    let (code, out) = run(skit(&root)
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    let (_c, json) = run(skit(&root).args(["list", "--json"]));
    assert!(json.contains("\"name\":\"archiver\""), "{json}");
    assert!(json.contains("\"kind\":\"exe\""), "{json}");
}

#[test]
fn test_add_exe_missing_path_errors_before_any_ask() {
    let root = sandbox();
    let missing = root.path().join("ghost.bin");
    let (code, out) = run(skit(&root).arg("add").arg(&missing).arg("--exe"));
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("File not found"), "{out}");
}

#[test]
fn test_add_cmd_needs_name() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["add", "--cmd", "echo hi"]));
    assert_eq!(code, 2);
}

#[test]
fn test_add_cmd_with_params() {
    // A `{msg}` placeholder becomes a managed param; `skit params` names it.
    let root = sandbox();
    let (code, out) = run(skit(&root).args(["add", "--cmd", "echo {msg}", "--name", "e"]));
    assert_eq!(code, 0, "{out}");
    let (_c, params) = run(skit(&root).args(["params", "e"]));
    assert!(params.contains("msg"), "{params}");
}

#[test]
fn test_add_with_explicit_deps_records() {
    let root = sandbox();
    let path = write_src(&root, "r.py", "import requests\nprint(requests)\n");
    let (code, out) = run(skit(&root).arg("add").arg(&path).args([
        "--name",
        "r",
        "--dep",
        "requests",
        "--dep",
        "rich",
        "--no-input",
    ]));
    assert_eq!(code, 0, "{out}");
}

#[test]
fn test_add_name_conflict_errors() {
    let root = sandbox();
    let path = write_src(&root, "job.py", "print(1)\n");
    run(skit(&root).arg("add").arg(&path).args(["--name", "dup"]));
    let (code, _out) = run(skit(&root).arg("add").arg(&path).args(["--name", "dup"]));
    assert_eq!(code, 1);
}

#[test]
fn test_add_missing_path_clean_error_not_traceback() {
    let root = sandbox();
    let missing = root.path().join("typo").join("path.py");
    let (code, out) = run(skit(&root).arg("add").arg(&missing));
    assert_eq!(code, 1);
    assert!(out.contains("File not found"), "{out}");
}

#[test]
fn test_add_directory_path_clean_error_not_traceback() {
    let root = sandbox();
    let dir = root.path().join("adir.py");
    fs::create_dir(&dir).unwrap();
    let (code, out) = run(skit(&root).arg("add").arg(&dir));
    assert_eq!(code, 1);
    assert!(out.contains("Not a file"), "{out}");
    assert!(!out.contains("--exe"), "{out}");
}

#[test]
fn test_add_unknown_directory_suggests_exe_and_exits_usage() {
    let root = sandbox();
    let dir = root.path().join("plainbundle");
    fs::create_dir(&dir).unwrap();
    let data_before = snapshot_tree(&root.path().join("data"));
    let state_before = snapshot_tree(&root.path().join("state"));
    let config_before = snapshot_tree(&root.path().join("config"));
    let (code, out) = run(skit(&root).arg("add").arg(&dir));
    assert_eq!(code, 2);
    assert!(out.contains("is a directory"), "{out}");
    assert!(out.contains("--exe"), "{out}");
    assert!(!out.contains("Not a file"), "{out}");
    assert!(dir.is_dir());
    assert_eq!(snapshot_tree(&root.path().join("data")), data_before);
    assert_eq!(snapshot_tree(&root.path().join("state")), state_before);
    assert_eq!(snapshot_tree(&root.path().join("config")), config_before);
}

#[test]
fn test_add_unknown_directory_with_exe_is_accepted() {
    // The escape the message advertises actually works: --exe adds the directory.
    let root = sandbox();
    let dir = root.path().join("plainbundle2");
    fs::create_dir(&dir).unwrap();
    let (code, out) = run(skit(&root)
        .arg("add")
        .arg(&dir)
        .args(["--exe", "--name", "bundled"]));
    assert_eq!(code, 0, "{out}");
    let (_c, json) = run(skit(&root).args(["list", "--json"]));
    assert!(json.contains("\"name\":\"bundled\""), "{json}");
    assert!(json.contains("\"kind\":\"exe\""), "{json}");
}

#[test]
fn test_add_unreadable_file_clean_error_not_traceback() {
    // An existing-but-unreadable file must be reported cleanly ("Can't read", distinct from
    // "File not found" since the path exists). Skipped when the euid bypasses perms — root reads
    // through chmod 0o000, exactly the oracle's `skipif(geteuid() == 0)`.
    let root = sandbox();
    if bypasses_permissions(&root) {
        return;
    }
    let path = write_src(&root, "job.py", "print(1)\n");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let (code, out) = run(skit(&root).arg("add").arg(&path));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(code, 1);
    assert!(out.contains("Can't read"), "{out}");
}

#[test]
#[ignore = "cross-crate: the oracle monkeypatches Path.read_text to raise PermissionError mid-add, exercising cli.py's `except OSError` read guard regardless of euid. A black-box binary harness cannot inject a read fault into skit's own process; the guard is cli.rs's read-error branch. Owning seam: cli.rs add read guard."]
fn test_add_read_error_reports_clean_message() {
    // A mid-add read failure surfaces as a localized "Can't read", never a traceback.
}

#[test]
fn test_add_onboards_params_non_interactive_skips() {
    // --no-input: even when candidates exist, select none and write no [tool.skit].
    let root = sandbox();
    let path = write_src(&root, "job.py", "CITY = \"Taipei\"\nprint(CITY)\n");
    let (code, out) = run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "j", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    let stored = fs::read_to_string(root.path().join("data/scripts/j/script.py")).unwrap();
    assert!(!stored.contains("[tool.skit]"), "{stored}");
}

// --------------------------------------------------------------------------
// list
// --------------------------------------------------------------------------

#[test]
fn test_list_empty() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).arg("list"));
    assert_eq!(code, 0);
}

#[test]
fn test_list_table() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, out) = run(skit(&root).arg("list"));
    assert_eq!(code, 0);
    assert!(out.contains('a'), "{out}");
}

#[test]
fn test_list_json() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, out) = run(skit(&root).args(["list", "--json"]));
    assert_eq!(code, 0);
    assert!(out.contains("\"slug\""), "{out}");
}

#[test]
fn test_list_table_marks_missing_target() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "gone", "--no-input"]));
    fs::remove_file(root.path().join("data/scripts/gone/script.py")).unwrap();
    let (code, out) = run(skit(&root).arg("list"));
    assert_eq!(code, 0);
    assert!(out.contains("missing"), "{out}"); // path itself may be truncated by column width
}

#[test]
fn test_list_table_does_not_mark_healthy_or_command_entries() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "healthy", "--no-input"]));
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "cmdok"]));
    let (code, out) = run(skit(&root).arg("list"));
    assert_eq!(code, 0);
    assert!(!out.contains("missing"), "{out}");
}

#[test]
fn test_list_json_missing_field() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "gone", "--no-input"]));
    fs::remove_file(root.path().join("data/scripts/gone/script.py")).unwrap();
    let (code, out) = run(skit(&root).args(["list", "--json"]));
    assert_eq!(code, 0);
    assert!(out.contains("\"missing\":true"), "{out}"); // compact JSON, no space after ':'
}

#[test]
#[ignore = "cross-crate: direct unit test of the CLI-private `cli._list_description` (cli.py:2300) — asserts the exact Rich-escaped cell string `[dim]⚠ missing: <path>[/dim]`. The Rust twin is a private `fn list_description` (cli.rs:2074) with no Rich layer and no `pub` surface; the end-to-end marker is covered by test_list_table_marks_missing_target. Owning seam: cli.rs list_description."]
fn test_list_description_exact_marker_when_no_description() {
    // No description -> the cell is exactly the dim missing marker.
}

#[test]
fn test_list_and_show_human_faces_use_translated_kind_labels() {
    // Human faces show the translated LABEL (Python/Prompt/Program); --json keeps raw ids.
    let root = sandbox();
    let py = write_src(&root, "pyjob.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&py)
        .args(["--name", "pyjob", "--no-input"]));
    let pr = write_src(&root, "p.prompt.md", "Do {{a}}\n");
    run(skit(&root)
        .arg("add")
        .arg(&pr)
        .args(["--name", "pr", "--no-input"]));
    let exe = write_src(&root, "tool", "#!/bin/sh\necho hi\n");
    run(skit(&root)
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--name", "prog"]));

    let (_c, listed) = run(skit(&root).arg("list"));
    for label in ["Python", "Prompt", "Program"] {
        assert!(listed.contains(label), "{listed}"); // the Kind column renders the label…
    }
    assert!(!listed.contains("python"), "{listed}"); // …never the raw id (label is capitalized)

    let (_p, show_py) = run(skit(&root).args(["show", "pyjob"]));
    assert!(show_py.contains("Python ·"), "{show_py}");
    let (_r, show_pr) = run(skit(&root).args(["show", "pr"]));
    assert!(show_pr.contains("Prompt ·"), "{show_pr}");
    let (_g, show_prog) = run(skit(&root).args(["show", "prog"]));
    assert!(show_prog.contains("Program ·"), "{show_prog}");

    let (_j, json) = run(skit(&root).args(["list", "--json"]));
    let payload: Value = serde_json::from_str(json.trim()).expect("json array");
    let kind_of = |name: &str| -> Option<String> {
        payload
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["name"].as_str() == Some(name))
            .and_then(|row| row["kind"].as_str().map(str::to_owned))
    };
    assert_eq!(kind_of("pyjob").as_deref(), Some("python"));
    assert_eq!(kind_of("pr").as_deref(), Some("prompt"));
    assert_eq!(kind_of("prog").as_deref(), Some("exe"));
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._list_description` (cli.py:2300) — asserts the exact `<desc>  [dim]⚠ missing: <path>[/dim]` join. The Rust `fn list_description` (cli.rs:2074) is private and Rich-free; end-to-end marker coverage is test_list_table_marks_missing_target."]
fn test_list_description_appends_marker_after_description() {
    // Description + two-space + dim missing marker.
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._list_description` (cli.py:2300) — a healthy entry returns its description, a bare command returns \"—\". The Rust `fn list_description` (cli.rs:2074) is private with no `pub` surface."]
fn test_list_description_healthy_and_command_entries_untouched() {
    // Healthy -> the description; bare -> "—".
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._list_description` asserting Rich markup in a description renders escaped (`\\[red]DANGER\\[/red]`). Rust has no Rich markup layer at all, so there is no escape to observe on a private function; the literal-render behavior is covered end to end by test_list_table_name_column_escapes_markup / test_list_table_renders_markup_literally_end_to_end."]
fn test_list_description_escapes_markup_in_description() {
    // A markup description renders as literal text.
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._list_description` asserting the missing-path marker Rich-escapes a hostile path. Rust has no Rich layer and the function is private; literal rendering is covered end to end by test_list_table_renders_markup_literally_end_to_end."]
fn test_list_description_escapes_markup_in_missing_path() {
    // A markup path renders escaped in the missing marker.
}

#[test]
fn test_list_table_renders_markup_literally_end_to_end() {
    // Both a markup description and a markup missing path render as literal text in `skit list`.
    let root = sandbox();
    let dir = root.path().join("[red]boom[bold]");
    fs::create_dir(&dir).unwrap();
    let exe = dir.join("tool");
    fs::write(&exe, "#!/bin/sh\n").unwrap();
    run(skit(&root).arg("add").arg(&exe).args([
        "--exe",
        "--name",
        "mkup-path",
        "--description",
        "[blue]hi[/blue]",
    ]));
    fs::remove_file(&exe).unwrap();
    let (code, out) = run(skit(&root).arg("list"));
    assert_eq!(code, 0);
    assert!(out.contains("[blue]hi[/blue]"), "{out}");
    assert!(out.contains("missing"), "{out}");
}

#[test]
fn test_list_table_name_column_escapes_markup() {
    // A NAME containing markup renders literally in the Name column.
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "[blue]hi[/blue]"]));
    let (code, out) = run(skit(&root).arg("list"));
    assert_eq!(code, 0);
    assert!(out.contains("[blue]hi[/blue]"), "{out}");
}

// --------------------------------------------------------------------------
// remove
// --------------------------------------------------------------------------

#[test]
fn test_remove_not_found() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["remove", "ghost"]));
    assert_eq!(code, 1);
}

#[test]
fn test_remove_with_yes() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["remove", "a", "--yes"]));
    assert_eq!(code, 0);
    let (gone, _o) = run(skit(&root).args(["show", "a"]));
    assert_ne!(gone, 0); // resolve now raises NotFoundError
}

#[test]
fn test_remove_confirm_abort() {
    // Aborting the confirm keeps the entry. The oracle pipes "n"; the non-tty Rust binary cannot
    // read a confirm from a pipe and refuses instead (exit 2, still != 0) — the SAME two facts the
    // oracle asserts: a non-zero exit and the entry retained.
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["remove", "a"]).write_stdin("n\n"));
    assert_ne!(code, 0); // abort
    let (still, _o) = run(skit(&root).args(["show", "a"]));
    assert_eq!(still, 0); // still there
}

// --------------------------------------------------------------------------
// run
// --------------------------------------------------------------------------

#[test]
fn test_run_python_with_params_injects() {
    // A managed value exists -> an injected temp copy is made (the `→ inject:` line). Substitution:
    // a python INJECT-const fixture becomes a shell inject entry (uv-free).
    let root = sandbox();
    inject_shell(
        &root,
        "j",
        "J",
        "CITY=Taipei\nprintf '%s\\n' \"$CITY\"\n",
        &[const_str("CITY", Some("Taipei"))],
    );
    seed_state(&root, "j", "[values]\nCITY = \"Kaohsiung\"\n");
    let (code, out) = run(skit(&root).args(["run", "j", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains(INJECT_MARKER), "{out}");
}

#[test]
fn test_run_extra_args_bypass_required_field_validation() {
    let root = sandbox();
    let path = write_src(&root, "ar.py", ARGPARSE_REQUIRED);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "ar", "--kind", "python", "--no-input"]));
    let (code, out) = run(skit(&root).env("PATH", "").args([
        "run",
        "ar",
        "--no-input",
        "--dry-run",
        "--",
        "-o",
        "x.png",
    ]));
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("-o x.png") || out.contains("--output x.png"),
        "{out}"
    );
}

#[test]
fn test_run_required_field_missing_without_extra_args_exits_125() {
    // Observed offline through --dry-run (PATH="" — the field-validation gate runs before uv).
    let root = sandbox();
    let path = write_src(&root, "ar2.py", ARGPARSE_REQUIRED);
    run(skit(&root).arg("add").arg(&path).args([
        "--name",
        "ar2",
        "--kind",
        "python",
        "--no-input",
    ]));
    let (code, out) =
        run(skit(&root)
            .env("PATH", "")
            .args(["run", "ar2", "--no-input", "--dry-run"]));
    assert_eq!(code, 125, "{out}");
    assert!(out.contains("output"), "{out}");
}

#[test]
fn test_run_remembered_extra_args_do_not_bypass_required_field_validation() {
    let root = sandbox();
    let path = write_src(&root, "ar3.py", ARGPARSE_REQUIRED);
    run(skit(&root).arg("add").arg(&path).args([
        "--name",
        "ar3",
        "--kind",
        "python",
        "--no-input",
    ]));
    seed_state(&root, "ar3", "extra_args = [\"-o\", \"x.png\"]\n");

    let (code, out) =
        run(skit(&root)
            .env("PATH", "")
            .args(["run", "ar3", "--no-input", "--dry-run"]));

    assert_eq!(code, 125, "{out}");
    assert!(out.contains("output"), "{out}");
}

#[test]
fn test_run_not_found_exits_127() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["run", "ghost"]));
    assert_eq!(code, 127); // docker convention: target not found
}

#[test]
fn test_run_raw_skips_form() {
    // --raw: no injection, so no `→ inject:` line. Substitution: shell inject entry (uv-free).
    let root = sandbox();
    inject_shell(
        &root,
        "j",
        "J",
        "CITY=Taipei\nprintf '%s\\n' \"$CITY\"\n",
        &[const_str("CITY", Some("Taipei"))],
    );
    let (code, out) = run(skit(&root).args(["run", "j", "--raw", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains(INJECT_MARKER), "{out}"); // --raw: no injection
}

#[test]
fn test_run_unknown_preset_rejected() {
    let root = sandbox();
    let path = write_src(&root, "j.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "j", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["run", "j", "--preset", "nope", "--no-input"]));
    assert_eq!(code, 2);
}

#[test]
fn test_run_passes_and_remembers_extra_args() {
    // The argv tail is passed through and remembered for the next argless run. Substitution: a
    // `printf '%s\n' "$@"` shell body echoes the tail (the uv-free witness for the spy's `extra`).
    let root = sandbox();
    shell_entry(&root, "j", "J", "printf '%s\\n' \"$@\"\n", "");
    let (code1, out1) = run(skit(&root).args(["run", "j", "--no-input", "--", "--flag", "v"]));
    assert_eq!(code1, 0, "{out1}");
    assert!(out1.contains("--flag") && out1.contains('v'), "{out1}");
    let (code2, out2) = run(skit(&root).args(["run", "j", "--no-input"]));
    assert_eq!(code2, 0, "{out2}");
    assert!(out2.contains("--flag") && out2.contains('v'), "{out2}"); // replayed
}

#[test]
fn test_run_command_reuses_last_extra_args() {
    // A command template remembers its appended tail too (docs/design/prompt.md v3.1): passing none
    // replays it, an explicit tail overrides. The tail rides on the command line skit runs.
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo ready", "--name", "cmd"]));
    let (c1, first) = run(skit(&root).args(["run", "cmd", "--no-input", "--", "--loud"]));
    assert_eq!(c1, 0, "{first}");
    assert!(first.contains("--loud"), "{first}");
    let (c2, second) = run(skit(&root).args(["run", "cmd", "--no-input"]));
    assert_eq!(c2, 0, "{second}");
    assert!(second.contains("--loud"), "{second}"); // replayed
    let (c3, third) = run(skit(&root).args(["run", "cmd", "--no-input", "--", "--quiet"]));
    assert_eq!(c3, 0, "{third}");
    assert!(
        third.contains("--quiet") && !third.contains("--loud"),
        "{third}"
    ); // overridden
}

#[test]
fn test_run_nonzero_exit_propagates() {
    // The script's own exit code is propagated (docker convention: 1-124 belong to the script).
    let root = sandbox();
    shell_entry(&root, "j", "J", "exit 3\n", "");
    let (code, _out) = run(skit(&root).args(["run", "j", "--no-input"]));
    assert_eq!(code, 3);
}

#[test]
#[ignore = "cross-crate: the oracle monkeypatches `shim.inject` to raise ShimError and asserts the skit-side injection failure maps to exit 125 (not the script's own code). A black-box binary cannot inject a shim failure into skit's own process; the mapping is run/command.rs's inject-error branch. Owning seam: run injection error mapping."]
fn test_run_shim_error() {
    // A skit-side injection failure -> exit 125.
}

#[test]
fn test_run_bad_typed_value_caught_at_validation() {
    let root = sandbox();
    let marker = root.path().join("typed-value-child.ran");
    inject_shell(
        &root,
        "j",
        "J",
        &format!(
            "RETRIES=3\nprintf child > {}\nprintf '%s\\n' \"$RETRIES\"\n",
            marker.display()
        ),
        &[const_int("RETRIES", Some(3))],
    );
    seed_state(&root, "j", "[values]\nRETRIES = \"not-a-number\"\n");
    let data_before = snapshot_tree(&root.path().join("data"));
    let state_before = snapshot_tree(&root.path().join("state"));
    let config_before = snapshot_tree(&root.path().join("config"));

    let (code, out) = run(skit(&root).args(["run", "j", "--no-input"]));

    assert_eq!(code, 125);
    assert!(out.contains("not-a-number"), "{out}");
    assert!(out.contains("whole number"), "{out}");
    assert!(!out.to_lowercase().contains("resync"), "{out}");
    assert!(!marker.exists(), "validation reached the child process");
    assert_eq!(snapshot_tree(&root.path().join("data")), data_before);
    assert_eq!(snapshot_tree(&root.path().join("state")), state_before);
    assert_eq!(snapshot_tree(&root.path().join("config")), config_before);
}

#[test]
#[ignore = "cross-crate: the oracle monkeypatches `launcher.run_entry` to raise LaunchError (a skit-side launch failure) and asserts exit 125. A black-box binary cannot inject that internal error; the nearest REAL fault — a pinned nonexistent interpreter — is a program-not-found that Rust maps to 126, not the skit-side 125. Owning seam: run launch-error mapping (run/command.rs)."]
fn test_run_launch_error() {
    // A skit-side launch failure -> exit 125.
}

#[test]
fn test_run_command_entry_collects_values() {
    // A command entry's placeholder is filled from the stored value and reaches the shell.
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo {msg}", "--name", "e"]));
    seed_state(&root, "e", "[values]\nmsg = \"hi\"\n");
    let (code, out) = run(skit(&root).args(["run", "e", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("hi"), "{out}");
}

// --------------------------------------------------------------------------
// preset
// --------------------------------------------------------------------------

#[test]
fn test_preset_list_none() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["preset", "list", "a"]));
    assert_eq!(code, 0);
}

#[test]
fn test_preset_list_shows() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    seed_state(&root, "a", "[presets.prod]\nCITY = \"Taipei\"\n");
    let (code, out) = run(skit(&root).args(["preset", "list", "a"]));
    assert_eq!(code, 0);
    assert!(out.contains("prod"), "{out}");
}

#[test]
fn test_preset_list_not_found() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["preset", "list", "ghost"]));
    assert_eq!(code, 1);
}

#[test]
fn test_preset_delete() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    seed_state(&root, "a", "[presets.prod]\nCITY = \"Taipei\"\n");
    let (code, _out) = run(skit(&root).args(["preset", "delete", "a", "prod"]));
    assert_eq!(code, 0);
    let state = state_text(&root, "a").unwrap_or_default();
    assert!(!state.contains("[presets.prod]"), "{state}"); // presets now empty
}

#[test]
fn test_preset_delete_unknown() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["preset", "delete", "a", "nope"]));
    assert_eq!(code, 1);
}

#[test]
fn test_preset_delete_not_found() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["preset", "delete", "ghost", "p"]));
    assert_eq!(code, 1);
}

#[test]
fn test_preset_save_not_found() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["preset", "save", "ghost", "p"]));
    assert_eq!(code, 1);
}

#[test]
fn test_preset_save_python_no_params() {
    // A field-less entry has nothing to save: USAGE (2), matching `run --save-preset`.
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root)
        .args(["preset", "save", "a", "p"])
        .write_stdin("\n"));
    assert_eq!(code, 2); // no managed parameters
}

#[test]
fn test_preset_save_command_no_params() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "e"])); // no placeholders
    let (code, _out) = run(skit(&root).args(["preset", "save", "e", "p"]));
    assert_eq!(code, 2);
}

#[test]
#[ignore = "cross-crate: the oracle calls `cli.preset_save(...)` directly with a tty + stubbed Prompt.ask to collect the placeholder value interactively (CliRunner cannot inject a tty). A command WITH placeholders needs interactive value collection; the non-tty binary has no way to answer, and the seam is cli.rs's inline preset-save form (unit-driven by src/cli/tests.rs)."]
fn test_preset_save_command_with_params() {
    // Interactive collection of {msg} -> preset {msg: hello}.
}

// --------------------------------------------------------------------------
// params
// --------------------------------------------------------------------------

#[test]
fn test_params_not_found() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["params", "ghost"]));
    assert_eq!(code, 1);
}

#[test]
fn test_params_empty() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["params", "a"]));
    assert_eq!(code, 0);
}

#[test]
fn test_params_command_entry() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo {msg}", "--name", "e"]));
    seed_state(&root, "e", "[values]\nmsg = \"hi\"\n");
    let (code, out) = run(skit(&root).args(["params", "e"]));
    assert_eq!(code, 0);
    assert!(out.contains("msg"), "{out}");
}

#[test]
fn test_params_command_no_placeholders() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "e"]));
    let (code, _out) = run(skit(&root).args(["params", "e"]));
    assert_eq!(code, 0);
}

#[test]
fn test_params_python_table_with_secret() {
    let root = sandbox();
    let block = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"API\"\n# binding = \"const\"\n# type = \"str\"\n# default = \"x\"\n# secret = true\n# ///\nAPI = \"x\"\nprint(API)\n";
    let path = write_src(&root, "a.py", block);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--kind", "python", "--no-input"]));
    seed_state(&root, "a", "[values]\nAPI = \"shown\"\n");
    let (code, out) = run(skit(&root).args(["params", "a"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("API"), "{out}");
}

#[test]
fn test_params_secret_purges_stored_last_value_and_presets() {
    let root = sandbox();
    let block = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"API_KEY\"\n# binding = \"const\"\n# type = \"str\"\n# default = \"x\"\n# ///\nAPI_KEY = \"x\"\nprint(API_KEY)\n";
    let path = write_src(&root, "a.py", block);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--kind", "python", "--no-input"]));
    seed_state(
        &root,
        "a",
        "[values]\nAPI_KEY = \"plaintext-secret-123\"\n\n[presets.prod]\nAPI_KEY = \"plaintext-secret-123\"\n",
    );
    let (code, out) = run(skit(&root).args(["params", "a", "--secret", "API_KEY"]));
    assert_eq!(code, 0, "{out}");
    assert!(!out.contains("plaintext-secret-123"), "{out}");
    let normalized = flat(&out);
    assert!(
        normalized.contains(
            "Removed previously stored plaintext value(s) for now-secret parameter(s): API_KEY"
        ),
        "{normalized}"
    );
    let state = state_text(&root, "a").unwrap_or_default();
    assert!(!state.contains("API_KEY"), "{state}");
    assert!(!state.contains("prod"), "{state}"); // emptied preset dropped entirely
    // The plaintext must be gone from disk, not merely hidden from a reader.
    let values_dir = root.path().join("state/values");
    for entry in fs::read_dir(&values_dir).unwrap() {
        let bytes = fs::read_to_string(entry.unwrap().path()).unwrap();
        assert!(!bytes.contains("plaintext-secret-123"), "{bytes}");
    }
}

#[test]
fn test_params_secret_purge_message_sorts_multiple_names() {
    let root = sandbox();
    let block = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "A"
# binding = "const"
# type = "str"
#
# [[tool.skit.params]]
# name = "B"
# binding = "const"
# type = "str"
# ///
A = "x"
B = "y"
print(A, B)
"#;
    let path = write_src(&root, "sorted.py", block);
    run(skit(&root).arg("add").arg(&path).args([
        "--name",
        "sorted",
        "--kind",
        "python",
        "--no-input",
    ]));
    seed_state(&root, "sorted", "[values]\nA = \"first\"\nB = \"second\"\n");

    let (code, out) = run(skit(&root).args(["params", "sorted", "--secret", "B", "--secret", "A"]));

    assert_eq!(code, 0, "{out}");
    assert!(
        flat(&out).contains(
            "Removed previously stored plaintext value(s) for now-secret parameter(s): A, B"
        ),
        "{out}"
    );
}

#[test]
fn test_params_secret_purge_json_stays_one_document() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "json-secret"]));
    let (code, out) = run(skit(&root).args([
        "params",
        "json-secret",
        "--add",
        "TOKEN",
        "--deliver",
        "TOKEN=env",
    ]));
    assert_eq!(code, 0, "{out}");
    seed_state(&root, "json-secret", "[values]\nTOKEN = \"plaintext\"\n");

    let output = skit(&root)
        .args(["params", "json-secret", "--secret", "TOKEN", "--json"])
        .output()
        .expect("skit runs");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout is not one JSON document: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    assert_eq!(payload["parameters"][0]["secret"], true);
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("Removed previously stored plaintext")
    );
}

#[test]
fn test_params_secret_does_not_purge_other_still_public_params() {
    let root = sandbox();
    let block = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"API_KEY\"\n# binding = \"const\"\n# type = \"str\"\n#\n# [[tool.skit.params]]\n# name = \"CITY\"\n# binding = \"const\"\n# type = \"str\"\n# ///\nAPI_KEY = \"x\"\nCITY = \"y\"\nprint(API_KEY, CITY)\n";
    let path = write_src(&root, "a.py", block);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--kind", "python", "--no-input"]));
    seed_state(
        &root,
        "a",
        "[values]\nAPI_KEY = \"secretval\"\nCITY = \"Taipei\"\n",
    );
    let (code, out) = run(skit(&root).args(["params", "a", "--secret", "API_KEY"]));
    assert_eq!(code, 0, "{out}");
    let state = state_text(&root, "a").unwrap();
    assert!(!state.contains("API_KEY"), "{state}");
    assert!(state.contains("CITY = \"Taipei\""), "{state}"); // untouched
}

#[test]
fn test_params_edit_without_stored_value_prints_no_purge_message() {
    // Nothing was ever stored for CITY, so marking it secret has nothing to purge — the purge
    // message must not appear.
    let root = sandbox();
    let block = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"CITY\"\n# binding = \"const\"\n# type = \"str\"\n# ///\nCITY = \"x\"\nprint(CITY)\n";
    let path = write_src(&root, "a.py", block);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--kind", "python", "--no-input"]));
    let (code, out) = run(skit(&root).args(["params", "a", "--secret", "CITY"]));
    assert_eq!(code, 0, "{out}");
    assert!(
        !out.contains("Removed previously stored plaintext"),
        "{out}"
    );
}

// --------------------------------------------------------------------------
// deps
// --------------------------------------------------------------------------

#[test]
fn test_deps_view() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["deps", "a"]));
    assert_eq!(code, 0);
}

#[test]
fn test_deps_not_found() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).args(["deps", "ghost"]));
    assert_eq!(code, 1);
}

#[test]
fn test_deps_not_python() {
    // The PEP 723 dependency flavor is python-only: --dep on a command entry is a usage error (2).
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "e"]));
    let (code, _out) = run(skit(&root).args(["deps", "e", "--dep", "requests"]));
    assert_eq!(code, 2);
}

#[test]
fn test_deps_set() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, out) = run(skit(&root).args([
        "deps", "a", "--dep", "requests", "--dep", "rich", "--python", ">=3.11",
    ]));
    assert_eq!(code, 0, "{out}");
    let (_c, view) = run(skit(&root).args(["deps", "a"]));
    assert!(view.contains("requests") && view.contains("rich"), "{view}");
}

#[test]
fn test_deps_view_with_requires_python() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    run(skit(&root).args(["deps", "a", "--dep", "requests", "--python", ">=3.12"]));
    let (code, out) = run(skit(&root).args(["deps", "a"]));
    assert_eq!(code, 0);
    assert!(out.contains("3.12"), "{out}");
}

#[test]
fn test_deps_command_strips_a_whitespace_only_python_constraint() {
    // A whitespace-only "   " is truthy but an unparseable specifier; the store strips it to "".
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root).args(["deps", "a", "--python", "   "]));
    assert_eq!(code, 0);
    // The oracle's `store.resolve("a").meta.requires_python == ""`: an EXACT stored-value check,
    // not a trimmed render (a bug that stored "   " would survive a trim). `insert_string` omits
    // an empty string, so the healthy shape is the key absent; a stored "   " would be a
    // non-empty `requires_python = "   "` and fail this equality.
    let meta_text =
        fs::read_to_string(root.path().join("data/scripts/a/meta.toml")).expect("meta.toml exists");
    let meta: toml::Value = toml::from_str(&meta_text).expect("meta.toml parses");
    let requires_python = meta
        .get("requires_python")
        .and_then(toml::Value::as_str)
        .unwrap_or("");
    assert_eq!(requires_python, "", "{meta_text}");
}

// --------------------------------------------------------------------------
// doctor
// --------------------------------------------------------------------------

#[test]
fn test_doctor_uv_found() {
    let root = sandbox();
    let bin = install_uv(&root);
    let (code, _out) = run(skit(&root).env("PATH", path_with(&bin)).arg("doctor"));
    assert_eq!(code, 0);
}

#[test]
fn test_doctor_uv_missing() {
    let root = sandbox();
    let (code, _out) = run(skit(&root).env("PATH", "").arg("doctor")); // no uv anywhere
    assert_eq!(code, 1);
}

#[test]
fn test_doctor_rebuild() {
    let root = sandbox();
    let bin = install_uv(&root);
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, _out) = run(skit(&root)
        .env("PATH", path_with(&bin))
        .args(["doctor", "--rebuild"]));
    assert_eq!(code, 0);
}

#[test]
fn test_doctor_reports_missing_reference() {
    let root = sandbox();
    let bin = install_uv(&root);
    let src = write_src(&root, "src.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&src)
        .args(["--name", "ref", "--ref", "--no-input"]));
    fs::remove_file(&src).unwrap();
    let (code, out) = run(skit(&root).env("PATH", path_with(&bin)).arg("doctor"));
    assert_eq!(code, 0);
    assert!(out.contains("ref"), "{out}");
}

// --------------------------------------------------------------------------
// lang (oracle: no tests)
// --------------------------------------------------------------------------

// --------------------------------------------------------------------------
// Interactive helpers: called directly + stubbed (CliRunner cannot inject a tty). Every one of
// these is a direct unit test of a CLI-PRIVATE helper with no `pub` surface in `skit_cli`, so an
// integration test cannot call it — the behavior lives in cli.rs, exercised by the crate's own
// #[cfg(test)] module (src/cli/tests.rs), not by this black-box port.
// --------------------------------------------------------------------------

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._parse_selection` (cli.py:431), the onboarding comma-list selection parser ('all'/'none'/'1,3'). Rust selects onboarding candidates with a dialoguer `MultiSelect` widget (cli.rs:2646), not a text parser, so this string helper has no equivalent to call; the selection capability itself is present."]
fn test_parse_selection_variants() {
    // all -> [0,1,2]; none/'' -> []; '1,3' -> [0,2]; dedup + out-of-range/non-numeric ignored.
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._parse_selection` — the isdecimal() vs isdigit() edge (superscripts/circled digits rejected, Arabic-indic accepted). Rust selects candidates with a dialoguer `MultiSelect` widget (cli.rs:2646), not a comma-list text parser, so there is no equivalent string helper to observe."]
fn test_parse_selection_ignores_non_ascii_digit_like_chars() {
    // '1,²,3' -> [0,2]; '①' -> []; Arabic-indic one -> [0].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._parse_kv_opts` (cli.py:4018), the NAME=VALUE option parser that collects malformed entries. Private in cli.rs (the run-side twin is apply_sets in run/command.rs); no `pub` surface."]
fn test_parse_kv_opts() {
    // {A:hello, B:''}; bad == ["--prompt: no-eq", "--prompt: =novalue"].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` (cli.py:171). An existing PEP 723 block is the source of truth: don't ask, don't fill -> ([], \"\"). Private in cli.rs add lane; no `pub` surface."]
fn test_resolve_metadata_existing_block_not_asked() {
    // A script with its own block -> ([], "").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` — explicit --dep/--python used directly. Private in cli.rs add lane."]
fn test_resolve_metadata_explicit_opts() {
    // (["requests","rich"], ">=3.11").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` — explicit values are stripped and empties dropped (([\"requests\"], \"\")). Private in cli.rs add lane."]
fn test_resolve_metadata_explicit_opts_strips_and_drops_empties() {
    // ["", "  requests  ", "   "], "   " -> (["requests"], "").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` — no imports means nothing to ask ((([], \"\"))). Private in cli.rs add lane."]
fn test_resolve_metadata_no_suggestions() {
    // print(1) -> ([], "").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` — non-interactive accepts the AST-suggested deps as-is ([\"requests\"]). Private in cli.rs add lane."]
fn test_resolve_metadata_non_interactive_uses_suggestions() {
    // import requests -> deps == ["requests"].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` with a tty + stubbed Prompt.ask — the interactive deps/python prompts. Private in cli.rs add lane + interactive tty seam."]
fn test_resolve_metadata_interactive() {
    // answers "requests, rich" / ">=3.12" -> (["requests","rich"], ">=3.12").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` — '-' at the deps prompt clears the suggested deps. Private in cli.rs add lane + interactive tty seam."]
fn test_resolve_metadata_interactive_dash_clears_deps() {
    // '-' -> deps == [].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._resolve_python_metadata` — 'None' at the deps prompt clears the suggested deps. Private in cli.rs add lane + interactive tty seam."]
fn test_resolve_metadata_interactive_none_word_clears_deps() {
    // 'None' -> deps == [].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._prompt_identity` (cli.py:374) — non-interactive passes name/description through unchanged ((None, None)). Private in cli.rs add lane."]
fn test_prompt_identity_non_interactive_passes_through() {
    // no_input=True -> (None, None).
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._prompt_identity` with a tty + stubbed Prompt.ask — prompts for name + description. Private in cli.rs add lane + interactive tty seam."]
fn test_prompt_identity_prompts_name_and_description() {
    // ("stitch", "Stack images vertically").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._prompt_identity` — explicit name+description skip all prompts. Private in cli.rs add lane."]
fn test_prompt_identity_explicit_values_skip_prompts() {
    // ("given", "a desc") with Prompt.ask never called.
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._prompt_identity` — an all-whitespace name answer collapses to None so the store derives the stem. Private in cli.rs add lane + interactive tty seam."]
fn test_prompt_identity_blank_name_falls_back_to_stem() {
    // ("   ", "") -> (None, "").
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._onboard_params` (cli.py:538) — a framework-detected script (argparse) yields no const candidates ([]). Private in cli.rs add lane."]
fn test_onboard_params_framework_detected() {
    // argparse present -> [].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._onboard_params` — no candidates -> []. Private in cli.rs add lane."]
fn test_onboard_params_no_candidates() {
    // print(1) -> [].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._onboard_params` — non-interactive returns [] even when candidates exist. Private in cli.rs add lane."]
fn test_onboard_params_non_interactive_returns_empty() {
    // no_input=True -> [].
}

#[test]
#[ignore = "cross-crate: direct unit test of CLI-private `cli._onboard_params` with a tty + stubbed Prompt.ask('all') — selects the detected consts. Private in cli.rs add lane + interactive tty seam."]
fn test_onboard_params_interactive_selection() {
    // 'all' -> specs include CITY.
}

#[test]
fn test_paramspec_from_candidate_roundtrip() {
    // analyzer.analyze -> ParamDecl.from_candidate: the first candidate of `CITY = "Taipei"` is a
    // parameter named CITY. The Rust analyzer returns ParamDecl directly (detect_candidates).
    let candidates = detect_candidates("python", "CITY = \"Taipei\"\nprint(CITY)\n");
    assert_eq!(candidates[0].name, "CITY");
}

#[test]
#[ignore = "cross-crate: the oracle drives `promptform.collect` with a tty + stubbed Prompt.ask to type a command placeholder's value interactively. Interactive collection is cli.rs's inline command form (+ promptform); a non-tty binary cannot type an answer. The prefill/reach-the-shell facts are covered by test_command_placeholders_prefill_from_last / test_run_command_entry_collects_values."]
fn test_command_placeholders_collect_interactively() {
    // typed -> {msg: typed}.
}

#[test]
fn test_command_placeholders_prefill_from_last() {
    // The last stored value prefills a command placeholder (flows.prefill), observed end to end:
    // a seeded value reaches the shell on the next argless run.
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo {msg}", "--name", "e"]));
    seed_state(&root, "e", "[values]\nmsg = \"remembered\"\n");
    let (code, out) = run(skit(&root).args(["run", "e", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("remembered"), "{out}");
}

#[test]
fn test_command_without_placeholders_has_no_fields() {
    // A placeholder-free command has no form fields (flows.plan_for_entry(...).fields == []),
    // observed through `skit show`'s "No form fields" face.
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "e"]));
    let (code, out) = run(skit(&root).args(["show", "e"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("No form fields"), "{out}");
}

#[test]
#[ignore = "cross-crate: the oracle drives `promptform.collect` with a tty + stubbed Prompt.ask for a SECRET inject field. Interactive secret collection is cli.rs's inline param form (+ promptform); a non-tty binary cannot answer the prompt."]
fn test_collect_param_form_interactive_secret() {
    // secretval -> {API: secretval}.
}

#[test]
fn test_param_form_prefill_uses_definition_default() {
    // With no stored value, a const's declared DEFAULT fills the form (flows.prefill), observed end
    // to end: the default is injected and reaches the script body. Substitution: shell inject entry
    // whose source constant equals the declared default, exactly as the oracle's fixture
    // (`metawriter.write_params('CITY = "Osaka"...', [ParamDecl(..., default="Osaka")])`).
    let root = sandbox();
    inject_shell(
        &root,
        "a",
        "A",
        "CITY=Osaka\nprintf '%s\\n' \"$CITY\"\n",
        &[const_str("CITY", Some("Osaka"))],
    );
    let (code, out) = run(skit(&root).args(["run", "a", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Osaka"), "{out}");
}

// --------------------------------------------------------------------------
// Markup escaping: user-controlled data (names, values, prompts, presets, deps, paths, errors)
// must render literally. Rust has no Rich markup layer, so each case reduces to "the datum is
// printed verbatim" — REAL where Rust prints the datum, a divergence where Rust omits it.
// --------------------------------------------------------------------------

#[test]
fn test_add_summary_escapes_markup_in_name_and_description() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "[blue]hi[/blue]"]));
    let (code, out) = run(skit(&root).args([
        "add",
        "--cmd",
        "echo {x}",
        "--name",
        "[red]evil[/red]",
        "--description",
        "[b]d[/b]",
    ]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("[red]evil[/red]"), "{out}");
    assert!(out.contains("[b]d[/b]"), "{out}");
}

#[test]
fn test_add_deps_summary_escapes_markup() {
    // `demo[bold]` is a valid PEP 508 requirement (extras) AND rich markup; the summary shows it.
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    let (code, out) =
        run(skit(&root)
            .arg("add")
            .arg(&path)
            .args(["--dep", "demo[bold]", "--no-input"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("demo[bold]"), "{out}");
}

#[test]
fn test_add_not_py_file_warning_escapes_markup_in_filename() {
    let root = sandbox();
    let path = write_src(&root, "[red]evil[bold].txt", "hi");
    let (code, out) = run(skit(&root).arg("add").arg(&path));
    assert_eq!(code, 2);
    assert!(out.contains("[red]evil[bold].txt"), "{out}");
}

#[test]
fn params_command_matrix_updates_every_declared_axis_and_preserves_machine_shape() {
    let root = sandbox();
    let (code, out) = run(skit(&root).args(["add", "--cmd", "echo {topic}", "--name", "matrix"]));
    assert_eq!(code, 0, "{out}");

    let (code, out) = run(skit(&root).args([
        "params",
        "matrix",
        "--add",
        "topic",
        "--add",
        "extra",
        "--type",
        "extra=int",
        "--default",
        "extra=3",
        "--deliver",
        "extra=env",
        "--env-target",
        "extra=EXTRA",
        "--help-text",
        "extra=Number of runs",
        "--prompt",
        "extra=Count",
        "--required",
        "extra",
        "--add",
        "choice",
        "--type",
        "choice=choice",
        "--choices",
        "choice=a,b",
        "--default",
        "choice=a",
        "--add",
        "secret",
        "--secret",
        "secret",
        "--env-source",
        "secret=TOKEN",
        "--add",
        "items",
        "--multiple",
        "items",
        "--repeat",
        "items",
        "--flag",
        "items=--item",
        "--add",
        "verbose",
        "--type",
        "verbose=bool",
        "--flag",
        "verbose=--verbose",
        "--action",
        "verbose=store_true",
    ]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("Declared parameters"), "{out}");

    let output = skit(&root)
        .args(["params", "matrix", "--json"])
        .output()
        .expect("skit runs");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = payload["parameters"].as_array().unwrap();
    let row = |name: &str| {
        rows.iter()
            .find(|row| row["name"] == name)
            .unwrap_or_else(|| panic!("missing {name}: {payload}"))
    };
    assert_eq!(row("topic")["delivery"], "placeholder");
    assert_eq!(row("extra")["type"], "int");
    assert_eq!(row("extra")["default"], 3);
    assert_eq!(row("extra")["delivery"], "env");
    assert_eq!(row("extra")["env_target"], "EXTRA");
    assert_eq!(row("extra")["required"], true);
    assert_eq!(row("choice")["choices"], serde_json::json!(["a", "b"]));
    assert_eq!(row("secret")["secret"], true);
    assert_eq!(row("secret")["env_source"], "TOKEN");
    assert_eq!(row("items")["multiple"], true);
    assert_eq!(row("items")["repeat"], true);
    assert_eq!(row("items")["flag"], "--item");
    assert_eq!(row("verbose")["action"], "store_true");

    let (code, out) = run(skit(&root).args([
        "params",
        "matrix",
        "--optional",
        "extra",
        "--no-secret",
        "secret",
        "--no-multiple",
        "items",
        "--no-repeat",
        "items",
        "--rm",
        "choice",
    ]));
    assert_eq!(code, 0, "{out}");
    let (code, out) = run(skit(&root).args(["params", "matrix", "--workdir", "invoke"]));
    assert_eq!(code, 0, "{out}");
    let output = skit(&root)
        .args(["params", "matrix", "--json"])
        .output()
        .expect("skit runs");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    let rows = payload["parameters"].as_array().unwrap();
    assert!(rows.iter().all(|row| row["name"] != "choice"));
    assert!(
        rows.iter()
            .find(|row| row["name"] == "extra")
            .unwrap()
            .get("required")
            .is_none()
    );
    let secret = rows.iter().find(|row| row["name"] == "secret").unwrap();
    assert!(secret.get("secret").is_none());
    assert!(secret.get("env_source").is_none());
    let items = rows.iter().find(|row| row["name"] == "items").unwrap();
    assert_eq!(items["multiple"], false);
    assert_eq!(items["repeat"], false);

    let before = snapshot_tree(root.path());
    let (code, out) = run(skit(&root).args(["params", "matrix", "--add", "extra"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("extra is already declared; skipped."), "{out}");
    assert_eq!(snapshot_tree(root.path()), before);
}

#[test]
fn test_remove_escapes_markup_in_name() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "[blue]hi[/blue]"]));
    let (code, out) = run(skit(&root).args(["remove", "[blue]hi[/blue]", "--yes"]));
    assert_eq!(code, 0);
    assert!(out.contains("[blue]hi[/blue]"), "{out}");
}

#[test]
fn test_not_found_error_escapes_markup_in_argument() {
    // The not-found error embeds the raw name the user typed; it renders literally.
    let root = sandbox();
    let (code, out) = run(skit(&root).args(["deps", "[red]ghost[/red]"]));
    assert_eq!(code, 1);
    assert!(out.contains("[red]ghost[/red]"), "{out}");
}

#[test]
fn test_params_table_escapes_markup_in_name_and_default() {
    // A canonical [tool.skit] block can carry markup in a param name/default; the table shows it.
    let root = sandbox();
    let source = write_managed_params(
        "python",
        "print(1)\n",
        &[const_str("[red]NAME[/red]", Some("[blue]hi[/blue]"))],
    )
    .unwrap();
    let path = write_src(&root, "a.py", &source);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--kind", "python", "--no-input"]));

    let data = root.path().join("data");
    let entry = data.join("scripts/a");
    let payload_path = entry.join("script.py");
    let meta_path = entry.join("meta.toml");
    let registry_path = data.join("registry.toml");
    let payload_before = fs::read(&payload_path).unwrap();
    let meta_before = fs::read(&meta_path).unwrap();
    let registry_before = fs::read(&registry_path).unwrap();
    let state_before = snapshot_tree(&root.path().join("state"));

    let output = skit(&root)
        .args(["params", "a", "--json"])
        .output()
        .expect("skit runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["params"][0]["name"], "[red]NAME[/red]");
    assert_eq!(record["params"][0]["kind"], "const");
    assert_eq!(record["params"][0]["default"], "[blue]hi[/blue]");

    let (code, out) = run(skit(&root).args(["params", "a"]));
    assert_eq!(code, 0, "{out}");
    assert_eq!(fs::read(payload_path).unwrap(), payload_before);
    assert_eq!(fs::read(meta_path).unwrap(), meta_before);
    assert_eq!(fs::read(registry_path).unwrap(), registry_before);
    assert_eq!(snapshot_tree(&root.path().join("state")), state_before);
    assert!(out.contains("[red]NAME[/red]"), "{out}");
    assert!(out.contains("[blue]hi[/blue]"), "{out}");
}

#[test]
fn test_params_command_placeholder_line_escapes_markup() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo {msg}", "--name", "e"]));
    seed_state(&root, "e", "[values]\nmsg = \"[green]hello[/green]\"\n");
    let (code, out) = run(skit(&root).args(["params", "e"]));
    assert_eq!(code, 0);
    assert!(out.contains("[green]hello[/green]"), "{out}");
}

#[test]
#[ignore = "cross-crate: the oracle monkeypatches `reconcile.analyze` to inject a hostile candidate name into the \"Detected but not yet managed\" line. The REAL Rust analyzer only ever produces valid-identifier candidate names, so a black-box harness cannot make a non-identifier candidate appear; the injection seam is the analyzer, defense-in-depth the params candidate render. Owning seam: analyzer candidate source (skit-language)."]
fn test_params_candidates_line_escapes_markup_in_name() {
    // A hostile candidate name renders literally in the candidates line.
}

#[test]
fn test_preset_list_escapes_markup_in_name_and_values() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    seed_state(
        &root,
        "a",
        "[presets.\"[blue]prod[/blue]\"]\nCITY = \"[red]Taipei[/red]\"\n",
    );
    let (code, out) = run(skit(&root).args(["preset", "list", "a"]));
    assert_eq!(code, 0);
    assert!(out.contains("[blue]prod[/blue]"), "{out}");
    assert!(out.contains("[red]Taipei[/red]"), "{out}");
}

#[test]
#[ignore = "cross-crate: the oracle pipes input=\"hi\\n\" to answer a command placeholder while `preset save` runs, i.e. interactive value collection on a placeholder-bearing command. The non-tty binary cannot answer the prompt; the seam is cli.rs's inline preset-save form. The name-echo (literal render) is covered by test_preset_delete_unknown_escapes_markup_in_preset_name and test_add_summary_escapes_markup_in_name_and_description."]
fn test_preset_save_command_escapes_markup_in_preset_name_and_entry_name() {
    // Both the entry name and preset name render literally in the save summary.
}

#[test]
fn test_preset_delete_unknown_escapes_markup_in_preset_name() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "a"]));
    let (code, out) = run(skit(&root).args(["preset", "delete", "a", "[red]nope[/red]"]));
    assert_eq!(code, 1);
    assert!(out.contains("[red]nope[/red]"), "{out}");
}

#[test]
fn test_validate_preset_unknown_escapes_markup() {
    let root = sandbox();
    run(skit(&root).args(["add", "--cmd", "echo hi", "--name", "a"]));
    let (code, out) = run(skit(&root).args(["run", "a", "--preset", "[red]nope[/red]"]));
    assert_eq!(code, 2);
    assert!(out.contains("[red]nope[/red]"), "{out}");
}

#[test]
fn test_deps_view_escapes_markup() {
    // `demo[bold]` is a valid requirement AND markup; the view renders the brackets literally.
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code0, _o0) = run(skit(&root).args(["deps", "a", "--dep", "demo[bold]"]));
    assert_eq!(code0, 0);
    let (code, out) = run(skit(&root).args(["deps", "a"]));
    assert_eq!(code, 0);
    assert!(out.contains("demo[bold]"), "{out}"); // brackets survive
}

#[test]
fn test_deps_set_summary_escapes_markup() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--no-input"]));
    let (code, out) = run(skit(&root).args(["deps", "a", "--dep", "demo[bold]"]));
    assert_eq!(code, 0);
    assert!(out.contains("demo[bold]"), "{out}");
}

#[test]
#[ignore = "cross-crate: the oracle monkeypatches `store.doctor_rebuild` to return a fabricated problem line `[red]broken[/red]`. A black-box binary cannot inject a rebuild-problem result into skit's own store; the seam is store.doctor_rebuild + doctor's problem render. Owning seam: skit-store doctor_rebuild."]
fn test_doctor_rebuild_problem_line_escapes_markup() {
    // A rebuild problem line renders literally.
}

#[test]
fn test_doctor_missing_reference_escapes_markup_in_name() {
    let root = sandbox();
    let bin = install_uv(&root);
    let exe = write_src(&root, "tool", "#!/bin/sh\n");
    run(skit(&root)
        .arg("add")
        .arg(&exe)
        .args(["--exe", "--name", "[red]gone[/red]"]));
    fs::remove_file(&exe).unwrap();
    let (_code, out) = run(skit(&root).env("PATH", path_with(&bin)).arg("doctor"));
    assert!(out.contains("[red]gone[/red]"), "{out}");
}

#[test]
fn test_doctor_uv_path_escapes_markup() {
    // The uv PATH is echoed verbatim; a markup-bearing directory renders literally. Black-box twin
    // of the oracle's monkeypatched find_uv -> a real uv installed under a "[red]bin[/red]" dir.
    let root = sandbox();
    let bin = uv_in(&root, "[red]bin[/red]");
    let (code, out) = run(skit(&root).env("PATH", path_with(&bin)).arg("doctor"));
    assert_eq!(code, 0);
    assert!(out.contains("[red]bin[/red]"), "{out}");
}

#[test]
fn test_config_set_unknown_language_escapes_markup() {
    let root = sandbox();
    let (code, out) = run(skit(&root).args(["config", "lang", "[red]xx-YY[/red]"]));
    assert_eq!(code, 2);
    assert!(out.contains("[red]xx-YY[/red]"), "{out}");
}

#[test]
fn test_config_set_unknown_mirror_escapes_markup() {
    let root = sandbox();
    let (code, out) = run(skit(&root).args(["config", "mirror", "[red]nope[/red]"]));
    assert_eq!(code, 2);
    assert!(out.contains("[red]nope[/red]"), "{out}");
}

#[test]
fn test_edit_reports_escape_markup_in_name() {
    let root = sandbox();
    let path = write_src(&root, "a.py", "print(1)\n");
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "[blue]a[/blue]", "--no-input"]));
    let (code, out) = run(skit(&root)
        .env("EDITOR", "true")
        .args(["edit", "[blue]a[/blue]"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("[blue]a[/blue]"), "{out}");
}

#[test]
fn test_edit_reference_mode_escapes_markup_in_name_and_path() {
    let root = sandbox();
    let script = root.path().join("[red]weird[bold]").join("job.py");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "print(1)\n").unwrap();
    run(skit(&root)
        .arg("add")
        .arg(&script)
        .args(["--ref", "--name", "refjob", "--no-input"]));
    let (code, out) = run(skit(&root).env("EDITOR", "true").args(["edit", "refjob"]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("[red]weird[bold]"), "{out}");
}

#[test]
fn test_edit_missing_reference_source_escapes_markup_in_path() {
    let root = sandbox();
    let script = root.path().join("[red]weird[bold]").join("job.py");
    fs::create_dir_all(script.parent().unwrap()).unwrap();
    fs::write(&script, "print(1)\n").unwrap();
    run(skit(&root)
        .arg("add")
        .arg(&script)
        .args(["--ref", "--name", "refjob", "--no-input"]));
    fs::remove_file(&script).unwrap();
    let (code, out) = run(skit(&root).env("EDITOR", "true").args(["edit", "refjob"]));
    assert_eq!(code, 1);
    assert!(out.contains("[red]weird[bold]"), "{out}");
}

#[test]
fn test_edit_params_updated_summary_escapes_markup_in_name() {
    let root = sandbox();
    let block = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"X\"\n# binding = \"const\"\n# type = \"int\"\n# default = 1\n# ///\nX = 1\nprint(X)\n";
    let path = write_src(&root, "a.py", block);
    run(skit(&root).arg("add").arg(&path).args([
        "--name",
        "[blue]a[/blue]",
        "--kind",
        "python",
        "--no-input",
    ]));
    let entry = root.path().join("data").join("scripts").join("blue-a-blue");
    let payload = entry.join("script.py");
    let meta = entry.join("meta.toml");
    let registry = root.path().join("data").join("registry.toml");
    let payload_before = fs::read(&payload).unwrap();
    let meta_before = fs::read(&meta).unwrap();
    let registry_before = fs::read(&registry).unwrap();
    let state_before = snapshot_tree(&root.path().join("state"));
    let (code, out) = run(skit(&root).args(["params", "[blue]a[/blue]", "--resync"]));
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("Updated [blue]a[/blue]. Managed parameters: X"),
        "{out}"
    );
    assert!(!out.contains("Parameter: X"), "{out}");
    assert_eq!(fs::read(payload).unwrap(), payload_before);
    assert_eq!(fs::read(meta).unwrap(), meta_before);
    assert_eq!(fs::read(registry).unwrap(), registry_before);
    assert_eq!(snapshot_tree(&root.path().join("state")), state_before);
}

#[test]
fn test_edit_params_malformed_prompt_escapes_markup() {
    let root = sandbox();
    let block = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# name = \"X\"\n# binding = \"const\"\n# type = \"int\"\n# default = 1\n# ///\nX = 1\nprint(X)\n";
    let path = write_src(&root, "a.py", block);
    run(skit(&root)
        .arg("add")
        .arg(&path)
        .args(["--name", "a", "--kind", "python", "--no-input"]));
    let (code, out) = run(skit(&root).args(["params", "a", "--prompt", "[red]bad[/red]"]));
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("Ignored a malformed value: --prompt: [red]bad[/red] (expected NAME=text)."),
        "{out}"
    );
}

#[test]
fn test_run_reusing_last_arguments_escapes_markup() {
    // The oracle stubs run_entry, so ONLY skit's own reused-arguments notice can carry the markup,
    // and that notice is a STDERR line (err_console, cli.py:3153-3156). Use a body that does NOT
    // echo its args (`printf 'ran'`) AND read stderr on its own — so neither the script's stdout
    // nor skit's stdout command-preview line can satisfy it; only skit's own notice can.
    let root = sandbox();
    shell_entry(&root, "j", "J", "printf 'ran\\n'\n", "");
    seed_state(&root, "j", "extra_args = [\"[red]arg[/red]\"]\n");
    let output = skit(&root)
        .args(["run", "j", "--no-input"])
        .output()
        .expect("skit runs");
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[red]arg[/red]"), "stderr: {stderr}");
}

#[test]
#[ignore = "cross-crate: the oracle stubs Prompt.ask and asserts the PROMPT TEXT passed to it is Rich-escaped (`  \\[red]msg\\[/red]`). That is an interactive-form assertion on the prompt string cli.rs builds for promptform.collect; a non-tty binary never issues the prompt, and Rust has no Rich escaping to observe. Owning seam: cli.rs inline command form prompt text."]
fn test_collect_command_values_prompt_escapes_markup_in_placeholder_name() {
    // The placeholder name is escaped in the prompt text.
}

#[test]
#[ignore = "cross-crate: the oracle stubs Prompt.ask and asserts the param-form PROMPT TEXT is Rich-escaped (`  \\[red]Where\\[/red]?`). Interactive-form assertion on the prompt string; a non-tty binary never issues it and Rust has no Rich layer. Owning seam: cli.rs inline param form prompt text."]
fn test_collect_param_form_prompt_escapes_markup_in_param_prompt_text() {
    // The param's prompt text is escaped.
}

#[test]
#[ignore = "cross-crate: the oracle calls `cli.preset_save(...)` with a stubbed Prompt.ask and asserts the escaped PROMPT TEXT (`  \\[red]msg\\[/red]`). Interactive preset-save form assertion; a non-tty binary never issues the prompt and Rust has no Rich layer. Owning seam: cli.rs inline preset-save form prompt text."]
fn test_preset_save_prompt_escapes_markup_in_placeholder_name() {
    // The placeholder name is escaped in the preset-save prompt text.
}

#[test]
fn test_run_raw_passes_argv_genuinely_raw() {
    // --raw is the escape hatch: no token pass, no glob pass — even weird argv survives verbatim.
    let root = sandbox();
    fs::write(root.path().join("match.txt"), "").unwrap();
    shell_entry(&root, "rawr", "Rawr", "printf '%s\\n' \"$@\"\n", "");
    let (code, out) = run(skit(&root).current_dir(root.path()).args([
        "run",
        "rawr",
        "--raw",
        "--no-input",
        "--",
        "{env:UNSET}",
        "*.txt",
    ]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("{env:UNSET}"), "{out}");
    assert!(out.contains("*.txt"), "{out}");
}

#[test]
fn test_run_cli_argv_not_reexpanded() {
    // `-- '*.txt'` already survived the user's shell; skit must not glob it a second time
    // (assemble is called with expand_extra=False). The literal `*.txt` survives even with a
    // matching file in the cwd.
    let root = sandbox();
    fs::write(root.path().join("match.txt"), "").unwrap();
    shell_entry(&root, "noglob", "Noglob", "printf '%s\\n' \"$@\"\n", "");
    let (code, out) = run(skit(&root).current_dir(root.path()).args([
        "run",
        "noglob",
        "--no-input",
        "--",
        "*.txt",
    ]));
    assert_eq!(code, 0, "{out}");
    assert!(out.contains("*.txt"), "{out}"); // literal, not re-expanded to match.txt
}
