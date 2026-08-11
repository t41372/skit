//! Mechanical port of the Python oracle module `tests/test_tui_edit.py`
//! (`origin/main@206f9ef`): "Edit-from-Library behavior: source resolution rules and the
//! suspend/editor round trip." Each `#[test]` keeps its Python `def test_*` name and its WHY
//! comment, so it traces back to its origin.
//!
//! WHY THIS PORT IS MOSTLY CROSS-CRATE STUBS. The Python `MenuApp.action_edit`
//! (`src/skit/tui.py:874-902`) is one monolithic method that (a) resolves the editable source,
//! (b) suspends and opens the editor, (c) writes the status line, and (d) invalidates a per-app
//! drift cache. The Rust rewrite splits that one method across three tiers:
//!   - the reducer (this crate's `skit-ui`): `Action::Edit` emits `Effect::Edit { selector }` for
//!     the selected entry and nothing more — no editability gate, no path, no status
//!     (`crates/skit-ui/src/lib.rs:1818-1822`);
//!   - the store (`skit-store`, NOT a dependency of `skit-tui`): the source resolution —
//!     `FileStore::payload_path` (`crates/skit-store/src/paths.rs:60-98`);
//!   - the composition root (`skit-cli`): `edit_with_config` serves `Effect::Edit`, refuses
//!     non-editable kinds, opens the editor, and reports the result
//!     (`crates/skit-cli/src/cli.rs:3229-3338`).
//!
//! `skit-tui` reaches only `skit-ui`/`skit-domain`/`skit-application` (verified: its Cargo.toml
//! has no `skit-store` or `skit-cli` edge), so every behavior below `Effect::Edit` is cross-crate.
//! The CLI/store-observable half of this oracle module is already owned by
//! `tests/test_edit.py -> crates/skit-cli/tests/port_test_edit.rs` (its
//! `test_cli_edit_command_entry_has_no_source` is the twin of `test_edit_command_entry_reports_no_source`).
//! One oracle module maps to one port file, so this file does not duplicate that coverage; it keeps
//! only the reducer-observable assertion and names the owning tier for the rest.
//!
//! Concept mapping:
//! - Python `tui.MenuApp()` -> `skit_ui::LibraryState::from_scan(..)` (the serializable reducer
//!   state; `from_scan` auto-selects the first visible row, mirroring the app's initial selection).
//! - Python `app.action_edit()` -> `state.update(Action::Edit)` returning an `Effect`.
//! - Python `app._editable_source(entry)` -> `skit_store::FileStore::payload_path(&entry)`
//!   (cross-crate; see the stubs).
//! - Python `store.add_python(path, name, mode=..)` -> the real add lane (`skit add`); no reducer
//!   equivalent, so the resolution stubs name the store path instead.
//!
//! Sandbox note: the Python `tmp_store` fixture points `SKIT_DATA_DIR`/`SKIT_STATE_DIR`/
//! `SKIT_CONFIG_DIR` at temp dirs because the app reads a real store. The Rust reducer is pure and
//! drives no store and no binary, so no `SKIT_*` environment is set here.

use skit_application::LibraryScan;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_ui::{Action, Effect, LibraryState};

/// One library row, mirroring the Python fixtures' single stored entry.
fn entry(slug: &str, name: &str, kind: &str, mode: StorageMode) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode,
        description: String::new(),
        target: None,
    }
}

/// Build reducer state from one scan. `from_scan` selects the first visible row, so a one-entry
/// library has that entry selected — the state in which the Python tests call `action_edit`.
fn state(entries: Vec<EntrySummary>) -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries,
        diagnostics: Vec::new(),
    })
}

// ---------- source resolution rules (`_editable_source`) ----------
//
// CROSS-CRATE (kind=cross-crate): `_editable_source` is `FileStore::payload_path`, owned by
// `skit-store`, which `skit-tui` cannot reach. The reducer never resolves a path; it emits only
// `Effect::Edit { selector }`. Each stub records the oracle contract AND that the store already
// implements it, so it names the owner rather than over-claiming a gap.

#[test]
#[ignore = "CROSS-CRATE (kind=cross-crate): source resolution is skit-store FileStore::payload_path, unreachable from skit-tui. Copy-mode returns the stored copy path (crates/skit-store/src/paths.rs:60-98: entry_dir_path(slug).join(\"script.py\")), matching the oracle."]
fn test_editable_source_copy_mode_points_at_the_stored_copy() {
    // A copy-mode entry edits its stored copy, not the vanished original.
    //   entry = store.add_python(_py("print(1)\n"), name="a")
    //   assert app._editable_source(entry) == entry.dir / "script.py"
}

#[test]
#[ignore = "CROSS-CRATE (kind=cross-crate): source resolution is skit-store FileStore::payload_path, unreachable from skit-tui. Reference-mode returns the original source path (crates/skit-store/src/paths.rs:61-63: PathBuf::from(&entry.meta.source)), matching the oracle."]
fn test_editable_source_reference_mode_points_at_the_original() {
    // A reference-mode entry edits the user's original file in place.
    //   entry = store.add_python(_py("print(1)\n", "orig.py"), name="r", mode="reference")
    //   assert app._editable_source(entry) == Path(entry.meta.source)
}

#[test]
#[ignore = "CROSS-CRATE (kind=cross-crate): the not-editable gate for command/exe is skit-cli edit_with_config (crates/skit-cli/src/cli.rs:3275-3282, refuses with \"entry {} does not have an editable source\"), unreachable from skit-tui. The reducer does NOT gate on kind."]
fn test_editable_source_command_entry_has_none() {
    // A command entry has no editable source; the Python spec's `editable=False` yields None.
    //   entry = store.add_command("echo hi", name="c")
    //   assert app._editable_source(entry) is None
}

// ---------- suspend / editor round trip (`action_edit`) ----------

#[test]
fn test_edit_opens_editor_and_reports() {
    // The Python `action_edit` opens the editor at the stored copy and writes "Edited a.".
    //   entry = store.add_python(_py("print(1)\n"), name="a")
    //   app.action_edit()
    //   assert opened == [entry.dir / "script.py"]
    //   assert "Edited a." in str(app.query_one("#status").render())
    //
    // TIER BOUNDARY: `skit-tui`'s whole contribution to `action_edit` is the reducer emitting an
    // `Effect::Edit` that carries the SELECTED entry's selector. Serving that effect — opening the
    // editor at the resolved copy path and writing the "Source saved" / "Edited a." status — is the
    // composition root's job (`skit-cli` `edit_with_config` + `tui_complete`), asserted in
    // `crates/skit-cli/tests/port_test_edit.rs`. So this test asserts the reducer half exactly: the
    // Edit effect names entry "a".
    let mut view = state(vec![entry("a", "a", "python", StorageMode::Copy)]);
    let effect = view.update(Action::Edit);
    assert_eq!(
        effect,
        Effect::Edit {
            selector: "a".to_owned(),
        }
    );
}

#[test]
#[ignore = "CROSS-CRATE (kind=cross-crate): the \"no editable source\" refusal is skit-cli edit_with_config (crates/skit-cli/src/cli.rs:3275-3278), unreachable from skit-tui; asserted by port_test_edit.rs::test_cli_edit_command_entry_has_no_source. The skit-ui reducer does NOT gate on editability — Action::Edit emits Effect::Edit for a command entry too — so no reducer assertion mirrors this oracle def."]
fn test_edit_command_entry_reports_no_source() {
    // Editing a command entry launches no editor and reports "no editable source".
    //   store.add_command("echo hi", name="c")
    //   app.action_edit()
    //   assert opened == []
    //   assert "no editable source" in str(app.query_one("#status").render())
}

#[test]
#[ignore = "CROSS-CRATE (kind=cross-crate): the Rust design has NO per-app drift cache; drift is recomputed on every scan by skit-store library_surface (crates/skit-store/src/library_surface.rs:120 `drifted: !plan.drift.is_empty()`), unreachable from skit-tui. The mtime-keyed _drift_cache is an oracle-internal optimization; a post-edit reload re-derives drift from the file, so the observable guarantee holds without a cache to invalidate."]
fn test_edit_invalidates_the_drift_cache() {
    // After an edit, the stale drift sentinel is gone and the truth is re-derived from the file.
    //   app._drift_cache[entry.slug] = (0.0, True)
    //   app.action_edit()
    //   mtime, drift = app._drift_cache[entry.slug]
    //   assert mtime != 0.0
    //   assert drift is False
}
