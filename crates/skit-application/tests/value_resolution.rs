use std::collections::BTreeMap;

use skit_application::{
    tokens::{TokenContext, TokenError},
    value_resolution::{ValueResolutionError, resolve_values},
};
use skit_domain::parameters::{ParamDecl, ParameterDelivery};

fn context() -> TokenContext {
    TokenContext {
        cwd: "/work/dir".to_owned(),
        home: Some("/home/user".to_owned()),
        env: BTreeMap::from([
            ("API_KEY".to_owned(), "from-env".to_owned()),
            ("REGION".to_owned(), "eu-west".to_owned()),
        ]),
        today: "2026-08-07".to_owned(),
        now: "17-22-00".to_owned(),
    }
}

fn raw(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn typed_secret_wins_over_env_source_and_is_never_token_expanded() {
    let mut secret = ParamDecl::new("api_key");
    secret.secret = true;
    secret.env_source = "API_KEY".to_owned();

    let values = resolve_values(
        &[secret],
        &raw(&[("api_key", "literal-{env:REGION}-{{cwd}}")]),
        &context(),
    )
    .unwrap();

    assert_eq!(values["api_key"], "literal-{env:REGION}-{{cwd}}");
}

#[test]
fn an_empty_secret_reads_its_named_environment_source() {
    let mut secret = ParamDecl::new("api_key");
    secret.secret = true;
    secret.env_source = "API_KEY".to_owned();

    let values = resolve_values(&[secret], &BTreeMap::new(), &context()).unwrap();

    assert_eq!(values["api_key"], "from-env");
}

#[test]
fn a_missing_secret_environment_source_is_a_named_structured_error() {
    let mut secret = ParamDecl::new("api_key");
    secret.secret = true;
    secret.env_source = "MISSING_KEY".to_owned();

    let error = resolve_values(&[secret], &BTreeMap::new(), &context()).unwrap_err();

    assert_eq!(
        error,
        ValueResolutionError::MissingSecretEnvironment {
            name: "api_key".to_owned(),
            environment: "MISSING_KEY".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "api_key reads from the environment variable MISSING_KEY, but it isn't set."
    );
}

#[test]
fn nonsecret_values_expand_named_tokens_for_every_delivery() {
    let mut inject = ParamDecl::new("inject");
    inject.delivery = ParameterDelivery::Inject;
    let mut env = ParamDecl::new("env");
    env.delivery = ParameterDelivery::Env;
    let mut flag = ParamDecl::new("flag");
    flag.delivery = ParameterDelivery::Flag;
    let mut placeholder = ParamDecl::new("placeholder");
    placeholder.delivery = ParameterDelivery::Placeholder;

    let values = resolve_values(
        &[inject, env, flag, placeholder],
        &raw(&[
            ("inject", "{cwd}/{today}"),
            ("env", "{env:REGION}"),
            ("flag", "~/out_{now}.txt"),
            ("placeholder", "{cwd}/{env:REGION}"),
        ]),
        &context(),
    )
    .unwrap();

    assert_eq!(values["inject"], "/work/dir/2026-08-07");
    assert_eq!(values["env"], "eu-west");
    assert_eq!(values["flag"], "/home/user/out_17-22-00.txt");
    assert_eq!(values["placeholder"], "/work/dir/eu-west");
}

#[test]
fn placeholder_values_keep_double_braces_while_other_deliveries_halve_them() {
    let mut placeholder = ParamDecl::new("placeholder");
    placeholder.delivery = ParameterDelivery::Placeholder;
    let mut flag = ParamDecl::new("flag");
    flag.delivery = ParameterDelivery::Flag;
    let mut inject = ParamDecl::new("inject");
    inject.delivery = ParameterDelivery::Inject;
    let mut env = ParamDecl::new("env");
    env.delivery = ParameterDelivery::Env;

    let values = resolve_values(
        &[placeholder, flag, inject, env],
        &raw(&[
            ("placeholder", "{{cwd}}"),
            ("flag", "{{cwd}}"),
            ("inject", "{{cwd}}"),
            ("env", "{{cwd}}"),
        ]),
        &context(),
    )
    .unwrap();

    assert_eq!(values["placeholder"], "{{cwd}}");
    assert_eq!(values["flag"], "{cwd}");
    assert_eq!(values["inject"], "{cwd}");
    assert_eq!(values["env"], "{cwd}");
}

#[test]
fn missing_token_environment_values_preserve_the_exact_token_error() {
    let field = ParamDecl::new("output");

    let error = resolve_values(
        &[field],
        &raw(&[("output", "{env:MISSING}/out")]),
        &context(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        ValueResolutionError::Token(TokenError::MissingEnvironment {
            name: "MISSING".to_owned(),
            token: "{env:MISSING}".to_owned(),
        })
    );
}

#[test]
fn missing_fields_resolve_to_empty_and_unknown_submitted_keys_are_ignored() {
    let first = ParamDecl::new("first");
    let second = ParamDecl::new("second");

    let values = resolve_values(
        &[first, second],
        &raw(&[("first", "value"), ("removed", "must-not-leak")]),
        &context(),
    )
    .unwrap();

    assert_eq!(
        values,
        BTreeMap::from([
            ("first".to_owned(), "value".to_owned()),
            ("second".to_owned(), String::new()),
        ])
    );
}
