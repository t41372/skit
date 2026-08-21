//! Mechanical port of the Python oracle `tests/test_store.py`
//! (`/home/ubuntu/coding/skit-oracle/tests/test_store.py`, origin/main@206f9ef).
//!
//! The Python module tests one flat `skit.store` module that bundles THREE concerns the Rust
//! rewrite deliberately split apart:
//!   1. Add orchestration (`add_python`/`add_command`/`add_exe`/`add_script`/`add_prompt`): open the
//!      source file, hash it, infer the kind, extract the description from a docstring/comment,
//!      choose the stored filename, resolve the source path, and pick the default work directory.
//!      In the rewrite this lives in `skit-ui::add` / `skit-cli` (with kind inference and
//!      description extraction in `skit-language`). `skit-store::FileStore` only receives an
//!      already-prepared `CreateEntry` and is responsible for: atomic create, a byte-exact copy of
//!      the given payload, the payload's `source_hash`, slug allocation/dedup, name-conflict
//!      rejection, registry membership, and confinement to its own directories.
//!   2. Small helpers (`infer_kind`, `extract_comment_description`, `dir_size`, `human_size`) that
//!      are not `skit-store` symbols at all (skit-language / doctor-health / absent).
//!   3. The `registry.toml` projection: membership, list/resolve, corrupt-entry isolation, and the
//!      legacy-row fallback. THIS is the pure store contract, and the bulk of the ported tests.
//!
//! Two design facts of the Rust registry drive the bucketing:
//!   * `list` SELF-HEALS, `resolve` DOES NOT. `FileStore::scan` (the `list` path) projects the index
//!     and, for any row that fell back to its meta, opportunistically re-projects that row under a
//!     NON-BLOCKING lock (`FileStore::repair_rows`, the faithful translation of `_repair_rows` -- see
//!     A2). The index is a REBUILDABLE cache, not user data, and the repair re-derives each row from
//!     the meta as it is NOW, so this is a rebuild of a cache, not the lossy "read that migrates data"
//!     AGENTS.md guards against; the try-only lock keeps a read from ever blocking. `resolve` shares
//!     only the pure projection half (`scan_inner`) and never repairs, matching the oracle where
//!     `resolve` loads the index but never calls `_repair_rows` (`registry_resolve.rs` guards that no
//!     repair rides the resolve path). The `store._repair_rows` tests that drive the private window
//!     between staging and repair still have no public seam and stay `#[ignore]`d; the OBSERVABLE
//!     self-heal (a legacy/broken/hand-edited row is served from the meta and then the row is repaired
//!     once, converging on the next listing) IS ported and must pass.
//!   * THE CACHE IS CONTENT-HASHED. A registry row's fast-path proof (`skit_cache`) covers the
//!     metadata file's id, size, mtime, ctime, and a content hash — not mtime alone. So the v0.4
//!     "forge the mtime while corrupting the bytes and the listing still serves it" probe cannot
//!     hold: Rust re-verifies content and falls back (then self-heals the row).
//!
//! Bucketing legend in the `#[ignore]` reasons: "-> higher layer" = add orchestration
//! (skit-ui/cli/skit-language); "-> white-box" = a Python-internal (`_repair_rows`,
//! `_save_registry`, `_registry_row`, `_summary_from_row`) with no public seam -- e.g. an exact
//! staging/repair interleaving the public `list` cannot reproduce.
//!
//! FINDINGS the supervisor must read (flagged inline on the failing tests):
//!   * `test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes`: RESOLVED (S2). A listing (a
//!     READ) moves a corrupt `registry.toml` aside to `registry.toml.corrupt` inside the shared
//!     `Registry::read` chokepoint (the analog of `_load_registry`), preserving the bad bytes while
//!     the listing degrades to empty; `doctor --rebuild` reconstructs the index from the untouched
//!     metas. This is a faithful translation of the oracle, not a divergence.

use std::{fs, path::PathBuf, time::UNIX_EPOCH};

use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, EntryRepository, RepositoryError,
    SourcePermissions,
};
use skit_domain::{Entry, EntryKind, EntrySettings, StorageMode, parameters::ParamDecl};
use skit_store::{FileStore, RegistryRebuildProblem, content_hash};
use tempfile::TempDir;
use toml::{Table, Value};

// ---------------------------------------------------------------------------
// Harness helpers. Every test uses `tempfile::TempDir`, never a real user directory, matching the
// existing skit-store test harness (`tests/mutations.rs`, `tests/file_store.rs`).
// ---------------------------------------------------------------------------

/// A prepared copy-mode Python `CreateEntry` (the store slice of `store.add_python`).
fn python_copy(name: &str, bytes: &[u8], description: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Copy,
        source: format!("/original/{name}.py"),
        workdir: "invoke".to_owned(),
        description: description.to_owned(),
        payload: Some(EntryPayload {
            bytes: bytes.to_vec(),
            stored_name: Some("script.py".to_owned()),
            permissions: SourcePermissions {
                readonly: false,
                unix_mode: Some(0o644),
            },
        }),
        settings: EntrySettings::default(),
    }
}

/// A prepared copy-mode prompt `CreateEntry` (the store slice of `store.add_prompt`).
fn prompt_copy(name: &str, bytes: &[u8], description: &str) -> CreateEntry {
    let mut request = python_copy(name, bytes, description);
    request.kind = EntryKind::parse("prompt").unwrap();
    request.source = format!("/original/{name}.prompt.md");
    request.payload.as_mut().unwrap().stored_name = Some("prompt.md".to_owned());
    request
}

/// A prepared reference-mode Python `CreateEntry` (the store slice of `add_python(mode=reference)`).
fn python_reference(name: &str, source: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("python").unwrap(),
        mode: StorageMode::Reference,
        source: source.to_owned(),
        workdir: "origin".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings::default(),
    }
}

/// A prepared command `CreateEntry` (the store slice of `store.add_command`).
fn command_entry(name: &str, template: &str, description: &str) -> CreateEntry {
    let settings = EntrySettings {
        template: template.to_owned(),
        ..EntrySettings::default()
    };
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Reference,
        source: String::new(),
        workdir: "invoke".to_owned(),
        description: description.to_owned(),
        payload: None,
        settings,
    }
}

/// A prepared exe `CreateEntry` (the store slice of `store.add_exe`).
fn exe_reference(name: &str, source: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: EntryKind::parse("exe").unwrap(),
        mode: StorageMode::Reference,
        source: source.to_owned(),
        workdir: "origin".to_owned(),
        description: String::new(),
        payload: None,
        settings: EntrySettings::default(),
    }
}

fn registry_path(root: &TempDir) -> PathBuf {
    root.path().join("registry.toml")
}

fn meta_path(root: &TempDir, slug: &str) -> PathBuf {
    root.path().join("scripts").join(slug).join("meta.toml")
}

/// Read `registry.toml` as a document a test can inspect or edit.
fn read_registry(root: &TempDir) -> Table {
    toml::from_str(&fs::read_to_string(registry_path(root)).unwrap()).unwrap()
}

/// Write a whole `registry.toml` document, as a person or an older skit can.
fn write_registry(root: &TempDir, document: &Table) {
    fs::write(
        registry_path(root),
        toml::to_string_pretty(document).unwrap(),
    )
    .unwrap();
}

/// Replace the whole index with exactly these rows (the analog of `store._save_registry(rows)`).
fn replace_registry(root: &TempDir, rows: Vec<(&str, Value)>) {
    let mut entries = Table::new();
    for (slug, row) in rows {
        entries.insert(slug.to_owned(), row);
    }
    let mut document = Table::new();
    document.insert("entries".to_owned(), Value::Table(entries));
    write_registry(root, &document);
}

/// A pre-index legacy row that carries no mode/mtime, forcing the meta fallback.
fn legacy_row(name: &str, kind: &str, description: &str) -> Value {
    let mut row = Table::new();
    row.insert("name".to_owned(), Value::String(name.to_owned()));
    row.insert("kind".to_owned(), Value::String(kind.to_owned()));
    row.insert(
        "description".to_owned(),
        Value::String(description.to_owned()),
    );
    Value::Table(row)
}

// ===========================================================================
// add_python / basic store behavior (add-orchestration lives in skit-ui/cli; the store slices port)
// ===========================================================================

#[ignore = "UNMAPPED -> higher layer. `store.add_python` is add orchestration in skit-ui::add / \
            skit-cli: it opens the file, hashes it, extracts the docstring description \
            (skit-language::description), resolves the source path, and chooses the stored name. \
            skit-store only takes a prepared CreateEntry. Its store-level guarantees (byte-exact \
            stored copy + source_hash) are covered by mutations.rs::\
            create_is_atomic_mints_identity_and_preserves_payload_bytes and port_test_atomic.rs; \
            the docstring-description assertion has no store analog."]
#[test]
fn test_add_copy_preserves_original_verbatim() {}

#[test]
fn test_add_reference_points_to_origin() {
    // WHY (store slice): a reference add copies NO payload into the store and launches the origin
    // path. `payload_path` answers from the source; the entry directory holds no `script.py`.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("hello.py");
    fs::write(&source, "print(1)\n").unwrap();

    let entry = store
        .create(python_reference("hi", source.to_str().unwrap()))
        .unwrap();

    assert_eq!(entry.meta.mode, StorageMode::Reference);
    assert_eq!(store.payload_path(&entry).unwrap(), source);
    assert!(!store.entry_dir_path(&entry.slug).join("script.py").exists());
}

#[test]
fn test_name_conflict_rejected() {
    // WHY: the store refuses a second entry that claims a name already taken.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(python_copy("hello", b"print(1)\n", ""))
        .unwrap();

    let error = store
        .create(python_copy("hello", b"print(2)\n", ""))
        .unwrap_err();

    assert!(matches!(error, RepositoryError::Conflict { .. }));
}

#[test]
fn test_slug_dedup() {
    // WHY: two different display names each get a distinct, non-empty slug. (In v0.4 the CJK names
    // slugify to a colliding base and exercise dedup; the Rust slugifier keeps CJK as alphanumeric,
    // so they do not collide — the asserted outcome, two distinct non-empty slugs, still holds.)
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(python_copy("任務A", b"print(1)\n", ""))
        .unwrap();
    let second = store
        .create(python_copy("任務B", b"print(1)\n", ""))
        .unwrap();

    let entries = store.scan_entries().unwrap();
    assert_eq!(entries.len(), 2);
    let slugs: std::collections::BTreeSet<_> =
        entries.iter().map(|entry| entry.slug.as_str()).collect();
    assert_eq!(slugs.len(), 2);
    assert!(!second.slug.as_str().is_empty());
}

#[test]
fn test_resolve_and_remove() {
    // WHY: resolve by display name and by slug, then remove drops membership and the directory.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(python_copy("hi", b"print(1)\n", "")).unwrap();

    assert_eq!(store.resolve("hi").unwrap().slug, entry.slug);
    assert_eq!(store.resolve(entry.slug.as_str()).unwrap().meta.name, "hi");

    store.remove(&entry).unwrap();
    assert!(matches!(
        store.resolve("hi").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
    assert!(!store.entry_dir_path(&entry.slug).exists());
}

#[test]
fn test_remove_copy_does_not_touch_original() {
    // WHY: removal is confined to skit's own directory; the user's original file is never touched
    // (skit changes only its own data directories).
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let original = root.path().join("hello.py");
    fs::write(&original, "print(1)\n").unwrap();
    let mut request = python_copy("hi", b"print(1)\n", "");
    request.source = original.to_str().unwrap().to_owned();
    let entry = store.create(request).unwrap();

    store.remove(&entry).unwrap();

    assert!(original.exists());
}

#[ignore = "UNMAPPED -> higher layer. add_command's defaults (workdir=invoke via \
            skit-application::add_workdir, template placement) are add orchestration; skit-store \
            create only round-trips whatever settings/workdir it is given."]
#[test]
fn test_add_command_entry() {}

#[ignore = "UNMAPPED -> higher layer. The non-empty-template rule is enforced by the add-command \
            use case (skit-ui/cli); skit-store create accepts any settings and does not validate \
            the template."]
#[test]
fn test_command_requires_nonempty_template() {}

#[test]
fn test_doctor_rebuild_from_meta() {
    // WHY: with the index gone, a listing (registry-backed) is empty; `rebuild` reprojects every
    // valid meta and the listing recovers exactly those entries.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(python_copy("a", b"print(1)\n", "")).unwrap();
    store.create(command_entry("b", "echo hi", "")).unwrap();

    fs::remove_file(registry_path(&root)).unwrap();
    assert!(store.scan().unwrap().entries.is_empty());

    let report = store.rebuild_registry_report().unwrap();
    assert_eq!(report.entry_count, 2);
    assert!(report.problems.is_empty());

    let names: std::collections::BTreeSet<_> = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|summary| summary.name)
        .collect();
    assert_eq!(
        names,
        ["a".to_owned(), "b".to_owned()].into_iter().collect()
    );
}

#[test]
fn test_doctor_reports_missing_reference() {
    // WHY: rebuild reports a reference entry whose owned source path is gone, naming its slug and
    // the missing path (behavior, not locale copy).
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("ref.py");
    fs::write(&source, "print(1)\n").unwrap();
    let source = source.to_str().unwrap().to_owned();
    store.create(python_reference("ref", &source)).unwrap();
    fs::remove_file(&source).unwrap();

    let report = store.rebuild_registry_report().unwrap();

    assert!(report.problems.iter().any(|problem| matches!(
        problem,
        RegistryRebuildProblem::MissingReferenceSource { slug, path }
            if slug == "ref" && path == &source
    )));
}

#[ignore = "UNMAPPED -> higher layer. Leaving the description empty on a syntax error is \
            skit-language::description / skit-ui add resilience; skit-store stores whatever \
            description it is handed."]
#[test]
fn test_syntax_error_script_still_addable() {}

#[ignore = "UNMAPPED -> higher layer. Missing-source detection lives in the add use case that reads \
            the file (skit-ui/cli); skit-store create takes bytes and never opens the source path."]
#[test]
fn test_add_python_missing_file_raises() {}

#[ignore = "UNMAPPED -> higher layer. add_exe's forced reference mode and description passthrough \
            are add orchestration; skit-store create round-trips a prepared exe CreateEntry."]
#[test]
fn test_add_exe_roundtrip() {}

#[ignore = "UNMAPPED -> higher layer. The missing-source check is in the add use case, not a store \
            responsibility."]
#[test]
fn test_add_exe_missing_file_raises() {}

#[test]
fn test_list_entries_skips_corrupt_meta() {
    // WHY: the whole-entry directory scan silently skips a directory whose meta.toml is corrupt and
    // still returns the healthy entry.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store.create(command_entry("good", "echo hi", "")).unwrap();
    let bad_dir = root.path().join("scripts").join("bad-slug");
    fs::create_dir_all(&bad_dir).unwrap();
    fs::write(bad_dir.join("meta.toml"), "not valid toml [[[").unwrap();

    let entries = store.scan_entries().unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].meta.name, "good");
}

#[test]
fn test_doctor_rebuild_corrupt_meta() {
    // WHY: rebuild isolates a missing-meta directory and a corrupt-meta directory as per-directory
    // problems, projecting neither.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    fs::create_dir_all(root.path().join("scripts").join("orphan")).unwrap();
    let corrupt_dir = root.path().join("scripts").join("corrupt");
    fs::create_dir_all(&corrupt_dir).unwrap();
    fs::write(corrupt_dir.join("meta.toml"), "[[[bad").unwrap();

    let report = store.rebuild_registry_report().unwrap();

    assert_eq!(report.entry_count, 0);
    let slugs: Vec<&str> = report
        .problems
        .iter()
        .map(|problem| match problem {
            RegistryRebuildProblem::MissingMetadata { slug }
            | RegistryRebuildProblem::CorruptMetadata { slug, .. }
            | RegistryRebuildProblem::MissingReferenceSource { slug, .. } => slug.as_str(),
        })
        .collect();
    assert!(slugs.contains(&"orphan"));
    assert!(slugs.contains(&"corrupt"));
}

#[ignore = "UNMAPPED -> higher layer. Rewriting the copy's script body with a PEP 723 block is \
            skit-language injection orchestrated by skit-ui/cli; skit-store persists dependency \
            metadata in meta.toml (update_settings/update_entry) but never edits the script source. \
            Confirmed: no pep723/inject write path exists in skit-store/src."]
#[test]
fn test_update_dependencies_copy_mode() {}

#[test]
fn test_resolve_not_found_raises() {
    // WHY: a selector that matches nothing is a typed NotFound, not a crash.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());

    assert!(matches!(
        store.resolve("nonexistent").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

// ===========================================================================
// extract_comment_description — skit-language::description, not the store.
// ===========================================================================

// ===========================================================================
// add_script — the generic Tier-0 add orchestration (skit-ui/cli + skit-language).
// ===========================================================================

#[ignore = "UNMAPPED -> higher layer. add_script is add orchestration: comment-extracted \
            description (skit-language), workdir defaults (skit-application::add_workdir), and the \
            stored filename. The store-level byte-exact copy + source_hash are covered by \
            port_test_atomic.rs and mutations.rs."]
#[test]
fn test_add_script_copy_is_byte_identical_and_records_hash() {}

#[ignore = "UNMAPPED -> higher layer. add_script's reference workdir default (origin) and no-copy \
            are add orchestration; the store no-copy-on-reference slice is ported in \
            test_add_reference_points_to_origin."]
#[test]
fn test_add_script_reference_points_to_origin() {}

#[ignore = "UNMAPPED -> higher layer. The explicit-workdir override is an add-orchestration input; \
            skit-store round-trips whatever workdir it is given."]
#[test]
fn test_add_script_explicit_workdir_override() {}

#[ignore = "UNMAPPED -> higher layer. 'explicit description wins over comment extraction' is an \
            add-orchestration precedence rule; the store stores what it is handed."]
#[test]
fn test_add_script_explicit_name_and_description() {}

#[ignore = "UNMAPPED -> higher layer. Recording a passed interpreter is add orchestration; the \
            store round-trips settings.interpreter (mutations.rs create test)."]
#[test]
fn test_add_script_records_interpreter() {}

#[ignore = "UNMAPPED -> higher layer. The interpreted/copyable-kind allowlist is an add-use-case \
            rule (skit-ui/cli). skit-store keeps kinds OPEN-ENDED for v0.4 compatibility \
            (EntryKind::parse accepts any non-blank kind), so create does NOT reject 'martian'."]
#[test]
fn test_add_script_unknown_kind_raises() {}

#[ignore = "UNMAPPED -> higher layer. Same add-use-case allowlist; the store accepts 'exe' as an \
            open kind, so create does not reject it here."]
#[test]
fn test_add_script_non_interpreted_kind_raises() {}

#[ignore = "UNMAPPED -> higher layer. The missing-source check is in the add use case that reads \
            the file; not a store responsibility."]
#[test]
fn test_add_script_missing_file_raises() {}

#[ignore = "UNMAPPED -> higher layer. The '--' comment description and the stored filename are add \
            orchestration (skit-language + skit-ui/cli)."]
#[test]
fn test_add_script_lua_uses_double_dash_description() {}

// ===========================================================================
// list_summaries — the listing view, served from the index (the pure store contract).
// ===========================================================================

#[test]
fn test_summaries_match_full_entries_field_for_field() {
    // WHY: the index projection and the meta agree on every listed field, so `skit list` and
    // `skit show` cannot disagree. (Rust's EntrySummary carries `target: Option<String>` — Some for
    // a reference entry, None for a copy — where v0.4 used "" for copy; it has no `script_path`
    // field, which is a launcher/library_surface projection, so the field-for-field parity is
    // asserted on name/kind/mode/description/target.)
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let linked_source = root.path().join("linked.py");
    fs::write(&linked_source, "print(1)\n").unwrap();
    let binary_source = root.path().join("tool");
    fs::write(&binary_source, b"binary").unwrap();

    store
        .create(python_copy("copied", b"print(1)\n", "a copy"))
        .unwrap();
    store
        .create(python_reference("linked", linked_source.to_str().unwrap()))
        .unwrap();
    store
        .create(command_entry("templated", "echo hi", "no file"))
        .unwrap();
    store
        .create(exe_reference("binary", binary_source.to_str().unwrap()))
        .unwrap();

    let by_slug: std::collections::BTreeMap<String, Entry> = store
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.slug.as_str().to_owned(), entry))
        .collect();
    let summaries = store.scan().unwrap().entries;

    let listed_slugs: Vec<String> = summaries
        .iter()
        .map(|summary| summary.slug.as_str().to_owned())
        .collect();
    let sorted_slugs: Vec<String> = by_slug.keys().cloned().collect();
    assert_eq!(listed_slugs, sorted_slugs);

    for summary in &summaries {
        let meta = &by_slug[summary.slug.as_str()].meta;
        assert_eq!(summary.name, meta.name);
        assert_eq!(summary.kind, meta.kind);
        assert_eq!(summary.mode, meta.mode);
        assert_eq!(summary.description, meta.description);
        let expected_target = if meta.mode == StorageMode::Reference {
            Some(meta.source.clone())
        } else {
            None
        };
        assert_eq!(summary.target, expected_target);
    }
}

#[ignore = "UNMAPPED -> white-box + design. The Rust fast-path proof is content-hashed (CacheProof: \
            file id, size, mtime, ctime, and a metadata content hash), not mtime-only. Forging the \
            mtime while corrupting the bytes does NOT keep the cache valid — Rust re-verifies \
            content and falls back, so the entry drops rather than serving stale bytes. The genuine \
            fast path (a verified cache hit skips the authoritative read) is proven by \
            read.rs::a_verified_cache_hit_does_not_call_the_authoritative_reader. FLAG: Rust's proof \
            is strictly stronger than v0.4's mtime cache, not a regression."]
#[test]
fn test_summaries_serve_from_the_index_without_parsing_metas() {}

#[ignore = "UNMAPPED -> white-box. Asserts the internal store._registry_row projection and reads \
            summary.script_path (a launcher/library_surface projection with no EntrySummary field). \
            The Rust read DOES repair the legacy row (FileStore::repair_rows), and that observable \
            self-heal is ported in test_an_older_registry_is_widened_the_first_time_it_is_listed; \
            the legacy-row fallback is ported in test_a_renamed_legacy_row_is_upgraded_not_patched \
            and test_a_hand_broken_row_falls_back_instead_of_inventing_a_summary."]
#[test]
fn test_a_row_an_older_skit_wrote_falls_back_to_its_meta() {}

#[test]
fn test_a_hand_broken_row_falls_back_instead_of_inventing_a_summary() {
    // WHY: a row a newer skit could not have written (non-string field, unknown mode, missing
    // field, non-string target) is never coerced into a summary — the meta answers instead.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("real", b"print(1)\n", "the truth"))
        .unwrap();
    let slug = entry.slug.as_str();

    let non_string_field = {
        let mut row = Table::new();
        row.insert("name".to_owned(), Value::String("x".to_owned()));
        row.insert("kind".to_owned(), Value::String("python".to_owned()));
        row.insert("description".to_owned(), Value::Integer(7));
        Value::Table(row)
    };
    let unknown_mode = {
        let mut row = Table::new();
        row.insert("name".to_owned(), Value::String("x".to_owned()));
        row.insert("kind".to_owned(), Value::String("python".to_owned()));
        row.insert("mode".to_owned(), Value::String("sideways".to_owned()));
        row.insert("description".to_owned(), Value::String(String::new()));
        Value::Table(row)
    };
    let missing_field = {
        let mut row = Table::new();
        row.insert("kind".to_owned(), Value::String("python".to_owned()));
        row.insert("description".to_owned(), Value::String(String::new()));
        Value::Table(row)
    };
    let non_string_target = {
        let mut row = Table::new();
        row.insert("name".to_owned(), Value::String("x".to_owned()));
        row.insert("kind".to_owned(), Value::String("python".to_owned()));
        row.insert("description".to_owned(), Value::String(String::new()));
        row.insert("mode".to_owned(), Value::String("reference".to_owned()));
        row.insert("target".to_owned(), Value::Integer(7));
        Value::Table(row)
    };

    for row in [
        non_string_field,
        unknown_mode,
        missing_field,
        non_string_target,
    ] {
        replace_registry(&root, vec![(slug, row)]);
        let summaries = store.scan().unwrap().entries;
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].name, "real");
        assert_eq!(summaries[0].description, "the truth");
    }
}

#[test]
fn test_a_broken_row_over_a_corrupt_meta_is_skipped_like_list_entries() {
    // WHY: a fallback that reaches a meta which is itself corrupt skips the entry, the same answer
    // the whole-entry scan gives, never a crash.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("doomed", b"print(1)\n", ""))
        .unwrap();
    let mut row = Table::new();
    row.insert("name".to_owned(), Value::String("doomed".to_owned()));
    replace_registry(&root, vec![(entry.slug.as_str(), Value::Table(row))]);
    fs::write(meta_path(&root, entry.slug.as_str()), "not [ toml").unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    assert!(store.scan_entries().unwrap().is_empty());
}

#[test]
fn test_rename_and_describe_keep_the_index_in_step() {
    // WHY: the two fields that change after add stay in step with the index, so `skit list` never
    // shows a stale name/description.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("before", b"print(1)\n", "old text"))
        .unwrap();
    let renamed = store.rename(&entry, "after").unwrap();
    store.describe(&renamed, "new text").unwrap();

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "after");
    assert_eq!(summaries[0].description, "new text");
}

#[test]
fn test_an_older_registry_is_widened_the_first_time_it_is_listed() {
    // WHY (A2 headline): a library an older skit wrote carries pre-index rows (name/kind/description,
    // no mode/mtime/proof), so every listing would fall back to reading each meta forever -- the
    // index gains the new fields only on add/rename/describe otherwise. The first listing re-projects
    // the row from the meta under the try-lock (self-healing), so nobody must know to run
    // `doctor --rebuild` after upgrading; the second listing is index-served and rewrites nothing.
    // Faithful translation of the oracle's `_repair_rows`, staged by `list_summaries`.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("legacy", b"print(1)\n", "old row"))
        .unwrap();
    let slug = entry.slug.as_str().to_owned();
    replace_registry(
        &root,
        vec![(&slug, legacy_row("legacy", "python", "old row"))],
    );

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "legacy");

    // Widened: the row now carries the fields a pre-index row lacked (mode, mtime_ns, a fresh cache
    // proof), re-derived from the meta -- not left as the legacy shape.
    let widened = read_registry(&root);
    let row = widened
        .get("entries")
        .unwrap()
        .get(&slug)
        .unwrap()
        .as_table()
        .unwrap();
    assert_eq!(row.get("mode").and_then(Value::as_str), Some("copy"));
    assert!(row.contains_key("mtime_ns"));
    assert!(row.contains_key("skit_cache"));

    // Converged: a second listing is served from the index and rewrites nothing.
    let converged = fs::read(registry_path(&root)).unwrap();
    assert_eq!(store.scan().unwrap().entries.len(), 1);
    assert_eq!(fs::read(registry_path(&root)).unwrap(), converged);
}

#[ignore = "UNMAPPED -> white-box. Drives the exact staging/repair window: stage a slug, commit a \
            concurrent add, then run _repair_rows and assert the raced row survived. The Rust \
            self-heal EXISTS (FileStore::repair_rows) but re-reads the index under the lock and \
            touches only the staged slugs, so the raced row is never in its working set; the public \
            `list` cannot reproduce that precise interleaving. The lock-and-re-derive discipline is \
            proven by test_an_older_registry_is_widened_the_first_time_it_is_listed and \
            test_a_reference_row_that_lost_its_target_is_repaired_once."]
#[test]
fn test_repair_never_drops_an_entry_added_meanwhile() {}

#[ignore = "UNMAPPED -> white-box. Same private staging/repair window as above: a slug removed after \
            staging is skipped (never resurrected) because repair_rows re-checks membership under the \
            lock; the public `list` has no seam to force that exact interleaving."]
#[test]
fn test_repair_skips_an_entry_removed_meanwhile() {}

#[cfg(unix)]
#[test]
fn test_a_store_that_cannot_be_written_still_lists() {
    // WHY: the self-heal is a side effect of a READ. A read-only store, or one another process is
    // mid-write on, must still answer `skit list` -- never fail on an index it does not depend on.
    // The read-path repair swallows its save error (a read must not fail because its optional write
    // could not land). Ported on unix by staging a stale legacy row, pre-creating the lock file (so
    // the try-lock succeeds), then making the data dir read-only so the repair's atomic save fails.
    use std::os::unix::fs::PermissionsExt as _;

    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("legacy", b"print(1)\n", "old row"))
        .unwrap();
    replace_registry(
        &root,
        vec![(
            entry.slug.as_str(),
            legacy_row("legacy", "python", "old row"),
        )],
    );
    // Pre-create the lock file so the try-lock is exercised (not the read-only-dir open failure),
    // isolating the save-failure swallow the oracle's monkeypatch tests.
    fs::write(root.path().join("registry.native.lock"), [0]).unwrap();
    let before = fs::read(registry_path(&root)).unwrap();
    let original = fs::metadata(root.path()).unwrap().permissions();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o555)).unwrap();

    let summaries = store.scan().unwrap().entries; // the repair's save fails; the read still answers

    fs::set_permissions(root.path(), original).unwrap(); // restore before TempDir drop
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "legacy");
    assert_eq!(summaries[0].mode, StorageMode::Copy);
    // The failed save left the index untouched -- the atomic writer's temp creation fails before the
    // target is touched, so no partial write and no leaked `.tmp` residue beside it.
    assert_eq!(
        fs::read(registry_path(&root)).unwrap(),
        before,
        "the swallowed save did not partially rewrite the index"
    );
    let residue: Vec<String> = fs::read_dir(root.path())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp"))
        .collect();
    assert!(
        residue.is_empty(),
        "no partial commit beside the index: {residue:?}"
    );
}

#[test]
fn test_a_corrupt_index_lists_nothing_and_preserves_the_bad_bytes() {
    // WHY: registry.toml is a rebuildable index; a corrupt one lists nothing and rebuilds cleanly.
    // Faithful translation of the oracle: `Registry::read` (the chokepoint `scan` and `resolve`
    // share, like `_load_registry`) moves the unparseable bytes aside to `registry.toml.corrupt`
    // during the read, so the bad bytes are preserved for inspection while the listing degrades to
    // empty. `doctor --rebuild` reconstructs the index from the untouched metas.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(python_copy("doomed", b"print(1)\n", ""))
        .unwrap();
    fs::write(registry_path(&root), "entries = [ this is not toml").unwrap();

    assert!(store.scan().unwrap().entries.is_empty());
    assert!(
        registry_path(&root)
            .with_file_name("registry.toml.corrupt")
            .exists()
    );
    assert_eq!(store.rebuild_registry_report().unwrap().entry_count, 1);
    let names: Vec<String> = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|summary| summary.name)
        .collect();
    assert_eq!(names, ["doomed".to_owned()]);
}

#[ignore = "UNMAPPED -> higher layer. add_exe's forced reference mode plus DirectLaunch \
            target(spec_for('exe')) is add orchestration + the launcher; the store-level launch \
            target state is tested through LibraryDetailRepository."]
#[test]
fn test_exe_is_always_reference_mode() {}

#[test]
fn test_an_entry_whose_meta_is_gone_is_not_listed() {
    // WHY: an entry whose storage was removed out from under the index is dropped by BOTH the
    // index-served listing and the whole-entry scan — the faces agree — and the reference original
    // is untouched. (The reference case is the one that bites: its launch target still exists.)
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("linked.py");
    fs::write(&source, "print(1)\n").unwrap();
    let linked = store
        .create(python_reference("linked", source.to_str().unwrap()))
        .unwrap();
    store.create(command_entry("kept", "echo hi", "")).unwrap();

    fs::remove_dir_all(store.entry_dir_path(&linked.slug)).unwrap();

    let listed: Vec<String> = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|summary| summary.name)
        .collect();
    assert_eq!(listed, ["kept".to_owned()]);
    let scanned: Vec<String> = store
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.meta.name)
        .collect();
    assert_eq!(scanned, ["kept".to_owned()]);
    assert!(source.exists());
}

#[test]
fn test_a_corrupted_meta_drops_out_of_the_listing_like_every_other_face() {
    // WHY: breaking a meta invalidates the row's content proof, the listing falls back, the parse
    // fails, and the entry is skipped — exactly what the whole-entry scan does.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let broken = store
        .create(python_copy("broken", b"print(1)\n", ""))
        .unwrap();
    store.create(command_entry("fine", "echo hi", "")).unwrap();
    fs::write(meta_path(&root, broken.slug.as_str()), "not [ toml").unwrap();

    let listed: Vec<String> = store
        .scan()
        .unwrap()
        .entries
        .into_iter()
        .map(|summary| summary.name)
        .collect();
    assert_eq!(listed, ["fine".to_owned()]);
    let scanned: Vec<String> = store
        .scan_entries()
        .unwrap()
        .into_iter()
        .map(|entry| entry.meta.name)
        .collect();
    assert_eq!(scanned, ["fine".to_owned()]);
}

#[test]
fn test_a_non_mapping_row_falls_back_instead_of_crashing() {
    // WHY: registry.toml is a file a person can edit, so a row may be a scalar. A listing degrades
    // into the meta, never dies.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("real", b"print(1)\n", "the truth"))
        .unwrap();
    replace_registry(
        &root,
        vec![(entry.slug.as_str(), Value::String("oops".to_owned()))],
    );

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "real");
    assert_eq!(summaries[0].description, "the truth");
}

#[ignore = "UNMAPPED -> design. The self-heal exists and DOES converge (save-only-if-changed), but \
            Rust models `mode` as a strict enum, so a hand-edited mode='sideways' meta makes \
            read_entry FAIL -- the row is never staged (dropped as an isolated corrupt entry, \
            siblings kept), not served as a passthrough string that keeps re-staging. The \
            convergence property this probes is proven by \
            test_an_older_registry_is_widened_the_first_time_it_is_listed and \
            test_a_reference_row_that_lost_its_target_is_repaired_once."]
#[test]
fn test_widening_gives_up_on_a_row_it_would_reject_again() {}

#[ignore = "UNMAPPED -> white-box. Drives the private staging/repair window: a rename lands after \
            staging, and _repair_rows must re-derive from the meta AS IT IS NOW (keeping the rename) \
            rather than writing the listing's snapshot. FileStore::repair_rows does exactly that \
            (re-reads each meta under the lock), but the public `list` cannot force that exact \
            interleaving; the re-derive-under-the-lock discipline is proven by \
            test_a_reference_row_that_lost_its_target_is_repaired_once."]
#[test]
fn test_repair_keeps_a_rename_that_landed_meanwhile() {}

#[ignore = "UNMAPPED -> white-box. Same private window with the nastiest interleaving: an older skit \
            reuses the slug for a new meta before the repair runs. repair_rows re-derives from the \
            slug's meta as it is NOW, so the new entry gets a correct row -- but the public `list` \
            has no seam to stage the old entry's slug then swap the meta underneath it."]
#[test]
fn test_repair_adopts_a_slug_reused_by_an_older_skit_meanwhile() {}

#[test]
fn test_a_renamed_legacy_row_is_upgraded_not_patched() {
    // WHY: renaming an entry that still carries a pre-index legacy row re-projects the WHOLE row
    // (not just `name`), so the listing reports the new name AND the correct reference mode — a
    // reference entry is never left pointed at a store path it does not use.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("linked.py");
    fs::write(&source, "print(1)\n").unwrap();
    let entry = store
        .create(python_reference("linked", source.to_str().unwrap()))
        .unwrap();
    replace_registry(
        &root,
        vec![(entry.slug.as_str(), legacy_row("linked", "python", ""))],
    );

    store.rename(&entry, "renamed").unwrap();

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "renamed");
    assert_eq!(summaries[0].mode, StorageMode::Reference);
    // (v0.4 also asserts summary.script_path; that is a launcher/library_surface projection with no
    // EntrySummary field in the rewrite.)
}

#[test]
fn test_a_reference_row_without_a_target_falls_back_to_its_meta() {
    // WHY: `target` is checked by PRESENCE. A reference row whose `target` key a hand edit removed
    // cannot say where the script is, so the listing must fall back to the meta and re-derive the
    // target rather than defaulting it to "" (which would resolve to the current directory and
    // report a deleted original as healthy). (v0.4 also asserts launcher.target_missing; that is a
    // launcher projection off the store surface.)
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("orig.py");
    fs::write(&source, "print(1)\n").unwrap();
    let source = source.to_str().unwrap().to_owned();
    let entry = store.create(python_reference("linked", &source)).unwrap();
    fs::remove_file(&source).unwrap();

    let mut document = read_registry(&root);
    document
        .get_mut("entries")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .get_mut(entry.slug.as_str())
        .unwrap()
        .as_table_mut()
        .unwrap()
        .remove("target");
    write_registry(&root, &document);

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].target.as_deref(), Some(source.as_str()));
}

#[test]
fn test_a_command_row_keeps_an_empty_target() {
    // WHY: a command template legitimately has no file target, so its row carries `target = ""`
    // rather than omitting the key; the listing keeps trusting that empty target and never restages
    // (the fresh row is a cache hit, so it is served from the index and the self-heal never fires).
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store.create(command_entry("tmpl", "echo hi", "")).unwrap();

    let document = read_registry(&root);
    let target = document
        .get("entries")
        .unwrap()
        .get(entry.slug.as_str())
        .unwrap()
        .get("target")
        .unwrap();
    assert_eq!(target.as_str(), Some(""));

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].mode, StorageMode::Reference);
    assert_eq!(summaries[0].target.as_deref(), Some(""));

    let before = fs::read(registry_path(&root)).unwrap();
    store.scan().unwrap();
    assert_eq!(fs::read(registry_path(&root)).unwrap(), before);
}

#[test]
fn test_a_hand_edited_meta_shows_up_on_the_next_listing() {
    // WHY: meta.toml is a file users hand edit; the row's content proof keeps the listing honest.
    // The edit invalidates the proof, the listing falls back, serves the truth, AND repairs the row
    // from that truth (oracle's third act), so the next listing is index-served -- `list` and `show`
    // never disagree for longer than one listing.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("job", b"print(1)\n", "the old text"))
        .unwrap();
    let slug = entry.slug.as_str().to_owned();

    let first = store.scan().unwrap().entries;
    assert_eq!(first[0].description, "the old text");

    let mut meta: Table =
        toml::from_str(&fs::read_to_string(meta_path(&root, &slug)).unwrap()).unwrap();
    meta.insert(
        "description".to_owned(),
        Value::String("edited by hand".to_owned()),
    );
    fs::write(
        meta_path(&root, &slug),
        toml::to_string_pretty(&meta).unwrap(),
    )
    .unwrap();

    let second = store.scan().unwrap().entries;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].description, "edited by hand");

    // ...and the self-heal stamped the new text into the row, so the third listing is index-served
    // and rewrites nothing (converged).
    let repaired = read_registry(&root);
    assert_eq!(
        repaired
            .get("entries")
            .unwrap()
            .get(&slug)
            .unwrap()
            .get("description")
            .unwrap()
            .as_str(),
        Some("edited by hand")
    );
    let converged = fs::read(registry_path(&root)).unwrap();
    let third = store.scan().unwrap().entries;
    assert_eq!(third[0].description, "edited by hand");
    assert_eq!(fs::read(registry_path(&root)).unwrap(), converged);
}

#[test]
fn test_a_listing_never_blocks_on_the_registry_lock() {
    // WHY: the self-heal rides on read paths (`skit list`, shell completion), so it must never block
    // on the cross-process registry lock; its lock is TRY-only. A STALE legacy row forces the
    // self-heal to fire, then the held native lock makes the try-lock decline: the listing still
    // answers (never deadlocks) and simply skips the repair -- registry.toml is left unchanged.
    // Dropping the lock lets the next listing repair the row.
    use std::fs::OpenOptions;

    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("legacy", b"print(1)\n", "old"))
        .unwrap();
    // A pre-index legacy row (no mode/mtime/proof): the listing cannot serve it from the index, so
    // it falls back to the meta and stages the row for the self-heal.
    replace_registry(
        &root,
        vec![(entry.slug.as_str(), legacy_row("legacy", "python", "old"))],
    );

    let lock_path = root.path().join("registry.native.lock");
    let held = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .unwrap();
    held.set_len(1).unwrap();
    held.lock().unwrap();
    let before = fs::read(registry_path(&root)).unwrap();

    // Must return, not deadlock, while the native lock is held elsewhere; the repair is declined.
    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].name, "legacy");
    assert_eq!(
        fs::read(registry_path(&root)).unwrap(),
        before,
        "the try-lock declined, so the self-heal did not rewrite the index"
    );

    // Released: the next listing takes the try-lock and repairs the stale row.
    drop(held);
    store.scan().unwrap();
    let repaired = read_registry(&root);
    let row = repaired
        .get("entries")
        .unwrap()
        .get(entry.slug.as_str())
        .unwrap()
        .as_table()
        .unwrap();
    assert!(row.contains_key("mtime_ns"));
    assert!(row.contains_key("skit_cache"));
}

#[test]
fn test_a_reference_row_that_lost_its_target_is_repaired_once() {
    // WHY: the convergence contract on the row shape that used to defeat it -- a reference row whose
    // `target` a hand edit removed. It cannot say where the script is, so the listing falls back to
    // the meta AND the self-heal re-derives the whole row (regaining `target`); the second listing is
    // index-served and rewrites nothing. Faithful translation of the oracle's repaired-once test.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("linked.py");
    fs::write(&source, "print(1)\n").unwrap();
    let source = source.to_str().unwrap().to_owned();
    let entry = store.create(python_reference("linked", &source)).unwrap();

    let mut document = read_registry(&root);
    document
        .get_mut("entries")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .get_mut(entry.slug.as_str())
        .unwrap()
        .as_table_mut()
        .unwrap()
        .remove("target");
    write_registry(&root, &document);

    let first = store.scan().unwrap().entries; // fallback: the row cannot say where the script is
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].target.as_deref(), Some(source.as_str()));
    // Repaired: the row regained its `target` from the meta.
    let repaired = read_registry(&root);
    assert_eq!(
        repaired
            .get("entries")
            .unwrap()
            .get(entry.slug.as_str())
            .unwrap()
            .get("target")
            .unwrap()
            .as_str(),
        Some(source.as_str())
    );
    // Converged: the second listing is served from the index and rewrites nothing.
    let converged = fs::read(registry_path(&root)).unwrap();
    let second = store.scan().unwrap().entries;
    assert_eq!(second[0].target.as_deref(), Some(source.as_str()));
    assert_eq!(fs::read(registry_path(&root)).unwrap(), converged);
}

#[test]
fn test_an_emptied_target_on_a_file_kind_falls_back_to_the_meta() {
    // WHY: the presence check guards a DELETED key; this guards an emptied VALUE. For a kind with a
    // file to launch, `target = ""` would resolve to the current directory (which exists), so a
    // deleted original could list as healthy. The listing must not trust it — it falls back and
    // re-derives the real target from the meta. (v0.4 also asserts launcher.target_missing, off the
    // store surface.)
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let source = root.path().join("orig.py");
    fs::write(&source, "print(1)\n").unwrap();
    let source = source.to_str().unwrap().to_owned();
    let entry = store.create(python_reference("linked", &source)).unwrap();
    fs::remove_file(&source).unwrap();

    let mut document = read_registry(&root);
    document
        .get_mut("entries")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .get_mut(entry.slug.as_str())
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("target".to_owned(), Value::String(String::new()));
    write_registry(&root, &document);

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].target.as_deref(), Some(source.as_str()));
}

#[test]
fn test_resolve_survives_a_hand_broken_row() {
    // WHY: hand-edited-registry tolerance must reach `resolve`, not just listing: a scalar junk row
    // must not crash resolve, real names around it still resolve, and the junk row matches nothing.
    // The scalar row for slug `stray` counts as membership (Registry::contains keys on presence), so
    // `resolve("stray")` reads its (missing) entry dir; the resolve fix (commit 2aebe6f) maps that
    // missing/corrupt meta to NotFound rather than Io, matching v0.4's NotFoundError degradation.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("real", b"print(1)\n", ""))
        .unwrap();

    let mut document = read_registry(&root);
    document
        .get_mut("entries")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert("stray".to_owned(), Value::String("oops".to_owned()));
    write_registry(&root, &document);

    assert_eq!(store.resolve("real").unwrap().slug, entry.slug);
    assert_eq!(store.resolve(entry.slug.as_str()).unwrap().slug, entry.slug);
    assert!(matches!(
        store.resolve("stray").unwrap_err(),
        RepositoryError::NotFound { .. }
    ));
}

#[test]
fn test_a_fresh_stamped_row_with_broken_fields_falls_back() {
    // WHY: defense in depth past the freshness gate — a row whose stamp matches but whose fields a
    // hand edit broke (a non-string description) still falls back to the meta rather than inventing
    // a summary or crashing. (v0.4 also calls the internal store._summary_from_row directly with a
    // scalar; that is a white-box helper with no public Rust surface — the normalize-at-the-
    // chokepoint guarantee it defends is covered by the fallback asserted here.)
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("real", b"print(1)\n", "the truth"))
        .unwrap();
    let slug = entry.slug.as_str();

    let modified = fs::metadata(meta_path(&root, slug))
        .unwrap()
        .modified()
        .unwrap();
    let mtime_ns = i64::try_from(modified.duration_since(UNIX_EPOCH).unwrap().as_nanos()).unwrap();
    let mut broken = Table::new();
    broken.insert("name".to_owned(), Value::String("real".to_owned()));
    broken.insert("kind".to_owned(), Value::String("python".to_owned()));
    broken.insert("description".to_owned(), Value::Integer(7));
    broken.insert("mtime_ns".to_owned(), Value::Integer(mtime_ns));
    replace_registry(&root, vec![(slug, Value::Table(broken))]);

    let summaries = store.scan().unwrap().entries;
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].description, "the truth");
}

#[test]
fn test_an_index_whose_entries_key_is_not_a_table_reads_empty() {
    // WHY: `entries` itself hand-edited into a scalar reads as an empty index (doctor rebuilds it)
    // instead of every consumer crashing.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    store
        .create(python_copy("real", b"print(1)\n", ""))
        .unwrap();

    let mut document = Table::new();
    document.insert("entries".to_owned(), Value::Integer(5));
    write_registry(&root, &document);

    assert!(store.scan().unwrap().entries.is_empty());
    assert_eq!(store.rebuild_registry_report().unwrap().entry_count, 1);
}

#[ignore = "UNMAPPED -> white-box. FileStore::repair_rows IS best-effort -- it skips a slug whose \
            meta corrupted since staging (`read_entry` Err -> continue) and does not write a row a \
            re-read cannot produce -- but driving the exact stage-then-corrupt window needs the \
            private _repair_rows seam the public `list` has no way to reproduce."]
#[test]
fn test_repair_skips_a_meta_that_broke_or_went_unrepresentable_meanwhile() {}

// ===========================================================================
// Every meta write keeps its own index row fresh (the pure store contract).
// ===========================================================================

#[test]
fn test_add_survives_a_hand_broken_row_that_can_claim_no_name() {
    // WHY: `add` cross-checks the index for a name conflict, and registry.toml is a file a person
    // can edit: a scalar row and a row whose `name` is a number both claim no name. Reading the key
    // blindly would make every `skit add` crash; instead such rows are skipped, the add succeeds,
    // and the real names around the junk still conflict.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store
        .create(python_copy("real", b"print(1)\n", ""))
        .unwrap();

    let mut document = read_registry(&root);
    {
        let entries = document.get_mut("entries").unwrap().as_table_mut().unwrap();
        entries.insert("bad".to_owned(), Value::String("garbage".to_owned()));
        let mut numeric = Table::new();
        numeric.insert("name".to_owned(), Value::Integer(7));
        numeric.insert("kind".to_owned(), Value::String("python".to_owned()));
        numeric.insert("description".to_owned(), Value::String(String::new()));
        entries.insert("numeric".to_owned(), Value::Table(numeric));
    }
    write_registry(&root, &document);

    let added = store
        .create(command_entry("newcmd", "echo hi", ""))
        .unwrap();
    assert_eq!(store.resolve("newcmd").unwrap().slug, added.slug);
    assert_eq!(store.resolve("real").unwrap().slug, good.slug);
    assert!(matches!(
        store
            .create(command_entry("real", "echo hi", ""))
            .unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
}

#[test]
fn test_an_entry_whose_row_was_mangled_still_defends_its_name() {
    // WHY: a REAL entry (directory and meta intact) whose registry row a hand edit turned into a
    // scalar must still defend its display name — the untrusted row falls through to the meta via
    // the directory sweep, so a colliding add is refused instead of producing two entries with one
    // name.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let good = store
        .create(python_copy("Guarded Name", b"print(1)\n", ""))
        .unwrap();

    let mut document = read_registry(&root);
    document
        .get_mut("entries")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .insert(
            good.slug.as_str().to_owned(),
            Value::String("garbage".to_owned()),
        );
    write_registry(&root, &document);

    assert!(matches!(
        store
            .create(command_entry("Guarded Name", "echo hi", ""))
            .unwrap_err(),
        RepositoryError::Conflict { .. }
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn test_a_meta_mutator_leaves_a_row_the_next_listing_serves_untouched() {
    // WHY: every meta write re-projects the row with a fresh content proof, so the next listing is
    // served from the index (the fresh row is a cache hit, so the self-heal never fires and
    // registry.toml is unchanged across the read) and still reflects what the mutation changed.
    // Proven by binding the row's cache proof to the new meta bytes and by byte-comparing
    // registry.toml across the listing. (v0.4's mutation set
    // maps to the store's identity-gated setters: describe and update_settings; dependency
    // injection lives above the store.)
    type Mutation = (&'static str, fn(&FileStore, &Entry) -> Entry, bool, bool);
    let mutations: [Mutation; 6] = [
        (
            "update_description",
            |store, entry| store.describe(entry, "the new text").unwrap(),
            true,
            false,
        ),
        (
            "update_needs",
            |store, entry| {
                store
                    .update_settings(
                        entry,
                        &EntrySettings {
                            needs: vec!["ffmpeg".to_owned()],
                            ..EntrySettings::default()
                        },
                        "invoke",
                    )
                    .unwrap()
            },
            false,
            false,
        ),
        (
            "update_dependencies",
            |store, entry| {
                store
                    .update_settings(
                        entry,
                        &EntrySettings {
                            dependencies: vec!["httpx".to_owned()],
                            ..EntrySettings::default()
                        },
                        "invoke",
                    )
                    .unwrap()
            },
            false,
            false,
        ),
        (
            "write_workdir",
            |store, entry| {
                store
                    .update_settings(entry, &EntrySettings::default(), "store")
                    .unwrap()
            },
            false,
            false,
        ),
        (
            "write_parameters",
            |store, entry| {
                store
                    .update_settings(
                        entry,
                        &EntrySettings {
                            parameters: vec![ParamDecl::new("CITY")],
                            ..EntrySettings::default()
                        },
                        "invoke",
                    )
                    .unwrap()
            },
            false,
            false,
        ),
        (
            "write_prompt_managed",
            |store, entry| {
                store
                    .update_settings(
                        entry,
                        &EntrySettings {
                            params: vec!["topic".to_owned()],
                            ..EntrySettings::default()
                        },
                        "invoke",
                    )
                    .unwrap()
            },
            false,
            true,
        ),
    ];

    for (label, mutate, changes_description, uses_prompt) in mutations {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(if uses_prompt {
                prompt_copy("subject", b"Summarize {{topic}}\n", "the old text")
            } else {
                python_copy("subject", b"print(1)\n", "the old text")
            })
            .unwrap();

        let updated = mutate(&store, &entry);
        if uses_prompt {
            assert_eq!(
                EntrySettings::from_meta(&updated.meta).params,
                ["topic"],
                "{label}: the prompt mutation did not persist"
            );
        }
        let slug = entry.slug.as_str();

        // Re-projected: the row carries a fresh cache proof bound to the new meta bytes.
        let document = read_registry(&root);
        let stored_hash = document
            .get("entries")
            .unwrap()
            .get(slug)
            .unwrap()
            .get("skit_cache")
            .unwrap()
            .get("metadata_hash")
            .unwrap()
            .as_str()
            .unwrap();
        assert_eq!(
            stored_hash,
            content_hash(&fs::read(meta_path(&root, slug)).unwrap()),
            "{label}: the mutator did not re-stamp the row"
        );

        // Index-served: the listing does not rewrite registry.toml and reflects the change.
        let before = fs::read(registry_path(&root)).unwrap();
        let summaries = store.scan().unwrap().entries;
        assert_eq!(
            fs::read(registry_path(&root)).unwrap(),
            before,
            "{label}: the listing self-healed instead of serving the index"
        );
        let expected = if changes_description {
            "the new text"
        } else {
            "the old text"
        };
        assert_eq!(summaries[0].description, expected, "{label}");
    }
}

#[test]
fn test_a_mutator_whose_row_vanished_mid_write_persists_the_meta_without_resurrecting_it() {
    // WHY: two halves of one rule. The meta — the entry's own truth — is written regardless, and
    // the row projection is skipped when the slug is no longer indexed. A person or an older skit
    // can drop the row; a setter must not resurrect it (that is doctor's call). Here the row is
    // removed from the index before the setter runs; the meta lands on disk, and no row is invented.
    let root = TempDir::new().unwrap();
    let store = FileStore::new(root.path());
    let entry = store
        .create(python_copy("subject", b"print(1)\n", ""))
        .unwrap();

    let mut document = read_registry(&root);
    document
        .get_mut("entries")
        .unwrap()
        .as_table_mut()
        .unwrap()
        .remove(entry.slug.as_str());
    write_registry(&root, &document);

    let updated = store
        .update_settings(
            &entry,
            &EntrySettings {
                needs: vec!["ffmpeg".to_owned()],
                ..EntrySettings::default()
            },
            "invoke",
        )
        .unwrap();

    assert_eq!(EntrySettings::from_meta(&updated.meta).needs, ["ffmpeg"]);
    let scanned = store.scan_entries().unwrap();
    assert_eq!(scanned.len(), 1);
    assert_eq!(EntrySettings::from_meta(&scanned[0].meta).needs, ["ffmpeg"]);
    // The row is doctor's to rebuild, not this write's.
    assert!(store.scan().unwrap().entries.is_empty());
}

#[ignore = "UNMAPPED -> white-box. Monkeypatches store._read_meta to rmtree the entry mid-read to \
            force one exact interleaving; no public seam exists. Rust isolates a per-row read \
            failure as a diagnostic (file_store.rs::\
            a_missing_metadata_file_is_an_io_diagnostic_during_best_effort_scan), never crashing the \
            scan."]
#[test]
fn test_a_listing_survives_an_entry_removed_while_it_was_mid_fallback() {}
