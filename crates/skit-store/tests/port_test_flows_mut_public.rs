//! Rust-additive public-boundary form/application/language regression coverage.
//!
//! These assertions are intentionally NOT counted as ports of Python v0.4 `tests/test_flows_mut.py`:
//! the frozen Python module does not contain these six contracts. They remain useful strengthening
//! tests over existing public Rust surfaces, but must not impersonate Python parity coverage.

use std::collections::BTreeMap;

use skit_application::{
    delivery::{PreparedValue, assemble},
    form_state::{prefill, remembered_values},
};
use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::form_plan;
use skit_language::{inject_values, render_prompt_body};

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn rust_additive_shell_missing_input_spec_name_is_ordered() {
    let mut later = ParamDecl::new("B");
    later.binding = ParameterBinding::Input;
    later.delivery = ParameterDelivery::Inject;
    later.order = 1;
    let mut earlier = ParamDecl::new("A");
    earlier.binding = ParameterBinding::Input;
    earlier.delivery = ParameterDelivery::Inject;
    earlier.order = 0;

    let error = inject_values(
        "shell",
        "#!/bin/sh\necho ok\n",
        &[later, earlier],
        &values(&[("A", "a"), ("B", "b")]),
    )
    .unwrap_err();

    assert_eq!(error.to_string(), "A, B");
}

#[test]
fn rust_additive_prompt_interpolation_uses_only_managed_values() {
    let body = "A={{keep}} B={{drop}}";
    let settings = EntrySettings {
        params: vec!["keep".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", body, &settings);
    assert_eq!(
        plan.fields
            .iter()
            .map(|field| field.declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["keep"]
    );

    let prepared = BTreeMap::from([
        ("keep".to_owned(), PreparedValue::Scalar("X".to_owned())),
        ("drop".to_owned(), PreparedValue::Scalar("Y".to_owned())),
    ]);
    let assembly = assemble(&plan.declarations(), &prepared, &[]).unwrap();
    assert_eq!(
        assembly.command_values,
        BTreeMap::from([("keep".to_owned(), "X".to_owned())])
    );
    assert_eq!(
        render_prompt_body(body, &assembly.command_values, true),
        "A=X B={{drop}}"
    );
}

#[test]
fn rust_additive_plan_for_prompt_no_interpolate_has_no_fields() {
    let settings = EntrySettings {
        params: vec!["x".to_owned(), "y".to_owned()],
        interpolate: false,
        ..EntrySettings::default()
    };
    let plan = form_plan("prompt", "{{x}} {{y}}", &settings);
    assert!(plan.fields.is_empty(), "{plan:?}");
}

#[test]
fn rust_additive_remembered_values_input_binding_is_never_remembered() {
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.prompt = "Name? ".to_owned();
    declaration.order = 0;

    assert_eq!(
        remembered_values(&[declaration], &values(&[("input-1", "Alice")])),
        BTreeMap::new()
    );
}

#[test]
fn rust_additive_remembered_values_typed_empty_not_remembered() {
    let mut declaration = ParamDecl::new("WIDTH");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Int;
    declaration.default = Some(ParameterValue::Integer(800));

    assert_eq!(
        remembered_values(&[declaration], &values(&[("WIDTH", "")])),
        BTreeMap::new()
    );
}

#[test]
fn rust_additive_prefill_ignores_stale_last_used_keys() {
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Flag;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));

    let actual = prefill(
        &[width],
        &values(&[("WIDTH", "900"), ("REMOVED", "stale")]),
        None,
    );
    assert_eq!(actual, values(&[("WIDTH", "900")]));
    assert!(!actual.contains_key("REMOVED"));
}
