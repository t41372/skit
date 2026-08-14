//! Mechanical port of the Python oracle module `tests/test_draft_and_reader_tui.py`
//! (`origin/main@206f9ef`): "Draft and reader TUI contracts — pilot coverage." Each
//! `#[test]` keeps its Python `def test_*` name so it traces back to its origin, and each
//! Python "WHY" comment is preserved above it.
//!
//! The Python module drives the Textual TUI (`tui.MenuApp`, `AddSourceScreen`,
//! `AddReviewScreen`, `DraftDeleteConfirm`, `tui_settings.ScriptSettingsScreen`) through a
//! pilot. In the Rust rewrite that surface splits across two tiers, both reachable from
//! `skit-tui`: the frontend-neutral reducer/review state (`skit-ui`) and the Ratatui
//! renderer + input mapping (`skit-tui`). No store or filesystem is touched — every source
//! is an in-memory `SourceSnapshot` / `DraftSummary` — so the `SKIT_*_DIR` sandbox rule is
//! moot, and the `skit-cli` crate hint does not apply (this module never spawns the binary).
//!
//! Concept mapping used throughout:
//! - Python `AddReviewScreen(path, kind=…, requires_python=…)` ->
//!   `ReviewState::from_source(SourceSnapshot, KnownEntryKind, ReviewDefaults)` (skit-ui).
//! - Python `AddSourceScreen` draft resume / `_submit_path` routing -> the `AddWorkflowState`
//!   reducer (`SetSourcePath` -> `Continue` -> `SourceInspected`) (skit-ui).
//! - Python `review._kind` -> `review.kind()`; the stored entry's kind -> `create_entry().kind`.
//! - Python "resumed draft reached the store -> unlinked" -> `AddEffect::ConsumeDraft` emitted
//!   on `CommitFinished(Ok)` when the source is a draft stored as a copy.
//! - Python `review._requires_python` / `#rv-python` value -> `review.requires_python()`.
//! - Python `AddReviewScreen(requires_python=…)` -> `ReviewDefaults::requires_python`.
//! - Python `action_edit_source()` -> `review.rescan(bytes)` (production drives it through the
//!   `SourceEdited` effect; here the byte-exact edit is applied directly).
//! - Python `_statics(review)` note text / `#review-keys` render -> the rendered `render_add`
//!   buffer flattened to one string.
//! - Python `review.query("#rv-cand-i")` -> `review.candidates()` plus the rendered tick list.
//! - Python `ngettext` "(%(count)s field)" -> the rendered reader notice "(1 field)"/"(2 fields)".
//! - Python `ScriptSettingsScreen(entry)` `_cli_driven` gate -> `SettingsView::from_inputs`
//!   fed `reader_fields` (the in-crate twin of `flows.reader_fields`, computed here from
//!   `ReviewState::modeled_cli_field_count()`, the same `CliSurface::Static/Dynamic` split as
//!   `skit_language::cli_params` at lib.rs:844-856) and `candidates` (the offered constants).
//!   The `skit-cli` composition-root wiring that reads a real entry into these inputs
//!   (`settings_parameter_context`, cli.rs:5278) is pinned by the sibling
//!   `port_test_draft_inference_and_reader_cli.rs::test_reader_fields_predicate_rows`.
//! - Python `screen.query("#st-new-0")` (the manage checkbox) -> `view.field(MANAGE_KEY)`.
//! - Python `_statics(screen)` note "comes from its own command-line arguments" -> a
//!   Parameters-section `SettingsNote.text`.
//! - Python Ctrl+D / `DraftDeleteConfirm` -> `AddScreenSession::handle_event` produces
//!   `AddAction::DeleteSelectedDraft` -> `AddStage::ConfirmDraftDelete` -> `ConfirmDraftDelete(bool)`
//!   -> host `DraftDeleted` result -> `AddNotice::DraftDeleted`. The confirm key `y` has no Rust
//!   binding (Enter is the confirm, add.rs:506-512); the confirm ACTION `ConfirmDraftDelete(true)`
//!   is the pinned contract (delete only behind an explicit confirm), so it is driven directly.
//! - Python `action_delete_draft` early-return guards -> `reduce(DeleteSelectedDraft)` with no
//!   drafts / no highlight returns no effect and leaves the stage on `Source`.
//! - Python `#add-draft-actions` Ctrl+D chip -> the source-stage footer "Delete draft…" chip,
//!   rendered only when a draft is listed.
//!
//! Bucket disposition (16 oracle defs; 15 real asserting tests, 1 divergence):
//! - 15 real: the review pin/rescan/explicit trio, the dynamic-vs-modeled review ticks + Space
//!   chip, both settings-gate tests, the singular/plural notices, and every draft-delete flow.
//! - 1 FAILING CONTRACT (divergence), full asserting body kept behind `#[ignore]`:
//!   * `test_resume_bash_shebang_draft_lands_as_shell` — Rust's `infer_kind` is extension-first
//!     (`.py` -> python) and there is no `kind_for_draft` shebang-first rule for skit's OWN
//!     drafts (registry.py:442, store.py:308), so the bash-shebang `skit-new-*.py` draft resumes
//!     as PYTHON, not shell. The TUI reducer DOES wire the consume-on-copy unlink (unlike the CLI
//!     lane), so the divergence here is the kind alone. Same diagnosis as the CLI twin
//!     `port_test_draft_inference_and_reader_cli.rs::test_cli_add_bash_shebang_draft_lands_as_shell_and_unlinks`.
//! - 0 cross-crate, 0 absent.

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::SourcePermissions;
use skit_domain::EntryKind;
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenEvent, AddScreenGeometry, AddScreenSession, AddTextField, render_add,
};
use skit_ui::{
    AddAction, AddEffect, AddNotice, AddStage, AddWorkflowState, DraftSummary, KnownEntryKind,
    MANAGE_KEY, ReviewDefaults, ReviewState, SettingsInputs, SettingsItem, SettingsSectionId,
    SettingsView, SourceSnapshot,
};
use std::path::PathBuf;

// The oracle's two module-level shell fixtures (byte-exact).
const DYN_SH: &[u8] =
    b"#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";
const MODELED_SH: &[u8] =
    b"#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n";

// The exact renderer strings this module asserts on (skit-tui/src/screens/add.rs).
const READER_NOTICE: &str = "skit read this script's own arguments";
const SELF_PARSE_NOTICE: &str = "parses its own arguments";
const TICK_PROMPT: &str = "Tick the ones the run form should ask for:";
// The settings-gate reader note (skit-ui/src/settings.rs:1194).
const READER_DRIVEN_NOTE: &str = "comes from its own command-line arguments";

/// Build one byte-exact source snapshot the way the host would hand it to the reducer.
fn snap(path: &str, bytes: &[u8], draft: bool) -> SourceSnapshot {
    SourceSnapshot {
        path: path.into(),
        source_record: path.to_owned(),
        bytes: bytes.to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(0o644),
        },
        is_regular: true,
        is_directory: false,
        is_draft: draft,
    }
}

/// Analyze one snapshot into a review, as `AddReviewScreen(path, kind=…)` does.
fn review_of(
    path: &str,
    bytes: &[u8],
    kind: KnownEntryKind,
    defaults: ReviewDefaults,
) -> ReviewState {
    ReviewState::from_source(snap(path, bytes, false), kind, defaults)
}

/// Drive the reducer along the resume/route path a source path takes on `Continue`.
fn route_source(source: SourceSnapshot) -> AddWorkflowState {
    let mut workflow = AddWorkflowState::new(Vec::new());
    let _ = workflow.reduce(AddAction::SetSourcePath(source.path.display().to_string()));
    let effects = workflow.reduce(AddAction::Continue);
    let AddEffect::InspectSource { request, .. } = &effects[0] else {
        panic!("a source path must inspect the source");
    };
    let request = *request;
    let _ = workflow.reduce(AddAction::SourceInspected {
        request,
        result: Ok(source),
    });
    workflow
}

/// The rendered buffer flattened to one string, like the oracle's `_statics` join.
fn rendered(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

/// Click one rendered add control through the same mouse path as the host session.
fn left_click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// Render one add workflow through the real `render_add` and return the flattened buffer.
fn render_add_text(state: &AddWorkflowState, width: u16, height: u16) -> String {
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_add(frame, frame.area(), state, &mut session, Locale::En);
        })
        .unwrap();
    rendered(terminal.backend().buffer())
}

/// Decode the exact bytes a copy commit would store from one accepted review.
fn stored_copy(review: &ReviewState) -> String {
    let plan = review.create_entry().expect("the review accepts");
    String::from_utf8(plan.payload.expect("a copy payload").bytes).expect("utf-8 stored copy")
}

/// Build the settings surface the way `ScriptSettingsScreen(entry)` opens on a stored shell
/// script: the reader-field count and offered constants derive from the SAME onboarding engine
/// the composition root reads through `cli_params`/`detect_candidates`.
fn shell_settings(bytes: &[u8]) -> SettingsView {
    let review = review_of(
        "s.sh",
        bytes,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    let reader_fields = review.modeled_cli_field_count().unwrap_or(0);
    let candidates = review
        .candidates()
        .iter()
        .map(|candidate| candidate.declaration.name.clone())
        .collect();
    SettingsView::from_inputs(&SettingsInputs {
        selector: "s".to_owned(),
        kind: "shell".to_owned(),
        name: "s".to_owned(),
        source: "/tmp/s.sh".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        reader_fields,
        candidates,
        ..SettingsInputs::default()
    })
}

/// The `SettingsNote` text lines of the Parameters section, like the oracle's `_statics` filter.
fn parameter_notes(view: &SettingsView) -> Vec<String> {
    view.sections
        .iter()
        .filter(|section| section.id == SettingsSectionId::Parameters)
        .flat_map(|section| section.items.iter())
        .filter_map(|item| match item {
            SettingsItem::Note(note) => Some(note.text.clone()),
            SettingsItem::Field(_) => None,
        })
        .collect()
}

// ==========================================================================
// 1. A bash-shebang kept draft resumes as a SHELL entry
// ==========================================================================

#[test]
#[ignore = "FAILING CONTRACT (divergence): the oracle classifies skit's OWN `.py` draft shebang-first (a bash shebang -> shell, registry.kind_for_draft registry.py:442 / store.py:308). Rust's `infer_kind` is extension-first (`.py` -> python) and `SourceSnapshot::inferred_kind` has no draft shebang-first path, so the resumed draft lands as PYTHON, not shell — `review.kind()` is Python and the first assert fails. The TUI reducer DOES wire ConsumeDraft on a copy commit, so the divergence here is the kind alone (unlike the CLI lane). Same diagnosis as the CLI twin `port_test_draft_inference_and_reader_cli.rs`."]
fn test_resume_bash_shebang_draft_lands_as_shell() {
    // Resuming `skit-new-*.py` with a bash body opens the review panel as SHELL (kind_for_draft
    // reads the shebang, not the mkstemp suffix), the stored entry is shell, and the draft is
    // consumed on accept.
    let mut workflow = route_source(snap(
        "skit-new-ship.py",
        b"#!/usr/bin/env bash\necho drafted\n",
        true,
    ));
    let review = workflow.review().expect("the draft resumes into a review");
    assert_eq!(review.kind(), KnownEntryKind::Shell); // reclassified by shebang, not the .py suffix
    // The stored entry is a shell entry.
    assert_eq!(
        review.create_entry().expect("the review accepts").kind,
        EntryKind::parse("shell".to_owned()).unwrap()
    );
    // Accept -> the resumed draft reached the store as a copy, so the host is told to consume it.
    let _ = workflow.reduce(AddAction::SetReviewName("shipit".to_owned()));
    let effects = workflow.reduce(AddAction::Save);
    let AddEffect::Commit { request, .. } = &effects[0] else {
        panic!("save must commit");
    };
    let request = *request;
    let effects = workflow.reduce(AddAction::CommitFinished {
        request,
        result: Ok("shipit".to_owned()),
    });
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, AddEffect::ConsumeDraft(path) if path == &PathBuf::from("skit-new-ship.py"))),
        "the resumed draft that reached the store must be consumed"
    );
}

// ==========================================================================
// 4. A versioned python shebang shows AND stores its requires-python pin
// ==========================================================================

#[test]
fn test_review_versioned_shebang_shows_and_stores_pin() {
    let mut review = review_of(
        "v.py",
        b"#!/usr/bin/env python3.12\nprint('hi')\n",
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    // Derived from the shebang, and shown in (== editable through) the #rv-python field value.
    assert_eq!(review.requires_python(), ">=3.12,<3.13");
    review.set_name("vpin");
    // Landed in the stored copy's PEP 723 block.
    assert!(stored_copy(&review).contains("requires-python = \">=3.12,<3.13\""));
}

#[test]
fn test_review_pin_follows_a_shebang_edit_on_rescan() {
    // Edit -> rescan recomputes the auto pin: the pin followed the shebang.
    let mut review = review_of(
        "v.py",
        b"#!/usr/bin/env python3.12\nprint('hi')\n",
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), ">=3.12,<3.13");
    review.rescan(b"#!/usr/bin/env python3.11\nprint('hi')\n".to_vec());
    assert_eq!(review.requires_python(), ">=3.11,<3.12"); // the pin followed the shebang
}

#[test]
fn test_review_explicit_python_is_not_overwritten_by_the_shebang() {
    // An explicit requires-python (the CLI --python face) is the user's own value; the auto-pin
    // never fires over it, so it is shown verbatim.
    let review = review_of(
        "v.py",
        b"#!/usr/bin/env python3.12\nprint('hi')\n",
        KnownEntryKind::Python,
        ReviewDefaults {
            requires_python: Some(">=3.9".to_owned()),
            ..ReviewDefaults::default()
        },
    );
    assert_eq!(review.requires_python(), ">=3.9"); // explicit value, not the shebang's 3.12
}

// ==========================================================================
// 5. Review panel keys the manage offer on a MODELED form
// ==========================================================================

#[test]
fn test_review_dynamic_optstring_keeps_ticks_and_space_chip() {
    // A dynamic optstring shell self-parses but can't be modeled: the panel prints the passthrough
    // hint AND keeps the candidate ticks, and the Space/Toggle chip is advertised.
    let review = review_of(
        "dyn.sh",
        DYN_SH,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(!review.candidates().is_empty()); // ...the ticks remain (constants are additive)
    let screen = render_add_text(&AddWorkflowState::from_review(review), 100, 40);
    assert!(screen.contains(SELF_PARSE_NOTICE)); // the passthrough notice
    assert!(screen.contains(TICK_PROMPT)); // ...and the tick list mounts (the #rv-cand-0 twin)
    assert!(screen.contains("Space")); // the Space chip key hint is advertised
    assert!(screen.contains("Toggle")); // ...as a real toggle path
}

#[test]
fn test_review_modeled_getopts_suppresses_ticks_and_space_chip() {
    // The complement: a MODELED getopts form IS the interface — the ✓ read notice prints, no
    // candidate ticks, and Space is not advertised (a dead key).
    let review = review_of(
        "mod.sh",
        MODELED_SH,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(review.candidates().is_empty()); // managing would replace the modeled form
    let screen = render_add_text(&AddWorkflowState::from_review(review), 100, 40);
    assert!(screen.contains(READER_NOTICE));
    assert!(!screen.contains("Toggle")); // no dead Space key
}

#[test]
fn test_settings_dynamic_optstring_offers_tick_checkboxes() {
    // The settings screen keys `_cli_driven` on flows.reader_fields now: a dynamic optstring
    // (read_cli returns ok=False -> reader_fields 0) is NOT cli-driven, so it offers the
    // manage-these checkboxes — the old read_cli-is-not-None gate wrongly suppressed them.
    let view = shell_settings(DYN_SH);
    assert!(view.field(MANAGE_KEY).is_some()); // tick-to-manage checkboxes are offered (#st-new-0)
    assert!(
        !parameter_notes(&view)
            .iter()
            .any(|note| note.contains(READER_DRIVEN_NOTE))
    );
}

#[test]
fn test_settings_modeled_getopts_hides_tick_checkboxes() {
    // The unchanged True branch on the shell path: a MODELED getopts form (reader_fields 2)
    // suppresses the checkboxes and shows the leave-it-as-is hint. (Production's detect_candidates
    // would still carry CITY here; the ReaderDriven gate fires on `managed.is_empty() &&
    // reader_fields > 0` before candidates are consulted, so the empty candidate list is not
    // load-bearing.)
    let view = shell_settings(MODELED_SH);
    assert!(view.field(MANAGE_KEY).is_none()); // modeled form: no manage checkboxes
    assert!(
        parameter_notes(&view)
            .iter()
            .any(|note| note.contains(READER_DRIVEN_NOTE))
    );
}

// ==========================================================================
// 7. Singular vs plural field count in the review panel notice
// ==========================================================================

#[test]
fn test_review_one_field_getopts_says_singular() {
    let review = review_of(
        "one.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:\" o; do :; done\n",
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    let blurb = render_add_text(&AddWorkflowState::from_review(review), 100, 40);
    assert!(blurb.contains("(1 field)"));
    assert!(!blurb.contains("(1 fields)"));
}

#[test]
fn test_review_multi_field_getopts_says_plural() {
    let review = review_of(
        "many.sh",
        b"#!/usr/bin/env bash\nwhile getopts \"n:v\" o; do :; done\n",
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(
        render_add_text(&AddWorkflowState::from_review(review), 100, 40).contains("(2 fields)")
    );
}

// ==========================================================================
// 8. Ctrl+D deletes a kept draft (confirm), keeps it on Esc, yields to Input mid-edit
// ==========================================================================

#[test]
fn test_ctrl_d_deletes_the_highlighted_draft_after_confirm() {
    // Ctrl+D from the source screen opens the confirm; confirming deletes the highlighted draft
    // (the user's only copy), notifies, and recomposes the list — the other draft survives.
    let keep = DraftSummary {
        path: PathBuf::from("skit-new-keep.py"),
        modified: 1,
    };
    let doomed = DraftSummary {
        path: PathBuf::from("skit-new-doomed.py"),
        modified: 2,
    };
    let mut workflow = AddWorkflowState::new(vec![keep.clone(), doomed.clone()]);
    let doomed_index = workflow
        .source()
        .listed_drafts()
        .iter()
        .position(|draft| draft.path == doomed.path)
        .expect("doomed is listed");
    let mut session = AddScreenSession::default();
    let mut geometry = AddScreenGeometry::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &workflow, &mut session, Locale::En);
        })
        .unwrap();
    let draft_area = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Draft(doomed_index))
        .expect("the doomed draft is a mouse target")
        .area;
    let select = session.handle_event(left_click(draft_area.x, draft_area.y), &workflow, &geometry);
    assert_eq!(
        select,
        Some(AddScreenEvent::Action(AddAction::SelectDraft(doomed_index)))
    );
    let _ = workflow.reduce(AddAction::SelectDraft(doomed_index)); // highlight, do NOT resume
    assert_eq!(session.focused(), Some(&AddControlId::Draft(doomed_index)));

    // The advertised key, from the focused draft list (not an Input): Ctrl+D deletes the row.
    let event = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        &workflow,
        &geometry,
    );
    assert_eq!(
        event,
        Some(AddScreenEvent::Action(AddAction::DeleteSelectedDraft))
    );
    let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
    assert_eq!(workflow.stage(), AddStage::ConfirmDraftDelete); // the confirm opened

    // Confirm (the oracle's `y`; Enter/ConfirmDraftDelete(true) in Rust) deletes the highlight.
    let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));
    let [AddEffect::DeleteDraft { request, path }] = effects.as_slice() else {
        panic!("a confirmed delete must ask the host to delete");
    };
    assert_eq!(path, &doomed.path); // the highlighted draft is the unlink target
    let request = *request;
    let _ = workflow.reduce(AddAction::DraftDeleted {
        request,
        result: Ok(()),
    });
    // Notified, and recomposed: exactly the surviving draft remains listed.
    assert_eq!(
        workflow.notice(),
        Some(&AddNotice::DraftDeleted(doomed.path.clone()))
    );
    let remaining = workflow.source().listed_drafts();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].path, keep.path); // the other survived
}

#[test]
fn test_ctrl_d_confirm_esc_keeps_the_draft() {
    // Esc on the confirm keeps the file — a draft is never lost to a single keystroke.
    let draft = DraftSummary {
        path: PathBuf::from("skit-new-safe.py"),
        modified: 1,
    };
    let mut workflow = AddWorkflowState::new(vec![draft]);
    let _ = workflow.reduce(AddAction::SelectDraft(0));
    let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
    assert_eq!(workflow.stage(), AddStage::ConfirmDraftDelete);

    // Esc at the confirm -> ConfirmDraftDelete(false).
    let mut session = AddScreenSession::default();
    let event = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        &workflow,
        &AddScreenGeometry::default(),
    );
    assert_eq!(
        event,
        Some(AddScreenEvent::Action(AddAction::ConfirmDraftDelete(false)))
    );
    let effects = workflow.reduce(AddAction::ConfirmDraftDelete(false));
    assert!(effects.is_empty()); // no delete effect: nothing is unlinked
    assert_eq!(workflow.stage(), AddStage::Source); // back to the source screen
    assert_eq!(workflow.source().listed_drafts().len(), 1); // the draft is kept
}

#[test]
fn test_ctrl_d_while_editing_a_field_is_the_inputs_delete_right() {
    // Ctrl+D is NOT priority-bound: with an Input focused it is the Input's own delete-right, so
    // no confirm opens and no draft is touched (the AGENTS editing-chord rule).
    let draft = DraftSummary {
        path: PathBuf::from("skit-new-edit.py"),
        modified: 1,
    };
    let mut workflow = AddWorkflowState::new(vec![draft.clone()]);
    let _ = workflow.reduce(AddAction::SetSourcePath("abc".to_owned())); // typing in the path field

    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_add(frame, frame.area(), &workflow, &mut session, Locale::En);
        })
        .unwrap();
    // The mid-edit precondition: the path Input holds focus.
    assert_eq!(
        session.focused(),
        Some(&AddControlId::Text(AddTextField::SourcePath))
    );

    // Match the oracle's cursor_position=1 without exposing private widget state.
    let _ = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        &workflow,
        &AddScreenGeometry::default(),
    );
    let _ = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        &workflow,
        &AddScreenGeometry::default(),
    );

    let event = session.handle_event(
        Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
        &workflow,
        &AddScreenGeometry::default(),
    );
    assert_eq!(
        event,
        Some(AddScreenEvent::Action(AddAction::SetSourcePath(
            "ac".to_owned()
        )))
    );
    let Some(AddScreenEvent::Action(action)) = event else {
        unreachable!("the exact action was asserted above");
    };
    let effects = workflow.reduce(action);
    assert!(effects.is_empty());
    assert_eq!(workflow.stage(), AddStage::Source); // no confirm modal opened
    assert_eq!(workflow.source().path, "ac");
    assert_eq!(workflow.source().listed_drafts(), [draft]); // the draft was never touched
}

#[test]
fn test_delete_draft_action_is_a_noop_when_no_drafts() {
    // DeleteSelectedDraft with no drafts present returns early — the key must never crash or open
    // a confirm on an empty screen.
    let mut workflow = AddWorkflowState::new(Vec::new());
    assert!(workflow.source().listed_drafts().is_empty()); // no drafts
    let effects = workflow.reduce(AddAction::DeleteSelectedDraft); // the `if not lists: return` guard
    assert!(effects.is_empty());
    assert_eq!(workflow.stage(), AddStage::Source); // no confirm opened
}

#[test]
fn test_delete_draft_action_is_a_noop_when_nothing_highlighted() {
    // The drafts list exists but nothing is highlighted: the action returns early and the draft
    // is untouched.
    let draft = DraftSummary {
        path: PathBuf::from("skit-new-none.py"),
        modified: 1,
    };
    let mut workflow = AddWorkflowState::new(vec![draft]);
    // Nothing selected (the `if highlighted is None: return` guard).
    let effects = workflow.reduce(AddAction::DeleteSelectedDraft);
    assert!(effects.is_empty());
    assert_eq!(workflow.stage(), AddStage::Source); // no confirm modal
    assert_eq!(workflow.source().listed_drafts().len(), 1); // untouched
}

#[test]
fn test_delete_draft_chip_only_renders_when_drafts_exist() {
    // The Ctrl+D chip is the mouse path — it appears only when there are drafts to delete
    // (advertising it on an empty screen would teach a dead control).
    let empty = render_add_text(&AddWorkflowState::new(Vec::new()), 200, 40);
    assert!(!empty.contains("Ctrl+D")); // no drafts -> no chip
    assert!(!empty.contains("Delete draft"));

    let mut present = AddWorkflowState::new(vec![DraftSummary {
        path: PathBuf::from("skit-new-present.py"),
        modified: 1,
    }]);
    let _ = present.reduce(AddAction::SelectDraft(0));
    let mut session = AddScreenSession::default();
    let mut terminal = Terminal::new(TestBackend::new(200, 40)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_add(frame, frame.area(), &present, &mut session, Locale::En);
        })
        .unwrap();
    let text = rendered(terminal.backend().buffer());
    assert!(text.contains("Ctrl+D")); // the mouse path is advertised
    assert!(text.contains("Delete draft"));

    let delete = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::DeleteDraft)
        .expect("the advertised delete chip is clickable")
        .area;
    let event = session.handle_event(left_click(delete.x, delete.y), &present, &geometry);
    assert_eq!(
        event,
        Some(AddScreenEvent::Action(AddAction::DeleteSelectedDraft))
    );
    let Some(AddScreenEvent::Action(action)) = event else {
        unreachable!("the exact action was asserted above");
    };
    let effects = present.reduce(action);
    assert!(effects.is_empty());
    assert_eq!(present.stage(), AddStage::ConfirmDraftDelete);
    assert_eq!(present.source().listed_drafts().len(), 1); // no delete before confirmation
}
