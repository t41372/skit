//! Public-API ports of Python v0.4 default-value injection regressions.
//!
//! A form value is authoritative even when it equals the current source default. Skipping the edit
//! can expose a later assignment or leave an interactive `input()` live under `--no-input`.

use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::inject_values;

fn const_decl(name: &str, default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

#[test]
fn test_value_equal_to_the_source_default_is_still_injected() {
    let source = "GREETING = 'bonjour'\nprint(GREETING)\n";
    let declaration = const_decl("GREETING", "bonjour");
    let values = BTreeMap::from([("GREETING".to_owned(), "bonjour".to_owned())]);

    let output = inject_values("python", source, &[declaration], &values).unwrap();

    assert!(output.contains("bonjour"));
    assert!(output.contains("print(GREETING)"));
}

#[test]
fn test_input_binding_with_a_default_is_intercepted_instead_of_left_live() {
    let source = "name = input(\"Your name? \")\nprint(name)\n";
    let mut declaration = ParamDecl::new("input-1");
    declaration.binding = ParameterBinding::Input;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String("Tim".to_owned()));
    declaration.order = 0;
    declaration.prompt = "Your name? ".to_owned();
    let values = BTreeMap::from([("input-1".to_owned(), "Tim".to_owned())]);

    let output = inject_values("python", source, &[declaration], &values).unwrap();

    assert!(!output.contains("input(\"Your name? \")"), "{output}");
    assert!(output.contains("Tim"), "{output}");
    assert!(output.contains("print(name)"), "{output}");
}

#[test]
fn test_main_guard_override_receives_the_unchanged_default_too() {
    let source = r#"HOST = "localhost"

if __name__ == "__main__":
    HOST = "127.0.0.1"
    print(HOST)
"#;
    let declaration = const_decl("HOST", "localhost");
    let values = BTreeMap::from([("HOST".to_owned(), "localhost".to_owned())]);

    let output = inject_values("python", source, &[declaration], &values).unwrap();

    assert_eq!(output.matches("localhost").count(), 2, "{output}");
    assert!(!output.contains("127.0.0.1"), "{output}");
}

#[test]
fn test_cleared_free_text_const_is_injected_as_an_explicit_empty_string() {
    let source = "GREETING = 'bonjour'\nprint(GREETING)\n";
    let declaration = const_decl("GREETING", "bonjour");
    let values = BTreeMap::from([("GREETING".to_owned(), String::new())]);

    let output = inject_values("python", source, &[declaration], &values).unwrap();

    assert!(!output.contains("bonjour"), "{output}");
    assert!(output.contains("GREETING"), "{output}");
}
