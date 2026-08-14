//! Real-interpreter ports of Python v0.4 shim runtime contracts.

use std::{
    collections::BTreeMap,
    io::Write as _,
    process::{Command, Stdio},
};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType,
};
use skit_language::inject_values;

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

fn spec(name: &str, binding: ParameterBinding, order: i64, secret: bool, prompt: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = binding;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.order = order;
    declaration.secret = secret;
    declaration.prompt = prompt.to_owned();
    declaration
}

fn const_spec(name: &str) -> ParamDecl { spec(name, ParameterBinding::Const, -1, false, "") }
fn input_spec(name: &str, order: i64, secret: bool, prompt: &str) -> ParamDecl {
    spec(name, ParameterBinding::Input, order, secret, prompt)
}

fn inject(source: &str, declarations: &[ParamDecl], pairs: &[(&str, &str)]) -> String {
    let values = pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    inject_values("python", source, declarations, &values).unwrap()
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

fn run_python(source: &str, stdin: &str) -> String {
    let mut child = Command::new(python_program())
        .args(["-c", source])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "real Python rejected/failed injected source:\n{}\nsource:\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn test_input_queue_by_order() {
    let out = inject(SCRIPT, &[input_spec("input-1", 0, false, "")], &[("input-1", "Alice")]);
    assert!(out.contains("who = _skit_i[0](\"Your name: \")"), "{out}");
    assert!(out.contains("# skit:shim"), "{out}");
    let stdout = run_python(&out, "");
    assert!(stdout.contains("Alice Taipei 3"), "{stdout}");
    assert!(stdout.contains("Your name: Alice"), "{stdout}");
}

#[test]
fn test_input_queue_exhaustion_falls_back_to_stdin() {
    let source = "a = input('a: ')\nb = input('b: ')\nprint(a, b)\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "")], &[("input-1", "one")]);
    let stdout = run_python(&out, "two\n");
    assert!(stdout.contains("one two"), "{stdout}");
}

#[test]
fn test_input_queue_in_loop_consumes_by_call_order() {
    let source = "vals = [input('v: ') for _ in range(3)]\nprint('|'.join(vals))\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "")], &[("input-1", "first")]);
    let stdout = run_python(&out, "second\nthird\n");
    assert!(stdout.contains("first|second|third"), "{stdout}");
}

#[test]
fn test_input_queue_secret_masks_echo() {
    let source = "token = input('token: ')\nprint('len', len(token))\n";
    let out = inject(source, &[input_spec("input-1", 0, true, "")], &[("input-1", "hunter2")]);
    let stdout = run_python(&out, "");
    assert!(!stdout.contains("hunter2"), "{stdout}");
    assert!(stdout.contains("token: ***"), "{stdout}");
    assert!(stdout.contains("len 7"), "{stdout}");
}

#[test]
fn test_input_queue_with_future_import() {
    let source = "\"\"\"doc\"\"\"\nfrom __future__ import annotations\nx = input()\nprint(x)\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "")], &[("input-1", "ok")]);
    let lines = out.lines().collect::<Vec<_>>();
    assert_eq!(lines[1], "from __future__ import annotations");
    assert!(lines[2].ends_with("# skit:shim"), "{out}");
    assert!(run_python(&out, "").contains("ok"));
}

#[test]
fn test_input_queue_combined_with_const_injection() {
    let out = inject(
        SCRIPT,
        &[const_spec("CITY"), input_spec("input-1", 0, false, "")],
        &[("CITY", "Tainan"), ("input-1", "Bob")],
    );
    assert!(out.contains("CITY = 'Tainan'"), "{out}");
    assert!(run_python(&out, "").contains("Bob Tainan 3"));
}

#[test]
fn test_unshadowed_input_is_rewritten_to_the_wrapper() {
    let source = "y = input('Q: ')\nprint(y)\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "Q: ")], &[("input-1", "typed")]);
    assert!(out.contains("y = _skit_i[0]('Q: ')"), "{out}");
    assert!(run_python(&out, "").contains("typed"));
}

#[test]
fn test_preamble_insertion_survives_form_feed_inside_docstring() {
    let source = "\"\"\"line one\u{000c}line two\"\"\"\nname = input(\"who: \")\nprint(name)\n";
    let out = inject(source, &[input_spec("input-1", 0, false, "")], &[("input-1", "Bob")]);
    assert!(out.starts_with("\"\"\"line one\u{000c}line two\"\"\"\n"), "{out}");
    assert!(out.contains("# skit:shim"), "{out}");
    assert!(run_python(&out, "").contains("Bob"));
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
    let declarations = [
        input_spec("input-1", 0, true, ""),
        input_spec("input-2", 1, false, ""),
    ];
    let out = inject(source, &declarations, &[("input-1", "SUPERSECRET"), ("input-2", "alice")]);
    let stdout = run_python(&out, "");
    assert!(stdout.contains("alice SUPERSECRET"), "{stdout}");
    assert!(stdout.contains("Password: ***"), "{stdout}");
    assert!(stdout.contains("Username: alice"), "{stdout}");
    let without_script_value = stdout.replace("alice SUPERSECRET", "");
    assert!(!without_script_value.contains("SUPERSECRET"), "{stdout}");
}

#[test]
fn test_input_value_follows_prompt_after_an_earlier_input_is_deleted() {
    let source = "password = input(\"Password: \")\nprint(\"got\", password)\n";
    let declarations = [input_spec("input-2", 1, true, "Password: ")];
    let out = inject(source, &declarations, &[("input-2", "hunter2")]);
    let stdout = run_python(&out, "");
    assert!(stdout.contains("got hunter2"), "{stdout}");
    assert!(stdout.contains("Password: ***"), "{stdout}");
}
