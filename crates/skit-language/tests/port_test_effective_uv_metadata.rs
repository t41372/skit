//! Mechanical port of the Python oracle module `tests/test_effective_uv_metadata.py`
//! (`origin/main@206f9ef`): the two independently editable uv axes — dependencies and the
//! Python constraint — and the rule "meta blank + block truthy => the block is the truth;
//! untouched and cleared are different inputs". Each `#[test]` keeps its Python `def test_*`
//! name and its "WHY" rationale so it traces back to its origin.
//!
//! Concept mapping:
//! - Python `store.effective_uv_metadata(entry) -> (deps, constraint)` maps to the pure
//!   `effective_uv_metadata_bytes(source, stored) -> UvMetadata` (skit-language). Python reads
//!   `entry.meta` and `entry.script_path`; the Rust function takes the same two inputs directly —
//!   `stored: &UvMetadata` is `entry.meta`, and `source: Option<&[u8]>` is the stored copy's bytes
//!   (`None` models a missing copy, the `script_path.exists()` guard).
//! - Python `store.update_dependencies(name, deps, requires_python)`: the None/[]/"" grammar for a
//!   python copy maps to the pure `plan_uv_metadata_edit(source, stored, deps, requires_python)`.
//!   The plan's `rewritten_source` is byte-for-byte what the store later writes to the copy's block
//!   (`skit-cli/src/cli.rs:3505` passes it as `UpdateEntry.source`), so a `contains` assertion on it
//!   IS the Python assertion on `_block("x")`. `plan.stored` is the meta the store persists.
//! - `UvMetadata.dependencies` is a plain `Vec<String>`, so Python's "meta.dependencies is None"
//!   maps to an empty vec (there is no Option to invent).
//!
//! Python's `effective_uv_metadata` guards on `kind == "python" and mode == "copy"` inside the
//! helper. Rust now keeps that same gate in `effective_entry_settings`, while the npm dependency
//! sweep and the `--python`-inapplicable refusal live in the CLI `deps` composition
//! (`skit-cli/src/cli.rs:3441`, `:3487`). The pure skit-language functions ported here know nothing
//! of storage adapters.
//!
//! Buckets (26 Python defs):
//! - REAL asserting (13): section 5's eight effective-read branches and section 4's five
//!   python-copy grammar cases that `plan_uv_metadata_edit` computes.
//! - CROSS-CRATE `#[ignore]` (13): the CLI end-to-end reads/writes (skit-cli-rs), the
//!   settings-screen prefill/diff (skit-ui), and the npm node_modules sweep (skit-cli-rs).

use skit_domain::{Entry, EntryKind, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_language::{
    UvMetadata, effective_entry_settings, effective_uv_metadata_bytes, plan_uv_metadata_edit,
};

/// A stored (meta) value with both axes set.
fn stored(dependencies: &[&str], requires_python: &str) -> UvMetadata {
    UvMetadata {
        dependencies: dependencies
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        requires_python: requires_python.to_owned(),
    }
}

fn entry(kind: &str, mode: StorageMode, settings: EntrySettings) -> Entry {
    let mut meta = EntryMeta::minimal("Demo", EntryKind::parse(kind).unwrap());
    meta.mode = mode;
    settings.write_to_meta(&mut meta);
    Entry {
        slug: Slug::parse("demo").unwrap(),
        meta,
    }
}

// ==========================================================================
// 1. HIGH-2 pilot end to end: block-only add-time deps + a python-only edit
//    keeps the deps AND gains the pin, reported on every surface.
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): drives the real CLI (`skit add --dep`, `skit deps --python`, \
`skit deps --json`) end to end. The dependency-axis grammar it proves is ported at the pure tier \
below (test_update_dependencies_none_python_lands_pin_and_preserves_block_deps); the CLI wiring \
that composes it belongs to skit-cli's `deps` command (skit-cli/src/cli.rs:3441)."]
fn test_add_dep_then_python_pin_keeps_block_deps_end_to_end() {
    // WHY: `skit add --dep` injects the dep into the copy's PEP 723 block and leaves meta blank;
    // `skit deps --python` then adds the pin WITHOUT erasing the block's dep. Both live in the
    // block uv reads, and `deps --json` reports both.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): asserts the `skit run --dry-run` launch command carries \
--python and --script. The launch-command projection lives in skit-cli's run path, not in the \
pure metadata functions."]
fn test_add_dep_then_python_pin_run_command_carries_both() {
    // WHY: the dry-run command shows the pin (--python) and `--script` (uv reads the surviving
    // `requests` straight from the block) — no --with, no dropped dependency.
}

// ==========================================================================
// 2. HIGH-1 pilot end to end (TUI): the settings screen prefills from the
//    block and diffs each axis against that baseline.
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-ui): the settings screen prefill-from-effective lives in \
skit-ui's SettingsInputs `effective_dependencies`/`effective_requires_python` \
(skit-ui/src/settings.rs:262). The effective read it prefills from is ported at the pure tier \
below (section 5)."]
fn test_settings_prefills_deps_and_python_from_the_block() {
    // WHY: a block-only add-time entry (meta blank) opens the ScriptSettingsScreen with #st-deps
    // AND #st-python prefilled from the block — not the empty strings raw meta would have shown.
}

#[test]
#[ignore = "CROSS-CRATE (skit-ui): the per-axis diff-against-baseline that makes an untouched \
axis travel as None lives in skit-ui's SettingsView::requires_python_edit \
(skit-ui/src/settings.rs:571)."]
fn test_settings_deps_only_edit_preserves_the_block_pin() {
    // WHY: editing ONLY the deps field must not unpin. #st-python was prefilled from the block, so
    // it equals its baseline and travels as None (no change) — the pin survives while the new dep
    // lands.
}

#[test]
#[ignore = "CROSS-CRATE (skit-ui): the settings-screen clear-what-you-see path lives in \
skit-ui's SettingsView::requires_python_edit, which returns Some(\"\") for a cleared field \
(skit-ui/src/settings.rs:571)."]
fn test_settings_clearing_python_on_block_only_entry_unpins() {
    // WHY: clearing #st-python (now visibly prefilled from the block) differs from its baseline, so
    // it travels explicitly and the save removes the block's requires-python line.
}

#[test]
#[ignore = "CROSS-CRATE (skit-ui + skit-store): an untouched save yielding None on both axes so the \
store update is never entered is a settings-surface + store contract (skit-ui/src/settings.rs:549); \
the pure-tier twin is test_update_dependencies_none_none_is_a_full_no_op."]
fn test_settings_untouched_save_never_touches_the_deps_axis() {
    // WHY: no edit to either field => both axes equal their baseline => pending_deps is None, so
    // the store dependency chokepoint is NEVER called (no unpin, no dep-wipe, no needless rewrite).
}

// ==========================================================================
// 3. Effective read views — human `deps`, `deps --json`, `show --json`
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the human `skit deps x` text projection lives in skit-cli's \
`deps` output (skit-cli/src/cli.rs write_deps). The effective read it prints is ported at the pure \
tier below (test_effective_block_only_reads_both_axes_from_the_block)."]
fn test_deps_read_human_reports_effective_block_only() {
    // WHY: `skit deps x` (human) reads EFFECTIVE metadata: a block-only entry prints its real dep
    // and pin, never the "—"/blank raw meta would have shown.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `--json` machine contract is produced by skit-cli's \
`deps` command; the effective read behind it is ported at the pure tier below."]
fn test_deps_read_json_reports_effective_block_only() {
    // WHY: `skit deps x --json` reports the effective dep and pin of a block-only entry.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the `skit show x --json` record is produced by skit-cli's \
`show` command; the effective read behind it is ported at the pure tier below."]
fn test_show_json_reports_effective_deps_for_block_only() {
    // WHY: `skit show x --json` reports the same effective metadata — the record must describe what
    // a run actually does, not the deliberately-blank meta.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the reference-mode read path through the CLI. The helper \
truth — meta-carried deps read straight from meta — is ported at the pure tier by \
test_effective_meta_carried_skips_the_block."]
fn test_deps_read_meta_carried_entry_is_unchanged() {
    // WHY: a meta-carried entry (reference mode records in meta) reads straight from meta — the
    // block fallback is a python-copy-only path and never fires here.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): `deps --json` printing the JS metadata is a CLI surface. \
The language-owned kind gate is executable in test_effective_js_entry_reads_meta_only."]
fn test_deps_read_js_entry_falls_through_to_meta() {
    // WHY: a non-python (npm) kind never reads a PEP 723 block — the helper returns meta verbatim,
    // with no constraint axis at all.
}

// ==========================================================================
// 4. store.update_dependencies — the None / [] / "" grammar on BOTH axes,
//    ported against the pure `plan_uv_metadata_edit`.
// ==========================================================================

#[test]
fn test_update_dependencies_none_none_is_a_full_no_op() {
    // WHY: (None, None): touch nothing. Meta unchanged on both axes AND the copy's block
    // byte-identical. At the pure tier: `plan.stored` equals the input meta and `rewritten_source`
    // is None, so the store writes no new block bytes.
    let source = br#"# /// script
# dependencies = ["requests"]
# requires-python = ">=3.11"
# ///
print(1)
"#;
    // A block-only add-time entry: meta is blank on both axes (deps_injected).
    let plan = plan_uv_metadata_edit(Some(source), &UvMetadata::default(), None, None).unwrap();
    assert_eq!(plan.stored, UvMetadata::default()); // meta unchanged (still blank both axes)
    assert!(plan.rewritten_source.is_none()); // block untouched, byte for byte
}

#[test]
fn test_update_dependencies_none_python_lands_pin_and_preserves_block_deps() {
    // WHY: (None, ">=3.12"): the deps axis is untouched, so the block's own `requests` is PRESERVED
    // while the pin lands — the store-unit form of the HIGH-2 fix.
    let source = br#"# /// script
# dependencies = ["requests"]
# ///
print(1)
"#;
    let plan = plan_uv_metadata_edit(
        Some(source),
        &UvMetadata::default(), // block-only deps, meta blank
        None,
        Some(">=3.12".to_owned()),
    )
    .unwrap();
    let block = String::from_utf8(plan.rewritten_source.expect("block rewritten")).unwrap();
    assert!(block.contains(r#""requests""#)); // deps axis preserved from the block
    assert!(block.contains(r#"requires-python = ">=3.12""#)); // pin landed
}

#[test]
fn test_update_dependencies_clear_deps_preserves_the_pin() {
    // WHY: ([], None) on a pinned python entry: an EXPLICIT deps clear empties the deps in meta AND
    // the block, but leaves the untouched constraint axis pinned.
    let source = br#"# /// script
# dependencies = ["requests"]
# requires-python = ">=3.11"
# ///
print(1)
"#;
    let plan = plan_uv_metadata_edit(
        Some(source),
        &UvMetadata::default(), // block-only, meta blank
        Some(Vec::new()),       // explicit clear
        None,
    )
    .unwrap();
    assert!(plan.stored.dependencies.is_empty()); // cleared in meta (Python: None)
    let block = String::from_utf8(plan.rewritten_source.expect("block rewritten")).unwrap();
    assert!(!block.contains(r#""requests""#)); // cleared in the block too
    assert!(block.contains(r#"requires-python = ">=3.11""#)); // the untouched pin SURVIVED
}

#[test]
fn test_update_dependencies_python_only_edit_syncs_block_from_meta_deps() {
    // WHY: the meta-carried branch of the block-deps derive rule: a copy whose source already had a
    // block records its deps in META. A python-only edit (deps=None) then syncs the block from
    // meta.dependencies — the LEFT side of `meta deps or block deps`.
    let source = br#"# /// script
# dependencies = []
# ///
print(1)
"#;
    let plan = plan_uv_metadata_edit(
        Some(source),
        &stored(&["requests"], ""), // existing block -> meta carries deps
        None,
        Some(">=3.13".to_owned()),
    )
    .unwrap();
    let block = String::from_utf8(plan.rewritten_source.expect("block rewritten")).unwrap();
    assert!(block.contains(r#""requests""#)); // block synced from meta deps, not wiped
    assert!(block.contains(r#"requires-python = ">=3.13""#));
}

#[test]
fn test_update_dependencies_missing_stored_copy_still_writes_meta() {
    // WHY: with the stored copy gone, a deps edit still persists meta and never crashes (the block
    // sync simply has nothing to write). At the pure tier: source=None => `rewritten_source` is None
    // while `plan.stored` carries the edit.
    let plan = plan_uv_metadata_edit(
        None, // stored copy gone
        &UvMetadata::default(),
        Some(vec!["rich".to_owned()]),
        None,
    )
    .unwrap();
    assert_eq!(plan.stored.dependencies, ["rich"]); // meta write survived the missing copy
    assert!(plan.rewritten_source.is_none()); // no block to write
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the npm node_modules sweep on an untouched (None) axis is a \
no-op because the CLI only sweeps on an explicit `[]` (`dependencies_edit.is_some_and(Vec::is_empty)` \
at skit-cli/src/cli.rs:3487, calling clear_javascript_dependencies). npm never routes through the \
pure python planner ported here."]
fn test_update_dependencies_npm_none_does_not_sweep_node_modules() {
    // WHY: (None) on an npm entry is UNTOUCHED — the node_modules sweep must not fire.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli-rs): the npm node_modules sweep on an explicit `[]` clear fires \
via clear_javascript_dependencies (skit-cli/src/cli.rs:3487-3492). npm never routes through the \
pure python planner ported here."]
fn test_update_dependencies_npm_clear_does_sweep_node_modules() {
    // WHY: ([]) on an npm entry is an EXPLICIT clear — the node_modules sweep DOES fire, the twin
    // of the None branch above.
}

// ==========================================================================
// 5. store.effective_uv_metadata — every branch of the read helper
//    (`effective_uv_metadata_bytes`).
// ==========================================================================

#[test]
fn test_effective_meta_carried_skips_the_block() {
    // WHY: both axes present in meta => the block fallback never runs (the `not deps or not
    // constraint` guard is False), and meta's values are returned verbatim. A decoy source block
    // with different values proves the fallback stayed shut.
    let decoy = br#"# /// script
# dependencies = ["decoy"]
# requires-python = ">=9.9"
# ///
print(1)
"#;
    assert_eq!(
        effective_uv_metadata_bytes(Some(decoy), &stored(&["requests"], ">=3.11")),
        stored(&["requests"], ">=3.11") // meta wins on both axes; the decoy block is ignored
    );
}

#[test]
fn test_effective_block_only_reads_both_axes_from_the_block() {
    // WHY: meta blank on both axes, copy-mode python => both deps and constraint come from the
    // block.
    let source = br#"# /// script
# dependencies = ["requests"]
# requires-python = ">=3.11"
# ///
print(1)
"#;
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &UvMetadata::default()),
        stored(&["requests"], ">=3.11")
    );
}

#[test]
fn test_effective_meta_deps_blank_constraint_reads_constraint_from_block() {
    // WHY: mixed split: meta carries deps but no pin => the deps axis stays from meta (`if not deps`
    // is False) while only the constraint is read from the block.
    let source = br#"# /// script
# requires-python = ">=3.9"
# ///
print(1)
"#;
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &stored(&["requests"], "")),
        stored(&["requests"], ">=3.9")
    );
}

#[test]
fn test_effective_meta_constraint_blank_deps_reads_deps_from_block() {
    // WHY: the mirror mix: meta carries the pin but no deps => the constraint stays from meta (`if
    // not constraint` is False) while only the deps come from the block.
    let source = br#"# /// script
# dependencies = ["rich"]
# ///
print(1)
"#;
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &stored(&[], ">=3.10")),
        stored(&["rich"], ">=3.10")
    );
}

#[test]
fn test_effective_both_blank_returns_empty() {
    // WHY: no deps, no pin, empty block => the helper returns the empty pair (block parse finds
    // nothing) — the display baseline that must read as "nothing set", not a crash.
    let source = b"print(1)\n"; // no block at all
    assert_eq!(
        effective_uv_metadata_bytes(Some(source), &UvMetadata::default()),
        UvMetadata::default()
    );
}

#[test]
fn test_effective_reference_mode_python_reads_meta_only() {
    // WHY: reference mode short-circuits the block read: even if the original file carries a block,
    // the helper reports meta — reference deps live only in meta.
    let settings = EntrySettings {
        dependencies: vec!["requests".to_owned()],
        requires_python: ">=3.11".to_owned(),
        ..EntrySettings::default()
    };
    let reference = entry("python", StorageMode::Reference, settings.clone());
    let decoy = br#"# /// script
# dependencies = ["decoy"]
# requires-python = ">=9.9"
# ///
"#;
    assert_eq!(effective_entry_settings(&reference, Some(decoy)), settings);
}

#[test]
fn test_effective_js_entry_reads_meta_only() {
    // WHY: a non-python kind fails the `kind == "python"` guard => meta verbatim, no block read.
    let settings = EntrySettings {
        dependencies: vec!["chalk".to_owned()],
        ..EntrySettings::default()
    };
    let javascript = entry("js", StorageMode::Copy, settings.clone());
    let decoy = br#"# /// script
# dependencies = ["decoy"]
# requires-python = ">=9.9"
# ///
"#;
    assert_eq!(effective_entry_settings(&javascript, Some(decoy)), settings);
}

#[test]
fn test_effective_missing_stored_copy_reads_meta_only() {
    // WHY: the `script_path.exists()` guard: a block-only entry whose copy is gone cannot be read,
    // so the helper reports the (blank) meta rather than crashing. At the pure tier a missing copy
    // is `source: None`.
    assert_eq!(
        effective_uv_metadata_bytes(None, &UvMetadata::default()),
        UvMetadata::default()
    );
}
