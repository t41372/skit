//! Mechanical port of the Python oracle module `tests/test_uv_metadata_views.py`
//! (`origin/main@206f9ef`): "UV metadata read-view and compose-time-baseline contracts."
//! Every Python assertion pins an OBSERVABLE surface — the human `show` a user reads, the
//! library detail pane a user sees, the settings save chokepoint — never an internal flag,
//! and each surface must read EFFECTIVE metadata (`effective_uv_metadata`), not raw meta, so
//! a block-only add-time entry (deps + pin in the copy's PEP 723 block, meta blank) shows the
//! deps and pin uv actually enforces.
//!
//! Concept mapping used throughout:
//! - Python `store.add_python(src, name=…, dependencies=…, requires_python=…)` ->
//!   `skit add <path> --name … --dep … --python …` driven through the real binary (`assert_cmd`).
//!   Copy mode is the default; a no-block source makes copy mode inject deps into the copy's
//!   PEP 723 block and leave meta blank (the "deps_injected" path). Reference mode is `--ref`
//!   and records deps in meta instead.
//! - Python `runner.invoke(cli.app, ["show", "x"])` -> spawn the `skit show x` subcommand and
//!   read its stdout (substring `contains`, exactly as Python asserts).
//! - Python `store.resolve("x").meta` sanity checks -> read the stored `scripts/<slug>/meta.toml`
//!   and `scripts/<slug>/script.py` bytes; a block-only entry's meta carries neither axis and its
//!   block carries both.
//! - Python `store.effective_uv_metadata(entry)` -> `effective_uv_metadata_bytes`, wired through
//!   `effective_settings` (cli.rs:2482) for `show` and through `library_surface`'s own
//!   `effective_settings` (library_surface.rs) for the detail pane's `LibraryEntryDetail.dependencies`.
//! - Python `tui.MenuApp()` detail pane -> the `library_surface(store, state, config)` projection
//!   fills `LibraryEntryDetail.dependencies` with the effective deps; the skit-tui render turns a
//!   nonempty list into the "Depends on  {}" line (`skit-tui/src/screens/library.rs:406`).
//!
//! Buckets:
//! - show-human effective (tests 1-3): REAL — the composition-root observable is `skit show`.
//! - detail-pane effective (tests 4-5): REAL at the facts level — `library_surface` is public and
//!   `skit-store` is a dependency, so the load-bearing chain the Python bug lived in
//!   (store -> effective facts) is asserted directly on `LibraryEntryDetail.dependencies`. The
//!   literal "Depends on" string is a thin skit-tui render conditional named in each WHY comment.
//! - compose-time save baseline (test 6): CROSS-CRATE `#[ignore]` stub. The Python mechanism
//!   (monkeypatch `store.effective_uv_metadata` after mount, then assert `update_dependencies` is
//!   not called) has no Rust analog by design: skit-ui `SettingsView::from_inputs` captures
//!   `effective_dependencies` BY VALUE at compose time (settings.rs:4/479/746), so there is no
//!   save-time re-read to intercept — the race is structurally impossible below the settings
//!   screen — and the "chokepoint not entered" observable is wired in the skit-tui settings save
//!   path. Behavior is PRESENT, just unreachable from a skit-cli-rs integration test.

use std::fs;

use tempfile::TempDir;

use skit_domain::Slug;
use skit_store::{FileStore, library_surface};

/// A local SKIT_* fixture: three temporary directories, never the real user dirs, never a chdir.
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

    /// Write the oracle's `_py` fixture (`print(1)\n` at `x.py`) into the data dir and return it.
    fn write_py(&self) -> std::path::PathBuf {
        let source = self.data.path().join("x.py");
        fs::write(&source, "print(1)\n").unwrap();
        source
    }

    /// The stored `scripts/<slug>/meta.toml` text for a committed entry.
    fn meta_text(&self, slug: &str) -> String {
        fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("meta.toml"),
        )
        .unwrap()
    }

    /// The stored `scripts/<slug>/script.py` text (the copy's PEP 723 block lives here).
    fn script_text(&self, slug: &str) -> String {
        fs::read_to_string(
            self.data
                .path()
                .join("scripts")
                .join(slug)
                .join("script.py"),
        )
        .unwrap()
    }

    /// The effective package dependencies the detail pane would render for one entry.
    ///
    /// This is the same projection the TUI reads: `library_surface` fills
    /// `LibraryEntryDetail.dependencies` from `effective_uv_metadata_bytes`.
    fn detail_dependencies(&self, slug: &str) -> Vec<String> {
        let store = FileStore::new(self.data.path());
        let surface = library_surface(&store, self.state.path(), self.config.path()).unwrap();
        surface
            .details
            .get(&Slug::parse(slug).unwrap())
            .expect("the added entry has a detail projection")
            .dependencies
            .clone()
    }

    fn show(&self, name: &str) -> String {
        let output = self.command().args(["show", name]).output().unwrap();
        assert!(
            output.status.success(),
            "show exited nonzero: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }
}

// ==========================================================================
// 1. show-human reads EFFECTIVE metadata (MEDIUM)
// ==========================================================================

#[test]
fn test_show_human_block_only_prints_effective_deps_and_constraint() {
    // `skit show x` (human) reads EFFECTIVE metadata: a block-only add-time entry (deps + pin in
    // the copy's PEP 723 block, meta deliberately blank) prints its real Dependencies AND Python
    // constraint lines — matching its own `show --json`, where raw meta showed a bare face with
    // neither. Both `if effective_deps` / `if effective_python` truthy.
    let sandbox = Sandbox::new();
    let source = sandbox.write_py();
    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "x",
            "--dep",
            "requests",
            "--python",
            ">=3.11",
        ])
        .assert()
        .success();
    // meta blank on both axes; the deps and pin live in the copy's PEP 723 block.
    let meta = sandbox.meta_text("x");
    assert!(!meta.contains("dependencies"), "{meta}"); // meta blank...
    assert!(!meta.contains("requires_python"), "{meta}"); // ...on both axes
    let script = sandbox.script_text("x");
    assert!(script.contains("dependencies = [\"requests\"]"), "{script}");
    assert!(script.contains("requires-python = \">=3.11\""), "{script}");

    let output = sandbox.show("x");
    assert!(output.contains("Dependencies: requests"), "{output}");
    assert!(output.contains("Python constraint: >=3.11"), "{output}");
}

#[test]
fn test_show_human_meta_carried_deps_unchanged() {
    // The meta-carried face is unchanged by the switch to effective_uv_metadata: a reference-mode
    // entry records its deps in meta, and the human `show` prints them straight from meta (the block
    // fallback is a python-copy-only path that never fires here).
    let sandbox = Sandbox::new();
    let source = sandbox.write_py();
    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "x",
            "--ref",
            "--dep",
            "rich",
        ])
        .assert()
        .success();
    assert!(
        sandbox.meta_text("x").contains("dependencies = [\"rich\"]"),
        "meta carries the deps"
    );

    let output = sandbox.show("x");
    assert!(output.contains("Dependencies: rich"), "{output}");
}

#[test]
fn test_show_human_no_uv_metadata_prints_neither_line() {
    // An entry with no deps and no pin (effective_uv_metadata returns the empty pair) prints
    // NEITHER line — the falsy branch of both display conditions.
    let sandbox = Sandbox::new();
    let source = sandbox.write_py();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "x"])
        .assert()
        .success();

    let output = sandbox.show("x");
    assert!(!output.contains("Dependencies:"), "{output}");
    assert!(!output.contains("Python constraint:"), "{output}");
}

// ==========================================================================
// 2. library detail pane reads EFFECTIVE deps (LOW)
// ==========================================================================

#[test]
fn test_detail_pane_block_only_shows_effective_depends_on() {
    // The library detail pane reads EFFECTIVE deps: a block-only add-time python entry (meta blank,
    // deps in the copy's block) shows "Depends on requests" — the same list its settings screen
    // shows, where raw meta would have left the line off entirely.
    //
    // Facts-level real port: `library_surface` fills `LibraryEntryDetail.dependencies` from the
    // effective metadata (store.py:1167 -> effective_uv_metadata_bytes); the skit-tui render turns a
    // nonempty list into "Depends on  {}" (skit-tui/src/screens/library.rs:406).
    let sandbox = Sandbox::new();
    let source = sandbox.write_py();
    sandbox
        .command()
        .args([
            "add",
            source.to_str().unwrap(),
            "--name",
            "x",
            "--dep",
            "requests",
        ])
        .assert()
        .success();
    assert!(
        !sandbox.meta_text("x").contains("dependencies"),
        "meta blank; deps live in the block"
    );

    assert_eq!(sandbox.detail_dependencies("x"), ["requests"]);
}

#[test]
fn test_detail_pane_no_deps_omits_the_depends_on_line() {
    // The falsy branch: an entry with no effective deps shows no "Depends on" line at all — the
    // detail facts carry an empty dependency list, so the render conditional never fires
    // (skit-tui/src/screens/library.rs:406 `if !facts.dependencies.is_empty()`).
    let sandbox = Sandbox::new();
    let source = sandbox.write_py();
    sandbox
        .command()
        .args(["add", source.to_str().unwrap(), "--name", "x"])
        .assert()
        .success();

    assert!(sandbox.detail_dependencies("x").is_empty());
}

// ==========================================================================
// 3. compose-time save baseline (LOW) — the settings deps diff runs on the
//    open-time clock, not a save-time re-read.
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-ui SettingsView + skit-tui settings save): the Python race (monkeypatch store.effective_uv_metadata after mount, assert update_dependencies not called) has no Rust analog. SettingsView::from_inputs captures effective_dependencies BY VALUE at compose time (skit-ui/src/settings.rs:4,479,746), so there is no save-time re-read to intercept; the untouched-save chokepoint (update_dependencies not entered) is wired in the skit-tui settings screen. Behavior is PRESENT, unreachable from a skit-cli-rs integration test."]
fn test_settings_save_diffs_against_compose_time_baseline_not_a_re_read() {
    // The deps/constraint save-diff runs against `_deps_baseline` stashed when the fields were
    // composed — NOT a save-time re-read of effective_uv_metadata. A concurrent CLI write that moves
    // the block underneath an open screen must not make an UNTOUCHED field look like an explicit
    // edit. Python pins it by monkeypatching effective_uv_metadata to a DIFFERENT pair after mount:
    // a save-time re-read would misclassify the unchanged Inputs as an edit, but the compose-time
    // baseline does not, so update_dependencies is never called.
    //
    // Rust residue (skit-ui, own crate): `SettingsView::dependencies_edit()` returns `None` for an
    // untouched field because the baseline is the same value captured at compose time
    // (see the skit-ui settings test "same read is the baseline").
}
