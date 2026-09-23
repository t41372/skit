//! Every domain error must present a complete message in each supported locale.

use skit_domain::{
    DomainError,
    parameters::{ParameterType, coerce_default},
};
use skit_i18n::{Locale, Localize};

/// Check that English text does not drift and that each locale keeps the value.
fn assert_localized(error: &(impl Localize + std::fmt::Display), value: &str) {
    let message = error.message();
    assert_eq!(error.to_string(), message.localize(Locale::En));
    for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw] {
        let text = message.localize(locale);
        let template = message.template();
        assert!(!text.trim().is_empty(), "{template} is empty");
        assert!(!text.contains("{}"), "{template} kept an empty hole");
        assert!(text.contains(value), "{text} lost the value {value}");
    }
}

#[test]
fn every_domain_error_localizes_and_keeps_its_value() {
    assert_localized(&DomainError::InvalidSlug("Bad Slug".to_owned()), "Bad Slug");
    assert_localized(
        &DomainError::InvalidEntryId("not-a-uuid".to_owned()),
        "not-a-uuid",
    );

    let blank = DomainError::InvalidKind;
    assert_eq!(blank.to_string(), blank.message().localize(Locale::En));
    assert_eq!(blank.message().localize(Locale::ZhCn), "条目类型不能为空");
    assert_eq!(blank.message().localize(Locale::ZhTw), "項目類型不能為空");
}

#[test]
fn a_default_coercion_failure_localizes_and_keeps_its_value() {
    let error = coerce_default("twelve", ParameterType::Int).unwrap_err();
    assert_localized(&error, "twelve");
    assert!(error.message().localize(Locale::ZhCn).contains("默认值"));
    assert!(error.message().localize(Locale::ZhTw).contains("預設值"));
}
