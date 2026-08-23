//! Mechanical port of the Python oracle module `tests/test_reset_default_ui.py`
//! (`origin/main@206f9ef`): "The ↺ reset-to-default affordance and the input-binding hint, across
//! every surface." A remembered last-used value overlays the script's own default in the run
//! form's prefill, which makes that default invisible; each defaulted, non-secret field grows a
//! `↺ default` chip and the screen answers Ctrl+O by restoring `field.default` into the live
//! control. Each `#[test]` keeps its Python `def test_*` name and its WHY comment so it traces back
//! to its origin.
//!
//! WHY `skit-tui` (crate choice, and the discovery that fixed it): the module's headline is the
//! OBSERVABLE affordance — the advertised Ctrl+O key, the ↺ chip's mouse click, the rendered chip,
//! and the footer pill. Those are render / input-event contracts. `skit-cli` (the composition
//! root) looks like it should reach every tier because it depends on `skit-tui`, but a depended-on
//! crate exposes its public TYPES, not its DEPENDENCIES: `Terminal`/`TestBackend`/`Frame`
//! (`ratatui_core`) and `Event`/`KeyEvent` (`ratatui_crossterm`) are separate crates that
//! `skit-cli` does not declare, and `skit-tui`'s `use` of them is private, so a `skit-cli`
//! integration test can drive the reducer but can never construct a key event or render a pixel
//! (`crates/skit-cli/tests/port_test_uv_metadata_unpinning.rs` documents this exact boundary). So
//! the interaction surface is only testable here.
//!
//! Concept mapping used throughout:
//! - Python `flows.FormField(key, default, has_default, kind, choices, secret, input_binding)` ->
//!   a `ParamDecl` fed to `RunFormView::from_declarations(...)`; the Python `prefill` dict is the
//!   `saved` map (a non-secret field's saved value overlays its default).
//! - Python `screen.action_reset_field(key)` (the ↺ chip's @click) -> `Action::ResetRunField(index)`
//!   (the mouse click routes `HitTarget::RunFieldCommand { command: ResetDefault }` to it).
//! - Python Ctrl+O (the footer key) -> the session maps `Ctrl+O` to `Action::ResetFocusedRunField`.
//! - Python `FieldRow.resettable` -> `RunField::resettable()`.
//! - Python "Ctrl+O in the footer" -> `LibraryState::command_enabled(UiCommand::ResetDefault)`
//!   (footer.rs filters chips on `command_enabled`, so a rendered "Reset to default"/"Ctrl+O" pill
//!   is exactly this gate).
//! - Python `input()`/read-bound field's help line -> `run_field_notes` emits the input-binding
//!   hint when `RunField::input_binding`; observed in the rendered run form.
//!
//! Buckets:
//! - REAL asserting `#[test]` (1-10): the reset affordance and the input-binding hint, driven
//!   through the reducer, the session key/mouse mapping, and the `TestBackend` render.
//! - CROSS-CRATE stubs (11-13): the plain line form and the `skit params` / `skit show --json`
//!   read views are `skit-cli` binary/private surfaces, unreachable from `skit-tui`. 12 and 13 are
//!   cheap REAL ports skit-cli-side (`port_test_show.rs`'s `Lib` harness) and are EXPECTED TO PASS;
//!   11 is also a headline DIVERGENCE FINDING (the Rust plain form drops all three pre-prompt
//!   hints).
//! - HOST-OWNED CONTRACT (14): the entry-settings parameter row needs both the real source
//!   projection and the rendered Settings screen. Its canonical exact owner is the live-host test
//!   in `skit-cli/tests/terminal_pty.rs`; lower-tier formatter coverage stays in `skit-form`.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterType, ParameterValue};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, TuiSession, ViewGeometry, render_with_session};
use skit_ui::{Action, LibraryState, RunFormView, Screen, UiCommand};

/// The exact English hint the intercepted-input field owes (i18n key kept verbatim).
const INPUT_BINDING_HINT: &str = "Leave empty and the script will ask you in the terminal.";

// --- ParamDecl builders (the oracle's `flows.FormField(...)` shapes) -----------------------------

/// A plain `str` field carrying a default (the oracle's `default=..., has_default=True`).
fn str_param(name: &str, default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

/// A `bool` field with a boolean default (`kind="bool"`).
fn bool_param(name: &str, default: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.parameter_type = ParameterType::Bool;
    declaration.default = Some(ParameterValue::Bool(default));
    declaration
}

/// A radio `choice` field (`kind="choice"`) whose default may or may not be one of its choices.
fn choice_param(name: &str, choices: &[&str], default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.parameter_type = ParameterType::Choice;
    declaration.choices = choices.iter().map(|choice| (*choice).to_owned()).collect();
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

/// A field with no default (nothing to restore).
fn plain_param(name: &str) -> ParamDecl {
    ParamDecl::new(name)
}

/// A secret field carrying a default that must never be echoed into the form.
fn secret_param(name: &str, default: &str) -> ParamDecl {
    let mut declaration = str_param(name, default);
    declaration.secret = true;
    declaration
}

/// A field bound to an intercepted `input()`/read prompt (`input_binding=True`).
fn input_param(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Input;
    declaration
}

// --- Reducer / render harness (mirrors interactive_run_form.rs) ----------------------------------

fn run_view(
    declarations: &[ParamDecl],
    saved: &[(&str, &str)],
    runners: &[&str],
    extra: &str,
) -> RunFormView {
    let saved = saved
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let runners = runners
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<Vec<_>>();
    RunFormView::from_declarations(
        "demo",
        "Demo",
        declarations,
        &saved,
        &runners,
        "",
        &BTreeMap::new(),
        extra,
    )
}

fn present(form: RunFormView) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

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

fn buffer_text(buffer: &Buffer) -> String {
    buffer
        .content()
        .chunks(usize::from(buffer.area.width))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn mouse(column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

/// The stable value the launch delivers for field `index` — the text, the checkbox's
/// `true`/`false`, or the choice's selected option.
fn field_value(state: &LibraryState, index: usize) -> String {
    state.run_form().unwrap().fields()[index].control.value()
}

/// Report whether the rendered ↺ default chip (a mouse click target) rides field `index`.
fn has_reset_chip(geometry: &ViewGeometry, field: usize) -> bool {
    geometry.hits.iter().any(|hit| {
        hit.action
            == HitTarget::RunFieldCommand {
                field,
                command: UiCommand::ResetDefault,
            }
    })
}

// ---------------------------------------------------------------------------
// 1. Ctrl+O from a focused field restores the default over the remembered value
// ---------------------------------------------------------------------------

#[test]
fn test_ctrl_o_from_focused_field_restores_default_over_remembered_value() {
    // A str field whose default is "hello" is prefilled with the DIFFERENT last-used "world";
    // focusing the input and pressing the advertised Ctrl+O restores "hello" — the positive test
    // the footer's chip owes.
    let mut state = present(run_view(
        &[str_param("greeting", "hello")],
        &[("greeting", "world")],
        &[],
        "",
    ));
    let mut session = TuiSession::default();
    state.update(Action::FocusField(0));
    let (_, geometry) = draw(&mut session, &state, 100, 40);
    assert_eq!(field_value(&state, 0), "world"); // the remembered value overlays the default
    // Ctrl+O is the advertised key; the session must map it to the reset action.
    assert_eq!(
        session.handle_event(
            key(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &state,
            &geometry
        ),
        EventHandling::Action(Action::ResetFocusedRunField)
    );
    state.update(Action::ResetFocusedRunField);
    assert_eq!(field_value(&state, 0), "hello"); // Ctrl+O restored the script's own default
}

// ---------------------------------------------------------------------------
// 2. The ↺ chip's action restores a text, a checkbox, and a choice default
// ---------------------------------------------------------------------------

#[test]
fn test_reset_field_by_key_restores_text_bool_and_choice_defaults() {
    // Action::ResetRunField(index) — exactly what the ↺ chip's click fires — restores the default
    // of a field across all three control kinds: a text input, a checkbox, and a radio choice. Each
    // is prefilled with a value that differs from its default so the restore is observable.
    let declarations = [
        str_param("greeting", "hello"),
        bool_param("flag", false),
        choice_param("mode", &["a", "b"], "a"),
    ];
    let mut state = present(run_view(
        &declarations,
        &[("greeting", "world"), ("flag", "true"), ("mode", "b")],
        &[],
        "",
    ));
    // Prefills overlay every default first.
    assert_eq!(field_value(&state, 0), "world");
    assert_eq!(field_value(&state, 1), "true");
    assert_eq!(field_value(&state, 2), "b");
    state.update(Action::ResetRunField(0));
    state.update(Action::ResetRunField(1));
    state.update(Action::ResetRunField(2));
    assert_eq!(field_value(&state, 0), "hello"); // text default restored
    assert_eq!(field_value(&state, 1), "false"); // checkbox back to its default off-state
    assert_eq!(field_value(&state, 2), "a"); // default option "a" reselected, "b" released
}

// ---------------------------------------------------------------------------
// 3. The ↺ chip is a mouse click target, per the footer grammar
// ---------------------------------------------------------------------------

#[test]
fn test_reset_chip_mouse_click_restores_the_default() {
    // The visible ↺ chip IS the click target: clicking it on the field row restores the default, no
    // keyboard involved — the mouse-only path the design guarantees for every action.
    let mut state = present(run_view(
        &[str_param("greeting", "hello")],
        &[("greeting", "world")],
        &[],
        "",
    ));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 120, 40);
    let hit = geometry
        .hits
        .iter()
        .find(|hit| {
            hit.action
                == HitTarget::RunFieldCommand {
                    field: 0,
                    command: UiCommand::ResetDefault,
                }
        })
        .expect("the ↺ default chip is a click target");
    // Clicking the chip resets THIS field: were its click wired to any other key, greeting's input
    // would stay "world" and this would fail — so the click pins the field-keyed routing end to end.
    assert_eq!(
        session.handle_event(mouse(hit.rect.x, hit.rect.y), &state, &geometry),
        EventHandling::Action(Action::ResetRunField(0))
    );
    state.update(Action::ResetRunField(0));
    assert_eq!(field_value(&state, 0), "hello");
}

// ---------------------------------------------------------------------------
// 4. The ↺ chip rides a defaulted non-secret field, and nothing else
// ---------------------------------------------------------------------------

#[test]
fn test_reset_chip_present_for_default_absent_for_secret_and_no_default() {
    // The ↺ default chip rides a defaulted, non-secret field, and stays off (a) a secret field
    // (its default is never echoed into the form) and (b) a field with no default (nothing to
    // restore). RunField::resettable agrees with the rendered affordance.
    let declarations = [
        str_param("withdef", "hi"),
        secret_param("sekret", "s"),
        plain_param("nodef"),
    ];
    let state = present(run_view(&declarations, &[], &[], ""));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state, 120, 44);
    // The rendered ↺ chip (a click target) is present only on the defaulted, non-secret field.
    assert!(has_reset_chip(&geometry, 0)); // withdef → chip present
    assert!(!has_reset_chip(&geometry, 1)); // sekret → secret default is never restorable
    assert!(!has_reset_chip(&geometry, 2)); // nodef → nothing to restore
    // The FieldRow.resettable property agrees with the rendered affordance.
    let form = state.run_form().unwrap();
    assert!(form.fields()[0].resettable());
    assert!(!form.fields()[1].resettable());
    assert!(!form.fields()[2].resettable());
}

// ---------------------------------------------------------------------------
// 5. A choice default outside its choices gets no chip and no Ctrl+O
// ---------------------------------------------------------------------------

#[test]
fn test_choice_default_outside_its_choices_gets_no_chip_and_no_ctrl_o() {
    // A script may declare a choice default that is not one of its own choices. There is no radio
    // button for it, so a reset would press nothing — the chip would be a button that visibly does
    // nothing, which is worse than no chip. The off-menu field is not resettable, carries no ↺
    // chip, and (as the only field) keeps Ctrl+O out of the footer; the sane twin keeps both.
    let off = present(run_view(
        &[choice_param("env", &["dev", "prod"], "staging")],
        &[],
        &[],
        "",
    ));
    let mut session = TuiSession::default();
    let (terminal, geometry) = draw(&mut session, &off, 120, 44);
    assert!(!off.run_form().unwrap().fields()[0].resettable());
    assert!(!has_reset_chip(&geometry, 0));
    assert!(!off.command_enabled(UiCommand::ResetDefault)); // no resettable field → no dead key taught
    let text = buffer_text(terminal.backend().buffer());
    assert!(!text.contains("Ctrl+O"));
    assert!(!text.contains("Reset to default"));

    let mut on = present(run_view(
        &[choice_param("env", &["dev", "prod"], "dev")],
        &[("env", "prod")],
        &[],
        "",
    ));
    let (terminal, geometry) = draw(&mut session, &on, 120, 44);
    assert!(on.run_form().unwrap().fields()[0].resettable());
    assert!(has_reset_chip(&geometry, 0));
    assert!(on.command_enabled(UiCommand::ResetDefault));
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("Ctrl+O") && text.contains("Reset to default"));
    assert_eq!(field_value(&on, 0), "prod"); // prefill overlays
    on.update(Action::ResetRunField(0));
    assert_eq!(field_value(&on, 0), "dev"); // "dev" restored
}

// ---------------------------------------------------------------------------
// 6. The footer advertises Ctrl+O only when some field is resettable
// ---------------------------------------------------------------------------

#[test]
fn test_footer_advertises_ctrl_o_only_when_some_field_is_resettable() {
    // The footer teaches Ctrl+O exactly when a field can act on it — a chip that refused to do
    // anything would teach a dead key. A plan with a defaulted field shows the pill; a plan whose
    // only fields are a secret-with-default and a no-default field does not.
    let mut session = TuiSession::default();

    let resettable = present(run_view(&[str_param("g", "h")], &[], &[], ""));
    assert!(resettable.command_enabled(UiCommand::ResetDefault));
    let (terminal, _) = draw(&mut session, &resettable, 120, 44);
    let text = buffer_text(terminal.backend().buffer());
    assert!(text.contains("Ctrl+O"));
    assert!(text.contains("Reset to default"));

    let none = present(run_view(
        &[secret_param("s", "x"), plain_param("p")],
        &[],
        &[],
        "",
    ));
    assert!(!none.command_enabled(UiCommand::ResetDefault));
    let (terminal, _) = draw(&mut session, &none, 120, 44);
    let text = buffer_text(terminal.backend().buffer());
    assert!(!text.contains("Ctrl+O"));
    assert!(!text.contains("Reset to default"));
}

// ---------------------------------------------------------------------------
// 7. Ctrl+O with no default (and the guard branches) is a safe no-op
// ---------------------------------------------------------------------------

#[test]
fn test_ctrl_o_on_field_without_default_leaves_value_unchanged() {
    // Ctrl+O from a field with no default does nothing (the row is not resettable) — the typed value
    // survives, and nothing crashes. A reset naming a field index that is not on the form is likewise
    // a safe no-op (reset_field's get_mut guard). Rust's focus is always a valid field index, so the
    // Python "no focused control" branch has no analog; the same guard is reset_field's resettable
    // and bounds check.
    let mut state = present(run_view(
        &[plain_param("plain")],
        &[("plain", "typed")],
        &[],
        "",
    ));
    state.update(Action::FocusField(0));
    state.update(Action::ResetFocusedRunField); // no default → the field is left exactly as typed
    assert_eq!(field_value(&state, 0), "typed");
    state.update(Action::ResetRunField(99)); // a field index that is not on the form: no row, no crash
    assert_eq!(field_value(&state, 0), "typed");
}

// ---------------------------------------------------------------------------
// 8. Ctrl+O with focus outside any parameter row is a no-op
// ---------------------------------------------------------------------------

#[test]
fn test_ctrl_o_with_focus_outside_any_field_row_is_a_no_op() {
    // Focus can legitimately sit on a control with no resettable default — the runner picker row.
    // The chord must return quietly there. (Python focuses #runner-select, which has no FieldRow
    // ancestor; the Rust analog is the runner field at index 0, whose default is None so
    // reset_field no-ops via resettable(). Same observable contract.)
    let mut state = present(run_view(
        &[str_param("greeting", "hello")],
        &[("greeting", "world")],
        &["claude"],
        "",
    ));
    // With a runner picker, field 0 is the runner and greeting is field 1.
    state.update(Action::FocusField(0)); // focus the runner picker, not a parameter row
    state.update(Action::ResetFocusedRunField); // an exception here would fail the test
    assert_eq!(field_value(&state, 1), "world"); // greeting untouched
}

// ---------------------------------------------------------------------------
// 9. The input-binding hint renders in the field, and only there
// ---------------------------------------------------------------------------

#[test]
fn test_input_binding_field_renders_the_ask_in_terminal_hint() {
    // A field bound to an intercepted input()/read prompt shows the "leave empty and the script
    // will ask you" help line — without it the intercept's semantics are invisible.
    let state = present(run_view(&[input_param("q")], &[], &[], ""));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 120, 30);
    assert!(buffer_text(terminal.backend().buffer()).contains(INPUT_BINDING_HINT));
}

#[test]
fn test_plain_const_field_renders_no_input_binding_hint() {
    // A plain const field (no input binding, no help) shows NO help line at all — the input-binding
    // hint is specific to the intercepted-input case, exactly as the oracle's single mounted FieldRow
    // reports zero `.field-help` widgets (query('.field-help') == []).
    //
    // render-model: skit-tui exposes no per-field help region to a test — `run_field_notes`
    // (session.rs:2204-2277) is private and `ViewGeometry` carries only `hits`, not the note lines.
    // Only the rendered buffer is reachable, so this mounts the plain const as the form's ONLY field
    // and asserts the buffer carries none of the pre-prompt hint text. A plain const drives
    // `run_field_notes` to an empty vector (no `help`, not `degraded`, no `input_binding`, no
    // `environment_source`, no feedback), so NONE of the three static "Leave empty …" hints
    // (input-binding, degraded, environment; session.rs:2212-2237) may render — strictly stronger
    // than the single INPUT_BINDING_HINT string, which a spurious OTHER help line would slip past.
    let state = present(run_view(&[plain_param("c")], &[], &[], ""));
    let mut session = TuiSession::default();
    let (terminal, _) = draw(&mut session, &state, 120, 30);
    let text = buffer_text(terminal.backend().buffer());
    assert!(!text.contains(INPUT_BINDING_HINT), "{text}");
    // "Leave empty" is the shared stem of every static run_field_notes hint, so its total absence
    // proves the const row grew no help line at all, not merely that the input-binding one is gone.
    assert!(!text.contains("Leave empty"), "{text}");
}

// ---------------------------------------------------------------------------
// 7 (oracle). CLI: params shows the source's live default; show carries delivers_empty
// ---------------------------------------------------------------------------

#[test]
#[ignore = "CROSS-CRATE (skit-cli): `skit params` is the composition root's private read view, driven only through the binary (assert_cmd), which skit-tui cannot spawn. This is a cheap REAL port skit-cli-side (crates/skit-cli/tests/port_test_show.rs's Lib harness plus its drift fixture) and is EXPECTED TO PASS, not a suspected gap: form_plan -> managed_form_plan -> refresh_default reconciles the live literal, so the Default column shows the source's live 'bonjour' and never the stale block cache 'hello'. Oracle: test_reset_default_ui.py:412-420."]
fn test_params_default_column_shows_the_sources_live_value() {}

#[test]
#[ignore = "CROSS-CRATE (skit-cli): `skit show --json` is driven only through the binary (assert_cmd), unreachable from skit-tui. Cheap REAL port skit-cli-side, EXPECTED TO PASS: PreparedField::delivers_empty (crates/skit-form/src/lib.rs:104-119) is true for a defaulted str const (Str + Inject) and false for an int const (not Str|Path), and the live default rides the JSON as 'bonjour' via the managed_form_plan reconcile. Oracle: test_reset_default_ui.py:423-432."]
fn test_show_json_delivers_empty_true_for_str_const_false_for_int() {}
