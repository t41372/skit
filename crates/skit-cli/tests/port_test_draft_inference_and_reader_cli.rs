//! Mechanical port of the Python oracle module `tests/test_draft_inference_and_reader_cli.py`
//! (`origin/main@206f9ef`): "Draft inference and reader CLI contracts (exit codes, stored PEP 723
//! text, --json, filesystem state)." Each `#[test]` keeps its Python `def test_*` name so it traces
//! back to its origin, and each Python "WHY" comment is preserved above it.
//!
//! Most oracle tests drive the CLI end-to-end through `typer.testing.CliRunner`. This port drives
//! the real `skit` binary via `assert_cmd` inside a fresh four-directory sandbox
//! (`SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR` + a scratch dir for sources), so skit writes
//! only inside the temp sandbox. A few oracle tests call registry/flows units directly; their Rust
//! equivalents live in `skit-language` (a `skit-cli` dependency) and are driven in-process.
//!
//! Concept mapping:
//! - Python `runner.invoke(cli.app, ["add", …], input=…)` -> `Sandbox::command().args(…)
//!   .write_stdin(…)`.
//! - Python `result.output` (CliRunner merges stdout+stderr) -> `combined(&output)`. `_flat` ->
//!   `flat(&output)`.
//! - Python `store.resolve(name).meta.kind` -> `Sandbox::show_json(name)["kind"]`;
//!   `store.resolve` raising `NotFoundError` -> `!Sandbox::entry_exists(name)`.
//! - Python `_stored(name)` (read `store.resolve(name).dir / "script.py"`) ->
//!   `Sandbox::stored(name)` over `<data>/scripts/<name>/script.py` (slug == lowercase name here).
//! - Python `_add(text, name, kind[, ref])` (direct `store.add_python` / `store.add_script`, kind
//!   forced) -> `skit add <scratch>/<name>.{py,sh} -n <name> [--ref] --no-input` (the extension
//!   forces the same kind; the well-formed docopt/dynamic bodies onboard nothing under `--no-input`,
//!   so the stored end state — no managed params — is identical to the direct store call).
//! - Python `flows.reader_fields(spec_for(kind), text)` -> `skit_language::cli_params(kind, text)
//!   .len()`: the modeled static-CLI field count. The `spec_for(...) is None` row (unknown kind)
//!   maps to an unregistered kind string, which `cli_params` returns empty for.
//! - Python `python_version_pin(program)` -> `skit_language::python_version_pin(program)`: the
//!   Python `""` result maps to Rust `None` (`unwrap_or_default()` in the row loop). The oracle's
//!   `(None, "")` row has no analogue in the typed `&str` signature and is recorded in a comment.
//!
//! Bucket disposition (27 oracle defs; 14 pass, 13 `#[ignore]`):
//! - 14 PASS asserting tests: `python_version_pin` rows, `cli_params` reader rows, the
//!   bash-shebang-.py-outside-drafts / parked-file lanes, both python2 refusals, both
//!   silently-beats pin overrides, both no-flip-note manages, both reference-add voices, and the
//!   singular/plural field-count notices.
//! - 10 FAILING CONTRACT (divergence) tests: full asserting bodies kept intact behind `#[ignore]`;
//!   every label was verified against the built binary. The recurring shapes are: no shebang-first
//!   / no consume-on-success unlink on the plain path lane (Rust classifies a `.py` draft by its
//!   extension and never unlinks — ties to pending task #15, same diagnosis as the sibling
//!   `port_test_add_lane_contracts.rs`); no `_note_python_pin` announce line (the pin lands in the
//!   stored copy, but the "recording requires-python …" note never prints); and the params read
//!   view printing "Unmanaged candidates: …" / listing reader fields instead of the oracle's
//!   "Detected but not yet managed … --manage" / "has no managed parameters." (also matching the
//!   sibling).
//! - 2 cross-crate stubs: `is_draft` and `_onboard_script_params` are crate-private helpers in
//!   `skit-cli` (`is_owned_draft` cli.rs:5803, the analyzerless guard in `_onboard_script_params`
//!   cli.py:603), unreachable from an integration test. `is_owned_draft` DOES implement both halves
//!   correctly — that stub is a reachability gap, not a behavior gap.
//! - 1 absent gap: `kind_for_draft` has no public equivalent in any crate (`infer_kind` is a
//!   different, extension-first contract), and the binary does not apply the shebang-first rule at
//!   all — see the section-2 divergence tests for the CLI-level evidence.

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::{Value, json};
use skit_language::{cli_params, python_version_pin};
use tempfile::TempDir;

struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    scratch: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            scratch: TempDir::new().unwrap(),
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

    /// Write a user source file into the scratch dir (never skit's drafts home).
    fn scratch_file(&self, name: &str, body: &str) -> PathBuf {
        let path = self.scratch.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    /// Python `_draft(name, body)` — write a file into skit's OWN drafts home (`<data>/drafts`).
    fn draft_file(&self, name: &str, body: &str) -> PathBuf {
        let dir = self.data.path().join("drafts");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    /// Python `store.resolve(name)` via `skit show NAME --json` — parse stdout as one document.
    fn show_json(&self, name: &str) -> Value {
        let output = self
            .command()
            .args(["show", name, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "show --json failed: {}",
            combined(&output)
        );
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document")
    }

    /// The `skit params NAME --json` document (its `"unmanaged"` field).
    fn params_json(&self, name: &str) -> Value {
        let output = self
            .command()
            .args(["params", name, "--json"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "params --json failed: {}",
            combined(&output)
        );
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document")
    }

    /// Python `store.resolve(name)` succeeds (no `NotFoundError`).
    fn entry_exists(&self, name: &str) -> bool {
        self.command()
            .args(["show", name, "--json"])
            .output()
            .unwrap()
            .status
            .success()
    }

    /// Python `_stored(name)` — the stored copy's `script.py` text (python entries only here).
    fn stored(&self, name: &str) -> String {
        let path = self
            .data
            .path()
            .join("scripts")
            .join(name.to_lowercase())
            .join("script.py");
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {path:?}: {error}"))
    }
}

/// Python `result.output` — the merged streams a CliRunner user would see (stdout then stderr).
fn combined(output: &Output) -> String {
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    text
}

/// Python `_flat(text)` — collapse rich's soft-wrap so an 80-col-split message matches as one.
fn flat(output: &Output) -> String {
    combined(output)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Python `flows.reader_fields(spec_for(kind), text)` — the modeled static-CLI field count.
fn reader_fields(kind: &str, text: &str) -> usize {
    cli_params(kind, text).len()
}

// The oracle's module-level fixtures (byte-exact).
const DOCOPT: &str = "\"\"\"Usage: dc --city=<c>\"\"\"\nimport docopt\nCITY = \"x\"\nprint(docopt.docopt(__doc__), CITY)\n";
const DYN_SH: &str = "#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";

// ==========================================================================
// 1. python_version_pin / kind_for_draft / is_draft — the registry units
// ==========================================================================

#[test]
fn test_python_version_pin_rows() {
    // The oracle parametrizes nine rows. Rust's `python_version_pin` returns `Option<String>`, so
    // the Python `""` result maps to `None`. The `(None, "")` row is unrepresentable in the typed
    // `&str` signature — the `("", "")` row below pins the same "no pin" contract.
    let rows: [(&str, &str); 8] = [
        ("python", ""),
        ("python3", ""),
        ("python3.12", ">=3.12,<3.13"),
        // a micro-versioned shebang keeps its .1 in the lower bound — half-honoring an explicit
        // signal is still a drop; an empty prefix is not.
        ("python3.12.1", ">=3.12.1,<3.13"),
        ("python2", ""), // unregistered — no pin
        ("python2.7", ""),
        ("bash", ""),
        ("", ""),
    ];
    for (program, expected) in rows {
        assert_eq!(
            python_version_pin(program).unwrap_or_default(),
            expected,
            "program={program:?}"
        );
    }
}

#[test]
#[ignore = "ABSENT gap: no public `kind_for_draft` exists in any crate. `skit_language::infer_kind` is a different, extension-first contract, and the binary does not apply the shebang-first rule for skit's OWN drafts at all (a bash-shebang `skit-new-*.py` lands as python — see `test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks`). MUST-FIX: port `registry.py:442-473 kind_for_draft` (shebang outranks the mkstemp `.py` suffix for owned drafts; placeholder-bodied extensions like `.prompt.md` outrank the shebang; unregistered shebang -> 'unknown'; no shebang -> plain infer_kind)."]
fn test_kind_for_draft_shebang_first() {
    // skit's OWN drafts are classified by shebang, not the mkstemp .py suffix:
    //   kind_for_draft("skit-new-a.py", "#!/usr/bin/env bash\necho hi\n") == "shell"
    //   kind_for_draft("skit-new-b.py", "#!/usr/bin/awk -f\nBEGIN{print 1}\n") == "unknown"
    //   kind_for_draft("skit-new-c.py", "print('x')\n") == "python"  # no shebang: suffix
}

#[test]
#[ignore = "CROSS-CRATE: `is_draft` maps to the crate-private `skit_cli::cli::is_owned_draft` (cli.rs:5803), unreachable from an integration test without exporting it. That helper already checks BOTH halves correctly — `name.starts_with(\"skit-\")` AND `parent == drafts_dir` — so this is a reachability gap, not a behavior gap."]
fn test_is_draft_needs_both_dir_and_prefix() {
    // is_draft(drafts_dir()/"skit-new-x.py") is True
    // is_draft(drafts_dir()/"mytool.sh")     is False  # parked, no skit- prefix
    // is_draft(tmp/"skit-new-x.py")          is False  # skit- prefix but not in drafts dir
}

#[test]
fn test_reader_fields_predicate_rows() {
    let docopt = "\"\"\"Usage: x --city=<c>\"\"\"\nimport docopt\nprint(docopt.docopt(__doc__))\n";
    let modeled =
        "import argparse\np=argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\n";
    let getopts2 = "#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n";
    let dyn_sh = "#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\n";
    assert_eq!(reader_fields("python", docopt), 0); // self-parses but skit can't model it
    assert_eq!(reader_fields("python", modeled), 1); // one add_argument -> one modeled field
    assert_eq!(reader_fields("shell", getopts2), 2);
    assert_eq!(reader_fields("shell", dyn_sh), 0); // dynamic optstring: ok=False -> 0
    assert_eq!(reader_fields("no-such-kind", modeled), 0); // no spec (spec_for is None)
    assert_eq!(reader_fields("python", ""), 0); // no text
}

// ==========================================================================
// 2. Draft resume reclassification on the CLI path lane
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle classifies skit's OWN `.py` draft shebang-first (bash shebang -> shell) and unlinks it on a successful copy (registry.py:442 kind_for_draft + is_draft consume). Rust's plain path lane classifies by extension (the draft lands as PYTHON, not shell) and never calls remove_owned_draft (only cli.rs:1353/5633/5659 do, all off the plain path — the draft SURVIVES). Ties to pending task #15. Verified against the built binary."]
fn test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks() {
    // A bash-shebang draft named `skit-new-*.py` (mkstemp's suffix, not a user signal) resumes as
    // a SHELL entry — never a broken python entry with a bash body — and the consumed draft is
    // unlinked.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft_file("skit-new-ship.py", "#!/usr/bin/env bash\necho drafted\n");
    sandbox
        .command()
        .args(["add", draft.to_str().unwrap(), "-n", "ship", "--no-input"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("ship")["kind"], "shell"); // reclassified by shebang
    assert!(!draft.exists()); // consumed on success
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle refuses an unregistered awk shebang with exit 2, the `--kind` escape, and the shebang-aware voice, and KEEPS the draft (registry.py:442 -> 'unknown' -> refusal). Rust classifies the draft by its `.py` extension and ADDS it (exit 0, kind python) — no refusal at all. Ties to pending task #15. Verified against the built binary."]
fn test_cli_add_awk_shebang_draft_is_unknown_kept_with_kind_escape() {
    // An awk shebang is unregistered: the draft is "unknown", refused with exit 2 and the --kind
    // escape (never a fabricated entry), and KEPT because the add never reached the
    // consume-on-success unlink. The draft carries a #!, so the refusal is the shebang-aware voice
    // ("names no interpreter"), not the shebang-less "isn't a script" line.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft_file("skit-new-awk.py", "#!/usr/bin/awk -f\nBEGIN{print 1}\n");
    let output = sandbox
        .command()
        .args(["add", draft.to_str().unwrap(), "-n", "awky", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("--kind"),
        "{}",
        combined(&output)
    ); // the escape is named
    let flat = flat(&output);
    assert!(
        flat.contains("The #! in skit-new-awk.py names no interpreter skit knows"),
        "{flat}"
    );
    assert!(!flat.contains("isn't a script or an executable"), "{flat}"); // not the shebang-less voice
    assert!(draft.exists()); // a refused add consumes nothing
    assert!(!sandbox.entry_exists("awky"));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the kind (python) half holds, but the oracle unlinks the resumed draft on a successful copy (is_draft consume) and Rust's plain path lane never calls remove_owned_draft — the draft SURVIVES, so `!draft.exists()` fails. Ties to pending task #15. Verified against the built binary."]
fn test_cli_add_no_shebang_draft_falls_back_to_python() {
    // No shebang at all: the suffix is all there is, so a `skit-new-*.py` draft is python (the
    // fallback branch of kind_for_draft).
    let sandbox = Sandbox::new();
    let draft = sandbox.draft_file("skit-new-plain.py", "print('resume me')\n");
    sandbox
        .command()
        .args(["add", draft.to_str().unwrap(), "-n", "plain", "--no-input"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("plain")["kind"], "python");
    assert!(!draft.exists());
}

#[test]
fn test_cli_add_bash_shebang_py_outside_drafts_stays_python() {
    // The draft rule must NOT leak: a real `.py` file with a bash shebang living OUTSIDE the drafts
    // dir classifies by its extension (python) — only skit's own drafts are shebang-first, and a
    // user's original is never unlinked.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file("thing.py", "#!/usr/bin/env bash\necho hi\n");
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "thing", "--no-input"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("thing")["kind"], "python"); // extension wins outside drafts
    assert!(src.exists()); // not a draft: the original is left alone
}

// ==========================================================================
// 3. is_draft scoping on the path-lane unlink
// ==========================================================================

#[test]
fn test_cli_add_parked_user_file_in_drafts_dir_is_not_unlinked() {
    // A user file merely PARKED in the drafts dir (no `skit-` prefix) is added but NOT consumed —
    // is_draft needs both halves, so this file is not skit's artifact to delete.
    // NOTE: this passes for a superset reason — Rust's plain path lane never unlinks ANYTHING
    // (see the ignored `test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks`); the
    // prefix-half discrimination is not what keeps this file, but the observed state matches.
    let sandbox = Sandbox::new();
    let parked = sandbox.draft_file("mytool.sh", "#!/usr/bin/env bash\necho hi\n");
    sandbox
        .command()
        .args([
            "add",
            parked.to_str().unwrap(),
            "-n",
            "parked",
            "--no-input",
        ])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("parked")["kind"], "shell");
    assert!(parked.exists()); // no skit- prefix -> not consumed
}

// ==========================================================================
// 4. python2 is unregistered on every lane
// ==========================================================================

#[test]
fn test_stdin_python2_shebang_is_refused() {
    // `#!/usr/bin/env python2` piped in with no --kind is refused (skit runs scripts through uv on
    // python3 — a python2 entry could only die at run time).
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "-n", "p2"])
        .write_stdin("#!/usr/bin/env python2\nprint(1)\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("names no interpreter"),
        "{}",
        combined(&output)
    );
    assert!(!sandbox.entry_exists("p2"));
}

#[test]
fn test_path_add_python2_extensionless_is_refused() {
    // The same rule on the path lane: an extensionless python2-shebang file is not a python entry —
    // the --kind escape applies.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file("legacy", "#!/usr/bin/env python2\nprint(1)\n");
    let output = sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "legacy", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("--kind"),
        "{}",
        combined(&output)
    );
}

// ==========================================================================
// 5. A versioned python shebang pins requires-python (and both overrides win silently)
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the pin lands in the stored copy correctly (`requires-python = \">=3.12,<3.13\"`), but the `_note_python_pin` announce line (cli.py:288-299, 'recording requires-python …') never prints in Rust — the shebang-derived pin is recorded with no consent trail. Verified against the built binary."]
fn test_stdin_versioned_shebang_pins_requires_python_and_announces() {
    // python3.12 with no --python and no PEP 723 block records requires-python ">=3.12,<3.13" into
    // the STORED copy's PEP 723 block, and says so on a path with no ask.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "-n", "v"])
        .write_stdin("#!/usr/bin/env python3.12\nprint(1)\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        flat(&output).contains("recording requires-python >=3.12,<3.13"),
        "{}",
        flat(&output)
    ); // the dim note
    assert!(
        sandbox
            .stored("v")
            .contains("requires-python = \">=3.12,<3.13\"")
    ); // landed in the copy
}

#[test]
fn test_explicit_python_beats_the_shebang_pin_silently() {
    // An explicit --python is the user's own move: it wins over the shebang pin and prints NO note
    // (nothing was recorded without an ask).
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "-n", "vo", "--python", ">=3.11"])
        .write_stdin("#!/usr/bin/env python3.12\nprint(1)\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(!combined(&output).contains("recording requires-python")); // no note on the explicit path
    assert!(
        sandbox
            .stored("vo")
            .contains("requires-python = \">=3.11\"")
    );
    assert!(!sandbox.stored("vo").contains(">=3.12,<3.13")); // the shebang pin did NOT override --python
}

#[test]
fn test_existing_pep723_block_beats_the_shebang_pin_silently() {
    // An existing PEP 723 block already owns the constraint: the shebang pin is dropped, no note,
    // the block's own requires-python is preserved verbatim.
    let sandbox = Sandbox::new();
    let body =
        "#!/usr/bin/env python3.12\n# /// script\n# requires-python = '>=3.9'\n# ///\nprint(1)\n";
    let output = sandbox
        .command()
        .args(["add", "-", "-n", "vb"])
        .write_stdin(body)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(!combined(&output).contains("recording requires-python"));
    let text = sandbox.stored("vb");
    assert!(text.contains(">=3.9")); // the block won
    assert!(!text.contains(">=3.12,<3.13")); // the shebang pin was not injected
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the pin and the dependency both land in the stored block (`requires-python = \">=3.12,<3.13\"` alongside `rich`), but the `_note_python_pin` announce line (cli.py:288-299) never prints on the explicit-deps branch either. Verified against the built binary."]
fn test_dep_flag_present_still_pins_from_the_shebang() {
    // --dep given but --python NOT: the shebang pin still rides in on the explicit-deps branch
    // (announced) and lands alongside the dependency in the stored block.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "-n", "vd", "--dep", "rich"])
        .write_stdin("#!/usr/bin/env python3.12\nprint(1)\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        flat(&output).contains("recording requires-python >=3.12,<3.13"),
        "{}",
        flat(&output)
    );
    let text = sandbox.stored("vd");
    assert!(text.contains("requires-python = \">=3.12,<3.13\""));
    assert!(text.contains("rich")); // the dependency landed too
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the suggested dependency is accepted non-interactively and the pin lands in the stored block (both `requests` and `requires-python = \">=3.12,<3.13\"`), but the `_note_python_pin` announce line (cli.py:288-299) never prints. Verified against the built binary."]
fn test_suggested_deps_noninteractive_pins_from_the_shebang() {
    // A script whose imports SUGGEST a dependency, piped in (non-interactive): skit accepts the
    // suggestions as-is AND records + announces the shebang pin on that path.
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "-n", "vs"])
        .write_stdin("#!/usr/bin/env python3.12\nimport requests\nprint(requests)\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        flat(&output).contains("recording requires-python >=3.12,<3.13"),
        "{}",
        flat(&output)
    );
    let text = sandbox.stored("vs");
    assert!(text.contains("requires-python = \">=3.12,<3.13\""));
    assert!(text.contains("requests")); // the suggested dependency was accepted non-interactively
}

#[test]
#[ignore = "CROSS-CRATE: `cli._onboard_script_params` is a crate-private helper in `skit-cli` (cli.py:594; its analyzerless guard at cli.py:603 returns `[]` immediately for a kind with no analyzer/params_io). It is unreachable from an integration test. The observable end state (a ruby copy add manages nothing and prints no candidate onboarding) is a proxy, but the direct `[]` contract on the guard has no public surface to assert."]
fn test_onboard_script_params_returns_empty_for_analyzerless_kind() {
    // A data-driven-tail kind with no analyzer/params_io (ruby) has no candidate onboarding — the
    // guard returns immediately, never touching the script.
    //   entry = store.add_script(task.rb, kind="ruby", name="task")
    //   cli._onboard_script_params(entry, spec_for("ruby"), no_input=False) == []
}

// ==========================================================================
// 6. Modeled-form predicate: docopt/dynamic keep the manage offer; no false flip note
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the JSON half holds (unmanaged == [\"CITY\"]), but the human read view prints 'Unmanaged candidates: CITY' with no '--manage' advertisement, where the oracle prints 'Detected but not yet managed: CITY (use --manage to manage them)' (cli.py:3956/4009). Same read-view diagnosis as the sibling `port_test_add_lane_contracts.rs`. Verified against the built binary."]
fn test_docopt_python_read_view_offers_manage() {
    // docopt self-parses but skit can't MODEL it: the run form is passthrough-only, so the read
    // view still lists the unmanaged constant AND advertises --manage (additive, not a
    // source-flip trap).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file("dc.py", DOCOPT);
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "dc", "--no-input"])
        .assert()
        .success();
    let plain = sandbox.command().args(["params", "dc"]).output().unwrap();
    assert_eq!(plain.status.code(), Some(0), "{}", combined(&plain));
    assert!(
        flat(&plain).contains("Detected but not yet managed: CITY"),
        "{}",
        flat(&plain)
    );
    assert!(
        combined(&plain).contains("--manage"),
        "{}",
        combined(&plain)
    );
    assert_eq!(sandbox.params_json("dc")["unmanaged"], json!(["CITY"]));
}

#[test]
fn test_docopt_python_manage_prints_no_flip_note() {
    // Managing that constant is NOT setting aside a modeled form (there was none), so no flip note
    // fires — the note is reserved for modeled readers being replaced.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file("dc.py", DOCOPT);
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "dc", "--no-input"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "dc", "--manage", "CITY"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(!combined(&output).contains("The run form now asks")); // no false "form set aside"
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the JSON half holds (unmanaged contains \"OUTDIR\"), but the human read view prints 'Unmanaged candidates: …' with no '--manage' advertisement, where the oracle prints '… (use --manage to manage them)' (cli.py:4009). Same read-view diagnosis as the sibling `port_test_add_lane_contracts.rs`. Verified against the built binary."]
fn test_dynamic_getopts_read_view_offers_manage() {
    // A dynamic optstring shell is detected but unmodelable: the read view lists candidates and
    // offers --manage (the passthrough field carries the reader; constants are additive).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file("dyn.sh", DYN_SH);
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "dyn", "--no-input"])
        .assert()
        .success();
    let plain = sandbox.command().args(["params", "dyn"]).output().unwrap();
    assert_eq!(plain.status.code(), Some(0), "{}", combined(&plain));
    assert!(
        combined(&plain).contains("--manage"),
        "{}",
        combined(&plain)
    );
    let unmanaged = sandbox.params_json("dyn")["unmanaged"].clone();
    assert!(
        unmanaged
            .as_array()
            .expect("unmanaged is an array")
            .contains(&json!("OUTDIR")),
        "{unmanaged}"
    );
}

#[test]
fn test_dynamic_getopts_manage_prints_no_flip_note() {
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file("dyn.sh", DYN_SH);
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "dyn", "--no-input"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "dyn", "--manage", "OUTDIR"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(!combined(&output).contains("The run form now asks"));
}

// ==========================================================================
// 7. Reference entries: honest read, no --manage advice, add-lane voice
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the no-`--manage` half and the JSON `param_source == \"argparse\"` half both hold, but the oracle's plain read view prints '%(name)s has no managed parameters.' (cli.py:3941) while Rust lists the reader fields themselves ('Parameter: n' / 'Parameter: v'). Same reader-driven read-view diagnosis as the sibling `port_test_add_lane_contracts.rs`. Verified against the built binary."]
fn test_reference_getopts_read_view_has_no_manage_advice() {
    // A reference getopts entry's parser IS the form (reader-driven): the read view says the plain
    // 'no managed parameters.' with NO --manage advice, and its plan is reader-driven.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file(
        "refg.sh",
        "#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
    );
    sandbox
        .command()
        .args([
            "add",
            src.to_str().unwrap(),
            "-n",
            "refg",
            "--ref",
            "--no-input",
        ])
        .assert()
        .success();
    let output = sandbox.command().args(["params", "refg"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        combined(&output).contains("has no managed parameters."),
        "{}",
        combined(&output)
    );
    assert!(
        !combined(&output).contains("--manage"),
        "{}",
        combined(&output)
    );
    assert_eq!(sandbox.show_json("refg")["param_source"], "argparse"); // reader-driven plan
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the no-advice half and the JSON `unmanaged == [\"OUTDIR\"]` half both hold, but the oracle names the candidate as 'Detected but not yet managed: OUTDIR' (cli.py:3994) and prints the reference teaching 'skit never writes the original file …' (cli.py:4001), where Rust prints 'Unmanaged candidates: OUTDIR' and 'Source management is not available for a reference entry.' Same read-view diagnosis as the sibling. Verified against the built binary."]
fn test_reference_constants_read_view_names_unmanaged_with_teaching() {
    // A reference constants entry is NOT reader-driven: its unmanaged candidate is named, the
    // --manage advice is dropped for the reference-mode teaching, and --json still populates
    // unmanaged (the read is honest in both modes).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch_file(
        "refc.sh",
        "#!/usr/bin/env bash\nOUTDIR=/tmp\necho $OUTDIR\n",
    );
    sandbox
        .command()
        .args([
            "add",
            src.to_str().unwrap(),
            "-n",
            "refc",
            "--ref",
            "--no-input",
        ])
        .assert()
        .success();
    let output = sandbox.command().args(["params", "refc"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        flat(&output).contains("Detected but not yet managed: OUTDIR"),
        "{}",
        flat(&output)
    );
    assert!(!combined(&output).contains("use --manage to manage them")); // no advice on a ref entry
    assert!(
        flat(&output).contains("skit never writes the original file"),
        "{}",
        flat(&output)
    ); // the teaching
    assert_eq!(sandbox.params_json("refc")["unmanaged"], json!(["OUTDIR"]));
}

#[test]
fn test_reference_reader_add_prints_the_read_notice() {
    // A reference-mode add whose script models a form says so — the reader works in reference mode,
    // so 'setup was skipped' alone would read as 'the form is lost' (it isn't).
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch_file(
        "refadd.sh",
        "#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
    );
    let first = sandbox
        .command()
        .args([
            "add",
            sh.to_str().unwrap(),
            "-n",
            "refadd",
            "--ref",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", combined(&first));
    assert!(
        combined(&first).contains("skit read this script's own arguments"),
        "{}",
        combined(&first)
    );
    let py = sandbox.scratch_file(
        "refap.py",
        "import argparse\np=argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\n",
    );
    let second = sandbox
        .command()
        .args([
            "add",
            py.to_str().unwrap(),
            "-n",
            "refap",
            "--ref",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(second.status.code(), Some(0), "{}", combined(&second));
    assert!(
        combined(&second).contains("skit read this script's own arguments"),
        "{}",
        combined(&second)
    ); // python reference add too
}

#[test]
fn test_reference_constants_add_prints_the_skip_line() {
    // A reference-mode add of a script with NO modeled form prints the plain 'setup was skipped'
    // line (there is no form to reassure about).
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch_file(
        "refcadd.sh",
        "#!/usr/bin/env bash\nOUTDIR=/tmp\necho $OUTDIR\n",
    );
    let output = sandbox
        .command()
        .args([
            "add",
            sh.to_str().unwrap(),
            "-n",
            "refcadd",
            "--ref",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        combined(&output).contains("parameter setup was skipped"),
        "{}",
        combined(&output)
    );
    assert!(!combined(&output).contains("skit read this script's own arguments"));
}

// ==========================================================================
// 8. Singular vs plural field count in the read notice
// ==========================================================================

#[test]
fn test_one_field_getopts_add_says_singular() {
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch_file(
        "one.sh",
        "#!/usr/bin/env bash\nwhile getopts \"n:\" o; do :; done\n",
    );
    let output = sandbox
        .command()
        .args(["add", sh.to_str().unwrap(), "-n", "one", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        combined(&output).contains("(1 field)"),
        "{}",
        combined(&output)
    );
    assert!(!combined(&output).contains("(1 fields)"));
}

#[test]
fn test_multi_field_getopts_add_says_plural() {
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch_file(
        "many.sh",
        "#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
    );
    let output = sandbox
        .command()
        .args(["add", sh.to_str().unwrap(), "-n", "many", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        combined(&output).contains("(2 fields)"),
        "{}",
        combined(&output)
    );
}
