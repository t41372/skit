//! Exact-name metadata-model ports from Python v0.4 `tests/test_declared_params.py`.

use serde_json::json;
use skit_domain::{
    EntryMeta, EntrySettings,
    parameters::{ParamDecl, ParameterDelivery, ParameterType},
};

#[test]
fn test_write_read_parameters_roundtrip_and_legacy_params_untouched() {
    let mut meta = EntryMeta::default();
    let mut a = ParamDecl::new("a");
    a.delivery = ParameterDelivery::Placeholder;
    a.parameter_type = ParameterType::Int;
    a.required = false;
    let settings = EntrySettings {
        params: vec!["a".to_owned(), "b".to_owned()],
        parameters: vec![a.clone()],
        ..EntrySettings::default()
    };
    settings.write_to_meta(&mut meta);

    let back = EntrySettings::from_meta(&meta);
    assert_eq!(back.parameters, [a]);
    assert_eq!(back.params, ["a", "b"]);

    let mut cleared = back;
    cleared.parameters.clear();
    cleared.write_to_meta(&mut meta);
    let after = EntrySettings::from_meta(&meta);
    assert!(after.parameters.is_empty());
    assert_eq!(after.params, ["a", "b"]);
    assert!(
        !meta.extra.contains_key("parameters"),
        "clearing declared rows must remove the optional parameters key"
    );
}

#[test]
fn test_meta_parameters_roundtrip_and_non_dict_rows_dropped() {
    let mut meta = EntryMeta::default();
    meta.extra.insert(
        "parameters".to_owned(),
        json!([
            {"name": "a", "delivery": "placeholder"},
            "garbage",
            5
        ]),
    );
    let settings = EntrySettings::from_meta(&meta);
    assert_eq!(settings.parameters.len(), 1);
    assert_eq!(settings.parameters[0].name, "a");
    assert_eq!(settings.parameters[0].delivery, ParameterDelivery::Placeholder);

    let mut rewritten = EntryMeta::default();
    settings.write_to_meta(&mut rewritten);
    assert_eq!(
        rewritten.extra["parameters"],
        json!([{"name": "a", "delivery": "placeholder"}])
    );
}
