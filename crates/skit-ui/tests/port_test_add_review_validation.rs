//! Mechanical port of the Python oracle module `tests/test_add_review_validation.py`
//! (`origin/main@206f9ef`): "TUI coverage for the drafts boundary and validate-then-write on
//! the add panel." Each `#[test]` keeps its Python `def test_*` name and its "WHY" comment so
//! it traces back to the oracle.
//!
//! Tier: the Python module drives the Textual screens `AddSourceScreen`, `KindPickModal`, and
//! `AddReviewScreen`. In the Rust rewrite that screen logic is the serializable reducer in
//! `skit-ui` (`AddWorkflowState` / `ReviewState`) — the state the Ratatui `skit-tui` layer only
//! renders, and the future Tauri seam. The reducer performs no file or repository I/O, so no
//! filesystem sandbox is needed: sources are byte snapshots and writes are typed effects. The
//! composition-root face (the real `skit` binary, `remove_owned_draft` at `cli.rs:5658`, and the
//! validator refusal copy) is covered separately by
//! `crates/skit-cli/tests/port_test_add_validation_contracts.rs`; this file asserts the reducer's
//! gates that back those screens.
//!
//! Concept mapping used throughout:
//! - Python `AddReviewScreen(path, kind=...)` -> `ReviewState::from_source(SourceSnapshot, kind,
//!   ReviewDefaults)` (the direct `push_screen(AddReviewScreen(p))` analog).
//! - Python `AddSourceScreen` continue / `KindPickModal` -> `AddWorkflowState::reduce` through
//!   `SetSourcePath` -> `Continue` -> `SourceInspected`, which opens a `KindPickerState`.
//! - Python `screen.action_accept()` (validate then store) -> `AddAction::Save`, which asks
//!   `ReviewState::create_entry()` for one `CreateEntry` plan or an `AddProblem`.
//! - Python `screen.action_edit_source()` rescan half -> `ReviewState::rescan(bytes)`.
//! - Python candidate `Checkbox` at `#rv-cand-i` -> `ReviewState::candidate(name).selected` /
//!   `set_candidate_selected(name, ...)`; the name-keyed override survival is `rescan`'s job.
//! - Python `#rv-python` `-`/`none` normalization -> `set_requires_python` (`normalize_python_
//!   automatic`); `#rv-deps` -> `set_dependencies_text`.
//! - Python `KindPickModal(offer_exe=False)` for a draft -> `KindPickerState`, whose choices are
//!   `KnownEntryKind::picker_choices(!is_draft)`; a draft that infers exe is remapped to unknown
//!   inside `SourceSnapshot::inferred_kind` (`add.rs:169`).
//! - Python `store.resolve(slug).meta.mode == "copy"` -> the `CreateEntry.mode` in the emitted
//!   `AddEffect::Commit`; the physical draft unlink -> `AddEffect::ConsumeDraft`, emitted by the
//!   `CommitFinished` gate only when `is_draft && storage == Copy` (`add.rs:1675`).
//! - Python `notify(severity="error")` + panel-stays-open + nothing-stored ->
//!   `problem() == Some(AddProblem::...)`, no effects, `stage() == Review`. The localized message
//!   ("package requirement" / "version constraint") lives at the skit-i18n tier, which skit-ui does
//!   not depend on, so the typed `AddProblem` variant is the strongest at-tier observable; the
//!   message-text divergence is already recorded by the sibling CLI port (not repeated here).
//!
//! Buckets (10 Python defs -> 10 `#[test]`): ALL asserting, all passing. Test 3's Python
//! `monkeypatch.setattr(store, "resolve", ...)` reference-mode fake has its Rust analog in the
//! serde state seam (skit-ui's one dev-dependency): a fresh draft is force-copied by
//! `ReviewState::set_storage` (`add.rs:793`), so the only way to reach the non-copy branch of the
//! success-unlink gate — the exact branch the oracle test exists to pin — is to flip `storage`
//! through `serde_json` round-trip, which is the monkeypatch analog. No cross-crate, absent, or
//! divergence stubs.

use std::path::PathBuf;

use skit_application::{CreateEntry, SourcePermissions};
use skit_domain::StorageMode;
use skit_ui::{
    AddAction, AddEffect, AddProblem, AddRequestId, AddStage, AddWorkflowState, DraftKind,
    KnownEntryKind, ReviewDefaults, ReviewState, SourceSnapshot,
};

/// One byte-exact host snapshot. `SourceInspected` trusts the caller's `is_draft`; `DraftEdited`
/// overrides it to `true` (the reducer, not the host, marks authored work a draft).
fn snapshot(path: &str, bytes: &[u8], unix_mode: u32, is_draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from(path),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(unix_mode),
        },
        executable: None,
        is_regular: true,
        is_directory: false,
        is_draft,
        identity: None,
    }
}

/// A parser-backed script review, the direct twin of `push_screen(AddReviewScreen(p, kind=...))`.
fn review(path: &str, bytes: &[u8], kind: KnownEntryKind) -> ReviewState {
    ReviewState::from_source(
        snapshot(path, bytes, 0o644, false),
        kind,
        ReviewDefaults::default(),
    )
}

fn request_of(
    effects: &[AddEffect],
    selector: fn(&AddEffect) -> Option<AddRequestId>,
) -> AddRequestId {
    effects
        .iter()
        .find_map(selector)
        .expect("the expected request-bearing effect")
}

fn inspect_request(effect: &AddEffect) -> Option<AddRequestId> {
    match effect {
        AddEffect::InspectSource { request, .. } => Some(*request),
        _ => None,
    }
}

fn author_request(effect: &AddEffect) -> Option<AddRequestId> {
    match effect {
        AddEffect::AuthorDraft { request, .. } => Some(*request),
        _ => None,
    }
}

fn commit_request(effect: &AddEffect) -> Option<AddRequestId> {
    match effect {
        AddEffect::Commit { request, .. } => Some(*request),
        _ => None,
    }
}

/// The `CreateEntry` plan the panel emitted on accept, or a panic when it refused (nothing stored).
fn committed_entry(effects: &[AddEffect]) -> &CreateEntry {
    effects
        .iter()
        .find_map(|effect| match effect {
            AddEffect::Commit { entry, .. } => Some(entry.as_ref()),
            _ => None,
        })
        .expect("a commit effect — the panel committed")
}

/// Drive `NewDraft` -> `DraftEdited` so the reducer stamps `is_draft` and opens the review (the
/// Ctrl+N authoring lane). Returns the opened workflow.
fn authored_draft(path: &str, bytes: &[u8]) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Script));
    let request = request_of(&effects, author_request);
    let _ = workflow.reduce(AddAction::DraftEdited {
        request,
        result: Ok(Some(snapshot(path, bytes, 0o644, false))),
    });
    assert_eq!(workflow.stage(), AddStage::Review);
    workflow
}

// ==========================================================================
// 1. An inferred exe on a resumed draft is remapped to the ASK (no program option)
// ==========================================================================

#[test]
fn test_draft_resume_inferred_exe_routes_to_ask_without_program_option() {
    // A resumed draft that INFERS exe (a hand-planted +x bit on an extensionless draft) is
    // remapped to unknown -> the kind picker, which for a draft offers no "A program" option:
    // an exe entry's reference mode is the one shape the drafts boundary forbids.
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath("skit-new-binish".to_owned()));
    let effects = workflow.reduce(AddAction::Continue);
    let request = request_of(&effects, inspect_request);
    // +x (0o755) on an extensionless, shebang-less draft: infer_kind classifies it exe.
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot(
            "skit-new-binish",
            b"opaque program bytes\n",
            0o755,
            true,
        )),
    });

    assert_eq!(workflow.stage(), AddStage::Kind); // exe->unknown remap -> ASK, not the exe review
    let picker = workflow.kind_picker().expect("kind picker");
    assert!(!picker.offers(KnownEntryKind::Executable)); // no "A program" option for a draft
}

#[test]
fn rust_additive_owned_draft_shaped_directory_never_routes_to_program_review() {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath("skit-new-directory".to_owned()));
    let effects = workflow.reduce(AddAction::Continue);
    let request = request_of(&effects, inspect_request);
    let mut directory = snapshot("skit-new-directory", b"", 0o755, true);
    directory.is_regular = false;
    directory.is_directory = true;
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(directory),
    });

    assert_eq!(workflow.stage(), AddStage::Kind);
    let picker = workflow
        .kind_picker()
        .expect("owned directory needs a safe kind choice");
    assert!(!picker.offers(KnownEntryKind::Executable));
}

// ==========================================================================
// 2. The fresh success-unlink is MODE-GATED
// ==========================================================================

#[test]
fn test_fresh_draft_copy_flow_unlinks_the_file() {
    // The copy arc (pin): a normal fresh draft lands as a copy, so the draft is consumed.
    let mut workflow = authored_draft("skit-new-copied.py", b"import sys\nprint('drafted')\n");
    let _ = workflow.reduce(AddAction::SetReviewName("copied".to_owned()));
    let expected = workflow.review().unwrap().source().clone();

    let saved = workflow.reduce(AddAction::Save);
    assert_eq!(committed_entry(&saved).mode, StorageMode::Copy); // copy: the store holds it
    let request = request_of(&saved, commit_request);

    let done = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok("copied".to_owned()),
    });
    // copy: the store holds it, so the draft is unlinked, and the slug completes.
    assert!(done.contains(&AddEffect::ConsumeDraft(expected)));
    assert!(done.contains(&AddEffect::Complete("copied".to_owned())));
}

#[test]
fn test_fresh_draft_keeps_the_file_when_the_entry_is_not_a_copy() {
    // The non-copy arc: the mode-gate reads mode, so a dismissal that resolves to a non-copy
    // entry keeps the file (no lane deletes what the store does not hold). Real fresh authoring
    // always copies, so the arc is exercised by forcing the review's stored mode to reference —
    // the serde-seam analog of the oracle's `monkeypatch.setattr(store, "resolve", ...)`, because
    // `set_storage` force-copies a draft (add.rs:793) and no public setter reaches this state.
    // Kills the mutant that drops the `storage == Copy` condition (would delete regardless).
    let mut workflow = authored_draft("skit-new-kept.py", b"import sys\nprint('drafted')\n");
    let _ = workflow.reduce(AddAction::SetReviewName("kept".to_owned()));

    let mut json = serde_json::to_value(&workflow).expect("serialize workflow");
    assert_eq!(json["review"]["storage"], serde_json::json!("copy")); // a draft is a copy by construction
    json["review"]["storage"] = serde_json::json!("reference");
    let mut workflow: AddWorkflowState =
        serde_json::from_value(json).expect("deserialize workflow");

    let saved = workflow.reduce(AddAction::Save); // still committed: create_entry forces mode=copy
    let request = request_of(&saved, commit_request);
    let done = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok("kept".to_owned()),
    });
    // non-copy dismissal: the gate kept the file — no ConsumeDraft, only the slug completes.
    assert!(
        !done
            .iter()
            .any(|effect| matches!(effect, AddEffect::ConsumeDraft(_)))
    );
    assert!(done.contains(&AddEffect::Complete("kept".to_owned())));
}

// ==========================================================================
// 3. Candidate ticks survive the edit->rescan recompose
// ==========================================================================

#[test]
fn test_candidate_tick_survives_a_noop_edit_rescan() {
    // Untick a candidate, run a no-op edit rescan, return -> the tick is still unticked (the
    // rescan refreshes detection but must not throw away the user's tick).
    let mut screen = review(
        "cand.py",
        b"CITY = \"Taipei\"\nprint(CITY)\n",
        KnownEntryKind::Python,
    );
    assert_eq!(screen.candidate("CITY").map(|c| c.selected), Some(true)); // CITY ticked by default
    screen.set_candidate_selected("CITY", false); // the user unticks it
    screen.rescan(b"CITY = \"Taipei\"\nprint(CITY)\n".to_vec()); // no-op edit, then rescan/recompose
    assert_eq!(screen.candidate("CITY").map(|c| c.selected), Some(false)); // tick persisted
}

#[test]
fn test_edit_source_capture_skips_a_candidate_with_no_checkbox() {
    // A getopts (modeled-reader) shell with a bare const has a candidate in analysis but NO tick
    // checkbox rendered — the modeled form replaces the list. The rescan capture never queries a
    // checkbox that is not mounted: no crash, no phantom override (the guarded arc).
    let source = b"#!/usr/bin/env bash\nREGION=us-east-1\nwhile getopts \"n:\" o; do case $o in n) NAME=$OPTARG;; esac; done\necho \"$REGION $NAME\"\n";
    let mut screen = review("opt.sh", source, KnownEntryKind::Shell);
    // REGION is a candidate in the analysis...
    assert!(
        screen
            .onboarding()
            .candidates
            .iter()
            .any(|candidate| candidate.declaration.name == "REGION")
    );
    // ...but the modeled reader hid the tick list (no offered candidate to render).
    assert!(screen.candidates().is_empty());
    assert!(
        screen
            .modeled_cli_field_count()
            .is_some_and(|count| count > 0)
    );

    screen.rescan(source.to_vec()); // the capture runs with no offered candidate (the guarded arc)
    assert!(screen.candidates().is_empty()); // nothing captured — no checkbox to read
    // still the modeled review, no crash: the const stays analyzed.
    assert!(
        screen
            .onboarding()
            .candidates
            .iter()
            .any(|candidate| candidate.declaration.name == "REGION")
    );
}

#[test]
fn test_new_candidate_after_a_real_edit_takes_its_default() {
    // A NEW candidate appearing after a real edit takes its detection default (ticked), while the
    // earlier candidate's unticked override is preserved.
    let mut screen = review(
        "cand2.py",
        b"CITY = \"Taipei\"\nprint(CITY)\n",
        KnownEntryKind::Python,
    );
    screen.set_candidate_selected("CITY", false); // untick CITY
    screen.rescan(b"CITY = \"Taipei\"\nREGION = \"us-east-1\"\nprint(CITY, REGION)\n".to_vec());
    assert_eq!(screen.candidate("CITY").map(|c| c.selected), Some(false)); // override preserved
    assert_eq!(screen.candidate("REGION").map(|c| c.selected), Some(true)); // new candidate: default tick
}

// ==========================================================================
// 4. AddReviewScreen: '-' normalization + validate-then-write (uv), npm not validated
// ==========================================================================

#[test]
fn test_review_dash_python_is_stored_as_automatic() {
    // '-' in #rv-python normalizes to automatic: the entry commits with no requires-python.
    let mut screen = review("auto.py", b"print(1)\n", KnownEntryKind::Python);
    screen.set_requires_python("-");
    screen.set_name("autoentry");

    let mut workflow = AddWorkflowState::from_review(screen);
    let saved = workflow.reduce(AddAction::Save);
    let entry = committed_entry(&saved); // committed, not rejected
    let stored = std::str::from_utf8(&entry.payload.as_ref().expect("copy payload").bytes)
        .expect("valid utf-8");
    assert!(!stored.contains("requires-python"), "{stored}");
    assert!(entry.settings.requires_python.is_empty());
}

#[test]
fn test_review_rejects_a_bad_uv_dep_and_keeps_the_panel_open() {
    // An unparseable uv requirement is refused BEFORE storing: a typed error, the panel stays
    // open, nothing lands.
    let mut screen = review("baddep.py", b"print(1)\n", KnownEntryKind::Python);
    screen.set_dependencies_text("@@@");
    screen.set_name("baddep");

    let mut workflow = AddWorkflowState::from_review(screen);
    let effects = workflow.reduce(AddAction::Save);
    assert!(effects.is_empty()); // nothing stored — no commit effect
    assert_eq!(
        workflow.problem(),
        Some(&AddProblem::InvalidDependency {
            value: "@@@".to_owned()
        })
    );
    assert_eq!(workflow.stage(), AddStage::Review); // still open
}

#[test]
fn test_review_rejects_a_bad_python_constraint_and_keeps_the_panel_open() {
    let mut screen = review("badpy.py", b"print(1)\n", KnownEntryKind::Python);
    screen.set_requires_python("not-a-version");
    screen.set_name("badpy");

    let mut workflow = AddWorkflowState::from_review(screen);
    let effects = workflow.reduce(AddAction::Save);
    assert!(effects.is_empty()); // nothing stored
    assert_eq!(
        workflow.problem(),
        Some(&AddProblem::InvalidPythonConstraint {
            value: "not-a-version".to_owned()
        })
    );
    assert_eq!(workflow.stage(), AddStage::Review); // still open
}

#[test]
fn test_review_does_not_validate_npm_deps() {
    // The complement: an npm dep string that would FAIL PEP 508 (a scoped package) still commits
    // on a js add — the npm installer owns that grammar, not skit's validator.
    let mut screen = review(
        "tool.js",
        b"import thing from \"@scope/thing\";\nconsole.log(thing);\n",
        KnownEntryKind::JavaScript,
    );
    screen.set_dependencies_text("@scope/thing");
    screen.set_name("jstool");

    let mut workflow = AddWorkflowState::from_review(screen);
    let saved = workflow.reduce(AddAction::Save);
    let entry = committed_entry(&saved); // committed, not rejected
    assert_eq!(entry.kind.as_str(), "js");
    assert_eq!(entry.settings.dependencies, ["@scope/thing"]);
}
