use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

fn map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn every_parameter_axis_has_one_stable_spelling() {
    assert_eq!(ParameterBinding::Const.as_str(), "const");
    assert_eq!(ParameterBinding::Input.as_str(), "input");
    assert_eq!(ParameterBinding::EnvDefault.as_str(), "envdefault");
    assert_eq!(ParameterBinding::None.as_str(), "none");

    assert_eq!(ParameterDelivery::Inject.as_str(), "inject");
    assert_eq!(ParameterDelivery::Env.as_str(), "env");
    assert_eq!(ParameterDelivery::Flag.as_str(), "flag");
    assert_eq!(ParameterDelivery::Placeholder.as_str(), "placeholder");

    assert_eq!(ParameterType::Str.as_str(), "str");
    assert_eq!(ParameterType::Int.as_str(), "int");
    assert_eq!(ParameterType::Float.as_str(), "float");
    assert_eq!(ParameterType::Bool.as_str(), "bool");
    assert_eq!(ParameterType::Choice.as_str(), "choice");
    assert_eq!(ParameterType::Path.as_str(), "path");
}

#[test]
fn every_scalar_default_round_trips_through_the_meta_shape() {
    let cases = [
        (ParameterType::Int, ParameterValue::Integer(42), json!(42)),
        (
            ParameterType::Float,
            ParameterValue::Float(2.5),
            json!(2.5),
        ),
        (
            ParameterType::Bool,
            ParameterValue::Bool(true),
            json!(true),
        ),
        (
            ParameterType::Str,
            ParameterValue::String("value".to_owned()),
            json!("value"),
        ),
    ];

    for (parameter_type, default, encoded_default) in cases {
        let declaration = ParamDecl {
            parameter_type,
            default: Some(default),
            ..ParamDecl::new(parameter_type.as_str())
        };
        let encoded = declaration.to_meta_map();
        assert_eq!(encoded.get("default"), Some(&encoded_default));
        assert_eq!(ParamDecl::from_meta_map(&encoded), declaration);
    }
}

#[test]
fn non_default_binding_and_order_are_explicit_meta_fields() {
    let declaration = ParamDecl {
        binding: ParameterBinding::Input,
        delivery: ParameterDelivery::Inject,
        order: 7,
        ..ParamDecl::new("input-7")
    };

    let encoded = declaration.to_meta_map();

    assert_eq!(encoded.get("binding"), Some(&json!("input")));
    assert_eq!(encoded.get("delivery"), Some(&json!("inject")));
    assert_eq!(encoded.get("order"), Some(&json!(7)));
    assert_eq!(ParamDecl::from_meta_map(&encoded), declaration);
}

#[test]
fn garbage_scalars_are_stringified_and_truthiness_stays_total() {
    let declaration = ParamDecl::from_meta_map(&map(json!({
        "name": true,
        "prompt": false,
        "help": null,
        "required": "",
        "multiple": [],
        "repeat": {"present": true},
        "secret": null,
        "choices": [true, false, null, 3, ["x"], {"k": "v"}]
    })));

    assert_eq!(declaration.name, "True");
    assert_eq!(declaration.prompt, "False");
    assert_eq!(declaration.help, "None");
    assert!(!declaration.required);
    assert!(!declaration.multiple);
    assert!(declaration.repeat);
    assert!(!declaration.secret);
    assert_eq!(
        declaration.choices,
        ["True", "False", "None", "3", "[\"x\"]", "{\"k\":\"v\"}"]
    );
}

#[test]
fn nonscalar_defaults_are_dropped_instead_of_escaping_the_boundary() {
    for default in [json!(null), json!([]), json!({"nested": true})] {
        let declaration = ParamDecl::from_meta_map(&map(json!({
            "name": "x",
            "default": default
        })));
        assert_eq!(declaration.default, None);
    }
}
