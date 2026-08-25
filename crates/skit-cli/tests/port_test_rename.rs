//! Mechanical port of the Python oracle module `tests/test_rename.py`
//! (`origin/main@206f9ef`): "store.rename: display name changes; the slug (dir, state key)
//! never moves." Each `#[test]` keeps its Python `def test_*` name and its WHY comment so it
//! traces back to its origin.
//!
//! WHY `skit-cli`: the oracle drives `skit.store` (add/rename/resolve/list/doctor_rebuild) and
//! `skit.argstate` (the per-slug values file) together. The Rust rewrite splits those seams —
//! the mutation into `skit-store`/`skit-application`, the state file under `SKIT_STATE_DIR` —
//! and the whole surface is reachable as a composed product only through the `skit` binary. So
//! the port drives the real binary through `assert_cmd`, exactly like the sibling exemplar
//! `port_test_declared_params.rs`.
//!
//! Concept mapping used throughout:
//! - Python `store.add_python(path, name=)` -> `skit add <path> --kind python --name <name>`
//!   (copy mode; slug is `slugify(name)`, e.g. "old" -> "old", "Old Name" -> "old-name").
//! - Python `store.rename(name_or_slug, new)` -> `skit rename <selector> <new>`.
//! - Python `store.resolve(x).meta.name` -> `skit show <x> --json` field `name`;
//!   `store.NotFoundError` -> the command fails ("entry not found: <x>").
//! - Python `entry.slug` -> `show --json` field `slug`; `entry.dir` -> the on-disk
//!   `<data>/scripts/<slug>` directory (immutable, so it never moves on a rename).
//! - Python `store.list_entries()` names -> `skit list --json` `name` fields.
//! - Python `store.doctor_rebuild()` -> `skit doctor --rebuild --json` fields `rebuilt`
//!   (the count) and `rebuild_problems` (the problems list).
//! - Python `argstate.save_last(slug, values={"X": "1"})` -> the state file
//!   `<state>/values/<slug>.toml` holding `[values]\nX = "1"\n`; `argstate.load_state(slug)
//!   ["values"]` -> that same file read back (untouched by a rename, because it is keyed by the
//!   immutable slug).
//! - Python `store.StoreError` on rename -> the command fails (non-zero exit).
//!
//! Buckets:
//! - REAL asserting `#[test]` (API exists): the six synchronous `store.rename` tests.
//! - CROSS-CRATE (`#[ignore]` stub): the four `async def` Textual tests. They drive
//!   `tui.MenuApp` + `skit.tui_settings.ScriptSettingsScreen`. In Rust that composed surface is
//!   the settings reducer in `skit-ui` (`SettingsView`, which owns hiding the "tick to manage"
//!   checkboxes and rendering the argparse blurb — `crates/skit-ui/src/settings.rs`), the screen
//!   in `skit-tui`, and the save->rename wiring, which is `skit-cli`-private
//!   (`tui_submit` / `FormPurpose::Rename`, exercised at `src/cli/tests.rs:2860-2885`). None of
//!   that is reachable as a composed outcome from a public integration test in this crate. The
//!   rename OUTCOME each one asserts (rename works; a conflict is refused) is covered by the
//!   synchronous CLI tests here.

use std::fs;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;
use tempfile::TempDir;

// ---- shared helpers (self-contained; this file edits no shared module) --------------------------

/// One isolated skit library: private data/state/config directories plus a source scratch dir.
struct Lib {
    data: TempDir,
    state: TempDir,
    config: TempDir,
    src: TempDir,
}

fn lib() -> Lib {
    Lib {
        data: TempDir::new().unwrap(),
        state: TempDir::new().unwrap(),
        config: TempDir::new().unwrap(),
        src: TempDir::new().unwrap(),
    }
}

impl Lib {
    /// A `skit` invocation with every SKIT_* directory pointed at this sandbox (task constraint:
    /// all three, on every call), and the source locale pinned so the human strings are English.
    fn cmd(&self) -> assert_cmd::Command {
        let mut command = assert_cmd::cargo::cargo_bin_cmd!("skit");
        command
            .env("SKIT_DATA_DIR", self.data.path())
            .env("SKIT_STATE_DIR", self.state.path())
            .env("SKIT_CONFIG_DIR", self.config.path())
            .env("SKIT_LANG", "en");
        command
    }

    /// Run one `skit` invocation and return its raw output (status + both streams).
    fn run(&self, args: &[&str]) -> Output {
        self.cmd().args(args).output().unwrap()
    }

    /// The oracle's `_py(tmp_path, name)`: a trivial python file written into the scratch dir.
    fn py(&self, file: &str) -> PathBuf {
        let path = self.src.path().join(file);
        fs::write(&path, "print(1)\n").unwrap();
        path
    }

    /// The oracle's `_py_text(tmp_path, text, name)`: an arbitrary python file in the scratch dir.
    fn py_text(&self, file: &str, text: &str) -> PathBuf {
        let path = self.src.path().join(file);
        fs::write(&path, text).unwrap();
        path
    }

    /// Python `store.add_python(_py(...), name=name)`: register a copy-mode python entry.
    fn add_python(&self, file: &str, name: &str) {
        let source = self.py(file);
        self.cmd()
            .arg("add")
            .arg(&source)
            .args(["--kind", "python", "--name", name])
            .assert()
            .success();
    }

    /// The `<data>/scripts/<slug>` directory — Python's `entry.dir`.
    fn entry_dir(&self, slug: &str) -> PathBuf {
        self.data.path().join("scripts").join(slug)
    }

    /// Seed the per-slug state file, as `argstate.save_last(slug, values=...)` would.
    fn seed_values(&self, slug: &str, body: &str) {
        let dir = self.state.path().join("values");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{slug}.toml")), body).unwrap();
    }

    /// The stored state file text for a slug — Python's `argstate.load_state(slug)` source.
    fn values_file(&self, slug: &str) -> String {
        fs::read_to_string(
            self.state
                .path()
                .join("values")
                .join(format!("{slug}.toml")),
        )
        .unwrap_or_default()
    }

    /// `store.resolve(x)` as JSON, or `None` when it fails (Python's `NotFoundError`).
    fn show(&self, selector: &str) -> Option<Value> {
        let output = self.run(&["show", selector, "--json"]);
        if output.status.success() {
            Some(serde_json::from_slice(&output.stdout).expect("show --json is one JSON document"))
        } else {
            None
        }
    }

    /// Make uv visible to `doctor` without a dependency on the host machine.
    ///
    /// `doctor` exits 1 when uv is missing and the library needs uv (an empty library, or any
    /// python entry). This library holds a python entry, so a machine without uv fails the
    /// command for a reason this test does not assert. The product also accepts the private uv
    /// below the data directory (`skit_runtime::managed_uv_path`), so put an executable file at
    /// that exact path. The skit binary itself is the executable that is always available here.
    fn install_private_uv_probe(&self) {
        let bin = self.data.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let name = if cfg!(windows) { "uv.exe" } else { "uv" };
        fs::copy(env!("CARGO_BIN_EXE_skit"), bin.join(name)).unwrap();
    }
}

/// Both streams a user would see, joined (mirrors Python's `result.output`).
fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

// ================================================================================================
// store.rename  (skit binary)
// ================================================================================================

#[test]
fn test_rename_changes_name_and_keeps_slug_dir_and_state() {
    // The display name changes but the slug is immutable: nothing on disk moves and the
    // remembered values (keyed by slug) survive.
    let workspace = lib();
    workspace.add_python("a.py", "old"); // slug == "old"
    workspace.seed_values("old", "[values]\nX = \"1\"\n"); // argstate.save_last(slug, {"X": "1"})

    workspace
        .cmd()
        .args(["rename", "old", "new"])
        .assert()
        .success();

    let renamed = workspace.show("new").expect("the new name resolves");
    assert_eq!(renamed["name"], "new"); // renamed.meta.name == "new"
    assert_eq!(renamed["slug"], "old"); // renamed.slug == entry.slug (immutable)
    // renamed.dir == entry.dir: the slug directory is unchanged, and no "new" directory appears.
    assert!(workspace.entry_dir("old").is_dir());
    assert!(!workspace.entry_dir("new").exists());
    // The slug still resolves to the renamed entry.
    assert_eq!(workspace.show("old").expect("slug resolves")["name"], "new");
    // load_state(entry.slug)["values"] == {"X": "1"}: the values file is byte-for-byte untouched.
    assert_eq!(workspace.values_file("old"), "[values]\nX = \"1\"\n");
}

#[test]
fn test_rename_updates_resolution_and_listing() {
    // Use a name whose slug differs, so "old name gone" and "slug survives" are
    // observable separately (the slug is the immutable internal id).
    let workspace = lib();
    workspace.add_python("a.py", "Old Name"); // slug == "old-name"

    workspace
        .cmd()
        .args(["rename", "Old Name", "new"])
        .assert()
        .success();

    assert_eq!(workspace.show("new").expect("new resolves")["name"], "new");
    assert!(workspace.show("Old Name").is_none()); // store.NotFoundError: the old name is gone
    // The slug keeps resolving to the renamed entry.
    assert_eq!(
        workspace.show("old-name").expect("slug resolves")["name"],
        "new"
    );
    // [e.meta.name for e in store.list_entries()] == ["new"]
    let listing = workspace.run(&["list", "--json"]);
    assert!(listing.status.success(), "{}", combined(&listing));
    let entries: Value = serde_json::from_slice(&listing.stdout).unwrap();
    let names: Vec<&str> = entries
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["new"]);
}

#[test]
fn test_rename_conflict_is_a_clean_error() {
    // Renaming onto an existing display name is refused, the message names the taken name, and
    // the source entry is untouched.
    let workspace = lib();
    workspace.add_python("a.py", "alpha");
    workspace.add_python("b.py", "beta");

    let output = workspace.run(&["rename", "beta", "alpha"]);
    assert!(!output.status.success(), "{}", combined(&output));
    assert!(combined(&output).contains("alpha"), "{}", combined(&output));

    // beta is untouched.
    assert_eq!(
        workspace.show("beta").expect("beta resolves")["name"],
        "beta"
    );
}

#[test]
fn test_rename_to_own_name_is_a_no_op() {
    // Renaming an entry to the name it already has succeeds and keeps the name.
    let workspace = lib();
    workspace.add_python("a.py", "same");
    workspace
        .cmd()
        .args(["rename", "same", "same"])
        .assert()
        .success();
    assert_eq!(
        workspace.show("same").expect("same resolves")["name"],
        "same"
    );
}

#[test]
fn test_rename_empty_name_rejected() {
    // A blank (all-whitespace) new name is a StoreError — the command fails.
    let workspace = lib();
    workspace.add_python("a.py", "x");
    let output = workspace.run(&["rename", "x", "   "]);
    assert!(!output.status.success(), "{}", combined(&output));
    // The entry keeps its old name.
    assert_eq!(workspace.show("x").expect("x resolves")["name"], "x");
}

#[test]
fn test_rename_survives_doctor_rebuild() {
    // meta.toml is the truth: rebuilding the registry from the per-slug metas recovers the new
    // name with no problems.
    let workspace = lib();
    workspace.install_private_uv_probe(); // the exit code must not depend on the host machine
    workspace.add_python("a.py", "old");
    workspace
        .cmd()
        .args(["rename", "old", "new"])
        .assert()
        .success();

    let output = workspace.run(&["doctor", "--rebuild", "--json"]);
    assert!(output.status.success(), "{}", combined(&output));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["rebuilt"], 1); // count == 1
    assert_eq!(report["rebuild_problems"], serde_json::json!([])); // problems == []
    assert_eq!(workspace.show("new").expect("new resolves")["name"], "new");
}

// ================================================================================================
// settings screen (Textual)  ->  CROSS-CRATE stubs
// ================================================================================================

#[test]
#[ignore = "CROSS-CRATE: oracle drives tui.MenuApp + skit.tui_settings.ScriptSettingsScreen (tests/test_rename.py:76-92). In Rust the settings save->rename wiring is skit-cli-private (tui_submit / FormPurpose::Rename, exercised at src/cli/tests.rs:2860-2885); the screen is skit-tui and the reducer is skit-ui. Not reachable as a composed outcome from a public integration test here. The rename OUTCOME is covered by test_rename_changes_name_and_keeps_slug_dir_and_state above."]
fn test_settings_screen_renames_on_save() {
    // Oracle: open settings on entry "old", set #st-name = "shiny", save; then
    // store.resolve("shiny").meta.name == "shiny".
}

#[test]
#[ignore = "CROSS-CRATE: oracle drives tui.MenuApp + skit.tui_settings.ScriptSettingsScreen (tests/test_rename.py:95-114). The conflict-refusal (screen stays open) is skit-ui SettingsView + skit-tui screen state, wired through skit-cli-private tui_submit (the refusal arm at src/cli/tests.rs:2860-2871). Not reachable from a public integration test here. The conflict-refusal OUTCOME is covered by test_rename_conflict_is_a_clean_error above."]
fn test_settings_screen_rename_conflict_stays_open() {
    // Oracle: select beta's slug, open settings, set #st-name = "alpha", save; the screen refuses
    // and stays a ScriptSettingsScreen, and store.resolve("beta").meta.name is still "beta".
}

#[test]
#[ignore = "CROSS-CRATE: oracle drives skit.tui_settings.ScriptSettingsScreen (tests/test_rename.py:131-148). Hiding the \"tick to manage\" candidate checkboxes and rendering the argparse blurb is owned by the skit-ui settings reducer (SettingsView note in crates/skit-ui/src/settings.rs:1194) and rendered by skit-tui. Not reachable as a SettingsView built through the composed product from a public integration test here."]
fn test_settings_hides_manage_checkboxes_for_argparse_script() {
    // Oracle: an argparse script (plan source == "argparse") shows no #st-new-0 manage
    // checkbox, and a Static blurb contains "comes from its own command-line arguments".
    let workspace = lib();
    let source = workspace.py_text(
        "s.py",
        "import argparse\nTIMEOUT = 30\nap = argparse.ArgumentParser()\nap.add_argument('--out', required=True)\nap.parse_args()\n",
    );
    // Sanity anchor the oracle also asserts: the plan is served by the script's own args.
    workspace
        .cmd()
        .arg("add")
        .arg(&source)
        .args(["--kind", "python", "--name", "ap"])
        .assert()
        .success();
    assert_eq!(
        workspace.show("ap").expect("ap resolves")["param_source"],
        "argparse"
    );
}

#[test]
#[ignore = "CROSS-CRATE: oracle drives skit.tui_settings.ScriptSettingsScreen (tests/test_rename.py:151-165). \"Saving from settings must not write a [tool.skit] block that shadows argparse\" is the skit-cli-private settings-save path (tui_submit) over the skit-ui SettingsView; there is no CLI settings-save command to trigger it from a public integration test here. The store-level fact that add alone keeps source == \"argparse\" is anchored in test_settings_hides_manage_checkboxes_for_argparse_script above."]
fn test_settings_save_keeps_argparse_source() {
    // Oracle: add argparse script, open settings, save (no edits); then
    // flows.plan_for_entry(store.resolve("ap2")).source == "argparse" (no shadowing block).
}
