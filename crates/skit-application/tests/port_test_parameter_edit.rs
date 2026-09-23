//! Public API ports of Python v0.4 boolean parameter edit contracts.

use skit_application::parameter_edit::finish_parameter_edit;
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};

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
