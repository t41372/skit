use std::collections::BTreeMap;

use skit_core::{
    Delivery, EntryState, FormField, FormPlan, ParamDecl, ParamDefault, ParamType, PlanSource,
    prefill, resolve_values, validate_values,
};

fn field(name: &str) -> FormField {
    FormField::from_decl(&ParamDecl {
        name: name.to_owned(),
        delivery: Delivery::Flag,
        ..ParamDecl::default()
    })
}

#[test]
fn resolution_order_is_default_then_last_then_preset_then_explicit() {
    let plan = FormPlan {
        source: PlanSource::Declared,
        fields: vec![FormField::from_decl(&ParamDecl {
            name: "CITY".to_owned(),
            delivery: Delivery::Env,
            default: Some(ParamDefault::String("Osaka".to_owned())),
            ..ParamDecl::default()
        })],
        ..FormPlan::default()
    };
    let mut state = EntryState::default();
    state.values.insert("CITY".to_owned(), "Taipei".to_owned());
    state.presets.insert(
        "jp".to_owned(),
        BTreeMap::from([("CITY".to_owned(), "Kyoto".to_owned())]),
    );

    assert_eq!(
        prefill(&plan, &EntryState::default(), None).get("CITY").map(String::as_str),
        Some("Osaka")
    );
    assert_eq!(prefill(&plan, &state, None)["CITY"], "Taipei");
    assert_eq!(prefill(&plan, &state, Some("jp"))["CITY"], "Kyoto");

    let explicit = BTreeMap::from([("CITY".to_owned(), "Tainan".to_owned())]);
    let resolved = resolve_values(&plan, &state, Some("jp"), &explicit);
    assert!(resolved.is_ok());
    assert_eq!(resolved.ok().and_then(|values| values.get("CITY").cloned()), Some("Tainan".to_owned()));
}

#[test]
fn secret_values_are_never_prefilled_but_explicit_secret_is_allowed_for_this_run() {
    let plan = FormPlan {
        source: PlanSource::Declared,
        fields: vec![FormField::from_decl(&ParamDecl {
            name: "API_KEY".to_owned(),
            delivery: Delivery::Env,
            default: Some(ParamDefault::String("in-script".to_owned())),
            secret: true,
            ..ParamDecl::default()
        })],
        ..FormPlan::default()
    };
    let mut state = EntryState::default();
    state.values.insert("API_KEY".to_owned(), "stale".to_owned());
    state.presets.insert(
        "prod".to_owned(),
        BTreeMap::from([("API_KEY".to_owned(), "also-stale".to_owned())]),
    );

    assert!(prefill(&plan, &state, Some("prod")).is_empty());
    let explicit = BTreeMap::from([("API_KEY".to_owned(), "typed-now".to_owned())]);
    let resolved = resolve_values(&plan, &state, Some("prod"), &explicit);
    assert_eq!(
        resolved.ok().and_then(|values| values.get("API_KEY").cloned()),
        Some("typed-now".to_owned())
    );
}

#[test]
fn explicit_unknown_key_is_refused_instead_of_dropped() {
    let plan = FormPlan {
        source: PlanSource::Declared,
        fields: vec![field("CITY")],
        ..FormPlan::default()
    };
    let explicit = BTreeMap::from([("STALE".to_owned(), "x".to_owned())]);
    let error = resolve_values(&plan, &EntryState::default(), None, &explicit);
    assert!(matches!(error, Err(error) if error.key == "STALE"));
}

#[test]
fn validation_covers_required_typed_choice_and_finite_float() {
    let plan = FormPlan {
        source: PlanSource::Declared,
        fields: vec![
            FormField::from_decl(&ParamDecl {
                name: "COUNT".to_owned(),
                required: true,
                param_type: ParamType::Integer,
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "RATIO".to_owned(),
                param_type: ParamType::Float,
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "ENABLED".to_owned(),
                param_type: ParamType::Boolean,
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "FORMAT".to_owned(),
                param_type: ParamType::Choice,
                choices: vec!["png".to_owned(), "jpg".to_owned()],
                ..ParamDecl::default()
            }),
        ],
        ..FormPlan::default()
    };
    let values = BTreeMap::from([
        ("COUNT".to_owned(), "".to_owned()),
        ("RATIO".to_owned(), "NaN".to_owned()),
        ("ENABLED".to_owned(), "maybe".to_owned()),
        ("FORMAT".to_owned(), "webp".to_owned()),
    ]);

    let errors = validate_values(&plan, &values);
    assert_eq!(errors.len(), 4);
    assert!(errors["COUNT"].contains("required"));
    assert!(errors["RATIO"].contains("number"));
    assert!(errors["ENABLED"].contains("on or off"));
    assert!(errors["FORMAT"].contains("png, jpg"));
}

#[test]
fn bool_spellings_match_the_existing_python_contract() {
    let plan = FormPlan {
        source: PlanSource::Declared,
        fields: vec![FormField::from_decl(&ParamDecl {
            name: "B".to_owned(),
            param_type: ParamType::Boolean,
            ..ParamDecl::default()
        })],
        ..FormPlan::default()
    };
    for value in ["true", "1", "yes", "y", "on", "false", "0", "no", "n", "off"] {
        let values = BTreeMap::from([("B".to_owned(), value.to_owned())]);
        assert!(validate_values(&plan, &values).is_empty(), "{value}");
    }
}

#[test]
fn multiple_typed_fields_validate_each_shell_quoted_piece() {
    let plan = FormPlan {
        source: PlanSource::Declared,
        fields: vec![FormField::from_decl(&ParamDecl {
            name: "POINTS".to_owned(),
            param_type: ParamType::Integer,
            multiple: true,
            ..ParamDecl::default()
        })],
        ..FormPlan::default()
    };
    let good = BTreeMap::from([("POINTS".to_owned(), "1 '2' 3".to_owned())]);
    assert!(validate_values(&plan, &good).is_empty());

    let bad = BTreeMap::from([("POINTS".to_owned(), "1 'two words' 3".to_owned())]);
    assert!(validate_values(&plan, &bad).contains_key("POINTS"));
}

#[test]
fn delivers_empty_is_typed_not_stringly() {
    let text = FormField::from_decl(&ParamDecl {
        name: "TEXT".to_owned(),
        delivery: Delivery::Env,
        default: Some(ParamDefault::String("hello".to_owned())),
        ..ParamDecl::default()
    });
    let integer = FormField::from_decl(&ParamDecl {
        name: "N".to_owned(),
        delivery: Delivery::Env,
        param_type: ParamType::Integer,
        default: Some(ParamDefault::Integer(3)),
        ..ParamDecl::default()
    });
    assert!(text.delivers_empty());
    assert!(!integer.delivers_empty());
}
