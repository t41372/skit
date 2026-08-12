//! Exact public-domain ports of executable Python v0.4 `tests/test_params_model.py` contracts.
//!
//! Frozen block/meta shapes are compared exactly. Hand-edited garbage decoders are total. The one
//! Python `field_replace` helper has no equivalent Rust product API and is guarded as blocked in a
//! cross-file manifest rather than being faked with `Clone` plus direct field mutation.

use std::collections::BTreeMap;

use serde_json::{Number, Value};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterInvariant, ParameterType,
    ParameterValue,
};

#[test]
fn test_block_dict_const_shape_is_frozen() {
    let mut declaration = ParamDecl::new("API_KEY");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String("xxx".to_owned()));
    declaration.secret = true;

    assert_eq!(
        declaration.to_block_map(),
        BTreeMap::from([
            ("default".to_owned(), Value::String("xxx".to_owned())),
            ("kind".to_owned(), Value::String("const".to_owned())),
            ("name".to_owned(), Value::String("API_KEY".to_owned())),
            ("secret".to_owned(), Value::Bool(true)),
            ("type".to_owned(), Value::String("str".to_owned())),
        ])
    );
}

#[test]
fn test_block_dict_input_shape_is_frozen() {
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.prompt = "Name: ".to_owned();
    declaration.order = 0;
    declaration.env_source = "MY_NAME".to_owned();

    assert_eq!(
        declaration.to_block_map(),
        BTreeMap::from([
            ("env_source".to_owned(), Value::String("MY_NAME".to_owned())),
            ("kind".to_owned(), Value::String("input".to_owned())),
            ("name".to_owned(), Value::String("input-1".to_owned())),
            ("order".to_owned(), Value::Number(Number::from(0))),
            ("prompt".to_owned(), Value::String("Name: ".to_owned())),
            ("type".to_owned(), Value::String("str".to_owned())),
        ])
    );
}

#[test]
fn test_block_roundtrip_derives_delivery_from_binding() {
    let mut source = ParamDecl::new("N");
    source.binding = ParameterBinding::Const;
    source.delivery = ParameterDelivery::Inject;
    source.parameter_type = ParameterType::Int;
    source.default = Some(ParameterValue::Integer(3));
    assert_eq!(ParamDecl::from_block_map(&source.to_block_map()), source);

    let decoded = ParamDecl::from_block_map(&BTreeMap::from([
        ("name".to_owned(), Value::String("V".to_owned())),
        ("kind".to_owned(), Value::String("envdefault".to_owned())),
        ("default".to_owned(), Value::String("x".to_owned())),
    ]));
    assert_eq!(decoded.binding, ParameterBinding::EnvDefault);
    assert_eq!(decoded.delivery, ParameterDelivery::Env);
}

#[test]
fn test_from_block_dict_is_total_on_garbage() {
    let decoded = ParamDecl::from_block_map(&BTreeMap::from([
        ("name".to_owned(), Value::Number(Number::from(5))),
        ("kind".to_owned(), Value::String("martian".to_owned())),
        ("type".to_owned(), Value::Array(Vec::new())),
        ("order".to_owned(), Value::String("NaN".to_owned())),
        (
            "default".to_owned(),
            Value::Object(serde_json::Map::from_iter([(
                "t".to_owned(),
                Value::Number(Number::from(1)),
            )])),
        ),
    ]));

    assert_eq!(decoded.name, "5");
    assert_eq!(decoded.binding, ParameterBinding::Const);
    assert_eq!(decoded.parameter_type, ParameterType::Str);
    assert_eq!(decoded.order, -1);
    assert_eq!(decoded.default, None);
}

#[test]
fn test_meta_roundtrip_full_model() {
    let mut source = ParamDecl::new("width");
    source.binding = ParameterBinding::None;
    source.delivery = ParameterDelivery::Flag;
    source.parameter_type = ParameterType::Choice;
    source.default = Some(ParameterValue::String("800".to_owned()));
    source.required = true;
    source.multiple = true;
    source.choices = vec!["400".to_owned(), "800".to_owned()];
    source.prompt = "Width".to_owned();
    source.help = "output width".to_owned();
    source.secret = false;
    source.flag = "--width".to_owned();
    source.action.clear();
    source.env_target.clear();

    assert_eq!(ParamDecl::from_meta_map(&source.to_meta_map()), source);
}

#[test]
fn test_meta_dict_omits_defaults() {
    assert_eq!(
        ParamDecl::new("x").to_meta_map(),
        BTreeMap::from([
            ("delivery".to_owned(), Value::String("flag".to_owned())),
            ("name".to_owned(), Value::String("x".to_owned())),
            ("type".to_owned(), Value::String("str".to_owned())),
        ])
    );
}

#[test]
fn test_meta_dict_omits_repeat_when_false() {
    assert!(!ParamDecl::new("x").to_meta_map().contains_key("repeat"));
}

#[test]
fn test_meta_dict_repeat_emitted_and_roundtrips_only_when_set() {
    let mut source = ParamDecl::new("tag");
    source.delivery = ParameterDelivery::Flag;
    source.flag = "--tag".to_owned();
    source.multiple = true;
    source.repeat = true;
    let encoded = source.to_meta_map();

    assert_eq!(encoded.get("repeat"), Some(&Value::Bool(true)));
    let decoded = ParamDecl::from_meta_map(&encoded);
    assert_eq!(decoded, source);
    assert!(decoded.repeat);
}

#[test]
fn test_from_meta_dict_repeat_defaults_false_when_absent() {
    let decoded = ParamDecl::from_meta_map(&BTreeMap::from([
        ("name".to_owned(), Value::String("x".to_owned())),
        ("delivery".to_owned(), Value::String("flag".to_owned())),
    ]));
    assert!(!decoded.repeat);
}

#[test]
fn test_from_meta_dict_repeat_coerces_truthy_to_bool() {
    let decoded = ParamDecl::from_meta_map(&BTreeMap::from([
        ("name".to_owned(), Value::String("x".to_owned())),
        ("delivery".to_owned(), Value::String("flag".to_owned())),
        ("repeat".to_owned(), Value::Number(Number::from(1))),
    ]));
    assert!(decoded.repeat);
}

#[test]
fn test_meta_dict_includes_binding_and_order_when_set() {
    let mut source = ParamDecl::new("input-1");
    source.binding = ParameterBinding::Input;
    source.delivery = ParameterDelivery::Inject;
    source.order = 2;
    let encoded = source.to_meta_map();

    assert_eq!(
        encoded.get("binding"),
        Some(&Value::String("input".to_owned()))
    );
    assert_eq!(encoded.get("order"), Some(&Value::Number(Number::from(2))));
    assert_eq!(ParamDecl::from_meta_map(&encoded), source);
}

#[test]
fn test_meta_roundtrip_env_delivery_and_target() {
    let mut source = ParamDecl::new("width");
    source.delivery = ParameterDelivery::Env;
    source.env_target = "WIDTH_PX".to_owned();
    source.secret = true;

    let decoded = ParamDecl::from_meta_map(&source.to_meta_map());
    assert_eq!(decoded, source);
    assert_eq!(decoded.env_var(), "WIDTH_PX");
}

#[test]
fn test_from_meta_dict_is_total_on_garbage() {
    let decoded = ParamDecl::from_meta_map(&BTreeMap::from([
        ("name".to_owned(), Value::String("x".to_owned())),
        (
            "delivery".to_owned(),
            Value::String("carrier-pigeon".to_owned()),
        ),
        ("choices".to_owned(), Value::String("abc".to_owned())),
        ("order".to_owned(), Value::Null),
    ]));

    assert_eq!(decoded.delivery, ParameterDelivery::Flag);
    assert!(decoded.choices.is_empty());
    assert_eq!(decoded.order, -1);
}

#[test]
fn test_env_var_defaults_to_name() {
    let mut direct = ParamDecl::new("WIDTH");
    direct.delivery = ParameterDelivery::Env;
    assert_eq!(direct.env_var(), "WIDTH");

    let mut targeted = ParamDecl::new("w");
    targeted.delivery = ParameterDelivery::Env;
    targeted.env_target = "WIDTH".to_owned();
    assert_eq!(targeted.env_var(), "WIDTH");
}

#[test]
fn test_invariants_binding_implies_delivery() {
    let mut ok = ParamDecl::new("a");
    ok.binding = ParameterBinding::Const;
    ok.delivery = ParameterDelivery::Inject;
    assert_eq!(ok.validate(), None);

    let mut bad = ok.clone();
    bad.delivery = ParameterDelivery::Env;
    assert_eq!(
        bad.validate(),
        Some(ParameterInvariant::BindingDeliveryMismatch)
    );

    let mut envd = ParamDecl::new("a");
    envd.binding = ParameterBinding::EnvDefault;
    envd.delivery = ParameterDelivery::Flag;
    assert_eq!(
        envd.validate(),
        Some(ParameterInvariant::BindingDeliveryMismatch)
    );

    let mut free = ParamDecl::new("a");
    free.binding = ParameterBinding::None;
    free.delivery = ParameterDelivery::Env;
    assert_eq!(free.validate(), None);
}

#[test]
fn test_invariants_choice_needs_choices() {
    let mut choice = ParamDecl::new("a");
    choice.parameter_type = ParameterType::Choice;
    assert_eq!(
        choice.validate(),
        Some(ParameterInvariant::ChoiceWithoutChoices)
    );
    choice.choices.push("x".to_owned());
    assert_eq!(choice.validate(), None);
}

#[test]
fn test_normalize_repairs_delivery_from_binding() {
    let mut bad = ParamDecl::new("a");
    bad.binding = ParameterBinding::EnvDefault;
    bad.delivery = ParameterDelivery::Flag;
    let fixed = bad.normalized();
    assert_eq!(fixed.delivery, ParameterDelivery::Env);

    let mut free = ParamDecl::new("b");
    free.binding = ParameterBinding::None;
    free.delivery = ParameterDelivery::Env;
    assert_eq!(free.clone().normalized(), free);
}

#[test]
fn rust_additive_block_order_coercion_keeps_numeric_string_and_truncates_float_but_rejects_garbage()
{
    for (value, expected) in [
        (Value::String("3".to_owned()), 3),
        (
            Value::Number(Number::from_f64(1.9).expect("finite number")),
            1,
        ),
        (Value::String("abc".to_owned()), -1),
        (Value::Null, -1),
    ] {
        let input = BTreeMap::from([
            ("name".to_owned(), Value::String("X".to_owned())),
            ("order".to_owned(), value),
        ]);
        assert_eq!(ParamDecl::from_block_map(&input).order, expected);
    }
}
