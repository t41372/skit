use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};

use skit_application::form_state::{
    FormStateRepository, FormStateService, LastRunState, PersistedFormState, StateWriteError,
};
use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterValue},
};

#[derive(Debug, Default)]
struct MemoryState {
    states: Mutex<BTreeMap<String, PersistedFormState>>,
}

impl FormStateRepository for MemoryState {
    fn load(&self, slug: &Slug) -> PersistedFormState {
        self.states
            .lock()
            .unwrap()
            .get(slug.as_str())
            .cloned()
            .unwrap_or_default()
    }

    fn update<T, F>(&self, slug: &Slug, update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        let mut states = self.states.lock().unwrap();
        let state = states.entry(slug.as_str().to_owned()).or_default();
        Ok(update(state))
    }

    fn forget(&self, slug: &Slug) -> Result<(), StateWriteError> {
        self.states.lock().unwrap().remove(slug.as_str());
        Ok(())
    }
}

fn slug() -> Slug {
    Slug::parse("demo").unwrap()
}

fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn declarations() -> Vec<ParamDecl> {
    let mut city = ParamDecl::new("city");
    city.default = Some(ParameterValue::String("Paris".to_owned()));
    let empty = ParamDecl::new("empty");
    let mut token = ParamDecl::new("token");
    token.secret = true;
    vec![city, empty, token]
}

#[test]
fn save_last_distinguishes_no_update_from_explicit_clear_and_scrubs_current_secrets() {
    let repository = MemoryState::default();
    repository
        .update(&slug(), |state| {
            state.values = map(&[("city", "Berlin"), ("token", "old-secret")]);
            state.extra_args = vec!["{today}".to_owned()];
            state.extra_args_raw = true;
        })
        .unwrap();
    let service = FormStateService::new(repository);

    service
        .save_last(&slug(), &declarations(), None, None, false)
        .unwrap();
    let kept = service.load(&slug());
    assert_eq!(kept.values, map(&[("city", "Berlin")]));
    assert_eq!(kept.extra_args, ["{today}"]);
    assert!(kept.extra_args_raw);

    service
        .save_last(
            &slug(),
            &declarations(),
            Some(&BTreeMap::new()),
            Some(Vec::new()),
            true,
        )
        .unwrap();
    let cleared = service.load(&slug());
    assert!(cleared.values.is_empty());
    assert!(cleared.extra_args.is_empty());
    assert!(!cleared.extra_args_raw);
}

#[test]
fn save_last_remembers_only_nondefault_nonsecret_intent() {
    let service = FormStateService::new(MemoryState::default());
    service
        .save_last(
            &slug(),
            &declarations(),
            Some(&map(&[
                ("city", "Paris"),
                ("empty", "value"),
                ("token", "secret"),
            ])),
            Some(vec!["--literal".to_owned()]),
            false,
        )
        .unwrap();

    let state = service.load(&slug());
    assert_eq!(state.values, map(&[("empty", "value")]));
    assert_eq!(state.extra_args, ["--literal"]);
    assert!(!state.extra_args_raw);
}

#[test]
fn presets_pin_exact_public_values_and_delete_reports_presence() {
    let service = FormStateService::new(MemoryState::default());
    service
        .save_preset(
            &slug(),
            "travel",
            &declarations(),
            &map(&[("city", "Paris"), ("empty", ""), ("token", "secret")]),
        )
        .unwrap();

    let state = service.load(&slug());
    assert_eq!(
        state.presets["travel"],
        map(&[("city", "Paris"), ("empty", "")])
    );
    assert!(service.delete_preset(&slug(), "travel").unwrap());
    assert!(!service.delete_preset(&slug(), "travel").unwrap());
}

#[test]
fn purge_secrets_scrubs_values_presets_and_last_run_in_one_transaction() {
    let repository = MemoryState::default();
    repository
        .update(&slug(), |state| {
            state.values = map(&[("token", "old"), ("city", "Berlin")]);
            state.presets.insert(
                "mixed".to_owned(),
                map(&[("token", "old"), ("city", "Tokyo")]),
            );
            state
                .presets
                .insert("secret-only".to_owned(), map(&[("token", "old")]));
            state.last_run = LastRunState {
                at: Some("2026-08-08T01:00:00Z".to_owned()),
                exit: Some(0),
                values: map(&[("token", "old"), ("city", "Rome")]),
            };
        })
        .unwrap();
    let service = FormStateService::new(repository);

    assert_eq!(
        service.purge_secrets(&slug(), &declarations()).unwrap(),
        BTreeSet::from(["token".to_owned()])
    );
    let state = service.load(&slug());
    assert_eq!(state.values, map(&[("city", "Berlin")]));
    assert_eq!(state.presets["mixed"], map(&[("city", "Tokyo")]));
    assert!(!state.presets.contains_key("secret-only"));
    assert_eq!(state.last_run.values, map(&[("city", "Rome")]));
    assert_eq!(state.last_run.exit, Some(0));
}

#[test]
fn record_run_replaces_stamp_but_none_preserves_the_previous_value_snapshot() {
    let service = FormStateService::new(MemoryState::default());
    service
        .record_run(
            &slug(),
            7,
            "2026-08-08T01:00:00Z",
            &declarations(),
            Some(&map(&[
                ("city", "Paris"),
                ("empty", ""),
                ("token", "secret"),
            ])),
        )
        .unwrap();
    assert_eq!(
        service.load(&slug()).last_run,
        LastRunState {
            at: Some("2026-08-08T01:00:00Z".to_owned()),
            exit: Some(7),
            values: map(&[("city", "Paris"), ("empty", "")]),
        }
    );

    service
        .record_run(&slug(), 0, "2026-08-08T02:00:00Z", &declarations(), None)
        .unwrap();
    assert_eq!(
        service.load(&slug()).last_run,
        LastRunState {
            at: Some("2026-08-08T02:00:00Z".to_owned()),
            exit: Some(0),
            values: map(&[("city", "Paris"), ("empty", "")]),
        }
    );
}

#[test]
fn forget_removes_all_per_entry_state() {
    let service = FormStateService::new(MemoryState::default());
    service
        .save_preset(&slug(), "one", &declarations(), &map(&[("city", "Tokyo")]))
        .unwrap();
    service.forget(&slug()).unwrap();
    assert_eq!(service.load(&slug()), PersistedFormState::default());
    assert!(service.repository().states.lock().unwrap().is_empty());
}
