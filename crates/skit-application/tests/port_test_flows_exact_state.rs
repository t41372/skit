//! Exact persistence-policy ports from Python v0.4 `tests/test_flows.py`.

use std::{collections::BTreeMap, sync::Mutex};

use skit_application::form_state::{
    FormStateRepository, FormStateService, LastRunState, PersistedFormState, StateWriteError,
};
use skit_domain::{Slug, parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue}};

#[derive(Debug, Default)]
struct MemoryState {
    states: Mutex<BTreeMap<String, PersistedFormState>>,
}
impl FormStateRepository for MemoryState {
    fn load(&self, slug: &Slug) -> PersistedFormState {
        self.states.lock().unwrap().get(slug.as_str()).cloned().unwrap_or_default()
    }
    fn last_run(&self, slug: &Slug) -> LastRunState { self.load(slug).last_run }
    fn update<T, F>(&self, slug: &Slug, update: F) -> Result<T, StateWriteError>
    where F: FnOnce(&mut PersistedFormState) -> T {
        let mut states = self.states.lock().unwrap();
        Ok(update(states.entry(slug.as_str().to_owned()).or_default()))
    }
    fn forget(&self, slug: &Slug) -> Result<(), StateWriteError> {
        self.states.lock().unwrap().remove(slug.as_str());
        Ok(())
    }
}
fn slug(value: &str) -> Slug { Slug::parse(value).unwrap() }
fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
}
fn managed() -> Vec<ParamDecl> {
    let mut output = ParamDecl::new("OUTPUT");
    output.binding = ParameterBinding::Const;
    output.delivery = ParameterDelivery::Inject;
    output.default = Some(ParameterValue::String("out.jpg".to_owned()));
    let mut width = ParamDecl::new("WIDTH");
    width.binding = ParameterBinding::Const;
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    let mut secret = ParamDecl::new("API_KEY");
    secret.binding = ParameterBinding::Const;
    secret.delivery = ParameterDelivery::Inject;
    secret.secret = true;
    secret.default = Some(ParameterValue::String("xxx".to_owned()));
    vec![output, width, secret]
}

#[test]
fn test_save_after_run_persists_intent_and_stamps_run() {
    let service = FormStateService::new(MemoryState::default());
    let slug = slug("s");
    let raw = map(&[("OUTPUT", "long_{today}.jpg"), ("WIDTH", "800"), ("API_KEY", "secret!")]);
    service.purge_secrets(&slug, &managed()).unwrap();
    service.save_last(&slug, &managed(), Some(&raw), Some(vec!["--fast".to_owned()]), false).unwrap();
    service.record_run(&slug, 0, "2026-07-09T14:30:05+00:00", &managed(), Some(&raw)).unwrap();

    let state = service.load(&slug);
    assert_eq!(state.values.get("OUTPUT").map(String::as_str), Some("long_{today}.jpg"));
    assert!(!state.values.contains_key("API_KEY"));
    assert_eq!(state.extra_args, ["--fast"]);
    assert_eq!(state.last_run, LastRunState {
        at: Some("2026-07-09T14:30:05+00:00".to_owned()),
        exit: Some(0),
        values: Some(map(&[("OUTPUT", "long_{today}.jpg"), ("WIDTH", "800")])),
    });
}

#[test]
fn test_record_run_zero_exit_survives_save() {
    let service = FormStateService::new(MemoryState::default());
    let slug = slug("z");
    service.record_run(&slug, 0, "2026-07-09T00:00:00+00:00", &[], None).unwrap();
    assert_eq!(service.load(&slug).last_run.exit, Some(0));
}

#[test]
fn test_save_after_run_clears_cleared_extra_args() {
    let service = FormStateService::new(MemoryState::default());
    let slug = slug("clr");
    let raw = map(&[("OUTPUT", "a")]);
    service.save_last(&slug, &managed(), Some(&raw), Some(vec!["--fast".to_owned()]), false).unwrap();
    service.record_run(&slug, 0, "2026-01-01T00:00:00+00:00", &managed(), Some(&raw)).unwrap();
    assert_eq!(service.load(&slug).extra_args, ["--fast"]);

    service.save_last(&slug, &managed(), Some(&raw), Some(Vec::new()), false).unwrap();
    service.record_run(&slug, 0, "2026-01-01T00:00:01+00:00", &managed(), Some(&raw)).unwrap();
    assert!(service.load(&slug).extra_args.is_empty());
}

#[test]
fn test_save_after_run_purges_secret_placeholder_from_presets() {
    let mut api = ParamDecl::new("api_key");
    api.delivery = ParameterDelivery::Placeholder;
    api.secret = true;
    api.required = true;
    let declarations = vec![api];
    let slug = slug("c3");
    let repository = MemoryState::default();
    repository.update(&slug, |state| {
        state.values = map(&[("api_key", "sk-123")]);
        state.presets.insert("old".to_owned(), map(&[("api_key", "sk-123")]));
    }).unwrap();
    let service = FormStateService::new(repository);

    service.purge_secrets(&slug, &declarations).unwrap();
    let raw = map(&[("api_key", "sk-456")]);
    service.save_last(&slug, &declarations, Some(&raw), Some(Vec::new()), false).unwrap();
    service.record_run(&slug, 0, "2026-01-01T00:00:00+00:00", &declarations, Some(&raw)).unwrap();

    let state = service.load(&slug);
    assert!(!state.values.contains_key("api_key"));
    assert!(state.presets.values().all(|preset| !preset.contains_key("api_key")));
    assert_eq!(state.last_run.values, Some(BTreeMap::new()));
}
