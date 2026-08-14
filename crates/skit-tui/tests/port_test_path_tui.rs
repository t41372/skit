//! Mechanical port of the Python oracle module `tests/test_path_tui.py`
//! (`origin/main@206f9ef`): "The path-entry TUI layer (docs/design/path.md P1b): the
//! ghost suggester's activation and root rules, the file-picker modal's keys, and the
//! per-shape insertion semantics." Each `#[test]` keeps its Python `def test_*` name and
//! its "WHY" rationale, driving the real public API.
//!
//! Crate choice (skit-tui): the oracle module `src/skit/tui_pathpick.py` maps onto three
//! Rust tiers, and only `skit-tui` reaches all three through its own `[dependencies]`
//! (`skit-application`, `skit-ui`, `skit-tui` itself). Textual's async `pilot` press/click
//! maps to synchronous `FilePickerSession::handle_event` and the `LibraryState` reducer, so
//! no async runtime is needed.
//!
//! Concept mapping used throughout:
//! - Python `tui_pathpick.insert_picked(box, PickedPath(text), mode=…)` ->
//!   `skit_application::path_insertion::insert_picked_path_for_dialect(existing, text, mode,
//!   dialect)` (the `sys.platform` split becomes an explicit `ArgumentDialect`).
//! - Python `argv_text.split(value)` -> `runner_management::split_editable_argv(value, dialect)`.
//! - Python `PathContext.value_for(target)` -> `PathPickerState::output_path` under
//!   `PathOutputPolicy::RelativeTo(workdir)` (skit-ui). The empty relative path renders as "."
//!   later, in `skit-tui`'s private `run_modal::picked_path_text`.
//! - Python `PathContext.picker_start()` degradation -> `FilePickerSession::new` +
//!   `nearest_directory` (skit-tui); the missing-root bool is `run_modal::file_picker_contract`
//!   (private).
//! - Python `FilePickerModal` (Textual pilot) -> `FilePickerSession` + `render_file_picker`
//!   (skit-tui): the pinned "(use this directory)" OptionList row is the mouse-only
//!   `FilePickerHit::CurrentDirectory`; the parent step is a real `..` `EntryType::ParentDir`
//!   row; `_list_filtered`'s rank is the free function `picker::apply_filter`.
//! - Python `RunFormScreen` browse/insert doors -> the `LibraryState` reducer
//!   (`Action::OpenRunTokenMenu…`, `OpenRunFilePicker`, `OpenFocusedRunFilePicker`,
//!   `SetRunPickedPathAndCloseModal`) over a `RunFormView::from_declarations(...).with_context`.
//! - Python `FieldRow.browsable/insertable` -> `RunField::browsable/insertable` and
//!   `RunFormView::can_browse_field/can_insert_field`; `FieldRow.insert_mode` -> the
//!   `ModalState::RunFilePicker { mode }` published when the picker opens.
//!
//! Buckets:
//! - REAL (asserting, passing): the insertion shapes, `value_for`, the token/browse/mode
//!   reducer flow, `_list_filtered` ranking, and most picker keys/mouse.
//! - ABSENT gaps (compiling `#[ignore]` stubs): the entire ghost-text suggester surface
//!   (`PathSuggester.get_suggestion`, `_lookup`, `_list_matches`, `_trailing_piece`,
//!   `_get_suggestion`, brace/quote/token internals) and `looks_pathy` have NO Rust
//!   equivalent — the rewrite ships a file picker but no append-only ghost completion. This
//!   is the headline finding of this port (superset-rule feature loss).
//! - CROSS-CRATE stubs: `PathContext.for_entry` workdir/origin resolution is composed in
//!   `skit-cli` (`cli.rs` builds `RunPathContext`); the cursor-position token insert is owned
//!   by skit-tui's interactive `TuiSession` cursor layer (not the reducer surface here).
//! - DIVERGENCE (full body, `#[ignore = "FAILING CONTRACT (divergence): …"]`): the mouse-only
//!   use-this-directory affordance (no keyboard route), absent PageUp/PageDown steering, the
//!   "(use this directory)" label, and the glob-escape byte spelling (`glob` escapes `]` too).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::path_insertion::{
    ArgumentDialect, RunPathInsertMode, insert_picked_path_for_dialect,
};
use skit_application::runner_management::{EditableArgvDialect, split_editable_argv};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_i18n::Locale;
use skit_tui::{
    FilePickerEvent, FilePickerGeometry, FilePickerHit, FilePickerSession, render_file_picker,
};
use skit_ui::{
    Action, LibraryState, ModalState, PathOutputPolicy, PathPickerState, PathSelectionMode,
    PickerPurpose, RunFormContext, RunFormView, RunPathContext, RunTokenOption, Screen,
};
use tempfile::TempDir;

// --- Filesystem and event helpers (the oracle's `_tree`, `pilot.press`, `pilot.click`) ---

/// The oracle's `_tree`: root/{data.csv, draft.txt, sub/{inner.txt}, .hidden}.
fn tree() -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("sub")).unwrap();
    fs::write(root.join("data.csv"), "x").unwrap();
    fs::write(root.join("draft.txt"), "x").unwrap();
    fs::write(root.join("sub").join("inner.txt"), "x").unwrap();
    fs::write(root.join(".hidden"), "x").unwrap();
    (tmp, root)
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn click(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// The production picker contract: `run_modal::file_picker_contract` builds exactly this
/// (`PickerPurpose::Argument`, file-or-directory, relative to the workdir, single).
fn contract(start: &Path, workdir: &Path) -> PathPickerState {
    PathPickerState::new(
        PickerPurpose::Argument,
        start.to_path_buf(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(workdir.to_path_buf()),
        false,
    )
}

/// Open a picker whose start and relative root are the same directory.
fn picker(dir: &Path) -> FilePickerSession {
    FilePickerSession::new(contract(dir, dir))
}

fn feed(session: &mut FilePickerSession, event: Event) -> Option<FilePickerEvent> {
    session.handle_event(event, &FilePickerGeometry::default())
}

fn typed(session: &mut FilePickerSession, text: &str) {
    for character in text.chars() {
        let _ = feed(session, key(KeyCode::Char(character)));
    }
}

/// The picker's visible listing as `(name, is_dir)`, excluding the `..` parent row — the
/// closest structural analog of the Textual OptionList's real (non-pinned) rows.
fn listing(session: &FilePickerSession) -> Vec<(String, bool)> {
    let explorer = session.explorer();
    let order = explorer
        .filtered_indices
        .clone()
        .unwrap_or_else(|| (0..explorer.entries.len()).collect());
    order
        .iter()
        .filter_map(|&index| explorer.entries.get(index))
        .filter(|entry| entry.name != "..")
        .map(|entry| (entry.name.clone(), entry.is_dir()))
        .collect()
}

/// Render the picker once so mouse tests can read the hit geometry and buffer.
fn render(
    session: &mut FilePickerSession,
    width: u16,
    height: u16,
) -> (String, FilePickerGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = FilePickerGeometry::default();
    terminal
        .draw(|frame| geometry = render_file_picker(frame, frame.area(), session, Locale::En))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let text = buffer.content().iter().map(|cell| cell.symbol()).collect();
    (text, geometry)
}

// --- Run-form reducer helpers (the oracle's RunFormScreen / FieldRow) ---

fn param(name: &str, parameter_type: ParameterType, multiple: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.parameter_type = parameter_type;
    declaration.multiple = multiple;
    declaration
}

/// Build a presented launch form and its reducer state. `workdir` (when set) supplies the
/// path completion roots that gate the file-picker door.
fn form_state(
    declarations: &[ParamDecl],
    saved: &[(&str, &str)],
    extra: &str,
    workdir: Option<&str>,
) -> LibraryState {
    let saved = saved
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        declarations,
        &saved,
        &[],
        "",
        &BTreeMap::new(),
        extra,
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: workdir.map(|workdir| RunPathContext {
            workdir: workdir.to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: Some("/home/demo".to_owned()),
            env: BTreeMap::new(),
            today: "2026-08-10".to_owned(),
            now: "10-11-12".to_owned(),
        },
    });
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

fn field_value(state: &LibraryState, index: usize) -> String {
    state.run_form().unwrap().fields()[index].control.value()
}

fn token_options(state: &LibraryState) -> Vec<RunTokenOption> {
    match state.modal() {
        Some(ModalState::RunTokenMenu { options, .. }) => options.clone(),
        other => panic!("expected an open token menu, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// PathSuggester: activation and the three-coordinate-system roots (path.md §3-§4)
//
// ABSENT (gap): the Rust rewrite ships the file picker but NO append-only ghost-text
// suggester. There is no `get_suggestion`/`_lookup`/`_list_matches`/`looks_pathy` on any
// reachable tier (`FormInputKind::Path` documents "completion" but nothing implements it).
// Each of the following is a compiling `#[ignore]` stub recording the Python contract; the
// MUST-FIX is to restore the ghost suggester (`src/skit/tui_pathpick.py:175-256`).
// ---------------------------------------------------------------------------

#[test]
#[ignore = "ABSENT gap: no ghost-text PathSuggester in the Rust surface. Oracle: tui_pathpick.PathSuggester.get_suggestion completes a bare prefix at the workdir ('da'->'data.csv', 'su'->'sub/'). MUST-FIX: restore the suggester (src/skit/tui_pathpick.py:194-216)."]
fn test_path_field_completes_bare_prefix_at_workdir() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a `str` field only completes path-shaped text ('da'->None, './da'->'./data.csv', 'sub/in'->'sub/inner.txt') via looks_pathy (tui_pathpick.py:200-201)."]
fn test_str_field_needs_pathy_text() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: 'zzz'->None; the suggester never invents beyond a real prefix (tui_pathpick.py:212-216)."]
fn test_secretless_activation_never_guesses_beyond_prefix() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: '.h'->'.hidden' but 'd'->'data.csv' — hidden entries surface only behind a dot prefix (tui_pathpick.py:159, _list_matches)."]
fn test_hidden_entries_only_behind_a_dot_prefix() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: '{cwd}/no'->'{cwd}/notes.md' completes at the invoke cwd, not the workdir (tui_pathpick.py:241-244, _lookup token expansion)."]
fn test_cwd_token_completes_at_invoke_cwd_not_workdir() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: '{env:SKIT_NO_SUCH_VAR}/d'->None (unexpandable token is silence, not a traceback) (tui_pathpick.py:242-246)."]
fn test_unset_env_token_is_silence_not_a_traceback() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a relative env token falls back to the workdir rule: '{env:SKIT_REL_DIR}/in'->'{env:SKIT_REL_DIR}/inner.txt' (tui_pathpick.py:250-256)."]
fn test_relative_env_token_falls_back_to_the_workdir_rule() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: '~/no'->'~/notes.md' completes inside HOME; '~'->None with no separator yet (tui_pathpick.py:236-239)."]
fn test_home_prefix_completes_inside_home() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a vanished workdir silences bare completion, 'da'->None (bare_root is None) (tui_pathpick.py:90-94, 238-240)."]
fn test_missing_workdir_silences_bare_completion() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a vanished workdir also silences the relative-token arm '{env:SKIT_REL_DIR}/in'->None (tui_pathpick.py:251-255)."]
fn test_missing_workdir_silences_relative_token_lookup() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a shlexy field completes only the trailing piece ('first.txt dr'->'first.txt draft.txt'), refuses a quote-in-progress, and refuses an empty trailing piece (tui_pathpick.py:218-227)."]
fn test_shlexy_field_completes_only_the_trailing_piece() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester (and no SCAN_CAP). Oracle: with SCAN_CAP=3 an entry in scan position 4 is never offered — pins the >= boundary (tui_pathpick.py:60, 139-141)."]
fn test_scan_cap_stops_the_scan_exactly() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a scandir OSError degrades to no suggestion and _list_filtered==[] (tui_pathpick.py:150-152)."]
fn test_scan_degrades_on_oserror() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: an entry that raises mid-stat is treated as a plain file, still offered ('da'->'dax') (tui_pathpick.py:145-149)."]
fn test_unstatable_entry_is_treated_as_a_file() {}

// ---------------------------------------------------------------------------
// PathContext: roots and inserted spellings (path.md §3, §5)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-cli): PathContext.for_entry (launcher._resolve_workdir + Path.cwd) is composed where RunPathContext is built (skit-cli/src/cli.rs); skit-ui's RunPathContext is a plain {workdir, invoke_cwd} data value with no for_entry. Oracle: an explicit absolute meta.workdir becomes ctx.workdir; ctx.invoke_cwd == Path.cwd (test_path_tui.py:199-206)."]
fn test_for_entry_resolves_the_entry_workdir() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): reference-mode workdir resolution (root at the origin) is composed in skit-cli, not reachable here. Oracle: a reference entry roots at its origin dir (test_path_tui.py:209-215)."]
fn test_for_entry_reference_entry_roots_at_its_origin() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli) + ABSENT: reference-mode workdir survival is skit-cli composition, and PathContext.bare_root has no Rust equivalent (it fed only the absent ghost). The picker_start degradation IS covered by test_picker_start_degrades_to_nearest_existing_ancestor. Oracle: workdir==origin, bare_root is None, picker_start()==(tmp/proj, True) (test_path_tui.py:218-234)."]
fn test_vanished_origin_reference_entry_degrades() {}

#[test]
#[ignore = "CROSS-CRATE (private) + UNPORTABLE: the whole-ancestor-chain-gone last resort lives in run_modal::file_picker_contract's `unwrap_or_else(invoke_cwd)`; FilePickerSession::new's nearest_directory falls back to std::env::current_dir. Neither is portably reachable — `/` always exists and Path::is_dir cannot be faked (needs the Python monkeypatch of Path.is_dir). Oracle: picker_start()==(invoke_cwd, True) (test_path_tui.py:237-242)."]
fn test_picker_start_last_resort_is_the_invoke_cwd() {}

#[test]
fn test_picker_start_degrades_to_nearest_existing_ancestor() {
    // A gone workdir opens the picker at the nearest existing ancestor. (The companion
    // `missing` bool is run_modal::file_picker_contract's, private to the render layer.)
    let tmp = tempfile::tempdir().unwrap();
    let gone = tmp.path().join("was").join("here");
    let session = FilePickerSession::new(contract(&gone, &gone));
    assert_eq!(session.current_dir(), &tmp.path().to_path_buf());
}

#[test]
fn test_value_for_is_relative_inside_the_root_and_posix_everywhere() {
    let (tmp, root) = tree();
    let state = contract(&root, &root);
    assert_eq!(
        state.output_path(&root.join("sub").join("inner.txt")),
        PathBuf::from("sub/inner.txt")
    );
    // private-render: the oracle's value_for(root)=='.' is produced by picked_path_text
    // (run_modal.rs:665), a private free function reached only when the run-modal file picker
    // accepts a directory; the reducer-level observable this test drives is the empty relative path
    // (which picked_path_text later renders as ".").
    assert_eq!(state.output_path(&root), PathBuf::new());
    let outside = tmp.path().join("other.txt");
    assert_eq!(state.output_path(&outside), outside);
}

// ---------------------------------------------------------------------------
// FilePickerModal: every advertised key, plus the mouse path (path.md §5)
// ---------------------------------------------------------------------------

#[test]
fn test_picker_enter_descends_then_picks_and_filter_clears() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    // The highlight sits on the first real entry — sub/ (dirs sort first here).
    assert_eq!(session.explorer().current_entry().unwrap().name, "sub");
    typed(&mut session, "su");
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(session.current_dir(), &root.join("sub"));
    assert!(session.explorer().filtered_indices.is_none()); // filter cleared on descend
    // Highlight: first real entry = inner.txt.
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from(
            "sub/inner.txt"
        )]))
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the use-this-directory affordance is mouse-only (FilePickerHit::CurrentDirectory); there is no keyboard route. Up from the first real entry lands on the real `..` ParentDir row, so Enter ASCENDS to the parent instead of selecting the current directory. Oracle: up+enter picks PickedPath('.') (test_path_tui.py:293-310)."]
fn test_picker_use_this_directory_row_by_real_keys() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    let _ = feed(&mut session, key(KeyCode::Up));
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from(".")]))
    );
}

#[test]
fn test_picker_arrows_steer_highlight_without_leaving_the_filter() {
    let (_tmp, root) = tree();
    fs::write(root.join("delta.md"), "x").unwrap();
    let mut session = picker(&root);
    typed(&mut session, "d");
    assert_eq!(session.explorer().visible_count(), 3);
    let start = session.explorer().cursor_index;
    assert_eq!(
        feed(&mut session, key(KeyCode::Down)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(session.explorer().cursor_index, start + 1);
    assert_eq!(
        feed(&mut session, key(KeyCode::Up)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(session.explorer().cursor_index, start);
    assert_eq!(
        feed(&mut session, key(KeyCode::End)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        session.explorer().cursor_index,
        session.explorer().visible_count() - 1
    );
    assert_eq!(
        feed(&mut session, key(KeyCode::Home)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(session.explorer().cursor_index, 0);
    assert_eq!(
        feed(&mut session, key(KeyCode::Down)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(session.explorer().cursor_index, start + 1);
    assert_eq!(
        feed(&mut session, key(KeyCode::PageDown)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        session.explorer().cursor_index,
        session.explorer().visible_count() - 1
    );
    assert_eq!(
        feed(&mut session, key(KeyCode::PageUp)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(session.explorer().cursor_index, 0);
    assert_eq!(
        listing(&session),
        vec![
            ("data.csv".to_owned(), false),
            ("delta.md".to_owned(), false),
            ("draft.txt".to_owned(), false),
        ]
    );
}

#[test]
fn test_picker_prefix_matches_outrank_substring_hits() {
    // Filter `da`: data.csv (prefix, a file) sits above Anaconda/ (a substring-matching
    // directory ASCII sort would float up); Enter picks what the user typed.
    let (_tmp, root) = tree();
    fs::create_dir(root.join("Anaconda")).unwrap();
    let mut session = picker(&root);
    typed(&mut session, "da");
    assert_eq!(
        listing(&session),
        vec![
            ("data.csv".to_owned(), false),
            ("Anaconda".to_owned(), true)
        ]
    );
    assert_eq!(session.explorer().cursor_index, 0);
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from("data.csv")]))
    );
}

#[test]
fn test_picker_filter_is_case_insensitive_substring() {
    // `eadm` must find README.md — the picker filters like EnvPickerModal.
    let (_tmp, root) = tree();
    fs::write(root.join("README.md"), "x").unwrap();
    let mut session = picker(&root);
    typed(&mut session, "eadm");
    assert_eq!(listing(&session), vec![("README.md".to_owned(), false)]);
}

#[test]
fn test_picker_row_click_is_the_mouse_path() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    typed(&mut session, "data");
    assert_eq!(listing(&session), vec![("data.csv".to_owned(), false)]);
    let (_text, geometry) = render(&mut session, 100, 30);
    let hit = geometry
        .hits
        .iter()
        .find(|hit| hit.target == FilePickerHit::Entry(0))
        .expect("a clickable row for data.csv");
    let event = click(hit.area.x + 2, hit.area.y);
    assert_eq!(
        session.handle_event(event, &geometry),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from("data.csv")]))
    );
}

#[test]
fn test_picker_zero_match_enter_is_a_noop() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    typed(&mut session, "zzz-no-such");
    assert_eq!(session.explorer().visible_count(), 0);
    assert_eq!(feed(&mut session, key(KeyCode::Enter)), None); // nothing highlighted
    assert_eq!(session.current_dir(), &root); // still alive at the same directory
}

#[test]
fn test_picker_filtering_hides_the_pinned_row() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    typed(&mut session, "d");
    assert_eq!(
        listing(&session),
        vec![
            ("data.csv".to_owned(), false),
            ("draft.txt".to_owned(), false)
        ]
    );
    // Enter acts on the first MATCH.
    assert_eq!(session.explorer().current_entry().unwrap().name, "data.csv");
    assert_eq!(session.explorer().cursor_index, 0);
}

#[test]
fn test_picker_backspace_ascends_only_on_empty_filter() {
    let (_tmp, root) = tree();
    let mut session = FilePickerSession::new(contract(&root.join("sub"), &root));
    typed(&mut session, "in");
    let _ = feed(&mut session, key(KeyCode::Backspace)); // editing: deletes, no ascend
    assert_eq!(session.current_dir(), &root.join("sub"));
    let _ = feed(&mut session, key(KeyCode::Backspace)); // now empty
    let _ = feed(&mut session, key(KeyCode::Backspace)); // empty: ascends to the parent
    assert_eq!(session.current_dir(), &root);
}

#[test]
fn test_picker_backspace_noops_at_the_filesystem_root() {
    // At the filesystem root there is no parent; an empty-filter Backspace is a no-op.
    let anchor = PathBuf::from("/");
    let mut session = FilePickerSession::new(contract(&anchor, &anchor));
    let _ = feed(&mut session, key(KeyCode::Backspace));
    assert_eq!(session.current_dir(), &anchor);
}

#[test]
fn test_picker_esc_cancels_and_up_chip_is_clickable() {
    let (_tmp, root) = tree();
    let mut session = FilePickerSession::new(contract(&root.join("sub"), &root));
    // Mouse path for ascend: click the Backspace/Up chip.
    let (_text, geometry) = render(&mut session, 100, 30);
    let up = geometry
        .hits
        .iter()
        .find(|hit| hit.target == FilePickerHit::Up)
        .expect("an Up chip");
    let _ = session.handle_event(click(up.area.x + 1, up.area.y), &geometry);
    assert_eq!(session.current_dir(), &root);
    assert_eq!(
        feed(&mut session, key(KeyCode::Esc)),
        Some(FilePickerEvent::Cancelled)
    );
}

#[test]
fn test_picker_missing_workdir_opens_at_ancestor_with_notice() {
    // The picker opens at the nearest existing ancestor of a gone workdir.
    // private-render: the oracle also asserts the notice contains "missing"; that text ("The
    // entry's working directory is missing — starting here instead.", i18n lib.rs:3355) is drawn by
    // run_modal::render_file (run_modal.rs:201-219), gated on file_picker_contract's private
    // missing_root bool. FilePickerSession exposes no notice, so the reachable observable is the
    // ancestor-open.
    let (_tmp, root) = tree();
    let gone = root.join("gone").join("deeper");
    let session = FilePickerSession::new(contract(&gone, &gone));
    assert_eq!(session.current_dir(), &root);
}

// ---------------------------------------------------------------------------
// The insert flow: token menu row, replace vs append (path.md §5)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "ABSENT gap: a `path` field carries FormInputKind::Path but there is NO suggester behind it (see the PathSuggester bucket). Oracle: the src row's Input has a PathSuggester and its label says 'path' (test_path_tui.py:541-548)."]
fn test_path_fields_render_hint_and_suggester() {}

#[test]
fn test_token_menu_puts_file_row_first_on_path_fields_and_picker_replaces() {
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[("src", "old-prefill.csv")],
        "",
        Some("/work"),
    );
    state.update(Action::OpenRunTokenMenuFor(0));
    // Path field: browse is the first (Enter-default) token row.
    assert_eq!(token_options(&state)[0], RunTokenOption::FileOrFolder);
    state.update(Action::OpenRunFilePicker(0));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 0,
            mode: RunPathInsertMode::Replace,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 0,
        path: "data.csv".to_owned(),
    });
    // The picked path REPLACES the prefilled value.
    assert_eq!(field_value(&state, 0), "data.csv");
}

#[test]
#[cfg(not(windows))]
fn test_picker_appends_quoted_to_the_extra_args_row() {
    // src(0), extra(1). The extra-args row is non-path, so its file row sits last.
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[],
        "--verbose",
        Some("/work"),
    );
    state.update(Action::OpenRunTokenMenuFor(1));
    assert_eq!(
        *token_options(&state).last().unwrap(),
        RunTokenOption::FileOrFolder
    );
    state.update(Action::OpenRunFilePicker(1));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 1,
            mode: RunPathInsertMode::Arguments,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 1,
        path: "a b.txt".to_owned(),
    });
    // Appended as one quoted piece in the row's own dialect (POSIX shlex here).
    assert_eq!(field_value(&state, 1), "--verbose 'a b.txt'");
    assert_eq!(
        split_editable_argv(&field_value(&state, 1), EditableArgvDialect::Posix).unwrap(),
        vec!["--verbose", "a b.txt"]
    );
}

#[test]
#[cfg(not(windows))]
fn test_picker_appends_quoted_to_a_multiple_field() {
    // The nargs='*' path field: append one shlex-quoted piece; it survives POSIX re-splitting.
    let mut state = form_state(
        &[
            param("src", ParameterType::Path, false),
            param("files", ParameterType::Path, true),
        ],
        &[("files", "first.txt")],
        "",
        Some("/work"),
    );
    assert!(state.run_form().unwrap().fields()[1].multiple);
    state.update(Action::OpenRunTokenMenuFor(1));
    assert_eq!(token_options(&state)[0], RunTokenOption::FileOrFolder); // path field
    state.update(Action::OpenRunFilePicker(1));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            field: 1,
            mode: RunPathInsertMode::Shlex,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 1,
        path: "a b.txt".to_owned(),
    });
    assert_eq!(field_value(&state, 1), "first.txt 'a b.txt'");
    assert_eq!(
        split_editable_argv(&field_value(&state, 1), EditableArgvDialect::Posix).unwrap(),
        vec!["first.txt", "a b.txt"]
    );
}

#[test]
#[ignore = "CROSS-CRATE (skit-tui interactive session): cursor-position token insertion is owned by TuiSession's field-cursor layer (run_modal emits RunModalEvent::InsertText applied at the focused LineInput cursor). The reducer surface this file drives replaces the whole value and has no cursor. Oracle: {today} inserted between 'out-' and '.csv' yields 'out-{today}.csv' (test_path_tui.py:635-652)."]
fn test_token_rows_still_insert_at_cursor() {}

// ---------------------------------------------------------------------------
// The browse link: the picker's own door, on the field (issue #7 follow-up)
// ---------------------------------------------------------------------------

fn mixed_state() -> LibraryState {
    // src(path), note(plain str), count(int), loud(bool), then the extra-args row.
    form_state(
        &[
            param("src", ParameterType::Path, false),
            param("note", ParameterType::Str, false),
            param("count", ParameterType::Int, false),
            param("loud", ParameterType::Bool, false),
        ],
        &[],
        "",
        Some("/work"),
    )
}

#[test]
fn test_browse_link_renders_on_text_fields_only() {
    // Browse rides every insertable text field (path, plain str, and the extra-args row) but
    // never a numeric or non-text one, where a picked path is a guaranteed validation error.
    let state = mixed_state();
    let form = state.run_form().unwrap();
    let extra = form.fields().len() - 1; // the extra-args row is last
    for index in [0, 1, extra] {
        assert!(form.fields()[index].browsable(), "field {index} browsable");
        assert!(form.can_browse_field(index), "field {index} can_browse");
        assert!(form.can_insert_field(index), "field {index} can_insert");
    }
    for index in [2, 3] {
        // count (whole number), loud (on/off)
        assert!(
            !form.fields()[index].browsable(),
            "field {index} not browsable"
        );
        assert!(
            !form.can_browse_field(index),
            "field {index} refuses browse"
        );
    }
    assert!(form.can_insert_field(2)); // the ▾ menu is unchanged on the numeric field
}

#[test]
fn test_browse_link_opens_the_picker_directly_and_replaces() {
    // The flagship journey, one door: browse -> pick -> the value is in the field, focused.
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[("src", "old-prefill.csv")],
        "",
        Some("/work"),
    );
    state.update(Action::OpenRunFilePicker(0));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker { field: 0, .. })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: 0,
        path: "data.csv".to_owned(),
    });
    assert_eq!(field_value(&state, 0), "data.csv");
    assert_eq!(state.run_form().unwrap().focused(), 0);
}

#[test]
#[cfg(not(windows))]
fn test_browse_without_a_key_uses_the_focused_field_and_its_dialect() {
    // The keyless (footer-chip) route lands on the focused row and honours its insert mode:
    // the extra-args row appends a quoted piece rather than replacing.
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[],
        "--verbose",
        Some("/work"),
    );
    let extra = state.run_form().unwrap().fields().len() - 1;
    state.update(Action::FocusField(extra));
    state.update(Action::OpenFocusedRunFilePicker);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker {
            mode: RunPathInsertMode::Arguments,
            ..
        })
    ));
    state.update(Action::SetRunPickedPathAndCloseModal {
        field: extra,
        path: "a b.txt".to_owned(),
    });
    assert_eq!(field_value(&state, extra), "--verbose 'a b.txt'");
}

#[test]
fn test_browse_refuses_numeric_secret_and_unknown_rows() {
    // Both gates hold: no browse door on a numeric/non-text row, and an unknown index opens
    // nothing.
    let mut state = mixed_state();
    for index in [2, 3, 99] {
        // count (int), loud (bool), a row that no longer exists
        state.update(Action::OpenRunFilePicker(index));
        assert!(state.modal().is_none(), "no picker for field {index}");
    }
    // The keyless route with focus on a non-text (checkbox) field also refuses.
    state.update(Action::FocusField(3));
    state.update(Action::OpenFocusedRunFilePicker);
    assert!(state.modal().is_none());
}

#[test]
fn test_fieldrow_browsable_needs_a_context() {
    // A field cannot browse without a path completion context: there is no root to open at.
    let declarations = [param("x", ParameterType::Str, false)];
    let without = form_state(&declarations, &[], "", None);
    assert!(!without.run_form().unwrap().can_browse_field(0));
    let with = form_state(&declarations, &[], "", Some("/work"));
    assert!(with.run_form().unwrap().can_browse_field(0));
}

#[test]
fn test_fieldrow_shlexy_and_insert_mode_all_branches() {
    // A single-value field replaces; a `multiple` field appends in POSIX shlex; the
    // extra-args row appends in the argv/native dialect (path.md §5).
    let mut state = form_state(
        &[
            param("single", ParameterType::Path, false),
            param("many", ParameterType::Path, true),
        ],
        &[],
        "",
        Some("/work"),
    );
    let extra = state.run_form().unwrap().fields().len() - 1;
    let expected = [
        (0_usize, RunPathInsertMode::Replace),
        (1, RunPathInsertMode::Shlex),
        (extra, RunPathInsertMode::Arguments),
    ];
    for (index, mode) in expected {
        state.update(Action::OpenRunTokenMenuFor(index));
        state.update(Action::OpenRunFilePicker(index));
        match state.modal() {
            Some(ModalState::RunFilePicker {
                field,
                mode: actual,
                ..
            }) => {
                assert_eq!(*field, index);
                assert_eq!(*actual, mode);
            }
            other => panic!("expected a picker for field {index}, got {other:?}"),
        }
        state.update(Action::SetRunPickedPathAndCloseModal {
            field: index,
            path: "x".to_owned(),
        });
    }
}

#[test]
fn test_insert_picked_shapes() {
    // replace: a scalar field is overwritten.
    assert_eq!(
        insert_picked_path_for_dialect(
            "old.csv",
            "new.csv",
            RunPathInsertMode::Replace,
            ArgumentDialect::Posix
        )
        .unwrap(),
        "new.csv"
    );
    // shlex: a lone piece, no leading space invented.
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "a b.txt",
            RunPathInsertMode::Shlex,
            ArgumentDialect::Posix
        )
        .unwrap(),
        "'a b.txt'"
    );
    // shlex is platform-agnostic (single quotes) even under the Windows dialect.
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "a b.txt",
            RunPathInsertMode::Shlex,
            ArgumentDialect::Windows
        )
        .unwrap(),
        "'a b.txt'"
    );
    // arguments under the Windows dialect: CRT double quotes; re-parsing keeps the name whole.
    let windows = insert_picked_path_for_dialect(
        "--verbose",
        "a b.txt",
        RunPathInsertMode::Arguments,
        ArgumentDialect::Windows,
    )
    .unwrap();
    assert_eq!(windows, "--verbose \"a b.txt\"");
    assert_eq!(
        split_editable_argv(&windows, EditableArgvDialect::Windows).unwrap(),
        vec!["--verbose", "a b.txt"]
    );
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the `glob` crate escapes `]` as `[]]` too, so insert_picked_path yields '\\'data[[]1[]].csv\\'' where Python's glob.escape (which leaves `]` literal) yields '\\'data[[]1].csv\\''. Benign — both suppress globbing and re-glob to the one literal file — but the exact bytes differ (path_insertion.rs:57 Pattern::escape vs glob.escape). Oracle: test_path_tui.py:852-873."]
fn test_insert_picked_escapes_glob_metacharacters() {
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "data[1].csv",
            RunPathInsertMode::Shlex,
            ArgumentDialect::Posix
        )
        .unwrap(),
        "'data[[]1].csv'"
    );
    let argv = insert_picked_path_for_dialect(
        "",
        "data[1].csv",
        RunPathInsertMode::Arguments,
        ArgumentDialect::Posix,
    )
    .unwrap();
    assert_eq!(
        split_editable_argv(&argv, EditableArgvDialect::Posix)
            .unwrap()
            .last()
            .unwrap(),
        "data[[]1].csv"
    );
}

#[test]
#[ignore = "ABSENT gap: no suggester exists to withhold from a secret field. The Rust equivalent gate is RunField::insertable()/browsable() returning false for a secret text control (covered structurally by the browse-link test). Oracle: a secret field's Input.suggester is None, a plain field's is a PathSuggester (test_path_tui.py:876-894)."]
fn test_secret_field_never_gets_a_suggester() {}

#[test]
fn test_token_menu_without_context_has_no_file_row() {
    // Without a path completion context the value menu still opens (base tokens) but offers no
    // file row. (Rust gates the file row on `context.path`; a context with no path is the
    // faithful analog of the oracle's context-less TokenMenuModal.)
    let mut state = form_state(&[param("value", ParameterType::Str, false)], &[], "", None);
    state.update(Action::OpenRunTokenMenuFor(0));
    assert!(!token_options(&state).contains(&RunTokenOption::FileOrFolder));
}

#[test]
#[ignore = "ABSENT gap: looks_pathy has no Rust equivalent (it fed only the absent ghost activation). Oracle: on Windows a `..\\data` / `C:\\Users` / `C:/Users` is path-shaped, a bare word is not (test_path_tui.py:909-915)."]
fn test_looks_pathy_windows_recognition() {}

#[test]
#[ignore = "ABSENT gap: looks_pathy has no Rust equivalent. Oracle: '~'/'~project'/'{cwd}' are path-shaped, '{CWD}' is not (case-sensitive), any slash activates, a bare word does not (test_path_tui.py:918-928)."]
fn test_looks_pathy_token_and_separator_spellings() {}

// ---------------------------------------------------------------------------
// PathSuggester constructor contract, observed through Textual's _get_suggestion
// (ABSENT gap: the whole ghost surface is missing — see the top bucket.)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: case_sensitive=True — 'DA' matches the uppercase file DATA.csv verbatim (test_path_tui.py:955-966)."]
fn test_suggester_is_case_sensitive_query_not_casefolded() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: use_cache=False — a re-query after the file is deleted re-scans and finds it gone (test_path_tui.py:969-982)."]
fn test_suggester_does_not_cache_stale_results() {}

// ---------------------------------------------------------------------------
// PathSuggester internals: brace-escape flag, quote refusal, token-without-sep
// (ABSENT gap.)
// ---------------------------------------------------------------------------

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a normal field has brace_escapes=True, so '{{x}}/da' halves to the real dir {x} and completes (test_path_tui.py:1000-1004)."]
fn test_brace_escapes_on_a_normal_field_halves_doubled_braces() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a placeholder field has brace_escapes=False, so '{{x}}/da' stays literal and completes nothing (test_path_tui.py:1007-1013)."]
fn test_brace_escapes_off_on_a_placeholder_field_keeps_doubled_braces() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a trailing piece bearing either quote refuses to complete; a clean one still completes (test_path_tui.py:1016-1027)."]
fn test_shlexy_trailing_piece_refuses_either_quote() {}

#[test]
#[ignore = "ABSENT gap: no ghost suggester. Oracle: a '~' or '{' that has not reached a separator completes nothing, even with a matching file present (test_path_tui.py:1030-1038)."]
fn test_bare_token_prefix_without_separator_is_silent() {}

// ---------------------------------------------------------------------------
// _list_filtered ranking and hidden-entry rules (picker only)
// The rank is the free function picker::apply_filter, observed through FilePickerSession.
// ---------------------------------------------------------------------------

#[test]
fn test_list_filtered_reveals_hidden_only_behind_a_dot_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join(".env"), "x").unwrap();
    fs::write(root.join("readme"), "x").unwrap();
    let mut plain = picker(&root);
    typed(&mut plain, "en"); // substring "en" not in any visible name
    assert_eq!(listing(&plain), Vec::<(String, bool)>::new());
    let mut dotted = picker(&root);
    typed(&mut dotted, ".en"); // the dot filter reveals the hidden entry
    assert_eq!(listing(&dotted), vec![(".env".to_owned(), false)]);
}

#[test]
fn test_list_filtered_dir_sorts_before_an_earlier_file_within_a_rank() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir_all(root.join("xz")).unwrap(); // a directory
    fs::write(root.join("xa"), "x").unwrap(); // an alphabetically-earlier file
    let mut session = picker(&root);
    typed(&mut session, "x"); // same prefix rank; the directory wins
    assert_eq!(
        listing(&session),
        vec![("xz".to_owned(), true), ("xa".to_owned(), false)]
    );
}

#[test]
fn test_list_filtered_tiebreak_is_case_insensitive() {
    // Both files contain the needle "txt" as a substring but as a prefix of NEITHER, so both of
    // apply_filter's leading rank keys tie (not-starts-with and not-is-dir are equal) and only its
    // final tiebreak, name.to_lowercase() (picker.rs:902), can order them. ASCII '_' (95) sits
    // between 'Z' (90) and 'a' (97): a to_lowercase() tiebreak keeps '_z.txt' before 'a.txt'; a
    // to_uppercase() tiebreak ('_'=95 sorts AFTER 'A'=65) would flip them.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("root");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("_z.txt"), "x").unwrap();
    fs::write(root.join("a.txt"), "x").unwrap();
    let mut session = picker(&root);
    typed(&mut session, "txt"); // a non-empty needle so apply_filter's comparator actually runs
    assert_eq!(
        listing(&session),
        vec![("_z.txt".to_owned(), false), ("a.txt".to_owned(), false)]
    );
}

// ---------------------------------------------------------------------------
// FilePickerModal display + navigation refresh
// ---------------------------------------------------------------------------

#[test]
#[ignore = "FAILING CONTRACT (divergence): the pinned use-this-directory affordance is the mouse-only CurrentDirectory row, rendered as '▶ <current dir path>', not a localized '(use this directory)' OptionList row with id '__use_dir__'. Oracle: option 0 id is '__use_dir__' and its prompt ends with '(use this directory)' (test_path_tui.py:1079-1092)."]
fn test_picker_pinned_row_shows_its_label() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    let (text, _geometry) = render(&mut session, 100, 30);
    assert!(text.contains("(use this directory)"));
}

#[test]
fn test_picker_empty_directory_highlights_the_pinned_row() {
    // An empty directory offers only the use-this-directory affordance: no real entries, and
    // the mouse CurrentDirectory door is rendered.
    let tmp = tempfile::tempdir().unwrap();
    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let mut session = picker(&empty);
    assert_eq!(listing(&session), Vec::<(String, bool)>::new());
    let (_text, geometry) = render(&mut session, 100, 30);
    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| hit.target == FilePickerHit::CurrentDirectory)
    );
}

#[test]
fn test_picker_ascend_repopulates_the_parent_listing() {
    let (_tmp, root) = tree();
    let mut session = FilePickerSession::new(contract(&root.join("sub"), &root));
    let _ = feed(&mut session, key(KeyCode::Backspace)); // empty filter -> ascend to root
    let names = listing(&session);
    // The parent's real entries are shown (not an empty filtered-to-nothing list).
    assert!(names.contains(&("sub".to_owned(), true)));
    assert!(names.contains(&("data.csv".to_owned(), false)));
}
