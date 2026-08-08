use std::collections::BTreeMap;

use skit_application::delivery::{AssemblyError, PreparedValue, assemble};
use skit_domain::parameters::{
    ParamDecl, ParameterDelivery, ParameterType, ParameterValue,
};

fn scalar(value: &str) -> PreparedValue {
    PreparedValue::Scalar(value.to_owned())
}

fn values(items: &[(&str, PreparedValue)]) -> BTreeMap<String, PreparedValue> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

#[test]
fn mixed_delivery_routes_each_field_without_cross_contamination() {
    let mut positional = ParamDecl::new("input");
    positional.delivery = ParameterDelivery::Flag;

    let mut output = ParamDecl::new("output");
    output.delivery = ParameterDelivery::Flag;
    output.flag = "--output".to_owned();

    let mut city = ParamDecl::new("CITY");
    city.delivery = ParameterDelivery::Inject;

    let mut port = ParamDecl::new("port");
    port.delivery = ParameterDelivery::Env;
    port.env_target = "APP_PORT".to_owned();

    let mut slot = ParamDecl::new("subject");
    slot.delivery = ParameterDelivery::Placeholder;

    let plan = assemble(
        &[output, city, positional, port, slot],
        &values(&[
            ("output", scalar("result.txt")),
            ("CITY", scalar("Paris")),
            ("input", scalar("source.txt")),
            ("port", scalar("8080")),
            ("subject", scalar("release notes")),
        ]),
        &["--literal-tail".to_owned()],
    )
    .unwrap();

    assert_eq!(
        plan.args,
        ["source.txt", "--output", "result.txt", "--literal-tail"]
    );
    assert_eq!(plan.masked_args, plan.args);
    assert_eq!(plan.inject_values, BTreeMap::from([("CITY".to_owned(), "Paris".to_owned())]));
    assert_eq!(plan.env_values, BTreeMap::from([("APP_PORT".to_owned(), "8080".to_owned())]));
    assert_eq!(
        plan.command_values,
        BTreeMap::from([("subject".to_owned(), "release notes".to_owned())])
    );
    assert_eq!(plan.display, vec![("CITY".to_owned(), "Paris".to_owned())]);
}

#[test]
fn secrets_remain_real_for_execution_but_are_masked_on_every_display_surface() {
    let mut flag = ParamDecl::new("api_key");
    flag.flag = "--api-key".to_owned();
    flag.secret = true;

    let mut env = ParamDecl::new("token");
    env.delivery = ParameterDelivery::Env;
    env.env_target = "TOKEN".to_owned();
    env.secret = true;

    let mut placeholder = ParamDecl::new("password");
    placeholder.delivery = ParameterDelivery::Placeholder;
    placeholder.secret = true;

    let mut inject = ParamDecl::new("SECRET");
    inject.delivery = ParameterDelivery::Inject;
    inject.secret = true;

    let plan = assemble(
        &[flag, env, placeholder, inject],
        &values(&[
            ("api_key", scalar("flag-secret")),
            ("token", scalar("env-secret")),
            ("password", scalar("body-secret")),
            ("SECRET", scalar("inject-secret")),
        ]),
        &[],
    )
    .unwrap();

    assert_eq!(plan.args, ["--api-key", "flag-secret"]);
    assert_eq!(plan.masked_args, ["--api-key", "•••"]);
    assert_eq!(plan.env_values["TOKEN"], "env-secret");
    assert_eq!(plan.masked_env["TOKEN"], "•••");
    assert_eq!(plan.command_values["password"], "body-secret");
    assert_eq!(plan.masked_command_values["password"], "•••");
    assert_eq!(plan.inject_values["SECRET"], "inject-secret");
    assert_eq!(plan.display, vec![("SECRET".to_owned(), "•••".to_owned())]);
}

#[test]
fn boolean_flags_fire_only_for_the_action_state_and_flagless_bools_emit_nothing() {
    let mut enable = ParamDecl::new("enable");
    enable.parameter_type = ParameterType::Bool;
    enable.flag = "--enable".to_owned();
    enable.action = "store_true".to_owned();

    let mut disable = ParamDecl::new("cache");
    disable.parameter_type = ParameterType::Bool;
    disable.flag = "--no-cache".to_owned();
    disable.action = "store_false".to_owned();

    let mut positional_bool = ParamDecl::new("truth");
    positional_bool.parameter_type = ParameterType::Bool;

    let plan = assemble(
        &[enable, disable, positional_bool],
        &values(&[
            ("enable", scalar("on")),
            ("cache", scalar("false")),
            ("truth", scalar("true")),
        ]),
        &[],
    )
    .unwrap();

    assert_eq!(plan.args, ["--enable", "--no-cache"]);
}

#[test]
fn multiple_flags_preserve_argparse_and_repeat_per_piece_shapes() {
    let mut points = ParamDecl::new("point");
    points.flag = "--point".to_owned();
    points.multiple = true;

    let mut tags = ParamDecl::new("tag");
    tags.flag = "--tag".to_owned();
    tags.multiple = true;
    tags.repeat = true;

    let mut files = ParamDecl::new("files");
    files.multiple = true;

    let plan = assemble(
        &[points, tags, files],
        &values(&[
            (
                "point",
                PreparedValue::Multiple(vec!["1".to_owned(), "2".to_owned()]),
            ),
            (
                "tag",
                PreparedValue::Multiple(vec!["red".to_owned(), "blue".to_owned()]),
            ),
            (
                "files",
                PreparedValue::Multiple(vec!["a.txt".to_owned(), "b.txt".to_owned()]),
            ),
        ]),
        &[],
    )
    .unwrap();

    assert_eq!(
        plan.args,
        [
            "a.txt",
            "b.txt",
            "--point",
            "1",
            "2",
            "--tag",
            "red",
            "--tag",
            "blue",
        ]
    );
}

#[test]
fn known_string_defaults_make_cleared_values_explicit_but_other_empty_values_stay_unset() {
    let mut inject = ParamDecl::new("CITY");
    inject.delivery = ParameterDelivery::Inject;
    inject.default = Some(ParameterValue::String("Paris".to_owned()));

    let mut env = ParamDecl::new("NAME");
    env.delivery = ParameterDelivery::Env;
    env.default = Some(ParameterValue::String("Ada".to_owned()));

    let mut flag = ParamDecl::new("output");
    flag.flag = "--output".to_owned();
    flag.default = Some(ParameterValue::String("out.txt".to_owned()));

    let mut optional = ParamDecl::new("optional");
    optional.flag = "--optional".to_owned();

    let mut secret = ParamDecl::new("secret");
    secret.delivery = ParameterDelivery::Env;
    secret.secret = true;
    secret.default = Some(ParameterValue::String("must-not-matter".to_owned()));

    let plan = assemble(
        &[inject, env, flag, optional, secret],
        &values(&[
            ("CITY", scalar("")),
            ("NAME", scalar("")),
            ("output", scalar("")),
            ("optional", scalar("")),
            ("secret", scalar("")),
        ]),
        &[],
    )
    .unwrap();

    assert_eq!(plan.inject_values["CITY"], "");
    assert_eq!(plan.display, vec![("CITY".to_owned(), "''".to_owned())]);
    assert_eq!(plan.env_values["NAME"], "");
    assert!(!plan.env_values.contains_key("secret"));
    assert_eq!(plan.args, ["--output", ""]);
}

#[test]
fn a_multiple_shape_on_a_non_multiple_field_is_refused_instead_of_guessed() {
    let declaration = ParamDecl::new("name");
    let error = assemble(
        &[declaration],
        &values(&[(
            "name",
            PreparedValue::Multiple(vec!["one".to_owned(), "two".to_owned()]),
        )]),
        &[],
    )
    .unwrap_err();

    assert_eq!(
        error,
        AssemblyError::UnexpectedMultiple {
            name: "name".to_owned(),
        }
    );
}
