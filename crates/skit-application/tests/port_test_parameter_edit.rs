//! Public-API ports of the Python v0.4 declared-parameter boolean-flag edit contract.
//!
//! An on-by-default boolean cannot truthfully use a positive presence flag: the flag could only
//! turn an already-on value on again. Python reports the closed `bool-flag-on-by-default` warning;
//! the Rust application use-case exposes the same refusal as a typed edit error.

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
fn test_existing_negative_store_false_boolean_is_not_reinterpreted_as_positive_flag() {
    let mut declaration = bool_flag("no-color", true);
    declaration.flag = "--no-color".to_owned();
    declaration.action = "store_false".to_owned();

    finish_parameter_edit(&mut declaration).unwrap();

    assert_eq!(declaration.action, "store_false");
    assert_eq!(declaration.default, Some(ParameterValue::Bool(true)));
}
