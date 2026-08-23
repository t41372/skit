use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use skit_application::form_state::{
    FormStateRepository, FormStateService, LastRunState, PersistedFormState, PresetSnapshotSource,
    StateWriteError,
};
use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterValue},
};

#[derive(Debug, Default)]
struct MemoryState {
    states: Mutex<BTreeMap<String, PersistedFormState>>,
}

#[derive(Debug, Default)]
struct NarrowReadState {
    full_loads: AtomicUsize,
    narrow_loads: AtomicUsize,
}

impl FormStateRepository for NarrowReadState {
    fn load(&self, _slug: &Slug) -> PersistedFormState {
        self.full_loads.fetch_add(1, Ordering::Relaxed);
        PersistedFormState::default()
    }

    fn last_run(&self, _slug: &Slug) -> LastRunState {
        self.narrow_loads.fetch_add(1, Ordering::Relaxed);
        LastRunState {
            at: Some("2026-08-08T00:00:00Z".to_owned()),
            exit: Some(3),
            values: None,
        }
    }

    fn update<T, F>(&self, _slug: &Slug, _update: F) -> Result<T, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> T,
    {
        panic!("the read contract must not update state")
    }

    fn try_update<T, E, F>(&self, _slug: &Slug, _update: F) -> Result<Result<T, E>, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> Result<T, E>,
    {
        panic!("the read contract must not update state")
    }

    fn forget(&self, _slug: &Slug) -> Result<(), StateWriteError> {
        panic!("the read contract must not remove state")
    }
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

    fn last_run(&self, slug: &Slug) -> LastRunState {
        self.states
            .lock()
            .unwrap()
            .get(slug.as_str())
            .map(|state| state.last_run.clone())
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

    fn try_update<T, E, F>(&self, slug: &Slug, update: F) -> Result<Result<T, E>, StateWriteError>
    where
        F: FnOnce(&mut PersistedFormState) -> Result<T, E>,
    {
        let mut states = self.states.lock().unwrap();
        let state = states.entry(slug.as_str().to_owned()).or_default();
        let before = state.clone();
        match update(state) {
            Ok(result) => Ok(Ok(result)),
            Err(error) => {
                *state = before;
                Ok(Err(error))
            }
        }
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
fn last_run_uses_the_narrow_repository_port_without_loading_full_state() {
    let service = FormStateService::new(NarrowReadState::default());

    assert_eq!(
        service.last_run(&slug()),
        LastRunState {
            at: Some("2026-08-08T00:00:00Z".to_owned()),
            exit: Some(3),
            values: None,
        }
    );
    assert_eq!(service.repository().narrow_loads.load(Ordering::Relaxed), 1);
    assert_eq!(service.repository().full_loads.load(Ordering::Relaxed), 0);
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
fn state_backed_presets_distinguish_prefill_legacy_and_missing_run_snapshots() {
    let repository = MemoryState::default();
    repository
        .update(&slug(), |state| {
            state.values = map(&[("city", "Berlin")]);
        })
        .unwrap();
    let service = FormStateService::new(repository);

    assert!(
        service
            .save_preset_from_state(
                &slug(),
                "prefill",
                &declarations(),
                PresetSnapshotSource::Prefill,
            )
            .unwrap()
    );
    assert!(
        service
            .save_preset_from_state(
                &slug(),
                "legacy",
                &declarations(),
                PresetSnapshotSource::LastRun,
            )
            .unwrap()
    );
    assert_eq!(service.load(&slug()).presets["legacy"]["city"], "Berlin");

    service
        .record_run(&slug(), 0, "2026-08-08T03:00:00Z", &declarations(), None)
        .unwrap();
    assert!(
        !service
            .save_preset_from_state(
                &slug(),
                "raw",
                &declarations(),
                PresetSnapshotSource::LastRun,
            )
            .unwrap()
    );
    assert!(!service.load(&slug()).presets.contains_key("raw"));

    service
        .record_run(
            &slug(),
            0,
            "2026-08-08T04:00:00Z",
            &declarations(),
            Some(&BTreeMap::new()),
        )
        .unwrap();
    assert!(
        service
            .save_preset_from_state(
                &slug(),
                "empty",
                &declarations(),
                PresetSnapshotSource::LastRun,
            )
            .unwrap()
    );
    assert!(service.load(&slug()).presets["empty"].is_empty());
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
                values: Some(map(&[("token", "old"), ("city", "Rome")])),
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
    assert_eq!(state.last_run.values, Some(map(&[("city", "Rome")])));
    assert_eq!(state.last_run.exit, Some(0));
}

#[test]
fn completed_run_re_resolves_once_and_commits_every_state_surface_together() {
    let repository = MemoryState::default();
    repository
        .update(&slug(), |state| {
            state.values = map(&[("token", "old"), ("city", "Berlin")]);
            state
                .presets
                .insert("legacy".to_owned(), map(&[("token", "old")]));
            state.last_run.values = Some(map(&[("token", "old")]));
        })
        .unwrap();
    let service = FormStateService::new(repository);
    let resolutions = AtomicUsize::new(0);
    let values = map(&[("city", "Paris"), ("empty", ""), ("token", "new-secret")]);

    service
        .record_completed_run_with(
            &slug(),
            3,
            "2026-08-21T01:02:03Z",
            Some(&values),
            Some(vec!["--tail".to_owned()]),
            false,
            Some("fresh"),
            || {
                resolutions.fetch_add(1, Ordering::Relaxed);
                Ok::<_, ()>(declarations())
            },
        )
        .unwrap()
        .unwrap();

    assert_eq!(resolutions.load(Ordering::Relaxed), 1);
    let state = service.load(&slug());
    assert!(state.values.is_empty());
    assert_eq!(state.extra_args, ["--tail"]);
    assert!(!state.extra_args_raw);
    assert!(!state.presets.contains_key("legacy"));
    assert_eq!(
        state.presets["fresh"],
        map(&[("city", "Paris"), ("empty", "")])
    );
    assert_eq!(state.last_run.at.as_deref(), Some("2026-08-21T01:02:03Z"));
    assert_eq!(state.last_run.exit, Some(3));
    assert_eq!(
        state.last_run.values,
        Some(map(&[("city", "Paris"), ("empty", "")]))
    );
}

#[test]
fn completed_run_resolution_failure_changes_no_state_surface() {
    let repository = MemoryState::default();
    repository
        .update(&slug(), |state| {
            state.values = map(&[("city", "Berlin")]);
            state.extra_args = vec!["--old".to_owned()];
            state.extra_args_raw = true;
            state
                .presets
                .insert("saved".to_owned(), map(&[("city", "Rome")]));
            state.last_run = LastRunState {
                at: Some("before".to_owned()),
                exit: Some(2),
                values: Some(map(&[("city", "Tokyo")])),
            };
        })
        .unwrap();
    let service = FormStateService::new(repository);
    let before = service.load(&slug());

    let result = service
        .record_completed_run_with(
            &slug(),
            0,
            "after",
            Some(&map(&[("city", "Paris")])),
            Some(vec!["--new".to_owned()]),
            false,
            Some("new"),
            || Err::<Vec<ParamDecl>, _>("source changed"),
        )
        .unwrap();

    assert_eq!(result, Err("source changed"));
    assert_eq!(service.load(&slug()), before);
}

#[test]
fn raw_completed_run_preserves_form_state_and_clears_only_the_run_snapshot() {
    let repository = MemoryState::default();
    repository
        .update(&slug(), |state| {
            state.values = map(&[("city", "Berlin")]);
            state.extra_args = vec!["--old".to_owned()];
            state.extra_args_raw = true;
            state
                .presets
                .insert("saved".to_owned(), map(&[("city", "Rome")]));
            state.last_run.values = Some(map(&[("city", "Tokyo")]));
        })
        .unwrap();
    let service = FormStateService::new(repository);

    service
        .record_completed_run_with(
            &slug(),
            7,
            "2026-08-21T02:03:04Z",
            None,
            Some(vec!["--ignored".to_owned()]),
            false,
            Some("ignored"),
            || -> Result<Vec<ParamDecl>, ()> {
                panic!("raw completion must not resolve form declarations")
            },
        )
        .unwrap()
        .unwrap();

    let state = service.load(&slug());
    assert_eq!(state.values, map(&[("city", "Berlin")]));
    assert_eq!(state.extra_args, ["--old"]);
    assert!(state.extra_args_raw);
    assert_eq!(state.presets["saved"], map(&[("city", "Rome")]));
    assert!(!state.presets.contains_key("ignored"));
    assert_eq!(state.last_run.at.as_deref(), Some("2026-08-21T02:03:04Z"));
    assert_eq!(state.last_run.exit, Some(7));
    assert_eq!(state.last_run.values, None);
}

#[test]
fn record_run_without_values_clears_the_previous_snapshot_for_raw_runs() {
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
            values: Some(map(&[("city", "Paris"), ("empty", "")])),
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
            values: None,
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
