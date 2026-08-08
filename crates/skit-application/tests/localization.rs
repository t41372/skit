//! Every application error must present a complete message in each supported locale.

use skit_application::{
    RepositoryError, delivery::AssemblyError, form_state::StateWriteError,
    run_inputs::RunInputError, tokens::TokenError, value_preparation::ValuePreparationError,
    value_resolution::ValueResolutionError,
};
use skit_domain::parameters::ParameterType;
use skit_i18n::{Locale, Localize, Message};

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
fn every_repository_error_localizes_and_keeps_its_values() {
    assert_localized(
        &RepositoryError::NotFound {
            query: "list-me".to_owned(),
        },
        &["list-me"],
    );
    assert_localized(
        &RepositoryError::Ambiguous {
            query: "Report".to_owned(),
            candidates: vec!["report".to_owned(), "report-2".to_owned()],
        },
        &["Report", "report-2"],
    );
    assert_localized(
        &RepositoryError::Conflict {
            name: "Taken".to_owned(),
            slug: "taken".to_owned(),
        },
        &["Taken", "taken"],
    );
    assert_localized(
        &RepositoryError::InvalidMutation {
            reason: Message::new("entry name cannot be blank"),
        },
        &[],
    );
    assert_localized(
        &RepositoryError::StaleEntry {
            slug: "moved".to_owned(),
        },
        &["moved"],
    );
    assert_localized(
        &RepositoryError::SourceChanged {
            slug: "edited".to_owned(),
            expected: "aaa".to_owned(),
            actual: "bbb".to_owned(),
        },
        &["edited", "aaa", "bbb"],
    );
    assert_localized(
        &RepositoryError::Corrupt {
            slug: "broken".to_owned(),
            reason: "unexpected key".to_owned(),
        },
        &["broken", "unexpected key"],
    );
    assert_localized(
        &RepositoryError::Io {
            operation: "read",
            path: "/library/broken".to_owned(),
            reason: "permission denied".to_owned(),
        },
        &["/library/broken", "permission denied"],
    );
    assert_localized(
        &RepositoryError::Rollback {
            path: "/library/demo".to_owned(),
            primary: Box::new(RepositoryError::Io {
                operation: "rename",
                path: "/library/demo".to_owned(),
                reason: "device is busy".to_owned(),
            }),
            rollback: Box::new(RepositoryError::Io {
                operation: "remove",
                path: "/library/demo".to_owned(),
                reason: "permission denied".to_owned(),
            }),
        },
        &["/library/demo", "device is busy", "permission denied"],
    );
}

#[test]
fn a_multi_value_message_places_every_value_in_its_own_hole() {
    // `contains` cannot see a swapped hole order, so pin the whole rendering.
    let error = StateWriteError::Io {
        operation: "lock",
        path: "/state/entry.toml".to_owned(),
        reason: "device is busy".to_owned(),
    };

    assert_eq!(
        error.message().localize(Locale::En),
        "could not lock state at /state/entry.toml: device is busy"
    );
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "无法锁定 /state/entry.toml 处的状态数据：device is busy"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "無法鎖定 /state/entry.toml 處的狀態資料：device is busy"
    );
}

#[test]
fn a_nested_reason_uses_the_locale_of_its_parent() {
    let error = RepositoryError::InvalidMutation {
        reason: Message::new("entry name cannot be blank"),
    };
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "无效的条目变更：条目名称不能为空"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "無效的項目變更：項目名稱不能為空"
    );
}

#[test]
fn every_state_write_error_localizes_and_keeps_its_values() {
    assert_localized(
        &StateWriteError::Io {
            operation: "lock",
            path: "/state/entry".to_owned(),
            reason: "device is busy".to_owned(),
        },
        &["/state/entry", "device is busy"],
    );
    assert_localized(
        &StateWriteError::Encode {
            reason: "unsupported value".to_owned(),
        },
        &["unsupported value"],
    );
}

#[test]
fn every_run_input_error_localizes_and_keeps_its_values() {
    assert_localized(
        &AssemblyError::UnexpectedMultiple {
            name: "target".to_owned(),
        },
        &["target"],
    );
    assert_localized(
        &TokenError::MissingEnvironment {
            name: "API_KEY".to_owned(),
            token: "{env:API_KEY}".to_owned(),
        },
        &["API_KEY", "{env:API_KEY}"],
    );
    assert_localized(
        &ValuePreparationError::Required {
            name: "target".to_owned(),
            label: "Target".to_owned(),
        },
        &["Target"],
    );
    assert_localized(
        &ValuePreparationError::InvalidType {
            name: "count".to_owned(),
            value: "many".to_owned(),
            parameter_type: ParameterType::Int,
        },
        &["count", "many"],
    );
    assert_localized(
        &ValuePreparationError::InvalidChoice {
            name: "mode".to_owned(),
            value: "fast".to_owned(),
            choices: vec!["copy".to_owned(), "reference".to_owned()],
        },
        &["mode", "fast", "reference"],
    );
    assert_localized(
        &ValueResolutionError::MissingSecretEnvironment {
            name: "token".to_owned(),
            environment: "SKIT_TOKEN".to_owned(),
        },
        &["token", "SKIT_TOKEN"],
    );
    assert_localized(
        &ValueResolutionError::Token(TokenError::MissingEnvironment {
            name: "HOME_DIR".to_owned(),
            token: "{env:HOME_DIR}".to_owned(),
        }),
        &["HOME_DIR"],
    );

    for error in [
        RunInputError::Resolution(ValueResolutionError::MissingSecretEnvironment {
            name: "token".to_owned(),
            environment: "SKIT_TOKEN".to_owned(),
        }),
        RunInputError::Preparation(ValuePreparationError::Required {
            name: "target".to_owned(),
            label: "Target".to_owned(),
        }),
        RunInputError::ExtraToken(TokenError::MissingEnvironment {
            name: "TAIL".to_owned(),
            token: "{env:TAIL}".to_owned(),
        }),
        RunInputError::Assembly(AssemblyError::UnexpectedMultiple {
            name: "target".to_owned(),
        }),
    ] {
        assert_localized(&error, &[]);
    }
}
