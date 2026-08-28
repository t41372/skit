//! Mechanical port of the Python oracle module `tests/test_tui_nav.py`
//! (`origin/main@206f9ef`): "Keyboard navigation on the form screens (zero memorization: the
//! footer says how to move, and arrows work wherever Tab works)." Each `#[test]` keeps its Python
//! `def test_*` name and its "WHY" rationale, and drives the real public API.
//!
//! The oracle drives a live Textual `MenuApp` through a `Pilot`, presses keys, clicks footer
//! chips, and reads `app.focused` (the widget that owns the keyboard). The Rust frontend has no
//! live App/pilot/widget-focus tree: it renders a serializable state through `render_with_session`
//! and maps Crossterm events to `Action`s through `TuiSession`. The CONTRACT is the same — the
//! first control boots focused, `↓`/`↑` are `Tab`/`Shift+Tab`'s arrow twins wherever a field does
//! not claim them, and every footer nav pill is the same clickable action — only the observation
//! mechanism differs.
//!
//! Concept mapping used throughout:
//! - Python `pilot.press("down"|"up"|"tab"|"shift+tab")` -> `TuiSession::handle_event(Event::Key)`
//!   / `AddScreenSession::handle_event`, applying any returned `Action` through the reducer.
//! - Python `app.focused.id` -> the per-screen focus observable: `LibraryState::focused_form_field`
//!   (run form), `SettingsView::focused` / `PreferencesView::focused` via `state.screen()`, and
//!   `AddScreenSession::focused` (add/review, whose focus is session-internal and never reaches
//!   `LibraryState`, so the add screens are driven one tier down, the same session `TuiSession`
//!   owns — the idiom `port_test_add_review_contracts.rs` already uses).
//! - Python `click_label(pilot, "#…-keys", "Tab/↓")` -> a click at the `HitTarget::Command`
//!   rect in the freshly rendered `ViewGeometry` (`register_geometry` rebuilds the click map each
//!   render, so a click needs a fresh draw first).
//! - Python's `tmp_store` autouse env fixture is unnecessary: the views are constructed directly,
//!   no `skit` binary is spawned and no store/filesystem is touched.
//!
//! Buckets:
//! - REAL (5): the run form, add-source, add-review, Preferences, and Settings tests drive boot focus, the `↓`/`↑`
//!   arrow twins, both footer direction chips, and `Tab`/`Shift+Tab` exactly as the oracle walks.
//!   Add Source keeps its visible Browse button outside the field-navigation ring and gives it a
//!   separate advertised keyboard path, so the oracle's path-to-template walk loses no capability.
//! - DIVERGENCE: none.
//! - CROSS-CRATE / ABSENT: none.

use std::collections::BTreeMap;

use ratatui_core::{backend::TestBackend, buffer::Buffer, layout::Rect, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{SourcePermissions, preferences::InteractiveFormChoice};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_i18n::Locale;
use skit_tui::{
    AddControlId, AddScreenEvent, AddScreenGeometry, AddScreenSession, AddTextField, EventHandling,
    HitTarget, TuiSession, ViewGeometry, render_add, render_with_session,
};
use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, KnownEntryKind, LibraryState,
    PreferencesAction, PreferencesControlId, ReviewDefaults, ReviewState, RunFormView, Screen,
    SettingsInputs, SettingsView, SourceSnapshot, UiCommand,
};

// The oracle drives every form at size (130, 40); the render size is load-bearing for the footer
// chips, so it is kept exactly.
const WIDTH: u16 = 130;
const HEIGHT: u16 = 40;

// --- shared event / observation helpers (self-contained; no shared-file edits) ---

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// `Shift+Tab` as the oracle presses it (`pilot.press("shift+tab")`).
fn shift_back_tab() -> Event {
    Event::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT))
}

fn control_key(character: char) -> Event {
    Event::Key(KeyEvent::new(
        KeyCode::Char(character),
        KeyModifiers::CONTROL,
    ))
}

fn left_click(column: u16, row: u16) -> Event {
    mouse(MouseEventKind::Down(MouseButton::Left), column, row)
}

fn left_release(column: u16, row: u16) -> Event {
    mouse(MouseEventKind::Up(MouseButton::Left), column, row)
}

fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

fn buffer_text(buffer: &Buffer) -> String {
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Render one state through the composition root and return the buffer text plus the click map.
fn draw(session: &mut TuiSession, state: &LibraryState) -> (String, ViewGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    let mut geometry = ViewGeometry::default();
    terminal
        .draw(|frame| geometry = render_with_session(frame, state, Locale::En, session))
        .unwrap();
    (buffer_text(terminal.backend().buffer()), geometry)
}

/// Send one event through the root, applying any returned `Action` back into the state — the frame
/// loop the host runs.
fn drive(
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

fn drive_click(
    session: &mut TuiSession,
    state: &mut LibraryState,
    geometry: &ViewGeometry,
    column: u16,
    row: u16,
) -> EventHandling {
    assert_eq!(
        drive(session, state, geometry, left_click(column, row)),
        EventHandling::Consumed
    );
    drive(session, state, geometry, left_release(column, row))
}

fn add_click(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
    geometry: &AddScreenGeometry,
    column: u16,
    row: u16,
) -> Option<AddScreenEvent> {
    assert_eq!(
        session.handle_event(left_click(column, row), state, geometry),
        Some(AddScreenEvent::Changed)
    );
    session.handle_event(left_release(column, row), state, geometry)
}

/// Press one key through the root and apply its action.
fn press(session: &mut TuiSession, state: &mut LibraryState, code: KeyCode) {
    let (_, geometry) = draw(session, state);
    drive(session, state, &geometry, key(code));
}

/// Click the footer chip that fires `command`, the same action its key twin fires. A fresh draw
/// rebuilds the click map first (see `register_geometry`).
fn click_chip(session: &mut TuiSession, state: &mut LibraryState, command: UiCommand) {
    let (_, geometry) = draw(session, state);
    let rect = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(command))
        .unwrap_or_else(|| panic!("no footer chip fires {command:?}"))
        .rect;
    let _ = drive_click(session, state, &geometry, rect.x, rect.y);
}

// --- run-form fixture: the oracle's two-const-str entry ---

/// One `const str` parameter, the oracle's `ParamDecl(name, binding="const", type="str",
/// default=…)`.
fn const_str(name: &str, default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

/// The oracle's `_two_field_entry` presented as a run form: fields CITY, NAME (both typeable
/// inputs). Rust appends a trailing "Extra arguments" field the walk never reaches.
fn run_form_two() -> RunFormView {
    RunFormView::from_declarations(
        "two",
        "Two",
        &[const_str("CITY", "x"), const_str("NAME", "y")],
        &BTreeMap::from([
            ("CITY".to_owned(), "x".to_owned()),
            ("NAME".to_owned(), "y".to_owned()),
        ]),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
}

fn present(screen: Screen) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(screen));
    state
}

// ==========================================================================
// 1. Run form — REAL: boots typeable, arrows walk the fields, chips and keys agree
// ==========================================================================

#[test]
fn test_run_form_boots_typeable_and_arrows_walk_the_fields() {
    // The run form boots with its FIRST field focused and ready to type (never the body scroll
    // container); ↓/↑ are Tab/Shift+Tab's arrow twins; the footer's "Tab/↓" and "Shift+Tab/↑" pills
    // are the same clickable action; and Tab/Shift+Tab themselves walk the same way.
    let mut state = present(Screen::Run(Box::new(run_form_two())));
    let mut session = TuiSession::default();

    // Boot: the first field owns the keyboard.
    let (screen, geometry) = draw(&mut session, &state);
    assert_eq!(state.focused_form_field(), Some(0));
    // Both directions are advertised as key-only pills on the form footer.
    assert!(screen.contains("Tab/↓"), "{screen}");
    assert!(screen.contains("Shift+Tab/↑"), "{screen}");
    // ...and the first field is a typeable Input: a character edits field 0 (does not walk).
    assert!(
        matches!(
            session.handle_event(key(KeyCode::Char('z')), &state, &geometry),
            EventHandling::Action(Action::SetFieldValue { field: 0, .. })
        ),
        "the first field must be a typeable input"
    );

    // ↓ moves on, ↑ comes back.
    press(&mut session, &mut state, KeyCode::Down);
    assert_eq!(state.focused_form_field(), Some(1));
    press(&mut session, &mut state, KeyCode::Up);
    assert_eq!(state.focused_form_field(), Some(0));

    // The chip is the same action, clickable — forward then back.
    click_chip(&mut session, &mut state, UiCommand::FocusNext);
    assert_eq!(state.focused_form_field(), Some(1));
    click_chip(&mut session, &mut state, UiCommand::FocusPrevious);
    assert_eq!(state.focused_form_field(), Some(0));

    // The advertised keys themselves, not just the chips.
    press(&mut session, &mut state, KeyCode::Tab);
    assert_eq!(state.focused_form_field(), Some(1));
    let (_, geometry) = draw(&mut session, &state);
    drive(&mut session, &mut state, &geometry, shift_back_tab());
    assert_eq!(state.focused_form_field(), Some(0));
}

// ==========================================================================
// 2. Add source — REAL: field navigation skips the independently reachable Browse button
// ==========================================================================

/// The add screen's focus lives in the session, not in `LibraryState`, so the add surfaces are
/// driven one tier down through the re-exported `AddScreenSession`/`render_add` — the same session
/// `TuiSession` owns and delegates to. Returns its geometry for the boot render + chip lookups.
fn draw_add(
    session: &mut AddScreenSession,
    state: &AddWorkflowState,
) -> (Buffer, AddScreenGeometry) {
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    let mut geometry = AddScreenGeometry::default();
    terminal
        .draw(|frame| geometry = render_add(frame, frame.area(), state, session, Locale::En))
        .unwrap();
    (terminal.backend().buffer().clone(), geometry)
}

fn region_text(buffer: &Buffer, area: Rect) -> String {
    let mut text = String::new();
    for row in area.y..area.bottom().min(buffer.area.height) {
        for column in area.x..area.right().min(buffer.area.width) {
            text.push_str(buffer[(column, row)].symbol());
        }
    }
    text
}

const ADD_PATH: AddControlId = AddControlId::Text(AddTextField::SourcePath);
const ADD_TEMPLATE: AddControlId = AddControlId::Text(AddTextField::CommandTemplate);

#[test]
fn test_add_source_arrows_walk_path_template_name() {
    // The add source screen boots with the path box focused (not the body scroll container); ↓/↑
    // walk path<->template, the footer chips do the same, and Tab/Shift+Tab agree. The visible
    // Browse button stays outside that field walk and has its own advertised key and mouse path.
    let state = AddWorkflowState::new(Vec::new());
    let mut session = AddScreenSession::default();
    let (buffer, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_PATH)); // add-path, not the body scroll
    let screen = buffer_text(&buffer);
    assert!(screen.contains("Tab/↓"), "{screen}");
    assert!(screen.contains("Shift+Tab/↑"), "{screen}");
    assert!(screen.contains("[Ctrl+O] Select"), "{screen}");

    // The independent key and the visible button both open the same typed picker contract without
    // moving the form keyboard away from the path field. Hover and release are inert.
    assert!(matches!(
        session.handle_event(control_key('o'), &state, &geometry),
        Some(AddScreenEvent::OpenPathPicker(_))
    ));
    assert_eq!(session.focused(), Some(&ADD_PATH));
    let browse = geometry
        .hits
        .iter()
        .find(|hit| {
            hit.target == AddControlId::BrowseSource
                && region_text(&buffer, hit.area).contains("[Ctrl+O] Select")
        })
        .expect("the advertised Browse button is a typed mouse hit");
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            session.handle_event(mouse(kind, browse.area.x, browse.area.y), &state, &geometry),
            None
        );
        assert_eq!(session.focused(), Some(&ADD_PATH));
    }
    assert!(matches!(
        add_click(
            &mut session,
            &state,
            &geometry,
            browse.area.x,
            browse.area.y,
        ),
        Some(AddScreenEvent::OpenPathPicker(_))
    ));
    assert_eq!(session.focused(), Some(&ADD_PATH));

    // ↓ moves on and ↑ comes back. Both paths report their real ephemeral-state change.
    assert_eq!(
        session.handle_event(key(KeyCode::Down), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_TEMPLATE));
    assert_eq!(
        session.handle_event(key(KeyCode::Up), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let (buffer, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_PATH));

    // The forward footer hit ignores pointer motion and a bare release. A primary click moves on.
    let next = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::NextField)
        .expect("the add footer advertises a typed next-field hit");
    assert!(region_text(&buffer, next.area).contains("Tab/↓"));
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            session.handle_event(mouse(kind, next.area.x, next.area.y), &state, &geometry),
            None
        );
        assert_eq!(session.focused(), Some(&ADD_PATH));
    }
    assert_eq!(
        add_click(&mut session, &state, &geometry, next.area.x, next.area.y,),
        Some(AddScreenEvent::Changed)
    );
    let (buffer, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_TEMPLATE));

    // The backward footer chip has the same event discipline and returns to the path field.
    let previous = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::PreviousField)
        .expect("the add footer advertises a typed previous-field hit");
    assert!(region_text(&buffer, previous.area).contains("Shift+Tab/↑"));
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            session.handle_event(
                mouse(kind, previous.area.x, previous.area.y),
                &state,
                &geometry,
            ),
            None
        );
        assert_eq!(session.focused(), Some(&ADD_TEMPLATE));
    }
    assert_eq!(
        add_click(
            &mut session,
            &state,
            &geometry,
            previous.area.x,
            previous.area.y,
        ),
        Some(AddScreenEvent::Changed)
    );
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_PATH));

    // The advertised keys themselves, not only their mouse twins.
    assert_eq!(
        session.handle_event(key(KeyCode::Tab), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_TEMPLATE));
    assert_eq!(
        session.handle_event(shift_back_tab(), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let _ = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&ADD_PATH));
}

// ==========================================================================
// 3. Add review — REAL: arrows, both footer hits, and Tab walk the review fields
// ==========================================================================

const RV_NAME: AddControlId = AddControlId::Text(AddTextField::ReviewName);
const RV_DESC: AddControlId = AddControlId::Text(AddTextField::ReviewDescription);

/// One byte-exact source snapshot, as the host hands it to the review, mirroring the oracle's
/// `AddReviewScreen(path)`.
fn snapshot(path: &str, bytes: &[u8]) -> SourceSnapshot {
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
        is_draft: false,
        identity: None,
    }
}

#[test]
fn test_add_review_boots_on_name_and_arrows_move() {
    // The review panel boots on its name field (not the body scroll container); ↓/↑ walk
    // name<->description, the footer chip does the same, and Tab/Shift+Tab agree.
    let review = ReviewState::from_source(
        snapshot("job.py", b"CITY = \"x\"\nprint(CITY)\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    let state = AddWorkflowState::from_review(review);
    let mut session = AddScreenSession::default();
    let (buffer, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_NAME)); // rv-name, not the body scroll
    let screen = buffer_text(&buffer);
    assert!(
        screen.contains("Tab/↓"),
        "missing forward navigation chip:\n{screen}"
    );
    assert!(
        screen.contains("Shift+Tab/↑"),
        "missing backward navigation chip:\n{screen}"
    );

    // ↓ moves on and ↑ comes back.
    assert_eq!(
        session.handle_event(key(KeyCode::Down), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_DESC));
    assert_eq!(
        session.handle_event(key(KeyCode::Up), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let (buffer, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_NAME));

    // The rendered forward hit ignores pointer motion and a bare release. A click moves on.
    let next = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::NextField)
        .expect("the review footer advertises a typed next-field hit");
    assert!(region_text(&buffer, next.area).contains("Tab/↓"));
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            session.handle_event(mouse(kind, next.area.x, next.area.y), &state, &geometry),
            None
        );
        assert_eq!(session.focused(), Some(&RV_NAME));
    }
    assert_eq!(
        add_click(&mut session, &state, &geometry, next.area.x, next.area.y,),
        Some(AddScreenEvent::Changed)
    );
    let (buffer, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_DESC));

    // The backward chip is a separate typed hit and has the same pointer-event discipline.
    let previous = geometry
        .hits
        .iter()
        .find(|hit| region_text(&buffer, hit.area).contains("Shift+Tab/↑"))
        .expect("the review footer advertises a typed previous-field hit");
    assert_eq!(previous.target, AddControlId::PreviousField);
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            session.handle_event(
                mouse(kind, previous.area.x, previous.area.y),
                &state,
                &geometry,
            ),
            None
        );
        assert_eq!(session.focused(), Some(&RV_DESC));
    }
    assert_eq!(
        add_click(
            &mut session,
            &state,
            &geometry,
            previous.area.x,
            previous.area.y,
        ),
        Some(AddScreenEvent::Changed)
    );
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_NAME));

    // Tab/Shift+Tab themselves walk the same way.
    assert_eq!(
        session.handle_event(key(KeyCode::Tab), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let (_, geometry) = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_DESC));
    assert_eq!(
        session.handle_event(shift_back_tab(), &state, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let _ = draw_add(&mut session, &state);
    assert_eq!(session.focused(), Some(&RV_NAME));
}

#[test]
fn add_kind_and_open_select_keep_their_arrow_ownership() {
    let mut kind = AddWorkflowState::new(Vec::new());
    let _ = kind.reduce(AddAction::SetSourcePath("mystery".to_owned()));
    let effects = kind.reduce(AddAction::Continue);
    let request = effects
        .iter()
        .find_map(|effect| match effect {
            AddEffect::InspectSource { request, .. } => Some(*request),
            _ => None,
        })
        .expect("the source lane requests inspection");
    let _ = kind.reduce(AddAction::SourceInspected {
        request,
        result: Ok(snapshot("mystery", b"opaque\n")),
    });
    assert_eq!(kind.stage(), AddStage::Kind);

    let mut kind_session = AddScreenSession::default();
    let (_, geometry) = draw_add(&mut kind_session, &kind);
    let first = kind_session.focused().cloned().expect("focused kind row");
    assert_eq!(
        kind_session.handle_event(key(KeyCode::Down), &kind, &geometry),
        Some(AddScreenEvent::Changed)
    );
    let second = kind_session.focused().cloned().expect("next kind row");
    assert!(matches!(second, AddControlId::Kind(_)));
    assert_ne!(second, first);
    let (_, geometry) = draw_add(&mut kind_session, &kind);
    assert_eq!(
        kind_session.handle_event(key(KeyCode::Up), &kind, &geometry),
        Some(AddScreenEvent::Changed)
    );
    assert_eq!(kind_session.focused(), Some(&first));

    let review = ReviewState::from_source(
        snapshot("job.py", b"print('ok')\n"),
        KnownEntryKind::Python,
        ReviewDefaults::default(),
    );
    let review = AddWorkflowState::from_review(review);
    let mut select_session = AddScreenSession::default();
    let (_, geometry) = draw_add(&mut select_session, &review);
    let storage = geometry
        .hits
        .iter()
        .find(|hit| hit.target == AddControlId::Storage)
        .expect("the non-fresh review renders its storage select");
    assert_eq!(
        add_click(
            &mut select_session,
            &review,
            &geometry,
            storage.area.x,
            storage.area.y,
        ),
        Some(AddScreenEvent::Changed)
    );
    assert_eq!(select_session.focused(), Some(&AddControlId::Storage));
    let (_, geometry) = draw_add(&mut select_session, &review);
    assert!(
        select_session
            .handle_event(key(KeyCode::Down), &review, &geometry)
            .is_some()
    );
    assert_eq!(select_session.focused(), Some(&AddControlId::Storage));
    let (_, geometry) = draw_add(&mut select_session, &review);
    assert!(
        select_session
            .handle_event(key(KeyCode::Up), &review, &geometry)
            .is_some()
    );
    assert_eq!(select_session.focused(), Some(&AddControlId::Storage));
}

// ==========================================================================
// 4. Preferences — REAL: widget-owned keys and both shared footer hits agree
// ==========================================================================

fn prefs_focus(state: &LibraryState) -> PreferencesControlId {
    match state.screen() {
        Screen::Preferences(view) => view.focused(),
        other => panic!("expected the preferences screen, got {other:?}"),
    }
}

/// The oracle's preferences fixture: the language picker, an editor input, and the form-style
/// choice section a ↓ steps into.
fn preferences_view() -> skit_ui::PreferencesView {
    use skit_application::preferences::{
        AfterRunChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
        PreferencesSnapshot,
    };
    skit_ui::PreferencesView::new(PreferencesDraft::from_snapshot(PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vim".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Exit,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: Vec::new(),
        mirror: MirrorConfiguration::default(),
    }))
}

#[test]
fn test_prefs_boots_on_language_and_arrows_move() {
    // Boots on the language dropdown (not the scroll); moving into the RadioSet, the arrows belong
    // to its OPTIONS — leaving it is Tab's (or the chip's) job, and the shared bindings must not
    // steal them; the chip walks on and the back chip returns.
    let mut state = present(Screen::Preferences(Box::new(preferences_view())));
    let mut session = TuiSession::default();
    let (screen, geometry) = draw(&mut session, &state);
    assert_eq!(prefs_focus(&state), PreferencesControlId::Language); // the language dropdown
    assert!(
        screen.contains("Tab/↓"),
        "missing forward navigation chip:\n{screen}"
    );
    assert!(
        screen.contains("Shift+Tab/↑"),
        "missing backward navigation chip:\n{screen}"
    );
    for command in [UiCommand::FocusNext, UiCommand::FocusPrevious] {
        assert_eq!(
            geometry
                .hits
                .iter()
                .filter(|hit| hit.action == HitTarget::Command(command))
                .count(),
            1,
            "the rendered Preferences footer needs one typed {command:?} hit"
        );
    }

    // Focus the editor input (the oracle's `query_one("#pf-editor").focus()`): one Tab, since
    // Language and Editor are adjacent stops.
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Tab)),
        EventHandling::Action(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::Editor,
        )))
    );
    assert_eq!(prefs_focus(&state), PreferencesControlId::Editor);

    // ↓ off the input moves into the form-style radio section.
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Down)),
        EventHandling::Action(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::InteractiveForm,
        )))
    );
    let radio = prefs_focus(&state);
    assert_eq!(radio, PreferencesControlId::InteractiveForm);
    // Inside the RadioSet the arrows belong to the OPTIONS: another ↓ stays on the same widget.
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Down)),
        EventHandling::Action(Action::Preferences(PreferencesAction::SetInteractiveForm(
            InteractiveFormChoice::Plain
        ),))
    );
    assert_eq!(prefs_focus(&state), radio); // still the same widget…

    // …and only a primary press and matching release moves to the next section.
    let (_, geometry) = draw(&mut session, &state);
    let next = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusNext))
        .expect("the Preferences footer exposes FocusNext")
        .rect;
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            drive(
                &mut session,
                &mut state,
                &geometry,
                mouse(kind, next.x, next.y),
            ),
            EventHandling::Ignored
        );
        assert_eq!(prefs_focus(&state), radio);
    }
    assert_eq!(
        drive_click(&mut session, &mut state, &geometry, next.x, next.y,),
        EventHandling::Action(Action::FocusNext)
    );
    assert_eq!(prefs_focus(&state), PreferencesControlId::AfterRun);

    // The backward chip has the same pointer-event discipline and returns to the radio.
    let (_, geometry) = draw(&mut session, &state);
    let previous = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusPrevious))
        .expect("the Preferences footer exposes FocusPrevious")
        .rect;
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            drive(
                &mut session,
                &mut state,
                &geometry,
                mouse(kind, previous.x, previous.y),
            ),
            EventHandling::Ignored
        );
        assert_eq!(prefs_focus(&state), PreferencesControlId::AfterRun);
    }
    assert_eq!(
        drive_click(&mut session, &mut state, &geometry, previous.x, previous.y,),
        EventHandling::Action(Action::FocusPrevious)
    );
    assert_eq!(prefs_focus(&state), radio);

    // The advertised keys themselves, not just the chips.
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, shift_back_tab()),
        EventHandling::Action(Action::Preferences(PreferencesAction::Focus(
            PreferencesControlId::Editor,
        )))
    );
    assert_eq!(prefs_focus(&state), PreferencesControlId::Editor);
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Tab)),
        EventHandling::Action(Action::Preferences(PreferencesAction::Focus(radio)))
    );
    assert_eq!(prefs_focus(&state), radio);
}

#[test]
fn preferences_widgets_claim_their_own_keys_before_shared_navigation() {
    let mut state = present(Screen::Preferences(Box::new(preferences_view())));
    let mut session = TuiSession::default();

    // Down opens the focused language picker instead of moving to the next Preferences control.
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Down)),
        EventHandling::Consumed
    );
    assert_eq!(prefs_focus(&state), PreferencesControlId::Language);
    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Up));
    assert_eq!(prefs_focus(&state), PreferencesControlId::Language);

    // A fresh input keeps printable and horizontal cursor keys; only its unused vertical arrows
    // move through the form.
    state = present(Screen::Preferences(Box::new(preferences_view())));
    session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    let _ = drive(&mut session, &mut state, &geometry, key(KeyCode::Tab));
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Char('x')),),
        EventHandling::Action(Action::Preferences(PreferencesAction::SetEditor(
            "x".to_owned(),
        )))
    );
    assert_eq!(prefs_focus(&state), PreferencesControlId::Editor);
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Left)),
        EventHandling::Consumed
    );
    assert_eq!(prefs_focus(&state), PreferencesControlId::Editor);
}

// ==========================================================================
// 5. Settings — DIVERGENCE: ↑ off the Multiline description cannot return to the name
// ==========================================================================

fn settings_focus(state: &LibraryState) -> String {
    match state.screen() {
        Screen::Settings(view) => view.focused().to_owned(),
        other => panic!("expected the settings screen, got {other:?}"),
    }
}

/// The oracle's `ScriptSettingsScreen(entry)` for a stored python copy with one managed constant.
fn settings_view() -> SettingsView {
    let mut message = ParamDecl::new("MESSAGE");
    message.default = Some(ParameterValue::String("Hello".to_owned()));
    SettingsView::from_inputs(&SettingsInputs {
        selector: "two".to_owned(),
        kind: "python".to_owned(),
        name: "two".to_owned(),
        description: "A stored python copy.".to_owned(),
        source: "/demo/two.py".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        has_analyzer: true,
        managed: vec![message],
        ..SettingsInputs::default()
    })
}

#[test]
fn test_settings_boots_on_name_and_arrows_move() {
    // The settings screen boots on its name field (not the body scroll container); ↓/↑ walk
    // name<->the next field, the footer chips do the same, and Tab/Shift+Tab agree.
    let mut state = present(Screen::Settings(Box::new(settings_view())));
    let mut session = TuiSession::default();
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(settings_focus(&state), "name"); // st-name

    // ↓ moves on to the next control.
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Down)),
        EventHandling::Action(Action::Settings(skit_ui::SettingsAction::FocusNext))
    );
    let second = settings_focus(&state);
    assert_eq!(second, "description");
    let description = match state.screen() {
        Screen::Settings(view) => view
            .field("description")
            .unwrap()
            .value()
            .as_text()
            .to_owned(),
        _ => unreachable!(),
    };
    // The one-line description is already at the textarea's top boundary, so ↑ yields navigation.
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Up)),
        EventHandling::Action(Action::Settings(skit_ui::SettingsAction::FocusPrevious))
    );
    assert_eq!(settings_focus(&state), "name");
    match state.screen() {
        Screen::Settings(view) => assert_eq!(
            view.field("description").unwrap().value().as_text(),
            description
        ),
        _ => unreachable!(),
    }

    // The chips are the same actions. Hover and a bare release are inert; a primary click acts.
    let (footer, geometry) = draw(&mut session, &state);
    assert!(footer.contains("Tab/↓"), "{footer}");
    assert!(footer.contains("Shift+Tab/↑"), "{footer}");
    let next = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusNext))
        .expect("the Settings footer exposes FocusNext")
        .rect;
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            drive(
                &mut session,
                &mut state,
                &geometry,
                mouse(kind, next.x, next.y),
            ),
            EventHandling::Ignored
        );
        assert_eq!(settings_focus(&state), "name");
    }
    assert_eq!(
        drive_click(&mut session, &mut state, &geometry, next.x, next.y,),
        EventHandling::Action(Action::FocusNext)
    );
    assert_eq!(settings_focus(&state), second);

    let (_, geometry) = draw(&mut session, &state);
    let previous = geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(UiCommand::FocusPrevious))
        .expect("the Settings footer exposes FocusPrevious")
        .rect;
    for kind in [MouseEventKind::Moved, MouseEventKind::Up(MouseButton::Left)] {
        assert_eq!(
            drive(
                &mut session,
                &mut state,
                &geometry,
                mouse(kind, previous.x, previous.y),
            ),
            EventHandling::Ignored
        );
        assert_eq!(settings_focus(&state), second);
    }
    assert_eq!(
        drive_click(&mut session, &mut state, &geometry, previous.x, previous.y,),
        EventHandling::Action(Action::FocusPrevious)
    );
    assert_eq!(settings_focus(&state), "name");

    // The advertised keys themselves, not just the chips.
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, key(KeyCode::Tab)),
        EventHandling::Action(Action::Settings(skit_ui::SettingsAction::FocusNext))
    );
    assert_eq!(settings_focus(&state), second);
    let (_, geometry) = draw(&mut session, &state);
    assert_eq!(
        drive(&mut session, &mut state, &geometry, shift_back_tab()),
        EventHandling::Action(Action::Settings(skit_ui::SettingsAction::FocusPrevious))
    );
    assert_eq!(settings_focus(&state), "name");
}
