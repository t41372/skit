//! Mechanical port of the Python oracle module `tests/test_uv_metadata_unpinning.py`
//! (`origin/main@206f9ef`): "UV metadata unpinning and preservation contracts."
//!
//! The oracle drives the store and the Typer CLI end to end (CliRunner + `store.*`),
//! so this port belongs in the CLI crate (`skit-cli-rs`), not the crate hint
//! `skit-language`: every assertion here is an OBSERVABLE end-to-end contract, and only
//! the CLI binary reaches the `deps` / `run --dry-run` / `add` chokepoints the oracle
//! exercises. The tests run the real `skit` binary through `assert_cmd` and read the
//! stored files back from disk, the same style as `crates/skit-cli/tests/edge_workflows.rs`
//! and `v040_compatibility.rs`.
//!
//! Concept mapping used throughout:
//! - Python `store.add_python(src, name="a")` -> `skit add <src> --kind python --name a --no-input`.
//! - Python `store.add_python(src, name="a", requires_python=">=3.11")`
//!   -> `skit add <src> --kind python --name a --python ">=3.11" --no-input`.
//! - Python `deps a --python <c>` / `deps a --python -` -> the same CLI args (the literal
//!   `-` unpin token is kept; the Rust `deps` normalizes `-`/`none` -> "" at cli.rs:3454).
//! - Python `store.update_dependencies("a", [], requires_python=">=3.10")`
//!   -> `skit deps a --clear --python ">=3.10"` (`--clear` = explicit empty deps list).
//! - Python `deps a --dep requests` (deps-only, python UNTOUCHED) -> `skit deps a --dep requests`.
//! - Python `_stored_block(slug)` -> read `<SKIT_DATA_DIR>/scripts/<slug>/script.py`.
//! - Python `store.resolve(slug).meta.requires_python` -> read the raw meta.toml field. skit
//!   OMITS an empty string field, so `meta.requires_python == ">=3.13"` ports to
//!   `meta_text.contains("requires_python = \">=3.13\"")` and `meta.requires_python == ""`
//!   ports to `!meta_text.contains("requires_python =")` (same rule as v040_compatibility.rs).
//! - Python `_dry_run(slug)` (`run slug --dry-run --no-input`, exit 0) -> the CLI dry run,
//!   whose stdout is the launch command line (`plan.display`); the `--python <c>` flag comes
//!   from `settings.requires_python` (launch.rs:396) exactly like the oracle's `_argv_tail`.
//! - Python `deps a --json`["requires_python"] -> `skit deps a --json`["requires_python"]
//!   (the EFFECTIVE read view, same shape `{dependencies, requires_python, needs}`).
//!
//! Buckets:
//! - Bucket 1 (CLI + store integration): tests 1-3 below drive the real binary and assert
//!   the stored block, raw meta, dry-run command, and `--json` read view.
//! - Bucket 3 (cross-crate, TUI settings screen): `test_settings_clearing_python_unpins_the_block`
//!   is `#[ignore]`d. The oracle drives the interactive `ScriptSettingsScreen`
//!   (`tui.MenuApp` + pilot), a reducer/TestBackend surface owned by the skit-tui / skit-ui
//!   tier (AGENTS.md: "UI tests drive the reducer and Ratatui TestBackend"); a CLI-binary
//!   integration test cannot drive it, and hand-gluing the reducer to a FileStore would test
//!   wiring the production loop may not share. Test 1's `deps a --python -` already covers the
//!   same store chokepoint (unpinning the stored block) from the CLI.

use std::fs;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::TempDir;

/// A self-contained sandbox: isolated SKIT_* dirs and HOME, driving the real `skit` binary.
struct Sandbox {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self {
            data: TempDir::new().unwrap(),
            state: TempDir::new().unwrap(),
            config: TempDir::new().unwrap(),
            home: TempDir::new().unwrap(),
        }
    }

    fn command(&self) -> Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en")
            .env("HOME", self.home.path())
            .current_dir(self.home.path());
        command
    }

    /// Run `skit <args>`, assert exit 0, and return stdout as text.
    fn ok(&self, args: &[&str]) -> String {
        let output = self.command().args(args).output().unwrap();
        assert!(
            output.status.success(),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn json(&self, args: &[&str]) -> Value {
        serde_json::from_str(&self.ok(args)).unwrap()
    }

    /// Python `_py(tmp_path, body, name)`: write a source file, return its path.
    fn write_source(&self, name: &str, body: &str) -> String {
        let path = self.data.path().join(name);
        fs::write(&path, body).unwrap();
        path.to_str().unwrap().to_owned()
    }

    /// Python `_stored_block(slug)`: the stored copy's `script.py` text.
    fn stored_block(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join(format!("scripts/{slug}/script.py"))).unwrap()
    }

    /// The raw stored meta.toml text (for the `store.resolve(slug).meta.*` assertions).
    fn stored_meta(&self, slug: &str) -> String {
        fs::read_to_string(self.data.path().join(format!("scripts/{slug}/meta.toml"))).unwrap()
    }

    /// Python `_dry_run(slug)`: `run slug --dry-run --no-input`, assert exit 0, return the
    /// launch command line.
    fn dry_run(&self, slug: &str) -> String {
        self.ok(&["run", slug, "--dry-run", "--no-input"])
    }
}

// ==========================================================================
// 1. pin -> unpin -> re-pin, tracked end to end across block + run command + --json
// ==========================================================================

#[test]
fn test_pin_unpin_repin_block_line_tracks_the_constraint_end_to_end() {
    // The whole arc through the CLI: a pin writes the block's requires-python AND puts --python
    // on the launch command; an explicit unpin removes both (and clears --json); a re-pin restores
    // both. The stored block is what uv actually enforces, so the visible command and the block
    // must never disagree -- the exact drift this guards against (unpin cleared the command while
    // the block stayed pinned).
    let sandbox = Sandbox::new();
    let src = sandbox.write_source("s.py", "print(1)\n");
    sandbox.ok(&["add", &src, "--kind", "python", "--name", "a", "--no-input"]);

    // --- pin ---
    sandbox.ok(&["deps", "a", "--python", ">=3.12"]);
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.12\""),
        "block line written"
    );
    assert!(
        sandbox.dry_run("a").contains("--python"),
        "and uv would launch with the constraint"
    );

    // --- unpin ---
    sandbox.ok(&["deps", "a", "--python", "-"]);
    assert!(
        !sandbox.stored_block("a").contains("requires-python"),
        "block line removed"
    );
    assert!(
        !sandbox.dry_run("a").contains("--python"),
        "the launch command drops it too"
    );
    assert_eq!(
        sandbox.json(&["deps", "a", "--json"])["requires_python"],
        "",
        "--json agrees"
    );

    // --- re-pin ---
    sandbox.ok(&["deps", "a", "--python", ">=3.13"]);
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.13\""),
        "block line returns"
    );
    assert!(
        sandbox.dry_run("a").contains("--python"),
        "and the launch command carries it again"
    );
    assert!(
        sandbox
            .stored_meta("a")
            .contains("requires_python = \">=3.13\""),
        "meta in step"
    );
}

// ==========================================================================
// 2. a DEPS-ONLY edit preserves the pin -- both branches of the preserve predicate
// ==========================================================================

#[test]
fn test_deps_only_edit_preserves_a_pin_that_lives_only_in_the_block() {
    // Branch A of `not constraint and requires_python is None`: an add-time constraint injects
    // the block but leaves meta.requires_python "" (the deps_injected path). A later deps-only edit
    // reads the block for the constraint and PRESERVES it -- the derive rule on the meta-blank side.
    let sandbox = Sandbox::new();
    let src = sandbox.write_source("s.py", "print(1)\n");
    sandbox.ok(&[
        "add",
        &src,
        "--kind",
        "python",
        "--name",
        "a",
        "--python",
        ">=3.11",
        "--no-input",
    ]);
    assert!(
        !sandbox.stored_meta("a").contains("requires_python ="),
        "add-time injection clears meta"
    );
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.11\""),
        "...but the block carries it"
    );
    sandbox.ok(&["deps", "a", "--dep", "requests"]);
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.11\""),
        "preserved from the block"
    );
    assert!(
        sandbox.stored_block("a").contains("requests"),
        "the deps edit landed"
    );
}

#[test]
fn test_deps_only_edit_preserves_a_pin_that_lives_in_meta() {
    // Branch B of the same predicate: a prior `deps --python` sets meta.requires_python, so a
    // deps-only edit finds the constraint truthy in meta (the `not constraint` guard is False) and
    // preserves it there -- the block stays pinned too.
    let sandbox = Sandbox::new();
    let src = sandbox.write_source("s.py", "print(1)\n");
    sandbox.ok(&["add", &src, "--kind", "python", "--name", "a", "--no-input"]);
    sandbox.ok(&["deps", "a", "--clear", "--python", ">=3.10"]); // meta + block pinned
    assert!(
        sandbox
            .stored_meta("a")
            .contains("requires_python = \">=3.10\""),
    );
    sandbox.ok(&["deps", "a", "--dep", "requests"]);
    assert!(
        sandbox
            .stored_meta("a")
            .contains("requires_python = \">=3.10\""),
        "meta pin preserved"
    );
    assert!(
        sandbox
            .stored_block("a")
            .contains("requires-python = \">=3.10\""),
        "block still pinned"
    );
    assert!(sandbox.stored_block("a").contains("requests"));
}

// ==========================================================================
// 3. the settings-screen twin: clearing #st-python unpins the block
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (bucket 3): the oracle drives the interactive ScriptSettingsScreen \
(tui.MenuApp + pilot, clear #st-python -> action_save). That reducer/TestBackend surface is \
owned by the skit-tui / skit-ui tier (AGENTS.md: UI tests drive the reducer and Ratatui \
TestBackend); a CLI-binary integration test cannot drive it. Test 1's `deps a --python -` already \
covers the same store chokepoint (unpinning the stored block) from the CLI. Python ref: \
tests/test_uv_metadata_unpinning.py::test_settings_clearing_python_unpins_the_block."]
fn test_settings_clearing_python_unpins_the_block() {
    // Emptying #st-python on a pinned copy-mode entry reaches the same store chokepoint with
    // requires_python == "", so the save removes the block's requires-python line (not just the
    // meta field). Owned by the skit-tui / skit-ui reducer tier; see the ignore note above.
}
