//! Exact public-surface ports from Python v0.4 `tests/test_js_inject_mut.py` whose observable
//! contract maps directly to Rust's public source-injection API.
//!
//! Private Node gate/suffix/runner helper tests are deliberately not recreated here. Behavioral
//! mismatches in these public cases stay red; this branch does not patch the injector.

use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType,
};
use skit_language::{LanguageError, detect_candidates, inject_values};

fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

#[test]
fn test_bad_value_error_carries_the_raw_value_and_type() {
    let declarations = [const_decl("W", ParameterType::Int)];
    let values = BTreeMap::from([("W".to_owned(), "not-a-number".to_owned())]);

    let error = inject_values("js", "const W = 800;\n", &declarations, &values).unwrap_err();

    assert_eq!(
        error,
        LanguageError::InvalidValue {
            name: "W".to_owned(),
            value: "not-a-number".to_owned(),
            parameter_type: ParameterType::Int,
        }
    );
}

#[test]
fn test_destructuring_binding_is_never_an_injection_target() {
    let declarations = [const_decl("{a}", ParameterType::Int)];
    let values = BTreeMap::from([("{a}".to_owned(), "9".to_owned())]);

    let error = inject_values("js", "const {a} = 5;\n", &declarations, &values).unwrap_err();

    assert_eq!(
        error,
        LanguageError::BindingNotFound {
            name: "{a}".to_owned(),
        }
    );
}

#[test]
fn test_a_spec_without_a_value_does_not_stop_later_injection() {
    let source = "const A = 1;\nconst B = 2;\n";
    let declarations = detect_candidates("js", source);
    assert_eq!(
        declarations.iter().map(|decl| decl.name.as_str()).collect::<Vec<_>>(),
        ["A", "B"]
    );

    let output = inject_values(
        "js",
        source,
        &declarations,
        &BTreeMap::from([("B".to_owned(), "9".to_owned())]),
    )
    .unwrap();

    assert!(output.contains("const A = 1;"), "{output}");
    assert!(output.contains("const B = 9;"), "{output}");
    assert!(!output.contains("const B = 2;"), "{output}");
}

#[test]
fn test_all_drifted_targets_are_collected_into_one_error() {
    let declarations = [
        const_decl("AA", ParameterType::Str),
        const_decl("BB", ParameterType::Str),
    ];
    let values = BTreeMap::from([
        ("AA".to_owned(), "x".to_owned()),
        ("BB".to_owned(), "y".to_owned()),
    ]);

    let error = inject_values("js", "const Z = 1;\n", &declarations, &values).unwrap_err();

    // Python's public InjectError reports every stale target together, in declaration order.
    // Do not weaken this to "contains AA": that would let the second lost target disappear.
    assert_eq!(error.to_string(), "AA, BB");
}
