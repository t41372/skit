//! Mechanical port of the Python oracle module `tests/test_add_lane_contracts.py`
//! (`origin/main@206f9ef`): "Add-lane contracts — real-behavior coverage (exit codes,
//! stored meta, filesystem)." Each `#[test]` keeps its Python `def test_*` name and its
//! WHY comment, so it traces back to its origin.
//!
//! Every oracle test drives the CLI end-to-end through `typer.testing.CliRunner`. This port
//! drives the real `skit` binary via `assert_cmd` inside a fresh four-directory sandbox
//! (`SKIT_DATA_DIR`/`SKIT_STATE_DIR`/`SKIT_CONFIG_DIR` + a scratch dir for sources and
//! sentinel editors), so skit writes only inside the temp sandbox.
//!
//! Concept mapping:
//! - Python `runner.invoke(cli.app, ["add", …], input=…)` -> `Sandbox::command().args(…)
//!   .write_stdin(…)`.
//! - Python `result.output` (CliRunner merges stdout+stderr) -> `combined(&output)` (Rust
//!   refusals go to stderr, notices/summaries to stdout — the merged view matches the oracle).
//!   `_flat` (collapse rich soft-wrap) -> `flat(&output)`.
//! - Python `store.resolve(name).meta` / `store.resolve` raising `NotFoundError` ->
//!   `Sandbox::show_json(name)` / `Sandbox::entry_exists(name)` (`skit show NAME --json`).
//! - Python `entry.meta.params` for a prompt -> `params --json` `"placeholders"`.
//! - Python `store.list_entries()` -> `skit list --json` (an empty array == nothing landed).
//! - Python `store.add_python(path, name=…)` -> `skit add PATH -n NAME --no-input`.
//! - Python `drafts_dir().glob("skit-*")` -> `drafts(&sandbox)` over `<data>/drafts`.
//! - Python `monkeypatch.setattr(cli.editor, "open_in_editor", …)` -> a real sentinel editor
//!   script on `$EDITOR`/`$VISUAL`: a `writer_editor` (copies fixture bytes into the draft) or
//!   a `boom_editor` (touches a marker; the test asserts the marker is absent). The Rust editor
//!   lane (`add_draft`, cli.rs:1399) does NOT gate on an interactive terminal, so these lanes
//!   are reachable under `assert_cmd`.
//! - Python `monkeypatch.setattr(cli, "_is_interactive", lambda: True)` has NO analogue:
//!   `assert_cmd` is never a terminal. Where a test's contract depends on the forced-terminal
//!   branch, the divergence note records the actual non-tty Rust behavior.
//!
//! Bucket disposition (all 21 defs drive the binary and COMPILE; zero absent/cross-crate stubs):
//! - 14 PASS asserting tests: the 5 versioned/piped/reader-notice lanes, both editor-lane
//!   `--description` threads, the versioned-shebang editor lane, the normal-file no-unlink lane,
//!   the JSON-is-one-document flip lane, both parameter read views, and both unknown-runner
//!   early-refusal lanes.
//! - 7 FAILING CONTRACT (divergence) tests: full asserting bodies kept intact behind
//!   `#[ignore]`; each label was verified against the built binary. Most tie to pending tasks
//!   #15 (refuse the add-lane inputs v0.4 refuses) and #16 (params batch fault tolerance). The
//!   recurring shapes are: no one-voice selector-collision refusal (clap `conflicts_with`
//!   answers first with a different message), pipe-spelling and dependency-refusal wording, no
//!   resumed-draft cleanup / kept-draft `--ref` guard on the plain path lane, and no flip note.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use serde_json::Value;
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

    /// The `skit params NAME --json` document (its `"unmanaged"` / `"placeholders"` fields).
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

/// Python `store.list_entries()` via `skit list --json` — the array of stored entries.
fn list_entries(sandbox: &Sandbox) -> Vec<Value> {
    let output = sandbox.command().args(["list", "--json"]).output().unwrap();
    assert!(
        output.status.success(),
        "list --json failed: {}",
        combined(&output)
    );
    serde_json::from_slice::<Value>(&output.stdout)
        .expect("stdout is exactly one JSON document")
        .as_array()
        .expect("list --json is an array")
        .clone()
}

/// Python `drafts_dir().glob("skit-*")` — the files under skit's OWN drafts home.
fn drafts(sandbox: &Sandbox) -> Vec<PathBuf> {
    let dir = sandbox.data.path().join("drafts");
    match fs::read_dir(&dir) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("skit-"))
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Sentinel `$EDITOR` that copies `content` into the draft skit opens (`$1`) — the oracle's
/// `_editor_writes` monkeypatch. Bytes go through a payload file so no shell quoting is needed.
#[cfg(unix)]
fn writer_editor(dir: &Path, content: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let payload = dir.join("editor-payload");
    fs::write(&payload, content).unwrap();
    let script = dir.join("writer-editor.sh");
    fs::write(&script, format!("#!/bin/sh\ncat {payload:?} > \"$1\"\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// Sentinel `$EDITOR` that touches `marker` when launched — the oracle's `_boom_editor`. A test
/// that must not open the editor asserts `marker` never appears.
#[cfg(unix)]
fn boom_editor(dir: &Path, marker: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = dir.join("boom-editor.sh");
    fs::write(&script, format!("#!/bin/sh\ntouch {marker:?}\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

// ==========================================================================
// 1. Lane selectors are mutually exclusive
// ==========================================================================

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle refuses colliding lane SELECTORS with one voice BEFORE dispatch (src/skit/cli.py:1603-1612, '%(flags)s each pick a different way to add — use exactly one'). Rust makes --edit/--cmd clap-`conflicts_with` `source` (cli.rs:302,311), so a collision is a clap 'the argument … cannot be used with …' error — same exit 2, but the one-voice message and the named selectors ('a file path'/'--edit') never appear. Ties to pending task #15. Verified against the built binary."]
fn test_selector_collisions_are_refused_one_voice() {
    // --cmd / --edit / stdin('-') / a file path each pick a DIFFERENT add lane; any pair is
    // a usage error with the single 'each pick a different way to add' voice, BEFORE the flag
    // matrix or any editor. Nothing is added and skit's drafts home is never touched.
    let sandbox = Sandbox::new();
    // The editor must never open for the --edit collisions (the refusal precedes dispatch).
    let marker = sandbox.scratch.path().join("editor-ran");
    let editor = boom_editor(sandbox.scratch.path(), &marker);
    let real = sandbox.scratch.path().join("real.py");
    fs::write(&real, "print(1)\n").unwrap();
    let real = real.display().to_string();
    let cases: [(Vec<String>, &str); 4] = [
        (
            vec![
                "add".into(),
                real.clone(),
                "--cmd".into(),
                "echo {x}".into(),
            ],
            "a file path",
        ),
        (
            vec!["add".into(), "-".into(), "--cmd".into(), "echo {x}".into()],
            "stdin ('-')",
        ),
        (vec!["add".into(), "--edit".into(), real.clone()], "--edit"),
        (vec!["add".into(), "--edit".into(), "-".into()], "--edit"),
    ];
    for (argv, needle) in cases {
        let output = sandbox
            .command()
            .env("EDITOR", &editor)
            .env("VISUAL", &editor)
            .args(&argv)
            .write_stdin("print(1)\n")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(2),
            "{argv:?}: {}",
            combined(&output)
        );
        let flat = flat(&output);
        assert!(
            flat.contains("each pick a different way to add"),
            "{argv:?}: {flat}"
        );
        assert!(flat.contains(needle), "{argv:?}: {flat}"); // the colliding selectors are named
        assert!(list_entries(&sandbox).is_empty(), "{argv:?}"); // nothing landed
        assert!(drafts(&sandbox).is_empty(), "{argv:?}"); // drafts home untouched
        assert!(!marker.exists(), "the editor stayed shut: {argv:?}");
    }
}

// ==========================================================================
// 2. Versioned python shebang is the registry's rule on every lane
// ==========================================================================

#[test]
fn test_stdin_versioned_python_shebang_lands_as_python() {
    // `#!/usr/bin/env python3.12` piped in with no --kind is a python entry — the stdin lane
    // reads the shebang through the same registry rule as the path/editor lanes.
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "-", "-n", "v"])
        .write_stdin("#!/usr/bin/env python3.12\nprint(1)\n")
        .assert()
        .success();
    assert_eq!(sandbox.show_json("v")["kind"], "python");
}

#[cfg(unix)]
#[test]
fn test_editor_lane_versioned_python_shebang_onboards_as_python() {
    // A draft whose shebang names python3.12 is onboarded as python (not refused as an
    // unregistered interpreter) — the versioned rule reaches the editor lane too.
    let sandbox = Sandbox::new();
    let editor = writer_editor(
        sandbox.scratch.path(),
        "#!/usr/bin/env python3.12\nprint('hi')\n",
    );
    sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "-e", "-n", "vpy"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("vpy")["kind"], "python");
}

// ==========================================================================
// 3. --runner is validated before any editor opens or a draft materializes
// ==========================================================================

#[test]
fn test_stdin_prompt_bogus_runner_refused_before_any_draft() {
    // A bogus --runner on the stdin prompt lane exits 2 with 'Unknown runner' and
    // materializes NO draft (the old code left a silent, anonymous file behind).
    let sandbox = Sandbox::new();
    let output = sandbox
        .command()
        .args(["add", "-", "--prompt", "--runner", "bogus", "-n", "p"])
        .write_stdin("x {{u}}\n")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("Unknown runner"),
        "{}",
        combined(&output)
    );
    assert!(list_entries(&sandbox).is_empty());
    assert!(drafts(&sandbox).is_empty()); // nothing was written to drafts/ before the refusal
}

#[cfg(unix)]
#[test]
fn test_prompt_editor_bogus_runner_refused_before_the_editor() {
    // --runner names static config, so the TTY prompt-editor lane refuses it BEFORE opening
    // $EDITOR — the editor is never launched (the same before-authoring rule as name conflicts).
    let sandbox = Sandbox::new();
    let marker = sandbox.scratch.path().join("editor-ran");
    let editor = boom_editor(sandbox.scratch.path(), &marker);
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "--prompt", "--runner", "bogus", "-n", "p"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("Unknown runner"),
        "{}",
        combined(&output)
    );
    assert!(!marker.exists()); // editor stayed shut
    assert!(drafts(&sandbox).is_empty());
}

// ==========================================================================
// 4. --no-input on the editor lanes
// ==========================================================================

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the no_input-before-editor ORDERING holds (cli.rs:1033 fires first — exit 2, editor stays shut), but the pipe spelling is 'skit add - --name NAME' where the oracle points at 'skit add - -n NAME' (src/skit/cli.py:770). Verified against the built binary."]
fn test_edit_no_input_is_refused_with_the_pipe_spelling() {
    // --edit opens an editor — interaction — so --no-input can't keep the never-prompt
    // promise: it is refused up front, pointing at the stdin spelling. (The oracle forces
    // interactive True to prove the no_input check fires first, not the interactivity gate;
    // Rust's check is unconditional.)
    let sandbox = Sandbox::new();
    let marker = sandbox.scratch.path().join("editor-ran");
    let editor = boom_editor(sandbox.scratch.path(), &marker);
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "-e", "-n", "x", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("skit add - -n NAME"),
        "{}",
        combined(&output)
    ); // the pipe spelling
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle forces an interactive terminal (monkeypatched `_is_interactive`), where `--prompt --no-input` opens an editor no keyboard-stdin can feed and is refused with 'skit add - --prompt -n NAME' (src/skit/cli.py:1216). assert_cmd is never a terminal, so Rust takes the pipe branch (cli.rs:1048-1051), reads (empty) stdin, and ADDS the prompt — exit 0, no refusal. Unreproducible without a PTY, and the spelling differs even then ('--name' vs '-n'). Verified against the built binary."]
fn test_prompt_editor_no_input_in_a_terminal_is_refused() {
    // --prompt with no path in a terminal opens an editor; --no-input there is refused with
    // the prompt pipe spelling — no body can arrive from a keyboard-attached stdin.
    let sandbox = Sandbox::new();
    let marker = sandbox.scratch.path().join("editor-ran");
    let editor = boom_editor(sandbox.scratch.path(), &marker);
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "--prompt", "-n", "p", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2), "{}", combined(&output));
    assert!(
        combined(&output).contains("skit add - --prompt -n NAME"),
        "{}",
        combined(&output)
    );
    assert!(!marker.exists());
}

#[test]
fn test_prompt_no_input_piped_still_adds() {
    // The documented non-interactive route: under a pipe there is no editor, so --prompt
    // --no-input reads the body from stdin and adds — this must keep working.
    let sandbox = Sandbox::new();
    sandbox
        .command()
        .args(["add", "--prompt", "-n", "pp", "--no-input"])
        .write_stdin("Summarize {{url}}\n")
        .assert()
        .success();
    assert_eq!(sandbox.show_json("pp")["kind"], "prompt");
    // Python `entry.meta.params == ["url"]`; the prompt placeholder list is `params --json`
    // `"placeholders"`.
    assert_eq!(
        sandbox.params_json("pp")["placeholders"],
        serde_json::json!(["url"])
    );
}

// ==========================================================================
// 5. --description threads into the editor-lane stored entry
// ==========================================================================

#[cfg(unix)]
#[test]
fn test_edit_description_flag_wins_over_python_docstring() {
    // A python draft with a docstring: --description is stored verbatim, not the docstring
    // (the flag threads through the editor lane to the python add).
    let sandbox = Sandbox::new();
    let editor = writer_editor(
        sandbox.scratch.path(),
        "\"\"\"Docstring one\"\"\"\nprint(1)\n",
    );
    sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "-e", "-n", "dpy", "--description", "flag wins"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("dpy")["description"], "flag wins");
}

#[cfg(unix)]
#[test]
fn test_edit_description_flag_on_non_python_draft_is_stored() {
    // A bash-shebang draft records --description too (the drafted-kind add now threads it) —
    // the description is not a python-only field.
    let sandbox = Sandbox::new();
    let editor = writer_editor(sandbox.scratch.path(), "#!/usr/bin/env bash\necho hi\n");
    sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "-e", "-n", "dsh", "--description", "shell note"])
        .assert()
        .success();
    let entry = sandbox.show_json("dsh");
    assert_eq!(entry["kind"], "shell");
    assert_eq!(entry["description"], "shell note");
}

// ==========================================================================
// 6. A post-editor refusal keeps the draft AND says so (short form)
// ==========================================================================

#[cfg(unix)]
#[test]
#[ignore = "FAILING CONTRACT (divergence): the keep-and-announce-SHORT behavior holds (post-editor --dep refusal exits 2, prints 'Your draft was kept at', omits the long 'fix the problem and add it with' form, keeps the draft, adds nothing), but the refusal reads 'shell entries do not take package dependencies' where the oracle says '--dep/--python are python flags …' (src/skit/cli.py:740-742). Verified against the built binary."]
fn test_edit_post_editor_refusal_keeps_draft_and_announces_short() {
    // --dep against a non-python draft is refused post-editor. The draft is the user's only
    // copy: it is kept on disk AND the SHORT 'kept at' line is printed (not the long
    // 'fix the problem and add it with' resumable form — this usage refusal names its own fix).
    let sandbox = Sandbox::new();
    let editor = writer_editor(
        sandbox.scratch.path(),
        "#!/usr/bin/env bash\necho drafted\n",
    );
    let output = sandbox
        .command()
        .env("EDITOR", &editor)
        .env("VISUAL", &editor)
        .args(["add", "-e", "-n", "d", "--dep", "foo"])
        .output()
        .unwrap();
    let combined = combined(&output);
    assert_eq!(output.status.code(), Some(2), "{combined}");
    assert!(combined.contains("python flags"), "{combined}"); // the --dep refusal
    assert!(combined.contains("Your draft was kept at"), "{combined}"); // the kept announcement…
    assert!(
        !combined.contains("fix the problem and add it with"),
        "{combined}"
    ); // …in its SHORT form
    assert_eq!(drafts(&sandbox).len(), 1, "the draft survived the refusal");
    assert!(!sandbox.entry_exists("d")); // nothing added
}

// ==========================================================================
// 8. Resume cleanup on the CLI path lane
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle's path lane consumes a resumed draft (a file under skit's OWN drafts home) on a successful copy. Rust's plain path lane never calls remove_owned_draft (only cli.rs:1353/5633/5659 do, all off the plain path) — the copy succeeds (mode copy) but the source draft SURVIVES. Verified against the built binary."]
fn test_path_add_of_a_drafts_home_file_unlinks_it_on_copy() {
    // A resumed draft (a file living in skit's OWN drafts home) added in copy mode reaches
    // the store, then the source is unlinked — the same 'the store holds the copy' cleanup the
    // authoring lanes do. Only files under drafts home; a user's original is never touched.
    let sandbox = Sandbox::new();
    let drafts_dir = sandbox.data.path().join("drafts");
    fs::create_dir_all(&drafts_dir).unwrap();
    let draft = drafts_dir.join("skit-new-resumeme.py");
    fs::write(&draft, "print('resume')\n").unwrap();
    sandbox
        .command()
        .args(["add", draft.to_str().unwrap(), "-n", "res", "--no-input"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("res")["mode"], "copy");
    assert!(!draft.exists()); // the resumed draft was cleaned up
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle refuses --ref against its OWN kept draft (a reference into drafts/ would list a live entry's file as a resumable/deletable draft) with exit 2 '… one of skit's own kept drafts … Drop --ref.' (src/skit/cli.py:1917-1933). Rust has no kept-draft guard on the path lane — it ADDS the reference (exit 0) and the draft remains. Ties to pending task #15. Verified against the built binary."]
fn test_path_add_of_a_drafts_home_file_refuses_reference() {
    // --ref on skit's OWN kept draft is refused: a reference entry pointing into drafts/ would
    // leave a live entry's file listed as a resumable draft — offered for re-adding and for
    // deletion as "the only copy", both lies. Exit 2, the draft is kept, no entry is created.
    let sandbox = Sandbox::new();
    let drafts_dir = sandbox.data.path().join("drafts");
    fs::create_dir_all(&drafts_dir).unwrap();
    let draft = drafts_dir.join("skit-new-keepme.py");
    fs::write(&draft, "print('keep')\n").unwrap();
    let output = sandbox
        .command()
        .args([
            "add",
            draft.to_str().unwrap(),
            "-n",
            "kep",
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
    assert!(!sandbox.entry_exists("kep"));
}

#[test]
fn test_path_add_of_a_normal_file_never_unlinks_the_original() {
    // The cleanup is scoped to drafts home: a normal user file added in copy mode is left
    // exactly where it was (skit copies it into the store, never moves it).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("mine.py");
    fs::write(&src, "print('mine')\n").unwrap();
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "mine", "--no-input"])
        .assert()
        .success();
    assert_eq!(sandbox.show_json("mine")["mode"], "copy");
    assert!(src.exists()); // the user's original is untouched
}

// ==========================================================================
// 9. The reader notice is one voice for every add lane
// ==========================================================================

#[test]
fn test_shell_getopts_add_prints_the_read_notice() {
    // A shell script whose getopts optstring skit CAN model statically prints the same
    // '✓ skit read this script's own arguments' notice the python lane does.
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch.path().join("flags.sh");
    fs::write(
        &sh,
        "#!/usr/bin/env bash\nwhile getopts \"n:v\" opt; do :; done\n",
    )
    .unwrap();
    let output = sandbox
        .command()
        .args(["add", sh.to_str().unwrap(), "-n", "flags", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        combined(&output).contains("skit read this script's own arguments"),
        "{}",
        combined(&output)
    );
}

#[test]
fn test_shell_dynamic_getopts_add_prints_the_passthrough_notice() {
    // A DYNAMIC optstring is detected but unmodelable: the honest passthrough variant fires
    // and names the framework (getopts) — not silence, not a false 'read your form'.
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch.path().join("dyn.sh");
    fs::write(
        &sh,
        "#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" opt; do :; done\n",
    )
    .unwrap();
    let output = sandbox
        .command()
        .args(["add", sh.to_str().unwrap(), "-n", "dyn", "--no-input"])
        .output()
        .unwrap();
    let combined = combined(&output);
    assert_eq!(output.status.code(), Some(0), "{combined}");
    assert!(combined.contains("parses its own arguments"), "{combined}");
    assert!(combined.contains("getopts"), "{combined}"); // the framework is named
}

#[test]
fn test_js_parseargs_add_prints_the_read_notice() {
    // The reader notice is not shell-only: a js entry with parseArgs surfaces it too.
    let sandbox = Sandbox::new();
    let js = sandbox.scratch.path().join("cli.js");
    fs::write(
        &js,
        "#!/usr/bin/env node\nimport { parseArgs } from 'node:util'\nconst { values } = parseArgs({ options: { name: { type: 'string' } } })\nconsole.log(values)\n",
    )
    .unwrap();
    let output = sandbox
        .command()
        .args(["add", js.to_str().unwrap(), "-n", "jscli", "--no-input"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    assert!(
        combined(&output).contains("skit read this script's own arguments"),
        "{}",
        combined(&output)
    );
}

// ==========================================================================
// 10. A python argparse read view no longer advertises --manage
// ==========================================================================

#[test]
fn test_params_python_argparse_read_view_is_plain() {
    // A python entry that parses its own arguments is reader-driven like every kind: its
    // parser IS the run form, so the read view says the plain 'no managed parameters.' and does
    // NOT advertise --manage; --json reports unmanaged == [] (no candidate offered).
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("ap.py");
    fs::write(
        &src,
        "import argparse\nOUT = 'hi'\np = argparse.ArgumentParser()\np.add_argument('--n')\np.parse_args()\nprint(OUT)\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "ap", "--no-input"])
        .assert()
        .success();
    let plain = sandbox.command().args(["params", "ap"]).output().unwrap();
    assert_eq!(plain.status.code(), Some(0), "{}", combined(&plain));
    assert!(
        combined(&plain).contains("has no managed parameters."),
        "{}",
        combined(&plain)
    );
    assert!(
        !combined(&plain).contains("--manage"),
        "{}",
        combined(&plain)
    ); // reader-driven: --manage would shadow argparse
    assert_eq!(
        sandbox.params_json("ap")["unmanaged"],
        serde_json::json!([])
    );
}

#[test]
fn test_params_python_constants_only_still_offers_manage() {
    // The gate is scoped to reader-driven entries: a constants-only python (no argparse) is
    // NOT reader-driven, so it keeps advertising --manage and lists the detected candidate.
    let sandbox = Sandbox::new();
    let src = sandbox.scratch.path().join("co.py");
    fs::write(&src, "OUT = 'hi'\nprint(OUT)\n").unwrap();
    sandbox
        .command()
        .args(["add", src.to_str().unwrap(), "-n", "co", "--no-input"])
        .assert()
        .success();
    let result = sandbox.command().args(["params", "co"]).output().unwrap();
    assert_eq!(result.status.code(), Some(0), "{}", combined(&result));
    assert!(
        combined(&result).contains("--manage"),
        "{}",
        combined(&result)
    ); // a bare-constant python still offers management
    assert_eq!(
        sandbox.params_json("co")["unmanaged"],
        serde_json::json!(["OUT"])
    );
}

// ==========================================================================
// 11. Flipping a reader-driven entry to managed params announces the trade-off
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle prints a one-time flip note when a reader-driven-ONLY entry first gets a managed const — 'The run form now asks for the managed parameters …' naming the set-aside reader form (getopts) (src/skit/cli.py:4575). Rust's params has no such note at all. Verified against the built binary."]
fn test_manage_flip_note_names_the_reader_form_then_stays_quiet() {
    // A getopts shell entry that ALSO holds a constant: the first `--manage CONST` prints the
    // flip note naming getopts (managed params REPLACE the reader form). A second --manage on the
    // now-managed entry does NOT reprint it (it was reader-driven only before the first flip).
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch.path().join("both.sh");
    fs::write(
        &sh,
        "#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts \"n:v\" opt; do :; done\necho $CITY\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", sh.to_str().unwrap(), "-n", "both", "--no-input"])
        .assert()
        .success();
    let first = sandbox
        .command()
        .args(["params", "both", "--manage", "CITY"])
        .output()
        .unwrap();
    assert_eq!(first.status.code(), Some(0), "{}", combined(&first));
    assert!(
        combined(&first).contains("The run form now asks for the managed parameters"),
        "{}",
        combined(&first)
    );
    assert!(combined(&first).contains("getopts"), "{}", combined(&first)); // the reader form set aside is named

    // A constant is already managed now → the entry is no longer reader-driven-only, so a
    // second manage prints no flip note.
    let sh2 = sandbox.scratch.path().join("second.sh");
    fs::write(
        &sh2,
        "#!/usr/bin/env bash\nCITY=Taipei\nPORT=8080\nwhile getopts \"n:v\" opt; do :; done\necho $CITY $PORT\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", sh2.to_str().unwrap(), "-n", "second", "--no-input"])
        .assert()
        .success();
    sandbox
        .command()
        .args(["params", "second", "--manage", "CITY"]) // first flip (has the note)
        .assert()
        .success();
    let again = sandbox
        .command()
        .args(["params", "second", "--manage", "PORT"])
        .output()
        .unwrap();
    assert_eq!(again.status.code(), Some(0), "{}", combined(&again));
    assert!(
        !combined(&again).contains("The run form now asks for the managed parameters"),
        "{}",
        combined(&again)
    );
}

#[test]
fn test_manage_flip_json_stdout_is_exactly_one_document() {
    // Under --json the flip note is silent (the maybe-quiet console) and stdout is EXACTLY one
    // JSON document — the note must never leak a human line onto the machine contract.
    let sandbox = Sandbox::new();
    let sh = sandbox.scratch.path().join("j.sh");
    fs::write(
        &sh,
        "#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts \"n:v\" opt; do :; done\necho $CITY\n",
    )
    .unwrap();
    sandbox
        .command()
        .args(["add", sh.to_str().unwrap(), "-n", "jflip", "--no-input"])
        .assert()
        .success();
    let output = sandbox
        .command()
        .args(["params", "jflip", "--manage", "CITY", "--json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", combined(&output));
    // parses whole stdout — one document, no leaked line
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document");
    let names = payload["params"]
        .as_array()
        .expect("params is an array")
        .iter()
        .map(|param| {
            param["name"]
                .as_str()
                .expect("a param name is a string")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert!(names.contains(&"CITY".to_owned()), "{names:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("The run form now asks"), "{stdout}");
}
