//! Public-API ports of Python v0.4 declared-parameter boolean-flag edit contracts.
//!
//! These cases map the bool-action hygiene section of `tests/test_params_edit.py`: a flag-delivered
//! bool gains `store_true` only when a positive flag can truthfully turn an off-by-default value on;
//! positional/env bools gain no flag action, stale actions are shed after a type leaves bool, and an
//! existing `store_false` action remains authoritative.

use skit_application::parameter_edit::{ParameterEditError, finish_parameter_edit};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue};

fn bool_flag(name: &str, default: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.default = Some(ParameterValue::Bool(default));
    declaration.flag = format!("--{name}");
    declaration
}

#[test]
fn test_bool_flag_that_is_on_by_default_is_refused() {
    let mut declaration = bool_flag("w", true);

    let error = finish_parameter_edit(&mut declaration).unwrap_err();

    assert_eq!(
        error,
        ParameterEditError::BoolFlagOnByDefault {
            name: "w".to_owned(),
        }
    );
    assert_eq!(declaration.action, "");
}

#[test]
fn test_bool_flag_that_is_off_by_default_becomes_store_true() {
    let mut declaration = bool_flag("w", false);

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "store_true");
    assert_eq!(declaration.default, Some(ParameterValue::Bool(false)));
    assert_eq!(declaration.flag, "--w");
}

#[test]
fn test_bool_flag_without_an_explicit_default_still_becomes_store_true() {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag = "--v".to_owned();

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "store_true");
}

#[test]
fn test_bool_positional_keeps_empty_action() {
    let mut declaration = ParamDecl::new("b");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag.clear();

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "");
}

#[test]
fn test_bool_env_delivery_keeps_empty_action_even_when_a_stale_flag_string_exists() {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Env;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag = "--v".to_owned();

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "");
}

#[test]
fn test_non_bool_row_sheds_a_stale_flag_action() {
    let mut declaration = ParamDecl::new("a");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Str;
    declaration.flag = "--a".to_owned();
    declaration.action = "store_true".to_owned();

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "");
}

#[test]
fn test_existing_negative_store_false_boolean_is_not_reinterpreted_as_positive_flag() {
    let mut declaration = bool_flag("no-color", true);
    declaration.flag = "--no-color".to_owned();
    declaration.action = "store_false".to_owned();

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "store_false");
    assert_eq!(declaration.default, Some(ParameterValue::Bool(true)));
}
