use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterValue};
use skit_ui::{Action, Effect, FieldValue, FormControl, FormPurpose, LibraryState, RunFormView, Screen};

fn state_with(form: RunFormView) -> LibraryState {
    let mut state = LibraryState::default();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

#[test]
fn test_command_placeholders_collect_interactively() {
    let form = RunFormView::from_declarations(
        "e",
        "e",
        &[ParamDecl::new("msg")],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    let mut state = state_with(form);
    state.update(Action::SetFieldValue {
        field: 0,
        value: "typed".to_owned(),
    });
    assert_eq!(
        state.update(Action::Submit),
        Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("e".to_owned()),
            values: BTreeMap::from([
                ("value:msg".to_owned(), FieldValue::text("typed")),
                ("_skit_args".to_owned(), FieldValue::text("")),
                ("_skit_save_preset".to_owned(), FieldValue::text("")),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ]),
        }
    );
}

#[test]
fn test_collect_param_form_interactive_secret() {
    let mut api = ParamDecl::new("API");
    api.secret = true;
    let form = RunFormView::from_declarations(
        "a",
        "a",
        &[api],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    assert!(matches!(
        &form.fields()[0].control,
        FormControl::Text(text) if text.secret
    ));
    let mut state = state_with(form);
    state.update(Action::SetFieldValue {
        field: 0,
        value: "secretval".to_owned(),
    });
    assert_eq!(
        state.update(Action::Submit),
        Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("a".to_owned()),
            values: BTreeMap::from([
                ("value:API".to_owned(), FieldValue::text("secretval")),
                ("_skit_args".to_owned(), FieldValue::text("")),
                ("_skit_save_preset".to_owned(), FieldValue::text("")),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ]),
        },
        "secret controls hide display text but must submit the exact plaintext value to the launch boundary"
    );
}

#[test]
fn test_param_form_prefill_uses_definition_default() {
    let mut city = ParamDecl::new("CITY");
    city.default = Some(ParameterValue::String("Osaka".to_owned()));
    let form = RunFormView::from_declarations(
        "a",
        "a",
        &[city],
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    );
    assert_eq!(form.fields()[0].control.value(), "Osaka");
}
