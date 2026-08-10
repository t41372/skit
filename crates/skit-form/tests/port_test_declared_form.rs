//! Exact-name form-planning ports from Python v0.4 `tests/test_declared_params.py`.

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_form::{FormSource, form_plan};

fn placeholder(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Placeholder;
    declaration
}

fn env(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Env;
    declaration
}

#[test]
fn test_command_plan_honors_declared_schema() {
    let mut size = placeholder("size");
    size.parameter_type = ParameterType::Choice;
    size.choices = vec!["s".to_owned(), "m".to_owned()];
    size.default = Some(ParameterValue::String("m".to_owned()));
    size.required = false;
    let settings = EntrySettings {
        params: vec!["size".to_owned(), "api_key".to_owned()],
        parameters: vec![size],
        ..EntrySettings::default()
    };

    let plan = form_plan("command", "convert {size} {api_key}", &settings);
    assert_eq!(plan.source, FormSource::Command);
    let declarations = plan.declarations();
    assert_eq!(declarations.len(), 2);
    assert_eq!(declarations[0].name, "size");
    assert_eq!(declarations[0].parameter_type, ParameterType::Choice);
    assert_eq!(declarations[0].choices, ["s", "m"]);
    assert_eq!(
        declarations[0].default,
        Some(ParameterValue::String("m".to_owned()))
    );
    assert!(!declarations[0].required);
    assert_eq!(declarations[1].name, "api_key");
    assert!(declarations[1].required);
    assert!(declarations[1].secret);
}

#[test]
fn test_exe_with_declared_params_gets_a_form() {
    let mut width = ParamDecl::new("width");
    width.delivery = ParameterDelivery::Flag;
    width.flag = "--width".to_owned();
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    let mut debug = env("DEBUG");
    debug.parameter_type = ParameterType::Bool;
    let mut meaningless = ParamDecl::new("slot");
    meaningless.delivery = ParameterDelivery::Placeholder;
    let settings = EntrySettings {
        parameters: vec![width, debug, meaningless],
        ..EntrySettings::default()
    };

    let plan = form_plan("exe", "", &settings);
    assert_eq!(plan.source, FormSource::Declared);
    assert_eq!(
        plan.declarations()
            .iter()
            .map(|declaration| (declaration.name.as_str(), declaration.delivery))
            .collect::<Vec<_>>(),
        [
            ("width", ParameterDelivery::Flag),
            ("DEBUG", ParameterDelivery::Env),
        ]
    );
}

#[test]
fn test_exe_without_declared_params_stays_none_plan() {
    let plan = form_plan("exe", "", &EntrySettings::default());
    assert_eq!(plan.source, FormSource::None);
    assert!(plan.declarations().is_empty());
}

#[test]
fn test_assemble_command_with_env_rider() {
    // Planning half of the Python contract: the command placeholder and declared env rider coexist,
    // and the rider stays an env declaration rather than becoming a template value.
    let settings = EntrySettings {
        params: vec!["msg".to_owned()],
        parameters: vec![env("RETRIES")],
        ..EntrySettings::default()
    };
    let plan = form_plan("command", "echo {msg}", &settings);
    assert_eq!(plan.source, FormSource::Command);
    assert_eq!(
        plan.declarations()
            .iter()
            .map(|declaration| (declaration.name.as_str(), declaration.delivery))
            .collect::<Vec<_>>(),
        [
            ("msg", ParameterDelivery::Placeholder),
            ("RETRIES", ParameterDelivery::Env),
        ]
    );
}
