use std::{collections::BTreeMap, fmt};

use skit_application::{
    delivery::{Assembly, AssemblyError, transparency_messages},
    glob_expansion::GlobExpander,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::{TokenContext, TokenError},
    value_preparation::ValuePreparationError,
    value_resolution::ValueResolutionError,
};
use skit_domain::parameters::{ParamDecl, ParameterDelivery, ParameterType};
use skit_i18n::Locale;

#[derive(Default)]
struct FakeGlob {
    matches: BTreeMap<String, Vec<String>>,
}

impl fmt::Debug for FakeGlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("FakeGlob").finish_non_exhaustive()
    }
}

impl GlobExpander for FakeGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        self.matches
            .get(piece)
            .cloned()
            .unwrap_or_else(|| vec![piece.to_owned()])
    }
}

fn context() -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: Some("/home/user".to_owned()),
        env: BTreeMap::from([
            ("TOKEN".to_owned(), "secret-from-env".to_owned()),
            ("PORT".to_owned(), "8080".to_owned()),
            ("EXTRA".to_owned(), "*.log".to_owned()),
        ]),
        today: "2026-08-08".to_owned(),
        now: "03-00-00".to_owned(),
    }
}

fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn one_pipeline_resolves_validates_splits_globs_masks_and_routes() {
    let mut input = ParamDecl::new("input");
    input.required = true;

    let mut files = ParamDecl::new("files");
    files.flag = "--file".to_owned();
    files.multiple = true;
    files.repeat = true;
    files.parameter_type = ParameterType::Path;

    let mut port = ParamDecl::new("port");
    port.flag = "--port".to_owned();
    port.parameter_type = ParameterType::Int;

    let mut secret = ParamDecl::new("token");
    secret.delivery = ParameterDelivery::Env;
    secret.env_target = "APP_TOKEN".to_owned();
    secret.env_source = "TOKEN".to_owned();
    secret.secret = true;

    let mut placeholder = ParamDecl::new("subject");
    placeholder.delivery = ParameterDelivery::Placeholder;

    let mut glob = FakeGlob::default();
    glob.matches.insert(
        "*.txt".to_owned(),
        vec!["a.txt".to_owned(), "b.txt".to_owned()],
    );
    glob.matches.insert(
        "*.log".to_owned(),
        vec!["one.log".to_owned(), "two.log".to_owned()],
    );

    let assembly = assemble_run_inputs(
        &[input, files, port, secret, placeholder],
        &map(&[
            ("input", "source.txt"),
            ("files", "'*.txt' literal.md"),
            ("port", "{env:PORT}"),
            ("token", ""),
            ("subject", "release {{notes}} {today}"),
            ("removed", "must-not-leak"),
        ]),
        &["{env:EXTRA}".to_owned(), "two words".to_owned()],
        true,
        &context(),
        &glob,
    )
    .unwrap();

    assert_eq!(
        assembly.args,
        [
            "source.txt",
            "--file",
            "a.txt",
            "--file",
            "b.txt",
            "--file",
            "literal.md",
            "--port",
            "8080",
            "one.log",
            "two.log",
            "two words",
        ]
    );
    assert_eq!(assembly.masked_args, assembly.args);
    assert_eq!(assembly.env_values["APP_TOKEN"], "secret-from-env");
    assert_eq!(assembly.masked_env["APP_TOKEN"], "•••");
    assert_eq!(
        assembly.command_values["subject"],
        "release {{notes}} 2026-08-08"
    );
}

#[test]
fn literal_extra_tail_is_not_reinterpreted_by_the_pipeline() {
    let field = ParamDecl::new("input");
    let glob = FakeGlob::default();

    let assembly = assemble_run_inputs(
        &[field],
        &map(&[("input", "file.txt")]),
        &["{today}".to_owned(), "*.log".to_owned()],
        false,
        &context(),
        &glob,
    )
    .unwrap();

    assert_eq!(assembly.args, ["file.txt", "{today}", "*.log"]);
}

#[test]
fn field_resolution_errors_remain_typed() {
    let field = ParamDecl::new("output");
    let error = assemble_run_inputs(
        &[field],
        &map(&[("output", "{env:MISSING}")]),
        &[],
        true,
        &context(),
        &FakeGlob::default(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        RunInputError::Resolution(ValueResolutionError::Token(
            TokenError::MissingEnvironment {
                name: "MISSING".to_owned(),
                token: "{env:MISSING}".to_owned(),
            }
        ))
    );
}

#[test]
fn validation_errors_remain_typed() {
    let mut field = ParamDecl::new("port");
    field.parameter_type = ParameterType::Int;
    let error = assemble_run_inputs(
        &[field],
        &map(&[("port", "wrong")]),
        &[],
        true,
        &context(),
        &FakeGlob::default(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        RunInputError::Preparation(ValuePreparationError::InvalidType {
            name: "port".to_owned(),
            value: "wrong".to_owned(),
            parameter_type: ParameterType::Int,
        })
    );
}

#[test]
fn extra_tail_token_errors_are_distinct_from_field_resolution_errors() {
    let error = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &["{env:MISSING}".to_owned()],
        true,
        &context(),
        &FakeGlob::default(),
    )
    .unwrap_err();

    assert_eq!(
        error,
        RunInputError::ExtraToken(TokenError::MissingEnvironment {
            name: "MISSING".to_owned(),
            token: "{env:MISSING}".to_owned(),
        })
    );
}

#[test]
fn impossible_prepared_shape_errors_keep_the_delivery_error_type() {
    assert_eq!(
        RunInputError::Assembly(AssemblyError::UnexpectedMultiple {
            name: "name".to_owned(),
        })
        .to_string(),
        "parameter \"name\" received multiple values but is not a multi-value flag"
    );
}

#[test]
fn launch_transparency_is_masked_and_frontend_neutral() {
    let assembly = Assembly {
        display: vec![
            ("name".to_owned(), "Ada".to_owned()),
            ("token".to_owned(), "•••".to_owned()),
        ],
        ..Assembly::default()
    };
    let lines = transparency_messages(&assembly, "python /tmp/.run-script.py")
        .into_iter()
        .map(|message| message.localize(Locale::En))
        .collect::<Vec<_>>();

    assert_eq!(
        lines,
        [
            "→ inject: name = Ada, token = •••",
            "  (written to a temporary copy, deleted after the run; your original file is untouched)",
            "→ python /tmp/.run-script.py",
        ]
    );
}
