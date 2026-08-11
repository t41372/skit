//! Exact application/storage ports of Python v0.4 `tests/test_presets.py`.
//!
//! The tests assert both typed state and raw persisted bytes. Secret stripping is structural: a
//! value that disappears only from a read projection but remains in TOML still fails this oracle.

use std::{collections::BTreeMap, fs};

use skit_application::form_state::{FormStateRepository as _, FormStateService, prefill};
use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug() -> Slug {
    Slug::parse("s").unwrap()
}

fn values(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn spec(name: &str, default: Option<&str>, secret: bool) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = default.map(|value| ParameterValue::String(value.to_owned()));
    declaration.secret = secret;
    declaration
}

fn public(name: &str) -> ParamDecl {
    spec(name, None, false)
}

fn secret(name: &str) -> ParamDecl {
    spec(name, None, true)
}

fn state_file(root: &TempDir) -> std::path::PathBuf {
    root.path().join("values/s.toml")
}

#[test]
fn test_preset_roundtrip() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let declarations = [public("CITY")];

    service
        .save_preset(&slug(), "prod", &declarations, &values(&[("CITY", "Taipei")]))
        .unwrap();
    assert_eq!(
        service.load(&slug()).presets.get("prod"),
        Some(&values(&[("CITY", "Taipei")]))
    );
    assert!(service.delete_preset(&slug(), "prod").unwrap());
    assert!(service.load(&slug()).presets.is_empty());
    assert!(!service.delete_preset(&slug(), "nope").unwrap());
}

#[test]
fn test_resolution_order_preset_over_last_over_default() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let declarations = [spec("CITY", Some("Osaka"), false), spec("N", Some("1"), false)];

    assert_eq!(
        prefill(&declarations, &BTreeMap::new(), None),
        values(&[("CITY", "Osaka"), ("N", "1")])
    );
    service
        .save_last(
            &slug(),
            &declarations,
            Some(&values(&[("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    let last = service.load(&slug());
    assert_eq!(
        prefill(&declarations, &last.values, None).get("CITY").map(String::as_str),
        Some("Taipei")
    );

    service
        .save_preset(
            &slug(),
            "jp",
            &declarations,
            &values(&[("CITY", "Kyoto")]),
        )
        .unwrap();
    let state = service.load(&slug());
    assert_eq!(
        prefill(&declarations, &state.values, state.presets.get("jp"))
            .get("CITY")
            .map(String::as_str),
        Some("Kyoto")
    );

    // Seed a stale key through the repository's own transaction surface: prefill must still be
    // declaration-driven and never leak it into the form.
    service
        .repository()
        .update(&slug(), |state| {
            state.values.insert("STALE".to_owned(), "x".to_owned());
        })
        .unwrap();
    let state = service.load(&slug());
    assert!(!prefill(&declarations, &state.values, None).contains_key("STALE"));
}

#[test]
fn test_c3_secret_never_touches_disk() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let declarations = [secret("API_KEY"), public("CITY")];
    let submitted = values(&[("API_KEY", "hunter2"), ("CITY", "Taipei")]);

    service
        .save_last(&slug(), &declarations, Some(&submitted), None, false)
        .unwrap();
    service
        .save_preset(&slug(), "prod", &declarations, &submitted)
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert!(!state.presets["prod"].contains_key("API_KEY"));
    assert_eq!(state.values.get("CITY").map(String::as_str), Some("Taipei"));
    assert_eq!(state.presets["prod"].get("CITY").map(String::as_str), Some("Taipei"));
    assert!(!fs::read_to_string(state_file(&root)).unwrap().contains("hunter2"));
}

#[test]
fn test_preset_preserved_across_save_last() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let declarations = [public("CITY")];
    service
        .save_preset(&slug(), "prod", &declarations, &values(&[("CITY", "Taipei")]))
        .unwrap();
    service
        .save_last(
            &slug(),
            &declarations,
            Some(&values(&[("CITY", "Tainan")])),
            Some(vec!["-v".to_owned()]),
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert_eq!(state.presets["prod"], values(&[("CITY", "Taipei")]));
    assert_eq!(state.values.get("CITY").map(String::as_str), Some("Tainan"));
    assert_eq!(state.extra_args, ["-v"]);
}

#[test]
fn test_purge_secret_removes_from_values_and_every_preset() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let before = [public("API_KEY"), public("CITY")];
    service
        .save_last(
            &slug(),
            &before,
            Some(&values(&[("API_KEY", "shown"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_preset(
            &slug(),
            "prod",
            &before,
            &values(&[("API_KEY", "shown"), ("CITY", "Taipei")]),
        )
        .unwrap();
    service
        .save_preset(&slug(), "dev", &before, &values(&[("API_KEY", "shown")]))
        .unwrap();

    let removed = service
        .purge_secrets(&slug(), &[secret("API_KEY"), public("CITY")])
        .unwrap();
    assert_eq!(removed.into_iter().collect::<Vec<_>>(), ["API_KEY"]);
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert_eq!(state.values.get("CITY").map(String::as_str), Some("Taipei"));
    assert!(!state.presets["prod"].contains_key("API_KEY"));
    assert_eq!(state.presets["prod"].get("CITY").map(String::as_str), Some("Taipei"));
    assert!(!state.presets.contains_key("dev"));
    assert!(!fs::read_to_string(state_file(&root)).unwrap().contains("shown"));
}

#[test]
fn test_purge_secret_drops_a_preset_left_empty_but_keeps_others() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let before = [public("API_KEY"), public("CITY")];
    service
        .save_preset(
            &slug(),
            "onlysecret",
            &before,
            &values(&[("API_KEY", "shown")]),
        )
        .unwrap();
    service
        .save_preset(
            &slug(),
            "mixed",
            &before,
            &values(&[("API_KEY", "shown"), ("CITY", "Taipei")]),
        )
        .unwrap();
    service
        .purge_secrets(&slug(), &[secret("API_KEY"), public("CITY")])
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.presets.contains_key("onlysecret"));
    assert_eq!(state.presets["mixed"], values(&[("CITY", "Taipei")]));
    let raw = fs::read_to_string(state_file(&root)).unwrap();
    assert!(!raw.contains("onlysecret"));
    assert!(!raw.contains("shown"));
}

#[test]
fn test_purge_secret_empty_names_is_noop() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    service
        .save_last(
            &slug(),
            &[public("CITY")],
            Some(&values(&[("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    let path = state_file(&root);
    let before = fs::read(&path).unwrap();
    assert!(service.purge_secrets(&slug(), &[]).unwrap().is_empty());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn test_purge_secret_reports_only_names_actually_stored() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    service
        .save_last(
            &slug(),
            &[public("CITY")],
            Some(&values(&[("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    let removed = service
        .purge_secrets(&slug(), &[secret("API_KEY"), secret("CITY")])
        .unwrap();
    assert_eq!(removed.into_iter().collect::<Vec<_>>(), ["CITY"]);
}

#[test]
fn test_save_last_drops_stale_value_once_param_becomes_secret() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    service
        .save_last(
            &slug(),
            &[public("API_KEY")],
            Some(&values(&[("API_KEY", "old-secret")])),
            None,
            false,
        )
        .unwrap();
    assert_eq!(
        service.load(&slug()).values.get("API_KEY").map(String::as_str),
        Some("old-secret")
    );
    service
        .save_last(
            &slug(),
            &[secret("API_KEY")],
            Some(&values(&[("API_KEY", "new-typed")])),
            None,
            false,
        )
        .unwrap();
    assert!(!service.load(&slug()).values.contains_key("API_KEY"));
    let raw = fs::read_to_string(state_file(&root)).unwrap();
    assert!(!raw.contains("old-secret"));
    assert!(!raw.contains("new-typed"));
}

#[test]
fn test_save_last_values_are_a_snapshot_not_a_merge() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    service
        .save_last(
            &slug(),
            &[public("API_KEY"), public("CITY")],
            Some(&values(&[("API_KEY", "old-secret"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_last(
            &slug(),
            &[secret("API_KEY"), public("CITY")],
            Some(&values(&[("API_KEY", "x")])),
            None,
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert!(!state.values.contains_key("CITY"));
}

#[test]
fn test_save_last_none_values_still_scrubs_stale_secret() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    service
        .save_last(
            &slug(),
            &[public("API_KEY"), public("CITY")],
            Some(&values(&[("API_KEY", "old-secret"), ("CITY", "Taipei")])),
            None,
            false,
        )
        .unwrap();
    service
        .save_last(
            &slug(),
            &[secret("API_KEY"), public("CITY")],
            None,
            None,
            false,
        )
        .unwrap();
    let state = service.load(&slug());
    assert!(!state.values.contains_key("API_KEY"));
    assert_eq!(state.values.get("CITY").map(String::as_str), Some("Taipei"));
}

#[test]
fn test_save_last_regression_non_secret_values_persist_normally() {
    let root = TempDir::new().unwrap();
    let service = FormStateService::new(FileFormStateStore::new(root.path()));
    let submitted = values(&[("CITY", "Taipei"), ("N", "3")]);
    service
        .save_last(
            &slug(),
            &[public("CITY"), public("N")],
            Some(&submitted),
            None,
            false,
        )
        .unwrap();
    assert_eq!(service.load(&slug()).values, submitted);
}
