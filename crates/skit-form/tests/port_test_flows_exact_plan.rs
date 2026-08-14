//! Exact form-plan ports from Python v0.4 `tests/test_flows.py`.

use skit_domain::{EntrySettings, parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue}};
use skit_form::{FormSource, form_plan};
use skit_language::write_managed_params;

const ARGPARSE_SCRIPT: &str = r#"import argparse
p = argparse.ArgumentParser()
p.add_argument("inputs", nargs="+", help="input files")
p.add_argument("--output", required=True, help="output path")
p.add_argument("--gap", type=int, default=0)
p.add_argument("--mode", choices=["a", "b"], default="a")
p.add_argument("--fast", action="store_true")
p.add_argument("--bg", type=str)
ns = p.parse_args()
"#;

fn managed_declarations() -> Vec<ParamDecl> {
    let mut output = ParamDecl::new("OUTPUT");
    output.binding = ParameterBinding::Const;
    output.delivery = ParameterDelivery::Inject;
    output.default = Some(ParameterValue::String("out.jpg".to_owned()));

    let mut width = ParamDecl::new("WIDTH");
    width.binding = ParameterBinding::Const;
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));

    let mut key = ParamDecl::new("API_KEY");
    key.binding = ParameterBinding::Const;
    key.delivery = ParameterDelivery::Inject;
    key.default = Some(ParameterValue::String("xxx".to_owned()));
    key.secret = true;
    key.env_source = "MY_API_KEY".to_owned();
    vec![output, width, key]
}

fn managed_source() -> String {
    write_managed_params(
        "python",
        "OUTPUT = \"out.jpg\"\nWIDTH = 800\nAPI_KEY = \"xxx\"\nprint(OUTPUT, WIDTH)\n",
        &managed_declarations(),
    )
    .unwrap()
}

#[test]
fn test_plan_managed_script_is_inject() {
    let source = managed_source();
    let plan = form_plan("python", &source, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(plan.fields.len(), 3);
    assert_eq!(
        plan.fields.iter().map(|field| field.declaration.name.as_str()).collect::<Vec<_>>(),
        ["OUTPUT", "WIDTH", "API_KEY"]
    );
    assert_eq!(plan.fields[0].declaration.parameter_type, ParameterType::Str);
    assert_eq!(plan.fields[1].declaration.parameter_type, ParameterType::Int);
    assert!(plan.fields[2].declaration.secret);
    assert_eq!(plan.fields[2].declaration.env_source, "MY_API_KEY");
}

#[test]
fn test_plan_argparse_script() {
    let plan = form_plan("python", ARGPARSE_SCRIPT, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(
        plan.fields.iter().map(|field| field.declaration.name.as_str()).collect::<Vec<_>>(),
        ["inputs", "output", "gap", "mode", "fast", "bg"]
    );
    let by = plan.fields.iter().map(|field| (field.declaration.name.as_str(), &field.declaration)).collect::<std::collections::BTreeMap<_, _>>();
    assert!(by["inputs"].required);
    assert!(by["inputs"].multiple);
    assert!(!by["inputs"].repeat);
    assert_eq!(by["output"].flag, "--output");
    assert!(by["output"].required);
    assert_eq!(by["gap"].parameter_type, ParameterType::Int);
    assert_eq!(by["mode"].parameter_type, ParameterType::Choice);
    assert_eq!(by["mode"].choices, ["a", "b"]);
    assert_eq!(by["fast"].parameter_type, ParameterType::Bool);
    assert_eq!(by["fast"].action, "store_true");
    assert_eq!(by["bg"].parameter_type, ParameterType::Str);
}

#[test]
fn test_plan_command_entry_placeholders() {
    let settings = EntrySettings {
        params: vec!["msg".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan("command", "echo {msg}", &settings);
    assert_eq!(plan.source, FormSource::Command);
    assert_eq!(plan.fields.len(), 1);
    let declaration = &plan.fields[0].declaration;
    assert_eq!(declaration.name, "msg");
    assert_eq!(declaration.delivery, ParameterDelivery::Placeholder);
    assert!(declaration.required);
}

#[test]
fn test_plan_managed_wins_over_argparse() {
    let mut output = ParamDecl::new("OUTPUT");
    output.binding = ParameterBinding::Const;
    output.delivery = ParameterDelivery::Inject;
    output.default = Some(ParameterValue::String("x".to_owned()));
    let source = write_managed_params("python", ARGPARSE_SCRIPT, &[output]).unwrap();
    let plan = form_plan("python", &source, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Inject);
    assert_eq!(plan.fields.iter().map(|field| field.declaration.name.as_str()).collect::<Vec<_>>(), ["OUTPUT"]);
}

#[test]
fn test_command_placeholders_are_required_and_secret_prechecked() {
    let settings = EntrySettings {
        params: vec!["api_key".to_owned(), "url".to_owned()],
        ..EntrySettings::default()
    };
    let plan = form_plan("command", "curl -H {api_key} {url}", &settings);
    assert_eq!(plan.source, FormSource::Command);
    let by = plan.fields.iter().map(|field| (field.declaration.name.as_str(), &field.declaration)).collect::<std::collections::BTreeMap<_, _>>();
    assert!(by["api_key"].secret);
    assert!(!by["url"].secret);
    assert!(plan.fields.iter().all(|field| field.declaration.required));
}
