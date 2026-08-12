//! Public-surface ports of the executable contracts in Python v0.4
//! `tests/test_shell_cli_reader_mut.py`.
//!
//! Five contracts are observable through the published getopts projection. Two Python tests call
//! private helpers for states that the public grammar cannot expose; those are explicitly blocked
//! by the companion manifest rather than being replaced with weaker tests.

use std::collections::BTreeMap;

use skit_domain::parameters::{ParameterBinding, ParameterDelivery, ParameterType, ParameterValue};
use skit_form::{CliFormProjection, cli_form_projection};

fn fields(source: &str) -> Vec<skit_domain::parameters::ParamDecl> {
    match cli_form_projection("shell", source) {
        CliFormProjection::Static { framework, fields } => {
            assert_eq!(framework, "getopts");
            fields
        }
        other => panic!("expected static getopts surface: {other:?}"),
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
fn test_getopts_found_after_an_earlier_non_getopts_command() {
    let actual = fields("echo starting\nwhile getopts \"a:v\" opt; do :; done\n");
    let actual = by_name(&actual);
    assert_eq!(actual["a"].parameter_type, ParameterType::Str);
    assert_eq!(actual["v"].parameter_type, ParameterType::Bool);
}

#[test]
fn test_trailing_value_marker_makes_a_str_flag() {
    let actual = fields("while getopts \"vn:\" opt; do :; done\n");
    let actual = by_name(&actual);
    assert_eq!(actual["n"].parameter_type, ParameterType::Str);
    assert_eq!(actual["v"].parameter_type, ParameterType::Bool);
}

#[test]
fn test_repeated_letter_emits_exactly_one_field() {
    let actual = fields("while getopts \"vv\" opt; do :; done\n");
    assert_eq!(
        actual.iter().map(|field| field.name.as_str()).collect::<Vec<_>>(),
        ["v"]
    );
}

#[test]
fn test_option_binding_and_delivery_and_flag() {
    let actual = fields("while getopts \"x:\" opt; do :; done\n");
    let [field] = actual.as_slice() else {
        panic!("expected one x field: {actual:?}");
    };
    assert_eq!(field.binding, ParameterBinding::None);
    assert_eq!(field.delivery, ParameterDelivery::Flag);
    assert_eq!(field.flag, "-x");
}

#[test]
fn test_bool_flag_shape_from_a_bare_letter() {
    let actual = fields("while getopts \"n:v\" opt; do :; done\n");
    let actual = by_name(&actual);
    assert_eq!(actual["v"].parameter_type, ParameterType::Bool);
    assert_eq!(actual["v"].action, "store_true");
    assert_eq!(actual["v"].default, Some(ParameterValue::Bool(false)));
    assert_eq!(actual["n"].parameter_type, ParameterType::Str);
    assert_eq!(actual["n"].action, "");
}
