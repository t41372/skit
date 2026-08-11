//! Public-API ports of Python v0.4 `params.coerce_default` contracts.

use skit_domain::parameters::{ParameterType, ParameterValue, coerce_default};

#[test]
fn test_integer_and_float_defaults_coerce_to_typed_values() {
    assert_eq!(
        coerce_default("42", ParameterType::Int).unwrap(),
        ParameterValue::Integer(42)
    );
    assert_eq!(
        coerce_default("3.5", ParameterType::Float).unwrap(),
        ParameterValue::Float(3.5)
    );
}

#[test]
fn test_every_documented_true_boolean_token_is_accepted_case_and_space_insensitively() {
    for raw in ["true", "1", "yes", "y", "on", "  YES  "] {
        assert_eq!(
            coerce_default(raw, ParameterType::Bool).unwrap(),
            ParameterValue::Bool(true),
            "{raw:?}"
        );
    }
}

#[test]
fn test_every_documented_false_boolean_token_is_accepted_case_and_space_insensitively() {
    for raw in ["false", "0", "no", "n", "off", "  Off  "] {
        assert_eq!(
            coerce_default(raw, ParameterType::Bool).unwrap(),
            ParameterValue::Bool(false),
            "{raw:?}"
        );
    }
}

#[test]
fn test_str_choice_and_path_defaults_remain_strings() {
    for parameter_type in [
        ParameterType::Str,
        ParameterType::Choice,
        ParameterType::Path,
    ] {
        assert_eq!(
            coerce_default("anything", parameter_type).unwrap(),
            ParameterValue::String("anything".to_owned()),
            "{parameter_type:?}"
        );
    }
}

#[test]
fn test_bad_integer_float_and_boolean_values_are_rejected() {
    for (value, parameter_type) in [
        ("x", ParameterType::Int),
        ("x", ParameterType::Float),
        ("maybe", ParameterType::Bool),
    ] {
        let error = coerce_default(value, parameter_type).unwrap_err();
        assert!(error.to_string().contains(value), "{error}");
    }
}

#[test]
fn test_nonfinite_float_literals_are_rejected() {
    for value in ["inf", "+inf", "-inf", "nan", "NaN"] {
        let error = coerce_default(value, ParameterType::Float).unwrap_err();
        assert!(
            error
                .to_string()
                .to_lowercase()
                .contains(&value.to_lowercase()),
            "{error}"
        );
    }
}

#[test]
fn test_integer_does_not_accept_float_spelling() {
    assert!(coerce_default("1.0", ParameterType::Int).is_err());
}

#[test]
fn test_boolean_does_not_accept_arbitrary_nonzero_number() {
    assert!(coerce_default("2", ParameterType::Bool).is_err());
}
