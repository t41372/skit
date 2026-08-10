//! Direct ports from Python `tests/test_source_default_semantics.py`
//! (`origin/main@206f9ef`). The Python implementation is the behavioral oracle.

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{FormSource, PreparedField, form_plan};

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

const SHELL_ENVDEFAULT_SCRIPT: &str = r#"#!/usr/bin/env bash
# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "PORT"
# kind = "envdefault"
# type = "int"
# default = 9999
# ///
echo "${PORT:-8080}"
"#;

#[test]
fn test_plan_refreshes_a_stale_block_default_from_the_python_body() {
    // Block says default = "hello"; the body assigns "bonjour". The form field must carry
    // the body's value, not the cache.
    let plan = form_plan("python", REFRESH_SCRIPT, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Inject);
    let [field] = plan.fields.as_slice() else {
        panic!("expected one field, got {:?}", plan.fields);
    };
    assert_eq!(field.declaration.name, "GREETING");
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::String("bonjour".to_owned()))
    );
}

#[test]
fn test_plan_refreshes_a_stale_shell_envdefault_from_the_body() {
    // The block's 9999 is stale. The live ${PORT:-8080} fallback is the form default.
    let plan = form_plan("shell", SHELL_ENVDEFAULT_SCRIPT, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Inject);
    let [field] = plan.fields.as_slice() else {
        panic!("expected one field, got {:?}", plan.fields);
    };
    assert_eq!(field.declaration.name, "PORT");
    assert_eq!(field.declaration.delivery, ParameterDelivery::Env);
    assert_eq!(
        field.declaration.default,
        Some(ParameterValue::Integer(8080))
    );
}

fn prepared_field(
    parameter_type: ParameterType,
    delivery: ParameterDelivery,
    has_default: bool,
    secret: bool,
    degraded: bool,
    multiple: bool,
    input_binding: bool,
) -> PreparedField {
    let mut declaration = ParamDecl::new("k");
    declaration.parameter_type = parameter_type;
    declaration.delivery = delivery;
    declaration.default = has_default.then(|| ParameterValue::String("value".to_owned()));
    declaration.secret = secret;
    declaration.degraded = degraded;
    declaration.multiple = multiple;
    declaration.binding = if input_binding {
        ParameterBinding::Input
    } else {
        ParameterBinding::None
    };
    PreparedField {
        declaration,
        input_binding,
        empty_uses_default: false,
    }
}

#[test]
fn test_delivers_empty_matrix() {
    // WYSIWYG applies to exactly one shape: a non-secret, single-value, free-text inject/flag/env
    // field with a known default. Every disqualifier keeps an empty value meaning "unset".
    assert!(
        prepared_field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            false,
            false,
            false,
        )
        .delivers_empty()
    );
    assert!(
        prepared_field(
            ParameterType::Path,
            ParameterDelivery::Inject,
            true,
            false,
            false,
            false,
            false,
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
            !prepared_field(
                parameter_type,
                ParameterDelivery::Inject,
                true,
                false,
                false,
                false,
                false,
            )
            .delivers_empty()
        );
    }
    assert!(
        !prepared_field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            true,
            false,
            false,
            false,
        )
        .delivers_empty()
    );
    assert!(
        !prepared_field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            true,
            false,
            false,
        )
        .delivers_empty()
    );
    assert!(
        !prepared_field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            true,
            false,
            false,
            true,
            false,
        )
        .delivers_empty()
    );
    assert!(
        !prepared_field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            false,
            false,
            false,
            false,
            false,
        )
        .delivers_empty()
    );
    assert!(
        !prepared_field(
            ParameterType::Str,
            ParameterDelivery::Inject,
            false,
            false,
            false,
            false,
            true,
        )
        .delivers_empty()
    );
}

#[test]
fn test_input_binding_flag_reflects_the_decl_binding() {
    let source = r#"# /// script
# [tool.skit]
# schema = 1
#
# [[tool.skit.params]]
# name = "input-1"
# kind = "input"
# type = "str"
# prompt = "Name: "
# order = 0
#
# [[tool.skit.params]]
# name = "X"
# kind = "const"
# type = "str"
# default = "v"
# ///
name = input("Name: ")
X = "v"
"#;
    let plan = form_plan("python", source, &EntrySettings::default());
    let input = plan
        .fields
        .iter()
        .find(|field| field.declaration.name == "input-1")
        .unwrap();
    let constant = plan
        .fields
        .iter()
        .find(|field| field.declaration.name == "X")
        .unwrap();
    assert!(input.input_binding);
    assert!(!constant.input_binding);
}
