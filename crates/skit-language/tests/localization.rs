//! Every language error must present a complete message in each supported locale.

use skit_i18n::{Locale, Localize, Message};
use skit_language::{
    LanguageError, PythonMetadataError, ShellInputError, validate_pep440_specifiers,
    validate_pep508_requirement,
};

/// Check that English text does not drift and that each locale keeps the values.
fn assert_localized(error: &(impl Localize + std::fmt::Display), values: &[&str]) {
    let message = error.message();
    assert_eq!(error.to_string(), message.localize(Locale::En));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let text = message.localize(locale);
        let template = message.template();
        assert!(!text.trim().is_empty(), "{template} is empty");
        assert!(!text.contains("{}"), "{template} kept an empty hole");
        for value in values {
            assert!(text.contains(value), "{text} lost the value {value}");
        }
    }
}

#[test]
fn every_language_error_localizes_and_keeps_its_values() {
    assert_localized(
        &LanguageError::UnsupportedKind {
            kind: "prompt".to_owned(),
        },
        &["prompt"],
    );
    assert_localized(
        &LanguageError::BindingNotFound {
            name: "target".to_owned(),
        },
        &["target"],
    );
    assert_localized(
        &LanguageError::InvalidMetadata {
            reason: Message::new("tool is not a table"),
        },
        &[],
    );
    assert_localized(
        &LanguageError::InvalidMetadata {
            reason: Message::new("tool.skit is not a table"),
        },
        &[],
    );
    assert_localized(
        &LanguageError::InvalidSource {
            kind: "python".to_owned(),
        },
        &["python"],
    );
    assert_localized(&LanguageError::SourceChanged, &[]);
    assert_localized(
        &LanguageError::InvalidValue {
            name: "count".to_owned(),
            value: "many".to_owned(),
            parameter_type: skit_domain::parameters::ParameterType::Int,
        },
        &["count", "many", "int"],
    );
    for source in [
        ShellInputError::Gap {
            empty: "input-1".to_owned(),
            filled: "input-2".to_owned(),
        },
        ShellInputError::LineBreak {
            name: "input-1".to_owned(),
        },
        ShellInputError::FieldSplit {
            name: "input-1".to_owned(),
        },
        ShellInputError::EdgeSpace {
            name: "input-1".to_owned(),
        },
    ] {
        let error = LanguageError::ShellInput(source);
        assert_localized(&error, &["input-1"]);
    }
}

#[test]
fn package_metadata_errors_localize_and_keep_the_value_verbatim() {
    // `not` and `version` are catalog words. Neither may change inside the value.
    let requirement = validate_pep508_requirement("!!!not valid!!!").unwrap_err();
    assert_localized(&requirement, &["!!!not valid!!!"]);
    assert!(
        requirement
            .message()
            .localize(Locale::ZhCn)
            .contains("不是有效的 PEP 508 依赖描述")
    );

    let constraint = validate_pep440_specifiers("not a version").unwrap_err();
    assert_localized(&constraint, &["not a version"]);
    assert!(
        constraint
            .message()
            .localize(Locale::ZhTw)
            .contains("不是有效的 PEP 440 版本限制")
    );

    assert!(matches!(
        requirement,
        PythonMetadataError::InvalidRequirement { .. }
    ));
    assert!(matches!(
        constraint,
        PythonMetadataError::InvalidVersionConstraint { .. }
    ));
}

#[test]
fn a_comment_metadata_failure_reports_a_localized_reason() {
    let error = LanguageError::InvalidMetadata {
        reason: Message::new("the comment metadata block is not valid TOML: {}")
            .with("expected `=`"),
    };
    assert_localized(&error, &["expected `=`"]);
    assert!(
        error
            .message()
            .localize(Locale::ZhCn)
            .contains("注释元数据块不是有效的 TOML")
    );
}
