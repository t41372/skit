//! Every configuration error must present a complete message in each supported locale.

use skit_i18n::{Locale, Localize, Message};
use skit_store::ConfigError;

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
fn every_config_error_localizes_and_keeps_its_values() {
    assert_localized(
        &ConfigError::Io {
            operation: "lock",
            path: "/config/config.toml".to_owned(),
            reason: "device is busy".to_owned(),
        },
        &["/config/config.toml", "device is busy"],
    );
    assert_localized(
        &ConfigError::Io {
            operation: "backup",
            path: "/config/config.toml.bak".to_owned(),
            reason: "the backup path is not a regular file".to_owned(),
        },
        &[
            "/config/config.toml.bak",
            "the backup path is not a regular file",
        ],
    );
    assert_localized(
        &ConfigError::Parse {
            path: "/config/config.toml".to_owned(),
            reason: "expected `=`".to_owned(),
        },
        &["/config/config.toml", "expected `=`"],
    );
    assert_localized(
        &ConfigError::Encode {
            reason: "unsupported value".to_owned(),
        },
        &["unsupported value"],
    );
    assert_localized(
        &ConfigError::Invalid(
            Message::new("Unknown setting: {}. Available: {}")
                .with("colour")
                .with("lang, editor, mirror"),
        ),
        &["colour", "lang, editor, mirror"],
    );
    assert_localized(
        &ConfigError::Usage(Message::new("No such file: {}").with("/missing/bash")),
        &["/missing/bash"],
    );
}

#[test]
fn a_multi_value_message_places_every_value_in_its_own_hole() {
    // `contains` cannot see a swapped hole order, so pin the whole rendering.
    let error = ConfigError::Io {
        operation: "lock",
        path: "/config/config.toml".to_owned(),
        reason: "device is busy".to_owned(),
    };

    assert_eq!(
        error.message().localize(Locale::En),
        "could not lock configuration at /config/config.toml: device is busy"
    );
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "无法锁定 /config/config.toml 处的配置：device is busy"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "無法鎖定 /config/config.toml 處的組態：device is busy"
    );
}

#[test]
fn a_rejected_configuration_value_stays_verbatim() {
    // `exit` and `stay` are catalog words, so a value must never change.
    let error = ConfigError::Usage(
        Message::new("Unknown after-run behavior: {}. Choose from: exit, stay").with("on-off"),
    );
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "未知的运行后行为：on-off。可选：exit、stay"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "未知的執行後行為：on-off。可選：exit、stay"
    );
}
