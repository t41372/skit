//! Pointer rules for the Run screen and the shared top-level click registry.
//!
//! The typed-picker contract compares a complete rendered endpoint (state, geometry, buffer and
//! cursor) between the pointer path and the keyboard path. The footer-chip contract compares the
//! two typed actions, and the lone-press contract compares event handling. Every state is built
//! from `LibraryState` values, so no host is involved.
//!
//! The two whole-frame `check_public_hit_parity` sweeps of legacy `driver.rs:3220` and `:3411`
//! use this crate's `parity` module, like every other probe helper here.

use std::collections::BTreeMap;

use ratatui_core::layout::{Rect, Size};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterType, ParameterValue},
};
use skit_i18n::Locale;
use skit_tui::{EventHandling, HitTarget, ViewGeometry};
use skit_tui_walker_model::parity::{
    ProbeEndpoint, apply_probe_handling, binding_event, check_public_hit_parity,
    command_key_action, render_probe_endpoint, render_probe_session, session_primary_click,
};
use skit_ui::{
    Action, Effect, LibraryState, LibrarySurface, RunFieldRole, RunFormView, Screen, UiCommand,
};

// --------------------------------------------------------------------------
// helpers this file owns
// --------------------------------------------------------------------------

/// The rule that `session_primary_click` keeps for a published target.
const PRESS_ARMS: &str = "a primary press over a published target must arm without activating";

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn center(rect: Rect) -> (u16, u16) {
    (
        rect.x.saturating_add(rect.width / 2),
        rect.y.saturating_add(rect.height / 2),
    )
}

fn endpoint_text(endpoint: &ProbeEndpoint) -> String {
    endpoint
        .backend
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn hit_rect(geometry: &ViewGeometry, target: HitTarget) -> Rect {
    geometry
        .hits
        .iter()
        .find(|hit| hit.action == target)
        .unwrap_or_else(|| panic!("the rendered frame must publish {target:?}"))
        .rect
}

// --------------------------------------------------------------------------
// state fixtures
// --------------------------------------------------------------------------

/// The runner the typed-controls form leaves unselected. It is rendered only by an open dropdown.
const RUNNER_ALTERNATIVE: &str = "claude";

fn entry(slug: &str, name: &str, kind: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).unwrap(),
        mode: StorageMode::Copy,
        description: format!("{name} description"),
        target: None,
    }
}

fn library_state() -> LibraryState {
    LibraryState::from_library_surface(LibrarySurface {
        scan: LibraryScan {
            entries: vec![
                entry("python-tool", "Python tool", "python"),
                entry("shell-tool", "Shell tool", "shell"),
            ],
            diagnostics: Vec::new(),
        },
        details: BTreeMap::new(),
    })
}

fn state_with_form(form: RunFormView) -> LibraryState {
    let mut state = LibraryState::default();
    assert_eq!(
        state.update(Action::Present(Screen::Run(Box::new(form)))),
        Effect::None
    );
    state
}

fn typed_controls_state() -> LibraryState {
    let mut enabled = ParamDecl::new("ENABLED");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(false));
    let mut format = ParamDecl::new("FORMAT");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned(), "toml".to_owned()];
    format.default = Some(ParameterValue::String("yaml".to_owned()));
    state_with_form(RunFormView::from_declarations(
        "typed-controls",
        "Typed controls",
        &[enabled, format],
        &BTreeMap::new(),
        &["codex".to_owned(), "claude".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    ))
}

fn long_form_state() -> LibraryState {
    let declarations = (0..16)
        .map(|index| ParamDecl::new(format!("field-{index}")))
        .collect::<Vec<_>>();
    state_with_form(RunFormView::from_declarations(
        "long",
        "Long form",
        &declarations,
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    ))
}

/// A form that overflows a 120x29 viewport by exactly one row, so one wheel notch stops at row 1
/// and the first field stays partly visible.
fn short_form_state() -> LibraryState {
    let declarations = (0..4)
        .map(|index| ParamDecl::new(format!("field-{index}")))
        .collect::<Vec<_>>();
    state_with_form(RunFormView::from_declarations(
        "short",
        "Short form",
        &declarations,
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    ))
}

fn runner_field(state: &LibraryState) -> usize {
    state
        .run_form()
        .unwrap()
        .fields()
        .iter()
        .position(|field| matches!(field.role, RunFieldRole::Runner))
        .expect("the fixture form has a runner picker")
}

// --------------------------------------------------------------------------
// contracts
// --------------------------------------------------------------------------

/// Ledger row `driver.rs:2937`.
#[test]
fn a_lone_primary_press_arms_a_footer_chip_without_activating_it() {
    let state = library_state();
    let size = Size::new(80, 24);
    let (session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();
    let (column, row) = center(hit_rect(&geometry, HitTarget::Command(UiCommand::Help)));

    let mut armed = session.try_fork().unwrap();
    assert_eq!(
        armed.handle_event(
            mouse(MouseEventKind::Down(MouseButton::Left), column, row),
            &state,
            &geometry,
        ),
        EventHandling::Consumed,
        "a lone press must be consumed as one pointer event without an action"
    );
    let snapshot = serde_json::to_value(armed.agent_review_snapshot().unwrap()).unwrap();
    assert_ne!(
        snapshot["top_level_click"]["fields"]["pressed"],
        serde_json::Value::Null,
        "raw Down was discarded or incorrectly auto-released"
    );

    let mut completed = armed.try_fork().unwrap();
    assert_eq!(
        completed.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::OpenHelp),
        "the kept arm must complete on the matching release"
    );

    let mut cancelled = armed.try_fork().unwrap();
    assert_eq!(
        cancelled.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), u16::MAX, u16::MAX),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a release away from the chip must cancel the arm"
    );
    assert_eq!(
        cancelled.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            &state,
            &geometry,
        ),
        EventHandling::Ignored,
        "a cancelled arm must not accept a stale release"
    );
}

/// Ledger row `driver.rs:3142`.
#[test]
fn the_run_previous_focus_chip_reaches_its_advertised_key_endpoint() {
    let state = typed_controls_state();
    let size = Size::new(80, 24);
    let (mut mouse_session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();
    let key_session = mouse_session.try_fork().unwrap();
    let rect = hit_rect(&geometry, HitTarget::Command(UiCommand::FocusPrevious));
    let handling = session_primary_click(
        &mut mouse_session,
        &state,
        &geometry,
        rect.x.saturating_add(rect.width / 2),
        rect.y,
    )
    .expect(PRESS_ARMS);
    let EventHandling::Action(mouse_action) = handling else {
        panic!("the Run footer chip did not emit an action: {handling:?}");
    };
    let bindings = key_session.advertised_command_bindings(&state, UiCommand::FocusPrevious);
    assert!(
        !bindings.is_empty(),
        "the Run footer must advertise a key for FocusPrevious"
    );
    let key_action = command_key_action(
        &key_session,
        &state,
        &geometry,
        UiCommand::FocusPrevious,
        &bindings,
        &mouse_action,
        0,
    )
    .unwrap();
    assert_eq!(
        key_action, mouse_action,
        "the Run FocusPrevious chip and its key must reach one endpoint"
    );
}

/// Ledger row `driver.rs:3183`.
#[test]
fn typed_run_picker_pointer_rules_match_their_keyboard_endpoints() {
    let mut state = typed_controls_state();
    let size = Size::new(100, 28);
    let (session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();

    assert!(
        geometry
            .hits
            .iter()
            .any(|hit| matches!(hit.action, HitTarget::ToggleField(_))),
        "a typed checkbox must publish its toggle hit"
    );
    assert!(
        geometry.hits.iter().any(|hit| matches!(
            hit.action,
            HitTarget::SelectFieldOption {
                field: _,
                option: 1
            }
        )),
        "a typed radio control must publish a hit for each option"
    );
    check_public_hit_parity(&state, &geometry, size, &session, Locale::En).unwrap();

    // Rule 1: the first click on a non-current picker focuses it and opens the dropdown, so its
    // endpoint is strictly more than `FocusField`.
    let runner = runner_field(&state);
    let runner_rect = hit_rect(&geometry, HitTarget::FocusField(runner));
    let (runner_column, runner_row) = center(runner_rect);
    let mut noncurrent_session = session.try_fork().unwrap();
    let mut noncurrent_state = state.clone();
    let handling = session_primary_click(
        &mut noncurrent_session,
        &noncurrent_state,
        &geometry,
        runner_column,
        runner_row,
    )
    .expect(PRESS_ARMS);
    assert_eq!(handling, EventHandling::Action(Action::FocusField(runner)));
    apply_probe_handling(&mut noncurrent_state, handling, "non-current picker click").unwrap();
    let noncurrent_mouse =
        render_probe_endpoint(&mut noncurrent_session, &noncurrent_state, Locale::En, size)
            .unwrap();
    let mut focus_only_session = session.try_fork().unwrap();
    let mut focus_only_state = state.clone();
    assert_eq!(
        focus_only_state.update(Action::FocusField(runner)),
        Effect::None
    );
    let focus_only =
        render_probe_endpoint(&mut focus_only_session, &focus_only_state, Locale::En, size)
            .unwrap();
    assert!(
        !endpoint_text(&focus_only).contains(RUNNER_ALTERNATIVE),
        "a focused closed picker must show only its selected runner"
    );
    assert!(
        endpoint_text(&noncurrent_mouse).contains(RUNNER_ALTERNATIVE),
        "the first picker click must open the dropdown and list every runner"
    );
    assert_ne!(
        noncurrent_mouse, focus_only,
        "the first picker click must focus and open the dropdown"
    );

    assert_eq!(state.update(Action::FocusField(runner)), Effect::None);
    let (session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();
    let picker_rect = hit_rect(&geometry, HitTarget::FocusField(runner));
    let (picker_column, picker_row) = center(picker_rect);
    let mut closed_session = session.try_fork().unwrap();
    let closed = render_probe_endpoint(&mut closed_session, &state, Locale::En, size).unwrap();

    let mut mouse_session = session.try_fork().unwrap();
    assert_eq!(
        session_primary_click(
            &mut mouse_session,
            &state,
            &geometry,
            picker_column,
            picker_row,
        )
        .expect(PRESS_ARMS),
        EventHandling::Consumed
    );
    let open = render_probe_endpoint(&mut mouse_session, &state, Locale::En, size).unwrap();
    let mut key_session = session.try_fork().unwrap();
    assert_eq!(
        key_session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Consumed
    );
    let open_by_key = render_probe_endpoint(&mut key_session, &state, Locale::En, size).unwrap();
    assert_ne!(open, closed, "the picker click must open its dropdown");
    assert_eq!(
        open, open_by_key,
        "the advertised Enter key must open the same dropdown"
    );

    let open_rect = hit_rect(&open.geometry, HitTarget::FocusField(runner));
    let (anchor_column, anchor_row) = center(open_rect);

    // Rule 3: a press on the open anchor only arms it.
    let mut cancelled_session = mouse_session.try_fork().unwrap();
    assert_eq!(
        cancelled_session.handle_event(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                anchor_column,
                anchor_row,
            ),
            &state,
            &open.geometry,
        ),
        EventHandling::Consumed
    );
    let armed = render_probe_endpoint(&mut cancelled_session, &state, Locale::En, size).unwrap();
    assert_eq!(armed, open, "picker Down must only arm the open anchor");

    // Rule 4: a release away from the anchor cancels the arm and keeps the open endpoint.
    assert_eq!(
        cancelled_session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), u16::MAX, u16::MAX),
            &state,
            &armed.geometry,
        ),
        EventHandling::Ignored
    );
    let cancelled =
        render_probe_endpoint(&mut cancelled_session, &state, Locale::En, size).unwrap();
    assert_eq!(
        cancelled, open,
        "a release away from the picker changed its open endpoint"
    );

    // Rule 5: a stale release on the anchor after cancellation is inert.
    assert_eq!(
        cancelled_session.handle_event(
            mouse(
                MouseEventKind::Up(MouseButton::Left),
                anchor_column,
                anchor_row,
            ),
            &state,
            &cancelled.geometry,
        ),
        EventHandling::Ignored
    );
    let stale_release =
        render_probe_endpoint(&mut cancelled_session, &state, Locale::En, size).unwrap();
    assert_eq!(
        stale_release, open,
        "a cancelled picker press accepted a stale release"
    );

    // Rule 2: the second click closes the dropdown and renders the exact Escape endpoint.
    let mut close_mouse_session = mouse_session.try_fork().unwrap();
    let mut close_key_session = mouse_session.try_fork().unwrap();
    assert_eq!(
        session_primary_click(
            &mut close_mouse_session,
            &state,
            &open.geometry,
            anchor_column,
            anchor_row,
        )
        .expect(PRESS_ARMS),
        EventHandling::Consumed
    );
    assert_eq!(
        close_key_session.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            &state,
            &open.geometry,
        ),
        EventHandling::Consumed
    );
    let close_mouse =
        render_probe_endpoint(&mut close_mouse_session, &state, Locale::En, size).unwrap();
    let close_key =
        render_probe_endpoint(&mut close_key_session, &state, Locale::En, size).unwrap();
    assert_ne!(
        close_mouse, open,
        "the second picker click must close its dropdown"
    );
    assert_eq!(
        close_mouse, close_key,
        "Escape must close the same dropdown as the picker click"
    );
    assert_eq!(
        close_key, closed,
        "Escape must restore the exact closed picker endpoint"
    );
    check_public_hit_parity(&state, &geometry, size, &session, Locale::En).unwrap();
}

/// Ledger rows `driver.rs:3414` and `driver.rs:3487`.
///
/// The keyboard half scrolls the form to reach a later field. The pointer half uses a form whose
/// scroll range is one row, so the wheel clamps there and the focused first row stays partly
/// visible. A larger scroll range moves the wheel three rows and takes the first row out of view.
#[test]
fn a_scrolled_run_form_keeps_a_positive_hit_for_its_focused_row() {
    let mut state = long_form_state();
    let size = Size::new(46, 12);
    let (mut session, mut geometry) = render_probe_session(&state, Locale::En, size).unwrap();
    assert_eq!(
        geometry.first_visible, 0,
        "an unscrolled Run form starts at its first row"
    );

    let bindings = session.advertised_command_bindings(&state, UiCommand::FocusNext);
    assert!(
        !bindings.is_empty(),
        "the Run footer must advertise a key for FocusNext"
    );
    let mut steps = 0_usize;
    while geometry.first_visible == 0 {
        assert!(
            steps < 16,
            "the keyboard focus path never scrolled the form"
        );
        let handling =
            session.handle_event(Event::Key(binding_event(bindings[0])), &state, &geometry);
        apply_probe_handling(&mut state, handling, "keyboard focus move").unwrap();
        geometry = render_probe_endpoint(&mut session, &state, Locale::En, size)
            .unwrap()
            .geometry;
        steps += 1;
    }
    let focused = state
        .focused_form_field()
        .expect("a keyboard focus move keeps one focused Run field");
    let rect = hit_rect(&geometry, HitTarget::FocusField(focused));
    assert!(
        rect.width > 0 && rect.height > 0,
        "a scrolled Run form must keep a positive hit for its focused row: {rect:?}"
    );
    assert!(
        geometry.first_visible > 0,
        "the keyboard focus path must scroll the form"
    );

    // The pointer path: a footer chip moves focus to the first field, then one wheel notch moves
    // the shared viewport without moving focus, and the focused row keeps its hit.
    let mut wheel_state = short_form_state();
    assert_eq!(wheel_state.update(Action::FocusField(1)), Effect::None);
    let wheel_size = Size::new(120, 29);
    let (mut wheel_session, wheel_geometry) =
        render_probe_session(&wheel_state, Locale::En, wheel_size).unwrap();
    let previous = hit_rect(
        &wheel_geometry,
        HitTarget::Command(UiCommand::FocusPrevious),
    );
    let handling = session_primary_click(
        &mut wheel_session,
        &wheel_state,
        &wheel_geometry,
        previous.x.saturating_add(previous.width / 2),
        previous.y,
    )
    .expect(PRESS_ARMS);
    apply_probe_handling(&mut wheel_state, handling, "Run FocusPrevious chip").unwrap();
    assert_eq!(
        wheel_state.focused_form_field(),
        Some(0),
        "the FocusPrevious chip must move focus to the first Run field"
    );
    let wheel_geometry =
        render_probe_endpoint(&mut wheel_session, &wheel_state, Locale::En, wheel_size)
            .unwrap()
            .geometry;
    assert_eq!(
        wheel_geometry.first_visible, 0,
        "the focused first Run field keeps the viewport at its first row"
    );
    assert_eq!(
        wheel_session.handle_event(
            mouse(
                MouseEventKind::ScrollDown,
                wheel_geometry.rows.x,
                wheel_geometry.rows.y,
            ),
            &wheel_state,
            &wheel_geometry,
        ),
        EventHandling::Consumed,
        "the Run rows viewport must own the wheel"
    );
    let after = render_probe_endpoint(&mut wheel_session, &wheel_state, Locale::En, wheel_size)
        .unwrap()
        .geometry;
    assert_eq!(
        wheel_state.focused_form_field(),
        Some(0),
        "a wheel scroll must not move focus"
    );
    assert_eq!(
        after.first_visible, 1,
        "the wheel stops at the last scrollable row of this form"
    );
    let rect = hit_rect(&after, HitTarget::FocusField(0));
    assert!(
        rect.width > 0 && rect.height > 0,
        "a scrolled Run form must keep a positive hit for its focused row: {rect:?}"
    );
}

#[test]
fn intervening_input_cancels_the_previous_pointer_gesture() {
    for event in [
        Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
        Event::Paste("text".to_owned()),
        Event::FocusLost,
        Event::FocusGained,
    ] {
        let state = library_state();
        let size = Size::new(80, 24);
        let (mut session, geometry) = render_probe_session(&state, Locale::En, size).unwrap();
        let (column, row) = center(hit_rect(&geometry, HitTarget::Command(UiCommand::Help)));
        assert_eq!(
            session.handle_event(
                mouse(MouseEventKind::Down(MouseButton::Left), column, row),
                &state,
                &geometry
            ),
            EventHandling::Consumed
        );
        let _ = session.handle_event(event.clone(), &state, &geometry);
        let released = session.handle_event(
            mouse(MouseEventKind::Up(MouseButton::Left), column, row),
            &state,
            &geometry,
        );
        assert_ne!(
            released,
            EventHandling::Action(Action::OpenHelp),
            "{event:?}"
        );
    }
}
