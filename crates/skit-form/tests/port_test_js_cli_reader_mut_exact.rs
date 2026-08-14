//! Exact public-surface ports of Python v0.4 `tests/test_js_cli_reader_mut.py`.
//!
//! All ten Python tests drive the real static `util.parseArgs` reader. Rust exercises the same
//! frontend-neutral projection through `onboarding_plan`; no private helper is recreated here.

use skit_domain::parameters::{ParameterBinding, ParameterDelivery, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, onboarding_plan};

fn fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    let plan = onboarding_plan("js", source);
    match plan.cli_surface {
        CliFormProjection::Static { fields, .. } => fields,
        other => panic!("expected a static parseArgs surface: {other:?}"),
    }
}

#[test]
fn test_option_spec_skips_computed_pair_then_keeps_reading_the_real_type() {
    let actual = fields("parseArgs({options:{flag:{[dyn]:1, type:\"boolean\"}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_option_spec_skips_non_pair_then_keeps_reading_the_real_type() {
    let actual = fields("parseArgs({options:{flag:{...rest, type:\"boolean\"}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert_eq!(field.parameter_type, ParameterType::Bool);
}

#[test]
fn test_string_type_yields_a_clean_str_field_not_a_degraded_one() {
    let actual = fields("parseArgs({options:{name:{type:\"string\"}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert!(!field.degraded);
}

#[test]
fn test_identifier_call_that_is_not_parseargs_is_not_read_as_a_surface() {
    let plan = onboarding_plan("js", "foo({options:{n:{type:\"string\"}}});\n");
    assert!(matches!(plan.cli_surface, CliFormProjection::Absent));
}

#[test]
fn test_member_call_that_is_not_parseargs_is_not_read_as_a_surface() {
    let plan = onboarding_plan("js", "console.log({options:{n:{type:\"string\"}}});\n");
    assert!(matches!(plan.cli_surface, CliFormProjection::Absent));
}

#[test]
fn test_numeric_option_key_names_no_field() {
    let actual = fields("parseArgs({options:{0:{type:\"string\"}}});\n");
    assert!(actual.is_empty(), "{actual:?}");
}

#[test]
fn test_read_option_defaults_binding_none_delivery_flag() {
    let actual = fields("parseArgs({options:{name:{type:\"string\"}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert_eq!(field.binding, ParameterBinding::None);
    assert_eq!(field.delivery, ParameterDelivery::Flag);
    assert_eq!(field.flag, "--name");
}

#[test]
fn test_multiple_true_option_sets_both_multiple_and_repeat() {
    let actual = fields("parseArgs({options:{tag:{type:\"string\",multiple:true}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert!(field.multiple);
    assert!(field.repeat);
}

#[test]
fn test_no_multiple_key_leaves_both_off() {
    let actual = fields("parseArgs({options:{name:{type:\"string\"}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert!(!field.multiple);
    assert!(!field.repeat);
}

#[test]
fn test_multiple_false_option_leaves_both_off() {
    let actual = fields("parseArgs({options:{tag:{type:\"string\",multiple:false}}});\n");
    let [field] = actual.as_slice() else {
        panic!("expected one field: {actual:?}");
    };
    assert!(!field.multiple);
    assert!(!field.repeat);
}
