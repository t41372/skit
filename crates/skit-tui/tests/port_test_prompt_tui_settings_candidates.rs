use ratatui_core::{backend::TestBackend, terminal::Terminal};
use ratatui_crossterm::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_i18n::Locale;
use skit_tui::{
    EventHandling, SettingsControlId, SettingsScreenEvent, SettingsScreenGeometry,
    SettingsScreenSession, TuiSession, ViewGeometry, render_settings, render_with_session,
};
use skit_ui::{
    Action, FieldValue, INTERPOLATE_KEY, LibraryState, MANAGE_KEY, Screen, SettingsAction,
    SettingsInputs, SettingsView, TypedValue,
};

fn managed(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Placeholder;
    declaration
}

fn prompt_view(interpolate: bool, managed_names: &[&str], candidates: Vec<String>) -> SettingsView {
    SettingsView::from_inputs(&SettingsInputs {
        selector: "p".to_owned(),
        kind: "prompt".to_owned(),
        name: "p".to_owned(),
        source: "/work/p.prompt.md".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        // Prompt parameters are metadata-declared rows in the Rust architecture, matching the
        // `DeclParamRow` surface in the frozen Textual test. Candidate offers still have to coexist
        // with those rows; setting this false just to make MANAGE_KEY appear would hide the bug.
        declared_schema: true,
        interpolate,
        managed: managed_names.iter().map(|name| managed(name)).collect(),
        candidates,
        configured_runners: vec!["claude".to_owned()],
        ..SettingsInputs::default()
    })
}

fn draw_settings(
    session: &mut SettingsScreenSession,
    view: &SettingsView,
    width: u16,
    height: u16,
) -> SettingsScreenGeometry {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut geometry = SettingsScreenGeometry::default();
    terminal
        .draw(|frame| {
            geometry = render_settings(frame, frame.area(), view, session, Locale::En);
        })
        .unwrap();
    geometry
}

fn dispatch(
    session: &mut SettingsScreenSession,
    view: &mut SettingsView,
    geometry: &SettingsScreenGeometry,
    event: Event,
) -> Option<SettingsScreenEvent> {
    let handled = session.handle_event(event, view, geometry);
    if let Some(SettingsScreenEvent::Action(action)) = handled.clone() {
        let _ = view.update(action);
    }
    handled
}

fn mouse(area: ratatui_core::layout::Rect) -> Event {
    Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: area.x,
        row: area.y,
        modifiers: KeyModifiers::NONE,
    })
}

fn candidate_hit<'a>(geometry: &'a SettingsScreenGeometry, name: &str) -> &'a skit_tui::SettingsHitRegion {
    geometry
        .hits
        .iter()
        .find(|hit| {
            hit.target
                == SettingsControlId::Option {
                    field: MANAGE_KEY.to_owned(),
                    value: name.to_owned(),
                }
        })
        .unwrap_or_else(|| panic!("prompt settings did not expose detected placeholder {name:?} as a candidate control"))
}

#[test]
fn test_settings_tick_to_manage_a_detected_placeholder() {
    let mut view = prompt_view(true, &["a"], vec!["b".to_owned()]);
    let mut session = SettingsScreenSession::default();
    let geometry = draw_settings(&mut session, &view, 110, 80);
    let hit = candidate_hit(&geometry, "b");
    assert!(matches!(dispatch(&mut session, &mut view, &geometry, mouse(hit.area)), Some(SettingsScreenEvent::Action(SettingsAction::SetField { .. }))));
    assert_eq!(
        view.submitted_values().get(MANAGE_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choices(vec!["b".to_owned()])))
    );
    assert!(
        !view.submitted_values().contains_key("parameter:a:keep"),
        "managing b spuriously rewrote the already-managed a row"
    );
}

#[test]
fn test_settings_unticking_a_row_unmanages_it() {
    let mut view = prompt_view(true, &["a", "b"], Vec::new());
    assert!(view.focus("parameter:a:keep"));
    let mut session = SettingsScreenSession::default();
    let geometry = draw_settings(&mut session, &view, 110, 80);
    assert!(matches!(
        dispatch(
            &mut session,
            &mut view,
            &geometry,
            Event::Key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE)),
        ),
        Some(SettingsScreenEvent::Action(SettingsAction::SetField { .. }))
    ));
    let values = view.submitted_values();
    assert_eq!(values.get("parameter:a:keep"), Some(&FieldValue::boolean(false)));
    assert!(!values.contains_key("parameter:b:keep"), "unticking a also changed b");
}

#[test]
fn test_settings_off_to_on_can_choose_first_parameters_in_the_same_save() {
    let mut view = prompt_view(false, &[], vec!["a".to_owned(), "b".to_owned()]);
    let mut session = SettingsScreenSession::default();
    let off = draw_settings(&mut session, &view, 110, 80);
    assert!(
        !off.hits.iter().any(|hit| matches!(&hit.target, SettingsControlId::Option { field, .. } if field == MANAGE_KEY)),
        "insertion-off prompt still offered placeholder candidate controls"
    );

    assert!(view.set_value(INTERPOLATE_KEY, FieldValue::boolean(true)));
    let on = draw_settings(&mut session, &view, 110, 80);
    let hit = candidate_hit(&on, "b");
    dispatch(&mut session, &mut view, &on, mouse(hit.area));
    let values = view.submitted_values();
    assert_eq!(values.get(INTERPOLATE_KEY), Some(&FieldValue::boolean(true)));
    assert_eq!(
        values.get(MANAGE_KEY),
        Some(&FieldValue::Explicit(TypedValue::Choices(vec!["b".to_owned()])))
    );
}

#[test]
fn test_settings_candidate_checkboxes_are_flood_capped() {
    let candidates = (0..29).map(|index| format!("u{index}")).collect::<Vec<_>>();
    let view = prompt_view(true, &["a"], candidates);
    let mut session = SettingsScreenSession::default();
    let geometry = draw_settings(&mut session, &view, 120, 140);
    let inline = geometry
        .hits
        .iter()
        .filter(|hit| matches!(&hit.target, SettingsControlId::Option { field, .. } if field == MANAGE_KEY))
        .count();
    assert_eq!(inline, 20, "frozen Prompt-TUI inline candidate preview is capped at 20, got {inline}");
}

#[test]
fn test_settings_choose_variables_key_is_harmless_when_off_or_short() {
    for view in [
        prompt_view(false, &["a"], vec!["b".to_owned(), "c".to_owned()]),
        prompt_view(true, &["a"], vec!["b".to_owned(), "c".to_owned()]),
    ] {
        let mut state = LibraryState::default();
        state.update(Action::Present(Screen::Settings(Box::new(view))));
        let mut session = TuiSession::default();
        let mut terminal = Terminal::new(TestBackend::new(110, 40)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();
        assert_eq!(
            session.handle_event(
                Event::Key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
                &state,
                &geometry,
            ),
            EventHandling::Ignored,
            "Ctrl+O opened or mutated a choose-variables surface when it should be harmless"
        );
        assert!(state.modal().is_none());
        assert!(matches!(state.screen(), Screen::Settings(_)));
    }
}
