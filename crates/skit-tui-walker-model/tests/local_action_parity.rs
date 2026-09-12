//! Screen-local action parity: every advertised local action works by key and by mouse.
//!
//! Product rule 2 requires each TUI action to stay available by keyboard and by mouse. These
//! contracts cover the screen-local action inventory beside the global command registry. Every
//! state is built from `LibraryState` values, so no host is involved. The probe helpers come from
//! this crate's `parity` module.

use std::collections::BTreeMap;

use ratatui_core::layout::{Rect, Size};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::LibraryScan;
use skit_application::preferences::{
    AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration, PreferencesDraft,
    PreferencesSnapshot,
};
use skit_application::tokens::TokenContext;
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterType, ParameterValue},
};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, HitTarget, LocalActionInventory, LocalAdvertisedAction, TuiSession, ViewGeometry,
};
use skit_tui_walker_model::parity::{
    check_local_endpoint, render_probe_endpoint, render_probe_session, session_primary_click,
};
use skit_ui::{
    Action, AddWorkflowState, Effect, HealthIssue, HealthIssueKind, HealthSnapshot, HealthView,
    LibraryState, LibrarySurface, MirrorHealth, PreferencesAction, PreferencesView, RunFormContext,
    RunFormView, RunPathContext, RunnerManagerAction, RunnerManagerView, RunnerRow,
    RunnerRowIdentity, Screen, UiCommand, UvHealth,
};

// --------------------------------------------------------------------------
// render and event helpers
// --------------------------------------------------------------------------

fn render(
    state: &LibraryState,
    session: &mut TuiSession,
    width: u16,
    height: u16,
) -> LocalActionInventory {
    render_locale(state, session, width, height, Locale::En)
}

fn render_locale(
    state: &LibraryState,
    session: &mut TuiSession,
    width: u16,
    height: u16,
    locale: Locale,
) -> LocalActionInventory {
    render_probe_endpoint(session, state, locale, Size::new(width, height)).unwrap();
    session.local_action_inventory().clone()
}

fn render_geometry(
    state: &LibraryState,
    session: &mut TuiSession,
    width: u16,
    height: u16,
) -> ViewGeometry {
    render_probe_endpoint(session, state, Locale::En, Size::new(width, height))
        .unwrap()
        .geometry
}

fn handling_for_key(state: &LibraryState, key: KeyEvent, width: u16, height: u16) -> EventHandling {
    handling_for_key_locale(state, key, width, height, Locale::En)
}

fn handling_for_key_locale(
    state: &LibraryState,
    key: KeyEvent,
    width: u16,
    height: u16,
    locale: Locale,
) -> EventHandling {
    let (mut session, geometry) =
        render_probe_session(state, locale, Size::new(width, height)).unwrap();
    session.handle_event(Event::Key(key), state, &geometry)
}

fn handling_for_hit(
    state: &LibraryState,
    rect: Rect,
    width: u16,
    height: u16,
) -> (Event, EventHandling) {
    handling_for_hit_locale(state, rect, width, height, Locale::En)
}

/// Send one complete primary click over the centre of `rect`.
///
/// The result is the release event and the endpoint that the release reached.
fn handling_for_hit_locale(
    state: &LibraryState,
    rect: Rect,
    width: u16,
    height: u16,
    locale: Locale,
) -> (Event, EventHandling) {
    let (mut session, geometry) =
        render_probe_session(state, locale, Size::new(width, height)).unwrap();
    let column = rect.x.saturating_add(rect.width / 2);
    let row = rect.y.saturating_add(rect.height / 2);
    let handling = session_primary_click(&mut session, state, &geometry, column, row)
        .expect("a positive local mouse path must arm on primary press");
    let release = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Up(MouseButton::Left),
        column,
        row,
        modifiers: KeyModifiers::NONE,
    });
    (release, handling)
}

fn assert_outcome(
    event: &Event,
    advertised: &LocalAdvertisedAction,
    actual: &EventHandling,
    context: &str,
) {
    if let Err(error) = check_local_endpoint(event, advertised, actual) {
        panic!("{context} descriptor does not match its live endpoint: {error}");
    }
}

fn assert_inventory_parity(state: &LibraryState, expected_context: &str) {
    let (width, height) = (120, 30);
    let inventory = render(state, &mut TuiSession::default(), width, height);
    assert!(
        !inventory.actions.is_empty(),
        "{expected_context} must expose its visible local advertised actions"
    );
    for advertised in &inventory.actions {
        let rect = advertised.hit.unwrap_or_else(|| {
            panic!("{expected_context} has a visible key without a mouse hit: {advertised:?}")
        });
        assert!(
            rect.width > 0 && rect.height > 0,
            "clipped hit leaked: {advertised:?}"
        );
        assert!(
            !advertised.keys.is_empty(),
            "{expected_context} has a visible mouse action without an advertised key: {advertised:?}"
        );
        let (release, mouse) = handling_for_hit(state, rect, width, height);
        assert_outcome(&release, advertised, &mouse, expected_context);
        for key in &advertised.keys {
            let handling = handling_for_key(state, key.event(), width, height);
            assert_outcome(
                &Event::Key(key.event()),
                advertised,
                &handling,
                expected_context,
            );
            assert_eq!(
                handling, mouse,
                "{expected_context} local action has different key and mouse endpoints: {advertised:?}",
            );
        }
    }
}

fn assert_inventory_surface(
    state: &LibraryState,
    context: &str,
    locale: Locale,
    width: u16,
    height: u16,
) {
    let inventory = render_locale(state, &mut TuiSession::default(), width, height, locale);
    assert!(
        width < 24 || !inventory.actions.is_empty(),
        "{context} advertises no local action at {width}x{height} in {locale:?}"
    );
    for (index, advertised) in inventory.actions.iter().enumerate() {
        let rect = advertised.hit.expect("visible action has a hit");
        assert!(
            !rect.is_empty() && rect.right() <= width && rect.bottom() <= height,
            "{context} local rect is outside {width}x{height}: {advertised:?}",
        );
        for other in inventory.actions.iter().skip(index + 1) {
            let other_rect = other.hit.expect("visible action has a hit");
            assert!(
                rect.intersection(other_rect).is_empty() || advertised.target == other.target,
                "{context} has ambiguous local hits: {advertised:?} and {other:?}",
            );
        }
        let (release, mouse) = handling_for_hit_locale(state, rect, width, height, locale);
        assert_outcome(&release, advertised, &mouse, context);
        for key in &advertised.keys {
            let handling = handling_for_key_locale(state, key.event(), width, height, locale);
            assert_outcome(&Event::Key(key.event()), advertised, &handling, context);
            assert_eq!(handling, mouse, "{context} alias differs: {advertised:?}");
        }
    }
}

// --------------------------------------------------------------------------
// screen fixtures
// --------------------------------------------------------------------------

fn present(screen: Screen) -> LibraryState {
    let mut state = LibraryState::default();
    let effect = state.update(Action::Present(screen));
    assert_eq!(effect, Effect::None);
    state
}

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

fn add_state() -> LibraryState {
    present(Screen::Add(Box::new(AddWorkflowState::new(Vec::new()))))
}

fn health_state() -> LibraryState {
    present(Screen::Health(Box::new(HealthView::new(HealthSnapshot {
        uv: UvHealth::NotRequired,
        entry_count: 2,
        issues: vec![HealthIssue {
            slug: "missing".to_owned(),
            name: "Missing".to_owned(),
            kind: HealthIssueKind::MissingTarget,
        }],
        invalid_runner_rows: Vec::new(),
        mirror: MirrorHealth::Off,
        library_path: "/data/scripts".to_owned(),
        library_size: "2 KiB".to_owned(),
        diagnostics: Vec::new(),
    }))))
}

/// The registry shape of the legacy `model_walker/fixtures.rs:905`: one valid pinned row and
/// one malformed row. Each snapshot token is distinct, as the production store always makes
/// them.
fn runner_rows() -> Vec<RunnerRow> {
    let valid = RunnerRowIdentity {
        index: Some(0),
        snapshot_token: concat!(
            "row:0:name=Some(\"codex\"):argv=Some([\"codex\", \"exec\", ",
            "\"{{prompt}}\"]):reason=None:descriptor=\"codex\""
        )
        .to_owned(),
    };
    vec![
        RunnerRow {
            identity: valid.clone(),
            name: Some("codex".to_owned()),
            argv: Some(vec![
                "codex".to_owned(),
                "exec".to_owned(),
                "{{prompt}}".to_owned(),
            ]),
            reason: None,
            descriptor: "codex".to_owned(),
            key_identities: vec![valid],
            pinned_count: 1,
        },
        RunnerRow {
            identity: RunnerRowIdentity {
                index: Some(1),
                snapshot_token: concat!(
                    "row:1:name=None:argv=Some([\"broken\", \"{{prompt}}\"]):",
                    "reason=Some(\"name_missing\"):descriptor=\"malformed row 1\""
                )
                .to_owned(),
            },
            name: None,
            argv: Some(vec!["broken".to_owned(), "{{prompt}}".to_owned()]),
            reason: Some("name_missing".to_owned()),
            descriptor: "malformed row 1".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        },
    ]
}

fn runners_state() -> LibraryState {
    present(Screen::Runners(Box::new(RunnerManagerView::new(
        runner_rows(),
    ))))
}

/// The Runners screen with the malformed row selected.
fn runners_on_invalid_row() -> LibraryState {
    let mut state = runners_state();
    assert_eq!(
        state.update(Action::Runners(RunnerManagerAction::Select(1))),
        Effect::None
    );
    state
}

fn runners_with(action: RunnerManagerAction) -> LibraryState {
    let mut state = runners_state();
    assert_eq!(state.update(Action::Runners(action)), Effect::None);
    state
}

fn prompt_run_state() -> LibraryState {
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    path.default = Some(ParameterValue::String("default.txt".to_owned()));
    let form = RunFormView::from_declarations(
        "prompt",
        "Prompt",
        &[path],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "prompt".to_owned(),
        path: Some(RunPathContext {
            workdir: "/work".to_owned(),
            invoke_cwd: "/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/invoke".to_owned(),
            home: None,
            env: BTreeMap::new(),
            today: "2026-08-28".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    present(Screen::Run(Box::new(form)))
}

fn standalone_runner_editor_state() -> LibraryState {
    let mut state = prompt_run_state();
    assert_eq!(state.update(Action::OpenRunRunnerEditor), Effect::None);
    state
}

fn preferences_state() -> LibraryState {
    let mut state = present(Screen::Preferences(Box::new(PreferencesView::new(
        PreferencesDraft::from_snapshot(PreferencesSnapshot {
            language: String::new(),
            available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
            effective_language: "en".to_owned(),
            editor: String::new(),
            editor_fallback: Some("vim".to_owned()),
            form: InteractiveFormChoice::Tui,
            after_run: AfterRunChoice::Exit,
            javascript: JavascriptChoice::Automatic,
            bash_path: None,
            runner_names: vec!["codex".to_owned()],
            mirror: MirrorConfiguration::default(),
        }),
    ))));
    assert_eq!(
        state.update(Action::Preferences(PreferencesAction::SetEditor(
            "micro".to_owned()
        ))),
        Effect::None
    );
    state
}

// --------------------------------------------------------------------------
// contracts
// --------------------------------------------------------------------------

/// Ledger row `local_inventory.rs:319`.
#[test]
fn local_inventory_describes_only_the_last_rendered_screen() {
    let library = library_state();
    let mut session = TuiSession::default();
    assert!(
        library.selected().is_some(),
        "the Library fixture must render its populated browser"
    );
    assert!(
        render(&library, &mut session, 80, 24).actions.is_empty(),
        "the Library screen has no screen-local action inventory"
    );
    let health = health_state();
    assert!(
        !render(&health, &mut session, 80, 24).actions.is_empty(),
        "the inventory must follow the last rendered frame, not a stale one"
    );
}

/// Ledger row `local_inventory.rs:216`.
#[test]
fn every_advertised_local_action_has_a_hit_and_an_equal_key_endpoint() {
    assert_inventory_parity(&add_state(), "Add");
    assert_inventory_parity(&health_state(), "Health");
    assert_inventory_parity(&runners_state(), "Runners");
    assert_inventory_parity(&runners_on_invalid_row(), "Runners on a malformed row");
    assert_inventory_parity(&runners_with(RunnerManagerAction::New), "Runners editor");
    assert_inventory_parity(&standalone_runner_editor_state(), "RunnerEditor");
}

/// Ledger row `local_inventory.rs:263`.
#[test]
fn local_inventory_is_empty_for_clipped_cells_and_open_overlays() {
    let state = add_state();
    let mut session = TuiSession::default();
    assert!(
        render(&state, &mut session, 1, 1).actions.is_empty(),
        "a fully clipped one-cell Add screen must not invent a local action",
    );
    let inventory = render(&state, &mut session, 24, 6);
    assert!(
        !inventory.actions.is_empty(),
        "the compact Add tier must keep its local actions"
    );
    assert!(
        inventory.actions.iter().all(|action| {
            action
                .hit
                .is_some_and(|rect| rect.width > 0 && rect.height > 0)
        }),
        "the compact Add tier published a clipped hit"
    );

    let geometry = render_geometry(&state, &mut session, 80, 24);
    let open_picker = KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(
        session.handle_event(Event::Key(open_picker), &state, &geometry),
        EventHandling::Consumed,
        "Ctrl+O must open the Add path picker"
    );
    assert!(
        render(&state, &mut session, 80, 24).actions.is_empty(),
        "an open overlay must empty the base screen inventory",
    );
}

/// Ledger row `local_inventory.rs:301`.
#[test]
fn every_advertised_alias_reaches_the_mouse_endpoint() {
    let state = runners_with(RunnerManagerAction::New);
    let inventory = render(&state, &mut TuiSession::default(), 120, 30);
    let alias = inventory
        .actions
        .iter()
        .find(|action| action.keys.len() > 1)
        .expect("the runner editor advertises both Tab/down or BackTab/up");
    let (_, expected) = handling_for_hit(&state, alias.hit.unwrap(), 120, 30);
    for key in &alias.keys {
        assert_eq!(
            handling_for_key(&state, key.event(), 120, 30),
            expected,
            "alias {key:?} does not reach the mouse endpoint of {alias:?}"
        );
    }
}

/// Ledger row `local_inventory.rs:329`.
#[test]
fn preferences_shared_focus_hits_return_typed_preferences_actions() {
    let state = preferences_state();
    let geometry = render_geometry(&state, &mut TuiSession::default(), 80, 24);
    let mut checked = 0_usize;
    for hit in geometry.hits.iter().filter(|hit| {
        matches!(
            hit.action,
            HitTarget::Command(UiCommand::FocusNext | UiCommand::FocusPrevious)
        )
    }) {
        let (_, handling) = handling_for_hit(&state, hit.rect, 80, 24);
        assert!(
            matches!(handling, EventHandling::Action(Action::Preferences(_))),
            "{hit:?} returned {handling:?}",
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "the Preferences frame must publish its shared focus hits"
    );
}

/// Ledger row `local_inventory.rs:485`.
#[test]
fn runner_local_surfaces_are_bounded_and_unambiguous_in_every_tier_and_locale() {
    let manager = runners_state();
    let actions = runners_with(RunnerManagerAction::ActivateSelected);
    let mut removal = actions.clone();
    assert_eq!(
        removal.update(Action::Runners(RunnerManagerAction::RemoveSelected)),
        Effect::None
    );
    let nested_editor = runners_with(RunnerManagerAction::New);
    let standalone_editor = standalone_runner_editor_state();

    let invalid = runners_on_invalid_row();
    let mut invalid_actions = invalid.clone();
    assert_eq!(
        invalid_actions.update(Action::Runners(RunnerManagerAction::ActivateSelected)),
        Effect::None
    );
    let mut invalid_removal = invalid_actions.clone();
    assert_eq!(
        invalid_removal.update(Action::Runners(RunnerManagerAction::RemoveSelected)),
        Effect::None
    );

    for (context, state) in [
        ("Runners", &manager),
        ("Runners actions", &actions),
        ("Runners removal", &removal),
        ("Runners editor", &nested_editor),
        ("RunnerEditor", &standalone_editor),
        ("Runners on a malformed row", &invalid),
        ("Runners actions on a malformed row", &invalid_actions),
        ("Runners removal of a malformed row", &invalid_removal),
    ] {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            for (width, height) in [(1, 1), (24, 6), (120, 30)] {
                assert_inventory_surface(state, context, locale, width, height);
            }
        }
    }
}
