//! Mechanical port of the Python oracle module `tests/test_add_validation_contracts.py`
//! (`origin/main@206f9ef`): "Add validation contracts (exit codes, exact refusal copy,
//! filesystem state, stored PEP 723 text, the two lazy `packaging` validators in isolation)."
//! Each `#[test]` keeps its Python `def test_*` name and its "WHY" comment so it traces back
//! to the oracle.
//!
//! Concept mapping used throughout:
//! - Python `pep723.requires_python_error(v)` -> `skit_language::validate_pep440_specifiers(v)`
//!   (`None` for valid <-> `Ok(())`; an invalid value <-> `Err(PythonMetadataError)`). The Rust
//!   typed error carries a DIFFERENT message ("invalid PEP 440 version constraint ..."), so the
//!   message-exactness tests are divergences (see the file doc's bucket list).
//! - Python `pep723.requirement_error(v)` -> `skit_language::validate_pep508_requirement(v)`.
//! - Python `cli._validate_python_flags(deps, python)` has NO public Rust function. The `skit add -`
//!   (stdin) lane runs the same validate-then-normalize contract inside `add_with_config`
//!   (`crates/skit-cli/src/cli.rs`), so these are verified END-TO-END through the composition root.
//! - Python `cli._resolve_python_metadata(...)` interactive re-ask loop -> ABSENT (the CLI never
//!   prompts for dependencies; see the two stubs in section 3).
//! - Python `runner.invoke(cli.app, ...)` -> the real `skit` binary via `assert_cmd`, sandboxed by
//!   the three `SKIT_*` temp dirs.
//! - Python `registry.kind_for_draft(path)` has NO Rust function; a draft's kind is decided by the
//!   generic, extension-first `infer_kind` inside `add`, so section 6 observes the contract through
//!   `skit add` on a kept draft under `<SKIT_DATA_DIR>/drafts/`.
//! - Python `cli._create_python_in_editor(...)` -> the `skit add --edit` lane (`add_draft`).
//!
//! Bucket disposition (31 Python defs -> 31 `#[test]`):
//! - PASSING contract tests: sections 1 (valid + bare-version), 2 (most flag cases), 5 (dash/valid
//!   python), 6 (prompt + extensionless kind).
//! - DIVERGENCE (`#[ignore = "FAILING CONTRACT (divergence): ..."]`, full asserting body kept): the
//!   validator message text (1, 5); the case-sensitive `-`/`none` normalization and blank `--python`
//!   (2); the entire drafts boundary refusal, which is not implemented in the CLI add path (4);
//!   validate-before-editor (5 editor lane); the draft shebang-outranks-script-suffix rule and
//!   draft-consume-on-success (6); the unknown-shebang refusal copy (7).
//! - ABSENT gap stubs (`kind="absent"`): the interactive deps/python re-ask loop (3).

use std::fs;
use std::path::PathBuf;

use skit_i18n::{Locale, Localize};
use skit_language::{validate_pep440_specifiers, validate_pep508_requirement};
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
            .env("SKIT_LANG", "en");
        command
    }
}

/// Python `_flat`: collapse every run of whitespace to one space (stdout+stderr concatenated,
/// because a refusal prints to stderr and Python's `result.output` mixes both streams).
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Run `skit <args>` with optional piped stdin and return (exit code, flattened combined output).
fn run(sandbox: &Sandbox, args: &[&str], stdin: Option<&str>) -> (Option<i32>, String) {
    let mut command = sandbox.command();
    command.args(args);
    if let Some(text) = stdin {
        command.write_stdin(text.to_owned());
    }
    let output = command.output().expect("run skit");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code(), flat(&combined))
}

/// Python `_draft`: write a body into `<SKIT_DATA_DIR>/drafts/<name>` (the real drafts home).
fn draft(sandbox: &Sandbox, name: &str, body: &str) -> PathBuf {
    let dir = sandbox.data.path().join("drafts");
    fs::create_dir_all(&dir).expect("create drafts dir");
    let path = dir.join(name);
    fs::write(&path, body).expect("write draft");
    path
}

/// Python `_drafts_files() == []`: the drafts dir is absent or holds nothing.
fn drafts_dir_is_empty(sandbox: &Sandbox) -> bool {
    let dir = sandbox.data.path().join("drafts");
    !dir.exists()
        || fs::read_dir(&dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
}

fn entry_dir(sandbox: &Sandbox, slug: &str) -> PathBuf {
    sandbox.data.path().join("scripts").join(slug)
}

/// The stored `meta.toml` text for one slug, empty when the entry was never created.
fn read_meta(sandbox: &Sandbox, slug: &str) -> String {
    fs::read_to_string(entry_dir(sandbox, slug).join("meta.toml")).unwrap_or_default()
}

/// The stored python `script.py` text for one slug, empty when it was never written.
fn read_script(sandbox: &Sandbox, slug: &str) -> String {
    fs::read_to_string(entry_dir(sandbox, slug).join("script.py")).unwrap_or_default()
}

// ==========================================================================
// 1. The pep723 validators (lazy `packaging` imports), in isolation
// ==========================================================================

#[test]
fn test_requires_python_error_is_none_for_valid_constraints() {
    assert!(validate_pep440_specifiers(">=3.11").is_ok());
    assert!(validate_pep440_specifiers(">=3.12,<3.13").is_ok());
}

#[test]
fn test_requires_python_error_localizes_a_message_for_an_invalid_constraint() {
    let error = validate_pep440_specifiers("not-a-version").unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(
        message.starts_with("not-a-version isn't a Python version constraint"),
        "{message}"
    );
}

#[test]
fn test_requires_python_error_rejects_a_bare_version_without_operator() {
    // `3.11` (no comparison operator) is a real, common mistake — PEP 440 refuses it.
    assert!(validate_pep440_specifiers("3.11").is_err());
}

#[test]
fn test_requirement_error_is_none_for_valid_requirements() {
    assert!(validate_pep508_requirement("requests").is_ok());
    assert!(validate_pep508_requirement("rich>=13,<16").is_ok());
    assert!(validate_pep508_requirement("demo[bold]").is_ok()); // extras are valid PEP 508
}

#[test]
fn test_requirement_error_localizes_a_message_for_an_invalid_requirement() {
    let error = validate_pep508_requirement("@@@").unwrap_err();
    let message = error.message().localize(Locale::En);
    assert!(
        message.starts_with("@@@ isn't a package requirement"),
        "{message}"
    );
}

// ==========================================================================
// 2. _validate_python_flags — validate + '-'/'none' normalization
//
// No public Rust `_validate_python_flags`; the same validate-then-normalize contract runs inside
// the `skit add -` lane (`add_with_config`), so each case is observed end-to-end.
// ==========================================================================

#[test]
fn test_validate_python_flags_passes_valid_and_normalizes_the_constraint() {
    // Oracle: _validate_python_flags(["requests", "rich>=13,<16"], ">=3.11") == ">=3.11".
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &[
            "add",
            "-",
            "-n",
            "flagvalid",
            "--dep",
            "requests",
            "--dep",
            "rich>=13,<16",
            "--python",
            ">=3.11",
        ],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    let stored = read_script(&sandbox, "flagvalid");
    assert!(stored.contains("requires-python = \">=3.11\""), "{stored}");
    assert!(stored.contains("\"requests\""), "{stored}");
    assert!(stored.contains("\"rich>=13,<16\""), "{stored}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle normalizes '-'/'none' case-insensitively \
(cli.py:280 `cleaned.lower() in (\"-\", \"none\")`); Rust matches case-sensitively \
(cli.rs:2928 `matches!(value.trim(), \"-\" | \"none\")`), so '  NONE  ' is validated and exits 2."]
fn test_validate_python_flags_normalizes_dash_and_none_to_empty() {
    // Oracle: _validate_python_flags(None, "-") == "" and "none" == "" and "  NONE  " == "".
    for (value, name) in [
        ("-", "flagdash"),
        ("none", "flagnonelit"),
        ("  NONE  ", "flagupper"),
    ] {
        let sandbox = Sandbox::new();
        let (code, _out) = run(
            &sandbox,
            &["add", "-", "-n", name, "--python", value],
            Some("print(1)\n"),
        );
        assert_eq!(code, Some(0), "value {value:?}");
        assert!(
            !read_script(&sandbox, name).contains("requires-python"),
            "value {value:?} left a requires-python"
        );
    }
}

#[test]
fn test_validate_python_flags_returns_none_when_no_python_given() {
    // Oracle: _validate_python_flags(["requests"], None) is None (nothing to record).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagnone", "--dep", "requests"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    let stored = read_script(&sandbox, "flagnone");
    assert!(stored.contains("\"requests\""), "{stored}");
    assert!(!stored.contains("requires-python"), "{stored}");
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): oracle _validate_python_flags(None, \"   \") == \"\" \
(blank normalizes to automatic, cli.py:279-281); Rust keeps the blank and validates it \
(cli.rs:2927-2948 trims only for the '-'/'none' check, filters only the empty string)."]
fn test_validate_python_flags_treats_an_empty_python_as_empty() {
    // Oracle: _validate_python_flags(None, "   ") == "" (a blank constraint means automatic).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagempty", "--python", "   "],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    assert!(!read_script(&sandbox, "flagempty").contains("requires-python"));
}

#[test]
fn test_validate_python_flags_skips_empty_dep_strings() {
    // A whitespace-only --dep is dropped (not routed to the validator), matching the block-write.
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagskip", "--dep", "  "],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0));
    assert!(!read_script(&sandbox, "flagskip").contains("dependencies"));
}

#[test]
fn test_validate_python_flags_exits_2_on_a_bad_dep() {
    // Oracle: _validate_python_flags(["@@@"], None) raises typer.Exit(EXIT_USAGE).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagbaddep", "--dep", "@@@"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2));
}

#[test]
fn test_validate_python_flags_exits_2_on_a_bad_python() {
    // Oracle: _validate_python_flags(None, "not-a-version") raises typer.Exit(EXIT_USAGE).
    let sandbox = Sandbox::new();
    let (code, _out) = run(
        &sandbox,
        &["add", "-", "-n", "flagbadpy", "--python", "not-a-version"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2));
}

// ==========================================================================
// 3. The interactive deps / python asks are re-ask loops on invalid input
//
// ABSENT: the CLI never prompts for dependencies. The oracle drives cli._resolve_python_metadata
// with a monkeypatched cli.Prompt.ask (deps asked twice, python asked twice, '-' means none). The
// Rust CLI resolves dependencies non-interactively (external_dependencies_at, cli.rs:2916-2921);
// the nearest analog is the skit-ui add reducer's field validation (skit-ui/src/add.rs:1053-1177),
// which has no re-ask loop and no '-'/'none' token. No public surface carries this contract.
// ==========================================================================

#[test]
#[ignore = "ABSENT (gap): the interactive deps/python re-ask loop is not implemented. MUST-FIX: \
port cli.py:224-261 — an invalid deps answer re-asks (never stored), '-' means none, an invalid \
python constraint re-asks, a valid one is finally recorded (four asks total)."]
fn test_interactive_deps_reask_then_python_reask_then_accept() {
    // An invalid deps answer re-asks (never stored); '-' means none. An invalid python
    // constraint re-asks; a valid one is finally recorded. Four asks: deps twice, python twice.
}

#[test]
#[ignore = "ABSENT (gap): the interactive deps/python re-ask loop is not implemented. MUST-FIX: \
port cli.py:224-261 — a valid deps list is taken on the first ask, and '-' at the python ask \
means automatic (two asks, no re-ask)."]
fn test_interactive_valid_deps_accepted_first_try() {
    // The complement: a valid deps list is taken on the first ask (the inner validate loop
    // completes with bad=None), and '-' at the python ask means automatic.
}

// ==========================================================================
// 4. exe / reference can never cross the drafts boundary (every face)
//
// The CLI add path (`add_with_config`) never checks `is_draft`, so none of these refusals fire and
// none of the drafts are kept — every case is a divergence against the oracle's boundary guard
// (cli.py:1894-1933). MUST-FIX tracked as "Refuse the add-lane inputs version 0.4 refuses".
// ==========================================================================

const DRAFT_HEAD: &str = "one of skit's own kept drafts";

#[test]
#[ignore = "FAILING CONTRACT (divergence): the drafts boundary guard is absent — `add` has no \
is_draft check (cli.rs:2857-2905), so --exe on a kept draft is accepted (exit 0) instead of \
refused with 'Drop --exe.' (oracle cli.py:1894-1933)."]
fn test_exe_flag_on_a_kept_draft_is_refused_naming_only_exe() {
    // --exe alone → the refusal tells the user to drop --exe and NOTHING else: naming a flag
    // the user never passed (--ref) would be its own small lie. The honest-naming rule is the
    // point, so the other flag names must be absent.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-prog.py", "print('run me')\n");
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "p1",
            "--exe",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains(DRAFT_HEAD), "{out}");
    assert!(out.contains("Drop --exe."), "{out}");
    assert!(!out.contains("--ref"), "{out}"); // never passed — never named
    assert!(!out.contains("--kind"), "{out}");
    assert!(source.exists()); // a refused add consumes nothing
    assert!(!entry_dir(&sandbox, "p1").exists());
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the drafts boundary guard is absent — --kind exe on a \
kept draft is accepted (exit 0) instead of refused with 'Drop --kind exe.' (oracle \
cli.py:1894-1933)."]
fn test_kind_exe_on_a_kept_draft_is_refused_naming_only_kind_exe() {
    // --kind exe alone → the refusal names only "--kind exe"; --ref and --exe (neither passed)
    // stay out of the message.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-prog2.py", "print('run me')\n");
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "p2",
            "--kind",
            "exe",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("Drop --kind exe."), "{out}");
    assert!(!out.contains("--ref"), "{out}"); // never passed — never named
    assert!(!out.contains("--exe"), "{out}"); // "--kind exe" is not the "--exe" flag literal
    assert!(source.exists());
    assert!(!entry_dir(&sandbox, "p2").exists());
}

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the drafts boundary guard is absent — an inferred +x \
exe on a kept draft is accepted (exit 0) instead of refused with '...pass --kind <language> to \
name its language.' (oracle cli.py:1894-1933, 1925-1930)."]
fn test_inferred_exe_on_a_kept_draft_is_refused_and_keeps_it() {
    use std::os::unix::fs::PermissionsExt as _;
    // A hand-planted +x bit on an extensionless draft INFERS exe — the widened guard covers the
    // inferred route just like the explicit flags. The INFERRED route (the user passed no flag)
    // gets the --kind message, not the Drop-flags one: there is no flag to drop, so it points at
    // the escape a draft can actually take.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-binish", "opaque program bytes\n");
    let mut permissions = fs::metadata(&source).unwrap().permissions();
    permissions.set_mode(0o755); // POSIX infer_kind classifies +x as exe
    fs::set_permissions(&source, permissions).unwrap();
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "b1", "--no-input"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains(DRAFT_HEAD), "{out}"); // still names the drafts boundary
    assert!(
        out.contains("pass --kind <language> to name its language"),
        "{out}"
    );
    assert!(!out.contains("Drop"), "{out}"); // NOT the flag-route message
    assert!(source.exists());
    assert!(!entry_dir(&sandbox, "b1").exists());
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the drafts boundary guard is absent — --ref on a kept \
draft is accepted (exit 0) instead of refused with 'Drop --ref.' (oracle cli.py:1894-1933)."]
fn test_ref_flag_on_a_kept_draft_is_refused_naming_only_ref() {
    // --ref alone keeps refusing, naming ONLY --ref — --exe (never passed) stays out of the
    // message.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-linkme.py", "print('link me')\n");
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "lk",
            "--ref",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains(DRAFT_HEAD), "{out}");
    assert!(out.contains("Drop --ref."), "{out}");
    assert!(!out.contains("--exe"), "{out}"); // never passed — never named
    assert!(!out.contains("--kind"), "{out}");
    assert!(source.exists());
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a resumed draft is not consumed on success — the path \
add lane copies bytes but never removes the source draft (cli.rs:2857-2905 has no is_draft/consume \
step), so the draft still exists (oracle cli.py:258-266)."]
fn test_a_normal_draft_resume_still_adds_as_a_copy() {
    // The complement: a draft added with NO exe/ref flag resumes normally (copy, consumed on
    // success) — the guard fires only for the two forbidden shapes.
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-ok.py", "print('ok')\n");
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "okentry",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    // Oracle: `store.resolve("okentry").meta.mode == "copy"`. The reachable Rust equivalent is the
    // stored `meta.toml`, whose `mode` field serializes StorageMode::Copy as the lowercase "copy".
    assert!(read_meta(&sandbox, "okentry").contains("mode = \"copy\""));
    assert!(!source.exists()); // consumed on success
}

// ==========================================================================
// 5. --dep / --python validated BEFORE the pipe is read or a draft materializes
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 holds and the drafts dir stays empty, but the \
refusal reads 'invalid PEP 440 version constraint ...' not the oracle's 'isn't a Python version \
constraint' (skit-language/src/lib.rs:194)."]
fn test_stdin_garbage_python_exits_2_and_leaves_the_drafts_dir_empty() {
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "x", "--python", "garbage"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("isn't a Python version constraint"), "{out}");
    assert!(drafts_dir_is_empty(&sandbox)); // refused before mkstemp — no kept-draft fingerprint
    assert!(!entry_dir(&sandbox, "x").exists());
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): exit 2 holds and the drafts dir stays empty, but the \
refusal reads 'invalid PEP 508 requirement ...' not the oracle's 'isn't a package requirement' \
(skit-language/src/lib.rs:189)."]
fn test_stdin_garbage_dep_exits_2_and_leaves_the_drafts_dir_empty() {
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "y", "--dep", "@@@"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("isn't a package requirement"), "{out}");
    assert!(drafts_dir_is_empty(&sandbox));
}

#[test]
fn test_stdin_dash_python_is_automatic() {
    // '-' at --python means automatic: the add succeeds and the stored block carries no
    // requires-python.
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "auto", "--python", "-"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(!read_script(&sandbox, "auto").contains("requires-python"));
}

#[test]
fn test_stdin_valid_python_lands_in_the_stored_block() {
    let sandbox = Sandbox::new();
    let (code, out) = run(
        &sandbox,
        &["add", "-", "-n", "pinned", "--python", ">=3.11"],
        Some("print(1)\n"),
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_script(&sandbox, "pinned").contains("requires-python = \">=3.11\""));
}

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the editor lane validates AFTER the editor opens — \
add_draft (cli.rs:1399-1440) opens the editor, then add validates, and keeps the draft on failure, \
so the editor DID run and a draft WAS materialized (oracle cli.py:309-319 validates first)."]
fn test_editor_lane_refuses_bad_python_before_opening_the_editor() {
    // The editor lane validates BEFORE the editor opens (the name-conflict precedent): a bad
    // --python is refused and open_in_editor is never called (no authoring session cost).
    let sandbox = Sandbox::new();
    let marker = with_sentinel_editor(&sandbox);
    let (code, out) = run(
        &sandbox,
        &["add", "--edit", "-n", "edX", "--python", "garbage"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(!marker.exists(), "the editor never opened"); // opened == []
    assert!(drafts_dir_is_empty(&sandbox)); // no draft was materialized
}

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the editor lane validates AFTER the editor opens — \
add_draft (cli.rs:1399-1440) opens the editor, then add validates, so a bad --dep does not refuse \
before the editor runs (oracle cli.py:322-329 validates first)."]
fn test_editor_lane_refuses_bad_dep_before_opening_the_editor() {
    let sandbox = Sandbox::new();
    let marker = with_sentinel_editor(&sandbox);
    let (code, out) = run(
        &sandbox,
        &["add", "--edit", "-n", "edY", "--dep", "@@@"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(!marker.exists(), "the editor never opened");
}

/// Configure a fake editor that (a) touches a marker so a run is observable and (b) writes valid
/// python so a materialized draft is non-empty. Returns the marker path. The oracle asserts the
/// editor NEVER runs, so the marker must stay absent for the test to pass.
#[cfg(unix)]
fn with_sentinel_editor(sandbox: &Sandbox) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let marker = sandbox.config.path().join("editor-ran");
    let editor = sandbox.config.path().join("editor.sh");
    fs::write(
        &editor,
        format!(
            "#!/bin/sh\ntouch '{}'\nprintf 'print(1)\\n' > \"$1\"\n",
            marker.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&editor).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&editor, permissions).unwrap();
    fs::write(
        sandbox.config.path().join("config.toml"),
        format!("editor = {:?}\n", editor.display().to_string()),
    )
    .unwrap();
    marker
}

// ==========================================================================
// 6. kind_for_draft: the exception keys on the rationale (placeholder-bodied kinds)
//
// No Rust kind_for_draft; the draft's kind is decided by the extension-first `infer_kind` in the
// add path, observed here through the stored `meta.toml` kind.
// ==========================================================================

#[test]
fn test_kind_for_draft_single_prompt_extension_outranks_the_shebang() {
    // A `.prompt` (single-extension) draft whose body opens with a #! resumes as a PROMPT: the
    // exception is keyed on placeholder_params, not on compound-suffix shape.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-note.prompt",
        "#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "psingle",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "psingle").contains("kind = \"prompt\""));
}

#[test]
fn test_kind_for_draft_extensionless_falls_through_to_the_shebang() {
    // No registered extension at all → by_ext is None → the shebang decides (here: shell).
    let sandbox = Sandbox::new();
    let source = draft(&sandbox, "skit-new-plain", "#!/usr/bin/env bash\necho hi\n");
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "plain", "--no-input"],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "plain").contains("kind = \"shell\""));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the draft shebang-outranks-script-suffix rule is absent \
— `add` uses the generic extension-first infer_kind (cli.rs:2885), so a `.py` draft with a bash \
shebang stores kind = \"python\"; the oracle kind_for_draft (registry.py:442-473) makes the draft's \
shebang win, storing shell."]
fn test_kind_for_draft_script_suffix_stays_shebang_first() {
    // A `.py` script suffix is NOT placeholder-bodied, so the shebang still outranks it.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-shellish.py",
        "#!/usr/bin/env bash\necho drafted\n",
    );
    let (code, out) = run(
        &sandbox,
        &[
            "add",
            source.to_str().unwrap(),
            "-n",
            "shellish",
            "--no-input",
        ],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "shellish").contains("kind = \"shell\""));
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a resumed draft is not consumed on success — the kind is \
inferred as prompt correctly, but the path add lane never removes the source draft \
(cli.rs:2857-2905), so it still exists (oracle cli.py:356-363)."]
fn test_prompt_single_extension_draft_resumes_as_prompt_end_to_end() {
    // The CLI face of the single-extension prompt rule: the draft resumes as a prompt entry and
    // is consumed on success.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-summ.prompt",
        "#!/usr/bin/env bash\nSummarize {{text}}.\n",
    );
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "psumm", "--no-input"],
        None,
    );
    assert_eq!(code, Some(0), "{out}");
    assert!(read_meta(&sandbox, "psumm").contains("kind = \"prompt\""));
    assert!(!source.exists()); // consumed on success
}

// ==========================================================================
// 7. The unknown+shebang refusal: --exe for an on-disk file, --kind-only for a draft
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): an unknown shebang on an on-disk file falls through to \
'could not infer the entry kind; pass --kind KIND' (cli.rs:2896-2900); the oracle names the \
interpreter gap and offers '--exe to run it directly' (cli.py:2040-2052)."]
fn test_nondraft_awk_shebang_refusal_offers_the_exe_escape() {
    let sandbox = Sandbox::new();
    let file = sandbox.data.path().join("report.awkish");
    fs::write(&file, "#!/usr/bin/awk -f\nBEGIN { print 1 }\n").unwrap();
    let (code, out) = run(
        &sandbox,
        &["add", file.to_str().unwrap(), "-n", "rep", "--no-input"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("names no interpreter skit knows"), "{out}");
    assert!(out.contains("--exe to run it directly"), "{out}"); // an on-disk file gets the escape
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): a `.py` kept draft with an awk shebang is classified \
python by the extension-first infer_kind and added (exit 0); the oracle refuses it as unclassifiable \
and offers only '--kind <language> to choose one', never --exe (cli.py:2040-2052)."]
fn test_kept_draft_awk_shebang_refusal_offers_only_kind() {
    // The same awk shebang, but as a KEPT DRAFT: --exe is refused at the boundary, so the hint
    // must NOT offer it — only --kind.
    let sandbox = Sandbox::new();
    let source = draft(
        &sandbox,
        "skit-new-report.py",
        "#!/usr/bin/awk -f\nBEGIN { print 1 }\n",
    );
    let (code, out) = run(
        &sandbox,
        &["add", source.to_str().unwrap(), "-n", "repd", "--no-input"],
        None,
    );
    assert_eq!(code, Some(2), "{out}");
    assert!(out.contains("names no interpreter skit knows"), "{out}");
    assert!(out.contains("--kind <language> to choose one"), "{out}");
    assert!(!out.contains("--exe"), "{out}"); // the draft variant never offers the program escape
    assert!(source.exists());
}
