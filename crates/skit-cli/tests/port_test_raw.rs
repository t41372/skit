//! Mechanical port of the Python oracle module `tests/test_raw.py`
//! (`origin/main@206f9ef`): "`skit run --raw` escape hatch: skip the parameter form and
//! injection, run the script as-is." Each `#[test]` keeps its Python `def test_*` name and
//! its WHY comment so it traces back to its origin.
//!
//! WHY `skit-cli`: the oracle drives the whole `run` pipeline through Typer's `CliRunner`
//! and inspects the launcher call. Only the composition-root crate can run the real `skit`
//! binary end to end (resolve -> reconcile -> prefill -> assemble -> stage -> launch).
//!
//! KIND SUBSTITUTION (python oracle fixture -> shell here). The oracle builds a PYTHON entry
//! (`store.add_python`) with one managed const `CITY`. A python entry can only launch through
//! `uv run --script`, which needs a uv-downloaded interpreter and network — the whole Rust
//! suite avoids it (every python `run_cli` test uses `--dry-run`). The behavior under test is
//! kind-agnostic in BOTH impls: the oracle's `flows.execute` documents "the caller function
//! knows nothing about any language", and the Rust `stage_injected_source`
//! (`crates/skit-cli/src/run/command.rs`) is generic. This port therefore uses a shell entry as
//! the uv-free vehicle and builds its managed source with `write_managed_params("shell", ...)`
//! exactly as the oracle builds its python source with `metawriter.write_params`.
//!
//! The one shell-vs-python worry — that the shell reconcile would read the body's `CITY=Taipei`
//! assignment as a const default and inject it even with no saved value — was disproven directly:
//! a bare managed const (no recorded default) stays `default = None` through `form_params` and
//! `prefill` (`refresh_default` only refreshes a default that is already `Some`), so `assemble`
//! yields an EMPTY `inject_values` for the no-value case, identical to the python const. The
//! injection DECISION is faithful; only the previous observable was wrong (see below).
//!
//! The binary prints the injection receipt and the child's output. For these managed-constant
//! fixtures, the receipt and the injected value show that an injected source ran. With no value,
//! or with `--raw`, the stored source runs at its stable path. `run_identity_races.rs` checks that
//! path with real shell, Python, and JavaScript processes.
//!
//! Each subprocess uses the test's state directory as its system temporary directory. Cleanup
//! checks both that directory and the entry directory for `.injected-*` files. A normal cleanup
//! case supplies a saved value, so it creates and removes an actual injected source.
//!
//! Buckets: all five are REAL asserting `#[test]`s (API EXISTS). None is cross-crate, absent, or
//! divergent — the injection decision reproduces the oracle exactly for every fixture.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use predicates::prelude::*;
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::write_managed_params;
use tempfile::TempDir;

/// The distinctive transparency prefix skit prints iff an injected temp copy is made — the
/// black-box witness for the oracle's `script_override is not None`.
const INJECT_MARKER: &str = "→ inject:";

/// The oracle's module-level fixture parameter:
/// `ParamDecl(name="CITY", binding="const", type="str")` — a managed const with NO default.
fn city_const() -> ParamDecl {
    let mut declaration = ParamDecl::new("CITY");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

/// Give the hand-written entry directory its registry membership (see `run_cli.rs`).
fn register(data: &TempDir, slug: &str) {
    fs::write(
        data.path().join("registry.toml"),
        format!("[entries.{slug}]\n"),
    )
    .unwrap();
}

/// The oracle's `entry_with_params`, ported: a `copy` entry whose managed source declares the
/// const `CITY` (assigned `Taipei` in the body) and whose body prints the value and `$0`.
///
/// Returns the data root, the state root, and the entry directory (for the artifact checks).
fn entry_with_params() -> (TempDir, TempDir, PathBuf) {
    let data = TempDir::new().unwrap();
    let state = TempDir::new().unwrap();
    let dir = data.path().join("scripts/demo");
    fs::create_dir_all(&dir).unwrap();
    // `CITY=Taipei` is the assignment the shell injector rewrites; the two-arg printf reuses
    // its format once per argument, so it prints the CITY value on one line and `$0` on the next.
    let body = "CITY=Taipei\nprintf '%s\\n' \"$CITY\" \"$0\"\n";
    let source = write_managed_params("shell", body, &[city_const()]).unwrap();
    fs::write(dir.join("script.sh"), source).unwrap();
    fs::write(
        dir.join("meta.toml"),
        r#"name = "Demo"
kind = "shell"
mode = "copy"
source = "/deleted/demo.sh"
workdir = "invoke"
"#,
    )
    .unwrap();
    register(&data, "demo");
    (data, state, dir)
}

/// The oracle's `argstate.save_last(slug, values={"CITY": value})`.
fn save_last_city(state: &TempDir, value: &str) {
    fs::create_dir_all(state.path().join("values")).unwrap();
    fs::write(
        state.path().join("values/demo.toml"),
        format!("[values]\nCITY = {value:?}\n"),
    )
    .unwrap();
}

/// The oracle's `_run`: invoke the real `skit run` binary with `SKIT_*` roots pinned.
fn skit(data: &TempDir, state: &TempDir) -> assert_cmd::Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", data.path())
        .env("SKIT_STATE_DIR", state.path())
        .env("SKIT_CONFIG_DIR", state.path())
        .env("SKIT_LANG", "en")
        .env("TMPDIR", state.path());
    command
}

/// The oracle's `entry.dir.glob(".injected*")` — a staged copy left in the entry directory.
fn has_staged_artifact(dir: &Path) -> bool {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .any(|item| item.file_name().to_string_lossy().starts_with(".injected-"))
}

#[test]
fn test_raw_skips_form_and_injection() {
    // --raw skips the form and injection: no temp copy is injected (script_override is None), so
    // the stored copy runs as-is — no `→ inject:` transparency line and the source constant prints.
    let (data, state, _dir) = entry_with_params();
    skit(&data, &state)
        .args(["run", "demo", "--raw", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Taipei"))
        .stdout(predicate::str::contains(INJECT_MARKER).not());
}

#[test]
fn test_default_run_injects() {
    // A managed value exists (remembered from a "previous run"), so the default path injects it
    // into a temp copy (script_override is not None): the `→ inject:` line appears and the injected
    // value reaches the script. With no value at all there is nothing to inject and the stored copy
    // runs directly — that case is test_no_values_runs_copy_directly.
    let (data, state, _dir) = entry_with_params();
    save_last_city(&state, "Kaohsiung");
    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains(INJECT_MARKER))
        .stdout(predicate::str::contains("Kaohsiung"));
}

#[test]
fn test_no_values_runs_copy_directly() {
    // No default, no last value: nothing to inject; the copy runs as written (script_override is
    // None) — no `→ inject:` line and the stored constant prints.
    let (data, state, _dir) = entry_with_params();
    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Taipei"))
        .stdout(predicate::str::contains(INJECT_MARKER).not());
}

#[test]
fn test_raw_does_not_leave_injected_artifact() {
    // --raw stages no injected copy, so nothing is left in the entry directory. (The oracle's own
    // setup injects nothing either — the run injects only when a value exists — so this is a
    // faithful clean-directory check.)
    let (data, state, dir) = entry_with_params();
    skit(&data, &state)
        .args(["run", "demo", "--raw", "--no-input"])
        .assert()
        .success();
    assert!(!has_staged_artifact(&dir));
    assert!(!has_staged_artifact(state.path()));
}

#[test]
fn test_normal_run_cleans_injected_artifact() {
    // A saved value requires an injected source. Both temporary locations are clean afterwards.
    let (data, state, dir) = entry_with_params();
    save_last_city(&state, "Kaohsiung");
    skit(&data, &state)
        .args(["run", "demo", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains(INJECT_MARKER));
    assert!(!has_staged_artifact(&dir));
    assert!(!has_staged_artifact(state.path()));
}
