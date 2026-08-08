use std::collections::BTreeMap;

use skit_application::{
    delivery::PreparedValue,
    value_preparation::{ValuePreparationError, prepare_values},
};
use skit_domain::parameters::{ParamDecl, ParameterType};

fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn required_fields_reject_missing_empty_and_whitespace_only_raw_input() {
    let mut field = ParamDecl::new("count");
    field.required = true;
    field.parameter_type = ParameterType::Int;
    field.prompt = "How many?".to_owned();

    for raw in [
        BTreeMap::new(),
        map(&[("count", "")]),
        map(&[("count", "   ")]),
    ] {
        let error = prepare_values(&[field.clone()], &raw, &map(&[("count", "")])).unwrap_err();
        assert_eq!(
            error,
            ValuePreparationError::Required {
                name: "count".to_owned(),
                label: "How many?".to_owned(),
            }
        );
    }
}

#[test]
fn every_typed_scalar_uses_the_shared_strict_coercion_contract() {
    let mut integer = ParamDecl::new("integer");
    integer.parameter_type = ParameterType::Int;
    let mut float = ParamDecl::new("float");
    float.parameter_type = ParameterType::Float;
    let mut boolean = ParamDecl::new("boolean");
    boolean.parameter_type = ParameterType::Bool;

    let prepared = prepare_values(
        &[integer.clone(), float.clone(), boolean.clone()],
        &map(&[("integer", "-7"), ("float", "1.25"), ("boolean", "on")]),
        &map(&[("integer", "-7"), ("float", "1.25"), ("boolean", "on")]),
    )
    .unwrap();
    assert_eq!(prepared["integer"], PreparedValue::Scalar("-7".to_owned()));
    assert_eq!(prepared["float"], PreparedValue::Scalar("1.25".to_owned()));
    assert_eq!(prepared["boolean"], PreparedValue::Scalar("on".to_owned()));

    for (field, value) in [(integer, "7.1"), (float, "NaN"), (boolean, "maybe")] {
        let error = prepare_values(
            std::slice::from_ref(&field),
            &map(&[(&field.name, value)]),
            &map(&[(&field.name, value)]),
        )
        .unwrap_err();
        assert_eq!(
            error,
            ValuePreparationError::InvalidType {
                name: field.name.clone(),
                value: value.to_owned(),
                parameter_type: field.parameter_type,
            }
        );
    }
}

#[test]
fn choices_are_exact_and_empty_optional_choices_remain_unset() {
    let mut field = ParamDecl::new("mode");
    field.parameter_type = ParameterType::Choice;
    field.choices = vec!["fast".to_owned(), "safe".to_owned()];

    assert_eq!(
        prepare_values(
            &[field.clone()],
            &map(&[("mode", "fast")]),
            &map(&[("mode", "fast")]),
        )
        .unwrap()["mode"],
        PreparedValue::Scalar("fast".to_owned())
    );
    assert_eq!(
        prepare_values(&[field.clone()], &BTreeMap::new(), &map(&[("mode", "")])).unwrap()["mode"],
        PreparedValue::Scalar(String::new())
    );
    assert_eq!(
        prepare_values(
            &[field],
            &map(&[("mode", "FAST")]),
            &map(&[("mode", "FAST")]),
        )
        .unwrap_err(),
        ValuePreparationError::InvalidChoice {
            name: "mode".to_owned(),
            value: "FAST".to_owned(),
            choices: vec!["fast".to_owned(), "safe".to_owned()],
        }
    );
}

#[test]
fn multiple_values_use_posix_shlex_and_validate_each_numeric_piece() {
    let mut words = ParamDecl::new("words");
    words.multiple = true;

    let mut points = ParamDecl::new("points");
    points.multiple = true;
    points.parameter_type = ParameterType::Int;

    let prepared = prepare_values(
        &[words, points.clone()],
        &map(&[("words", "'alpha beta' gamma"), ("points", "1 '2' -3")]),
        &map(&[("words", "'alpha beta' gamma"), ("points", "1 '2' -3")]),
    )
    .unwrap();

    assert_eq!(
        prepared["words"],
        PreparedValue::Multiple(vec!["alpha beta".to_owned(), "gamma".to_owned()])
    );
    assert_eq!(
        prepared["points"],
        PreparedValue::Multiple(vec!["1".to_owned(), "2".to_owned(), "-3".to_owned()])
    );

    let error = prepare_values(
        &[points],
        &map(&[("points", "1 nope")]),
        &map(&[("points", "1 nope")]),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ValuePreparationError::InvalidType {
            name: "points".to_owned(),
            value: "nope".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn an_unbalanced_quote_is_one_literal_multi_value_instead_of_a_parser_failure() {
    let mut field = ParamDecl::new("words");
    field.multiple = true;

    assert_eq!(
        prepare_values(
            &[field],
            &map(&[("words", "'alpha beta")]),
            &map(&[("words", "'alpha beta")]),
        )
        .unwrap()["words"],
        PreparedValue::Multiple(vec!["'alpha beta".to_owned()])
    );
}

#[test]
fn token_bearing_values_are_type_checked_after_resolution_not_before() {
    let mut field = ParamDecl::new("port");
    field.parameter_type = ParameterType::Int;

    assert_eq!(
        prepare_values(
            &[field.clone()],
            &map(&[("port", "{env:PORT}")]),
            &map(&[("port", "8080")]),
        )
        .unwrap()["port"],
        PreparedValue::Scalar("8080".to_owned())
    );
    assert_eq!(
        prepare_values(
            &[field],
            &map(&[("port", "{env:PORT}")]),
            &map(&[("port", "not-a-port")]),
        )
        .unwrap_err(),
        ValuePreparationError::InvalidType {
            name: "port".to_owned(),
            value: "not-a-port".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn degraded_fields_validate_as_free_text_and_missing_multi_values_become_empty_lists() {
    let mut degraded = ParamDecl::new("dynamic");
    degraded.parameter_type = ParameterType::Int;
    degraded.degraded = true;

    let mut multiple = ParamDecl::new("files");
    multiple.multiple = true;

    let prepared = prepare_values(
        &[degraded, multiple],
        &map(&[("dynamic", "not-an-int")]),
        &map(&[("dynamic", "not-an-int"), ("files", "")]),
    )
    .unwrap();

    assert_eq!(
        prepared["dynamic"],
        PreparedValue::Scalar("not-an-int".to_owned())
    );
    assert_eq!(prepared["files"], PreparedValue::Multiple(Vec::new()));
}

#[test]
fn secret_token_spelling_is_literal_and_is_not_validated_after_resolution() {
    let mut field = ParamDecl::new("port");
    field.parameter_type = ParameterType::Int;
    field.secret = true;

    let prepared = prepare_values(
        &[field],
        &map(&[("port", "{env:PORT}")]),
        &map(&[("port", "not-an-int")]),
    )
    .unwrap();
    assert_eq!(
        prepared["port"],
        PreparedValue::Scalar("not-an-int".to_owned())
    );
}
