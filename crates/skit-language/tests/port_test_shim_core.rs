//! Exact core ports of Python v0.4 `tests/test_shim.py`.
//!
//! The Python oracle is authoritative. Rust parser architecture may differ, but these tests keep
//! the same observable source rewrite/error contracts. Cases that Python explicitly compiled are
//! compiled by a real Python interpreter here too; a behavioral mismatch is intentionally red.

use std::{
    collections::BTreeMap,
    process::Command,
};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType,
};
use skit_language::{LanguageError, inject_values};

const SCRIPT: &str = r#""""Docstring stays."""
# /// script
# dependencies = ["requests"]
# ///
CITY = "Taipei"  # trailing comment stays
RETRIES = 3

def main():
    who = input("Your name: ")
    print(who, CITY, RETRIES)

if __name__ == "__main__":
    DEBUG = True
    main()
"#;

fn spec(
    name: &str,
    binding: ParameterBinding,
    parameter_type: ParameterType,
    order: i64,
    secret: bool,
    prompt: &str,
) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = binding;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration.order = order;
    declaration.secret = secret;
    declaration.prompt = prompt.to_owned();
    declaration
}

fn const_spec(name: &str, parameter_type: ParameterType) -> ParamDecl {
    spec(name, ParameterBinding::Const, parameter_type, -1, false, "")
}

fn input_spec(name: &str, order: i64, secret: bool, prompt: &str) -> ParamDecl {
    spec(name, ParameterBinding::Input, ParameterType::Str, order, secret, prompt)
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn inject(source: &str, declarations: &[ParamDecl], pairs: &[(&str, &str)]) -> Result<String, LanguageError> {
    inject_values("python", source, declarations, &values(pairs))
}

fn python_program() -> &'static str {
    for program in ["python3", "python"] {
        if Command::new(program)
            .args(["-c", "raise SystemExit(0)"])
            .status()
            .is_ok_and(|status| status.success())
        {
            return program;
        }
    }
    panic!("Python v0.4 shim parity tests require a real Python interpreter");
}

fn assert_python_compiles(source: &str) {
    let status = Command::new(python_program())
        .args(["-c", "import sys; compile(sys.stdin.read(), '<shim-parity>', 'exec')"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child.stdin.take().unwrap().write_all(source.as_bytes())?;
            child.wait()
        })
        .unwrap();
    assert!(status.success(), "real Python rejected injected source:\n{source}");
}

#[test]
fn test_const_str_injection_preserves_everything_else() {
    let out = inject(SCRIPT, &[const_spec("CITY", ParameterType::Str)], &[("CITY", "Kaohsiung")]).unwrap();
    assert!(out.contains("CITY = 'Kaohsiung'  # trailing comment stays"), "{out}");
    assert!(out.contains("# dependencies = [\"requests\"]"), "{out}");
    assert!(out.contains("RETRIES = 3"), "{out}");
}

#[test]
fn test_const_typed_injection() {
    let out = inject(SCRIPT, &[const_spec("RETRIES", ParameterType::Int)], &[("RETRIES", "7")]).unwrap();
    assert!(out.contains("RETRIES = 7"), "{out}");
}

#[test]
fn test_main_guard_const() {
    let out = inject(SCRIPT, &[const_spec("DEBUG", ParameterType::Bool)], &[("DEBUG", "false")]).unwrap();
    assert!(out.contains("DEBUG = False"), "{out}");
}

#[test]
fn test_input_queue_preamble_is_single_line_after_docstring() {
    let out = inject(SCRIPT, &[input_spec("input-1", 0, false, "")], &[("input-1", "Alice")]).unwrap();
    let lines = out.lines().collect::<Vec<_>>();
    assert_eq!(lines[0], "\"\"\"Docstring stays.\"\"\"");
    assert_eq!(lines.iter().filter(|line| line.ends_with("# skit:shim")).count(), 1);
    assert!(out.contains("# dependencies = [\"requests\"]"), "{out}");
}

#[test]
fn test_missing_value_leaves_script_untouched() {
    let out = inject(SCRIPT, &[const_spec("CITY", ParameterType::Str)], &[]).unwrap();
    assert_eq!(out, SCRIPT);
}

#[test]
fn test_shadowed_input_is_not_rewritten_and_surfaces_as_drift() {
    let source = "def input(prompt=''):\n    return 'HARDCODED'\ny = input('Q: ')\nprint(y)\n";
    let error = inject(source, &[input_spec("input-1", 0, false, "Q: ")], &[("input-1", "typed")]).unwrap_err();
    assert_eq!(error, LanguageError::BindingNotFound { name: "input-1".to_owned() });
}

#[test]
fn test_shadowed_input_leaves_the_call_site_text_intact_when_only_a_const_is_delivered() {
    let source = "def input(prompt=''):\n    return 'x'\nCITY = 'Taipei'\ny = input('Q: ')\nprint(y, CITY)\n";
    let out = inject(source, &[const_spec("CITY", ParameterType::Str)], &[("CITY", "Tainan")]).unwrap();
    assert!(out.contains("CITY = 'Tainan'"), "{out}");
    assert!(out.contains("y = input('Q: ')"), "{out}");
    assert!(!out.contains("_skit_i"), "{out}");
}

#[test]
fn test_drifted_target_raises() {
    let error = inject(SCRIPT, &[const_spec("GONE", ParameterType::Str)], &[("GONE", "x")]).unwrap_err();
    assert_eq!(error, LanguageError::BindingNotFound { name: "GONE".to_owned() });
}

#[test]
fn test_bad_type_coercion_raises() {
    let error = inject(SCRIPT, &[const_spec("RETRIES", ParameterType::Int)], &[("RETRIES", "not-a-number")]).unwrap_err();
    assert!(matches!(error, LanguageError::InvalidValue { name, parameter_type: ParameterType::Int, .. } if name == "RETRIES"));
}

#[test]
fn test_bad_type_coercion_raises_the_value_subclass_not_plain_shim_error() {
    let error = inject(SCRIPT, &[const_spec("RETRIES", ParameterType::Int)], &[("RETRIES", "not-a-number")]).unwrap_err();
    assert_eq!(
        error,
        LanguageError::InvalidValue {
            name: "RETRIES".to_owned(),
            value: "not-a-number".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn test_drifted_target_raises_plain_shim_error_not_value_subclass() {
    let error = inject(SCRIPT, &[const_spec("GONE", ParameterType::Str)], &[("GONE", "x")]).unwrap_err();
    assert!(matches!(error, LanguageError::BindingNotFound { name } if name == "GONE"));
}

#[test]
fn test_multiline_value_span() {
    let source = "MSG = (\n    \"hello\"\n)\nprint(MSG)\n";
    let out = inject(source, &[const_spec("MSG", ParameterType::Str)], &[("MSG", "bye")]).unwrap();
    assert!(out.contains("'bye'"), "{out}");
    assert!(!out.contains("\"hello\""), "{out}");
    assert_python_compiles(&out);
}

#[test]
fn test_coerce_bool_invalid_raises() {
    let error = inject("FLAG = True\n", &[const_spec("FLAG", ParameterType::Bool)], &[("FLAG", "maybe")]).unwrap_err();
    assert!(matches!(error, LanguageError::InvalidValue { name, parameter_type: ParameterType::Bool, .. } if name == "FLAG"));
}

#[test]
fn test_inject_syntax_error_raises() {
    let error = inject("def broken(:\n", &[const_spec("X", ParameterType::Str)], &[("X", "1")]).unwrap_err();
    assert_eq!(error, LanguageError::InvalidSource { kind: "python".to_owned() });
}

#[test]
fn test_input_order_beyond_calls_is_drift() {
    let error = inject("print('hello')\n", &[input_spec("input-1", 5, false, "")], &[("input-1", "x")]).unwrap_err();
    assert_eq!(error, LanguageError::BindingNotFound { name: "input-1".to_owned() });
}

#[test]
fn test_inject_two_identical_prompts_one_deleted_raises_cleanly_never_corrupts() {
    let source = "second = input(\"Go? \")\nprint(second)\n";
    let declarations = [
        input_spec("input-1", 0, false, "Go? "),
        input_spec("input-2", 1, false, "Go? "),
    ];
    let error = inject(source, &declarations, &[("input-1", "AAA"), ("input-2", "BBB")]).unwrap_err();
    assert!(matches!(error, LanguageError::BindingNotFound { name } if name == "input-2"));
}

#[test]
fn test_inject_duplicate_prompt_winner_only_still_injects_and_compiles() {
    let source = "second = input(\"Go? \")\nprint(second)\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "Go? ")], &[("input-1", "AAA")]).unwrap();
    assert!(!out.contains("_skit_i[0]_i[0]"), "{out}");
    assert_python_compiles(&out);
}

#[test]
fn test_inject_specs_sharing_the_same_order_never_double_bind() {
    let source = "x = input(\"Go? \")\nprint(x)\n";
    let declarations = [input_spec("input-1", 0, false, "Go? "), input_spec("input-2", 0, false, "Go? ")];
    let error = inject(source, &declarations, &[("input-1", "AAA"), ("input-2", "BBB")]).unwrap_err();
    assert!(matches!(error, LanguageError::BindingNotFound { name } if name == "input-2"));
}

#[test]
fn test_inject_triple_duplicate_specs_same_order_never_double_bind() {
    let source = "x = input(\"Go? \")\nprint(x)\n";
    let declarations = [
        input_spec("input-1", 0, false, "Go? "),
        input_spec("input-2", 0, false, "Go? "),
        input_spec("input-3", 0, false, "Go? "),
    ];
    let error = inject(source, &declarations, &[("input-1", "AAA"), ("input-2", "BBB"), ("input-3", "CCC")]).unwrap_err();
    assert!(matches!(error, LanguageError::BindingNotFound { name } if name == "input-2" || name == "input-3"));
}

#[test]
fn test_preamble_inserted_at_end_for_no_docstring_no_future() {
    let source = "x = input('v: ')\nprint(x)\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "")], &[("input-1", "hi")]).unwrap();
    assert!(out.lines().next().is_some_and(|line| line.ends_with("# skit:shim")), "{out}");
}

#[test]
fn test_multiline_span_replacement() {
    let source = "X = (\n    \"old\"\n    \"also old\"\n)\nprint(X)\n";
    let out = inject(source, &[const_spec("X", ParameterType::Str)], &[("X", "new")]).unwrap();
    assert!(out.contains("'new'"), "{out}");
    assert_python_compiles(&out);
}

#[test]
fn test_const_injection_survives_form_feed_between_targets() {
    let source = "HOST = \"localhost\"\n\u{000c}\nPORT = 8080\nprint(HOST, PORT)\n";
    let out = inject(source, &[const_spec("PORT", ParameterType::Int)], &[("PORT", "9090")]).unwrap();
    assert_eq!(out, "HOST = \"localhost\"\n\u{000c}\nPORT = 9090\nprint(HOST, PORT)\n");
    assert_python_compiles(&out);
}

#[test]
fn test_const_injection_survives_u2028_inside_earlier_string_literal() {
    let source = "MSG = \"hi\u{2028}there\"\nPORT = 8080\nprint(MSG, PORT)\n";
    let out = inject(source, &[const_spec("PORT", ParameterType::Int)], &[("PORT", "9090")]).unwrap();
    assert_eq!(out, "MSG = \"hi\u{2028}there\"\nPORT = 9090\nprint(MSG, PORT)\n");
    assert_python_compiles(&out);
}
