//! Mechanical port of the Python oracle module `tests/test_add_review_contracts.py`
//! (`origin/main@206f9ef`): "Add-review TUI contracts — pilot coverage." Each `#[test]`
//! keeps its Python `def test_*` name and its "WHY" rationale, and drives the real
//! public API.
//!
//! The Python module exercises `src/skit/tui_add.py` — the Textual `AddReviewScreen`,
//! `AddSourceScreen`, `KindPickModal`, and `PromptReviewScreen`. That impl is split across
//! two Rust tiers, and `skit-tui` reaches both, so every test here is real (0 ignored).
//! This module never drives the `skit` CLI binary, so the `skit-cli` crate hint does not
//! apply; no store or filesystem is touched, so the `SKIT_*_DIR` sandbox rule is moot.
//!
//! Concept mapping used throughout:
//! - Python `AddReviewScreen(path, kind=…)` -> `ReviewState::from_source(SourceSnapshot,
//!   KnownEntryKind, ReviewDefaults)` (skit-ui).
//! - Python `AddSourceScreen` draft-resume / `KindPickModal` routing -> the `AddWorkflowState`
//!   reducer (`SetSourcePath` -> `Continue` -> `SourceInspected`) (skit-ui).
//! - Python `review._reader_modeled()` (`flows.reader_fields(spec, text) > 0`) ->
//!   `review.modeled_cli_field_count().is_some_and(|count| count > 0)`.
//! - Python `review.query("#rv-cand-i")` checkboxes -> `review.candidates()`.
//! - Python `#rv-name` value -> `review.set_name`.
//! - Python `#rv-python` value / editability -> `review.requires_python()` /
//!   `review.set_requires_python`; a rescan edit -> `review.rescan(bytes)`.
//! - Python `review._analysis.uses_argv` / `.uses_cli_framework` ->
//!   `review.onboarding().uses_argv` / `.uses_cli_framework()`.
//! - Python `action_accept()` + `store.resolve(slug).script_path.read_text()` ->
//!   `review.create_entry().payload.bytes` — the exact bytes a copy commit stores; the store
//!   adapter that writes them is another tier's contract.
//! - Python `store.resolve(slug).meta.mode` / `.meta.kind` -> `create_entry().mode` / `.kind`.
//! - Python `review._py_pin_auto is False` -> the field is private in Rust; the asserted
//!   observable is `requires_python()` after the rescan (theirs beats the auto pin).
//! - Python rendered widgets (`KindPickModal` `Label`, `#rv-ref-note` `Static`, the reader
//!   `✓` notice, the "extra-arguments field" hints) -> the buffer text produced by
//!   `render_with_session(frame, LibraryState presenting Screen::Add, Locale::En, TuiSession)`
//!   (skit-tui).
//! - Python `_flip_mode(review, 1)` (the `RadioSet.Changed` mode-change path) ->
//!   `Action::Add(AddAction::SetReviewStorage(StorageMode::Reference))` through the root.
//! - Python `PromptReviewScreen` lane -> `review.lane() == ReviewLane::Prompt`.
//! - Python `review._fresh` / "no `#rv-mode`" -> `review.is_fresh()` gates the rendered
//!   Storage section.
//! - Python `body.scroll_offset.y > 0` -> `ViewGeometry.first_visible > 0` (through the root).
//!   Test 8 also needs the FOCUSED control and its region, which the root's `ViewGeometry` does
//!   not carry (the add hits are empty there), so it drives the re-exported `AddScreenSession`/
//!   `render_add` one tier down — the very session `TuiSession` owns and delegates to: Python
//!   `app.focused` -> `AddScreenSession::focused()`; `focused.region` inside `body.region` ->
//!   the focused control's `AddScreenGeometry.hits` rect inside `AddScreenGeometry.body`;
//!   `body.max_scroll_y > 0` -> the last checkbox absent from the initial render's hits.
//!
//! Buckets: every Python `def` maps to an API that exists, so all 12 are real asserting
//! tests. No cross-crate stub, no absent gap, no divergence.

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use skit_application::SourcePermissions;
use skit_domain::{EntryKind, StorageMode};
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenGeometry, AddScreenSession, TuiSession, ViewGeometry, render_add,
    render_with_session,
};
use skit_ui::{
    Action, AddAction, AddEffect, AddWorkflowState, KnownEntryKind, LibraryState, ReviewDefaults,
    ReviewLane, ReviewState, Screen, SourceSnapshot,
};

// The oracle's two module-level shell fixtures.
const DYN_SH: &[u8] =
    b"#!/usr/bin/env bash\nOUTDIR=/tmp\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho $OUTDIR\n";
const MODELED_SH: &[u8] =
    b"#!/usr/bin/env bash\nCITY=Taipei\nwhile getopts 'n:v' o; do :; done\necho $CITY\n";
// The oracle's section-8 constant-heavy Python fixture.
const CONST_PY: &[u8] = b"MESSAGE = 'Hello'\nTIMES = 3\nWIDTH = 40\nprint(MESSAGE)\n";

// The exact renderer strings this module asserts on (skit-tui/src/screens/add.rs).
const TICK_PROMPT: &str = "Tick the ones the run form should ask for:";
const READER_NOTICE: &str = "skit read this script's own arguments";
const REF_NOTE_MODELED: &str = "Link the original: skit never writes to the file.";
const REF_NOTE_UNMODELED: &str = "parameter setup is skipped";

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
        executable: None,
        is_regular: true,
        is_directory: false,
        is_draft: draft,
        identity: None,
    }
}

/// Convenience: analyze one snapshot into a review, as `AddReviewScreen(path, kind=…)` does.
fn review_of(
    path: &str,
    bytes: &[u8],
    kind: KnownEntryKind,
    defaults: ReviewDefaults,
) -> ReviewState {
    ReviewState::from_source(snap(path, bytes, false), kind, defaults)
}

/// Python `review._reader_modeled()`: the reader models a nonempty static form.
fn reader_modeled(review: &ReviewState) -> bool {
    review
        .modeled_cli_field_count()
        .is_some_and(|count| count > 0)
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

/// Present one add workflow through the composition root, ready to render.
fn present(workflow: AddWorkflowState) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(workflow))));
    state
}

/// Render one library state onto a fixed test backend.
fn draw(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| geometry = render_with_session(frame, state, Locale::En, session))
        .unwrap();
    (terminal, geometry)
}

/// The rendered buffer flattened to one string, like the oracle's `_statics` join.
fn rendered(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

/// Render one add workflow directly through the re-exported `AddScreenSession`/`render_add`
/// (the tier `TuiSession` delegates to) and return its geometry — the `hits` carry each visible
/// control's rendered rect, which the root's `ViewGeometry` drops for the add screen.
fn draw_add(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
    width: u16,
    height: u16,
) -> AddScreenGeometry {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| geometry = render_add(frame, frame.area(), state, session, Locale::En))
        .unwrap();
    geometry
}

/// Decode the exact bytes a copy commit would store from one accepted review.
fn stored_copy(review: &ReviewState) -> String {
    let plan = review.create_entry().expect("the review accepts");
    String::from_utf8(plan.payload.expect("a copy payload").bytes).expect("utf-8 stored copy")
}

// ==========================================================================
// 1. Ticked candidates are WRITTEN on an unmodeled self-parser
// ==========================================================================

#[test]
fn test_high_unmodeled_self_parser_writes_ticked_candidate() {
    // A dynamic-optstring shell self-parses (uses_cli_framework) but can't be modeled, so the
    // candidate ticks render and are actually collected on accept: the stored copy's [tool.skit]
    // block holds the ticked constant. The old gate (not uses_cli_framework) dropped it silently.
    let mut review = review_of(
        "dyn.sh",
        DYN_SH,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(!reader_modeled(&review)); // unmodeled -> the ticks are additive
    // The checkbox the accept gate must honor: candidate 0 is OUTDIR.
    assert_eq!(review.candidates()[0].declaration.name, "OUTDIR");
    // The ticks render (the oracle's `query("#rv-cand-0")`): the tick list mounts on screen.
    let mut session = TuiSession::default();
    let (terminal, _) = draw(
        &mut session,
        &present(AddWorkflowState::from_review(review.clone())),
        120,
        40,
    );
    assert!(rendered(terminal.backend().buffer()).contains(TICK_PROMPT));
    review.set_name("dynh");
    review.set_candidate_selected("OUTDIR", true); // tick the constant
    let stored = stored_copy(&review);
    assert!(stored.contains("[tool.skit]")); // the block was written at all (it was dropped before)
    assert!(stored.contains("name = \"OUTDIR\"")); // ...and holds the ticked constant
}

#[test]
fn test_high_modeled_form_collects_nothing_without_crashing() {
    // The complement: a MODELED getopts form has no checkboxes, so the collection gate skips —
    // accept commits the entry and never queries a candidate that doesn't exist (the crash the
    // gate must not cause).
    let mut review = review_of(
        "mod.sh",
        MODELED_SH,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(reader_modeled(&review));
    assert!(review.candidates().is_empty());
    review.set_name("modh");
    let plan = review.create_entry().expect("accept must not raise");
    assert_eq!(plan.kind, EntryKind::parse("shell".to_owned()).unwrap());
}

// ==========================================================================
// 2. A .prompt.md kept draft with a #! body resumes into the PromptReviewScreen
// ==========================================================================

#[test]
fn test_prompt_draft_with_shebang_body_resumes_into_prompt_review() {
    // Resuming `skit-new-*.prompt.md` (bash-shebang body) opens the PROMPT review lane, not the
    // shell one — the compound suffix is the user's lane choice (kind_for_draft).
    let draft = snap(
        "skit-new-p.prompt.md",
        b"#!/usr/bin/env bash\nSummarize {{text}}.\n",
        true,
    );
    let workflow = route_source(draft);
    let review = workflow.review().expect("the draft resumes into a review");
    assert_eq!(review.lane(), ReviewLane::Prompt); // prompt lane, not the script lane
}

// ==========================================================================
// 3. Reference-mode note is reader-aware (modeled keeps the wrap; unmodeled folds)
// ==========================================================================

#[test]
fn test_reference_note_modeled_keeps_wrap_and_short_line() {
    // A MODELED getopts script in reference mode keeps the params wrap visible (the ✓ notice
    // stays — the reader works in reference mode) and the note is the short "never writes to the
    // file" line; accept in reference mode does not crash and the entry is a reference.
    let review = review_of(
        "mod.sh",
        MODELED_SH,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    let mut state = present(AddWorkflowState::from_review(review.clone()));
    let mut session = TuiSession::default();

    // Positive control: in COPY mode the reader wrap renders and there is no reference note yet.
    // This makes the "wrap survives the flip" claim below discriminate the mode dimension — the
    // oracle's `#rv-params-wrap.display is True` (tui_add.py:827, `display = not reference or
    // modeled`), which the sibling unmodeled test shows a reference flip otherwise folds. A bare
    // `contains(READER_NOTICE)` in reference mode was near-vacuous: add.rs pushes the reader
    // notice with no mode gate, so it could not fail on the mode axis.
    let (copy_terminal, _) = draw(&mut session, &state, 120, 40);
    let copy_screen = rendered(copy_terminal.backend().buffer());
    assert!(copy_screen.contains(READER_NOTICE)); // reader wrap present before the flip
    assert!(!copy_screen.contains(REF_NOTE_MODELED)); // ...and no reference note in copy mode

    // Flip via the mode-change path (RadioSet.Changed twin), through the composition root.
    state.update(Action::Add(AddAction::SetReviewStorage(
        StorageMode::Reference,
    )));
    let (terminal, _) = draw(&mut session, &state, 120, 40);
    let screen = rendered(terminal.backend().buffer());
    assert!(screen.contains(REF_NOTE_MODELED)); // modeled -> the short line (the flip took effect)
    assert!(!screen.contains(REF_NOTE_UNMODELED));
    assert!(screen.contains(READER_NOTICE)); // ...and the ✓ reader wrap SURVIVES the flip (kept)

    // accept -> reference: create_entry records the reference mode.
    let mut accepted = review;
    accepted.set_storage(StorageMode::Reference);
    accepted.set_name("modref");
    assert_eq!(
        accepted.create_entry().expect("reference accept").mode,
        StorageMode::Reference
    );
}

#[test]
fn test_reference_note_unmodeled_folds_and_keeps_old_line() {
    // An UNMODELED script (dynamic optstring) in reference mode folds the params wrap and keeps
    // the old "parameter setup is skipped" line — nothing to preserve, so say so plainly.
    let review = review_of(
        "dyn.sh",
        DYN_SH,
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    let mut state = present(AddWorkflowState::from_review(review));
    let mut session = TuiSession::default();

    // Positive control: in copy mode the tick prompt renders, so its later absence is meaningful.
    let (copy_terminal, _) = draw(&mut session, &state, 120, 40);
    assert!(rendered(copy_terminal.backend().buffer()).contains(TICK_PROMPT));

    state.update(Action::Add(AddAction::SetReviewStorage(
        StorageMode::Reference,
    )));
    let (ref_terminal, _) = draw(&mut session, &state, 120, 40);
    let screen = rendered(ref_terminal.backend().buffer());
    assert!(screen.contains(REF_NOTE_UNMODELED)); // unmodeled -> the old line
    assert!(!screen.contains(TICK_PROMPT)); // ...and the wrap folds
    assert!(!screen.contains(REF_NOTE_MODELED)); // not the short modeled line
}

// ==========================================================================
// 4. KindPickModal label switches on has_shebang
// ==========================================================================

#[test]
fn test_kind_pick_modal_label_switches_on_shebang() {
    // With a #! present, "can't tell from the name" is false — the label instead explains the
    // unknown interpreter. Without one, the name told skit nothing.
    let with_shebang = route_source(snap(
        "foo.xyz",
        b"#!/usr/bin/env florbleflarg\nblah\n",
        false,
    ));
    let state = present(with_shebang);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 100, 40);
    assert!(
        rendered(terminal.backend().buffer())
            .contains("The #! in foo.xyz names no interpreter skit knows. What is it?")
    );

    let without_shebang = route_source(snap("foo.xyz", b"plain text\n", false));
    let state = present(without_shebang);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 100, 40);
    assert!(
        rendered(terminal.backend().buffer())
            .contains("What is foo.xyz? skit can't tell from the name.")
    );
}

// ==========================================================================
// 5. The extra-arguments field is named exactly once in the review panel
// ==========================================================================

#[test]
fn test_review_names_extra_arguments_field_once() {
    // A dynamic-optstring shell that ALSO reads $@ (uses_argv AND a framework) mentions the
    // extra-arguments field exactly ONCE — the reader notice, with the argv info hint suppressed.
    let review = review_of(
        "dynargv.sh",
        b"#!/usr/bin/env bash\nOPTS=\"n:v\"\nwhile getopts \"$OPTS\" o; do :; done\necho \"$@\"\n",
        KnownEntryKind::Shell,
        ReviewDefaults::default(),
    );
    assert!(review.onboarding().uses_argv); // reads $@
    assert!(review.onboarding().uses_cli_framework()); // ...and getopts self-parses

    let state = present(AddWorkflowState::from_review(review));
    let mut session = TuiSession::default();
    // A wide backend keeps each notice on one line so the count is exact.
    let (terminal, _) = draw(&mut session, &state, 200, 50);
    let screen = rendered(terminal.backend().buffer());
    assert_eq!(screen.matches("extra-arguments field").count(), 1);
    // The framework notice is the one that names it; the argv-only hint is suppressed.
    assert!(screen.contains("so the run form offers an extra-arguments field"));
    assert!(!screen.contains("the run form has an extra-arguments field for them"));
}

// ==========================================================================
// 6. The #rv-python field is editable
// ==========================================================================

#[test]
fn test_rv_python_typed_constraint_lands_in_stored_copy() {
    // Typing a constraint into #rv-python records it verbatim in the stored copy's PEP 723 block.
    let mut review = review_of(
        "plain.py",
        b"print(1)\n",
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), ""); // no auto pin (no versioned shebang)
    review.set_requires_python(">=3.10");
    review.set_name("pytyped");
    assert!(stored_copy(&review).contains("requires-python = \">=3.10\""));
}

#[test]
fn test_rv_python_empty_means_automatic() {
    // Clearing #rv-python records NO requires-python — automatic. With no deps either, the stored
    // copy carries no PEP 723 block at all.
    let mut review = review_of(
        "plain.py",
        b"print(1)\n",
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    review.set_requires_python(""); // explicit clear
    review.set_name("pyauto");
    let stored = stored_copy(&review);
    assert!(!stored.contains("requires-python")); // automatic -> nothing recorded
    assert!(!stored.contains("# ///")); // ...and no PEP 723 block fence at all
}

#[test]
fn test_rv_python_typed_value_survives_an_edit_rescan() {
    // A typed constraint is the user's own value: it survives an edit->rescan even when the
    // shebang changes underneath (theirs beats the auto pin). `_py_pin_auto` is private in Rust;
    // the observable is that the typed value stays and does not become the new shebang's auto pin.
    let mut review = review_of(
        "v.py",
        b"#!/usr/bin/env python3.12\nprint(1)\n",
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    assert_eq!(review.requires_python(), ">=3.12,<3.13"); // auto pin
    review.set_requires_python(">=3.9"); // the user's own constraint
    review.rescan(b"#!/usr/bin/env python3.11\nprint(1)\n".to_vec()); // shebang moves
    // The auto pin would be >=3.11,<3.12; the typed override wins instead.
    assert_eq!(review.requires_python(), ">=3.9");
    assert_ne!(review.requires_python(), ">=3.11,<3.12");
}

// ==========================================================================
// 7. A resumed draft shows NO Storage section (fresh)
// ==========================================================================

#[test]
fn test_resumed_draft_has_no_storage_section() {
    // Resuming a kept draft is fresh authoring (fresh=True): there's no original to link, so the
    // Storage radio set is absent — a --ref there would have made the delete-confirm's "only copy"
    // a lie.
    let draft = snap("skit-new-fresh.py", b"print('fresh')\n", true);
    let workflow = route_source(draft);
    let review = workflow.review().expect("the draft resumes into a review");
    assert!(review.is_fresh()); // fresh resume: the state that withholds the Storage ask

    let state = present(workflow);
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 120, 40);
    // The Storage section (the "Keep a copy" radio, storage_options in the renderer) is absent.
    assert!(!rendered(terminal.backend().buffer()).contains("Keep a copy — skit stores it"));
}

// ==========================================================================
// 8. On a short terminal, focus scrolls the candidate checkboxes into view
// ==========================================================================

#[test]
fn test_short_terminal_scrolls_focused_candidate_into_view() {
    // Tabbing down the review panel keeps the focused widget reachable at a terminal too short to
    // hold the whole form: every focus stop stays inside the body viewport, the walk reaches the
    // last checkbox, and getting there moves the viewport. The root's `ViewGeometry` carries no
    // focused control or per-control rect for the add screen, so this drives the re-exported
    // `AddScreenSession`/`render_add` directly (the same session `TuiSession` owns).
    let review = review_of(
        "banner.py",
        CONST_PY,
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    // The oracle's `query("Checkbox").last()`: the last parser-backed candidate.
    let last = AddControlId::Candidate(
        review
            .candidates()
            .last()
            .expect("the const-heavy python yields candidates")
            .declaration
            .name
            .clone(),
    );
    let state = AddWorkflowState::from_review(review);
    let mut session = AddScreenSession::default();

    // A body too short to hold the whole review form (the oracle's 106x30 is through the app
    // chrome; driven here directly the height is the OVERFLOWING body itself — "too short to hold
    // the form" is the contract, the exact row count is scenario). The form opens at the top.
    let mut geometry = draw_add(&mut session, &state, 106, 12);
    assert_eq!(geometry.first_visible, 0);
    // The reachable twin of `body.max_scroll_y > 0`: the last checkbox starts below the fold, so
    // it is not among the rendered hits (the body must overflow for this to hold).
    assert!(
        !geometry.hits.iter().any(|hit| hit.target == last),
        "the last checkbox should start below the fold"
    );

    // Tab down. At EVERY focus stop the focused control stays inside the body viewport: it is
    // present in the hits (rendered, i.e. scrolled into view) with its rect within `body`. The hit
    // rect is viewport-clipped, so this asserts the focused row is on screen — exactly what
    // scroll-into-view guarantees — and it catches the real regression (focus below the fold with
    // nothing scrolling). The walk must reach the last checkbox.
    for _ in 0..40 {
        if let Some(focused) = session.focused().cloned() {
            let hit = geometry
                .hits
                .iter()
                .find(|hit| hit.target == focused)
                .expect("the focused control is on screen (scrolled into view)");
            assert!(
                geometry.body.y <= hit.area.y,
                "{focused:?} scrolled off the top"
            );
            assert!(
                hit.area.y + hit.area.height <= geometry.body.y + geometry.body.height,
                "{focused:?} scrolled off below"
            );
            if focused == last {
                break;
            }
        }
        let handled = session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &state,
            &geometry,
        );
        assert!(handled.is_some());
        geometry = draw_add(&mut session, &state, 106, 12);
    }
    assert_eq!(session.focused(), Some(&last)); // the walk actually reached the last checkbox
    assert!(geometry.first_visible > 0); // ...and getting there moved the viewport
}
