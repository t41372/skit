//! Parser-backed public ports from Python v0.4 `tests/test_js_inject.py` at `main@206f9ef`.
//!
//! These tests derive declarations from the real JS/TS analyzer and feed them to the real injector.
//! They never recreate the injector in test code. Filesystem staging/runtime-gate contracts live in
//! the CLI integration port; Python-private runner/gate fault seams are accounted separately.

use std::collections::BTreeMap;

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType,
};
use skit_language::{LanguageError, ParseOutcome, inject_values, parse_document};

fn specs_of(kind: &str, source: &str) -> Vec<ParamDecl> {
    let ParseOutcome::Parsed(document) = parse_document(kind, source) else {
        panic!("expected valid {kind} source");
    };
    document
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration)
        .collect()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn inject(kind: &str, source: &str, pairs: &[(&str, &str)]) -> Result<String, LanguageError> {
    inject_values(kind, source, &specs_of(kind, source), &values(pairs))
}

#[test]
fn test_int_injects_a_bare_number() {
    assert_eq!(
        inject("js", "const W = 800;\n", &[("W", "1200")]).unwrap(),
        "const W = 1200;\n"
    );
}

#[test]
fn test_float_injects_a_bare_number() {
    assert_eq!(
        inject("js", "const R = 0.5;\n", &[("R", "2.75")]).unwrap(),
        "const R = 2.75;\n"
    );
}

#[test]
fn test_string_injects_a_json_dumps_literal() {
    assert_eq!(
        inject("js", "const C = \"x\";\n", &[("C", "New York")]).unwrap(),
        "const C = \"New York\";\n"
    );
}

#[test]
fn test_string_json_escapes_quote_backslash_newline() {
    let output = inject("js", "const M = \"x\";\n", &[("M", "a\"b\\c\nd")]).unwrap();
    assert_eq!(output, "const M = \"a\\\"b\\\\c\\nd\";\n");
    assert!(matches!(parse_document("js", &output), ParseOutcome::Parsed(_)));
}

#[test]
fn test_cjk_and_emoji_escape_to_valid_js() {
    let output = inject("js", "const C = \"x\";\n", &[("C", "高雄 🚀")]).unwrap();
    assert!(output.contains("\\u9ad8"), "Python json.dumps compatibility was lost: {output}");
    assert!(matches!(parse_document("js", &output), ParseOutcome::Parsed(_)));
}

#[test]
fn test_bool_injects_true_or_false_lowercase() {
    assert_eq!(
        inject("js", "const B = false;\n", &[("B", "yes")]).unwrap(),
        "const B = true;\n"
    );
    assert_eq!(
        inject("js", "const B = true;\n", &[("B", "0")]).unwrap(),
        "const B = false;\n"
    );
}

#[test]
fn test_rewrites_every_same_name_occurrence() {
    let source = "var M = 1;\nvar M = 2;\nconsole.log(M);\n";
    let output = inject("js", source, &[("M", "9")]).unwrap();
    assert_eq!(output.matches("= 9;").count(), 2);
    assert_eq!(output, "var M = 9;\nvar M = 9;\nconsole.log(M);\n");
}

#[test]
fn test_same_name_nonliteral_declaration_is_not_a_target() {
    let source = "var M = 1;\nvar M = compute();\n";
    let output = inject("js", source, &[("M", "9")]).unwrap();
    assert_eq!(output, "var M = 9;\nvar M = compute();\n");
}

#[test]
fn test_missing_target_is_drift_not_value_error() {
    let mut spec = ParamDecl::new("GONE");
    spec.binding = ParameterBinding::Const;
    spec.delivery = ParameterDelivery::Inject;
    spec.parameter_type = ParameterType::Str;
    let error = inject_values(
        "js",
        "const W = 800;\n",
        &[spec],
        &values(&[("GONE", "x")]),
    )
    .unwrap_err();
    assert_eq!(
        error,
        LanguageError::BindingNotFound {
            name: "GONE".to_owned()
        }
    );
}

#[test]
fn test_bad_int_value_raises_value_error() {
    let error = inject("js", "const W = 800;\n", &[("W", "not-a-number")]).unwrap_err();
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
fn test_bad_float_and_non_finite_are_refused() {
    for bad in ["abc", "inf", "-inf", "nan"] {
        assert_eq!(
            inject("js", "const R = 0.5;\n", &[("R", bad)]).unwrap_err(),
            LanguageError::InvalidValue {
                name: "R".to_owned(),
                value: bad.to_owned(),
                parameter_type: ParameterType::Float,
            }
        );
    }
}

#[test]
fn test_bad_bool_value_raises_value_error() {
    assert_eq!(
        inject("js", "const B = true;\n", &[("B", "maybe")]).unwrap_err(),
        LanguageError::InvalidValue {
            name: "B".to_owned(),
            value: "maybe".to_owned(),
            parameter_type: ParameterType::Bool,
        }
    );
}

#[test]
fn test_no_values_writes_nothing() {
    let source = "const W = 800;\n";
    assert_eq!(inject_values("js", source, &specs_of("js", source), &BTreeMap::new()).unwrap(), source);
}

#[test]
fn test_value_for_unmanaged_name_is_ignored() {
    let source = "const W = 800;\n";
    assert_eq!(
        inject_values("js", source, &[], &values(&[("OTHER", "x")])).unwrap(),
        source
    );
}
