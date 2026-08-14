//! Mechanical port of the Python oracle module `tests/test_add_feedback_contracts.py`
//! (`origin/main@206f9ef`): "Add feedback contracts (exit codes, stored PEP 723 text,
//! prompt-ask labels, filesystem state, notice counts)." Each `#[test]` keeps its Python
//! `def test_*` name so it traces back to its origin, and each Python "WHY" comment is
//! preserved above it.
//!
//! Most oracle tests drive the CLI end-to-end through `typer.testing.CliRunner`. This port
//! drives the real `skit` binary via `assert_cmd` inside a fresh four-directory sandbox
//! (`SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR` + a scratch dir for sources), so skit
//! writes only inside the temp sandbox. A few oracle tests call `cli`/`langs.registry` units
//! directly; the reachable ones live in `skit-language` (a `skit-cli` dependency) and are
//! driven in-process, the private-`cli`-helper ones are stubbed (see the bucket disposition).
//!
//! Concept mapping:
//! - Python `runner.invoke(cli.app, ["add", …], input=…)` -> `Sandbox::command().args(…)
//!   .write_stdin(…)`. `result.output` (CliRunner merges stdout+stderr) -> `combined(&output)`;
//!   `_flat` (collapse rich soft-wrap) -> `flat(&output)`.
//! - Python `store.resolve(name).meta.kind`/`.mode` -> `Sandbox::show_json(name)["kind"]`/
//!   `["mode"]`; `store.resolve` raising `NotFoundError` -> `!Sandbox::entry_exists(name)`.
//! - Python `_draft(name, body)` (write under `drafts_dir()`) -> `Sandbox::draft(name, body)`
//!   over `<data>/drafts/<name>`. The `skit-` prefix keeps `is_owned_draft`'s check true.
//! - Python `store.resolve(name).dir / "script.py"` -> `Sandbox::stored(name)` over
//!   `<data>/scripts/<name>/script.py` (slug == lowercase name here).
//! - Python `pep723.parse_block(stored)["dependencies"]` -> `skit_language::read_uv_metadata
//!   (&stored).dependencies` (the metawriter exemplar's established mapping).
//! - Python `python_version_pin(program)` -> `skit_language::python_version_pin(program)`: the
//!   Python `""` result maps to Rust `None`, a non-empty pin to `Some(pin)`.
//! - Python `cli._resolve_python_metadata(text, None, None, no_input=True)` (deps half) ->
//!   `skit_language::external_dependencies_at("python", text, None)`. The `py == ""` half is
//!   the "no shebang -> nothing to pin" rule: the first line is not a `#!`, so the add path's
//!   `shebang.and_then(shebang_program).and_then(python_version_pin)` yields `None` -> `""`.
//!
//! Bucket disposition (16 oracle defs):
//! - PASS asserting tests: the whole set the built binary already honors (assigned by the run).
//! - FAILING CONTRACT (divergence): full asserting bodies kept intact behind `#[ignore]`; every
//!   label was verified against the built binary.
//! - 3 absent gaps: `cli._resolve_python_metadata`'s INTERACTIVE python-metadata ask
//!   (deps/python `Prompt.ask` with the pin-aware label switch, src/skit/cli.py:224-261) has no
//!   equivalent anywhere in the Rust workspace — the add flow resolves deps/python
//!   non-interactively (`external_dependencies_at`, cli.rs:2920) and never asks. The label
//!   strings "Enter accepts the #! pin" / "leave empty for automatic" appear in NO crate. These
//!   three call a function that does not exist, so they are `#[ignore]` stubs with a MUST-FIX.
//! - 2 cross-crate stubs: `cli._print_add_hints` is the crate-private `print_copy_onboarding_facts`
//!   (cli.rs:2669-2694) in the `skit-cli` binary, unreachable from an integration test. Its
//!   argv-yields-to-framework gate IS implemented (cli.rs:2677) and IS covered end-to-end by
//!   `test_dynamic_optstring_with_argv_names_extra_arguments_once` in this same file — a
//!   reachability gap, not a behavior gap.

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;
use skit_language::{
    external_dependencies_at, python_version_pin, read_uv_metadata, shebang_program,
};
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

    /// Python `_draft(name, body)` — write a file under skit's OWN drafts home so
    /// `is_owned_draft` sees it (the `skit-` prefix is the check the guard keys on).
    fn draft(&self, name: &str, body: &str) -> PathBuf {
        let dir = self.data.path().join("drafts");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        path
    }

    /// Python `store.resolve(name)` via `skit show NAME --json` — one document on stdout.
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

    /// Python `store.resolve(name)` succeeds (no `NotFoundError`).
    fn entry_exists(&self, name: &str) -> bool {
        self.command()
            .args(["show", name, "--json"])
            .output()
            .unwrap()
            .status
            .success()
    }

    /// Python `store.resolve(name).dir / "script.py"` — the stored copy's text.
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

// ==========================================================================
// 1. --ref on a kept draft is refused
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle refuses --ref against its OWN kept draft (a reference into drafts/ would list a live entry's file as a resumable/deletable draft) with exit 2 '… one of skit's own kept drafts … Drop --ref.' (src/skit/cli.py:1917-1933). Rust's plain path lane has no kept-draft guard — it ADDS the reference ('Added: linky (reference mode)', exit 0) and the draft remains. Ties to pending task #15. Verified against the built binary."]
fn test_ref_on_kept_draft_is_refused_and_keeps_it() {
    // --ref into drafts/ would leave a live entry's file resumable — refuse it: exit 2, the
    // 'kept drafts' message naming Drop --ref, the draft kept, and NO entry created.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft("skit-new-linkme.py", "print('link me')\n");
    let output = sandbox
        .command()
        .args([
            "add",
            draft.to_str().unwrap(),
            "-n",
            "linky",
            "--ref",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    let flat = flat(&output);
    assert!(flat.contains("one of skit's own kept drafts"), "{flat}");
    assert!(flat.contains("Drop --ref"), "{flat}");
    assert!(draft.exists()); // a refused add consumes nothing
    assert!(!sandbox.entry_exists("linky"));
}

#[test]
fn test_ref_on_a_normal_file_still_works() {
    // The refusal is scoped to drafts: --ref on a user's OWN file is untouched (a reference
    // entry that points at a real path is exactly what --ref is for).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("mine.py");
    fs::write(&src, "print('mine')\n").unwrap();
    sandbox
        .command()
        .args([
            "add",
            src.to_str().unwrap(),
            "-n",
            "mine",
            "--ref",
            "--no-input",
        ])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("mine")["mode"], "reference");
    assert!(src.exists());
}

// ==========================================================================
// 2. Prompt draft with a #! body resumes as a PROMPT (compound suffix outranks shebang)
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the kind half HOLDS (infer_kind maps `.prompt.md` to prompt, cli.rs → skit-language lib.rs:220, so exit 0 and kind == prompt), but the consume-on-success unlink is MISSING: Rust's plain path lane never calls remove_owned_draft (only the authoring lanes do), so the resumed draft SURVIVES — `!draft.exists()` fails. Ties to pending task #15, same shape as the sibling port_test_add_lane_contracts.rs. Verified against the built binary."]
fn test_prompt_draft_with_shebang_body_resumes_as_prompt() {
    // A `skit-new-*.prompt.md` draft whose body opens `#!/usr/bin/env bash` resumes as a
    // PROMPT, not shell — the .prompt.md suffix is the user's lane choice, and a prompt body may
    // legitimately quote a shebang line. The consumed draft is unlinked on success.
    let sandbox = Sandbox::new();
    let draft = sandbox.draft(
        "skit-new-summ.prompt.md",
        "#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );
    let output = sandbox
        .command()
        .args(["add", draft.to_str().unwrap(), "-n", "summ", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(sandbox.show_json("summ")["kind"], "prompt"); // compound suffix wins over the shebang
    assert!(!draft.exists()); // consumed on success
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle classifies a KEPT draft shebang-FIRST — store.infer_kind delegates to registry.kind_for_draft for is_draft paths (src/skit/store.py:308-309, src/skit/langs/registry.py:442-457), so a bash-shebang `.py` draft resumes as shell (the mkstemp suffix is skit's artifact, not a user signal). Rust has no draft-specific classifier: its infer_kind is extension-FIRST (skit-language lib.rs:223 before the shebang branch at 226), so a `.py` file is always python — the add succeeds as kind == 'python', not 'shell'. Ties to pending task #15. Verified against the built binary."]
fn test_py_draft_with_shebang_body_still_resumes_as_shell() {
    // The complement / regression pin: a SCRIPT-starter `.py` draft is still shebang-first,
    // so a bash body resumes as shell (only the compound prompt suffix outranks the shebang).
    let sandbox = Sandbox::new();
    let draft = sandbox.draft(
        "skit-new-shellish.py",
        "#!/usr/bin/env bash\necho drafted\n",
    );
    let output = sandbox
        .command()
        .args([
            "add",
            draft.to_str().unwrap(),
            "-n",
            "shellish",
            "--no-input",
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(sandbox.show_json("shellish")["kind"], "shell");
    assert!(!draft.exists());
}

// ==========================================================================
// 3. The interactive python ask label is honest about what Enter does
// ==========================================================================

#[test]
#[ignore = "ABSENT GAP (kind=absent): the INTERACTIVE python-metadata ask has no equivalent in the Rust workspace. cli._resolve_python_metadata (src/skit/cli.py:224-261) asks Prompt.ask for deps then the python version, switching the label to 'Python version (Enter accepts the #! pin, ...)' when a versioned shebang seeds a pin (cli.py:249-253) and passing the pin as the ask default (cli.py:255). Rust's add flow resolves deps/python NON-interactively (external_dependencies_at, cli.rs:2920) and never asks; the label strings 'Enter accepts the #! pin' / 'leave empty for automatic' appear in NO crate. MUST FIX: add the interactive python-metadata ask with the pin-aware label. No public function to call, so this is a stub."]
fn test_python_ask_label_names_the_pin_and_enter_records_it() {
    // With a #! pin as the default, the label reads 'Enter accepts the #! pin' (never the
    // 'leave empty' lie), and returning the pin (Enter) records it.
    // Python: deps '-' (none), then Enter=pin ->
    //   deps, py = cli._resolve_python_metadata(_PIN_TEXT, None, None, no_input=False)
    //   assert deps == []
    //   assert "Enter accepts the #! pin" in <the python ask's label>
    //   assert "leave empty" not in <label>
    //   assert py == ">=3.12,<3.13"   # Enter recorded the pin (python3.12 shebang)
}

#[test]
#[ignore = "ABSENT GAP (kind=absent): the INTERACTIVE python-metadata ask has no equivalent in the Rust workspace (see test_python_ask_label_names_the_pin_and_enter_records_it). cli._resolve_python_metadata (src/skit/cli.py:254-257) treats '-'/'none' at the python ask as automatic, returning '' even when a #! pin seeded the default. MUST FIX: add the interactive ask with the '-'-means-automatic escape. No public function to call, so this is a stub."]
fn test_python_ask_dash_records_automatic_even_with_a_pin() {
    // '-' at the pin-aware ask really means automatic — an empty requires-python, not the pin.
    // Python: deps none, python '-' -> automatic
    //   _deps, py = cli._resolve_python_metadata(_PIN_TEXT, None, None, no_input=False)
    //   assert py == ""
}

#[test]
#[ignore = "ABSENT GAP (kind=absent): the INTERACTIVE python-metadata ask has no equivalent in the Rust workspace (see test_python_ask_label_names_the_pin_and_enter_records_it). With no #! pin, cli._resolve_python_metadata (src/skit/cli.py:252) keeps the original 'Python version (leave empty for automatic)' label. MUST FIX: add the interactive ask with the no-pin label voice. No public function to call, so this is a stub."]
fn test_python_ask_label_is_leave_empty_without_a_pin() {
    // No #! pin: the label keeps the original 'leave empty for automatic' voice, and '-'
    // there is automatic too.
    // Python:
    //   _deps, py = cli._resolve_python_metadata(_NOPIN_TEXT, None, None, no_input=False)
    //   assert "leave empty for automatic" in <label>
    //   assert "Enter accepts the #! pin" not in <label>
    //   assert py == ""
}

// ==========================================================================
// 4. A micro-versioned shebang keeps its .1 in the stored PEP 723 block
// ==========================================================================

#[test]
fn test_micro_version_pin_unit() {
    // Python `""` maps to Rust `None`; a non-empty pin maps to `Some(pin)`.
    assert_eq!(
        python_version_pin("python3.12.1"),
        Some(">=3.12.1,<3.13".to_owned())
    );
    assert_eq!(
        python_version_pin("python3.12.1.7"),
        Some(">=3.12.1.7,<3.13".to_owned())
    ); // every micro group kept
}

#[test]
fn test_micro_versioned_shebang_lands_in_stored_pep723() {
    // `#!/usr/bin/env python3.12.1` records requires-python `>=3.12.1,<3.13` in the stored
    // copy's PEP 723 block AND announces the pin (a value recorded on a no-ask path is said aloud).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("mv.py");
    fs::write(&src, "#!/usr/bin/env python3.12.1\nprint(1)\n").unwrap();
    let output = sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "mv", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        flat(&output).contains("requires-python >=3.12.1,<3.13"),
        "{}",
        flat(&output)
    ); // announced
    let stored = sandbox.stored("mv");
    assert!(
        stored.contains("requires-python = \">=3.12.1,<3.13\""),
        "{stored}"
    ); // and landed in the block
}

// ==========================================================================
// 5. The unknown-kind refusal is shebang-aware
// ==========================================================================

#[test]
fn test_shebangless_unknown_uses_the_isnt_a_script_voice() {
    // A shebang-LESS unknown file keeps the original 'isn't a script or an executable' message
    // (the registered-shebang complement has its own test).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("mystery");
    fs::write(&src, "just some text, no shebang\n").unwrap();
    let output = sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "mys", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    let flat = flat(&output);
    assert!(flat.contains("isn't a script or an executable"), "{flat}");
    assert!(!flat.contains("names no interpreter"), "{flat}"); // the shebang voice is not used here
}

#[test]
fn test_shebang_unknown_uses_the_names_no_interpreter_voice() {
    // A file WITH an unregistered #! (awk) gets the shebang-aware 'names no interpreter' voice
    // even OUTSIDE drafts (path lane, not just resume), naming --kind.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("report.tricky");
    fs::write(&src, "#!/usr/bin/awk -f\nBEGIN{print 1}\n").unwrap();
    let output = sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "rep", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    let flat = flat(&output);
    assert!(
        flat.contains("The #! in report.tricky names no interpreter skit knows"),
        "{flat}"
    );
    assert!(flat.contains("--kind"), "{flat}");
    assert!(!flat.contains("isn't a script or an executable"), "{flat}");
}

// ==========================================================================
// 6. The extra-arguments field is named exactly once (argv hint yields to reader notice)
//    (The $0 self-location warning rewrite is pinned in test_shell_inject.py.)
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE STUB (kind=cross-crate): cli._print_add_hints is the crate-private print_copy_onboarding_facts (crates/skit-cli/src/cli.rs:2669-2694) in the skit-cli BINARY, unreachable from an integration test. Its argv-yields-to-framework gate IS implemented (`if plan.uses_argv && !plan.uses_cli_framework()`, cli.rs:2677) and IS covered end-to-end by test_dynamic_optstring_with_argv_names_extra_arguments_once in this same file — a reachability gap, not a behavior gap."]
fn test_add_hints_suppresses_argv_when_a_framework_was_detected() {
    // _print_add_hints yields the argv line when a framework was detected (frameworks -> the
    // uses_cli_framework property) — the reader notice already named the extra-arguments field;
    // the same fact twice reads as two facts.
    // Python:
    //   cli._print_add_hints(analysis.Analysis(uses_argv=True, frameworks=["argparse"]), "tool")
    //   assert _flat(capsys.readouterr().out) == ""   # nothing printed: the argv hint yielded
}

#[test]
#[ignore = "CROSS-CRATE STUB (kind=cross-crate): cli._print_add_hints is the crate-private print_copy_onboarding_facts (crates/skit-cli/src/cli.rs:2669-2694) in the skit-cli BINARY, unreachable from an integration test. The no-framework argv line IS implemented (cli.rs:2677-2680) and its end-to-end shape is exercised by the sibling port_test_add_lane_contracts.rs — a reachability gap, not a behavior gap."]
fn test_add_hints_prints_argv_when_no_framework() {
    // The complement (unchanged branch): no framework -> the argv hint DOES print.
    // Python:
    //   cli._print_add_hints(analysis.Analysis(uses_argv=True, frameworks=[]), "tool")
    //   assert "reads command-line arguments" in _flat(capsys.readouterr().out)
}

#[test]
fn test_dynamic_optstring_with_argv_names_extra_arguments_once() {
    // End-to-end (CLI add): a dynamic-optstring shell that ALSO reads $@ mentions the
    // extra-arguments field exactly ONCE — the reader notice, not doubled by the argv hint.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("dyn.sh");
    fs::write(
        &src,
        "#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho \"$@\"\n",
    )
    .unwrap();
    let output = sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "dyn", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert_eq!(flat(&output).matches("extra-arguments field").count(), 1);
}

// ==========================================================================
// 7. A sibling local module is not recorded as a PyPI dependency
// ==========================================================================

#[test]
fn test_add_records_only_third_party_deps_not_sibling_modules() {
    // End-to-end (CLI add --no-input): a script importing a SIBLING module (helpers.py next to
    // it) and a genuine third-party package records only the third-party one in the stored copy's
    // PEP 723 block — the add passes script_dir=source.parent, so `helpers` (which would resolve
    // to the local file at run time) is filtered out rather than installed as an unrelated
    // PyPI `helpers`.
    let sandbox = Sandbox::new();
    fs::write(
        sandbox.scratch.path().join("helpers.py"),
        "def go():\n    return 1\n",
    )
    .unwrap();
    let script = sandbox.scratch.path().join("job.py");
    fs::write(
        &script,
        "import helpers\nimport requests\nprint(helpers.go(), requests)\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", script.to_str().unwrap(), "-n", "job", "--no-input"])
        .assert()
        .success();
    let stored = sandbox.stored("job");
    let meta = read_uv_metadata(&stored).expect("metadata block present");
    assert_eq!(meta.dependencies, ["requests"]); // helpers excluded as a local sibling
}

#[test]
fn test_resolve_python_metadata_without_script_dir_does_not_filter() {
    // Called WITHOUT script_dir (the default None), `_resolve_python_metadata` has no directory
    // to resolve siblings against, so nothing is treated as local — pinning the default parameter
    // and the contract for any caller that omits it.
    let text = "import helpers\nimport requests\n";
    let deps = external_dependencies_at("python", text, None);
    assert_eq!(deps, ["helpers", "requests"]); // unfiltered: both survive

    // The oracle's `py == ""` half. `_resolve_python_metadata` derives requires-python ONLY
    // from a versioned `#!` (`python_version_pin(shebang_program_from_line(first_line))`,
    // cli.py:200-206); with no shebang it returns "". No single public resolver exists in the
    // Rust workspace (only these parts), so verify the contract end-to-end: `skit add` the same
    // shebang-less python and the stored copy carries NO requires-python pin. (The stored copy
    // still gets a PEP 723 block for the deps — the assertion is on the key, not the block.)
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("nopin.py");
    fs::write(&src, text).unwrap();
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "nopin", "--no-input"])
        .assert()
        .success();
    let stored = sandbox.stored("nopin");
    assert!(!stored.contains("requires-python"), "{stored}"); // no shebang -> nothing to pin
    // ...and the reachable twin of the oracle's pin step (the add path's own
    // `shebang_program` -> `python_version_pin`, cli.rs mapping) yields nothing on this text.
    let first_line = text.split('\n').next().unwrap_or("");
    assert_eq!(
        shebang_program(first_line).and_then(python_version_pin),
        None
    );
}
