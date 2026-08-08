use std::collections::BTreeMap;

use skit_application::form_state::{
    LastRunState, PersistedFormState, prefill, preset_values, remembered_values, scrub_secrets,
};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn declaration(name: &str, default: Option<ParameterValue>) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.default = default;
    declaration
}

#[test]
fn prefill_is_default_then_last_used_then_preset_and_never_prefills_secrets() {
    let public = declaration("city", Some(ParameterValue::String("Paris".to_owned())));
    let mut secret = declaration(
        "api_key",
        Some(ParameterValue::String("definition-secret".to_owned())),
    );
    secret.secret = true;
    let count = declaration("count", Some(ParameterValue::Integer(3)));

    let output = prefill(
        &[public, secret, count],
        &map(&[
            ("city", "Berlin"),
            ("api_key", "old-secret"),
            ("removed", "stale"),
        ]),
        Some(&map(&[("city", "Tokyo"), ("api_key", "preset-secret")])),
    );

    assert_eq!(
        output,
        map(&[("city", "Tokyo"), ("count", "3")]),
        "only current non-secret fields may survive into the next form"
    );
}

#[test]
fn definition_defaults_render_with_stable_python_compatible_scalar_spelling() {
    let text = declaration("text", Some(ParameterValue::String("hello".to_owned())));
    let integer = declaration("integer", Some(ParameterValue::Integer(-7)));
    let float = declaration("float", Some(ParameterValue::Float(1.0)));
    let decimal = declaration("decimal", Some(ParameterValue::Float(1.25)));
    let boolean = declaration("boolean", Some(ParameterValue::Bool(true)));
    let mut choice = declaration("choice", Some(ParameterValue::String("fast".to_owned())));
    choice.parameter_type = ParameterType::Choice;
    let mut path = declaration("path", Some(ParameterValue::String("./out".to_owned())));
    path.parameter_type = ParameterType::Path;

    assert_eq!(
        prefill(
            &[text, integer, float, decimal, boolean, choice, path],
            &BTreeMap::new(),
            None,
        ),
        map(&[
            ("boolean", "true"),
            ("choice", "fast"),
            ("decimal", "1.25"),
            ("float", "1.0"),
            ("integer", "-7"),
            ("path", "./out"),
            ("text", "hello"),
        ])
    );
}

#[test]
fn remembered_values_store_only_nondefault_intent_and_structurally_strip_secrets() {
    let city = declaration("city", Some(ParameterValue::String("Paris".to_owned())));

    let mut output = declaration("output", Some(ParameterValue::String("out.txt".to_owned())));
    output.delivery = ParameterDelivery::Flag;
    output.flag = "--output".to_owned();

    let optional = declaration("optional", None);

    let mut input_binding = declaration(
        "answer",
        Some(ParameterValue::String("default answer".to_owned())),
    );
    input_binding.binding = ParameterBinding::Input;

    let mut secret = declaration("token", None);
    secret.secret = true;

    let remembered = remembered_values(
        &[city, output, optional, input_binding, secret],
        &map(&[
            ("city", "Paris"),
            ("output", ""),
            ("optional", ""),
            ("answer", ""),
            ("token", "plaintext-must-never-persist"),
        ]),
    );

    assert_eq!(remembered, map(&[("output", "")]));
}

#[test]
fn presets_pin_submitted_values_but_never_persist_secrets_or_removed_fields() {
    let city = declaration("city", Some(ParameterValue::String("Paris".to_owned())));
    let empty = declaration("empty", None);
    let mut secret = declaration("password", None);
    secret.secret = true;

    let preset = preset_values(
        &[city, empty, secret],
        &map(&[
            ("city", "Paris"),
            ("empty", ""),
            ("password", "secret"),
            ("removed", "stale"),
        ]),
    );

    assert_eq!(preset, map(&[("city", "Paris"), ("empty", "")]));
}

#[test]
fn secret_transition_scrubs_all_value_surfaces_without_touching_tail_or_run_metadata() {
    let public = declaration("public", None);
    let mut token = declaration("token", None);
    token.secret = true;
    let mut password = declaration("password", None);
    password.secret = true;

    let mut state = PersistedFormState {
        values: map(&[
            ("public", "keep"),
            ("token", "old-token"),
            ("unknown", "forward-compatible"),
        ]),
        extra_args: vec!["--literal".to_owned()],
        extra_args_raw: true,
        presets: BTreeMap::from([
            (
                "mixed".to_owned(),
                map(&[("public", "keep"), ("password", "old-password")]),
            ),
            ("secret-only".to_owned(), map(&[("token", "old-token")])),
        ]),
        last_run: LastRunState {
            at: Some("2026-08-07T17:22:00Z".to_owned()),
            exit: Some(7),
            values: map(&[("token", "old-token"), ("public", "keep")]),
        },
    };

    let removed = scrub_secrets(&[public, token, password], &mut state);

    assert_eq!(removed, ["password".to_owned(), "token".to_owned()].into());
    assert_eq!(
        state.values,
        map(&[("public", "keep"), ("unknown", "forward-compatible")])
    );
    assert_eq!(
        state.presets,
        BTreeMap::from([("mixed".to_owned(), map(&[("public", "keep")]))])
    );
    assert_eq!(state.last_run.values, map(&[("public", "keep")]));
    assert_eq!(state.last_run.at.as_deref(), Some("2026-08-07T17:22:00Z"));
    assert_eq!(state.last_run.exit, Some(7));
    assert_eq!(state.extra_args, ["--literal"]);
    assert!(state.extra_args_raw);
}

#[test]
fn defaulted_nontext_fields_do_not_treat_empty_as_an_explicit_delivered_value() {
    let mut integer = declaration("count", Some(ParameterValue::Integer(3)));
    integer.parameter_type = ParameterType::Int;

    let mut boolean = declaration("enabled", Some(ParameterValue::Bool(true)));
    boolean.parameter_type = ParameterType::Bool;

    assert!(
        remembered_values(&[integer, boolean], &map(&[("count", ""), ("enabled", "")]),).is_empty()
    );
}
