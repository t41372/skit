//! Exact-name pure helper ports from Python v0.4 `tests/test_params_edit.py`.
//!
//! Rust's domain adds the `path` type. That additive capability does not weaken these frozen Python
//! contracts: the historical five must still parse, and the same invalid spellings must still be
//! rejected.

use serde_json::json;
use skit_domain::parameters::{ParameterType, ParameterValue, coerce_default};

#[test]
fn test_coerce_default_success() {
    let cases = [
        ("42", ParameterType::Int, ParameterValue::Integer(42)),
        ("3.5", ParameterType::Float, ParameterValue::Float(3.5)),
        ("true", ParameterType::Bool, ParameterValue::Bool(true)),
        ("YES", ParameterType::Bool, ParameterValue::Bool(true)),
        ("on", ParameterType::Bool, ParameterValue::Bool(true)),
        ("false", ParameterType::Bool, ParameterValue::Bool(false)),
        ("0", ParameterType::Bool, ParameterValue::Bool(false)),
        ("off", ParameterType::Bool, ParameterValue::Bool(false)),
        (
            "anything",
            ParameterType::Str,
            ParameterValue::String("anything".to_owned()),
        ),
        (
            "anything",
            ParameterType::Choice,
            ParameterValue::String("anything".to_owned()),
        ),
    ];
    for (value, parameter_type, expected) in cases {
        assert_eq!(
            coerce_default(value, parameter_type).unwrap(),
            expected,
            "{value:?} as {parameter_type:?}"
        );
    }
}

#[test]
fn test_coerce_default_rejects_bad_values() {
    for (value, parameter_type) in [
        ("x", ParameterType::Int),
        ("x", ParameterType::Float),
        ("maybe", ParameterType::Bool),
        ("inf", ParameterType::Float),
        ("nan", ParameterType::Float),
    ] {
        let error = coerce_default(value, parameter_type).unwrap_err();
        assert!(
            error.to_string().contains(value),
            "the rejected value must remain diagnosable: {error}"
        );
    }
}

#[test]
fn test_coerce_default_rejects_infinity_specifically() {
    let error = coerce_default("1e999", ParameterType::Float).unwrap_err();
    assert!(error.to_string().contains("1e999"), "{error}");
}

#[test]
fn test_as_param_type_accepts_the_five() {
    for (value, expected) in [
        ("str", ParameterType::Str),
        ("int", ParameterType::Int),
        ("float", ParameterType::Float),
        ("bool", ParameterType::Bool),
        ("choice", ParameterType::Choice),
    ] {
        assert_eq!(
            serde_json::from_value::<ParameterType>(json!(value)).unwrap(),
            expected,
            "{value}"
        );
    }
}

#[test]
fn test_as_param_type_rejects_others() {
    for value in ["integer", "", "STR", "number"] {
        assert!(
            serde_json::from_value::<ParameterType>(json!(value)).is_err(),
            "{value:?} must not silently become a parameter type"
        );
    }
}
