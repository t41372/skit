//! Public-API ports of Python v0.4 `tests/test_path_type.py` domain contracts.

use std::collections::BTreeMap;

use serde_json::Value;
use skit_domain::parameters::{
    ParamDecl, ParameterDelivery, ParameterType, ParameterValue, coerce_default,
};

#[test]
fn test_block_round_trip_carries_path_type() {
    let mut declaration = ParamDecl::new("SRC");
    declaration.parameter_type = ParameterType::Path;

    let decoded = ParamDecl::from_block_map(&declaration.to_block_map());
    assert_eq!(decoded.parameter_type, ParameterType::Path);
}

#[test]
fn test_meta_round_trip_carries_path_type() {
    let mut declaration = ParamDecl::new("src");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Path;

    let decoded = ParamDecl::from_meta_map(&declaration.to_meta_map());
    assert_eq!(decoded.parameter_type, ParameterType::Path);
}

#[test]
fn test_unknown_block_type_degrades_to_str() {
    let row = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("kind".to_owned(), Value::String("const".to_owned())),
        ("type".to_owned(), Value::String("pathlike".to_owned())),
    ]);

    assert_eq!(
        ParamDecl::from_block_map(&row).parameter_type,
        ParameterType::Str
    );
}

#[test]
fn test_unknown_meta_type_degrades_to_str() {
    let row = BTreeMap::from([
        ("name".to_owned(), Value::String("X".to_owned())),
        ("delivery".to_owned(), Value::String("flag".to_owned())),
        ("type".to_owned(), Value::String("pathlike".to_owned())),
    ]);

    assert_eq!(
        ParamDecl::from_meta_map(&row).parameter_type,
        ParameterType::Str
    );
}

#[test]
fn test_path_default_coercion_keeps_raw_string_without_existence_check() {
    assert_eq!(
        coerce_default("./no such file.csv", ParameterType::Path).unwrap(),
        ParameterValue::String("./no such file.csv".to_owned())
    );
}

#[test]
fn test_path_is_free_text_for_empty_and_arbitrary_values() {
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
