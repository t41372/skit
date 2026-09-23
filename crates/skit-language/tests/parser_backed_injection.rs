use std::collections::BTreeMap;

use skit_domain::parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};
use skit_language::{inject_values, normalize_shell_default};

fn injected(name: &str, binding: ParameterBinding) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = binding;
    declaration.delivery = ParameterDelivery::Inject;
    declaration
}

#[test]
fn shell_and_javascript_injection_rewrite_every_parser_selected_constant() {
    let shell = "VALUE=first\nprintf '%s\\n' \"$VALUE\"\nVALUE=last\n";
    let rewritten = inject_values(
        "shell",
        shell,
        &[injected("VALUE", ParameterBinding::Const)],
        &BTreeMap::from([("VALUE".to_owned(), "a'b".to_owned())]),
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "VALUE='a'\\''b'\nprintf '%s\\n' \"$VALUE\"\nVALUE='a'\\''b'\n"
    );

    let javascript = "const VALUE = 'first';\nlet OTHER = 1;\nconst VALUE = 'last';\n";
    let rewritten = inject_values(
        "js",
        javascript,
        &[injected("VALUE", ParameterBinding::Const)],
        &BTreeMap::from([("VALUE".to_owned(), "a\"b\n".to_owned())]),
    )
    .unwrap();
    assert_eq!(
        rewritten,
        "const VALUE = \"a\\\"b\\n\";\nlet OTHER = 1;\nconst VALUE = \"a\\\"b\\n\";\n"
    );
}

#[test]
fn javascript_typed_conversion_refuses_instead_of_quoting_the_raw_value() {
    let mut declaration = injected("COUNT", ParameterBinding::Const);
    declaration.parameter_type = ParameterType::Int;
    assert!(
        inject_values(
            "ts",
            "const COUNT: number = 1;\n",
            &[declaration],
            &BTreeMap::from([("COUNT".to_owned(), "many".to_owned())]),
        )
        .is_err()
    );
}

#[test]
fn shell_read_injection_keeps_the_original_call_and_matches_prompts() {
    let mut first = injected("first", ParameterBinding::Input);
    first.order = 0;
    first.prompt = "First: ".to_owned();
    let mut second = injected("second", ParameterBinding::Input);
    second.order = 1;
    second.prompt = "Second: ".to_owned();
    let source = concat!(
        "#!/bin/sh\n",
        "read -r -p 'Second: ' SECOND\n",
        "command read -p 'First: ' FIRST\n",
    );
    let rewritten = inject_values(
        "shell",
        source,
        &[first, second],
        &BTreeMap::from([
            ("first".to_owned(), "Ada".to_owned()),
            ("second".to_owned(), "C:\\Users".to_owned()),
        ]),
    )
    .unwrap();

    assert!(rewritten.starts_with("#!/bin/sh\n_skit_read() {\n"));
    assert!(rewritten.contains("_skit_read 0 'C:\\Users' 0 'Second: ' -r -p 'Second: ' SECOND"));
    assert!(rewritten.contains("_skit_read 1 'Ada' 0 'First: ' -p 'First: ' FIRST"));
    assert!(!rewritten.contains("SECOND=C:"));
}

#[test]
fn shell_normalization_uses_the_same_assignment_tree_as_analysis() {
    assert!(normalize_shell_default("VALUE=first\nVALUE=last\n", "VALUE").is_err());
    assert!(normalize_shell_default("VALUE='a;b'\n", "VALUE").is_err());
    assert!(normalize_shell_default("readonly VALUE=first\n", "VALUE").is_err());
    assert_eq!(
        normalize_shell_default("#!/bin/sh\r\nVALUE='hello world'\r\n", "VALUE").unwrap(),
        "#!/bin/sh\r\nVALUE=\"${VALUE:-hello world}\"\r\n"
    );
}
