//! Real-Python ports of the runtime-gated contracts in Python v0.4 `tests/test_shim.py`.
//!
//! The injector stays in `skit-language`. This integration target composes it with the production
//! runtime program probe, so Windows uses the same `PATHEXT` behavior as a product launch. These
//! contracts require Python on every supported CI platform and never skip when it is unavailable.

use std::{
    collections::BTreeMap,
    io::Write as _,
    path::PathBuf,
    process::{Command, Output, Stdio},
};

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{LanguageError, inject_values};
use skit_runtime::{ProgramProbe as _, SystemProbe};

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

fn spec(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration
}

fn input_spec(name: &str, order: i64) -> ParamDecl {
    let mut declaration = spec(name);
    declaration.binding = ParameterBinding::Input;
    declaration.order = order;
    declaration
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn inject(
    text: &str,
    specs: &[ParamDecl],
    pairs: &[(&str, &str)],
) -> Result<String, LanguageError> {
    inject_values("python", text, specs, &values(pairs))
}

fn resolved_python() -> PathBuf {
    SystemProbe
        .find_program("python3")
        .or_else(|| SystemProbe.find_program("python"))
        .expect("the frozen Shim runtime contracts require Python on this platform")
}

fn python_output(code: &str, stdin: &str) -> Output {
    let program = resolved_python();
    let mut child = Command::new(&program)
        .arg("-c")
        .arg(code)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {}: {error}", program.display()));
    child
        .stdin
        .take()
        .expect("Python stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("write Python stdin");
    child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to wait for {}: {error}", program.display()))
}

fn run_injected(source: &str, stdin: &str) -> String {
    let output = python_output(source, stdin);
    assert!(
        output.status.success(),
        "Python rejected or failed the injected source: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("Python stdout is UTF-8")
}

fn assert_python_compiles(source: &str) {
    let output = python_output(
        "import sys; compile(sys.stdin.read(), '<shim-parity>', 'exec')",
        source,
    );
    assert!(
        output.status.success(),
        "Python rejected the injected source: {}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_input_queue_by_order() {
    let out = inject(SCRIPT, &[input_spec("input-1", 0)], &[("input-1", "Alice")]).unwrap();
    assert!(out.contains(r#"who = _skit_i[0]("Your name: ")"#));
    assert!(out.contains("# skit:shim"));
    let stdout = run_injected(&out, "");
    assert!(stdout.contains("Alice Taipei 3"));
    assert!(stdout.contains("Your name: Alice"));
}

#[test]
fn test_input_queue_exhaustion_falls_back_to_stdin() {
    let source = "a = input('a: ')\nb = input('b: ')\nprint(a, b)\n";
    let out = inject(source, &[input_spec("input-1", 0)], &[("input-1", "one")]).unwrap();
    assert!(run_injected(&out, "two\n").contains("one two"));
}

#[test]
fn test_input_queue_in_loop_consumes_by_call_order() {
    let source = "vals = [input('v: ') for _ in range(3)]\nprint('|'.join(vals))\n";
    let out = inject(source, &[input_spec("input-1", 0)], &[("input-1", "first")]).unwrap();
    assert!(run_injected(&out, "second\nthird\n").contains("first|second|third"));
}

#[test]
fn test_input_queue_secret_masks_echo() {
    let source = "token = input('token: ')\nprint('len', len(token))\n";
    let mut secret = input_spec("input-1", 0);
    secret.secret = true;
    let out = inject(source, &[secret], &[("input-1", "hunter2")]).unwrap();
    let stdout = run_injected(&out, "");
    assert!(!stdout.contains("hunter2"));
    assert!(stdout.contains("token: ***"));
    assert!(stdout.contains("len 7"));
}

#[test]
fn test_input_queue_with_future_import() {
    let source = "\"\"\"doc\"\"\"\nfrom __future__ import annotations\nx = input()\nprint(x)\n";
    let out = inject(source, &[input_spec("input-1", 0)], &[("input-1", "ok")]).unwrap();
    let lines = out.lines().collect::<Vec<_>>();
    assert_eq!(lines[1], "from __future__ import annotations");
    assert!(lines[2].ends_with("# skit:shim"));
    assert!(run_injected(&out, "").contains("ok"));
}

#[test]
fn test_input_queue_combined_with_const_injection() {
    let out = inject(
        SCRIPT,
        &[spec("CITY"), input_spec("input-1", 0)],
        &[("CITY", "Tainan"), ("input-1", "Bob")],
    )
    .unwrap();
    assert!(out.contains("CITY = 'Tainan'"));
    assert!(run_injected(&out, "").contains("Bob Tainan 3"));
}

#[test]
fn test_unshadowed_input_is_rewritten_to_the_wrapper() {
    let mut declaration = input_spec("input-1", 0);
    declaration.prompt = "Q: ".to_owned();
    let out = inject(
        "y = input('Q: ')\nprint(y)\n",
        &[declaration],
        &[("input-1", "typed")],
    )
    .unwrap();
    assert!(out.contains("y = _skit_i[0]('Q: ')"));
    assert!(run_injected(&out, "").contains("typed"));
}

#[test]
fn test_preamble_insertion_survives_form_feed_inside_docstring() {
    let source = "\"\"\"line one\u{0c}line two\"\"\"\nname = input(\"who: \")\nprint(name)\n";
    let out = inject(source, &[input_spec("input-1", 0)], &[("input-1", "Bob")]).unwrap();
    assert!(out.starts_with("\"\"\"line one\u{0c}line two\"\"\"\n"));
    assert!(out.contains("# skit:shim"));
    assert!(run_injected(&out, "").contains("Bob"));
}

#[test]
fn test_input_value_follows_prompt_despite_runtime_call_order_diverging_from_source_order() {
    let source = concat!(
        "def get_password():\n",
        "    return input(\"Password: \")\n",
        "\n",
        "username = input(\"Username: \")\n",
        "password = get_password()\n",
        "print(username, password)\n",
    );
    let mut password = input_spec("input-1", 0);
    password.secret = true;
    let username = input_spec("input-2", 1);
    let out = inject(
        source,
        &[password, username],
        &[("input-1", "SUPERSECRET"), ("input-2", "alice")],
    )
    .unwrap();
    let stdout = run_injected(&out, "");
    assert!(stdout.contains("alice SUPERSECRET"));
    assert!(
        !stdout
            .replace("alice SUPERSECRET", "")
            .contains("SUPERSECRET")
    );
    assert!(stdout.contains("Password: ***"));
    assert!(stdout.contains("Username: alice"));
}

#[test]
fn test_input_value_follows_prompt_after_an_earlier_input_is_deleted() {
    let source = "password = input(\"Password: \")\nprint(\"got\", password)\n";
    let mut stored = input_spec("input-2", 1);
    stored.secret = true;
    stored.prompt = "Password: ".to_owned();
    let out = inject(source, &[stored], &[("input-2", "hunter2")]).unwrap();
    let stdout = run_injected(&out, "");
    assert!(stdout.contains("got hunter2"));
    assert!(stdout.contains("Password: ***"));
}

#[test]
fn test_multiline_value_span() {
    let source = "MSG = (\n    \"hello\"\n)\nprint(MSG)\n";
    let out = inject(source, &[spec("MSG")], &[("MSG", "bye")]).unwrap();
    assert!(out.contains("'bye'"));
    assert!(!out.contains("\"hello\""));
    assert_python_compiles(&out);
}

#[test]
fn test_inject_duplicate_prompt_winner_only_still_injects_and_compiles() {
    let source = "first = input(\"Go? \")\nsecond = input(\"Go? \")\nprint(first, second)\n";
    let edited = source.replace("first = input(\"Go? \")\n", "");
    let mut input_one = input_spec("input-1", 0);
    input_one.prompt = "Go? ".to_owned();
    let out = inject(&edited, &[input_one], &[("input-1", "AAA")]).unwrap();
    assert!(!out.contains("_skit_i[0]_i[0]"));
    assert_python_compiles(&out);
}

#[test]
fn test_multiline_span_replacement() {
    let source = "X = (\n    \"old\"\n    \"also old\"\n)\nprint(X)\n";
    let out = inject(source, &[spec("X")], &[("X", "new")]).unwrap();
    assert!(out.contains("'new'"));
    assert_python_compiles(&out);
}

#[test]
fn test_const_injection_survives_form_feed_between_targets() {
    let source = "HOST = \"localhost\"\n\u{0c}\nPORT = 8080\nprint(HOST, PORT)\n";
    let mut port = spec("PORT");
    port.parameter_type = ParameterType::Int;
    let out = inject(source, &[port], &[("PORT", "9090")]).unwrap();
    assert_eq!(
        out,
        "HOST = \"localhost\"\n\u{0c}\nPORT = 9090\nprint(HOST, PORT)\n"
    );
    assert_python_compiles(&out);
}

#[test]
fn test_const_injection_survives_u2028_inside_earlier_string_literal() {
    let source = "MSG = \"hi\u{2028}there\"\nPORT = 8080\nprint(MSG, PORT)\n";
    let mut port = spec("PORT");
    port.parameter_type = ParameterType::Int;
    let out = inject(source, &[port], &[("PORT", "9090")]).unwrap();
    assert_eq!(
        out,
        "MSG = \"hi\u{2028}there\"\nPORT = 9090\nprint(MSG, PORT)\n"
    );
    assert_python_compiles(&out);
}
