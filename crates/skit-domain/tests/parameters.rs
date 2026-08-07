use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterInvariant, ParameterType,
    ParameterValue, coerce_default,
};

fn map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn block_shapes_are_frozen_and_round_trip_with_implied_delivery() {
    let constant = ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        default: Some(ParameterValue::String("xxx".to_owned())),
        secret: true,
        ..ParamDecl::new("API_KEY")
    };
    assert_eq!(
        constant.to_block_map(),
        map(json!({
            "name": "API_KEY",
            "kind": "const",
            "type": "str",
            "default": "xxx",
            "secret": true
        }))
    );
    assert_eq!(
        ParamDecl::from_block_map(&constant.to_block_map()),
        constant
    );

    let input = ParamDecl {
        binding: ParameterBinding::Input,
        delivery: ParameterDelivery::Inject,
        prompt: "Name: ".to_owned(),
        order: 0,
        env_source: "MY_NAME".to_owned(),
        ..ParamDecl::new("input-1")
    };
    assert_eq!(
        input.to_block_map(),
        map(json!({
            "name": "input-1",
            "kind": "input",
            "type": "str",
            "prompt": "Name: ",
            "order": 0,
            "env_source": "MY_NAME"
        }))
    );
    assert_eq!(ParamDecl::from_block_map(&input.to_block_map()), input);

    let environment = ParamDecl::from_block_map(&map(json!({
        "name": "V",
        "kind": "envdefault",
        "default": "x"
    })));
    assert_eq!(environment.binding, ParameterBinding::EnvDefault);
    assert_eq!(environment.delivery, ParameterDelivery::Env);
}

#[test]
fn block_decoding_is_total_on_hand_edited_garbage() {
    let declaration = ParamDecl::from_block_map(&map(json!({
        "name": 5,
        "kind": "martian",
        "type": [],
        "order": "NaN",
        "default": {"t": 1}
    })));

    assert_eq!(declaration.name, "5");
    assert_eq!(declaration.binding, ParameterBinding::Const);
    assert_eq!(declaration.parameter_type, ParameterType::Str);
    assert_eq!(declaration.order, -1);
    assert_eq!(declaration.default, None);
}

#[test]
fn meta_shape_round_trips_the_full_model_and_omits_defaults() {
    let declaration = ParamDecl {
        delivery: ParameterDelivery::Flag,
        parameter_type: ParameterType::Choice,
        default: Some(ParameterValue::String("800".to_owned())),
        required: true,
        multiple: true,
        repeat: true,
        choices: vec!["400".to_owned(), "800".to_owned()],
        prompt: "Width".to_owned(),
        help: "output width".to_owned(),
        flag: "--width".to_owned(),
        ..ParamDecl::new("width")
    };
    let encoded = declaration.to_meta_map();
    assert_eq!(encoded.get("repeat"), Some(&Value::Bool(true)));
    assert_eq!(ParamDecl::from_meta_map(&encoded), declaration);

    assert_eq!(
        ParamDecl::new("x").to_meta_map(),
        map(json!({"name": "x", "delivery": "flag", "type": "str"}))
    );
    assert!(!ParamDecl::new("x").to_meta_map().contains_key("repeat"));
}

#[test]
fn meta_decoding_is_total_and_normalizes_truthy_booleans() {
    let declaration = ParamDecl::from_meta_map(&map(json!({
        "name": "x",
        "delivery": "carrier-pigeon",
        "choices": "abc",
        "order": null,
        "repeat": 1
    })));

    assert_eq!(declaration.delivery, ParameterDelivery::Flag);
    assert!(declaration.choices.is_empty());
    assert_eq!(declaration.order, -1);
    assert!(declaration.repeat);
}

#[test]
fn env_target_defaults_to_the_parameter_name() {
    assert_eq!(
        ParamDecl {
            delivery: ParameterDelivery::Env,
            ..ParamDecl::new("WIDTH")
        }
        .env_var(),
        "WIDTH"
    );
    assert_eq!(
        ParamDecl {
            delivery: ParameterDelivery::Env,
            env_target: "WIDTH_PX".to_owned(),
            ..ParamDecl::new("width")
        }
        .env_var(),
        "WIDTH_PX"
    );
}

#[test]
fn invariants_are_symbolic_and_normalization_repairs_only_implied_delivery() {
    let valid = ParamDecl {
        binding: ParameterBinding::Const,
        delivery: ParameterDelivery::Inject,
        ..ParamDecl::new("a")
    };
    assert_eq!(valid.validate(), None);

    let mismatched = ParamDecl {
        binding: ParameterBinding::EnvDefault,
        delivery: ParameterDelivery::Flag,
        ..ParamDecl::new("a")
    };
    assert_eq!(
        mismatched.validate(),
        Some(ParameterInvariant::BindingDeliveryMismatch)
    );
    assert_eq!(mismatched.normalized().delivery, ParameterDelivery::Env);

    let choice = ParamDecl {
        parameter_type: ParameterType::Choice,
        ..ParamDecl::new("choice")
    };
    assert_eq!(
        choice.validate(),
        Some(ParameterInvariant::ChoiceWithoutChoices)
    );

    let free = ParamDecl {
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("free")
    };
    assert_eq!(free.clone().normalized(), free);
}

#[test]
fn typed_defaults_share_one_strict_coercion_contract() {
    assert_eq!(
        coerce_default("42", ParameterType::Int).unwrap(),
        ParameterValue::Integer(42)
    );
    assert_eq!(
        coerce_default("2.5", ParameterType::Float).unwrap(),
        ParameterValue::Float(2.5)
    );
    assert_eq!(
        coerce_default("YES", ParameterType::Bool).unwrap(),
        ParameterValue::Bool(true)
    );
    assert_eq!(
        coerce_default("off", ParameterType::Bool).unwrap(),
        ParameterValue::Bool(false)
    );
    assert_eq!(
        coerce_default("relative/path", ParameterType::Path).unwrap(),
        ParameterValue::String("relative/path".to_owned())
    );
    assert!(coerce_default("NaN", ParameterType::Float).is_err());
    assert!(coerce_default("inf", ParameterType::Float).is_err());
    assert!(coerce_default("perhaps", ParameterType::Bool).is_err());
}
