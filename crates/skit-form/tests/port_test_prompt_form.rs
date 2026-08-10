//! Public-API prompt form-schema ports from Python v0.4 `test_prompt_kind.py`.
//!
//! Prompt bodies are user text; the stored managed list remains the form contract. Newly appearing
//! holes are not auto-adopted at run time, while disappeared managed holes remain visible as drift.

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{FormDrift, FormSource, form_plan};

fn placeholder(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Placeholder;
    declaration.required = true;
    declaration
}

#[test]
fn test_prompt_plan_fields_follow_the_stored_managed_list_only() {
    let settings = EntrySettings {
        params: vec!["a".to_owned(), "api_key".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan(
        "prompt",
        "{{a}} {{api_key}} {{new_unmanaged}}\n",
        &settings,
    );

    assert_eq!(plan.source, FormSource::Command);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "api_key"]
    );
    assert!(plan.drift.is_empty());
    assert!(plan
        .fields
        .iter()
        .all(|field| field.declaration.delivery == ParameterDelivery::Placeholder));
}

#[test]
fn test_prompt_plan_reports_gone_managed_names_without_dropping_their_fields() {
    let settings = EntrySettings {
        params: vec!["a".to_owned(), "b".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", "only {{a}} now\n", &settings);

    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    assert!(matches!(
        plan.drift.as_slice(),
        [FormDrift::PromptMissing { names }] if names == &["b".to_owned()]
    ));
}

#[test]
fn test_prompt_declared_placeholder_enriches_the_managed_field_and_env_rider_survives() {
    let mut n = placeholder("n");
    n.parameter_type = ParameterType::Int;
    n.default = Some(ParameterValue::Integer(3));
    n.required = false;

    let mut extra = ParamDecl::new("EXTRA");
    extra.delivery = ParameterDelivery::Env;
    extra.env_target = "EXTRA".to_owned();

    let settings = EntrySettings {
        params: vec!["n".to_owned()],
        parameters: vec![n, extra],
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", "{{n}}\n", &settings);

    assert_eq!(plan.source, FormSource::Command);
    assert_eq!(plan.fields.len(), 2);
    assert_eq!(plan.fields[0].declaration.name, "n");
    assert_eq!(plan.fields[0].declaration.parameter_type, ParameterType::Int);
    assert_eq!(
        plan.fields[0].declaration.default,
        Some(ParameterValue::Integer(3))
    );
    assert!(!plan.fields[0].declaration.required);
    assert_eq!(plan.fields[1].declaration.name, "EXTRA");
    assert_eq!(plan.fields[1].declaration.delivery, ParameterDelivery::Env);
}

#[test]
fn test_prompt_new_body_holes_do_not_expand_the_managed_schema_at_run_time() {
    let settings = EntrySettings {
        params: vec!["kept".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan(
        "prompt",
        "{{kept}} {{new_one}} {{new_two}}\n",
        &settings,
    );

    assert_eq!(plan.fields.len(), 1);
    assert_eq!(plan.fields[0].declaration.name, "kept");
}

#[test]
fn test_prompt_interpolate_off_exposes_no_managed_fields_or_drift() {
    let settings = EntrySettings {
        params: vec!["a".to_owned(), "gone".to_owned()],
        interpolate: false,
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", "{{a}}\n", &settings);

    assert_eq!(plan.source, FormSource::Command);
    assert!(plan.fields.is_empty());
    assert!(plan.drift.is_empty());
}

#[test]
fn test_prompt_declared_duplicate_name_uses_the_last_row_for_managed_enrichment() {
    let mut first = placeholder("x");
    first.parameter_type = ParameterType::Str;
    first.default = Some(ParameterValue::String("old".to_owned()));

    let mut second = placeholder("x");
    second.parameter_type = ParameterType::Int;
    second.default = Some(ParameterValue::Integer(7));

    let settings = EntrySettings {
        params: vec!["x".to_owned()],
        parameters: vec![first, second],
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", "{{x}}\n", &settings);

    let [field] = plan.fields.as_slice() else {
        panic!("expected one deduplicated managed field: {plan:?}");
    };
    assert_eq!(field.declaration.parameter_type, ParameterType::Int);
    assert_eq!(field.declaration.default, Some(ParameterValue::Integer(7)));
}

#[test]
fn test_prompt_non_placeholder_row_with_same_name_does_not_replace_the_managed_placeholder() {
    let mut env = ParamDecl::new("x");
    env.delivery = ParameterDelivery::Env;
    env.default = Some(ParameterValue::String("env-default".to_owned()));

    let settings = EntrySettings {
        params: vec!["x".to_owned()],
        parameters: vec![env],
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", "{{x}}\n", &settings);

    let [field] = plan.fields.as_slice() else {
        panic!("expected the synthesized placeholder only: {plan:?}");
    };
    assert_eq!(field.declaration.name, "x");
    assert_eq!(field.declaration.delivery, ParameterDelivery::Placeholder);
    assert_eq!(field.declaration.default, None);
}

#[test]
fn test_command_form_keeps_the_same_placeholder_schema_contract() {
    let settings = EntrySettings {
        params: vec!["size".to_owned(), "out".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan("command", "", &settings);

    assert_eq!(plan.source, FormSource::Command);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["size", "out"]
    );
    assert!(plan.drift.is_empty());
}
