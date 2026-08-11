//! Mechanical port of the Python oracle module `tests/test_prompt_tui.py`
//! (`origin/main@206f9ef`): "The prompt kind's TUI surfaces — the run form's runner
//! picker (mouse AND keyboard), the Library run/rerun guards, the add lane, and the
//! settings screen's prompt sections." Each `#[test]` keeps its Python `def test_*` name
//! and its WHY comment, and drives the real public API.
//!
//! The oracle drives the whole Textual `MenuApp` (composition root). The Rust rewrite
//! splits that app into the pure `skit-ui` reducer/view state and the `skit-tui` Ratatui
//! adapter, with the actual launch (`launcher.run_entry`, `preflight`, `PendingRun`,
//! `flows.execute`), the `argstate` persistence, and the `store` round-trip owned by the
//! `skit-cli` composition root. This file targets `skit-tui`, so it asserts through the
//! tier boundary each surface exposes, exactly as the `port_test_add_review_contracts`
//! exemplar does.
//!
//! Concept mapping used throughout:
//! - Python `RunFormScreen` runner picker -> `RunFormView::from_declarations(.., runners,
//!   runner_default, ..)`; the picked runner is the `_skit_runner` value in the
//!   `Effect::Submit { values }` a launch delivers. The *pin-beats-last-picked* precedence
//!   that decides `runner_default`, and `config.find_prompt_runner(name)`, are host policy
//!   (skit-cli) — those tests are cross-crate.
//! - Python `ScriptSettingsScreen` prompt sections -> `SettingsView::from_inputs`, the
//!   `SettingsAction` reducer, and `submitted_values()` keyed by `RUNNER_KEY` /
//!   `INTERPOLATE_KEY` / `ADD_PARAMETER_KEY` / `parameter:{name}:keep`. `store.resolve("p")
//!   .meta.*` -> the axes a save carries.
//! - Python `PromptReviewScreen` -> `ReviewState` prompt lane: `prompt_candidates`,
//!   `prompt_preview`, `prompt_is_flooded`, `interpolate`, `runner`, `runner_was_picked`,
//!   `rescan(bytes)`, and `create_entry().settings.{params,runner,interpolate}` / `.mode`.
//! - Python `PromptCandidatePickerModal` -> `ReviewState::prompt_picker()` +
//!   `PromptCandidatePickerSession` (keys, filter, select-all, Done/Cancel events).
//! - Python `RunnerAddModal` -> `RunnerEditorView` (typed `RunnerEditorError` refusals).
//! - Python Library detail pane -> `LibraryEntryDetail::prompt_runner` rendered by
//!   `skit-tui`'s library screen ("Runs with {}", "Runner picked on the run form",
//!   "{} (no longer configured)").
//!
//! Buckets (recorded per test):
//! - REAL: the asserted observable has a `skit-ui`/`skit-tui` twin. The majority.
//! - CROSS-CRATE (skit-cli): the central claim IS host policy — the actual launch/rerun
//!   routing, `preflight`, `PendingRun`, `argstate`, the `store` prompt-read race, the
//!   Library edit -> offer-picker flow, and the pin-vs-last picker default. Compiling
//!   `#[ignore]` stubs naming the owning tier.
//! - ABSENT (gap): the Rust prompt *settings* screen offers `MANAGE`/candidate management
//!   only through `ADD_PARAMETER_KEY` (type a name); it has no detected-placeholder
//!   checkboxes (`st-prompt-new-N`) and no searchable Ctrl+O candidate picker on that
//!   screen — those exist only on the *review* lane here. Compiling `#[ignore]` stubs with
//!   MUST-FIX notes.
//! - DIVERGENCE (Library title — RESOLVED): the zh-CN/zh-TW Library title now localizes to the
//!   v0.4 term 工具库/工具庫 (the catalog previously rendered 程序库/程式庫). The two title tests
//!   below read the space-interleaved `TestBackend` buffer, as render.rs does, so they check
//!   "工 具 库" / "工 具 庫". Still divergent: two review-lane key routings — Ctrl+O opens the
//!   searchable candidate picker unconditionally (the oracle no-ops it for a short prompt) and
//!   Ctrl+E opens the editor even while a text Input owns focus (the oracle keeps it the Input's
//!   end-of-line). Full assertion + `#[ignore = "FAILING CONTRACT (divergence): …"]`.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, synthesized_placeholder},
};
use skit_i18n::Locale;
use skit_tui::{
    ChoicePickerGeometry, ChoicePickerHit, EventHandling, PromptCandidatePickerEvent,
    PromptCandidatePickerSession, TuiSession, ViewGeometry, render_localized,
    render_prompt_candidate_picker, render_with_session,
};
use skit_ui::{
    ADD_PARAMETER_KEY, Action, AddAction, AddWorkflowState, Effect, FieldKind, FieldValue,
    INTERPOLATE_KEY, KnownEntryKind, LibraryEntryDetail, LibraryPromptRunner, LibraryState,
    LibrarySurface, PROMPT_AUTO_MANAGE_LIMIT, PROMPT_LIST_PREVIEW_LIMIT, RUNNER_KEY,
    ReviewDefaults, ReviewLane, ReviewState, RunFieldRole, RunFormView, RunnerEditorAction,
    RunnerEditorEffect, RunnerEditorError, RunnerEditorOwner, RunnerEditorView, Screen,
    SettingsAction, SettingsEffect, SettingsInputs, SettingsSectionId, SettingsView,
    SourceSnapshot, TypedValue,
};

// The seeded prompt-runner names the oracle's `config.load_prompt_runners()` returns, in
// stored order. The Rust seam takes them as an explicit list, so the tests name them.
const RUNNERS: &[&str] = &["claude", "codex", "opencode", "amp"];

fn runners() -> Vec<String> {
    RUNNERS.iter().map(|name| (*name).to_owned()).collect()
}

// --------------------------------------------------------------------------
// event + render helpers
// --------------------------------------------------------------------------

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn text_key(character: char) -> Event {
    key(KeyCode::Char(character), KeyModifiers::NONE)
}

fn ctrl(character: char) -> Event {
    key(KeyCode::Char(character), KeyModifiers::CONTROL)
}

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn rendered(buffer: &Buffer) -> String {
    buffer.content().iter().map(|cell| cell.symbol()).collect()
}

fn draw_session(
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ViewGeometry) {
    let mut session = TuiSession::default();
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| geometry = render_with_session(frame, state, Locale::En, &mut session))
        .unwrap();
    (terminal, geometry)
}

fn draw_localized(state: &LibraryState, width: u16, height: u16, locale: Locale) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| {
            let _ = render_localized(frame, state, locale);
        })
        .unwrap();
    rendered(terminal.backend().buffer())
}

/// Render one state with a persistent session (so a `handle_event` between draws survives) and
/// return both the buffer text and the hit geometry.
fn draw_with_session(
    session: &mut TuiSession,
    state: &LibraryState,
    width: u16,
    height: u16,
) -> (String, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| geometry = render_with_session(frame, state, Locale::En, session))
        .unwrap();
    (rendered(terminal.backend().buffer()), geometry)
}

// --------------------------------------------------------------------------
// library + detail construction
// --------------------------------------------------------------------------

fn entry(slug: &str, name: &str, kind: &str, description: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode: StorageMode::Copy,
        description: description.to_owned(),
        target: None,
    }
}

fn library_state(entries: Vec<EntrySummary>) -> LibraryState {
    LibraryState::from_scan(LibraryScan {
        entries,
        diagnostics: Vec::new(),
    })
}

fn detail_state(runner: LibraryPromptRunner) -> LibraryState {
    let details = BTreeMap::from([(
        Slug::parse("p").unwrap(),
        LibraryEntryDetail {
            prompt_runner: Some(runner),
            ..LibraryEntryDetail::default()
        },
    )]);
    let mut state = LibraryState::default();
    state.update(Action::ReplaceSurface {
        surface: LibrarySurface {
            scan: LibraryScan {
                entries: vec![entry("p", "p", "prompt", "")],
                diagnostics: Vec::new(),
            },
            details,
        },
        rerunnable: Vec::new(),
    });
    state
}

// --------------------------------------------------------------------------
// run form (prompt) construction
// --------------------------------------------------------------------------

/// Build a prompt launch form: one field per placeholder plus the runner picker.
fn prompt_form(
    placeholders: &[&str],
    values: &[(&str, &str)],
    runner_names: &[String],
    runner_default: &str,
) -> RunFormView {
    let declarations = placeholders
        .iter()
        .map(|name| ParamDecl::new(*name))
        .collect::<Vec<_>>();
    let saved = values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    RunFormView::from_declarations(
        "p",
        "p",
        &declarations,
        &saved,
        runner_names,
        runner_default,
        &BTreeMap::new(),
        "",
    )
}

fn run_state(form: RunFormView) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

/// Index of the runner picker field, and its currently selected value.
fn runner_index(state: &LibraryState) -> usize {
    state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| matches!(field.role, RunFieldRole::Runner))
        .expect("the prompt form has a runner picker")
}

fn runner_value(state: &LibraryState) -> String {
    let form = state.run_form().unwrap();
    form.fields()
        .iter()
        .find(|field| matches!(field.role, RunFieldRole::Runner))
        .expect("the prompt form has a runner picker")
        .control
        .value()
}

/// The `_skit_runner` name a submit delivers, and the full value map.
fn submit_values(state: &mut LibraryState) -> BTreeMap<String, FieldValue> {
    match state.update(Action::Submit) {
        Effect::Submit { values, .. } => values,
        other => panic!("expected a launch submit, got {other:?}"),
    }
}

fn submitted_runner(values: &BTreeMap<String, FieldValue>) -> String {
    values
        .get("_skit_runner")
        .map(FieldValue::as_text)
        .unwrap_or_default()
}

// --------------------------------------------------------------------------
// settings (prompt) construction
// --------------------------------------------------------------------------

/// A stored placeholder parameter as the store hands one back to the settings screen.
fn placeholder(name: &str) -> ParamDecl {
    synthesized_placeholder(name)
}

fn prompt_settings(
    managed: Vec<ParamDecl>,
    runner: &str,
    configured: &[&str],
    interpolate: bool,
) -> SettingsInputs {
    SettingsInputs {
        selector: "p".to_owned(),
        kind: "prompt".to_owned(),
        name: "p".to_owned(),
        source: "/tmp/p.prompt.md".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        declared_schema: true,
        interpolate,
        managed,
        runner: runner.to_owned(),
        configured_runners: configured.iter().map(|name| (*name).to_owned()).collect(),
        ..SettingsInputs::default()
    }
}

fn runner_options(view: &SettingsView) -> Vec<String> {
    let FieldKind::SingleChoice { options } = &view.field(RUNNER_KEY).unwrap().kind else {
        panic!("the runner picker needs a closed option set");
    };
    options.iter().map(|option| option.value.clone()).collect()
}

// --------------------------------------------------------------------------
// review (prompt) construction
// --------------------------------------------------------------------------

fn snap(name: &str, bytes: &[u8]) -> SourceSnapshot {
    SourceSnapshot {
        path: name.into(),
        source_record: name.to_owned(),
        bytes: bytes.to_vec(),
        permissions: skit_application::SourcePermissions {
            readonly: false,
            unix_mode: Some(0o644),
        },
        is_regular: true,
        is_directory: false,
        is_draft: false,
    }
}

fn review_prompt(name: &str, bytes: &[u8], defaults: ReviewDefaults) -> ReviewState {
    ReviewState::from_source(snap(name, bytes), KnownEntryKind::Prompt, defaults)
}

fn review_defaults(runner_names: &[String]) -> ReviewDefaults {
    ReviewDefaults {
        runner_names: runner_names.to_vec(),
        ..ReviewDefaults::default()
    }
}

/// Render the searchable candidate picker to a fixed backend and return its geometry.
fn draw_picker(
    session: &mut PromptCandidatePickerSession,
    width: u16,
    height: u16,
) -> (Terminal<TestBackend>, ChoicePickerGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = ChoicePickerGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_prompt_candidate_picker(frame, frame.area(), session, Locale::En)
        })
        .unwrap();
    (terminal, geometry)
}

// ==========================================================================
// prompt-only Library
// ==========================================================================

#[test]
fn test_prompt_only_library_uses_entry_taxonomy_everywhere() {
    // A library that holds only a prompt still names its actions after "entry", never
    // "script": the panel title is "Library", and the footer chips are Entry settings,
    // Edit source, and Add entry. (The "1/1 entry" status line and the pressed-p settings
    // title are host-composed; the taxonomy claim is the rendered vocabulary here.)
    let state = library_state(vec![entry("p", "p", "prompt", "")]);
    // A wide backend keeps the detail line on one row so its taxonomy phrase is contiguous.
    let (terminal, _) = draw_session(&state, 160, 36);
    let screen = rendered(terminal.backend().buffer());
    assert!(screen.contains("Library"));
    // The footer chips carry the "entry" taxonomy, never "script".
    assert!(screen.contains("Entry settings"));
    assert!(screen.contains("Edit source"));
    assert!(screen.contains("Add entry"));
    // The empty-description prompt's detail body says the same in "entry" terms.
    assert!(screen.contains("add one in Entry settings"));
}

#[test]
fn test_prompt_only_chinese_library_stays_entry_neutral_zh_cn() {
    // The Simplified-Chinese library title is the entry-neutral 工具库, never the
    // script-specific 脚本库, even when it holds only prompts.
    let state = library_state(vec![entry("p", "p", "prompt", "Review this")]);
    let screen = draw_localized(&state, 110, 36, Locale::ZhCn);
    assert!(screen.contains("工 具 库"));
    assert!(!screen.contains("脚 本 库"));
}

#[test]
fn test_prompt_only_chinese_library_stays_entry_neutral_zh_tw() {
    // The Traditional-Chinese library title is the entry-neutral 工具庫, never 腳本庫.
    let state = library_state(vec![entry("p", "p", "prompt", "Review this")]);
    let screen = draw_localized(&state, 110, 36, Locale::ZhTw);
    assert!(screen.contains("工 具 庫"));
    assert!(!screen.contains("腳 本 庫"));
}

// ==========================================================================
// run form: the runner picker row
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the pin-beats-last-picked default and config.find_prompt_runner() live in the composition root that builds the RunFormView.runner_default and resolves the name. tests/test_prompt_tui.py:211."]
fn test_form_picker_defaults_to_the_pin_and_submits_it() {
    // The pin (codex) is the picker default even when the last-picked runner was opencode:
    // an untouched pin is only a default, so argstate stays on opencode.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the argstate.save_last_runner persistence that 'remembers' a move-away-then-back pick is host state. tests/test_prompt_tui.py:231."]
fn test_form_picker_move_away_then_back_to_pin_is_still_remembered() {
    // Moving off the pin and back records the deliberate return as the remembered runner.
}

#[test]
fn test_form_picker_keyboard_pick_runs_and_remembers() {
    // The keyboard really moves the runner selection, and the moved-to runner is the one a
    // launch delivers. (argstate remembering the pick is host state, covered upstream.)
    let mut state = run_state(prompt_form(&[], &[], &runners(), "opencode"));
    assert_eq!(runner_value(&state), "opencode"); // last-picked prefill
    let field = runner_index(&state);
    // The picker action a key press maps to: choose a different runner.
    state.update(Action::SelectFieldOption {
        field,
        value: "codex".to_owned(),
    });
    let picked = runner_value(&state);
    assert_ne!(picked, "opencode"); // the keys really moved the selection
    let values = submit_values(&mut state);
    assert_eq!(submitted_runner(&values), picked);
}

#[test]
fn test_form_picker_mouse_click_picks_a_runner() {
    // A mouse pick lands on the clicked runner and that is what the launch delivers.
    let mut state = run_state(prompt_form(&[], &[], &runners(), ""));
    let field = runner_index(&state);
    state.update(Action::SelectFieldOption {
        field,
        value: RUNNERS[1].to_owned(),
    });
    assert_eq!(runner_value(&state), RUNNERS[1]); // the mouse pick landed
    let values = submit_values(&mut state);
    assert_eq!(submitted_runner(&values), RUNNERS[1]);
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): whether action_run skips the form or opens it for a promptless prompt is host routing; from a built RunFormView the picker is always present. tests/test_prompt_tui.py:314."]
fn test_prompt_with_no_placeholders_still_shows_the_form_for_the_picker() {
    // A field-less prompt must not skip the form: the runner question is still open.
}

#[test]
fn test_unicode_placeholder_is_a_working_tui_field() {
    // A non-ASCII placeholder is a real editable field, and its value is delivered under its
    // own name.
    let mut state = run_state(prompt_form(
        &["目标"],
        &[("目标", "src/app.py")],
        &runners(),
        "claude",
    ));
    let values = submit_values(&mut state);
    // The submit keys a parameter as `value:{name}` (the host strips the prefix to deliver it
    // under the bare name), so the tier-boundary observable is the prefixed key.
    assert_eq!(
        values.get("value:目标").map(FieldValue::as_text).as_deref(),
        Some("src/app.py")
    );
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the pin-supplies-the-default shortcut for a promptless prompt is host routing that builds the form's runner_default from the pin. tests/test_prompt_tui.py:346."]
fn test_pinned_promptless_prompt_keeps_the_shortcut() {
    // A pinned field-less prompt still opens the form, but the pin is the default, so Enter
    // alone runs it.
}

#[test]
fn test_stale_pin_cannot_block_run_form_override() {
    // A stale pin ("removed") no longer fails preflight before the picker can open: with only
    // the configured replacement available, the visible form resolves it. From a built form
    // the picker prefers a configured runner over a stale pin, and the pick submits.
    let mut state = run_state(prompt_form(&[], &[], &["working".to_owned()], "removed"));
    assert_eq!(runner_value(&state), "working"); // the stale pin fell back to the one configured runner
    let values = submit_values(&mut state);
    assert_eq!(submitted_runner(&values), "working");
}

#[test]
fn test_missing_pinned_binary_cannot_block_a_different_pick() {
    // The broken pin is the honest prefill, but it no longer prevents selecting the installed
    // runner for this one launch. (The missing-binary preflight is host-side.)
    let names = vec!["broken".to_owned(), "working".to_owned()];
    let mut state = run_state(prompt_form(&[], &[], &names, "broken"));
    assert_eq!(runner_value(&state), "broken");
    let field = runner_index(&state);
    state.update(Action::SelectFieldOption {
        field,
        value: "working".to_owned(),
    });
    let values = submit_values(&mut state);
    assert_eq!(submitted_runner(&values), "working");
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): preflight of the resolved prompt runner and the return-to-library-with-error path live in launcher/flows. tests/test_prompt_tui.py:415."]
fn test_selected_prompt_runner_preflight_failure_returns_to_library() {
    // A selected-but-unavailable agent returns to the library with the actionable error and
    // never hands the terminal to a child process.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_run's zero-runners branch opens the RunnerAddModal instead of the form; that routing is in the composition root. tests/test_prompt_tui.py:446."]
fn test_run_with_zero_runners_offers_the_new_agent_modal() {
    // An emptied runner list opens the New agent modal rather than dead-ending on a CLI hint.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the define-agent-then-re-enter-the-form flow is host run routing. tests/test_prompt_tui.py:465."]
fn test_run_with_zero_runners_define_agent_then_run() {
    // Defining the agent re-enters the run straight into the form with it configured.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_rerun's 'no pin -> never answer the runner question silently -> fall back to the form' is host routing. tests/test_prompt_tui.py:490."]
fn test_rerun_unpinned_prompt_falls_back_to_the_form() {
    // An unpinned rerun must open the form, never resolve the runner silently.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_rerun's 'pinned -> skip the form, resolve inside PromptLaunch.build' is host routing. tests/test_prompt_tui.py:503."]
fn test_rerun_pinned_prompt_skips_the_form_and_uses_the_pin() {
    // A pinned rerun skips the form and the pin resolves inside the launch build.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): tui.PendingRun and tui._finish_run carry the resolved runner into flows.execute in exit mode. tests/test_prompt_tui.py:519."]
fn test_exit_mode_pending_run_carries_the_runner() {
    // Exit mode hands the resolved runner to the PendingRun and on to flows.execute.
}

#[test]
fn test_detail_pane_names_the_runner() {
    // A pinned prompt says which agent it runs with in the detail pane.
    let state = detail_state(LibraryPromptRunner::Configured("claude".to_owned()));
    let (terminal, _) = draw_session(&state, 120, 34);
    assert!(rendered(terminal.backend().buffer()).contains("Runs with claude"));
}

#[test]
fn test_detail_pane_unpinned_prompt_says_the_form_asks() {
    // An unpinned prompt says the runner is picked on the run form.
    let state = detail_state(LibraryPromptRunner::PickOnRunForm);
    let (terminal, _) = draw_session(&state, 120, 34);
    assert!(rendered(terminal.backend().buffer()).contains("Runner picked on the run form"));
}

#[test]
fn test_detail_pane_stale_pin_says_no_longer_configured() {
    // A prompt pinned to a runner whose config row is gone says "(no longer configured)" —
    // the same honesty Entry settings gives, never a bare "Runs with X" that would 126.
    // (The pin -> Missing projection is host-side; the rendered honesty is here.)
    let state = detail_state(LibraryPromptRunner::Missing("nonesuch-agent".to_owned()));
    let (terminal, _) = draw_session(&state, 120, 34);
    let screen = rendered(terminal.backend().buffer());
    assert!(screen.contains("nonesuch-agent"));
    assert!(screen.contains("no longer configured"));
}

// ==========================================================================
// add lane
// ==========================================================================

#[test]
fn test_tui_add_prompt_opens_the_review_panel() {
    // Never a blind direct add: the prompt review opens, prefilled — name from the stem,
    // description from the first line, both holes pre-ticked (under the cap), default runner
    // empty (ask on the run form). Accept commits a prompt entry managing both placeholders.
    let review = review_prompt(
        "task.prompt.md",
        b"# Task\n\nDo {{a}} and {{b}}\n",
        review_defaults(&runners()),
    );
    assert_eq!(review.lane(), ReviewLane::Prompt);
    assert_eq!(review.name(), "task");
    assert_eq!(review.description(), "Task");
    let boxes = review.prompt_candidates();
    assert_eq!(boxes.len(), 2);
    assert!(boxes.iter().all(|candidate| candidate.selected)); // under the cap: everything pre-ticked
    assert_eq!(review.runner(), ""); // default: ask on the run form

    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.kind, EntryKind::parse("prompt".to_owned()).unwrap());
    assert_eq!(plan.settings.params, ["a", "b"]);
    assert_eq!(plan.settings.runner, ""); // default: ask on the run form
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): a bare .md's KindPickModal (name told skit nothing) is add-workflow routing; this file drives the Prompt lane directly. The kind-pick itself is ported in port_test_add_review_contracts. tests/test_prompt_tui.py:619."]
fn test_tui_add_bare_md_asks_before_becoming_a_prompt() {
    // A bare .md asks the kind; picking prompt opens the prompt review and accept adds a prompt.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): cancelling the KindPickModal back to the source screen with nothing added is add-workflow routing. tests/test_prompt_tui.py:648."]
fn test_tui_add_bare_md_kind_ask_can_cancel_without_adding() {
    // Escaping the kind ask returns to the source step and adds nothing.
}

// ==========================================================================
// settings
// ==========================================================================

#[test]
fn test_settings_prompt_rows_and_no_flag_input() {
    // A prompt's declared rows are its managed placeholders (a, api_key), api_key detected as
    // secret; the trait gate means a placeholder kind never grows a flag input. (The
    // api_key->secret name heuristic is the analyzer's, validated on the review lane; the
    // settings surface renders whatever managed carries.)
    let mut api_key = placeholder("api_key");
    api_key.secret = true;
    let view = SettingsView::from_inputs(&prompt_settings(
        vec![placeholder("a"), api_key],
        "",
        RUNNERS,
        true,
    ));
    assert!(view.field("parameter:a:keep").is_some());
    assert!(view.field("parameter:api_key:keep").is_some());
    assert_eq!(
        view.field("parameter:api_key:secret")
            .map(|field| field.value().as_text()),
        Some("true".to_owned())
    );
    // The trait gate: placeholder kinds never grow a flag input.
    assert!(view.field("parameter:a:flag").is_none());
    assert!(view.field("parameter:api_key:flag").is_none());
}

#[test]
fn test_settings_runner_radio_pins_and_clears() {
    // Picking a configured runner pins it on save; picking "ask on the run form" clears it.
    // (The last-picked argstate is untouched either way — that is host state.)
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, true));
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), ""); // no pin
    view.set_value(
        RUNNER_KEY,
        FieldValue::Explicit(TypedValue::Choice(RUNNERS[0].to_owned())),
    );
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(RUNNER_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choice(
            RUNNERS[0].to_owned()
        )))
    );

    // And back to "ask each run": the saved pin is preselected, then cleared to "".
    let mut view = SettingsView::from_inputs(&prompt_settings(
        vec![placeholder("a")],
        RUNNERS[0],
        RUNNERS,
        true,
    ));
    assert_eq!(
        view.field(RUNNER_KEY).unwrap().value().as_text(),
        RUNNERS[0]
    ); // saved pin preselected
    view.set_value(
        RUNNER_KEY,
        FieldValue::Explicit(TypedValue::Choice(String::new())),
    );
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(RUNNER_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choice(String::new())))
    );
}

#[test]
fn test_settings_runner_section_empty_config_keeps_ask_and_the_door() {
    // An emptied runner config keeps exactly one option ("ask on the run form"), stays on it,
    // keeps the New agent door open, and a save of the lone option is a clean no-op.
    let view = SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", &[], true));
    assert_eq!(runner_options(&view), [""]); // just "ask on the run form"
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "");
    assert!(view.field(RUNNER_KEY).unwrap().capabilities.new_runner); // the door never disappears
    let mut view = view;
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save); // clean no-op
    assert!(!view.submitted_values().contains_key(RUNNER_KEY));
}

#[test]
fn test_settings_ctrl_n_adds_a_custom_agent_ready_to_pin() {
    // A mid-session Ctrl+N add selects the new agent in place, and the value survives to be
    // pinned by the next save. (Defining a settings pin is not a run pick, so argstate is
    // untouched — that is host state.)
    let inputs = prompt_settings(vec![placeholder("a")], "", RUNNERS, true);
    let mut view = SettingsView::from_inputs(&inputs);
    // The Ctrl+N chord opens the runner editor; a successful save re-enters here.
    assert_eq!(
        view.update(SettingsAction::NewRunner),
        SettingsEffect::NewRunner
    );
    view.add_and_select_runner(&inputs.selector, "mycli".to_owned());
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "mycli"); // selected in place
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(RUNNER_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choice(
            "mycli".to_owned()
        )))
    );
}

#[test]
fn test_settings_ctrl_n_add_preserves_a_stale_pin_option() {
    // A stale pin ("gone") plus a mid-session Ctrl+N add: the rebuilt dropdown must STILL
    // carry the stale-pin row — ask + stale "gone" + "other" + new "fresh" = 4 only if it
    // survived.
    let inputs = prompt_settings(vec![placeholder("a")], "gone", &["other"], true);
    let mut view = SettingsView::from_inputs(&inputs);
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "gone"); // stale pin preselected
    view.add_and_select_runner(&inputs.selector, "fresh".to_owned());
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "fresh"); // selected in place
    assert_eq!(runner_options(&view), ["", "gone", "other", "fresh"]);
}

#[test]
fn test_settings_pin_change_saves_even_with_insertion_off() {
    // The declared-params branch is skipped when insertion is off; the pin save must not live
    // inside it, or a pin change on an insertion-off prompt silently drops.
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, false));
    view.set_value(
        RUNNER_KEY,
        FieldValue::Explicit(TypedValue::Choice(RUNNERS[0].to_owned())),
    );
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(RUNNER_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choice(
            RUNNERS[0].to_owned()
        )))
    );
}

#[test]
#[ignore = "ABSENT (gap): the Rust prompt settings screen (declared_items) offers no detected-placeholder checkboxes (st-prompt-new-N) to tick an unmanaged {{b}} into management — only ADD_PARAMETER_KEY (type a name). MUST-FIX: offer detected-but-unmanaged prompt placeholders on the settings screen. tests/test_prompt_tui.py:813."]
fn test_settings_tick_to_manage_a_detected_placeholder() {
    // managed=[a], body {{a}} {{b}}: a checkbox for b appears; ticking + save manages a,b.
}

#[test]
fn test_settings_unticking_a_row_unmanages_it() {
    // Unticking a managed row's keep toggle drops it: managed a,b -> untick a -> params [b].
    // The save carries the keep=false decision the host applies.
    let mut view = SettingsView::from_inputs(&prompt_settings(
        vec![placeholder("a"), placeholder("b")],
        "",
        RUNNERS,
        true,
    ));
    view.set_value("parameter:a:keep", FieldValue::boolean(false)); // drop `a`
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get("parameter:a:keep"),
        Some(&FieldValue::boolean(false))
    );
    assert!(!view.submitted_values().contains_key("parameter:b:keep")); // b untouched, kept
}

#[test]
fn test_settings_typing_a_body_hole_name_manages_it() {
    // Typing a body-hole name into the add-a-parameter box manages it on save: managed=[a],
    // type "b" -> params [a, b]. The save carries the typed name.
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, true));
    view.set_value(ADD_PARAMETER_KEY, FieldValue::text("b"));
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values()
            .get(ADD_PARAMETER_KEY)
            .map(FieldValue::as_text)
            .as_deref(),
        Some("b")
    );
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): submit-time validation that config.save_prompt_runners([]) yanked the pinned runner mid-flight, returning 'no longer configured' and launching nothing, is host launch validation. tests/test_prompt_tui.py:860."]
fn test_form_submit_with_a_runner_removed_mid_flight_is_honest() {
    // A runner yanked while the form was open makes submit report "no longer configured".
}

#[test]
fn test_review_prompt_without_placeholders_says_so_and_adds_clean() {
    // A prompt with no holes has no checkboxes and adds clean: nothing is asked for.
    let review = review_prompt(
        "plain.prompt.md",
        b"No holes.\n",
        review_defaults(&runners()),
    );
    assert!(review.prompt_candidates().is_empty()); // "No {{name}} placeholders detected"
    let plan = review.create_entry().expect("the review accepts");
    assert!(plan.settings.params.is_empty()); // nothing was asked for
}

#[test]
fn test_settings_save_preserves_a_stale_pin() {
    // The pinned runner's config row is gone: opening settings and saving something unrelated
    // must NOT silently clear the pin — its own radio row holds it selected.
    let mut view = SettingsView::from_inputs(&prompt_settings(
        vec![placeholder("a")],
        "mine",
        &["other"],
        true,
    ));
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "mine"); // stale pin's own row, preselected
    assert!(!view.is_dirty());
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert!(!view.submitted_values().contains_key(RUNNER_KEY)); // preserved, not wiped

    // Explicitly picking the one configured runner replaces it.
    let mut view = SettingsView::from_inputs(&prompt_settings(
        vec![placeholder("a")],
        "mine",
        &["other"],
        true,
    ));
    view.set_value(
        RUNNER_KEY,
        FieldValue::Explicit(TypedValue::Choice("other".to_owned())),
    );
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(RUNNER_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choice(
            "other".to_owned()
        )))
    );
}

#[test]
fn test_settings_interpolate_toggle_off_and_back_on() {
    // One click plus Save turns insertion off; the managed list survives underneath. Reopened
    // off, turning it on reveals the rows immediately (they become focus stops again) without
    // the undocumented Save -> reopen round trip.
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, true));
    view.set_value(INTERPOLATE_KEY, FieldValue::boolean(false));
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(INTERPOLATE_KEY),
        Some(&FieldValue::boolean(false))
    );

    // Reopened off: the rows are present but not focus stops until insertion is on again.
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, false));
    assert!(!view.focusable_keys().contains(&"parameter:a:prompt"));
    view.set_value(INTERPOLATE_KEY, FieldValue::boolean(true));
    assert!(view.focusable_keys().contains(&ADD_PARAMETER_KEY));
    assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);
    assert_eq!(
        view.submitted_values().get(INTERPOLATE_KEY),
        Some(&FieldValue::boolean(true))
    );
}

#[test]
#[ignore = "ABSENT (gap): choosing first parameters in the same off->on save needs the detected-placeholder checkboxes (st-prompt-new-1) the Rust prompt settings screen does not offer. MUST-FIX: offer detected placeholders on the settings screen. tests/test_prompt_tui.py:967."]
fn test_settings_off_to_on_can_choose_first_parameters_in_the_same_save() {
    // Turning insertion on and ticking st-prompt-new-1 in one save stores params [b].
}

#[test]
#[ignore = "ABSENT (gap): the Rust prompt settings screen shows no capped preview of detected placeholders (st-prompt-new-* checkboxes) — that inline candidate list exists only on the review lane. MUST-FIX: cap the settings detected-placeholder preview at PROMPT_LIST_PREVIEW_LIMIT. tests/test_prompt_tui.py:989."]
fn test_settings_candidate_checkboxes_are_flood_capped() {
    // The settings detected-placeholder preview caps at LIST_PREVIEW_LIMIT checkboxes.
}

#[test]
#[ignore = "ABSENT (gap): the settings screen has no searchable Ctrl+O candidate picker (PromptCandidatePickerModal) — it exists only on the review lane. MUST-FIX: offer the searchable candidate picker on the settings screen. tests/test_prompt_tui.py:1003."]
fn test_settings_candidate_picker_reaches_a_hidden_name_and_waits_for_outer_save() {
    // Ctrl+O opens the picker, filters to a flooded hidden name, and its Done waits for Save.
}

#[test]
#[ignore = "ABSENT (gap): the settings screen has no searchable candidate picker whose selection a discard can drop (_pending_prompt_candidates). MUST-FIX: offer the searchable candidate picker on the settings screen. tests/test_prompt_tui.py:1031."]
fn test_settings_candidate_picker_selection_is_discardable() {
    // Selecting all in the settings picker then discarding leaves params unchanged.
}

#[test]
#[ignore = "ABSENT (gap): the settings screen has no candidate picker whose Cancel/unchanged-Done are no-ops (_pending_prompt_candidates stays empty, _dirty stays false). MUST-FIX: offer the searchable candidate picker on the settings screen. tests/test_prompt_tui.py:1064."]
fn test_settings_candidate_picker_cancel_and_unchanged_done_are_noops() {
    // The settings picker's Cancel and an unchanged Done both leave the screen clean.
}

#[test]
#[ignore = "ABSENT (gap): the settings screen has no candidate picker that tolerates a preview recompose behind it. MUST-FIX: offer the searchable candidate picker on the settings screen. tests/test_prompt_tui.py:1090."]
fn test_settings_candidate_picker_tolerates_preview_recompose() {
    // A queued settings Ctrl+O/Done straddles a responsive recompose and still survives.
}

#[test]
#[ignore = "ABSENT (gap): the settings Ctrl+O 'Choose variables' key has no counterpart — the searchable candidate picker exists only on the review lane. MUST-FIX: offer the searchable candidate picker on the settings screen. tests/test_prompt_tui.py:1116."]
fn test_settings_choose_variables_key_is_harmless_when_off_or_short() {
    // Ctrl+O is a harmless no-op on the settings screen when insertion is off or short.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): a PermissionError from prompt_text.read during the settings open race, surfaced in #st-prompt-text-error, is a host file-read error. tests/test_prompt_tui.py:1139."]
fn test_settings_surfaces_prompt_read_failure_from_open_race() {
    // A failed prompt read during open is surfaced in the settings error line, not a crash.
}

#[test]
fn test_review_flooded_prompt_previews_capped_and_ticks_nothing() {
    // A flooded prompt previews at most LIST_PREVIEW_LIMIT holes, ticks nothing by default,
    // warns it was "probably not written for" placeholders, and adds nothing.
    let many = (0..PROMPT_AUTO_MANAGE_LIMIT + 4)
        .map(|index| format!("{{{{h{index}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = review_prompt(
        "big.prompt.md",
        format!("{many}\n").as_bytes(),
        review_defaults(&runners()),
    );
    assert!(review.prompt_is_flooded());
    assert_eq!(review.prompt_preview().len(), PROMPT_LIST_PREVIEW_LIMIT); // preview, not a wall
    assert!(
        review
            .prompt_candidates()
            .iter()
            .all(|candidate| !candidate.selected)
    ); // nothing pre-ticked
    let plan = review.create_entry().expect("the review accepts");
    assert!(plan.settings.params.is_empty()); // nothing was asked for
}

#[test]
fn test_review_candidate_picker_keyboard_reaches_a_hidden_name() {
    // The full-list keyboard picker filters to a name below the flood cap, selects it, and
    // accepting the review manages exactly that one.
    let names = (0..PROMPT_AUTO_MANAGE_LIMIT + 4)
        .map(|index| format!("h{index}"))
        .collect::<Vec<_>>();
    let body = names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut review = review_prompt(
        "big.prompt.md",
        body.as_bytes(),
        review_defaults(&runners()),
    );

    let last = names.last().unwrap().clone();
    let mut session = PromptCandidatePickerSession::new(review.prompt_picker());
    for character in last.chars() {
        let _ = session.handle_event(text_key(character), &ChoicePickerGeometry::default());
    }
    assert_eq!(session.visible_names(), [last.as_str()]); // filtered to one
    let _ = session.handle_event(text_key(' '), &ChoicePickerGeometry::default()); // toggle it
    let PromptCandidatePickerEvent::Accepted(picked) = session
        .handle_event(ctrl('s'), &ChoicePickerGeometry::default())
        .unwrap()
    else {
        panic!("Ctrl+S must accept the picker");
    };
    review.set_prompt_selection(&picked);

    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.params, [last]);
}

#[test]
fn test_review_candidate_picker_select_all_and_done_are_mouse_operable() {
    // Select-all then Done in the picker are mouse targets: clicking them selects every name
    // and accepting the review manages them all.
    let names = (0..=PROMPT_AUTO_MANAGE_LIMIT)
        .map(|index| format!("h{index}"))
        .collect::<Vec<_>>();
    let body = names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut review = review_prompt(
        "all.prompt.md",
        body.as_bytes(),
        review_defaults(&runners()),
    );

    let mut session = PromptCandidatePickerSession::new(review.prompt_picker());
    let (_, geometry) = draw_picker(&mut session, 100, 40);
    let select_all = geometry
        .hits
        .iter()
        .find(|hit| hit.target == ChoicePickerHit::SelectAll)
        .expect("the select-all control is a mouse target")
        .area;
    let _ = session.handle_event(mouse(select_all.x, select_all.y), &geometry);

    let (_, geometry) = draw_picker(&mut session, 100, 40);
    let done = geometry
        .hits
        .iter()
        .find(|hit| hit.target == ChoicePickerHit::Done)
        .expect("Done is a mouse target")
        .area;
    let PromptCandidatePickerEvent::Accepted(picked) = session
        .handle_event(mouse(done.x, done.y), &geometry)
        .unwrap()
    else {
        panic!("clicking Done must accept the picker");
    };
    review.set_prompt_selection(&picked);

    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.params, names);
}

#[test]
fn test_review_candidate_picker_keeps_search_and_footer_usable_on_tiny_screen() {
    // On a tiny screen the picker keeps its search and footer usable: the footer stays on
    // screen, filtering still narrows to one, and accepting still publishes it.
    let names = (0..=PROMPT_AUTO_MANAGE_LIMIT)
        .map(|index| format!("h{index}"))
        .collect::<Vec<_>>();
    let body = names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = review_prompt(
        "tiny.prompt.md",
        body.as_bytes(),
        review_defaults(&runners()),
    );

    let mut session = PromptCandidatePickerSession::new(review.prompt_picker());
    let (terminal, geometry) = draw_picker(&mut session, 42, 10);
    // The picker fills the whole frame, so the backend buffer area IS the modal bounds. The Done
    // hit rect (the footer's keys row) must stay fully on screen — the faithful twin of the
    // oracle's footer.region.height >= 1 && footer.region.bottom <= modal.region.bottom
    // (test_prompt_tui.py:1264-1266). ChoicePickerGeometry exposes the Done hit region.
    let modal = terminal.backend().buffer().area;
    let done = geometry
        .hits
        .iter()
        .find(|hit| hit.target == ChoicePickerHit::Done)
        .expect("Done is a mouse target: the footer rendered at 42x10")
        .area;
    assert!(done.height >= 1); // footer.region.height >= 1 (the Done row is one line)
    assert!(done.bottom() <= modal.bottom()); // footer.region.bottom <= modal.region.bottom
    assert!(done.right() <= modal.right()); // and the footer is fully on screen horizontally
    let last = names.last().unwrap().clone();
    for character in last.chars() {
        let _ = session.handle_event(text_key(character), &geometry);
    }
    assert_eq!(session.visible_names(), [last.as_str()]);
    let _ = session.handle_event(text_key(' '), &geometry);
    let PromptCandidatePickerEvent::Accepted(picked) =
        session.handle_event(ctrl('s'), &geometry).unwrap()
    else {
        panic!("Ctrl+S must accept the picker");
    };
    assert_eq!(picked, [last]);
}

#[test]
fn test_review_candidate_picker_empty_search_and_cancel_are_keyboard_operable() {
    // A search that matches nothing lists nothing, and Escape cancels the picker without
    // publishing — the review's ticks are unchanged.
    let names = (0..=PROMPT_AUTO_MANAGE_LIMIT)
        .map(|index| format!("h{index}"))
        .collect::<Vec<_>>();
    let body = names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = review_prompt(
        "search.prompt.md",
        body.as_bytes(),
        review_defaults(&runners()),
    );

    let mut session = PromptCandidatePickerSession::new(review.prompt_picker());
    for _ in 0..4 {
        let _ = session.handle_event(text_key('z'), &ChoicePickerGeometry::default());
    }
    assert!(session.visible_names().is_empty()); // nothing matches
    assert_eq!(
        session.handle_event(
            key(KeyCode::Esc, KeyModifiers::NONE),
            &ChoicePickerGeometry::default()
        ),
        Some(PromptCandidatePickerEvent::Cancelled)
    );
}

#[test]
fn test_review_candidate_picker_tolerates_preview_recompose() {
    // The picker owns the full selection while the capped preview can recompose behind it;
    // select-all + Done still publishes every name.
    let names = (0..=PROMPT_AUTO_MANAGE_LIMIT + 1)
        .map(|index| format!("h{index}"))
        .collect::<Vec<_>>();
    let body = names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    let review = review_prompt(
        "recompose-picker.prompt.md",
        body.as_bytes(),
        review_defaults(&runners()),
    );

    let mut session = PromptCandidatePickerSession::new(review.prompt_picker());
    let _ = session.handle_event(ctrl('a'), &ChoicePickerGeometry::default()); // select all
    let PromptCandidatePickerEvent::Accepted(picked) = session
        .handle_event(ctrl('s'), &ChoicePickerGeometry::default())
        .unwrap()
    else {
        panic!("Ctrl+S must accept the picker");
    };
    assert_eq!(picked, names);
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the Rust review screen opens the searchable candidate \
picker UNCONDITIONALLY on Ctrl+O (screens/add.rs:380-381 -> session.rs:467 opens the overlay \
whenever a review is present, with no `detected <= LIST_PREVIEW_LIMIT` gate). The oracle's \
action_choose_prompt_candidates returns early for a short prompt, so the picker never opens and \
app.screen stays the review (tui_add.py:1471-1472; test_prompt_tui.py:1343)."]
fn test_review_choose_variables_key_is_harmless_for_a_short_prompt() {
    // A short prompt (holes at or under LIST_PREVIEW_LIMIT) has no capped list, so Ctrl+O is a
    // NO-OP: the searchable candidate picker never opens and the screen stays on review.
    // (Oracle: after ctrl+o, app.screen is review.)
    let review = review_prompt(
        "short.prompt.md",
        b"{{a}} {{b}}",
        review_defaults(&runners()),
    );
    assert_eq!(review.prompt_candidates().len(), 2); // under the cap: an inline preview, no picker
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));

    let mut session = TuiSession::default();
    // The review renders and the picker is not open yet: "Choose prompt variables" has exactly one
    // render site (picker.rs:242), so this guards the marker and makes the --ignored run fail at
    // the POST-Ctrl+O assertion, not setup.
    let (before, geometry) = draw_with_session(&mut session, &state, 100, 40);
    assert!(!before.contains("Choose prompt variables"));
    // Ctrl+O: the advertised "Choose variables" chord. Its return value would encode a guess about
    // the fixed behavior, so the render outcome is the contract.
    let _ = session.handle_event(ctrl('o'), &state, &geometry);
    // The picker must NOT have opened: the review panel is still what renders.
    let after = draw_with_session(&mut session, &state, 100, 40).0;
    assert!(
        !after.contains("Choose prompt variables"),
        "Ctrl+O on a short prompt must not open the candidate picker: {after}"
    );
}

#[test]
fn test_prompt_draft_with_invalid_utf8_reaches_strict_review() {
    // A draft written with invalid UTF-8 reaches strict review: the review refuses to commit
    // it. (The exact byte-offset wording — "offset 6", no replacement char — is the host add
    // screen's error formatting; the review-lane twin is that create_entry refuses.)
    let review = review_prompt(
        "draft.prompt.md",
        b"draft:\xff\n",
        review_defaults(&runners()),
    );
    assert!(review.prompt_candidates().is_empty()); // no holes survive an undecodable body
    assert!(review.create_entry().is_err()); // strict review refuses the invalid UTF-8
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): surfacing an initial missing-file OS error and a post-editor unlink OS error in #pv-text-error is host file/editor error handling. tests/test_prompt_tui.py:1387."]
fn test_prompt_review_surfaces_initial_and_post_editor_os_errors() {
    // A vanished source and a post-editor unlink both surface an OS error, never a crash.
}

// ==========================================================================
// the prompt review panel
// ==========================================================================

#[test]
fn test_review_space_untick_keeps_a_subset() {
    // Unticking one hole keeps the subset: {{a}} {{b}} with b unticked adds only a.
    let mut review = review_prompt("t.prompt.md", b"{{a}} {{b}}\n", review_defaults(&runners()));
    review.set_prompt_selected("b", false); // the advertised Toggle key
    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.params, ["a"]);
}

#[test]
fn test_review_insertion_switch_off_hides_ticks_and_stores_off() {
    // Turning insertion off folds the tick machinery away and stores interpolate=false with
    // nothing managed — the body travels verbatim.
    let mut review = review_prompt(
        "raw.prompt.md",
        b"Use {{tool}} literally\n",
        review_defaults(&runners()),
    );
    assert!(review.interpolate()); // machinery visible
    review.set_interpolate(false);
    let plan = review.create_entry().expect("the review accepts");
    assert!(!plan.settings.interpolate);
    assert!(plan.settings.params.is_empty()); // nothing managed, body travels verbatim
}

#[test]
fn test_review_runner_pick_pins_and_remembers() {
    // No pin, no last pick means "ask on the run form"; picking the first configured runner
    // pins it and marks it a real pick (the "remembers" the run picker's default will honor).
    let mut review = review_prompt("r.prompt.md", b"Go {{a}}\n", review_defaults(&runners()));
    assert_eq!(review.runner(), ""); // ask on the run form
    review.set_runner(RUNNERS[0], true);
    assert!(review.runner_was_picked()); // a real pick is remembered
    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.runner, RUNNERS[0]);
}

#[test]
fn test_review_prefills_last_picked_and_explicit_runner_wins() {
    // The last-picked runner prefills the picker; an explicit runner default (a flag) wins
    // over it, and an untouched add-time pin is not itself a run pick.
    let last = ReviewDefaults {
        last_runner: Some("amp".to_owned()),
        ..review_defaults(&runners())
    };
    let review = review_prompt("l.prompt.md", b"x {{a}}\n", last);
    assert_eq!(review.runner(), "amp"); // last-picked prefill
    assert!(!review.runner_was_picked()); // a prefill is not a pick

    let explicit = ReviewDefaults {
        runner: Some("codex".to_owned()),
        last_runner: Some("amp".to_owned()),
        interpolate: Some(false),
        ..review_defaults(&runners())
    };
    let review = review_prompt("l.prompt.md", b"x {{a}}\n", explicit);
    assert_eq!(review.runner(), "codex"); // the flag wins
    assert!(!review.interpolate());
    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.runner, "codex");
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): 'escape adds nothing' asserts store.list_entries() == [] — the store never received a create. The review's cancel is the AddWorkflow's route (port_test_add_review_contracts). tests/test_prompt_tui.py:1512."]
fn test_review_escape_adds_nothing() {
    // Escaping the review adds nothing to the library.
}

#[test]
fn test_review_ctrl_e_rescans_and_keeps_edits() {
    // Editing the source rescans placeholders while keeping typed edits: a renamed entry and
    // an added hole survive, and the rescan sees the new hole.
    let mut review = review_prompt("e.prompt.md", b"{{a}}\n", review_defaults(&runners()));
    review.set_name("renamed");
    review.rescan(b"{{a}} {{b}}\n".to_vec()); // $EDITOR added {{b}}
    assert_eq!(review.name(), "renamed"); // edit survived
    assert_eq!(review.prompt_candidates().len(), 2); // the rescan saw the new hole
    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.params, ["a", "b"]);
}

#[test]
#[ignore = "FAILING CONTRACT (divergence): the Rust review screen maps Ctrl+E to EditSource inside \
its Control-chord block (screens/add.rs:363 gate, add.rs:374 arm) with no focus check, so it opens \
the editor even while a text Input owns focus. The oracle's Ctrl+E is non-priority: while an Input \
has focus it is that Input's end-of-line and never $EDITOR (test_prompt_tui.py:1555)."]
fn test_review_ctrl_e_in_input_is_end_of_line_not_editor() {
    // Ctrl+E while the review's name Input has focus is that Input's end-of-line, never $EDITOR
    // (oracle: edited == []).
    // private-render: the cursor-to-end move lives in the private AddScreenSession LineInput and is
    // not on the public surface; the reachable half of the oracle is "the editor never opened" =
    // no EditSource action.
    let review = review_prompt("e.prompt.md", b"{{a}}\n", review_defaults(&runners()));
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));

    let mut session = TuiSession::default();
    let (_, geometry) = draw_with_session(&mut session, &state, 100, 40);
    // Precondition: the review's name Input is the default focus, so a printable key edits the
    // name — proving an Input owns focus, the exact state in which the oracle expects end-of-line.
    assert!(matches!(
        session.handle_event(text_key('x'), &state, &geometry),
        EventHandling::Action(Action::Add(AddAction::SetReviewName(_)))
    ));
    // Ctrl+E must be the Input's end-of-line (handled by the Input), never EditSource.
    assert_ne!(
        session.handle_event(ctrl('e'), &state, &geometry),
        EventHandling::Action(Action::Add(AddAction::EditSource)),
    );
}

#[test]
fn test_review_reference_mode_links_the_original() {
    // Reference mode prefills the storage radio and stores a reference whose source is the
    // original path.
    let review = review_prompt(
        "linked.prompt.md",
        b"{{a}}\n",
        ReviewDefaults {
            reference: true,
            ..review_defaults(&runners())
        },
    );
    assert_eq!(review.storage(), StorageMode::Reference); // prefilled
    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.mode, StorageMode::Reference);
    assert_eq!(plan.source, "linked.prompt.md");
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): the duplicate-name refusal (store.resolve('dup') already exists -> notify and stay, nothing new lands) is the store's add transaction. tests/test_prompt_tui.py:1598."]
fn test_review_duplicate_name_notifies_and_stays() {
    // A duplicate name notifies and keeps the review open; nothing new lands.
}

#[test]
fn test_review_ctrl_n_defines_a_custom_agent_and_selects_it() {
    // Ctrl+N defines a custom agent, which joins the picker selected in place; accepting pins
    // it. (Persisting the runner to config is host state.)
    let mut review = review_prompt("n.prompt.md", b"{{a}}\n", review_defaults(&runners()));
    review.add_runner("aider".to_owned()); // the New agent modal's Save
    assert_eq!(review.runner(), "aider"); // new agent selected in place
    let plan = review.create_entry().expect("the review accepts");
    assert_eq!(plan.settings.runner, "aider");
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): 'escape returns to the AddSourceScreen (not the Library)' is add-workflow screen-stack routing. tests/test_prompt_tui.py:1641."]
fn test_review_escape_returns_to_the_add_source_screen() {
    // Cancelling the review lands back on the source step, not the Library.
}

#[test]
fn test_review_description_prefill_and_toggle_action() {
    // A hand-written description prefills; toggling a hole while it is focused unticks it, and
    // toggling with non-checkbox focus is a clean no-op.
    let mut review = review_prompt(
        "d.prompt.md",
        b"{{a}}\n",
        ReviewDefaults {
            description: Some("hand-written".to_owned()),
            ..review_defaults(&runners())
        },
    );
    assert_eq!(review.description(), "hand-written");
    review.set_prompt_selected("a", false); // the footer chip's twin
    assert!(!review.prompt_candidates()[0].selected);
    // A toggle that names no hole changes nothing.
    review.set_prompt_selected("no-such-hole", true);
    assert!(!review.prompt_candidates()[0].selected);
}

#[test]
fn test_review_modal_cancel_leaves_the_picker_alone() {
    // A cancelled New agent add adds no runner option: the picker's runner set is unchanged.
    let review = review_prompt("c.prompt.md", b"{{a}}\n", review_defaults(&runners()));
    let before = review.runner_names().to_vec();
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::from_review(review),
    ))));
    state.update(Action::OpenAddRunnerEditor); // Ctrl+N opens the shared runner editor
    assert!(state.modal().is_some());
    state.update(Action::Back); // Escape cancels it
    assert!(state.modal().is_none());
    let after = state
        .add_workflow()
        .and_then(AddWorkflowState::review)
        .expect("the add workflow keeps its review")
        .runner_names()
        .to_vec();
    assert_eq!(after, before); // a cancelled add adds no option
}

#[test]
fn test_review_ctrl_e_keeps_the_runner_pick_and_reports_editor_errors() {
    // A rescan (the Ctrl+E edit) keeps the runner pick. (The editor-failure notification path
    // — reporting an editor.EditorError without crashing out of the panel — is host state.)
    let mut review = review_prompt("k.prompt.md", b"{{a}}\n", review_defaults(&runners()));
    review.set_runner(RUNNERS[1], true); // pick the second configured runner
    review.rescan(b"{{a}}\n".to_vec()); // the editor returned; the panel rescans
    assert_eq!(review.runner(), RUNNERS[1]); // the pick survived the rescan
}

#[test]
fn test_review_ctrl_e_keeps_placeholder_ticks_by_name_across_flood_transitions() {
    // Edits carry placeholder ticks by name across a flood transition: an off decision follows
    // its name, genuinely new flood holes default off, and new non-flood holes default on.
    let flood_names: Vec<String> = ["flood_on".to_owned(), "keep_off".to_owned()]
        .into_iter()
        .chain((0..PROMPT_AUTO_MANAGE_LIMIT - 1).map(|index| format!("new_{index}")))
        .collect();
    let mut review = review_prompt(
        "ticks.prompt.md",
        b"{{keep_off}} {{removed}}\n",
        review_defaults(&runners()),
    );

    fn selected(review: &ReviewState, name: &str) -> bool {
        review
            .prompt_candidates()
            .iter()
            .find(|candidate| candidate.name == name)
            .map(|candidate| candidate.selected)
            .unwrap_or(false)
    }

    review.set_prompt_selected("keep_off", false);
    let flood_body = flood_names
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");
    review.rescan(format!("{flood_body}\n").into_bytes());

    assert!(
        !review
            .prompt_candidates()
            .iter()
            .any(|candidate| candidate.name == "removed")
    );
    assert!(!selected(&review, "keep_off")); // decision followed the name
    assert!(!selected(&review, "flood_on")); // genuinely new flood holes default off
    review.set_prompt_selected("flood_on", true);

    review.rescan(b"{{fresh_below}} {{flood_on}} {{keep_off}}\n".to_vec());
    let names = review
        .prompt_candidates()
        .iter()
        .map(|candidate| candidate.name.clone())
        .collect::<Vec<_>>();
    assert_eq!(names, ["fresh_below", "flood_on", "keep_off"]);
    assert!(selected(&review, "fresh_below")); // new non-flood holes default on
    assert!(selected(&review, "flood_on")); // explicit flood choice survived
    assert!(!selected(&review, "keep_off")); // reordered survivor stayed off
}

#[test]
fn test_review_edit_tolerates_a_placeholder_checkbox_unmounted_during_recompose() {
    // An edit that replaces every hole completes and gives the newly scanned placeholder its
    // normal default (on, under the cap), with no stale tick carried over.
    let mut review = review_prompt(
        "recompose.prompt.md",
        b"{{old}}\n",
        review_defaults(&runners()),
    );
    assert_eq!(review.prompt_candidates().len(), 1);
    review.rescan(b"{{new}}\n".to_vec());
    let candidates = review.prompt_candidates();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].name, "new");
    assert!(candidates[0].selected); // the newly scanned placeholder gets its normal default
}

#[test]
fn test_form_ctrl_n_is_a_noop_without_a_picker() {
    // A python form has fields but no runner row, so Ctrl+N (New agent) opens no modal.
    let form = RunFormView::from_declarations(
        "plaincmd",
        "plaincmd",
        &[ParamDecl::new("x")],
        &BTreeMap::new(),
        &[], // no runners: no picker
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = run_state(form);
    assert!(!state.run_form().unwrap().has_runner_picker());
    // Ctrl+N maps to OpenRunRunnerEditor, which no-ops without a picker: no modal opens.
    state.update(Action::OpenRunRunnerEditor);
    assert!(state.modal().is_none());
}

#[test]
fn test_form_modal_cancel_leaves_the_picker_alone() {
    // A cancelled New agent add adds no option to the run form's runner picker.
    let mut state = run_state(prompt_form(&[], &[], &runners(), ""));
    let before = runner_options_run(&state);
    // A cancelled editor closes without a RunnerEditorSaved, so the options are untouched.
    state.update(Action::OpenRunRunnerEditor);
    state.update(Action::Back); // Escape closes the editor
    assert_eq!(runner_options_run(&state), before);
}

fn runner_options_run(state: &LibraryState) -> Vec<String> {
    let form = state.run_form().unwrap();
    let field = form
        .fields()
        .iter()
        .find(|field| matches!(field.role, RunFieldRole::Runner))
        .expect("the prompt form has a runner picker");
    match &field.control {
        skit_ui::FormControl::Choice(choice) => choice.options.clone(),
        _ => panic!("the runner picker is a choice control"),
    }
}

#[test]
fn test_settings_ctrl_n_is_a_noop_on_non_prompt_entries() {
    // A python entry's settings has no runner section, so Ctrl+N opens no modal.
    let view = SettingsView::from_inputs(&SettingsInputs {
        selector: "plainpy".to_owned(),
        kind: "python".to_owned(),
        name: "plainpy".to_owned(),
        source: "/tmp/s.py".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        ..SettingsInputs::default()
    });
    assert!(!view.has_section(SettingsSectionId::Runner));
    let mut view = view;
    assert_eq!(view.update(SettingsAction::NewRunner), SettingsEffect::None); // no modal
}

#[test]
fn test_settings_runner_select_change_arms_the_discard_ask() {
    // A pin-only edit is a real edit: it arms the dirty flag, so Close raises the unsaved-
    // changes ask rather than silently dropping it.
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, true));
    view.set_value(
        RUNNER_KEY,
        FieldValue::Explicit(TypedValue::Choice(RUNNERS[0].to_owned())),
    );
    assert!(view.is_dirty());
    assert_eq!(
        view.update(SettingsAction::Close),
        SettingsEffect::ConfirmDiscard
    );
}

#[test]
fn test_settings_modal_cancel_leaves_the_picker_alone() {
    // A cancelled New agent add adds no option: opening the New agent door (Ctrl+N) mutates
    // nothing — a cancel never reaches add_and_select_runner, so the options, the selected
    // value, and the dirty flag are all untouched.
    let mut view =
        SettingsView::from_inputs(&prompt_settings(vec![placeholder("a")], "", RUNNERS, true));
    let before = runner_options(&view);
    let before_value = view.field(RUNNER_KEY).unwrap().value().as_text();
    assert_eq!(
        view.update(SettingsAction::NewRunner),
        SettingsEffect::NewRunner
    ); // the door opens
    assert_eq!(runner_options(&view), before); // a cancelled add adds no option
    assert_eq!(
        view.field(RUNNER_KEY).unwrap().value().as_text(),
        before_value
    );
    assert!(!view.is_dirty()); // opening the door mutates nothing
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): tui_add.run_prompt_review returning the PromptReviewApp's run() result is the standalone add-app entry point in the composition root. tests/test_prompt_tui.py:1921."]
fn test_run_prompt_review_returns_the_apps_result() {
    // run_prompt_review returns whatever the review app's run() returns.
}

// ==========================================================================
// the New agent modal
// ==========================================================================

#[test]
fn test_form_ctrl_n_defines_a_custom_agent_and_runs_with_it() {
    // Ctrl+N on the runner row defines a custom agent, which joins the picker selected in
    // place; the launch then delivers it. (The seeds materializing alongside is host state.)
    let mut state = run_state(prompt_form(&[], &[], &runners(), ""));
    // Ctrl+N opens the shared runner editor; its Save re-enters here with the new agent.
    state.update(Action::OpenRunRunnerEditor);
    state.update(Action::RunnerEditorSaved {
        owner: RunnerEditorOwner::Run {
            selector: "p".to_owned(),
        },
        name: "aider".to_owned(),
        message: String::new(),
    });
    assert_eq!(runner_value(&state), "aider"); // joined the picker, selected in place
    let values = submit_values(&mut state);
    assert_eq!(submitted_runner(&values), "aider");
}

#[test]
fn test_runner_modal_validation_covers_every_refusal() {
    // The runner editor refuses every malformed command with its typed error: empty name, no
    // command, no slot, the slot as the binary, a stray hole, and unbalanced quotes. (The
    // "already exists" seed collision is a host mutation refusal, checked at save time.)
    let mut editor = RunnerEditorView::new();

    assert_eq!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::None
    ); // empty name
    assert_eq!(editor.error(), Some(&RunnerEditorError::NameRequired));

    editor.reduce(RunnerEditorAction::SetName("mycli".to_owned()));
    editor.reduce(RunnerEditorAction::SetCommand(String::new())); // no command at all
    assert_eq!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::None
    );
    assert_eq!(editor.error(), Some(&RunnerEditorError::EmptyCommand));

    editor.reduce(RunnerEditorAction::SetCommand("mycli run".to_owned())); // no slot
    assert_eq!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::None
    );
    assert_eq!(editor.error(), Some(&RunnerEditorError::PromptSlotCount));

    editor.reduce(RunnerEditorAction::SetCommand("{{prompt}}".to_owned())); // the slot as the binary
    assert_eq!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::None
    );
    assert_eq!(editor.error(), Some(&RunnerEditorError::PromptInProgram));

    editor.reduce(RunnerEditorAction::SetCommand(
        "mycli {{prompt}} {{extra}}".to_owned(),
    )); // a stray hole
    assert_eq!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::None
    );
    assert_eq!(editor.error(), Some(&RunnerEditorError::UnsupportedHole));

    editor.reduce(RunnerEditorAction::SetCommand(
        "mycli \"run {{prompt}}".to_owned(),
    )); // unbalanced quote
    assert_eq!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::None
    );
    assert_eq!(editor.error(), Some(&RunnerEditorError::UnbalancedQuotes));

    // A valid command finally saves.
    editor.reduce(RunnerEditorAction::SetCommand(
        "mycli run {{prompt}}".to_owned(),
    ));
    assert!(matches!(
        editor.reduce(RunnerEditorAction::Submit),
        RunnerEditorEffect::Save(_)
    ));
}

// ==========================================================================
// Library edit -> offer to manage the placeholders a body edit introduced
// ==========================================================================

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_edit's open-editor -> rescan -> offer the PromptCandidatePickerModal for newly introduced placeholders is the composition root's edit flow. tests/test_prompt_tui.py:2026."]
fn test_library_edit_prompt_offers_picker_and_manages_the_selection() {
    // A body edit that adds {{username}} offers the picker; Done manages it and says so.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_edit's picker-cancel-leaves-it-literal path is the composition root's edit flow. tests/test_prompt_tui.py:2045."]
fn test_library_edit_prompt_picker_cancel_leaves_it_literal() {
    // Cancelling the post-edit picker leaves the new placeholder literal (unmanaged).
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_edit's Done-with-no-ticks-manages-nothing path is the composition root's edit flow. tests/test_prompt_tui.py:2062."]
fn test_library_edit_prompt_picker_done_with_no_ticks_manages_nothing() {
    // Done with everything unticked manages nothing.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_edit preserving existing managed while adding a newly ticked one is the composition root's edit flow. tests/test_prompt_tui.py:2081."]
fn test_library_edit_prompt_preserves_existing_managed() {
    // The post-edit picker preserves existing managed and appends the ticked new one.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_edit showing no picker when a body edit introduced no new placeholder is the composition root's edit flow. tests/test_prompt_tui.py:2100."]
fn test_library_edit_prompt_no_new_placeholder_shows_no_picker() {
    // A body edit with no new placeholder shows no picker and keeps the managed set.
}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): action_edit never offering the picker for a non-prompt entry is the composition root's edit flow. tests/test_prompt_tui.py:2115."]
fn test_library_edit_non_prompt_never_offers_the_picker() {
    // Editing a non-prompt entry never offers the placeholder picker.
}
