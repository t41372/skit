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
        &["lock", "/config/config.toml", "device is busy"],
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
        &ConfigError::Invalid(Message::new("unknown configuration key: {}").with("colour")),
        &["colour"],
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
        "无法lock /config/config.toml 处的配置：device is busy"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "無法lock /config/config.toml 處的組態：device is busy"
    );
}

#[test]
fn a_rejected_configuration_value_stays_verbatim() {
    // `on` and `off` are catalog words, so a value must never change.
    let error = ConfigError::Invalid(
        Message::new("invalid configuration value for {}: {}")
            .with("after_run")
            .with("on-off"),
    );
    assert_eq!(
        error.message().localize(Locale::ZhCn),
        "after_run 的配置值无效：on-off"
    );
    assert_eq!(
        error.message().localize(Locale::ZhTw),
        "after_run 的組態值無效：on-off"
    );
}
