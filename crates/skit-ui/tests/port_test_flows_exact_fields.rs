//! Exact field-projection ports from Python v0.4 `tests/test_flows.py`.
//!
//! Rust keeps launch-only flag/action/repeat semantics on `ParamDecl` while `RunField` owns the
//! frontend control.  Exact tests therefore cross both public projections instead of manufacturing
//! a Python-shaped FormField in test code.

use std::collections::BTreeMap;

use skit_application::{
    glob_expansion::GlobExpander,
    run_inputs::assemble_run_inputs,
    tokens::TokenContext,
};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_ui::{ChoicePresentation, FormControl, FormInputKind, RunFormView};

#[derive(Debug)]
struct NoGlob;
impl GlobExpander for NoGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> { vec![piece.to_owned()] }
}
fn context() -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: None,
        env: BTreeMap::new(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}
fn view(declaration: &ParamDecl) -> RunFormView {
    RunFormView::from_declarations(
        "demo",
        "Demo",
        std::slice::from_ref(declaration),
        &BTreeMap::new(),
        &[],
        "",
        &BTreeMap::new(),
        "",
    )
}

#[test]
fn test_field_from_arg_maps_every_field() {
    let mut declaration = ParamDecl::new("mode");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--mode".to_owned();
    declaration.required = true;
    declaration.parameter_type = ParameterType::Choice;
    declaration.choices = vec!["a".to_owned(), "b".to_owned()];
    declaration.default = Some(ParameterValue::String("a".to_owned()));
    declaration.help = "pick one".to_owned();
    declaration.multiple = true;
    declaration.secret = true;

    let form = view(&declaration);
    let field = &form.fields()[0];
    assert_eq!(field.key, "value:mode");
    assert_eq!(field.label, "mode");
    assert_eq!(field.delivery, ParameterDelivery::Flag);
    assert_eq!(field.parameter_type, ParameterType::Choice);
    let FormControl::Choice(choice) = &field.control else {
        panic!("choice parameter stopped rendering as a choice control: {field:?}");
    };
    assert_eq!(choice.options, ["a", "b"]);
    assert_eq!(choice.presentation, ChoicePresentation::Radio);
    assert_eq!(field.default.as_deref(), Some("a"));
    assert_eq!(field.help, "pick one");
    assert!(field.required);
    assert!(field.secret(), "the frozen field projection marks the choice secret");
    assert!(field.multiple);
    assert!(!field.degraded);
    assert_eq!(declaration.flag, "--mode");
    assert_eq!(declaration.action, "");
}

#[test]
fn test_field_from_arg_degraded_renders_as_text() {
    let mut declaration = ParamDecl::new("bg");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--bg".to_owned();
    declaration.parameter_type = ParameterType::Int;
    declaration.degraded = true;
    let form = view(&declaration);
    let field = &form.fields()[0];
    let FormControl::Text(text) = &field.control else {
        panic!("a degraded field must be free text, not a typed control: {field:?}");
    };
    assert_eq!(text.kind, FormInputKind::Text);
    assert_eq!(text.value, "");
    assert_eq!(field.default, None);
    assert!(field.degraded);
}

#[test]
fn test_field_from_arg_copies_repeat() {
    let mut declaration = ParamDecl::new("tag");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--tag".to_owned();
    declaration.multiple = true;
    declaration.repeat = true;
    let form = view(&declaration);
    assert!(form.fields()[0].multiple);
    assert!(declaration.repeat, "repeat must remain attached to the launch declaration");
    let assembly = assemble_run_inputs(
        &[declaration],
        &BTreeMap::from([("tag".to_owned(), "a b".to_owned())]),
        &[],
        true,
        &context(),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--tag", "a", "--tag", "b"]);
}

#[test]
fn test_field_from_arg_bool_flag_empty_action_defaults_store_true() {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--v".to_owned();
    declaration.parameter_type = ParameterType::Bool;
    declaration.action.clear();
    let form = view(&declaration);
    assert!(matches!(form.fields()[0].control, FormControl::Checkbox { .. }));
    let assembly = assemble_run_inputs(
        &[declaration],
        &BTreeMap::from([("v".to_owned(), "true".to_owned())]),
        &[],
        true,
        &context(),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(
        assembly.args,
        ["--v"],
        "a bool flag with an empty stored action must acquire the frozen store_true behavior"
    );
}

#[test]
fn test_field_from_arg_bool_flag_degraded_stays_text_and_keeps_empty_action() {
    let mut declaration = ParamDecl::new("v");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--v".to_owned();
    declaration.parameter_type = ParameterType::Bool;
    declaration.degraded = true;
    declaration.action.clear();
    let form = view(&declaration);
    let FormControl::Text(text) = &form.fields()[0].control else {
        panic!("a degraded bool must remain free text: {:?}", form.fields()[0]);
    };
    assert_eq!(text.kind, FormInputKind::Text);
    assert_eq!(declaration.action, "");
}

#[test]
fn test_field_from_arg_bool_positional_no_flag_keeps_empty_action() {
    let mut declaration = ParamDecl::new("b");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.parameter_type = ParameterType::Bool;
    declaration.flag.clear();
    declaration.action.clear();
    assert_eq!(declaration.action, "");
    let assembly = assemble_run_inputs(
        &[declaration],
        &BTreeMap::from([("b".to_owned(), "true".to_owned())]),
        &[],
        true,
        &context(),
        &NoGlob,
    )
    .unwrap();
    assert!(assembly.args.is_empty());
}

#[test]
fn test_field_from_arg_bool_flag_explicit_action_preserved() {
    let mut declaration = ParamDecl::new("c");
    declaration.delivery = ParameterDelivery::Flag;
    declaration.flag = "--c".to_owned();
    declaration.parameter_type = ParameterType::Bool;
    declaration.action = "store_false".to_owned();
    let form = view(&declaration);
    assert!(matches!(form.fields()[0].control, FormControl::Checkbox { .. }));
    assert_eq!(declaration.action, "store_false");
    let assembly = assemble_run_inputs(
        &[declaration],
        &BTreeMap::from([("c".to_owned(), "false".to_owned())]),
        &[],
        true,
        &context(),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--c"]);
}

#[test]
fn test_render_default_spells_booleans_lowercase() {
    for (value, expected) in [
        (ParameterValue::Bool(true), "true"),
        (ParameterValue::Bool(false), "false"),
        (ParameterValue::Integer(8), "8"),
        (ParameterValue::String("x".to_owned()), "x"),
    ] {
        let mut declaration = ParamDecl::new("x");
        declaration.default = Some(value);
        let form = view(&declaration);
        assert_eq!(form.fields()[0].default.as_deref(), Some(expected));
    }
}

#[test]
fn test_field_from_spec_maps_every_field() {
    let mut declaration = ParamDecl::new("API");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Int;
    declaration.default = Some(ParameterValue::Integer(7));
    declaration.prompt = "How many?".to_owned();
    declaration.secret = true;
    declaration.env_source = "API_N".to_owned();
    let form = view(&declaration);
    let field = &form.fields()[0];
    assert_eq!(field.key, "value:API");
    assert_eq!(field.label, "How many?");
    assert_eq!(field.parameter_type, ParameterType::Int);
    assert_eq!(field.delivery, ParameterDelivery::Inject);
    assert_eq!(field.default.as_deref(), Some("7"));
    assert!(field.secret());
    assert_eq!(field.environment_source(), Some("API_N"));
}

#[test]
fn test_field_from_spec_unknown_type_falls_back_to_text() {
    let mut declaration = ParamDecl::new("X");
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Choice;
    let form = view(&declaration);
    let FormControl::Text(text) = &form.fields()[0].control else {
        panic!("choice-without-choices must fall back to text");
    };
    assert_eq!(text.kind, FormInputKind::Text);
    assert_eq!(text.value, "");
    assert_eq!(form.fields()[0].default, None);
}

#[test]
fn test_field_from_spec_maps_numeric_and_bool_kinds() {
    for (name, parameter_type, expected) in [
        ("R", ParameterType::Float, "float"),
        ("B", ParameterType::Bool, "bool"),
        ("I", ParameterType::Int, "int"),
    ] {
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = parameter_type;
        let form = view(&declaration);
        match (&form.fields()[0].control, expected) {
            (FormControl::Text(text), "float") => assert_eq!(text.kind, FormInputKind::Float),
            (FormControl::Text(text), "int") => assert_eq!(text.kind, FormInputKind::Integer),
            (FormControl::Checkbox { .. }, "bool") => {}
            (control, _) => panic!("wrong control for {name}/{expected}: {control:?}"),
        }
    }
}
