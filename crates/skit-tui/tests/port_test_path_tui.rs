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
//!   (skit-tui): the pinned "(use this directory)" OptionList row is the keyboard-and-mouse
//!   `FilePickerHit::CurrentDirectory`; the parent step is a real `..` `EntryType::ParentDir` row;
//!   `_list_filtered`'s rank is the free function `picker::apply_filter`.
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

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use ratatui_core::{
    backend::TestBackend, buffer::Buffer, layout::Rect, style::Color, terminal::Terminal,
};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::path_completion::{
    DirectoryEntry, DirectoryReadError, DirectoryReader, PathCompletionContext, PathCompletionKind,
    PathCompletionProvider, PathCompletionRequest, PathCompletionService, PathInputDialect,
};
use skit_application::path_insertion::{
    ArgumentDialect, RunPathInsertMode, insert_picked_path_for_dialect,
};
use skit_application::runner_management::{EditableArgvDialect, split_editable_argv};
use skit_application::tokens::TokenContext;
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_i18n::Locale;
use skit_store::SystemDirectoryReader;
use skit_tui::{
    EventHandling, FilePickerEvent, FilePickerGeometry, FilePickerHit, FilePickerSession,
    TuiSession, ViewGeometry, render_file_picker, render_with_session,
};
use skit_ui::{
    Action, LibraryState, ModalState, PathOutputPolicy, PathPickerState, PathSelectionMode,
    PickerPurpose, RunFormContext, RunFormView, RunPathContext, RunTokenOption, Screen,
};
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr as _;

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
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
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
    let (buffer, geometry) = render_localized(session, width, height, Locale::En);
    let text = buffer.content().iter().map(|cell| cell.symbol()).collect();
    (text, geometry)
}

fn render_localized(
    session: &mut FilePickerSession,
    width: u16,
    height: u16,
    locale: Locale,
) -> (Buffer, FilePickerGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = FilePickerGeometry::default();
    terminal
        .draw(|frame| geometry = render_file_picker(frame, frame.area(), session, locale))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (buffer, geometry)
}

fn region_text(buffer: &Buffer, area: Rect) -> String {
    let mut text = String::new();
    for row in area.y..area.bottom().min(buffer.area.height) {
        let mut column = area.x;
        while column < area.right().min(buffer.area.width) {
            let symbol = buffer[(column, row)].symbol();
            text.push_str(symbol);
            column = column.saturating_add(u16::try_from(symbol.width()).unwrap_or(1).max(1));
        }
    }
    text
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

fn render_root(session: &mut TuiSession, state: &LibraryState) -> ViewGeometry {
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_with_session(frame, state, Locale::En, session);
        })
        .unwrap();
    geometry
}

fn drive_root(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    event: Event,
) -> EventHandling {
    let handling = session.handle_event(event, state, geometry);
    if let EventHandling::Action(action) = &handling {
        state.update(action.clone());
    }
    handling
}

fn token_options(state: &LibraryState) -> Vec<RunTokenOption> {
    match state.modal() {
        Some(ModalState::RunTokenMenu { options, .. }) => options.clone(),
        other => panic!("expected an open token menu, got {other:?}"),
    }
}

fn completion_request(
    workdir: &Path,
    invoke_cwd: &Path,
    value: &str,
    kind: PathCompletionKind,
) -> PathCompletionRequest {
    PathCompletionRequest {
        value: value.to_owned(),
        kind,
        shlexy: false,
        placeholder_braces: false,
        dialect: if cfg!(windows) {
            PathInputDialect::Windows
        } else {
            PathInputDialect::Posix
        },
        context: PathCompletionContext {
            workdir: workdir.to_path_buf(),
            tokens: TokenContext {
                cwd: invoke_cwd.display().to_string(),
                home: None,
                env: BTreeMap::new(),
                today: "2026-08-10".to_owned(),
                now: "10-11-12".to_owned(),
            },
        },
    }
}

fn complete(request: PathCompletionRequest) -> Option<String> {
    PathCompletionService::new(SystemDirectoryReader).complete(&request)
}

fn completion_session() -> TuiSession {
    TuiSession::with_path_completion(Arc::new(PathCompletionService::new(SystemDirectoryReader)))
}

static COMPLETION_WORKER_TEST: Mutex<()> = Mutex::new(());

fn completion_worker_test() -> MutexGuard<'static, ()> {
    COMPLETION_WORKER_TEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How long a worker answer may take before the harness declares a failure.
///
/// A full workspace run can oversubscribe the host before a test's two fresh workers get a time
/// slice, and coverage instrumentation slows every slice further — a 10-second budget expired
/// once on the instrumented CI runner. 30 seconds follows the harness-budget precedent
/// (`08046bd`, `24fe85c`): the deadline exists only to stop a hung worker from stalling the
/// suite, and the polls below read the real checkpoint, so a healthy run never waits it out.
const WORKER_ANSWER_BUDGET: Duration = Duration::from_secs(30);

fn wait_for_completion(
    session: &mut TuiSession,
    state: &LibraryState,
    locale: Locale,
    expected: &str,
) {
    // A refresh can request a render that retries a full worker queue. It does not prove that a
    // result is visible. Follow the production refresh-render loop and wait for the real ghost.
    let deadline = Instant::now() + WORKER_ANSWER_BUDGET;
    while Instant::now() < deadline {
        let _ = session.refresh_background();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_with_session(frame, state, locale, session);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        if region_text(buffer, buffer.area).contains(expected) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("path completion worker did not answer");
}

fn type_run_value(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    value: &str,
) {
    for character in value.chars() {
        let handling = drive_root(session, state, geometry, key(KeyCode::Char(character)));
        assert!(matches!(handling, EventHandling::Action(_)));
    }
}

#[derive(Debug)]
struct FixedDirectoryReader {
    entries: Vec<DirectoryEntry>,
    failure: Option<DirectoryReadError>,
}

impl DirectoryReader for FixedDirectoryReader {
    fn read_directory(
        &self,
        _path: &Path,
        scan_cap: usize,
        filter: &skit_application::path_completion::DirectoryReadFilter,
    ) -> Result<Vec<DirectoryEntry>, DirectoryReadError> {
        if let Some(error) = &self.failure {
            return Err(*error);
        }
        Ok(self
            .entries
            .iter()
            .take(scan_cap)
            .filter(|entry| filter.accepts(&entry.name))
            .cloned()
            .collect())
    }
}

#[derive(Debug)]
struct CountingProvider {
    calls: Arc<AtomicUsize>,
}

impl PathCompletionProvider for CountingProvider {
    fn complete(&self, request: &PathCompletionRequest) -> Option<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Some(format!("{}-suggested", request.value))
    }
}

#[derive(Debug)]
struct RacingProvider;

impl PathCompletionProvider for RacingProvider {
    fn complete(&self, request: &PathCompletionRequest) -> Option<String> {
        match request.value.as_str() {
            "a" => {
                std::thread::sleep(Duration::from_millis(100));
                Some("alpha".to_owned())
            }
            "b" => Some("beta".to_owned()),
            _ => None,
        }
    }
}

#[derive(Debug)]
struct SlowProvider {
    released: Arc<AtomicBool>,
}

impl PathCompletionProvider for SlowProvider {
    fn complete(&self, _request: &PathCompletionRequest) -> Option<String> {
        while !self.released.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        None
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
fn test_path_field_completes_bare_prefix_at_workdir() {
    let (_tmp, root) = tree();
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "da",
            PathCompletionKind::Path,
        )),
        Some("data.csv".to_owned())
    );
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "su",
            PathCompletionKind::Path,
        )),
        Some("sub/".to_owned())
    );
}

#[test]
fn test_str_field_needs_pathy_text() {
    let (_tmp, root) = tree();
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "da",
            PathCompletionKind::Text,
        )),
        None
    );
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "./da",
            PathCompletionKind::Text,
        )),
        Some("./data.csv".to_owned())
    );
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "sub/in",
            PathCompletionKind::Text,
        )),
        Some("sub/inner.txt".to_owned())
    );
}

#[test]
fn test_secretless_activation_never_guesses_beyond_prefix() {
    let (_tmp, root) = tree();
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "zzz",
            PathCompletionKind::Path,
        )),
        None
    );
}

#[test]
fn test_hidden_entries_only_behind_a_dot_prefix() {
    let (_tmp, root) = tree();
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            ".h",
            PathCompletionKind::Path,
        )),
        Some(".hidden".to_owned())
    );
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "d",
            PathCompletionKind::Path,
        )),
        Some("data.csv".to_owned())
    );
}

#[test]
fn test_cwd_token_completes_at_invoke_cwd_not_workdir() {
    let (_tmp, root) = tree();
    let invoke = tempfile::tempdir().unwrap();
    fs::write(invoke.path().join("notes.md"), "x").unwrap();
    let request = completion_request(&root, invoke.path(), "{cwd}/no", PathCompletionKind::Path);
    assert_eq!(complete(request), Some("{cwd}/notes.md".to_owned()));
}

#[test]
fn test_unset_env_token_is_silence_not_a_traceback() {
    let (_tmp, root) = tree();
    assert_eq!(
        complete(completion_request(
            &root,
            &root,
            "{env:SKIT_NO_SUCH_VAR}/d",
            PathCompletionKind::Path,
        )),
        None
    );
}

#[test]
fn test_relative_env_token_falls_back_to_the_workdir_rule() {
    let (_tmp, root) = tree();
    let mut request = completion_request(
        &root,
        &root,
        "{env:SKIT_REL_DIR}/in",
        PathCompletionKind::Path,
    );
    request
        .context
        .tokens
        .env
        .insert("SKIT_REL_DIR".to_owned(), "sub".to_owned());
    assert_eq!(
        complete(request),
        Some("{env:SKIT_REL_DIR}/inner.txt".to_owned())
    );
}

#[test]
fn test_home_prefix_completes_inside_home() {
    let (_tmp, root) = tree();
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("notes.md"), "x").unwrap();
    let mut request = completion_request(&root, &root, "~/no", PathCompletionKind::Text);
    request.context.tokens.home = Some(home.path().display().to_string());
    assert_eq!(complete(request), Some("~/notes.md".to_owned()));
    let mut bare = completion_request(&root, &root, "~", PathCompletionKind::Text);
    bare.context.tokens.home = Some(home.path().display().to_string());
    assert_eq!(complete(bare), None);
}

#[test]
fn test_missing_workdir_silences_bare_completion() {
    let tmp = tempfile::tempdir().unwrap();
    let gone = tmp.path().join("vanished");
    assert_eq!(
        complete(completion_request(
            &gone,
            tmp.path(),
            "da",
            PathCompletionKind::Path,
        )),
        None
    );
}

#[test]
fn test_missing_workdir_silences_relative_token_lookup() {
    let tmp = tempfile::tempdir().unwrap();
    let gone = tmp.path().join("vanished");
    let mut request = completion_request(
        &gone,
        tmp.path(),
        "{env:SKIT_REL_DIR}/in",
        PathCompletionKind::Path,
    );
    request
        .context
        .tokens
        .env
        .insert("SKIT_REL_DIR".to_owned(), "sub".to_owned());
    assert_eq!(complete(request), None);
}

#[test]
fn test_shlexy_field_completes_only_the_trailing_piece() {
    let (_tmp, root) = tree();
    let mut request = completion_request(&root, &root, "first.txt dr", PathCompletionKind::Path);
    request.shlexy = true;
    assert_eq!(
        PathCompletionService::new(SystemDirectoryReader).complete(&request),
        Some("first.txt draft.txt".to_owned())
    );
    request.value = "'quote in progress".to_owned();
    assert_eq!(complete(request.clone()), None);
    request.value = "done.txt ".to_owned();
    assert_eq!(complete(request), None);
}

#[test]
fn test_scan_cap_stops_the_scan_exactly() {
    let reader = FixedDirectoryReader {
        entries: ["dax3", "dax2", "dax4", "daa-first"]
            .into_iter()
            .map(DirectoryEntry::file)
            .collect(),
        failure: None,
    };
    let root = Path::new("/root");
    let service = PathCompletionService::with_scan_cap(reader, 3);
    assert_eq!(
        service.complete(&completion_request(
            root,
            root,
            "da",
            PathCompletionKind::Path,
        )),
        Some("dax2".to_owned())
    );
}

#[test]
fn test_scan_degrades_on_oserror() {
    let reader = FixedDirectoryReader {
        entries: Vec::new(),
        failure: Some(DirectoryReadError::Unavailable),
    };
    let root = Path::new("/root");
    assert_eq!(
        PathCompletionService::new(reader).complete(&completion_request(
            root,
            root,
            "da",
            PathCompletionKind::Path,
        )),
        None
    );
}

#[test]
fn test_unstatable_entry_is_treated_as_a_file() {
    let reader = FixedDirectoryReader {
        entries: vec![DirectoryEntry::file("dax")],
        failure: None,
    };
    let root = Path::new("/root");
    assert_eq!(
        PathCompletionService::new(reader).complete(&completion_request(
            root,
            root,
            "da",
            PathCompletionKind::Path,
        )),
        Some("dax".to_owned())
    );
}

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
fn test_vanished_origin_reference_entry_degrades() {
    let tmp = tempfile::tempdir().unwrap();
    let parent = tmp.path().join("proj");
    let origin = parent.join("deep");
    fs::create_dir_all(&origin).unwrap();
    fs::remove_dir(&origin).unwrap();

    assert_eq!(
        complete(completion_request(
            &origin,
            tmp.path(),
            "da",
            PathCompletionKind::Path,
        )),
        None
    );
    let session = FilePickerSession::new(contract(&origin, &origin));
    assert_eq!(session.current_dir(), &parent);
}

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
fn test_picker_use_this_directory_row_by_real_keys() {
    let (_tmp, root) = tree();
    let mut session = picker(&root);
    assert_eq!(
        feed(&mut session, key(KeyCode::Up)),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Accepted(vec![PathBuf::new()]))
    );

    // The composition root converts the frontend-neutral empty relative path into the oracle's
    // visible PickedPath(".") value. Drive the real Run form, token menu, file picker, and reducer
    // instead of publishing that action from the test.
    let root_text = root.to_string_lossy().into_owned();
    let mut state = form_state(
        &[param("path", ParameterType::Path, false)],
        &[("path", "old.txt")],
        "",
        Some(&root_text),
    );
    let mut root_session = TuiSession::default();
    let geometry = render_root(&mut root_session, &state);
    assert!(matches!(
        drive_root(
            &mut root_session,
            &mut state,
            &geometry,
            Event::Key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL)),
        ),
        EventHandling::Action(_)
    ));
    let geometry = render_root(&mut root_session, &state);
    assert!(matches!(
        drive_root(
            &mut root_session,
            &mut state,
            &geometry,
            key(KeyCode::Enter),
        ),
        EventHandling::Action(_)
    ));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunFilePicker { .. })
    ));
    let geometry = render_root(&mut root_session, &state);
    assert_eq!(
        drive_root(&mut root_session, &mut state, &geometry, key(KeyCode::Up),),
        EventHandling::Consumed
    );
    assert_eq!(field_value(&state, 0), "old.txt");
    let geometry = render_root(&mut root_session, &state);
    assert_eq!(
        drive_root(
            &mut root_session,
            &mut state,
            &geometry,
            key(KeyCode::Enter),
        ),
        EventHandling::Action(Action::SetRunPickedPathAndCloseModal {
            field: 0,
            path: ".".to_owned(),
        })
    );
    assert_eq!(field_value(&state, 0), ".");
    assert!(state.modal().is_none());
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
    assert_eq!(
        session.handle_event(click(hit.area.x + 2, hit.area.y), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                hit.area.x + 2,
                hit.area.y,
            ),
            &geometry,
        ),
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
    let (buffer, geometry) = render_localized(&mut session, 100, 30, Locale::En);
    assert!(
        geometry
            .hits
            .iter()
            .all(|hit| hit.target != FilePickerHit::CurrentDirectory),
        "a nonempty filter must remove the pinned row and its click target"
    );
    assert!(!region_text(&buffer, geometry.rows).contains("(use this directory)"));
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Accepted(vec![PathBuf::from("data.csv")]))
    );
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
    assert_eq!(
        session.handle_event(click(up.area.x + 1, up.area.y), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                up.area.x + 1,
                up.area.y,
            ),
            &geometry,
        ),
        Some(FilePickerEvent::Changed)
    );
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
fn test_path_fields_render_hint_and_suggester() {
    let _workers = completion_worker_test();
    let (_tmp, root) = tree();
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[],
        "",
        root.to_str(),
    );
    let mut session = completion_session();
    let geometry = render_root(&mut session, &state);
    type_run_value(&mut session, &mut state, &geometry, "da");
    wait_for_completion(&mut session, &state, Locale::En, "data.csv");

    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_with_session(frame, &state, Locale::En, &mut session);
        })
        .unwrap();
    let rendered = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();
    assert!(rendered.contains("path"), "{rendered}");
    assert!(rendered.contains("data.csv"), "{rendered}");
}

#[test]
fn right_accepts_only_the_current_ghost_at_the_end_of_the_input() {
    let _workers = completion_worker_test();
    let (_tmp, root) = tree();
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[],
        "",
        root.to_str(),
    );
    let mut session = completion_session();
    let geometry = render_root(&mut session, &state);
    type_run_value(&mut session, &mut state, &geometry, "da");
    wait_for_completion(&mut session, &state, Locale::En, "data.csv");
    assert_eq!(field_value(&state, 0), "da", "a ghost is not a value");

    assert_eq!(
        drive_root(&mut session, &mut state, &geometry, key(KeyCode::Home)),
        EventHandling::Consumed
    );
    assert_eq!(
        drive_root(&mut session, &mut state, &geometry, key(KeyCode::Right)),
        EventHandling::Consumed,
        "Right inside the value keeps its cursor meaning"
    );
    assert_eq!(field_value(&state, 0), "da");
    let _ = drive_root(&mut session, &mut state, &geometry, key(KeyCode::End));
    assert!(matches!(
        drive_root(&mut session, &mut state, &geometry, key(KeyCode::Right)),
        EventHandling::Action(Action::SetFieldValue { .. })
    ));
    assert_eq!(field_value(&state, 0), "data.csv");
}

#[test]
fn stale_out_of_order_completion_never_replaces_the_latest_ghost() {
    let _workers = completion_worker_test();
    let root = tempfile::tempdir().unwrap();
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[],
        "",
        root.path().to_str(),
    );
    let mut session = TuiSession::with_path_completion(Arc::new(RacingProvider));
    let geometry = render_root(&mut session, &state);
    type_run_value(&mut session, &mut state, &geometry, "a");
    let _ = drive_root(&mut session, &mut state, &geometry, key(KeyCode::Backspace));
    type_run_value(&mut session, &mut state, &geometry, "b");

    // Same worker-answer wait as `wait_for_completion`, with the checkpoint read through a
    // render because this test asserts what the user SEES; the shared budget keeps the two
    // waits on one convention.
    let deadline = Instant::now() + WORKER_ANSWER_BUDGET;
    loop {
        let _ = session.refresh_background();
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered = region_text(buffer, buffer.area);
        if rendered.contains("beta") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "latest completion did not arrive"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    std::thread::sleep(Duration::from_millis(125));
    let _ = session.refresh_background();
    assert!(matches!(
        drive_root(&mut session, &mut state, &geometry, key(KeyCode::Right)),
        EventHandling::Action(Action::SetFieldValue { .. })
    ));
    assert_eq!(field_value(&state, 0), "beta");
}

#[test]
fn secret_fields_never_dispatch_a_filesystem_completion_request() {
    let _workers = completion_worker_test();
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = CountingProvider {
        calls: Arc::clone(&calls),
    };
    let mut secret = param("token", ParameterType::Str, false);
    secret.secret = true;
    let mut state = form_state(&[secret], &[], "", Some("/work"));
    let mut session = TuiSession::with_path_completion(Arc::new(provider));
    let geometry = render_root(&mut session, &state);
    type_run_value(&mut session, &mut state, &geometry, "./token");
    let _ = session.refresh_background();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_slow_completion_worker_does_not_block_escape() {
    let _workers = completion_worker_test();
    let released = Arc::new(AtomicBool::new(false));
    let provider = SlowProvider {
        released: Arc::clone(&released),
    };
    let mut state = form_state(
        &[param("src", ParameterType::Path, false)],
        &[],
        "",
        Some("/work"),
    );
    let mut session = TuiSession::with_path_completion(Arc::new(provider));
    let geometry = render_root(&mut session, &state);
    type_run_value(&mut session, &mut state, &geometry, "d");
    let started = Instant::now();
    assert_eq!(
        session.handle_event(key(KeyCode::Esc), &state, &geometry),
        EventHandling::Action(Action::Back)
    );
    assert!(started.elapsed() < Duration::from_millis(50));
    released.store(true, Ordering::SeqCst);
}

#[test]
fn path_hint_and_existing_browse_door_stay_complete_in_three_locales() {
    let _workers = completion_worker_test();
    let (_tmp, root) = tree();
    for (locale, path_label, browse_label) in [
        (Locale::En, "path", "browse"),
        (Locale::ZhCn, "路径", "浏览"),
        (Locale::ZhTw, "路徑", "瀏覽"),
    ] {
        let mut state = form_state(
            &[param("src", ParameterType::Path, false)],
            &[],
            "",
            root.to_str(),
        );
        let mut session = completion_session();
        let geometry = render_root(&mut session, &state);
        type_run_value(&mut session, &mut state, &geometry, "da");
        wait_for_completion(&mut session, &state, locale, "data.csv");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_with_session(frame, &state, locale, &mut session);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let rendered = region_text(buffer, buffer.area);
        assert!(rendered.contains(path_label), "{locale:?}: {rendered}");
        assert!(rendered.contains(browse_label), "{locale:?}: {rendered}");
        assert!(rendered.contains("data.csv"), "{locale:?}: {rendered}");
    }
}

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
fn test_secret_field_never_gets_a_suggester() {
    let (_tmp, root) = tree();
    let mut secret = param("token", ParameterType::Str, false);
    secret.secret = true;
    let plain = param("out", ParameterType::Str, false);
    let form = RunFormView::from_declarations(
        "demo",
        "Demo",
        &[secret, plain],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "python".to_owned(),
        path: Some(RunPathContext {
            workdir: root.display().to_string(),
            invoke_cwd: root.display().to_string(),
        }),
        tokens: TokenContext {
            cwd: root.display().to_string(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-10".to_owned(),
            now: "10-11-12".to_owned(),
        },
    });
    assert!(
        form.path_completion_request(0, "./da", PathInputDialect::Posix)
            .is_none()
    );
    assert!(
        form.path_completion_request(1, "./da", PathInputDialect::Posix)
            .is_some()
    );
}

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
fn test_looks_pathy_windows_recognition() {
    use skit_application::path_completion::looks_pathy;

    assert!(looks_pathy(r"..\data", PathInputDialect::Windows));
    assert!(looks_pathy(r"C:\Users", PathInputDialect::Windows));
    assert!(looks_pathy("C:/Users", PathInputDialect::Windows));
    assert!(!looks_pathy("data", PathInputDialect::Windows));
    assert!(!looks_pathy(r"..\data", PathInputDialect::Posix));
}

#[test]
fn test_looks_pathy_token_and_separator_spellings() {
    use skit_application::path_completion::looks_pathy;

    for value in ["~", "~project", "{cwd}", "a/b", "./x"] {
        assert!(looks_pathy(value, PathInputDialect::Posix), "{value}");
    }
    for value in ["{CWD}", "plain"] {
        assert!(!looks_pathy(value, PathInputDialect::Posix), "{value}");
    }
}

// ---------------------------------------------------------------------------
// PathSuggester constructor contract, observed through Textual's _get_suggestion
// (ABSENT gap: the whole ghost surface is missing — see the top bucket.)
// ---------------------------------------------------------------------------

#[test]
fn test_suggester_is_case_sensitive_query_not_casefolded() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("DATA.csv"), "x").unwrap();
    assert_eq!(
        complete(completion_request(
            root.path(),
            root.path(),
            "DA",
            PathCompletionKind::Path,
        )),
        Some("DATA.csv".to_owned())
    );
    assert_eq!(
        complete(completion_request(
            root.path(),
            root.path(),
            "da",
            PathCompletionKind::Path,
        )),
        None
    );
}

#[test]
fn test_suggester_does_not_cache_stale_results() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("data.csv");
    fs::write(&target, "x").unwrap();
    let request = completion_request(root.path(), root.path(), "da", PathCompletionKind::Path);
    let service = PathCompletionService::new(SystemDirectoryReader);
    assert_eq!(service.complete(&request), Some("data.csv".to_owned()));
    fs::remove_file(target).unwrap();
    assert_eq!(service.complete(&request), None);
}

// ---------------------------------------------------------------------------
// PathSuggester internals: brace-escape flag, quote refusal, token-without-sep
// (ABSENT gap.)
// ---------------------------------------------------------------------------

#[test]
fn test_brace_escapes_on_a_normal_field_halves_doubled_braces() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("{x}")).unwrap();
    fs::write(root.path().join("{x}/data.csv"), "x").unwrap();
    assert_eq!(
        complete(completion_request(
            root.path(),
            root.path(),
            "{{x}}/da",
            PathCompletionKind::Path,
        )),
        Some("{{x}}/data.csv".to_owned())
    );
}

#[test]
fn test_brace_escapes_off_on_a_placeholder_field_keeps_doubled_braces() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("{x}")).unwrap();
    fs::write(root.path().join("{x}/data.csv"), "x").unwrap();
    let mut request = completion_request(
        root.path(),
        root.path(),
        "{{x}}/da",
        PathCompletionKind::Path,
    );
    request.placeholder_braces = true;
    assert_eq!(complete(request), None);
}

#[test]
fn test_shlexy_trailing_piece_refuses_either_quote() {
    let (_tmp, root) = tree();
    for value in ["done.txt 'q", "done.txt \"q"] {
        let mut request = completion_request(&root, &root, value, PathCompletionKind::Path);
        request.shlexy = true;
        assert_eq!(complete(request), None);
    }
    let mut request = completion_request(&root, &root, "done.txt dr", PathCompletionKind::Path);
    request.shlexy = true;
    assert_eq!(complete(request), Some("done.txt draft.txt".to_owned()));
}

#[test]
fn test_bare_token_prefix_without_separator_is_silent() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("~data.txt"), "x").unwrap();
    fs::write(root.path().join("{data.txt"), "x").unwrap();
    for value in ["~da", "{da"] {
        assert_eq!(
            complete(completion_request(
                root.path(),
                root.path(),
                value,
                PathCompletionKind::Path,
            )),
            None
        );
    }
}

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
fn test_picker_pinned_row_shows_its_label() {
    let (_tmp, root) = tree();
    for (locale, expected) in [
        (Locale::En, "(use this directory)"),
        (Locale::ZhCn, "(使用此目录)"),
        (Locale::ZhTw, "(使用此目錄)"),
    ] {
        let mut session = picker(&root);
        let (buffer, geometry) = render_localized(&mut session, 100, 30, locale);
        let pinned = geometry
            .hits
            .iter()
            .filter(|hit| hit.target == FilePickerHit::CurrentDirectory)
            .collect::<Vec<_>>();
        assert_eq!(
            pinned.len(),
            1,
            "{} must expose one typed row",
            locale.tag()
        );
        let row = region_text(&buffer, pinned[0].area);
        assert!(
            row.trim_end().ends_with(expected),
            "{} must render the exact localized label tail: {row:?}",
            locale.tag(),
        );
    }

    let mut session = picker(&root);
    let (_buffer, geometry) = render_localized(&mut session, 100, 30, Locale::En);
    let area = geometry
        .hits
        .iter()
        .find(|hit| hit.target == FilePickerHit::CurrentDirectory)
        .expect("a typed current-directory row")
        .area;
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            session.handle_event(mouse(kind, area.x, area.y), &geometry),
            None,
            "hover and release must not accept the directory"
        );
    }
    assert_eq!(
        session.handle_event(click(area.x, area.y), &geometry),
        Some(FilePickerEvent::Changed)
    );
    assert_eq!(
        session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), area.x, area.y),
            &geometry,
        ),
        Some(FilePickerEvent::Accepted(vec![PathBuf::new()]))
    );
}

#[test]
fn test_picker_empty_directory_highlights_the_pinned_row() {
    // An empty directory offers only the use-this-directory affordance. It is both highlighted
    // and keyboard-selectable.
    let tmp = tempfile::tempdir().unwrap();
    let empty = tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let mut session = picker(&empty);
    assert_eq!(listing(&session), Vec::<(String, bool)>::new());
    let (buffer, geometry) = render_localized(&mut session, 100, 30, Locale::En);
    let pinned = geometry
        .hits
        .iter()
        .filter(|hit| hit.target == FilePickerHit::CurrentDirectory)
        .collect::<Vec<_>>();
    assert_eq!(pinned.len(), 1, "only one pinned row may be clickable");
    assert!(
        (pinned[0].area.x..pinned[0].area.right()).any(|column| {
            let cell = &buffer[(column, pinned[0].area.y)];
            cell.fg == Color::Rgb(0xEE, 0xEE, 0xEE) && cell.bg == Color::Rgb(0x5A, 0x2D, 0x1E)
        }),
        "the sole pinned row must use the selection highlight"
    );
    assert_eq!(
        feed(&mut session, key(KeyCode::Enter)),
        Some(FilePickerEvent::Accepted(vec![PathBuf::new()]))
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
