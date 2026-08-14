//! Exact core application-pipeline ports from Python v0.4 `tests/test_flows.py`.

use std::collections::BTreeMap;

use skit_application::{
    form_state::prefill,
    glob_expansion::GlobExpander,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
    value_preparation::{ValuePreparationError, validate_form_value},
    value_resolution::resolve_values,
};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

#[derive(Debug)]
struct NoGlob;
impl GlobExpander for NoGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> { vec![piece.to_owned()] }
}

fn context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: Some("/home/me".to_owned()),
        env: env.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}
fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}

fn arg_declarations() -> Vec<ParamDecl> {
    let mut inputs = ParamDecl::new("inputs");
    inputs.delivery = ParameterDelivery::Flag;
    inputs.required = true;
    inputs.multiple = true;
    inputs.flag.clear();

    let mut output = ParamDecl::new("output");
    output.delivery = ParameterDelivery::Flag;
    output.required = true;
    output.flag = "--output".to_owned();

    let mut gap = ParamDecl::new("gap");
    gap.delivery = ParameterDelivery::Flag;
    gap.parameter_type = ParameterType::Int;
    gap.flag = "--gap".to_owned();
    gap.default = Some(ParameterValue::Integer(0));

    let mut mode = ParamDecl::new("mode");
    mode.delivery = ParameterDelivery::Flag;
    mode.parameter_type = ParameterType::Choice;
    mode.flag = "--mode".to_owned();
    mode.choices = vec!["a".to_owned(), "b".to_owned()];
    mode.default = Some(ParameterValue::String("a".to_owned()));

    let mut fast = ParamDecl::new("fast");
    fast.delivery = ParameterDelivery::Flag;
    fast.parameter_type = ParameterType::Bool;
    fast.flag = "--fast".to_owned();
    fast.action = "store_true".to_owned();
    fast.default = Some(ParameterValue::Bool(false));

    let mut bg = ParamDecl::new("bg");
    bg.delivery = ParameterDelivery::Flag;
    bg.flag = "--bg".to_owned();
    bg.degraded = true;
    vec![inputs, output, gap, mode, fast, bg]
}
fn values_ok() -> BTreeMap<String, String> {
    values(&[("inputs", "a.png"), ("output", "o.png"), ("gap", "0"), ("mode", "a"), ("fast", "false")])
}
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

    let mut secret = ParamDecl::new("API_KEY");
    secret.binding = ParameterBinding::Const;
    secret.delivery = ParameterDelivery::Inject;
    secret.default = Some(ParameterValue::String("xxx".to_owned()));
    secret.secret = true;
    secret.env_source = "MY_API_KEY".to_owned();
    vec![output, width, secret]
}

#[test]
fn test_prefill_default_then_last_then_preset() {
    let declarations = managed_declarations();
    assert_eq!(prefill(&declarations, &BTreeMap::new(), None).get("OUTPUT").map(String::as_str), Some("out.jpg"));
    let last = values(&[("OUTPUT", "last.jpg")]);
    assert_eq!(prefill(&declarations, &last, None).get("OUTPUT").map(String::as_str), Some("last.jpg"));
    let preset = values(&[("OUTPUT", "web.jpg")]);
    assert_eq!(prefill(&declarations, &last, Some(&preset)).get("OUTPUT").map(String::as_str), Some("web.jpg"));
    assert_eq!(prefill(&declarations, &last, None).get("OUTPUT").map(String::as_str), Some("last.jpg"));
}

#[test]
fn test_prefill_never_offers_secrets() {
    assert!(!prefill(&managed_declarations(), &BTreeMap::new(), None).contains_key("API_KEY"));
}

#[test]
fn test_validate_required_empty() {
    let errors = arg_declarations().into_iter().filter_map(|decl| {
        validate_form_value(&decl, "").err().map(|error| (decl.name, error))
    }).collect::<Vec<_>>();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors.iter().map(|(name, _)| name.as_str()).collect::<std::collections::BTreeSet<_>>(), ["inputs", "output"].into_iter().collect());
    assert!(errors.iter().all(|(_, error)| matches!(error, ValuePreparationError::Required { .. })));
}

#[test]
fn test_validate_int_error_names_field_and_value() {
    let gap = arg_declarations().into_iter().find(|d| d.name == "gap").unwrap();
    assert_eq!(validate_form_value(&gap, "abc").unwrap_err(), ValuePreparationError::InvalidType {
        name: "gap".to_owned(), value: "abc".to_owned(), parameter_type: ParameterType::Int,
    });
}

#[test]
fn test_validate_choice() {
    let mode = arg_declarations().into_iter().find(|d| d.name == "mode").unwrap();
    assert_eq!(validate_form_value(&mode, "zzz").unwrap_err(), ValuePreparationError::InvalidChoice {
        name: "mode".to_owned(), value: "zzz".to_owned(), choices: vec!["a".to_owned(), "b".to_owned()],
    });
}

#[test]
fn test_validate_token_values_deferred() {
    let gap = arg_declarations().into_iter().find(|d| d.name == "gap").unwrap();
    assert!(validate_form_value(&gap, "{env:GAP}").is_ok());
}

#[test]
fn test_assemble_argparse_positionals_then_flags() {
    let asm = assemble_run_inputs(&arg_declarations(), &values(&[
        ("inputs", "a.png b.png"), ("output", "o.png"), ("gap", "4"), ("mode", "b"), ("fast", "true"),
    ]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(asm.args, ["a.png", "b.png", "--output", "o.png", "--gap", "4", "--mode", "b", "--fast"]);
}

#[test]
fn test_assemble_unchecked_store_true_omits_flag() {
    let asm = assemble_run_inputs(&arg_declarations(), &values_ok(), &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(!asm.args.iter().any(|arg| arg == "--fast"));
}

#[test]
fn test_assemble_degraded_empty_omitted_filled_passed() {
    let empty = assemble_run_inputs(&arg_declarations(), &values_ok(), &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(!empty.args.iter().any(|arg| arg == "--bg"));
    let mut raw = values_ok();
    raw.insert("bg".to_owned(), "#fff".to_owned());
    let filled = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(&filled.args[filled.args.len() - 2..], ["--bg", "#fff"]);
}

#[test]
fn test_assemble_tokens_expand_and_type_check_after_expansion() {
    let mut raw = values_ok();
    raw.insert("output".to_owned(), "out_{today}.png".to_owned());
    raw.insert("gap".to_owned(), "{env:GAP}".to_owned());
    let asm = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[("GAP", "8")]), &NoGlob).unwrap();
    assert!(asm.args.iter().any(|arg| arg == "out_2026-07-09.png"));
    assert!(asm.args.iter().any(|arg| arg == "8"));
    let error = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[("GAP", "not-a-number")]), &NoGlob).unwrap_err();
    assert!(matches!(error, RunInputError::Preparation(ValuePreparationError::InvalidType { name, value, parameter_type: ParameterType::Int }) if name == "gap" && value == "not-a-number"));
}

#[test]
fn test_assemble_missing_env_token_is_named_error() {
    let mut raw = values_ok();
    raw.insert("output".to_owned(), "{env:NOPE}".to_owned());
    let error = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[]), &NoGlob).unwrap_err();
    assert!(error.to_string().contains("NOPE"), "{error}");
}

#[test]
fn test_assemble_inject_values_expanded_and_masked_display() {
    let asm = assemble_run_inputs(&managed_declarations(), &values(&[
        ("OUTPUT", "long_{today}.jpg"), ("WIDTH", "800"), ("API_KEY", "typed-secret"),
    ]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(asm.inject_values.get("OUTPUT").map(String::as_str), Some("long_2026-07-09.jpg"));
    assert!(asm.display.contains(&("API_KEY".to_owned(), "•••".to_owned())));
    assert!(asm.display.iter().all(|(_, value)| value != "typed-secret"));
}

#[test]
fn test_assemble_secret_env_source_reads_environment() {
    let asm = assemble_run_inputs(&managed_declarations(), &values(&[("OUTPUT", "o.jpg"), ("WIDTH", "1"), ("API_KEY", "")]), &[], true, &context(&[("MY_API_KEY", "from-env")]), &NoGlob).unwrap();
    assert_eq!(asm.inject_values.get("API_KEY").map(String::as_str), Some("from-env"));
}

#[test]
fn test_assemble_secret_env_source_missing_is_named_error() {
    let error = assemble_run_inputs(&managed_declarations(), &values(&[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "")]), &[], true, &context(&[]), &NoGlob).unwrap_err();
    assert_eq!(error.to_string(), "API_KEY reads from the environment variable MY_API_KEY, but it isn't set.");
}

#[test]
fn test_assemble_typed_secret_beats_env_source() {
    let asm = assemble_run_inputs(&managed_declarations(), &values(&[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "typed")]), &[], true, &context(&[("MY_API_KEY", "env")]), &NoGlob).unwrap();
    assert_eq!(asm.inject_values.get("API_KEY").map(String::as_str), Some("typed"));
}

#[test]
fn test_assemble_command_values_and_extra_args() {
    let mut msg = ParamDecl::new("msg");
    msg.delivery = ParameterDelivery::Placeholder;
    msg.required = true;
    let asm = assemble_run_inputs(&[msg], &values(&[("msg", "hi {today}")]), &["--verbose".to_owned()], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(asm.command_values, values(&[("msg", "hi 2026-07-09")]));
    assert_eq!(asm.args, ["--verbose"]);
    assert_eq!(asm.masked_args, ["--verbose"]);
}

#[test]
fn test_assemble_extra_arg_token_error_forwards_the_token_message() {
    let error = assemble_run_inputs(&[], &BTreeMap::new(), &["{env:NOPE_EXTRA}".to_owned()], true, &context(&[]), &NoGlob).unwrap_err();
    assert!(error.to_string().contains("NOPE_EXTRA"), "{error}");
}

#[test]
fn test_assemble_inject_source_forwards_extra_args() {
    let asm = assemble_run_inputs(&managed_declarations(), &values(&[("OUTPUT", "o"), ("WIDTH", "1"), ("API_KEY", "k")]), &["--flag".to_owned(), "v".to_owned()], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(asm.args, ["--flag", "v"]);
}

#[test]
fn test_assemble_field_expands_cwd_and_now_tokens() {
    let mut raw = values_ok();
    raw.insert("output".to_owned(), "{cwd}/{now}.png".to_owned());
    let asm = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(asm.args.iter().any(|arg| arg == "/work/14-30-05.png"));
}

#[test]
fn test_assemble_flags_tolerates_missing_keys() {
    let asm = assemble_run_inputs(&arg_declarations(), &values(&[("inputs", "a"), ("output", "o")]), &[], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(asm.args, ["a", "--output", "o"]);
}

#[test]
fn test_assemble_empty_field_does_not_stop_later_flags() {
    let mut raw = values_ok();
    raw.insert("gap".to_owned(), String::new());
    raw.insert("mode".to_owned(), "b".to_owned());
    let asm = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[]), &NoGlob).unwrap();
    assert!(asm.args.iter().any(|arg| arg == "--mode"));
    assert!(asm.args.iter().any(|arg| arg == "b"));
}

#[test]
fn test_split_multi_falls_back_on_unbalanced_quote() {
    let mut raw = values_ok();
    raw.insert("inputs".to_owned(), "a\"b".to_owned());
    let asm = assemble_run_inputs(&arg_declarations(), &raw, &[], true, &context(&[]), &NoGlob).unwrap();
    assert_eq!(asm.args.first().map(String::as_str), Some("a\"b"));
}

#[test]
fn test_resolve_secret_empty_when_no_input_and_no_env_source() {
    let mut secret = ParamDecl::new("k");
    secret.secret = true;
    let resolved = resolve_values(&[secret], &values(&[("k", "")]), &context(&[])).unwrap();
    assert_eq!(resolved.get("k").map(String::as_str), Some(""));
}

#[test]
fn test_validate_value_accepts_a_valid_choice() {
    let mut mode = ParamDecl::new("m");
    mode.parameter_type = ParameterType::Choice;
    mode.choices = vec!["a".to_owned(), "b".to_owned()];
    assert!(validate_form_value(&mode, "a").is_ok());
    assert!(validate_form_value(&mode, "b").is_ok());
}

#[test]
fn test_prefill_drops_a_secret_that_leaked_into_saved_values() {
    let actual = prefill(&managed_declarations(), &values(&[("OUTPUT", "o.jpg"), ("API_KEY", "leaked")]), None);
    assert_eq!(actual.get("OUTPUT").map(String::as_str), Some("o.jpg"));
    assert!(!actual.contains_key("API_KEY"));
}

#[test]
fn test_prefill_preset_drops_leaked_secret() {
    let preset = values(&[("OUTPUT", "web.jpg"), ("API_KEY", "leaked")]);
    let actual = prefill(&managed_declarations(), &BTreeMap::new(), Some(&preset));
    assert_eq!(actual.get("OUTPUT").map(String::as_str), Some("web.jpg"));
    assert!(!actual.contains_key("API_KEY"));
}

#[test]
fn test_prefill_unknown_preset_is_no_op_not_a_crash() {
    let empty = BTreeMap::new();
    let actual = prefill(&managed_declarations(), &BTreeMap::new(), Some(&empty));
    assert_eq!(actual.get("OUTPUT").map(String::as_str), Some("out.jpg"));
}
