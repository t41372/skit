//! Exact-name domain ports from Python v0.4 `tests/test_declared_params.py`.
//!
//! These functions have direct public Rust counterparts, so no CLI translation or architectural
//! substitution is involved.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use skit_domain::parameters::{
    ParamDecl, ParameterDelivery, ParameterType, ParameterValue, declared_for_template,
    declared_from_meta, synthesized_placeholder,
};

fn map(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}

#[test]
fn test_undeclared_placeholders_synthesize_the_historical_field() {
    let placeholders = vec!["input".to_owned(), "api_key".to_owned()];
    let declarations = declared_for_template(None, &placeholders);
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["input", "api_key"]
    );
    assert!(
        declarations
            .iter()
            .all(|declaration| declaration.delivery == ParameterDelivery::Placeholder)
    );
    assert!(declarations.iter().all(|declaration| declaration.required));
    assert!(!declarations[0].secret);
    assert!(declarations[1].secret);
}

#[test]
fn test_declared_row_overrides_placeholder_schema_including_secret() {
    let rows = vec![ParamDecl {
        delivery: ParameterDelivery::Placeholder,
        parameter_type: ParameterType::Str,
        default: Some(ParameterValue::String("creds.json".to_owned())),
        required: false,
        secret: false,
        ..ParamDecl::new("token_file")
    }
    .to_meta_map()];
    let placeholders = vec!["token_file".to_owned(), "host".to_owned()];
    let declarations = declared_for_template(Some(&rows), &placeholders);
    assert_eq!(declarations[0].name, "token_file");
    assert_eq!(declarations[0].delivery, ParameterDelivery::Placeholder);
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("creds.json".to_owned()))
    );
    assert!(!declarations[0].required);
    assert!(!declarations[0].secret);
    assert_eq!(declarations[1].name, "host");
    assert!(declarations[1].required);
}

#[test]
fn test_declared_env_param_rides_along_after_placeholders() {
    let rows = vec![ParamDecl {
        delivery: ParameterDelivery::Env,
        parameter_type: ParameterType::Int,
        default: Some(ParameterValue::Integer(3)),
        ..ParamDecl::new("RETRIES")
    }
    .to_meta_map()];
    let declarations = declared_for_template(Some(&rows), &["file".to_owned()]);
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["file", "RETRIES"]
    );
    assert_eq!(declarations[1].delivery, ParameterDelivery::Env);
    assert_eq!(declarations[1].parameter_type, ParameterType::Int);
    assert_eq!(declarations[1].default, Some(ParameterValue::Integer(3)));
}

#[test]
fn test_declared_flag_row_is_dropped_for_templates() {
    let rows = vec![ParamDecl {
        delivery: ParameterDelivery::Flag,
        flag: "--width".to_owned(),
        ..ParamDecl::new("width")
    }
    .to_meta_map()];
    let declarations = declared_for_template(Some(&rows), &["file".to_owned()]);
    assert_eq!(
        declarations
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["file"]
    );
}

#[test]
fn test_declared_row_with_wrong_delivery_for_its_placeholder_is_replaced_by_synth() {
    let rows = vec![ParamDecl {
        delivery: ParameterDelivery::Env,
        ..ParamDecl::new("file")
    }
    .to_meta_map()];
    let declarations = declared_for_template(Some(&rows), &["file".to_owned()]);
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "file");
    assert_eq!(declarations[0].delivery, ParameterDelivery::Placeholder);
    assert!(declarations[0].required);
    assert_eq!(declarations[0].default, None);
}

#[test]
fn test_declared_from_meta_drops_nameless_rows() {
    let rows = vec![
        map(json!({"delivery": "flag"})),
        ParamDecl::new("ok").to_meta_map(),
    ];
    assert_eq!(
        declared_from_meta(Some(&rows))
            .iter()
            .map(|declaration| declaration.name.as_str())
            .collect::<Vec<_>>(),
        ["ok"]
    );
}

#[test]
fn test_synthesized_placeholder_shape() {
    let declaration = synthesized_placeholder("api_key");
    assert_eq!(declaration.delivery, ParameterDelivery::Placeholder);
    assert!(declaration.required);
    assert!(declaration.secret);

    let public = synthesized_placeholder("input");
    assert_eq!(public.delivery, ParameterDelivery::Placeholder);
    assert!(public.required);
    assert!(!public.secret);
}
