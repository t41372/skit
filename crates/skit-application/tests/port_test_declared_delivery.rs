//! Exact-name delivery ports from Python v0.4 `tests/test_declared_params.py`.

use std::collections::BTreeMap;

use skit_application::delivery::{PreparedValue, assemble};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};

fn scalar(value: &str) -> PreparedValue {
    PreparedValue::Scalar(value.to_owned())
}

fn values(items: &[(&str, &str)]) -> BTreeMap<String, PreparedValue> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), scalar(value)))
        .collect()
}

#[test]
fn test_assemble_env_values_masked_and_empty_absent() {
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Env;
    width.parameter_type = ParameterType::Int;

    let mut token = ParamDecl::new("token");
    token.delivery = ParameterDelivery::Env;
    token.secret = true;
    token.env_target = "API_TOKEN".to_owned();

    let mut unset = ParamDecl::new("UNSET");
    unset.delivery = ParameterDelivery::Env;

    let assembly = assemble(
        &[width, token, unset],
        &values(&[("WIDTH", "800"), ("token", "hunter2")]),
        &[],
    )
    .unwrap();

    assert_eq!(
        assembly.env_values,
        BTreeMap::from([
            ("WIDTH".to_owned(), "800".to_owned()),
            ("API_TOKEN".to_owned(), "hunter2".to_owned()),
        ])
    );
    assert_eq!(
        assembly.masked_env,
        BTreeMap::from([
            ("WIDTH".to_owned(), "800".to_owned()),
            ("API_TOKEN".to_owned(), "•••".to_owned()),
        ])
    );
    assert!(!assembly.env_values.contains_key("UNSET"));
    assert!(assembly.args.is_empty());
}

#[test]
fn test_assemble_mixed_flag_and_env_fields() {
    let mut width = ParamDecl::new("width");
    width.delivery = ParameterDelivery::Flag;
    width.flag = "--width".to_owned();
    width.parameter_type = ParameterType::Int;

    let mut debug = ParamDecl::new("DEBUG");
    debug.delivery = ParameterDelivery::Env;

    let assembly = assemble(
        &[width, debug],
        &values(&[("width", "800"), ("DEBUG", "1")]),
        &["-v".to_owned()],
    )
    .unwrap();

    assert_eq!(assembly.args, ["--width", "800", "-v"]);
    assert_eq!(
        assembly.env_values,
        BTreeMap::from([("DEBUG".to_owned(), "1".to_owned())])
    );
}

#[test]
fn test_assemble_command_with_env_rider() {
    let mut msg = ParamDecl::new("msg");
    msg.delivery = ParameterDelivery::Placeholder;
    let mut retries = ParamDecl::new("RETRIES");
    retries.delivery = ParameterDelivery::Env;

    let assembly = assemble(
        &[msg, retries],
        &values(&[("msg", "hi"), ("RETRIES", "3")]),
        &[],
    )
    .unwrap();

    assert_eq!(
        assembly.command_values,
        BTreeMap::from([("msg".to_owned(), "hi".to_owned())])
    );
    assert_eq!(
        assembly.env_values,
        BTreeMap::from([("RETRIES".to_owned(), "3".to_owned())])
    );
    assert!(!assembly.command_values.contains_key("RETRIES"));
}

#[test]
fn test_declared_plan_secret_placeholder_masks_in_command_values() {
    let mut password = ParamDecl::new("password");
    password.delivery = ParameterDelivery::Placeholder;
    password.secret = true;

    let assembly = assemble(
        &[password],
        &values(&[("password", "s3cret")]),
        &[],
    )
    .unwrap();

    assert_eq!(
        assembly.command_values,
        BTreeMap::from([("password".to_owned(), "s3cret".to_owned())])
    );
    assert_eq!(
        assembly.masked_command_values,
        BTreeMap::from([("password".to_owned(), "•••".to_owned())])
    );
}
