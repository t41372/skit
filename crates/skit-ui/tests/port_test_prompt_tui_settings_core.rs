use skit_domain::parameters::{ParamDecl, ParameterDelivery};
use skit_ui::{
    ADD_PARAMETER_KEY, INTERPOLATE_KEY, RUNNER_KEY, FieldKind, FieldValue, SettingsInputs,
    SettingsView, TypedValue,
};

fn prompt_inputs() -> SettingsInputs {
    SettingsInputs {
        selector: "p".to_owned(),
        kind: "prompt".to_owned(),
        name: "p".to_owned(),
        source: "/work/p.prompt.md".to_owned(),
        workdir: "invoke".to_owned(),
        supports_modes: true,
        has_original_file: true,
        has_stored_name: true,
        declared_schema: true,
        interpolate: true,
        configured_runners: vec!["claude".to_owned(), "codex".to_owned()],
        ..SettingsInputs::default()
    }
}

fn managed(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Placeholder;
    declaration
}

fn choice(value: &str) -> FieldValue {
    FieldValue::Explicit(TypedValue::Choice(value.to_owned()))
}

#[test]
fn test_settings_prompt_rows_and_no_flag_input() {
    let mut api = managed("api_key");
    api.secret = true;
    let view = SettingsView::from_inputs(&SettingsInputs {
        managed: vec![managed("a"), api],
        ..prompt_inputs()
    });

    let keys = view.fields().map(|field| field.key.as_str()).collect::<Vec<_>>();
    assert!(keys.iter().any(|key| *key == "parameter:a:keep"));
    assert!(keys.iter().any(|key| *key == "parameter:api_key:keep"));
    assert_eq!(
        view.field("parameter:api_key:secret").unwrap().value().as_text(),
        "true"
    );
    assert!(
        !keys.iter().any(|key| key.ends_with(":flag")),
        "placeholder kinds must never grow argv flag inputs: {keys:?}"
    );
}

#[test]
fn test_settings_runner_radio_pins_and_clears() {
    let mut view = SettingsView::from_inputs(&prompt_inputs());
    let runner = view.field(RUNNER_KEY).unwrap();
    assert_eq!(runner.value().as_text(), "");
    let FieldKind::SingleChoice { options } = &runner.kind else {
        panic!("prompt runner is not a closed picker");
    };
    assert_eq!(
        options.iter().map(|option| option.value.as_str()).collect::<Vec<_>>(),
        ["", "claude", "codex"]
    );

    assert!(view.set_value(RUNNER_KEY, choice("claude")));
    assert_eq!(view.submitted_values().get(RUNNER_KEY), Some(&choice("claude")));

    // A fresh screen opened on the persisted pin must preselect it; clearing back to ask is an
    // explicit typed choice, not an absent edit.
    let mut pinned = SettingsView::from_inputs(&SettingsInputs {
        runner: "claude".to_owned(),
        ..prompt_inputs()
    });
    assert_eq!(pinned.field(RUNNER_KEY).unwrap().value().as_text(), "claude");
    assert!(pinned.set_value(RUNNER_KEY, choice("")));
    assert_eq!(pinned.submitted_values().get(RUNNER_KEY), Some(&choice("")));
}

#[test]
fn test_settings_runner_section_empty_config_keeps_ask_and_the_door() {
    let view = SettingsView::from_inputs(&SettingsInputs {
        configured_runners: Vec::new(),
        ..prompt_inputs()
    });
    let field = view.field(RUNNER_KEY).unwrap();
    assert_eq!(field.value().as_text(), "");
    let FieldKind::SingleChoice { options } = &field.kind else {
        panic!("runner field is not a closed picker");
    };
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].value, "");
    assert!(field.capabilities.new_runner, "the New agent door disappeared with an empty runner list");
    assert!(view.submitted_values().is_empty(), "saving the lone ask option is not an edit");
}

#[test]
fn test_settings_ctrl_n_add_preserves_a_stale_pin_option() {
    let mut view = SettingsView::from_inputs(&SettingsInputs {
        runner: "gone".to_owned(),
        configured_runners: vec!["other".to_owned()],
        ..prompt_inputs()
    });
    assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "gone");
    view.add_and_select_runner("p", "fresh".to_owned());
    let field = view.field(RUNNER_KEY).unwrap();
    assert_eq!(field.value().as_text(), "fresh");
    let FieldKind::SingleChoice { options } = &field.kind else {
        panic!("runner field is not a closed picker");
    };
    assert_eq!(
        options.iter().map(|option| option.value.as_str()).collect::<Vec<_>>(),
        ["", "gone", "other", "fresh"],
        "mid-session agent add dropped or reordered the stale pin option"
    );
}

#[test]
fn test_settings_pin_change_saves_even_with_insertion_off() {
    let mut view = SettingsView::from_inputs(&SettingsInputs {
        interpolate: false,
        managed: vec![managed("a")],
        ..prompt_inputs()
    });
    assert_eq!(view.field(INTERPOLATE_KEY).unwrap().value().as_text(), "false");
    assert!(
        !view.focusable_keys().iter().any(|key| key.starts_with("parameter:")),
        "insertion-off prompt still exposes parameter-row editing"
    );
    assert!(view.set_value(RUNNER_KEY, choice("claude")));
    assert_eq!(
        view.submitted_values().get(RUNNER_KEY),
        Some(&choice("claude")),
        "runner edit was hidden together with insertion-off parameter rows"
    );
}

#[test]
fn test_settings_save_preserves_a_stale_pin() {
    let mut view = SettingsView::from_inputs(&SettingsInputs {
        runner: "mine".to_owned(),
        configured_runners: vec!["other".to_owned()],
        ..prompt_inputs()
    });
    let field = view.field(RUNNER_KEY).unwrap();
    assert_eq!(field.value().as_text(), "mine");
    let FieldKind::SingleChoice { options } = &field.kind else {
        panic!("runner field is not a closed picker");
    };
    let stale = options.iter().find(|option| option.value == "mine").expect("stale pin row");
    assert_eq!(stale.label, "{} (no longer configured)");
    assert!(view.submitted_values().is_empty(), "opening/saving unrelated settings would clear the stale pin");

    assert!(view.set_value(RUNNER_KEY, choice("other")));
    assert_eq!(view.submitted_values().get(RUNNER_KEY), Some(&choice("other")));
}

#[test]
fn test_settings_interpolate_toggle_off_and_back_on() {
    let mut view = SettingsView::from_inputs(&SettingsInputs {
        managed: vec![managed("a")],
        ..prompt_inputs()
    });
    assert_eq!(view.field(INTERPOLATE_KEY).unwrap().value().as_text(), "true");
    assert!(view.set_value(INTERPOLATE_KEY, FieldValue::boolean(false)));
    assert_eq!(view.submitted_values().get(INTERPOLATE_KEY), Some(&FieldValue::boolean(false)));
    assert!(
        !view.focusable_keys().iter().any(|key| key.starts_with("parameter:")),
        "off state did not hide the precomposed parameter rows"
    );
    assert!(view.field("parameter:a:keep").is_some(), "off state destroyed the managed row instead of hiding it");

    assert!(view.set_value(INTERPOLATE_KEY, FieldValue::boolean(true)));
    assert!(view.focusable_keys().iter().any(|key| *key == "parameter:a:keep"));
    assert!(view.submitted_values().is_empty(), "off then back on to the opening value must preserve the managed list without an edit");
}

#[test]
fn test_settings_typing_a_body_hole_name_manages_it() {
    let mut view = SettingsView::from_inputs(&SettingsInputs {
        managed: vec![managed("a")],
        candidates: vec!["b".to_owned()],
        ..prompt_inputs()
    });
    assert!(view.field(ADD_PARAMETER_KEY).is_some());
    assert!(view.set_value(ADD_PARAMETER_KEY, FieldValue::text("b")));
    assert_eq!(
        view.submitted_values().get(ADD_PARAMETER_KEY),
        Some(&FieldValue::text("b")),
        "body-hole name never reached the settings host as an explicit add request"
    );
}
