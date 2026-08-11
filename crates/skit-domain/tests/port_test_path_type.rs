//! Exact domain-layer ports of Python v0.4 `tests/test_path_type.py`.
//!
//! Frozen oracle: `origin/main@206f9ef946fc45835cb2479593794431f2620c32`.
//! Rust-only strengthening is explicitly named `rust_additive_*` and is not Python parity credit.

use std::collections::BTreeMap;

use serde_json::Value;
use skit_domain::parameters::{
    ParamDecl, ParameterDelivery, ParameterType, ParameterValue, coerce_default,
};

#[test]
fn test_path_is_an_allowed_type() {
    let row = BTreeMap::from([
        ("name".to_owned(), Value::String("SRC".to_owned())),
        ("type".to_owned(), Value::String("path".to_owned())),
    ]);
    let declaration = ParamDecl::from_meta_map(&row);
    assert_eq!(declaration.parameter_type, ParameterType::Path);
    assert_eq!(declaration.parameter_type.as_str(), "path");
}

#[test]
fn test_unknown_type_still_degrades_to_str() {
    let block = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("kind".to_owned(), Value::String("const".to_owned())),
        ("type".to_owned(), Value::String("pathlike".to_owned())),
    ]);
    let meta = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("type".to_owned(), Value::String("pathlike".to_owned())),
    ]);
    assert_eq!(
        ParamDecl::from_block_map(&block).parameter_type,
        ParameterType::Str
    );
    assert_eq!(
        ParamDecl::from_meta_map(&meta).parameter_type,
        ParameterType::Str
    );
}

#[test]
fn test_block_round_trip_carries_path() {
    let mut declaration = ParamDecl::new("SRC");
    declaration.parameter_type = ParameterType::Path;
    let decoded = ParamDecl::from_block_map(&declaration.to_block_map());
    assert_eq!(decoded.parameter_type, ParameterType::Path);
}

#[test]
fn test_meta_round_trip_carries_path() {
    let mut declaration = ParamDecl::new("src");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Path;
    let decoded = ParamDecl::from_meta_map(&declaration.to_meta_map());
    assert_eq!(decoded.parameter_type, ParameterType::Path);
}

#[test]
fn test_coerce_default_path_keeps_raw_string() {
    assert_eq!(
        coerce_default("./no such file.csv", ParameterType::Path).unwrap(),
        ParameterValue::String("./no such file.csv".to_owned())
    );
}

#[test]
fn rust_additive_path_coercion_never_checks_existence_or_platform_spelling() {
    for value in [
        "",
        "../relative.csv",
        "/does/not/exist",
        "C:\\future\\file.txt",
    ] {
        assert_eq!(
            coerce_default(value, ParameterType::Path).unwrap(),
            ParameterValue::String(value.to_owned())
        );
    }
}
