//! Public-API ports of the Python v0.4 unified form pipeline (`tests/test_flows.py`).
//!
//! These cases stay filesystem-free: ambient values and glob behavior are injected explicitly so
//! the same resolve -> validate -> prepare -> delivery contract is exercised without a frontend.

use std::collections::BTreeMap;

use skit_application::{
    form_state::prefill,
    glob_expansion::GlobExpander,
    run_inputs::{RunInputError, assemble_run_inputs},
    tokens::TokenContext,
    value_preparation::{ValuePreparationError, validate_form_value},
    value_resolution::{ValueResolutionError, resolve_values},
};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};

#[derive(Debug)]
struct NoGlob;

impl GlobExpander for NoGlob {
    fn expand_piece(&self, piece: &str) -> Vec<String> {
        vec![piece.to_owned()]
    }
}

fn context(env: &[(&str, &str)]) -> TokenContext {
    TokenContext {
        cwd: "/work".to_owned(),
        home: Some("/home/me".to_owned()),
        env: env
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        today: "2026-07-09".to_owned(),
        now: "14-30-05".to_owned(),
    }
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn field(name: &str, delivery: ParameterDelivery, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = delivery;
    declaration.parameter_type = parameter_type;
    declaration
}

#[test]
fn test_prefill_definition_default_then_last_used_then_selected_preset() {
    let mut output = field("OUTPUT", ParameterDelivery::Inject, ParameterType::Str);
    output.default = Some(ParameterValue::String("out.jpg".to_owned()));
    let declarations = [output];

    assert_eq!(
        prefill(&declarations, &BTreeMap::new(), None),
        values(&[("OUTPUT", "out.jpg")])
    );
    let last = values(&[("OUTPUT", "last.jpg")]);
    assert_eq!(prefill(&declarations, &last, None), last);
    let preset = values(&[("OUTPUT", "web.jpg")]);
    assert_eq!(prefill(&declarations, &last, Some(&preset)), preset);
}

#[test]
fn test_prefill_never_offers_secrets_or_stale_unknown_keys() {
    let mut public = field("PUBLIC", ParameterDelivery::Inject, ParameterType::Str);
    public.default = Some(ParameterValue::String("definition".to_owned()));
    let mut secret = field("API_KEY", ParameterDelivery::Inject, ParameterType::Str);
    secret.secret = true;
    secret.default = Some(ParameterValue::String("must-not-surface".to_owned()));
    let declarations = [public, secret];
    let last = values(&[
        ("PUBLIC", "last"),
        ("API_KEY", "old-plaintext"),
        ("REMOVED", "stale"),
    ]);
    let preset = values(&[
        ("PUBLIC", "preset"),
        ("API_KEY", "preset-plaintext"),
        ("REMOVED", "preset-stale"),
    ]);

    assert_eq!(
        prefill(&declarations, &last, Some(&preset)),
        values(&[("PUBLIC", "preset")])
    );
}

#[test]
fn test_token_bearing_int_defers_validation_until_after_expansion() {
    let mut gap = field("gap", ParameterDelivery::Flag, ParameterType::Int);
    gap.flag = "--gap".to_owned();
    let declarations = [gap.clone()];
    let raw = values(&[("gap", "{env:GAP}")]);

    assert!(validate_form_value(&gap, "{env:GAP}").is_ok());
    let assembly = assemble_run_inputs(
        &declarations,
        &raw,
        &[],
        true,
        &context(&[("GAP", "8")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--gap", "8"]);

    let error = assemble_run_inputs(
        &declarations,
        &raw,
        &[],
        true,
        &context(&[("GAP", "not-a-number")]),
        &NoGlob,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RunInputError::Preparation(ValuePreparationError::InvalidType { name, value, .. })
            if name == "gap" && value == "not-a-number"
    ));
}

#[test]
fn test_secret_env_source_reads_the_explicit_launch_environment() {
    let mut secret = field("API_KEY", ParameterDelivery::Inject, ParameterType::Str);
    secret.secret = true;
    secret.env_source = "MY_API_KEY".to_owned();
    let declarations = [secret];

    let assembly = assemble_run_inputs(
        &declarations,
        &values(&[("API_KEY", "")]),
        &[],
        true,
        &context(&[("MY_API_KEY", "from-env")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([("API_KEY".to_owned(), "from-env".to_owned())])
    );
    assert_eq!(assembly.display, [("API_KEY".to_owned(), "•••".to_owned())]);
}

#[test]
fn test_typed_secret_beats_env_source_and_is_never_token_expanded() {
    let mut secret = field("API_KEY", ParameterDelivery::Inject, ParameterType::Str);
    secret.secret = true;
    secret.env_source = "MY_API_KEY".to_owned();
    let declarations = [secret];
    let typed = "literal-{today}-{env:OTHER}";

    let assembly = assemble_run_inputs(
        &declarations,
        &values(&[("API_KEY", typed)]),
        &[],
        true,
        &context(&[("MY_API_KEY", "env"), ("OTHER", "expanded")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(
        assembly.inject_values.get("API_KEY").map(String::as_str),
        Some(typed)
    );
    assert!(
        assembly
            .display
            .iter()
            .all(|(_, rendered)| rendered != typed && rendered == "•••")
    );
}

#[test]
fn test_missing_secret_env_source_is_a_named_resolution_error() {
    let mut secret = field("API_KEY", ParameterDelivery::Inject, ParameterType::Str);
    secret.secret = true;
    secret.env_source = "MY_API_KEY".to_owned();

    let error = resolve_values(&[secret], &values(&[("API_KEY", "")]), &context(&[])).unwrap_err();
    assert!(matches!(
        error,
        ValueResolutionError::MissingSecretEnvironment { name, environment }
            if name == "API_KEY" && environment == "MY_API_KEY"
    ));
}

#[test]
fn test_positionals_emit_before_flags_and_store_true_only_fires_when_checked() {
    let mut inputs = field("inputs", ParameterDelivery::Flag, ParameterType::Path);
    inputs.multiple = true;
    inputs.required = true;
    inputs.flag.clear();

    let mut output = field("output", ParameterDelivery::Flag, ParameterType::Path);
    output.flag = "--output".to_owned();
    output.required = true;

    let mut fast = field("fast", ParameterDelivery::Flag, ParameterType::Bool);
    fast.flag = "--fast".to_owned();
    fast.action = "store_true".to_owned();
    fast.default = Some(ParameterValue::Bool(false));

    let declarations = [inputs, output, fast];
    let assembly = assemble_run_inputs(
        &declarations,
        &values(&[
            ("inputs", "a.png b.png"),
            ("output", "o.png"),
            ("fast", "true"),
        ]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(
        assembly.args,
        ["a.png", "b.png", "--output", "o.png", "--fast"]
    );

    let unchecked = assemble_run_inputs(
        &declarations,
        &values(&[("inputs", "a.png"), ("output", "o.png"), ("fast", "false")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(!unchecked.args.iter().any(|arg| arg == "--fast"));
}

#[test]
fn test_multi_flag_repeat_false_emits_one_flag_then_many_values() {
    let mut inputs = field("inputs", ParameterDelivery::Flag, ParameterType::Path);
    inputs.flag = "--input".to_owned();
    inputs.multiple = true;
    inputs.repeat = false;

    let assembly = assemble_run_inputs(
        &[inputs],
        &values(&[("inputs", "a.png b.png")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--input", "a.png", "b.png"]);
}

#[test]
fn test_multi_flag_repeat_true_repeats_the_option_for_each_value() {
    let mut include = field("include", ParameterDelivery::Flag, ParameterType::Path);
    include.flag = "--include".to_owned();
    include.multiple = true;
    include.repeat = true;

    let assembly = assemble_run_inputs(
        &[include],
        &values(&[("include", "a b")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(assembly.args, ["--include", "a", "--include", "b"]);
}

#[test]
fn test_degraded_flag_omits_empty_but_forwards_filled_value() {
    let mut bg = field("bg", ParameterDelivery::Flag, ParameterType::Str);
    bg.flag = "--bg".to_owned();
    bg.degraded = true;

    let empty = assemble_run_inputs(
        &[bg.clone()],
        &values(&[("bg", "")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert!(empty.args.is_empty());

    let filled = assemble_run_inputs(
        &[bg],
        &values(&[("bg", "#fff")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(filled.args, ["--bg", "#fff"]);
}

#[test]
fn test_extra_arg_tokens_expand_only_when_the_tail_is_marked_raw() {
    let raw = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &[
            "{today}".to_owned(),
            "{cwd}".to_owned(),
            "{env:XV}".to_owned(),
        ],
        true,
        &context(&[("XV", "envval")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(raw.args, ["2026-07-09", "/work", "envval"]);

    let literal = assemble_run_inputs(
        &[],
        &BTreeMap::new(),
        &["{today}".to_owned(), "{env:XV}".to_owned()],
        false,
        &context(&[("XV", "envval")]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(literal.args, ["{today}", "{env:XV}"]);
}

#[test]
fn test_stale_submitted_keys_not_in_the_current_schema_are_ignored() {
    let declaration = field("CURRENT", ParameterDelivery::Inject, ParameterType::Str);
    let assembly = assemble_run_inputs(
        &[declaration],
        &values(&[("CURRENT", "yes"), ("REMOVED", "must-not-leak")]),
        &[],
        true,
        &context(&[]),
        &NoGlob,
    )
    .unwrap();
    assert_eq!(
        assembly.inject_values,
        BTreeMap::from([("CURRENT".to_owned(), "yes".to_owned())])
    );
    assert!(!format!("{assembly:?}").contains("must-not-leak"));
}

#[test]
fn test_input_binding_requiredness_uses_the_prompt_label() {
    let mut declaration = field("input-1", ParameterDelivery::Inject, ParameterType::Str);
    declaration.binding = ParameterBinding::Input;
    declaration.required = true;
    declaration.prompt = "Your name? ".to_owned();

    let error = validate_form_value(&declaration, "").unwrap_err();
    assert!(matches!(
        error,
        ValuePreparationError::Required { name, label }
            if name == "input-1" && label == "Your name? "
    ));
}
