//! Mechanical port of the Python oracle module
//! `tests/test_settings_and_draft_review_atomicity.py` (`origin/main@206f9ef`): "TUI coverage for
//! settings validate-then-write atomicity and review panels' self-derived `fresh` (the drafts
//! boundary through the CLI-hosted panel)." Each `#[test]` keeps its Python `def test_*` name and
//! its "WHY" comment so it traces back to the oracle.
//!
//! Tier: the Python module drives the Textual screens `ScriptSettingsScreen`, `AddReviewScreen`,
//! and `PromptReviewScreen` and asserts on the store AFTER `action_save`/`action_accept`. In the
//! Rust rewrite that screen logic is split: the serializable reducers `SettingsView` and
//! `ReviewState` in `skit-ui` model the screens (no file or repository I/O), and the composition
//! root `skit-cli` (`tui_submit_settings`, `cli.rs:7101`) hosts the validate-then-write and the
//! atomic store writes. So the settings deps/python REFUSAL and the write-pass ordering live at the
//! `skit-cli` tier, unreachable from a `skit-ui` integration test — `tui_submit_settings` is a
//! private, TUI-only function with no CLI subcommand — and are cross-crate stubs here. This file
//! asserts the reducer gates that back those screens.
//!
//! Concept mapping used throughout:
//! - Python `ScriptSettingsScreen(entry)` -> `SettingsView::from_inputs(&SettingsInputs)`; the
//!   `#st-deps`/`#st-python` widgets -> the `DEPENDENCIES_KEY`/`PYTHON_KEY` fields, read back as the
//!   per-axis edits `dependencies_edit()` / `requires_python_edit()` a save travels.
//! - Python `screen.action_save()` -> `SettingsView::update(SettingsAction::Save)`, which validates
//!   NAME + WORKDIR only (`SettingsError`) and returns `SettingsEffect::Save`/`Refused`. The deps
//!   and python validation the oracle also does in `action_save` was moved to the host
//!   (`cli.rs:7171-7184`, `validate_pep508_requirement` / `validate_pep440_specifiers` before any
//!   write) — the `#st-deps`/`#st-python` refusals are therefore cross-crate.
//! - Python `#st-python` `-`/`none` -> "" -> `requires_python_edit()` normalizes the same
//!   (`normalize_python_automatic`, `settings.rs:571-584`).
//! - Python `AddReviewScreen(path)._fresh = is_draft(path)` -> `ReviewState::is_fresh()`, which
//!   returns `source.is_draft`. The Python derivation splits in Rust: the path->is_draft
//!   classification is the HOST's (skit-cli fills `SourceSnapshot.is_draft` from `is_draft(path)`);
//!   the reducer contract these tests pin is `is_fresh() == source.is_draft` plus Copy-forcing
//!   (`set_storage` keeps a draft on `Copy`, `add.rs:792-800`), which makes the "Link the original"
//!   route unreachable exactly as the hidden `#rv-mode` radio does.
//! - Python `", ".join(suggest_dependencies(text, script_dir=...))` prefill -> the
//!   `dependencies_text()` a `ReviewState` opens with, computed by `external_dependencies_at`
//!   (`skit-language`), which drops a PEP 508-illegal name and a sibling local module the same way.
//!
//! Buckets (16 Python defs -> 15 `#[test]` here + 1 asserting skit-cli owner): 8 asserting here,
//! 1 asserting at the composition root, and 7 `#[ignore]` cross-crate stubs. The settings
//! deps/python REFUSAL atomicity IS present at
//! the skit-cli tier: `tui_submit_settings` runs `validate_pep508_requirement` /
//! `validate_pep440_specifiers` before its single atomic `update_entry`, so a bad requirement or
//! constraint refuses the whole save with nothing written. The npm-clear-first canonical owner is
//! rehomed to `skit-cli::cli::tests::test_settings_failed_npm_clear_commits_no_other_form_edits`,
//! where the real FileStore and JavaScript cleanup adapter are observable. Each remaining stub
//! names its owning tier in its `#[ignore]`.

use std::path::{Path, PathBuf};

use skit_application::SourcePermissions;
use skit_domain::StorageMode;
use skit_ui::{
    DEPENDENCIES_KEY, DependencyFlavor, FieldValue, KnownEntryKind, PYTHON_KEY, ReviewDefaults,
    ReviewState, SettingsAction, SettingsEffect, SettingsInputs, SettingsView, SourceSnapshot,
};

/// A python entry-settings screen with a chosen effective deps/python baseline (the values the
/// fields prefill from AND the save-time diff baseline, `settings.rs:987-988`).
fn python_settings(dependencies: &[&str], requires_python: &str) -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "entry".to_owned(),
        kind: "python".to_owned(),
        name: "orig".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        dependency_flavor: Some(DependencyFlavor::Uv),
        effective_dependencies: dependencies
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        effective_requires_python: requires_python.to_owned(),
        ..SettingsInputs::default()
    })
}

/// A js entry-settings screen: npm-flavored deps, and no `#st-python` widget.
fn js_settings() -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "entry".to_owned(),
        kind: "js".to_owned(),
        name: "jsset".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        dependency_flavor: Some(DependencyFlavor::Npm),
        ..SettingsInputs::default()
    })
}

/// One byte-exact host snapshot, the direct twin of the reviewed on-disk file or kept draft.
fn snapshot(path: &str, bytes: &[u8], is_draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(path),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(0o644),
        },
        is_regular: true,
        is_directory: false,
        is_draft,
        identity: None,
    }
}

/// A review the way the CLI-hosted panel (`AddReviewApp` / `PromptReviewApp`) opens one: it passes
/// no `fresh`, so the reducer derives it from the snapshot's `is_draft`.
fn review(path: &str, bytes: &[u8], kind: KnownEntryKind, is_draft: bool) -> ReviewState {
    ReviewState::from_source(
        snapshot(path, bytes, is_draft),
        kind,
        ReviewDefaults::default(),
    )
}

/// A self-contained scratch directory (skit-ui carries no `tempfile` dev-dependency, and its
/// `Cargo.toml` is a shared file this port must not edit). Removed on drop; only ever a directory
/// this run created under the system temp root.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = format!(
            "skit-ui-port-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create scratch dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

// ==========================================================================
// 1. Settings save: validate-then-write is atomic across the deps section
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli `tui_submit_settings`): the `#st-deps` refusal that keeps the whole save atomic lives at the composition root, not the skit-ui reducer."]
fn test_settings_bad_dep_refuses_the_whole_save_including_the_rename() {
    // WHY (oracle, tui_settings.py:978-984): a garbage `#st-deps` entry (`@@@`) plus a changed
    // `#st-name` must refuse the ENTIRE save — notify("package requirement", error), keep the
    // screen open, and NOT persist the rename.
    //
    // In the Rust split the skit-ui `SettingsView::Save` validates only NAME + WORKDIR
    // (`SettingsError`), so a bad requirement travels in `submitted_values()` as split text and the
    // HOST refuses it: `tui_submit_settings` runs `validate_pep508_requirement` before any write
    // (`cli.rs:7171-7175`, `-> CliError::Usage`, nothing written). The localized "package
    // requirement" wording is a skit-i18n concern. Owner: skit-cli `tui_submit_settings`; exercised
    // by the in-crate `crates/skit-cli/src/cli/tests.rs` settings suite.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli `tui_submit_settings`): the `#st-python` constraint refusal lives at the composition root, not the skit-ui reducer."]
fn test_settings_bad_python_refuses_the_whole_save_including_the_rename() {
    // WHY (oracle, tui_settings.py:982-984): the `#st-python` twin — an unparseable constraint plus
    // a changed name refuses everything with notify("version constraint", error).
    //
    // The host validates it: `tui_submit_settings` runs `validate_pep440_specifiers` before any
    // write (`cli.rs:7176-7184`, `-> CliError::Usage`). Owner: skit-cli `tui_submit_settings`.
}

#[test]
fn test_settings_dash_python_saves_as_automatic() {
    // '-' in #st-python normalizes to automatic: the save commits with the constraint cleared to "".
    let mut view = python_settings(&["requests"], ">=3.11");
    assert!(view.set_value(PYTHON_KEY, FieldValue::text("-")));
    // The python axis travels as the cleared "" the host writes (`meta.requires_python == ""`),
    // and the deps axis nobody touched stays "do not touch".
    assert_eq!(view.requires_python_edit(), Some(String::new()));
    assert_eq!(view.dependencies_edit(), None);
    // Committed & dismissed: the reducer refuses nothing (name/workdir are valid).
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
}

#[test]
fn test_settings_valid_deps_and_python_save_normally() {
    // The complement: valid values pass and land as the axes a save writes to meta + the block.
    let mut view = python_settings(&[], "");
    assert!(view.set_value(DEPENDENCIES_KEY, FieldValue::text("requests>=2,<3")));
    assert!(view.set_value(PYTHON_KEY, FieldValue::text("~=3.12")));
    // The PEP 508 splitter keeps the specifier's own comma, so this is one requirement, not two.
    assert_eq!(
        view.dependencies_edit(),
        Some(vec!["requests>=2,<3".to_owned()])
    );
    assert_eq!(view.requires_python_edit(), Some("~=3.12".to_owned()));
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
}

#[test]
fn test_settings_npm_deps_are_not_pep508_validated() {
    // A js entry's #st-deps is split with the npm splitter and NOT PEP 508-validated: a scoped
    // package (@scope/thing, which requirement_error rejects) still saves. There is no #st-python
    // widget on the npm flavor either.
    let mut view = js_settings();
    assert!(view.field(PYTHON_KEY).is_none()); // npm flavor: no Python constraint field
    assert!(view.set_value(DEPENDENCIES_KEY, FieldValue::text("@scope/thing")));
    // The npm splitter keeps the scoped package whole; the reducer validates nothing.
    assert_eq!(
        view.dependencies_edit(),
        Some(vec!["@scope/thing".to_owned()])
    );
    // Committed, not refused.
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli `tui_submit_settings` + skit-store): the name-collision precheck that runs before the npm clear lives at the composition root."]
fn test_settings_name_conflict_is_refused_before_npm_clear() {
    // WHY (oracle, tui_settings.py:1000-1016): a name already taken by another entry is refused in
    // the validation pass ("already taken"), BEFORE the npm clear runs — so `deps.clear` is never
    // called and the stored deps ("chalk") survive.
    //
    // The reducer does not know other entries exist; the collision precheck reads the registry.
    // Owner: skit-cli `tui_submit_settings` name precheck + skit-store `rename`.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli `tui_submit_settings` + skit-store): a store failure during the name precheck is reported with no writes at the composition root."]
fn test_settings_name_precheck_store_failure_is_reported_without_writes() {
    // WHY (oracle, tui_settings.py:1003-1010): when `store.resolve(new_name)` raises a StoreError
    // during the name precheck, the screen reports exactly that message and writes nothing (the
    // original name survives).
    //
    // The reducer has no registry to fail. Owner: skit-cli `tui_submit_settings` + skit-store.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli `tui_submit_settings` + skit-store): a rename-race failure stopping the later writes lives at the composition root."]
fn test_settings_rename_race_failure_stops_later_writes() {
    // WHY (oracle, tui_settings.py:1038-1043): when `store.rename` fails at write time ("name
    // became taken"), the later independent writes (description, ...) do not run — the message is
    // reported and nothing after the rename lands.
    //
    // The reducer emits typed axes; the ordered store writes and their early return are the host's.
    // Owner: skit-cli `tui_submit_settings` write pass + skit-store `rename`.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli `tui_submit_settings` + skit-store): a late dependency-store failure reported with the panel kept open lives at the composition root."]
fn test_settings_late_dependency_store_failure_is_reported_and_stays_open() {
    // WHY (oracle, tui_settings.py:1102-1108): when the late `store.update_dependencies` write
    // fails ("dependency metadata unavailable"), that message is reported, the panel stays open,
    // and the entry keeps no dependencies.
    //
    // The reducer plans the deps edit; the store write is the host's. Owner: skit-cli
    // `tui_submit_settings` late deps write + skit-store `update_dependencies`.
}

// ==========================================================================
// 2. Review panels DERIVE fresh from is_draft — the CLI-hosted panel hides Storage
// ==========================================================================

#[test]
fn test_add_panel_on_a_kept_draft_hides_storage_and_copies() {
    // AddReviewApp is EXACTLY what `skit add <file>` (form=tui) builds — and it never passes fresh.
    // On a kept draft the panel must still hide the Storage section (the derived fresh), so the
    // reference radio is unreachable and accept can only copy.
    let mut screen = review(
        "skit-new-resume.py",
        b"print('resumed')\n",
        KnownEntryKind::Python,
        true,
    );
    assert!(screen.is_fresh()); // derived from is_draft(path), not the (unset) flag
    assert_eq!(screen.storage(), StorageMode::Copy);
    // The reference route the hidden `#rv-mode` radio forbids: a draft stays Copy no matter what.
    screen.set_storage(StorageMode::Reference);
    assert_eq!(screen.storage(), StorageMode::Copy);
    screen.set_name("resumed");
    let entry = screen.create_entry().expect("a draft copy commits");
    assert_eq!(entry.mode, StorageMode::Copy); // the only shape the panel can reach
}

#[test]
fn test_prompt_panel_on_a_kept_draft_hides_storage_and_copies() {
    // The PromptReviewApp face of the same fix: a kept prompt draft opened through the CLI-hosted
    // panel hides Storage (derived fresh), so the entry can only be a copy.
    let mut screen = review(
        "skit-new-ask.prompt.md",
        b"Summarize {{text}}.\n",
        KnownEntryKind::Prompt,
        true,
    );
    assert!(screen.is_fresh());
    assert_eq!(screen.storage(), StorageMode::Copy);
    screen.set_storage(StorageMode::Reference);
    assert_eq!(screen.storage(), StorageMode::Copy);
    screen.set_name("asker");
    let entry = screen.create_entry().expect("a prompt draft copy commits");
    assert_eq!(entry.kind.as_str(), "prompt");
    assert_eq!(entry.mode, StorageMode::Copy);
}

#[test]
fn test_add_panel_on_a_nondraft_still_shows_storage() {
    // The complement (the derivation must not over-fire): a NON-draft on-disk file opened through
    // the same CLI-hosted panel still shows the Storage section — its original is real and
    // linkable, so fresh stays False.
    let mut screen = review(
        "ondisk.py",
        b"print('ondisk')\n",
        KnownEntryKind::Python,
        false,
    );
    assert!(!screen.is_fresh());
    // Storage present: copy vs link the original — the reference route is reachable.
    screen.set_storage(StorageMode::Reference);
    assert_eq!(screen.storage(), StorageMode::Reference);
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli add lane): the `skit add <draft>` (form=tui) host wiring + physical draft unlink. The skit-ui ConsumeDraft gate it rides on is covered at-tier by port_test_add_review_validation.rs::test_fresh_draft_copy_flow_unlinks_the_file."]
fn test_resumed_draft_through_the_tui_add_lane_is_consumed() {
    // WHY (oracle): the panel-hosted CLI lane's success arc — `skit add <draft>` (form=tui,
    // interactive) hosts the panel, and on a copy result the shared consume-on-success unlink fires
    // (`cli.py`: a resumed skit draft is done accumulating). The oracle STUBS the panel to a copy
    // accept, so what it pins is the tui-branch wiring (`_is_interactive`, `load_form()=="tui"`,
    // `run_add_review`) + the physical `not draft.exists()`, NOT the panel UI.
    //
    // That host wiring is skit-cli's add command lane (the owned-draft removal at `cli.rs:5658`),
    // covered by `crates/skit-cli/tests/port_test_add_validation_contracts.rs`. The reducer gate it
    // depends on — `AddEffect::ConsumeDraft` emitted only when a draft commits as a copy — is
    // asserted at the skit-ui tier by the sibling
    // `port_test_add_review_validation.rs::test_fresh_draft_copy_flow_unlinks_the_file`.
}

// ==========================================================================
// 3. The panel's dependency prefill runs through suggest_dependencies
// ==========================================================================

#[test]
fn test_add_panel_prefill_drops_a_pep508_illegal_import() {
    // The #rv-deps prefill is `", ".join(suggest_dependencies(text))`, which filters PEP 508-illegal
    // names: an `import café` (legal identifier, illegal distribution name) never seeds the field,
    // while a legal import beside it does.
    let screen = review(
        "/nonexistent-skit-ui-port/mixed.py",
        "import café\nimport requests\nprint(café, requests)\n".as_bytes(),
        KnownEntryKind::Python,
        false,
    );
    let prefill = screen.dependencies_text();
    assert!(!prefill.contains("café"), "{prefill}");
    assert!(prefill.contains("requests"), "{prefill}");
}

#[test]
fn test_add_panel_prefill_drops_a_sibling_local_module() {
    // The #rv-deps prefill is `suggest_dependencies(text, script_dir=self._path.parent)`, so a bare
    // `import helpers` that resolves to a sibling `helpers.py` next to the script never seeds the
    // field (suggesting it would install an unrelated PyPI `helpers` that the script's own module
    // shadows), while a real third-party import beside it still does.
    let scratch = ScratchDir::new();
    std::fs::write(
        scratch.path().join("helpers.py"),
        "def go():\n    return 1\n",
    )
    .expect("write sibling module");
    let script = scratch.path().join("uses_sib.py");
    let screen = review(
        script.to_str().expect("utf-8 path"),
        b"import helpers\nimport requests\nprint(helpers, requests)\n",
        KnownEntryKind::Python,
        false,
    );
    let prefill = screen.dependencies_text();
    assert!(!prefill.contains("helpers"), "{prefill}");
    assert!(prefill.contains("requests"), "{prefill}");
}
