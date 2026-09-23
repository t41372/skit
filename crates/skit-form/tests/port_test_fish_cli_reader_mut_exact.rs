//! Exact public-surface ports of Python v0.4 `tests/test_fish_cli_reader_mut.py`.
//!
//! All eight Python tests are observable through Fish's published argparse projection. The Python
//! SIGALRM helper only prevents a mutated implementation from hanging; the Rust test exercises the
//! real reader directly and must complete with the exact projected field shape.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParameterBinding, ParameterDelivery, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, onboarding_plan};

fn fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    match onboarding_plan("fish", source).cli_surface {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "argparse");
            fields
        }
        other => panic!("expected static Fish argparse surface: {other:?}"),
    }
}

fn by_name(
    fields: &[skit_domain::parameters::ParamDecl],
) -> BTreeMap<&str, &skit_domain::parameters::ParamDecl> {
    fields
        .iter()
        .map(|field| (field.name.as_str(), field))
        .collect()
}

#[test]
fn test_find_argparse_skips_a_lone_leading_prefix() {
    let actual = fields("or\nargparse 'h/help' -- $argv\n");
    assert_eq!(
        actual
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["help"]
    );
}

#[test]
fn test_find_argparse_advances_past_every_stacked_prefix() {
    let actual = fields("or not argparse 'h/help' -- $argv\n");
    assert_eq!(
        actual
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["help"]
    );
}

#[test]
fn test_flag_spec_binding_and_delivery() {
    let actual = fields("argparse 'h/help' -- $argv\n");
    let [field] = actual.as_slice() else {
        panic!("expected one help field: {actual:?}");
    };
    assert_eq!(field.binding, ParameterBinding::None);
    assert_eq!(field.delivery, ParameterDelivery::Flag);
}

#[test]
fn test_valueless_flag_is_a_false_default_bool() {
    let actual = fields("argparse 'v/verbose' -- $argv\n");
    let field = &by_name(&actual)["verbose"];
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_single_required_value_flag_is_not_multiple() {
    let actual = fields("argparse 'n/name=' -- $argv\n");
    let field = &by_name(&actual)["name"];
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert!(!field.multiple);
}

#[test]
fn test_single_char_short_flag_is_not_degraded() {
    let actual = fields("argparse 'x' -- $argv\n");
    let [field] = actual.as_slice() else {
        panic!("expected one x field: {actual:?}");
    };
    assert_eq!(field.flag, "-x");
    assert!(!field.degraded);
    assert_eq!(field.parameter_type, ParameterType::Bool);
}

#[test]
fn test_plain_long_flag_is_not_degraded() {
    let actual = fields("argparse 'verbose' -- $argv\n");
    let [field] = actual.as_slice() else {
        panic!("expected one verbose field: {actual:?}");
    };
    assert_eq!(field.flag, "--verbose");
    assert!(!field.degraded);
    assert_eq!(field.parameter_type, ParameterType::Bool);
}

#[test]
fn test_validator_is_dropped_from_the_first_bang_forward() {
    let actual = fields("argparse 'verbose!a!b' -- $argv\n");
    assert_eq!(
        actual
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["verbose"]
    );
    assert_eq!(actual[0].flag, "--verbose");
}
