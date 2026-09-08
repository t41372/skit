use std::collections::BTreeMap;

use ratatui_core::layout::{Rect, Size};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_application::{
    LibraryScan,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    },
    tokens::TokenContext,
};
use skit_domain::{
    EntryKind, EntrySummary, Slug, StorageMode,
    parameters::{ParamDecl, ParameterType, ParameterValue},
};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, HitRegion, HitTarget, LocalActionOutcome, LocalActionTarget,
    LocalAdvertisedAction, RunFieldCommand, TuiSession, ViewGeometry,
};
use skit_ui::{
    Action, AddWorkflowState, Effect, FormField, FormPurpose, FormView, HealthAction, HealthIssue,
    HealthIssueKind, HealthSnapshot, HealthView, LibraryState, LibrarySurface, MirrorHealth,
    PreferencesAction, PreferencesView, RunFormContext, RunFormView, RunPathContext,
    RunTokenOption, RunnerManagerView, RunnerRow, RunnerRowIdentity, Screen, UiBinding, UiCommand,
    UiKey, UiModifiers, UvHealth,
};

use crate::parity::{
    ProbeEndpoint, apply_probe_handling, binding_event, browse_keyboard_action,
    check_browse_endpoint, check_command_hit_parity, check_command_key_endpoint,
    check_field_hit_parity, check_hit_bounds, check_keyboard_focus_path, check_local_endpoint,
    check_local_event, check_local_press, check_probe_effect, check_public_hit_parity,
    check_run_field_hit, command_binding, command_bindings, command_key_action,
    expected_hit_action, field_activation_sequences, focus_prefix_failure, host_effect_pending,
    is_clipped, is_plain_focus_field, is_plain_focus_handling, matching_primary_release,
    primary_click_events, public_command_bindings, public_hit_probe_point, publishes_target,
    render_probe_endpoint, render_probe_session, run_field_action, run_field_ui_command,
    select_last_token, session_action, session_primary_click, token_file_index,
};

// --------------------------------------------------------------------------
// state fixtures
// --------------------------------------------------------------------------

fn present(screen: Screen) -> LibraryState {
    let mut state = LibraryState::default();
    assert_eq!(state.update(Action::Present(screen)), Effect::None);
    state
}

fn entry(slug: &str, name: &str, kind: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).expect("the fixture slug is valid"),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).expect("the fixture kind is valid"),
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

fn typed_controls_state() -> LibraryState {
    let mut enabled = ParamDecl::new("ENABLED");
    enabled.parameter_type = ParameterType::Bool;
    enabled.default = Some(ParameterValue::Bool(false));
    let mut format = ParamDecl::new("FORMAT");
    format.parameter_type = ParameterType::Choice;
    format.choices = vec!["json".to_owned(), "yaml".to_owned(), "toml".to_owned()];
    format.default = Some(ParameterValue::String("yaml".to_owned()));
    present(Screen::Run(Box::new(RunFormView::from_declarations(
        "typed-controls",
        "Typed controls",
        &[enabled, format],
        &BTreeMap::new(),
        &["codex".to_owned(), "claude".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    ))))
}

fn long_form_state() -> LibraryState {
    let declarations = (0..16)
        .map(|index| ParamDecl::new(format!("field-{index}")))
        .collect::<Vec<_>>();
    present(Screen::Run(Box::new(RunFormView::from_declarations(
        "long",
        "Long form",
        &declarations,
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    ))))
}

fn prompt_run_state() -> LibraryState {
    let mut path = ParamDecl::new("path");
    path.parameter_type = ParameterType::Path;
    path.default = Some(ParameterValue::String("default.txt".to_owned()));
    let mut note = ParamDecl::new("note");
    note.default = Some(ParameterValue::String("hello".to_owned()));
    let form = RunFormView::from_declarations(
        "prompt",
        "Prompt",
        &[path, note],
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

fn runner_row(index: usize, name: &str) -> RunnerRow {
    let identity = RunnerRowIdentity {
        index: Some(index),
        snapshot_token: "snapshot".to_owned(),
    };
    RunnerRow {
        identity: identity.clone(),
        name: Some(name.to_owned()),
        argv: Some(vec![name.to_owned(), "{{prompt}}".to_owned()]),
        reason: None,
        descriptor: name.to_owned(),
        key_identities: vec![identity],
        pinned_count: 0,
    }
}

fn runners_state() -> LibraryState {
    present(Screen::Runners(Box::new(RunnerManagerView::new(vec![
        runner_row(0, "codex"),
        runner_row(1, "claude"),
    ]))))
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

fn rename_form_state() -> LibraryState {
    present(Screen::Form(FormView {
        purpose: FormPurpose::Rename,
        title: "Rename".to_owned(),
        title_arguments: Vec::new(),
        translate_title: false,
        selector: Some("python-tool".to_owned()),
        fields: vec![FormField::text("name", "Name", "python-tool")],
        focused: 0,
        submit_label: "Save".to_owned(),
    }))
}

// --------------------------------------------------------------------------
// probe helpers
// --------------------------------------------------------------------------

const WIDE: Size = Size::new(120, 30);
const NORMAL: Size = Size::new(80, 24);
const COMPACT: Size = Size::new(24, 6);
const TINY: Size = Size::new(1, 1);

fn frame(state: &LibraryState, size: Size) -> (TuiSession, ViewGeometry) {
    render_probe_session(state, Locale::En, size).expect("the fixture frame renders")
}

fn region(action: HitTarget, rect: Rect) -> HitRegion {
    HitRegion { rect, action }
}

fn only(geometry: &ViewGeometry, hit: HitRegion) -> ViewGeometry {
    ViewGeometry {
        hits: vec![hit],
        ..geometry.clone()
    }
}

fn chip(geometry: &ViewGeometry, command: UiCommand) -> Rect {
    geometry
        .hits
        .iter()
        .find(|hit| hit.action == HitTarget::Command(command) && !is_clipped(hit))
        .unwrap_or_else(|| panic!("the frame must publish {command:?}"))
        .rect
}

fn field_hit(geometry: &ViewGeometry, target: HitTarget) -> Rect {
    geometry
        .hits
        .iter()
        .find(|hit| hit.action == target && !is_clipped(hit))
        .unwrap_or_else(|| panic!("the frame must publish {target:?}"))
        .rect
}

fn context_for<'a>(
    session: &'a TuiSession,
    state: &'a LibraryState,
    geometry: &'a ViewGeometry,
    size: Size,
) -> crate::parity::FieldParityContext<'a> {
    crate::parity::FieldParityContext {
        session,
        state,
        geometry,
        locale: Locale::En,
        size,
    }
}

fn binding(key: UiKey, modifiers: UiModifiers) -> UiBinding {
    UiBinding {
        key,
        modifiers,
        hint: "hint",
        compact_hint: "hint",
    }
}

fn advertised(target: LocalActionTarget, outcome: LocalActionOutcome) -> LocalAdvertisedAction {
    LocalAdvertisedAction {
        target,
        keys: Vec::new(),
        hit: Some(Rect::new(4, 4, 6, 1)),
        outcome,
    }
}

/// One advertised local action from a live Runners frame, with keys and a hit.
fn live_local_action() -> LocalAdvertisedAction {
    let state = runners_state();
    let (session, _) = frame(&state, NORMAL);
    session
        .local_action_inventory()
        .actions
        .iter()
        .find(|action| !action.keys.is_empty() && action.hit.is_some())
        .expect("the Runners frame advertises a local action with a key and a hit")
        .clone()
}

// --------------------------------------------------------------------------
// the positive sweep
// --------------------------------------------------------------------------

/// Ledger row `driver.rs:965`: every published hit reaches its advertised endpoint.
#[test]
fn every_published_hit_reaches_its_advertised_endpoint() {
    let states = [
        ("library", library_state(), 15),
        ("typed controls", typed_controls_state(), 16),
        ("prompt run", prompt_run_state(), 19),
        ("long form", long_form_state(), 9),
        ("preferences", preferences_state(), 4),
        ("rename form", rename_form_state(), 5),
    ];
    for (name, state, published) in &states {
        for size in [TINY, COMPACT, NORMAL, WIDE] {
            let (session, geometry) = frame(state, size);
            check_public_hit_parity(state, &geometry, size, &session, Locale::En)
                .unwrap_or_else(|error| panic!("{name} at {size:?}: {error}"));
        }
        for size in [NORMAL, WIDE] {
            let (_, geometry) = frame(state, size);
            assert!(
                geometry.hits.len() >= *published,
                "{name} at {size:?} publishes {} hits",
                geometry.hits.len()
            );
        }
    }
    // The Health, Runners and Add screens publish local actions instead of public hits.
    for (name, state) in [
        ("health", health_state()),
        ("runners", runners_state()),
        ("add", add_state()),
    ] {
        for size in [TINY, COMPACT, NORMAL, WIDE] {
            let (session, geometry) = frame(&state, size);
            assert!(
                geometry.hits.is_empty(),
                "{name} at {size:?} publishes hits"
            );
            check_public_hit_parity(&state, &geometry, size, &session, Locale::En)
                .unwrap_or_else(|error| panic!("{name} at {size:?}: {error}"));
        }
    }
    // The smallest viewport keeps one clipped record, which takes no mouse input.
    let clipped = rename_form_state();
    let (_, geometry) = frame(&clipped, TINY);
    assert!(
        geometry.hits.iter().any(is_clipped),
        "the smallest rename frame keeps a clipped record"
    );
}

// --------------------------------------------------------------------------
// public hit refusals
// --------------------------------------------------------------------------

/// A hit that reaches past either viewport edge is refused.
#[test]
fn refuses_a_hit_that_reaches_outside_the_viewport() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let inside = region(HitTarget::Command(UiCommand::Help), Rect::new(2, 2, 4, 1));
    check_hit_bounds(&inside, NORMAL).expect("a hit inside the viewport is accepted");
    let edge = region(HitTarget::Command(UiCommand::Help), Rect::new(76, 23, 4, 1));
    check_hit_bounds(&edge, NORMAL).expect("a hit that ends on the last cell is accepted");

    let right = region(HitTarget::Command(UiCommand::Help), Rect::new(78, 2, 4, 1));
    assert!(
        check_hit_bounds(&right, NORMAL)
            .unwrap_err()
            .starts_with("PUBLIC_HIT_BOUNDS")
    );
    let bottom = region(HitTarget::Command(UiCommand::Help), Rect::new(2, 23, 4, 2));
    assert!(
        check_hit_bounds(&bottom, NORMAL)
            .unwrap_err()
            .starts_with("PUBLIC_HIT_BOUNDS")
    );

    let forged = only(&geometry, right);
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert!(error.starts_with("PUBLIC_HIT_BOUNDS"), "{error}");
}

/// A clipped hit is not visible, so the sweep skips it.
#[test]
fn skips_a_clipped_hit() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let flat = region(HitTarget::Command(UiCommand::Help), Rect::new(78, 2, 4, 0));
    let thin = region(HitTarget::Command(UiCommand::Help), Rect::new(78, 2, 0, 1));
    assert!(is_clipped(&flat));
    assert!(is_clipped(&thin));
    assert!(!is_clipped(&region(
        HitTarget::Command(UiCommand::Help),
        Rect::new(2, 2, 4, 1)
    )));
    for hit in [flat, thin] {
        let forged = only(&geometry, hit);
        check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En)
            .expect("a clipped hit takes no mouse input");
    }
}

/// A command hit that this context cannot run is refused.
#[test]
fn refuses_a_command_hit_that_this_context_cannot_run() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let forged = only(
        &geometry,
        region(
            HitTarget::Command(UiCommand::NewRunner),
            Rect::new(2, 2, 4, 1),
        ),
    );
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert_eq!(error, "PUBLIC_HIT_CONTEXT command=NewRunner");
}

/// A launch-option hit that the form does not carry is refused.
#[test]
fn refuses_a_launch_option_hit_that_the_form_does_not_have() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let choice = choice_field(&state);
    let forged = only(
        &geometry,
        region(
            HitTarget::SelectFieldOption {
                field: choice,
                option: 99,
            },
            Rect::new(2, 2, 4, 1),
        ),
    );
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert_eq!(error, format!("RUN_OPTION_HIT field={choice} option=99"));
}

/// A hit over a cell that arms nothing is refused.
#[test]
fn refuses_a_press_that_the_session_does_not_arm() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let forged = only(
        &geometry,
        region(
            HitTarget::Command(UiCommand::Help),
            Rect::new(200, 200, 1, 1),
        ),
    );
    let error = check_public_hit_parity(&state, &forged, Size::new(300, 300), &session, Locale::En)
        .unwrap_err();
    assert!(error.starts_with("SESSION_CLICK_PRESS"), "{error}");
}

/// A hit whose click reaches another action is refused.
#[test]
fn refuses_a_hit_whose_click_reaches_another_action() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let help = chip(&geometry, UiCommand::Help);
    let forged = only(
        &geometry,
        region(HitTarget::Command(UiCommand::Reload), help),
    );
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert!(error.starts_with("PUBLIC_SESSION_HIT"), "{error}");
    assert!(error.contains("actual=OpenHelp"), "{error}");
}

/// A hit whose click returns no action is refused.
#[test]
fn refuses_a_hit_whose_click_returns_no_action() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let hit = region(
        HitTarget::Command(UiCommand::Help),
        chip(&geometry, UiCommand::Help),
    );
    let context = context_for(&session, &state, &geometry, NORMAL);
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_command_hit_parity(
        context,
        &hit,
        &Action::OpenHelp,
        fork,
        EventHandling::Consumed,
    )
    .unwrap_err();
    assert!(error.starts_with("PUBLIC_SESSION_HIT"), "{error}");
    assert!(error.contains("handling=Consumed"), "{error}");
}

/// A focus command may click to the concrete field that its key reaches.
#[test]
fn accepts_a_focus_command_click_that_names_another_action() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let hit = region(
        HitTarget::Command(UiCommand::FocusNext),
        chip(&geometry, UiCommand::FocusNext),
    );
    let context = context_for(&session, &state, &geometry, NORMAL);
    let fork = session.try_fork().expect("the fixture session forks");
    let result = check_command_hit_parity(
        context,
        &hit,
        &Action::FocusNext,
        fork,
        EventHandling::Action(Action::FocusField(1)),
    );
    let error = result.expect_err("the forged pointer action breaks the endpoint");
    assert!(!error.starts_with("PUBLIC_SESSION_HIT"), "{error}");
}

/// The typed field branch owns every field hit.
#[test]
fn refuses_a_field_hit_that_the_typed_branch_did_not_take() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    let rect = Rect::new(2, 2, 4, 1);

    let focus = region(HitTarget::FocusField(1), rect);
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_command_hit_parity(
        context,
        &focus,
        &Action::FocusField(1),
        fork,
        EventHandling::Action(Action::FocusField(1)),
    )
    .unwrap_err();
    assert_eq!(
        error,
        "FOCUS_PARITY_INTERNAL field=1; the typed field branch did not run"
    );

    let toggle = region(HitTarget::ToggleField(0), rect);
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_command_hit_parity(
        context,
        &toggle,
        &Action::ToggleField(0),
        fork,
        EventHandling::Action(Action::ToggleField(0)),
    )
    .unwrap_err();
    assert_eq!(
        error,
        "FIELD_PARITY_INTERNAL; the typed field branch did not run"
    );
}

// --------------------------------------------------------------------------
// field parity refusals
// --------------------------------------------------------------------------

fn choice_field(state: &LibraryState) -> usize {
    state
        .run_form()
        .expect("the fixture presents a launch form")
        .fields()
        .iter()
        .rposition(|field| matches!(&field.control, skit_ui::FormControl::Choice(_)))
        .expect("the fixture form has a choice field")
}

fn checkbox_field(state: &LibraryState) -> usize {
    state
        .run_form()
        .expect("the fixture presents a launch form")
        .fields()
        .iter()
        .position(|field| matches!(&field.control, skit_ui::FormControl::Checkbox { .. }))
        .expect("the fixture form has a checkbox field")
}

/// The field probe refuses a command target.
#[test]
fn refuses_a_command_target_in_the_field_probe() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_field_hit_parity(
        context,
        HitTarget::Command(UiCommand::Help),
        fork,
        EventHandling::Action(Action::OpenHelp),
    )
    .unwrap_err();
    assert_eq!(error, "FIELD_PARITY_TARGET target=Command(Help)");

    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_field_hit_parity(
        context,
        HitTarget::RunFieldCommand {
            field: 0,
            command: RunFieldCommand::ResetDefault,
        },
        fork,
        EventHandling::Action(Action::ResetRunField(0)),
    )
    .unwrap_err();
    assert!(error.starts_with("FIELD_PARITY_TARGET"), "{error}");
}

/// A field hit that no form owns is refused.
#[test]
fn refuses_a_field_hit_that_no_form_owns() {
    let library = library_state();
    let (session, geometry) = frame(&library, NORMAL);
    let forged = only(
        &geometry,
        region(HitTarget::FocusField(0), chip(&geometry, UiCommand::Help)),
    );
    let error =
        check_public_hit_parity(&library, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert_eq!(error, "FIELD_PARITY_OWNER target=FocusField(0) fields=0");

    let form = long_form_state();
    let (session, geometry) = frame(&form, NORMAL);
    let forged = only(
        &geometry,
        region(
            HitTarget::FocusField(99),
            chip(&geometry, UiCommand::SavePreset),
        ),
    );
    let error = check_public_hit_parity(&form, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert_eq!(error, "FIELD_PARITY_OWNER target=FocusField(99) fields=17");

    // The last field index is one less than the field count.
    let forged = only(
        &geometry,
        region(
            HitTarget::FocusField(17),
            chip(&geometry, UiCommand::SavePreset),
        ),
    );
    let error = check_public_hit_parity(&form, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert_eq!(error, "FIELD_PARITY_OWNER target=FocusField(17) fields=17");
}

/// A toggle hit whose click returns another action is refused.
#[test]
fn refuses_a_toggle_hit_that_returns_another_action() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let field = checkbox_field(&state);
    let forged = only(
        &geometry,
        region(
            HitTarget::ToggleField(field),
            chip(&geometry, UiCommand::SavePreset),
        ),
    );
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert!(error.starts_with("FIELD_SESSION_ACTION"), "{error}");
    assert!(
        error.contains(&format!("expected=ToggleField({field})")),
        "{error}"
    );
}

/// A probe step that asks the host is refused.
#[test]
fn refuses_a_probe_step_that_asks_the_host() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let forged = only(
        &geometry,
        region(HitTarget::FocusField(0), chip(&geometry, UiCommand::Reload)),
    );
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert_eq!(
        error,
        "FIELD_SESSION_EFFECT mouse target=FocusField(0) effect=Reload"
    );
}

/// An ignored probe step is refused, and a consumed step is accepted.
#[test]
fn refuses_an_ignored_probe_step() {
    let mut state = library_state();
    assert_eq!(
        apply_probe_handling(&mut state, EventHandling::Consumed, "probe"),
        Ok(())
    );
    assert_eq!(
        apply_probe_handling(&mut state, EventHandling::Action(Action::Next), "probe"),
        Ok(())
    );
    assert_eq!(
        apply_probe_handling(&mut state, EventHandling::Ignored, "probe").unwrap_err(),
        "FIELD_SESSION_IGNORED probe"
    );
    assert_eq!(
        apply_probe_handling(&mut state, EventHandling::Action(Action::Reload), "probe")
            .unwrap_err(),
        "FIELD_SESSION_EFFECT probe effect=Reload"
    );
}

/// A text-focus click that reaches another state is refused.
#[test]
fn refuses_a_text_focus_click_that_moves_another_way() {
    let state = long_form_state();
    let (session, geometry) = frame(&state, NORMAL);
    let forged = only(
        &geometry,
        region(
            HitTarget::FocusField(0),
            chip(&geometry, UiCommand::SavePreset),
        ),
    );
    let error = check_public_hit_parity(&state, &forged, NORMAL, &session, Locale::En).unwrap_err();
    assert!(
        error.starts_with("FIELD_TEXT_FOCUS_STATE field=0"),
        "{error}"
    );
}

/// A text-focus click whose endpoint hides its own target is refused.
#[test]
fn refuses_a_text_focus_click_that_hides_its_own_target() {
    let state = long_form_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, Size::new(1, 1));
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_field_hit_parity(
        context,
        HitTarget::FocusField(0),
        fork,
        EventHandling::Action(Action::FocusField(0)),
    )
    .unwrap_err();
    assert_eq!(error, "FIELD_TEXT_FOCUS_HIDDEN field=0");
}

/// A field target that no focus key reaches is refused.
#[test]
fn refuses_a_field_target_that_no_focus_key_reaches() {
    let mut state = long_form_state();
    assert_eq!(state.update(Action::FocusField(1)), Effect::None);
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_field_hit_parity(
        context,
        HitTarget::ToggleField(0),
        fork,
        EventHandling::Action(Action::ToggleField(0)),
    )
    .unwrap_err();
    assert!(error.starts_with("FIELD_FOCUS_KEY_PATH field=0"), "{error}");
    assert!(error.contains("visible=false"), "{error}");

    // A published target that no focus step reaches keeps the same refusal.
    let typed = typed_controls_state();
    let (session, geometry) = frame(&typed, NORMAL);
    let context = context_for(&session, &typed, &geometry, NORMAL);
    let mut unreachable = typed_controls_state();
    assert_eq!(unreachable.update(Action::OpenHelp), Effect::None);
    let error = check_keyboard_focus_path(context, HitTarget::FocusField(0), 0, 4, &unreachable)
        .unwrap_err();
    assert!(error.starts_with("FIELD_FOCUS_KEY_PATH field=0"), "{error}");
    assert!(error.contains("visible=true"), "{error}");
}

/// A field click that no activation sequence reproduces is refused.
#[test]
fn refuses_a_field_click_that_no_activation_reproduces() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    let fork = session.try_fork().expect("the fixture session forks");
    // The runner picker is published, so the focus keys reach it. No activation from that focused
    // state reproduces a click that changed nothing.
    let error = check_field_hit_parity(
        context,
        HitTarget::FocusField(0),
        fork,
        EventHandling::Consumed,
    )
    .unwrap_err();
    assert!(error.starts_with("FIELD_SESSION_PARITY"), "{error}");
}

/// Every focus and token step must stay inside the frontend.
#[test]
fn refuses_a_focus_step_that_asks_the_host() {
    assert_eq!(
        check_probe_effect("FIELD_FOCUS_EFFECT", 2, &Effect::None),
        Ok(())
    );
    assert_eq!(
        check_probe_effect("FIELD_FOCUS_EFFECT", 2, &Effect::Quit),
        Ok(())
    );
    assert_eq!(
        check_probe_effect("FIELD_FOCUS_EFFECT", 2, &Effect::Reload).unwrap_err(),
        "FIELD_FOCUS_EFFECT field=2 effect=Reload"
    );
    assert_eq!(
        check_probe_effect("RUN_FIELD_FOCUS_EFFECT", 3, &Effect::Reload).unwrap_err(),
        "RUN_FIELD_FOCUS_EFFECT field=3 effect=Reload"
    );
    assert_eq!(
        check_probe_effect("RUN_TOKEN_EFFECT", 4, &Effect::Reload).unwrap_err(),
        "RUN_TOKEN_EFFECT field=4 effect=Reload"
    );
    assert!(!host_effect_pending(&Effect::None));
    assert!(!host_effect_pending(&Effect::Quit));
    assert!(host_effect_pending(&Effect::Reload));
}

// --------------------------------------------------------------------------
// launch-field refusals
// --------------------------------------------------------------------------

/// A browse endpoint that differs on state or on effect is refused.
#[test]
fn refuses_a_browse_endpoint_that_differs() {
    let one = library_state();
    let mut other = library_state();
    assert_eq!(other.update(Action::Next), Effect::None);
    check_browse_endpoint(0, (&one, &Effect::None), (&one, &Effect::None))
        .expect("one identical endpoint is accepted");
    let error =
        check_browse_endpoint(0, (&one, &Effect::None), (&other, &Effect::None)).unwrap_err();
    assert!(error.starts_with("RUN_BROWSE_ENDPOINT field=0"), "{error}");
    let error =
        check_browse_endpoint(1, (&one, &Effect::None), (&one, &Effect::Reload)).unwrap_err();
    assert!(error.starts_with("RUN_BROWSE_ENDPOINT field=1"), "{error}");
}

/// A browse probe without the capability is refused.
#[test]
fn refuses_a_browse_path_without_the_capability() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let error = browse_keyboard_action(&session, &state, &geometry, 0).unwrap_err();
    assert_eq!(error, "RUN_BROWSE_CAPABILITY field=0");
}

/// A token menu that another field owns is refused.
#[test]
fn refuses_a_token_menu_that_another_field_owns() {
    let state = prompt_run_state();
    let (session, geometry) = frame(&state, NORMAL);
    let error = browse_keyboard_action(&session, &state, &geometry, 0).unwrap_err();
    assert!(error.starts_with("RUN_TOKEN_OWNER field=0"), "{error}");
}

/// The file entry is first for a path field and last for every other field.
#[test]
fn refuses_a_token_list_whose_file_entry_moved() {
    let first = [
        RunTokenOption::FileOrFolder,
        RunTokenOption::Today,
        RunTokenOption::Now,
    ];
    assert_eq!(token_file_index(0, &first), Ok(0));
    let last = [
        RunTokenOption::Today,
        RunTokenOption::Now,
        RunTokenOption::FileOrFolder,
    ];
    assert_eq!(token_file_index(0, &last), Ok(2));
    let middle = [
        RunTokenOption::Today,
        RunTokenOption::FileOrFolder,
        RunTokenOption::Now,
    ];
    assert_eq!(
        token_file_index(2, &middle).unwrap_err(),
        "RUN_TOKEN_FILE_POSITION field=2 index=1 options=3"
    );
    let missing = [RunTokenOption::Today, RunTokenOption::Now];
    assert!(
        token_file_index(3, &missing)
            .unwrap_err()
            .starts_with("RUN_TOKEN_FILE_OPTION field=3")
    );
}

/// A list that does not take the End key is refused.
#[test]
fn refuses_a_token_list_that_does_not_take_the_end_key() {
    let state = library_state();
    let (mut session, geometry) = frame(&state, NORMAL);
    let error = select_last_token(&mut session, &state, &geometry, 5).unwrap_err();
    assert!(error.starts_with("RUN_TOKEN_END field=5"), "{error}");
}

/// A browse key path that reaches another action is refused.
#[test]
fn refuses_a_browse_key_path_that_reaches_another_action() {
    let state = prompt_run_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    check_run_field_hit(
        context,
        1,
        RunFieldCommand::BrowsePath,
        &Action::OpenRunFilePicker(1),
    )
    .expect("the advertised browse path matches the pointer path");
    let error = check_run_field_hit(
        context,
        1,
        RunFieldCommand::BrowsePath,
        &Action::OpenRunFilePicker(9),
    )
    .unwrap_err();
    assert!(error.starts_with("RUN_BROWSE_PARITY field=1"), "{error}");
}

// --------------------------------------------------------------------------
// session and command refusals
// --------------------------------------------------------------------------

/// An event that returns no action is refused.
#[test]
fn refuses_an_event_that_returns_no_action() {
    let state = library_state();
    let (mut session, geometry) = frame(&state, NORMAL);
    let error = session_action(
        &mut session,
        Event::Key(KeyEvent::new(KeyCode::F(9), KeyModifiers::NONE)),
        &state,
        &geometry,
    )
    .unwrap_err();
    assert!(error.starts_with("SESSION_ACTION"), "{error}");
}

/// A command key path that never reaches the endpoint is refused.
#[test]
fn refuses_a_command_key_path_that_misses_the_endpoint() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let bindings = public_command_bindings(&session, &state, UiCommand::Help)
        .expect("Help is advertised on the Library screen");
    assert_eq!(
        command_key_action(
            &session,
            &state,
            &geometry,
            UiCommand::Help,
            &bindings,
            &Action::OpenHelp,
            0,
        ),
        Ok(Action::OpenHelp)
    );
    let error = command_key_action(
        &session,
        &state,
        &geometry,
        UiCommand::Help,
        &bindings,
        &Action::Quit,
        0,
    )
    .unwrap_err();
    assert!(
        error.starts_with("COMMAND_KEY_PATH command=Help"),
        "{error}"
    );
    // An endpoint that shares the effect but not the state is still a miss.
    let error = command_key_action(
        &session,
        &state,
        &geometry,
        UiCommand::Help,
        &bindings,
        &Action::Next,
        0,
    )
    .unwrap_err();
    assert!(
        error.starts_with("COMMAND_KEY_PATH command=Help"),
        "{error}"
    );
    assert!(error.contains("action=OpenHelp effect=None"), "{error}");

    // An open modal takes no focus key, so every prefix beyond zero ends at its first step.
    let mut modal = library_state();
    assert_eq!(modal.update(Action::OpenHelp), Effect::None);
    let (modal_session, modal_geometry) = frame(&modal, NORMAL);
    let modal_bindings = public_command_bindings(&modal_session, &modal, UiCommand::CloseModal)
        .expect("an open modal advertises its close command");
    let error = command_key_action(
        &modal_session,
        &modal,
        &modal_geometry,
        UiCommand::CloseModal,
        &modal_bindings,
        &Action::Quit,
        2,
    )
    .unwrap_err();
    assert!(
        error.starts_with("COMMAND_KEY_PATH command=CloseModal"),
        "{error}"
    );
    assert!(error.contains("focus_ignored step=0"), "{error}");
    let error = command_key_action(
        &session,
        &state,
        &geometry,
        UiCommand::Help,
        &[],
        &Action::OpenHelp,
        0,
    )
    .unwrap_err();
    assert_eq!(error, "visible command Help has no key binding");

    // A chord that the screen does not take reaches no command.
    let unbound = [binding(UiKey::Function(9), UiModifiers::NONE)];
    let error = command_key_action(
        &session,
        &state,
        &geometry,
        UiCommand::Help,
        &unbound,
        &Action::OpenHelp,
        0,
    )
    .unwrap_err();
    assert!(
        error.starts_with("COMMAND_KEY_PATH command=Help"),
        "{error}"
    );
    assert!(error.contains("handling=Ignored"), "{error}");
}

/// A focus step must stay inside the frontend and must move the focus ring.
#[test]
fn a_focus_prefix_step_stays_in_the_frontend() {
    let mut state = typed_controls_state();
    assert_eq!(
        focus_prefix_failure(&mut state, EventHandling::Action(Action::FocusNext), 0),
        None
    );
    assert_eq!(
        focus_prefix_failure(&mut state, EventHandling::Consumed, 1),
        None
    );
    assert_eq!(
        focus_prefix_failure(&mut state, EventHandling::Action(Action::Reload), 2),
        Some("focus_effect step=2 effect=Reload".to_owned())
    );
    assert_eq!(
        focus_prefix_failure(&mut state, EventHandling::Ignored, 3),
        Some("focus_ignored step=3".to_owned())
    );
}

/// A shared command endpoint that differs between the two paths is refused.
#[test]
fn refuses_a_command_endpoint_that_differs() {
    let state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    let bindings = public_command_bindings(&session, &state, UiCommand::FocusNext)
        .expect("the launch form advertises the focus command");
    let fork = session.try_fork().expect("the fixture session forks");
    check_command_key_endpoint(context, &bindings, &Action::FocusNext, fork)
        .expect("the advertised key reaches the pointer endpoint");
    let fork = session.try_fork().expect("the fixture session forks");
    let error =
        check_command_key_endpoint(context, &bindings, &Action::FocusPrevious, fork).unwrap_err();
    assert!(error.starts_with("COMMAND_ENDPOINT"), "{error}");
}

/// A command click that asks the host is refused.
#[test]
fn refuses_a_command_click_that_asks_the_host() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let context = context_for(&session, &state, &geometry, NORMAL);
    let bindings = public_command_bindings(&session, &state, UiCommand::Reload)
        .expect("Reload is advertised on the Library screen");
    let fork = session.try_fork().expect("the fixture session forks");
    let error = check_command_key_endpoint(context, &bindings, &Action::Reload, fork).unwrap_err();
    assert_eq!(error, "COMMAND_MOUSE_EFFECT effect=Reload");
}

/// A command that this context hides has no advertised binding.
#[test]
fn command_bindings_refuse_a_command_this_context_hides() {
    let state = library_state();
    let (session, _) = frame(&state, NORMAL);
    assert_eq!(
        command_binding(&state, UiCommand::Help).map(|binding| binding.key),
        Ok(UiKey::Character('?'))
    );
    let error = command_bindings(&state, UiCommand::SavePreferences).unwrap_err();
    assert_eq!(
        error,
        "visible command SavePreferences is not advertised in this context"
    );
    let error = public_command_bindings(&session, &state, UiCommand::SavePreferences).unwrap_err();
    assert_eq!(
        error,
        "visible command SavePreferences has no printed key binding"
    );
    // The footer does not print Previous, so the shared registry answers for it.
    let registry = public_command_bindings(&session, &state, UiCommand::Previous)
        .expect("the shared registry advertises the selection keys");
    assert_eq!(
        registry,
        command_bindings(&state, UiCommand::Previous).expect("Previous is advertised here")
    );
    assert!(
        session
            .advertised_command_bindings(&state, UiCommand::Previous)
            .is_empty()
    );
}

// --------------------------------------------------------------------------
// local dispatch refusals
// --------------------------------------------------------------------------

/// An event that the advertised chip does not carry is refused.
#[test]
fn refuses_a_local_event_that_the_chip_does_not_carry() {
    let live = live_local_action();
    let advertised_key = live.keys[0].event();
    check_local_event(&Event::Key(advertised_key), &live).expect("the advertised key is accepted");

    let other_code = KeyEvent::new(KeyCode::F(9), advertised_key.modifiers);
    assert!(
        check_local_event(&Event::Key(other_code), &live)
            .unwrap_err()
            .starts_with("LOCAL_ACTION_KEY")
    );
    let other_modifiers = KeyEvent::new(advertised_key.code, KeyModifiers::ALT);
    assert!(
        check_local_event(&Event::Key(other_modifiers), &live)
            .unwrap_err()
            .starts_with("LOCAL_ACTION_KEY")
    );
    let mut other_kind = advertised_key;
    other_kind.kind = ratatui_crossterm::crossterm::event::KeyEventKind::Release;
    assert!(
        check_local_event(&Event::Key(other_kind), &live)
            .unwrap_err()
            .starts_with("LOCAL_ACTION_KEY")
    );

    let rect = live.hit.expect("the fixture chip has a hit");
    let inside = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x,
        row: rect.y,
        modifiers: KeyModifiers::NONE,
    });
    check_local_event(&inside, &live).expect("a press inside the chip is accepted");
    let outside = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: u16::MAX,
        row: u16::MAX,
        modifiers: KeyModifiers::NONE,
    });
    assert!(
        check_local_event(&outside, &live)
            .unwrap_err()
            .starts_with("LOCAL_ACTION_RECT")
    );
    let mut hidden = live.clone();
    hidden.hit = None;
    assert!(
        check_local_event(&inside, &hidden)
            .unwrap_err()
            .starts_with("LOCAL_ACTION_RECT")
    );
    assert!(
        check_local_event(&Event::Paste("value".to_owned()), &live)
            .unwrap_err()
            .starts_with("LOCAL_ACTION_EVENT")
    );
}

/// A local press that the session does not consume is refused.
#[test]
fn refuses_a_local_press_that_the_session_does_not_consume() {
    let live = live_local_action();
    let press = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 4,
        row: 4,
        modifiers: KeyModifiers::NONE,
    });
    check_local_press(&press, &live, &EventHandling::Consumed).expect("an armed press is accepted");
    let error = check_local_press(&press, &live, &EventHandling::Ignored).unwrap_err();
    assert!(error.starts_with("LOCAL_ACTION_PRESS"), "{error}");
}

/// A live local endpoint that differs from the advertised outcome is refused.
#[test]
fn refuses_a_local_endpoint_that_differs_from_the_descriptor() {
    let event = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let action = advertised(
        LocalActionTarget::Health(HealthAction::Next),
        LocalActionOutcome::Action(Action::Reload),
    );
    let consumed = advertised(
        LocalActionTarget::Health(HealthAction::Next),
        LocalActionOutcome::Consumed,
    );
    check_local_endpoint(&event, &action, &EventHandling::Action(Action::Reload))
        .expect("the advertised action is accepted");
    check_local_endpoint(&event, &consumed, &EventHandling::Consumed)
        .expect("the advertised consumption is accepted");
    for handling in [
        EventHandling::Action(Action::Quit),
        EventHandling::Consumed,
        EventHandling::Ignored,
    ] {
        assert!(
            check_local_endpoint(&event, &action, &handling)
                .unwrap_err()
                .starts_with("LOCAL_ACTION_ENDPOINT")
        );
    }
    for handling in [EventHandling::Action(Action::Quit), EventHandling::Ignored] {
        assert!(
            check_local_endpoint(&event, &consumed, &handling)
                .unwrap_err()
                .starts_with("LOCAL_ACTION_ENDPOINT")
        );
    }
}

/// A semantic click needs a primary press.
#[test]
fn refuses_a_click_event_that_is_not_a_primary_press() {
    let down = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 3,
        row: 4,
        modifiers: KeyModifiers::NONE,
    });
    let (press, release) = primary_click_events(down.clone()).expect("a primary press has a pair");
    assert_eq!(press, down);
    assert_eq!(
        release,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 3,
            row: 4,
            modifiers: KeyModifiers::NONE,
        })
    );
    let right = Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Right),
        column: 3,
        row: 4,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(matching_primary_release(&right), None);
    let key = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(matching_primary_release(&key), None);
    let error = primary_click_events(key).unwrap_err();
    assert!(error.starts_with("SEMANTIC_CLICK_EVENT"), "{error}");
}

// --------------------------------------------------------------------------
// tables and pure contracts
// --------------------------------------------------------------------------

/// Each field target carries its exact bounded activation sequences.
#[test]
fn field_activation_sequences_are_exact_for_every_target() {
    let state = typed_controls_state();
    let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
    assert_eq!(
        field_activation_sequences(&state, HitTarget::ToggleField(0)),
        vec![vec![key(KeyCode::Char(' '))]]
    );
    let choice = choice_field(&state);
    assert_eq!(
        field_activation_sequences(
            &state,
            HitTarget::SelectFieldOption {
                field: choice,
                option: 0
            }
        ),
        vec![
            vec![key(KeyCode::Right)],
            vec![key(KeyCode::Left)],
            vec![key(KeyCode::Right); 2],
            vec![key(KeyCode::Left); 2],
            vec![key(KeyCode::Right); 3],
            vec![key(KeyCode::Left); 3],
            vec![key(KeyCode::Right), key(KeyCode::Left)],
            vec![key(KeyCode::Left), key(KeyCode::Right)],
        ]
    );
    assert_eq!(
        field_activation_sequences(
            &state,
            HitTarget::SelectFieldOption {
                field: checkbox_field(&state),
                option: 0
            }
        ),
        vec![
            vec![key(KeyCode::Right)],
            vec![key(KeyCode::Left)],
            vec![key(KeyCode::Right), key(KeyCode::Left)],
            vec![key(KeyCode::Left), key(KeyCode::Right)],
        ]
    );
    assert_eq!(
        field_activation_sequences(&state, HitTarget::FocusField(0)),
        vec![
            vec![key(KeyCode::Esc)],
            vec![key(KeyCode::Enter)],
            vec![key(KeyCode::Char(' '))],
            vec![key(KeyCode::Down)],
            vec![key(KeyCode::Up)],
            vec![key(KeyCode::Right)],
            vec![key(KeyCode::Left)],
        ]
    );
    assert!(field_activation_sequences(&state, HitTarget::Command(UiCommand::Help)).is_empty());
    assert!(
        field_activation_sequences(
            &state,
            HitTarget::RunFieldCommand {
                field: 0,
                command: RunFieldCommand::InsertValue
            }
        )
        .is_empty()
    );
}

/// A plain focus move names its own field and carries no other action.
#[test]
fn a_plain_focus_result_names_its_own_field() {
    assert!(is_plain_focus_handling(
        &EventHandling::Action(Action::FocusField(2)),
        2
    ));
    assert!(!is_plain_focus_handling(
        &EventHandling::Action(Action::FocusField(3)),
        2
    ));
    assert!(!is_plain_focus_handling(
        &EventHandling::Action(Action::ToggleField(2)),
        2
    ));
    assert!(!is_plain_focus_handling(&EventHandling::Consumed, 2));
}

/// A plain focus field takes focus and nothing else.
#[test]
fn plain_focus_fields_are_text_fields_and_form_rows() {
    let run = prompt_run_state();
    assert!(is_plain_focus_field(&run, 1));
    assert!(!is_plain_focus_field(&run, 0));
    let typed = typed_controls_state();
    assert!(!is_plain_focus_field(&typed, choice_field(&typed)));
    assert!(!is_plain_focus_field(&typed, 99));
    let form = rename_form_state();
    assert!(is_plain_focus_field(&form, 0));
    assert!(!is_plain_focus_field(&form, 1));
    assert!(!is_plain_focus_field(&library_state(), 0));
}

/// The probe presses the field corner and the chip centre.
#[test]
fn probe_points_use_the_field_corner_and_the_chip_centre() {
    assert_eq!(
        public_hit_probe_point(&region(HitTarget::FocusField(2), Rect::new(4, 6, 10, 3))),
        (4, 6)
    );
    assert_eq!(
        public_hit_probe_point(&region(
            HitTarget::Command(UiCommand::Help),
            Rect::new(4, 6, 10, 3)
        )),
        (9, 7)
    );
}

/// Each launch-field chip names one command and one action.
#[test]
fn launch_field_chips_map_to_their_command_and_action() {
    assert_eq!(
        run_field_ui_command(RunFieldCommand::BrowsePath),
        UiCommand::BrowsePath
    );
    assert_eq!(
        run_field_ui_command(RunFieldCommand::InsertValue),
        UiCommand::InsertValue
    );
    assert_eq!(
        run_field_ui_command(RunFieldCommand::ResetDefault),
        UiCommand::ResetDefault
    );
    assert_eq!(
        run_field_action(1, RunFieldCommand::BrowsePath),
        Action::OpenRunFilePicker(1)
    );
    assert_eq!(
        run_field_action(2, RunFieldCommand::InsertValue),
        Action::OpenRunTokenMenuFor(2)
    );
    assert_eq!(
        run_field_action(3, RunFieldCommand::ResetDefault),
        Action::ResetRunField(3)
    );
}

/// Every advertised key maps to its exact terminal event.
#[test]
fn every_advertised_key_maps_to_its_terminal_event() {
    let cases = [
        (UiKey::Character('x'), KeyCode::Char('x')),
        (UiKey::Enter, KeyCode::Enter),
        (UiKey::Escape, KeyCode::Esc),
        (UiKey::Delete, KeyCode::Delete),
        (UiKey::Backspace, KeyCode::Backspace),
        (UiKey::Tab, KeyCode::Tab),
        (UiKey::BackTab, KeyCode::BackTab),
        (UiKey::Up, KeyCode::Up),
        (UiKey::Down, KeyCode::Down),
        (UiKey::PageUp, KeyCode::PageUp),
        (UiKey::PageDown, KeyCode::PageDown),
        (UiKey::Home, KeyCode::Home),
        (UiKey::End, KeyCode::End),
        (UiKey::Function(5), KeyCode::F(5)),
    ];
    for (key, code) in cases {
        assert_eq!(
            binding_event(binding(key, UiModifiers::NONE)),
            KeyEvent::new(code, KeyModifiers::NONE)
        );
    }
    assert_eq!(
        binding_event(binding(UiKey::Enter, UiModifiers::CONTROL)),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)
    );
    assert_eq!(
        binding_event(binding(UiKey::Enter, UiModifiers::SHIFT)),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)
    );
    assert_eq!(
        binding_event(binding(
            UiKey::Enter,
            UiModifiers {
                control: false,
                alt: true,
                shift: false,
            }
        )),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
    );
}

/// A published target needs a visible rect and the exact action.
#[test]
fn published_targets_need_a_visible_rect_and_the_exact_action() {
    let target = HitTarget::Command(UiCommand::Help);
    let visible = ViewGeometry {
        hits: vec![region(target, Rect::new(1, 1, 4, 1))],
        ..ViewGeometry::default()
    };
    assert!(publishes_target(&visible, target));
    let flat = ViewGeometry {
        hits: vec![region(target, Rect::new(1, 1, 4, 0))],
        ..ViewGeometry::default()
    };
    assert!(!publishes_target(&flat, target));
    let thin = ViewGeometry {
        hits: vec![region(target, Rect::new(1, 1, 0, 1))],
        ..ViewGeometry::default()
    };
    assert!(!publishes_target(&thin, target));
    let other = ViewGeometry {
        hits: vec![region(
            HitTarget::Command(UiCommand::Quit),
            Rect::new(1, 1, 4, 1),
        )],
        ..ViewGeometry::default()
    };
    assert!(!publishes_target(&other, target));
}

/// The public hit contract names one action for each published target.
#[test]
fn expected_hit_action_is_the_public_hit_contract() {
    let state = typed_controls_state();
    let (_, geometry) = frame(&state, NORMAL);
    let detail = ViewGeometry {
        detail_pane_visible: true,
        ..geometry.clone()
    };
    assert_eq!(
        expected_hit_action(HitTarget::Command(UiCommand::ToggleDetail), &detail, &state),
        Ok(Action::ToggleDetail {
            currently_visible: true
        })
    );
    let hidden = ViewGeometry {
        detail_pane_visible: false,
        ..geometry.clone()
    };
    assert_eq!(
        expected_hit_action(HitTarget::Command(UiCommand::ToggleDetail), &hidden, &state),
        Ok(Action::ToggleDetail {
            currently_visible: false
        })
    );
    assert_eq!(
        expected_hit_action(HitTarget::Command(UiCommand::Help), &geometry, &state),
        Ok(Action::OpenHelp)
    );
    assert_eq!(
        expected_hit_action(
            HitTarget::RunFieldCommand {
                field: 1,
                command: RunFieldCommand::ResetDefault
            },
            &geometry,
            &state
        ),
        Ok(Action::ResetRunField(1))
    );
    assert_eq!(
        expected_hit_action(HitTarget::FocusField(1), &geometry, &state),
        Ok(Action::FocusField(1))
    );
    assert_eq!(
        expected_hit_action(HitTarget::ToggleField(1), &geometry, &state),
        Ok(Action::ToggleField(1))
    );
    let choice = choice_field(&state);
    assert_eq!(
        expected_hit_action(
            HitTarget::SelectFieldOption {
                field: choice,
                option: 0
            },
            &geometry,
            &state
        ),
        Ok(Action::SelectFieldOption {
            field: choice,
            value: "json".to_owned()
        })
    );
    let checkbox = checkbox_field(&state);
    assert!(
        expected_hit_action(
            HitTarget::SelectFieldOption {
                field: checkbox,
                option: 0
            },
            &geometry,
            &state
        )
        .unwrap_err()
        .starts_with("RUN_OPTION_HIT")
    );
}

/// One complete primary click arms the target and then activates it.
#[test]
fn a_primary_click_arms_the_target_before_it_activates() {
    let state = library_state();
    let (session, geometry) = frame(&state, NORMAL);
    let help = chip(&geometry, UiCommand::Help);
    let mut fork = session.try_fork().expect("the fixture session forks");
    let (column, row) = public_hit_probe_point(&region(HitTarget::Command(UiCommand::Help), help));
    assert_eq!(
        session_primary_click(&mut fork, &state, &geometry, column, row),
        Ok(EventHandling::Action(Action::OpenHelp))
    );
    let mut fork = session.try_fork().expect("the fixture session forks");
    let error = session_primary_click(&mut fork, &state, &geometry, 200, 200).unwrap_err();
    assert!(error.starts_with("SESSION_CLICK_PRESS"), "{error}");
}

/// One probe endpoint carries the state, the geometry and the drawn frame.
#[test]
fn a_probe_endpoint_carries_the_state_geometry_and_frame() {
    let state = library_state();
    let (session, _) = frame(&state, NORMAL);
    let mut fork = session.try_fork().expect("the fixture session forks");
    let endpoint: ProbeEndpoint = render_probe_endpoint(&mut fork, &state, Locale::En, NORMAL)
        .expect("the probe frame renders");
    assert_eq!(endpoint.state, state);
    assert!(!endpoint.geometry.hits.is_empty());
    assert_eq!(endpoint.backend.buffer().area.as_size(), NORMAL);
    let same = render_probe_endpoint(&mut fork, &state, Locale::En, NORMAL)
        .expect("the probe frame renders again");
    assert_eq!(same.geometry, endpoint.geometry);
    let field_hit_rect = field_hit(
        &frame(&long_form_state(), NORMAL).1,
        HitTarget::FocusField(0),
    );
    assert!(field_hit_rect.width > 0);
}

/// A text field keeps its parity after a pointer click focused another field.
///
/// Every other field probe starts from a pristine session. This one starts from a session that
/// already carries one click. The click registry, the caret and the scroll history of that click
/// must not change the endpoint that the focus keys reach for a later text field.
#[test]
fn a_text_field_keeps_its_parity_after_a_click_on_another_field() {
    let mut state = typed_controls_state();
    let (session, geometry) = frame(&state, NORMAL);
    let first = field_hit(&geometry, HitTarget::FocusField(0));
    let mut base = session.try_fork().expect("the fixture session forks");
    assert_eq!(
        session_primary_click(&mut base, &state, &geometry, first.x, first.y),
        Ok(EventHandling::Action(Action::FocusField(0)))
    );
    assert_eq!(state.update(Action::FocusField(0)), Effect::None);
    assert_eq!(state.focused_form_field(), Some(0));

    let geometry = render_probe_endpoint(&mut base, &state, Locale::En, NORMAL)
        .expect("the clicked frame renders")
        .geometry;
    let mut probe = base.try_fork().expect("the clicked session forks");
    assert_eq!(
        probe.handle_event(
            Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            &state,
            &geometry,
        ),
        EventHandling::Action(Action::FocusField(1)),
        "Tab must leave the clicked field for the next one"
    );

    assert!(
        is_plain_focus_field(&state, 3),
        "field 3 is the text field of this fixture"
    );
    let context = context_for(&base, &state, &geometry, NORMAL);
    let fork = base.try_fork().expect("the clicked session forks again");
    assert_eq!(
        check_field_hit_parity(
            context,
            HitTarget::FocusField(3),
            fork,
            EventHandling::Action(Action::FocusField(3)),
        ),
        Ok(())
    );
}
