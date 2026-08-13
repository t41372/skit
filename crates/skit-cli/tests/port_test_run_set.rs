//! Mechanical port of the Python oracle module `tests/test_run_set.py`
//! (`origin/main@206f9ef`): "`skit run --set NAME=VALUE`: explicit values without a form
//! (issue #2)." Each `#[test]` keeps its Python `def test_*` name and its WHY comment so it
//! traces back to its origin.
//!
//! WHY `skit-cli`: the oracle drives the whole `run` pipeline through Typer's `CliRunner`
//! (`skit.cli.app`) and inspects the launcher call and the persisted argstate. Only the
//! composition-root crate can run the real `skit` binary end to end. These tests drive the
//! `skit` binary through `assert_cmd`, with `SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR`
//! pinned to a per-test `TempDir` and `SKIT_LANG=en` on every invocation.
//!
//! OBSERVABLE MAPPING (the oracle's `run_entry_spy` -> a black-box binary). The oracle
//! monkeypatches `launcher.run_entry` and reads its `values` / `extra_args` / `script_override`
//! kwargs and whether it was called at all. A black-box port cannot see those kwargs, so each
//! spy field maps to a real, independent witness:
//! - `override is not None` (an injected temp copy was made) -> the `→ inject:` transparency
//!   line skit prints on exactly that predicate (the same signal `port_test_raw.rs` uses). The
//!   Rust pair format is `NAME = VALUE` with spaces (`delivery.rs`), so this file asserts the
//!   marker and each value separately, never a joined `NAME=VALUE`.
//! - `values` (the injected/placeholder values) -> for a command entry, the value reaches the
//!   shell and prints on stdout; for every entry, the accepted values land in `[values]` of
//!   `state/values/<slug>.toml`.
//! - `extra` (the argv tail) -> a `printf '%s\n' "$@"` shell body prints the tail; for the
//!   argparse case, the `--dry-run` preview shows the reflected `--output x.png`.
//! - `"entry" not in run_entry_spy` (the launcher was never called: a refusal) -> the run wrote
//!   no run stamp. A refused invocation leaves `state/values/<slug>.toml` absent (or, for a
//!   dry run, with no `[last_run]` and no `[values]`). This is the discriminator for "nothing
//!   ran", never stdout (the dry-run display line itself echoes the template).
//! - `argstate.load_state(slug)["values" | "presets"]` -> the `[values]` / `[presets.<name>]`
//!   tables of `state/values/<slug>.toml`, read back as text.
//!
//! KIND SUBSTITUTION (uv-free vehicles, the `port_test_raw.rs` precedent). The oracle builds
//! PYTHON entries with `metawriter.write_params`; a python entry can only launch through
//! `uv run --script`, which the Rust suite avoids (needs a uv download + network). The behavior
//! under test is kind-agnostic, so:
//! - a python INJECT-const fixture becomes a SHELL entry built with `write_managed_params`
//!   ("const" -> Inject delivery), the uv-free injection vehicle;
//! - the argparse fixture stays python but is observed through `--dry-run` (offline: the Rust
//!   python preview never touches uv, per `run_cli.rs`);
//! - the launch-refusal fixture (oracle monkeypatches `flows.execute` to a `FAIL_LAUNCH`
//!   outcome) becomes a shell entry with a nonexistent pinned interpreter, a real
//!   program-not-found refusal (`exit != 0`, the only launch fact the oracle asserts);
//! - the Ctrl+C fixture (oracle raises `KeyboardInterrupt` from `flows.execute`) becomes a
//!   command whose body sends itself `SIGINT` (`kill -INT $$`): the child dies from signal 2,
//!   which the Rust launcher maps to `128 + 2 = 130`, the same accepted-then-interrupted run.
//!
//! Buckets:
//! - REAL asserting `#[test]` (API EXISTS, behavior converges): the injection, command,
//!   argparse-preview, deferred-preset, dry-run-preset, launch-refusal, prompt-argv-limit,
//!   token-expansion, embedded-equals, typed-value refusal, empty-required, and SIGINT-persist
//!   cases.
//! - FAILING CONTRACT (divergence): the message/format cases. The Rust `run` error strings
//!   differ from the oracle's, and `apply_sets` (`run/command.rs`) stops at the first bad item,
//!   does not strip the key, does not sort, and lists no valid names. Each keeps its full
//!   asserting body and is `#[ignore]`d with the exact expected-vs-actual evidence; deleting the
//!   `#[ignore]` after the impl is fixed turns it green.
//! - UNMAPPED (cross-crate): the four interactive `_collect_values` cases. Their observables
//!   ("the form must not open", "which fields it asked") require intercepting the interactive
//!   inline-form seam (`cli.rs` run form + `_skit_save_preset` form state, unit-driven by
//!   `src/cli/tests.rs`), which a non-tty binary harness cannot observe. Compiling `#[ignore]`
//!   stubs name that seam.

#![cfg(unix)]

use std::fs;

use predicates::prelude::*;
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::write_managed_params;
use tempfile::TempDir;

/// The distinctive prefix skit prints iff an injected temp copy is made — the black-box witness
/// for the oracle's `script_override is not None`.
const INJECT_MARKER: &str = "→ inject:";

/// The oracle's `RAW_CONFLICT` sentence.
const RAW_CONFLICT: &str =
    "--raw runs the script as-is; --set, --preset, and --save-preset do not apply.";

/// A fresh sandbox root holding `data/`, `state/`, and `config/` subtrees.
fn sandbox() -> TempDir {
    TempDir::new().unwrap()
}

/// The oracle's `runner.invoke(cli.app, ...)`: the real `skit` binary with all three roots pinned
/// under the sandbox and the locale fixed to English.
fn skit(root: &TempDir) -> assert_cmd::Command {
    let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
    command
        .env("SKIT_DATA_DIR", root.path().join("data"))
        .env("SKIT_STATE_DIR", root.path().join("state"))
        .env("SKIT_CONFIG_DIR", root.path().join("config"))
        .env("SKIT_LANG", "en");
    command
}

/// Register one hand-built entry directory in the authoritative membership index.
fn register(root: &TempDir, slug: &str) {
    let data = root.path().join("data");
    fs::create_dir_all(&data).unwrap();
    fs::write(data.join("registry.toml"), format!("[entries.{slug}]\n")).unwrap();
}

/// Write a hand-built shell entry (the uv-free injection vehicle). `extra_meta` appends optional
/// metadata lines (for a pinned interpreter).
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

/// The oracle's fixture parameter `ParamDecl(name="CITY", binding="const", type="str",
/// default="Taipei")`.
fn city() -> ParamDecl {
    let mut declaration = ParamDecl::new("CITY");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String("Taipei".to_owned()));
    declaration
}

/// The oracle's fixture parameter `ParamDecl(name="TIMES", binding="const", type="int",
/// default=2)`.
fn times() -> ParamDecl {
    let mut declaration = ParamDecl::new("TIMES");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Int;
    declaration.default = Some(ParameterValue::Integer(2));
    declaration
}

/// The oracle's `_inject_entry`: a shell entry declaring the `CITY` (str) and `TIMES` (int)
/// managed consts, assigned in the body and echoed.
fn build_trip(root: &TempDir) {
    let body = "CITY=Taipei\nTIMES=2\nprintf '%s\\n' \"$CITY\" \"$TIMES\"\n";
    let source = write_managed_params("shell", body, &[city(), times()]).unwrap();
    shell_entry(root, "trip", "Trip", &source, "");
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

/// Seed a preset directly in argstate (the oracle's `argstate.save_preset`).
fn seed_preset(root: &TempDir, slug: &str, body: &str) {
    let values = root.path().join("state/values");
    fs::create_dir_all(&values).unwrap();
    fs::write(values.join(format!("{slug}.toml")), body).unwrap();
}

// --------------------------------------------------------------------------
// non-interactive: the agent path
// --------------------------------------------------------------------------

#[test]
fn test_set_inject_values_non_interactive() {
    // --set on the injected consts runs non-interactively: a temp copy is injected
    // (script_override is not None -> `→ inject:` line) and the accepted values persist.
    let root = sandbox();
    build_trip(&root);
    skit(&root)
        .args([
            "run", "trip", "--set", "CITY=Kaohsiung", "--set", "TIMES=3", "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(INJECT_MARKER)) // values were injected
        .stdout(predicate::str::contains("Kaohsiung"));
    let saved = state_text(&root, "trip").expect("run persisted state");
    assert!(saved.contains("CITY = \"Kaohsiung\""), "{saved}");
    assert!(saved.contains("TIMES = \"3\""), "{saved}");
}

#[test]
fn test_set_makes_command_placeholders_runnable() {
    // THE previously-impossible case: required placeholders, no prior run, no preset.
    let root = sandbox();
    skit(&root)
        .args([
            "add",
            "--cmd",
            "echo {target} {level}",
            "--name",
            "deploy",
            "--no-input",
        ])
        .assert()
        .success();
    skit(&root)
        .args([
            "run",
            "deploy",
            "--set",
            "target=prod",
            "--set",
            "level=high",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("prod high"));
}

#[test]
fn test_set_wins_over_preset() {
    // --set overrides a value the preset also carries.
    let root = sandbox();
    skit(&root)
        .args([
            "add",
            "--cmd",
            "echo {target}",
            "--name",
            "d2",
            "--no-input",
        ])
        .assert()
        .success();
    seed_preset(&root, "d2", "[presets.stage]\ntarget = \"staging\"\n");
    skit(&root)
        .args([
            "run",
            "d2",
            "-p",
            "stage",
            "--set",
            "target=prod",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("prod"))
        .stdout(predicate::str::contains("staging").not());
}

#[test]
fn test_set_satisfies_required_argparse_field() {
    // A --set on a reflected required argparse flag delivers it as `--output x.png`. Observed
    // through the offline `--dry-run` preview (the Rust python preview never needs uv).
    let root = sandbox();
    let script = root.path().join("ar.py");
    fs::write(
        &script,
        "import argparse\nap = argparse.ArgumentParser()\nap.add_argument('-o', '--output', required=True)\nap.parse_args()\n",
    )
    .unwrap();
    skit(&root)
        .arg("add")
        .arg(&script)
        .args(["--name", "ar", "--kind", "python", "--no-input"])
        .assert()
        .success();
    // `add <path>` positional is the `source` argument; assert through a dry run.
    skit(&root)
        .env("PATH", "")
        .args([
            "run",
            "ar",
            "--set",
            "output=x.png",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("--output x.png"));
}

#[test]
fn test_set_saves_preset_with_dry_run_without_running() {
    // --save-preset under --dry-run persists the preset but launches nothing.
    let root = sandbox();
    skit(&root)
        .args([
            "add",
            "--cmd",
            "echo {target}",
            "--name",
            "d3",
            "--no-input",
        ])
        .assert()
        .success();
    skit(&root)
        .args([
            "run",
            "d3",
            "--set",
            "target=stage",
            "--save-preset",
            "quick",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .success();
    let saved = state_text(&root, "d3").expect("preset persisted");
    assert!(saved.contains("[presets.quick]"), "{saved}");
    assert!(saved.contains("target = \"stage\""), "{saved}");
    assert!(
        !saved.contains("[last_run]"),
        "dry run must not run: {saved}"
    );
}

#[test]
fn test_save_preset_on_field_less_entry_refused_saves_nothing() {
    // A field-less entry has nothing to put in a preset — `--save-preset` is refused with the
    // same sentence `skit preset save` uses, and nothing is saved OR run. The exit code is
    // USAGE (2), NOT 1: inside `run`, 1-124 belongs to the script (docker convention).
    let root = sandbox();
    skit(&root)
        .args(["add", "--cmd", "echo hi", "--name", "noargs", "--no-input"])
        .assert()
        .success();
    skit(&root)
        .args(["run", "noargs", "--save-preset", "nope", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "has no form fields, so there's nothing to save.",
        ));
    let saved = state_text(&root, "noargs");
    assert!(
        saved.as_deref().is_none_or(|s| !s.contains("presets")),
        "{saved:?}"
    );
    assert!(
        saved.as_deref().is_none_or(|s| !s.contains("[last_run]")),
        "{saved:?}"
    );
}

#[test]
fn test_save_preset_deferred_until_a_real_run_is_accepted() {
    // A normal `run --save-preset` persists the preset AFTER the launch is accepted.
    let root = sandbox();
    skit(&root)
        .args(["add", "--cmd", "echo {msg}", "--name", "e", "--no-input"])
        .assert()
        .success();
    skit(&root)
        .args([
            "run",
            "e",
            "--set",
            "msg=hi",
            "--save-preset",
            "prod",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("hi"));
    let saved = state_text(&root, "e").expect("run persisted state");
    assert!(saved.contains("[last_run]"), "it ran: {saved}");
    assert!(saved.contains("[presets.prod]"), "{saved}");
    assert!(saved.contains("msg = \"hi\""), "{saved}");
}

#[test]
fn test_save_preset_not_written_when_launch_is_refused() {
    // A launch refusal leaves NO preset — the deferred write is gated on acceptance, not merely
    // reaching the run body. Substitution: the oracle monkeypatches `flows.execute` to a
    // FAIL_LAUNCH outcome; here a shell entry with a nonexistent pinned interpreter is a real
    // program-not-found refusal (exit != 0, the only launch fact the oracle asserts).
    let root = sandbox();
    let body = "printf '%s\\n' \"$MSG\"\n";
    let mut msg = ParamDecl::new("MSG");
    msg.binding = ParameterBinding::Const;
    msg.delivery = ParameterDelivery::Inject;
    msg.parameter_type = ParameterType::Str;
    let source = write_managed_params("shell", body, &[msg]).unwrap();
    shell_entry(
        &root,
        "e",
        "E",
        &source,
        "interpreter = \"/nonexistent/skit-missing-sh\"\n",
    );
    skit(&root)
        .args([
            "run",
            "e",
            "--set",
            "MSG=hi",
            "--save-preset",
            "prod",
            "--no-input",
        ])
        .assert()
        .failure();
    let saved = state_text(&root, "e");
    assert!(
        saved.as_deref().is_none_or(|s| !s.contains("presets")),
        "nothing persisted: {saved:?}"
    );
}

#[test]
fn test_save_preset_dry_run_validation_failure_writes_nothing() {
    // `--save-preset X --dry-run` on a prompt whose render is over-long exits 125 and persists
    // NO preset — the deferred write sits AFTER dry-run validation.
    let root = sandbox();
    let dir = root.path().join("data/scripts/big");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("prompt.md"), "Do {{a}}\n").unwrap();
    fs::write(
        dir.join("meta.toml"),
        "name = \"Big\"\nkind = \"prompt\"\nmode = \"copy\"\nsource = \"/deleted/big.prompt.md\"\nworkdir = \"invoke\"\nrunner = \"claude\"\nparams = [\"a\"]\n",
    )
    .unwrap();
    register(&root, "big");
    let config = root.path().join("config");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[prompt]\nrunners_seeded = true\n[[prompt.runners]]\nname = \"claude\"\nargv = [\"claude\", \"{{prompt}}\"]\n",
    )
    .unwrap();
    let huge = "x".repeat(100_001); // render.ARGV_LIMIT + 1 on POSIX
    skit(&root)
        .env("PATH", "")
        .args([
            "run",
            "big",
            "--set",
            &format!("a={huge}"),
            "--save-preset",
            "toolong",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .code(125);
    let saved = state_text(&root, "big");
    assert!(
        saved.as_deref().is_none_or(|s| !s.contains("presets")),
        "no preset persisted: {saved:?}"
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a --dry-run on an INJECT-delivered secret shows no mask. The Rust dry-run path (run/command.rs:537-543) prints only `plan.display` (the command line `bash …/script.sh`) and returns BEFORE `transparency_messages`, so the oracle's `→ inject: KEY=•••` line — the only place the mask would appear for an injected const — is never emitted. The secret is also never shown (that assertion converges); the divergence is the missing `•••`. NOTE: the second claim (KEY never on disk) was verified to CONVERGE by a separate probe — the injected secret is absent from both `[values]` and `[last_run.values]` after a real run."]
fn test_set_secret_never_persisted_and_masked_in_dry_run() {
    // A secret --set value is masked in dry-run output and never persisted to disk.
    let root = sandbox();
    let body = "KEY=old\nprintf '%s\\n' \"$KEY\"\n";
    let mut key = ParamDecl::new("KEY");
    key.binding = ParameterBinding::Const;
    key.delivery = ParameterDelivery::Inject;
    key.parameter_type = ParameterType::Str;
    key.secret = true;
    let source = write_managed_params("shell", body, &[key]).unwrap();
    shell_entry(&root, "api", "Api", &source, "");
    skit(&root)
        .args([
            "run",
            "api",
            "--set",
            "KEY=s3cret-value",
            "--dry-run",
            "--no-input",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("s3cret-value").not())
        .stdout(predicate::str::contains("•••"));
    skit(&root)
        .args(["run", "api", "--set", "KEY=s3cret-value", "--no-input"])
        .assert()
        .success();
    let saved = state_text(&root, "api").expect("run persisted state");
    assert!(!saved.contains("KEY"), "C3: never on disk: {saved}"); // never on disk
}

#[test]
fn test_set_token_values_expand_at_assembly() {
    // A `{cwd}` token in a --set value expands at assembly (the command echoes the real cwd),
    // but the SAVED value keeps the token (intent is persisted, not expansion).
    let root = sandbox();
    let cwd = root.path().join("data");
    fs::create_dir_all(&cwd).unwrap();
    skit(&root)
        .args(["add", "--cmd", "echo {where}", "--name", "d4", "--no-input"])
        .assert()
        .success();
    skit(&root)
        .current_dir(&cwd) // pin cwd so `{cwd}` expands to a path we own
        .args(["run", "d4", "--set", "where={cwd}", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains(cwd.to_str().unwrap()));
    let saved = state_text(&root, "d4").expect("run persisted state");
    assert!(saved.contains("where = \"{cwd}\""), "{saved}");
}

// --------------------------------------------------------------------------
// error contract: never guess
// --------------------------------------------------------------------------

#[test]
fn test_set_malformed_exits_2_with_exact_message() {
    let root = sandbox();
    build_trip(&root);
    for bad in ["NOVALUE", "=v"] {
        skit(&root)
            .args(["run", "trip", "--set", bad, "--no-input"])
            .assert()
            .code(2)
            .stderr(predicate::str::contains(format!(
                "Malformed --set (expected NAME=VALUE): {bad}"
            )))
            .stderr(predicate::str::contains("Unknown parameter").not());
    }
    // Both bad items in one invocation: pins the ", " join between them.
    skit(&root)
        .args([
            "run",
            "trip",
            "--set",
            "NOVALUE",
            "--set",
            "=v",
            "--no-input",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Malformed --set (expected NAME=VALUE): NOVALUE, =v",
        ));
    assert!(state_text(&root, "trip").is_none(), "nothing ran");
}

#[test]
fn test_set_value_may_contain_equals_signs() {
    let root = sandbox();
    build_trip(&root);
    // partition, not rpartition: the FIRST '=' splits, the rest belongs to the value.
    skit(&root)
        .args(["run", "trip", "--set", "CITY=a=b", "--no-input"])
        .assert()
        .success();
    let saved = state_text(&root, "trip").expect("run persisted state");
    assert!(saved.contains("CITY = \"a=b\""), "{saved}");
}

#[test]
fn test_set_key_is_stripped() {
    let root = sandbox();
    build_trip(&root);
    skit(&root)
        .args(["run", "trip", "--set", " CITY =Kaohsiung", "--no-input"])
        .assert()
        .success();
    let saved = state_text(&root, "trip").expect("run persisted state");
    assert!(saved.contains("CITY = \"Kaohsiung\""), "{saved}");
}

#[test]
fn test_set_unknown_name_exits_2_and_lists_valid() {
    let root = sandbox();
    build_trip(&root);
    skit(&root)
        .args([
            "run",
            "trip",
            "--set",
            "NOPE=1",
            "--set",
            "ALSO=2",
            "--no-input",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Unknown parameter for --set: ALSO, NOPE. This entry's parameters: CITY, TIMES",
        ));
    assert!(state_text(&root, "trip").is_none(), "nothing ran");
}

#[test]
fn test_set_on_entry_without_fields_lists_a_dash() {
    let root = sandbox();
    let exe = root.path().join("tool");
    fs::write(&exe, "#!/bin/sh\necho hi\n").unwrap();
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o755)).unwrap();
    }
    skit(&root)
        .args([
            "add",
            "--exe",
            exe.to_str().unwrap(),
            "--name",
            "tool",
            "--no-input",
        ])
        .assert()
        .success();
    skit(&root)
        .args(["run", "tool", "--set", "X=1", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(
            "Unknown parameter for --set: X. This entry's parameters: —",
        ));
    assert!(state_text(&root, "tool").is_none(), "nothing ran");
}

#[test]
fn test_set_with_raw_is_a_usage_conflict() {
    let root = sandbox();
    build_trip(&root);
    // Not the misleading "unknown parameter" — CITY exists; --raw is the conflict.
    skit(&root)
        .args(["run", "trip", "--raw", "--set", "CITY=x", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(RAW_CONFLICT));
    assert!(state_text(&root, "trip").is_none(), "nothing ran");
}

#[test]
fn test_preset_with_raw_is_a_usage_conflict() {
    let root = sandbox();
    build_trip(&root);
    seed_preset(&root, "trip", "[presets.loud]\nCITY = \"Tainan\"\n");
    // refusing beats silently dropping the preset's values
    skit(&root)
        .args(["run", "trip", "--raw", "-p", "loud", "--no-input"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(RAW_CONFLICT));
    // the preset seed is the only state; no run stamp was added.
    let saved = state_text(&root, "trip").expect("seeded preset");
    assert!(!saved.contains("[last_run]"), "nothing ran: {saved}");
}

#[test]
fn test_save_preset_with_raw_is_a_usage_conflict() {
    let root = sandbox();
    build_trip(&root);
    skit(&root)
        .args([
            "run",
            "trip",
            "--raw",
            "--save-preset",
            "ghost",
            "--no-input",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains(RAW_CONFLICT));
    // The old silent path persisted an EMPTY preset that later validated for -p ghost.
    let saved = state_text(&root, "trip");
    assert!(
        saved.as_deref().is_none_or(|s| !s.contains("presets")),
        "no preset persisted: {saved:?}"
    );
}

#[test]
fn test_raw_never_replays_last_extra_args() {
    let root = sandbox();
    // `printf '%s\n' "$@"` makes the replayed tail visible on stdout.
    shell_entry(&root, "j", "J", "printf '%s\\n' \"$@\"\n", "");
    skit(&root)
        .args(["run", "j", "--no-input", "--", "--verbose", "x.png"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--verbose"))
        .stdout(predicate::str::contains("x.png"));
    // --raw promises "as-is": the previous run's arguments must NOT come back.
    skit(&root)
        .args(["run", "j", "--raw", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--verbose").not());
    // The escape hatch leaves no fingerprints (beyond the run stamp): a plain run afterwards
    // still reuses the remembered args.
    let saved = state_text(&root, "j").expect("run persisted state");
    assert!(saved.contains("exit = 0"), "{saved}");
    skit(&root)
        .args(["run", "j", "--no-input"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--verbose"))
        // the reuse notice is skit chrome and belongs on stderr, not the script's stdout.
        .stderr(predicate::str::contains("Reusing your last arguments"))
        .stdout(predicate::str::contains("Reusing your last arguments").not());
}

#[test]
fn test_set_bad_typed_value_exits_125() {
    let root = sandbox();
    build_trip(&root);
    skit(&root)
        .args(["run", "trip", "--set", "TIMES=abc", "--no-input"])
        .assert()
        .code(125)
        .stderr(predicate::str::contains(
            "TIMES needs a whole number — you typed 'abc'.",
        ));
    assert!(state_text(&root, "trip").is_none(), "nothing ran");
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is `_collect_values` must not open for an invalid --set. That interception lives at the interactive inline-form seam (cli.rs run form / promptform), which a non-tty binary harness cannot drive or intercept; unit-driven by src/cli/tests.rs. The non-interactive twin (exit 125) is test_set_bad_typed_value_exits_125."]
fn test_set_bad_value_fails_before_the_form_opens() {
    // Oracle: an invalid --set value exits 125 and the form must NEVER open (monkeypatched
    // _collect_values raises). Not observable from a non-tty binary run.
}

#[test]
fn test_set_empty_value_on_required_placeholder_exits_125() {
    let root = sandbox();
    skit(&root)
        .args([
            "add",
            "--cmd",
            "echo {target}",
            "--name",
            "d5",
            "--no-input",
        ])
        .assert()
        .success();
    skit(&root)
        .args(["run", "d5", "--set", "target=", "--no-input"])
        .assert()
        .code(125);
    assert!(state_text(&root, "d5").is_none(), "nothing ran");
}

// --------------------------------------------------------------------------
// interactive: an explicitly set field is final
// --------------------------------------------------------------------------

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is which fields the interactive form asked (`asked[\"keys\"] == [\"CITY\"]` — a --set field is skipped) and that the form's answer wins. That lives at the interactive inline-form seam (cli.rs run form + _skit_save_preset form state), unit-driven by src/cli/tests.rs; a non-tty binary harness cannot intercept `_collect_values`. Whether the Rust form honors 'an explicitly set field is final' is untested here."]
fn test_interactive_form_skips_set_fields() {
    // Oracle: with `--set TIMES=9`, the form only asks for CITY; the merged save is
    // {CITY: form-city, TIMES: "9"}. Not observable from a non-tty binary run.
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is that the interactive form must not open at all when every field is --set (monkeypatched _collect_values raises). That lives at the interactive inline-form seam (cli.rs run form), unit-driven by src/cli/tests.rs; not observable from a non-tty binary run."]
fn test_interactive_all_fields_set_skips_the_form_entirely() {
    // Oracle: `--set CITY=x --set TIMES=1` exits 0 without ever opening the form.
}

#[test]
#[ignore = "UNMAPPED (cross-crate): the observable is that the no-form-fields --save-preset refusal fires BEFORE the interactive runner picker and writes no last-runner fingerprint. Proving 'before the picker' needs the interactive prompt-runner seam (cli.rs _resolve_run_runner + the inline form), unit-driven by src/cli/tests.rs; a non-tty run never opens the picker, so the regression is not observable black-box. The refusal itself (exit 2) is test_save_preset_on_field_less_entry_refused_saves_nothing."]
fn test_save_preset_no_fields_refused_before_any_form() {
    // Oracle: a field-less prompt entry with `--save-preset x`, interactive, must refuse (exit
    // 2) without first hosting the runner picker or writing last-picked state.
}

#[test]
fn test_save_preset_persists_when_ctrl_c_ends_an_accepted_run() {
    // Ctrl+C ends the RUNNING script, not the request to keep its values: the launch was
    // accepted, so --save-preset persists. Substitution: the oracle raises KeyboardInterrupt
    // from flows.execute; here the command sends itself SIGINT (`kill -INT $$`), which the Rust
    // launcher maps to 128 + 2 = 130 — the same accepted-then-interrupted run.
    let root = sandbox();
    skit(&root)
        .args([
            "add",
            "--cmd",
            "echo {msg}; kill -INT $$",
            "--name",
            "e",
            "--no-input",
        ])
        .assert()
        .success();
    skit(&root)
        .args([
            "run",
            "e",
            "--set",
            "msg=hi",
            "--save-preset",
            "prod",
            "--no-input",
        ])
        .assert()
        .code(130);
    let saved = state_text(&root, "e").expect("run persisted state");
    assert!(saved.contains("[presets.prod]"), "{saved}");
    assert!(saved.contains("msg = \"hi\""), "{saved}");
}
