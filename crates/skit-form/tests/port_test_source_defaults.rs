//! Public-API ports of source-default and form reconciliation contracts from Python v0.4.
//!
//! The Python suite is authoritative. Behavior mismatches are intentionally left red; this file
//! does not require or justify production changes on the oracle-port branch.

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{FormDrift, FormSource, PreparedField, form_plan};

const REFRESH_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "GREETING"
# kind = "const"
# type = "str"
# default = "hello"
# ///
GREETING = 'bonjour'
print(GREETING)
"#;

const TYPE_CHANGED_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "RETRIES"
# kind = "const"
# type = "int"
# default = 3
# ///
RETRIES = "three"
print(RETRIES)
"#;

const MISSING_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "const"
# type = "str"
# default = "Taipei"
# ///
print("no managed assignment now")
"#;

const INPUT_SCRIPT: &str = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "input-1"
# kind = "input"
# type = "str"
# default = "Tim"
# order = 0
# prompt = "Your name? "
# ///
name = input("Your name? ")
print(name)
"#;

fn shell_envdefault(
    operator: &str,
    declared_type: &str,
    block_default: &str,
    source_default: &str,
) -> String {
    format!(
        r#"#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "CITY"
# kind = "envdefault"
# type = "{declared_type}"
# default = {block_default}
# ///
echo "${{CITY{operator}{source_default}}}"
"#,
    )
}

fn assert_string_default(field: &PreparedField, expected: &str) {
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::String(expected.to_owned()))
    );
}

#[test]
fn test_plan_refreshes_a_stale_block_default_from_the_python_body() {
    let plan = form_plan("python", REFRESH_SCRIPT, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Inject);
    assert!(plan.drift.is_empty());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one managed field: {plan:?}");
    };
    assert_eq!(field.declaration.name, "GREETING");
    assert_string_default(field, "bonjour");
    assert!(field.delivers_empty());
}

#[test]
fn test_plan_refreshes_a_stale_shell_envdefault_from_the_body() {
    let text = shell_envdefault(":-", "int", "9999", "8080");
    let plan = form_plan("shell", &text, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Inject);
    assert!(plan.drift.is_empty());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one managed field: {plan:?}");
    };
    assert_eq!(field.declaration.name, "CITY");
    assert_eq!(field.declaration.delivery, ParameterDelivery::Env);
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::Integer(8080))
    );
}

#[test]
fn test_type_changed_const_keeps_the_stored_prefill_and_reports_drift() {
    let plan = form_plan("python", TYPE_CHANGED_SCRIPT, &EntrySettings::default());

    let [field] = plan.fields.as_slice() else {
        panic!("expected the changed field to stay visible: {plan:?}");
    };
    assert_eq!(field.declaration.name, "RETRIES");
    assert_eq!(field.declaration.parameter_type, ParameterType::Int);
    assert_eq!(field.declaration.default, Some(ParameterValue::Integer(3)));
    assert!(matches!(
        plan.drift.as_slice(),
        [FormDrift::TypeChanged { stored, current }]
            if stored.name == "RETRIES"
                && stored.parameter_type == ParameterType::Int
                && current.parameter_type == ParameterType::Str
    ));
}

#[test]
fn test_missing_managed_binding_is_removed_from_fields_but_reported_as_drift() {
    let plan = form_plan("python", MISSING_SCRIPT, &EntrySettings::default());

    assert_eq!(plan.source, FormSource::Inject);
    assert!(plan.fields.is_empty());
    assert!(matches!(
        plan.drift.as_slice(),
        [FormDrift::Missing { declaration }] if declaration.name == "CITY"
    ));
}

#[test]
fn test_input_binding_flag_reflects_the_decl_binding() {
    let plan = form_plan("python", INPUT_SCRIPT, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one input field: {plan:?}");
    };

    assert_eq!(field.declaration.binding, ParameterBinding::Input);
    assert!(field.input_binding);
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::String("Tim".to_owned()))
    );
    assert!(!field.delivers_empty());
    assert!(plan.drift.is_empty());
}

#[test]
fn test_envdefault_default_that_no_longer_fits_the_type_is_not_published() {
    let text = shell_envdefault(":-", "int", "8080", "$FALLBACK");
    let plan = form_plan("shell", &text, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one envdefault field: {plan:?}");
    };

    assert_eq!(field.declaration.parameter_type, ParameterType::Int);
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::Integer(8080))
    );
}

#[test]
fn test_secret_source_literal_never_becomes_a_form_default() {
    let text = r#"# /// script
# dependencies = []
#
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "TOKEN"
# kind = "const"
# type = "str"
# secret = true
# ///
TOKEN = "sk-live-source"
print(TOKEN)
"#;
    let plan = form_plan("python", text, &EntrySettings::default());
    let [field] = plan.fields.as_slice() else {
        panic!("expected exactly one secret field: {plan:?}");
    };

    assert!(field.declaration.secret);
    assert_eq!(field.declaration.default, None);
}

#[test]
fn test_shell_colon_envdefaults_mark_empty_as_source_default_semantics() {
    for operator in [":-", ":="] {
        let text = shell_envdefault(operator, "str", "\"Taipei\"", "Taipei");
        let plan = form_plan("shell", &text, &EntrySettings::default());
        let [field] = plan.fields.as_slice() else {
            panic!("expected one field for {operator}: {plan:?}");
        };

        assert!(field.empty_uses_default, "operator {operator}");
        assert!(!field.delivers_empty(), "operator {operator}");
    }
}

#[test]
fn test_shell_noncolon_envdefaults_genuinely_deliver_empty() {
    for operator in ["-", "="] {
        let text = shell_envdefault(operator, "str", "\"Taipei\"", "Taipei");
        let plan = form_plan("shell", &text, &EntrySettings::default());
        let [field] = plan.fields.as_slice() else {
            panic!("expected one field for {operator}: {plan:?}");
        };

        assert!(!field.empty_uses_default, "operator {operator}");
        assert!(field.delivers_empty(), "operator {operator}");
    }
}

#[test]
fn test_delivers_empty_matrix() {
    fn field(
        parameter_type: ParameterType,
        delivery: ParameterDelivery,
        has_default: bool,
        secret: bool,
        degraded: bool,
        multiple: bool,
        input_binding: bool,
        empty_uses_default: bool,
    ) -> PreparedField {
        let mut declaration = ParamDecl::new("k");
        declaration.parameter_type = parameter_type;
        declaration.delivery = delivery;
        declaration.default = has_default.then(|| ParameterValue::String("default".to_owned()));
        declaration.secret = secret;
        declaration.degraded = degraded;
        declaration.multiple = multiple;
        PreparedField {
            declaration,
            input_binding,
            empty_uses_default,
        }
    }

    assert!(
        field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            false,
            false,
            false,
            false
        )
        .delivers_empty()
    );
    assert!(
        field(
            ParameterType::Path,
            ParameterDelivery::Env,
            true,
            false,
            false,
            false,
            false,
            false
        )
        .delivers_empty()
    );

    for parameter_type in [
        ParameterType::Int,
        ParameterType::Float,
        ParameterType::Bool,
        ParameterType::Choice,
    ] {
        assert!(
            !field(
                parameter_type,
                ParameterDelivery::Inject,
                true,
                false,
                false,
                false,
                false,
                false
            )
            .delivers_empty()
        );
    }
    assert!(
        !field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            true,
            false,
            false,
            false,
            false
        )
        .delivers_empty()
    );
    assert!(
        !field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            true,
            false,
            false,
            false
        )
        .delivers_empty()
    );
    assert!(
        !field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            false,
            true,
            false,
            false
        )
        .delivers_empty()
    );
    assert!(
        !field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            false,
            false,
            false,
            false,
            false,
            false
        )
        .delivers_empty()
    );
    assert!(
        !field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            false,
            false,
            true,
            false
        )
        .delivers_empty()
    );
    assert!(
        !field(
            ParameterType::Str,
            ParameterDelivery::Env,
            true,
            false,
            false,
            false,
            false,
            true
        )
        .delivers_empty()
    );
}
