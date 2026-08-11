//! Public-API ports of Python v0.4 default/preset history regressions.
//!
//! Last-used state deliberately follows future source defaults, while a named preset deliberately
//! pins what actually ran. The exact last-run snapshot is therefore the only honest `--from-last`
//! source once the script changes.

use std::collections::BTreeMap;

use skit_application::form_state::{FormStateRepository, FormStateService, PresetSnapshotSource};
use skit_domain::{
    Slug,
    parameters::{ParamDecl, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_store::FileFormStateStore;
use tempfile::TempDir;

fn slug(value: &str) -> Slug {
    Slug::parse(value.to_owned()).unwrap()
}

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn defaulted(name: &str, default: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Str;
    declaration.default = Some(ParameterValue::String(default.to_owned()));
    declaration
}

#[test]
fn test_preset_from_last_saves_effective_values_after_an_all_defaults_run() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository.clone());
    let slug = slug("greet");
    let declarations = [defaulted("GREETING", "bonjour")];
    let accepted = values(&[("GREETING", "bonjour")]);

    service
        .save_last(
            &slug,
            &declarations,
            Some(&accepted),
            Some(Vec::new()),
            false,
        )
        .unwrap();
    service
        .record_run(
            &slug,
            0,
            "2026-07-09T14:30:05+00:00",
            &declarations,
            Some(&accepted),
        )
        .unwrap();

    let before = service.load(&slug);
    assert!(before.values.is_empty());
    assert_eq!(before.last_run.exit, Some(0));
    assert_eq!(before.last_run.values, Some(accepted.clone()));

    assert!(
        service
            .save_preset_from_state(&slug, "p", &declarations, PresetSnapshotSource::LastRun,)
            .unwrap()
    );
    assert_eq!(
        service.load(&slug).presets,
        BTreeMap::from([("p".to_owned(), accepted)])
    );
}

#[test]
fn test_preset_from_last_refuses_an_entry_that_never_ran_and_has_no_remembered_values() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository);
    let slug = slug("fresh");
    let declarations = [defaulted("GREETING", "bonjour")];

    assert!(
        !service
            .save_preset_from_state(&slug, "p", &declarations, PresetSnapshotSource::LastRun,)
            .unwrap()
    );
    assert!(service.load(&slug).presets.is_empty());
}

#[test]
fn test_preset_from_last_pins_the_default_that_actually_ran_not_todays_default() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository);
    let slug = slug("history");
    let old_declarations = [defaulted("GREETING", "A")];
    let ran = values(&[("GREETING", "A")]);

    service
        .save_last(&slug, &old_declarations, Some(&ran), None, false)
        .unwrap();
    service
        .record_run(
            &slug,
            0,
            "2026-07-09T14:30:05+00:00",
            &old_declarations,
            Some(&ran),
        )
        .unwrap();
    assert!(service.load(&slug).values.is_empty());

    let current_declarations = [defaulted("GREETING", "B")];
    assert!(
        service
            .save_preset_from_state(
                &slug,
                "p",
                &current_declarations,
                PresetSnapshotSource::LastRun,
            )
            .unwrap()
    );
    assert_eq!(
        service.load(&slug).presets.get("p"),
        Some(&values(&[("GREETING", "A")]))
    );
}

#[test]
fn test_preset_from_legacy_run_without_a_snapshot_refuses_to_guess() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository.clone());
    let slug = slug("legacy-history");
    let declarations = [defaulted("GREETING", "B")];

    repository
        .update(&slug, |state| {
            state.last_run.at = Some("2026-07-09T14:30:05+00:00".to_owned());
            state.last_run.exit = Some(0);
            state.last_run.values = None;
        })
        .unwrap();

    assert!(
        !service
            .save_preset_from_state(&slug, "p", &declarations, PresetSnapshotSource::LastRun,)
            .unwrap()
    );
    assert!(service.load(&slug).presets.is_empty());
}

#[test]
fn test_named_preset_pins_a_default_value_while_last_used_filters_it_out() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository);
    let slug = slug("pinned");
    let declarations = [defaulted("GREETING", "bonjour")];
    let accepted = values(&[("GREETING", "bonjour")]);

    service
        .save_last(&slug, &declarations, Some(&accepted), None, false)
        .unwrap();
    service
        .save_preset(&slug, "p", &declarations, &accepted)
        .unwrap();

    let state = service.load(&slug);
    assert!(state.values.is_empty());
    assert_eq!(state.presets, BTreeMap::from([("p".to_owned(), accepted)]));
}

#[test]
fn test_last_used_keeps_a_delivered_empty_string_but_drops_an_unset_typed_empty() {
    let root = TempDir::new().unwrap();
    let repository = FileFormStateStore::new(root.path());
    let service = FormStateService::new(repository);
    let slug = slug("empty-history");

    let mut greeting = defaulted("GREETING", "bonjour");
    greeting.delivery = ParameterDelivery::Inject;
    let mut width = ParamDecl::new("WIDTH");
    width.delivery = ParameterDelivery::Inject;
    width.parameter_type = ParameterType::Int;
    width.default = Some(ParameterValue::Integer(800));
    let declarations = [greeting, width];
    let submitted = values(&[("GREETING", ""), ("WIDTH", "")]);

    service
        .save_last(&slug, &declarations, Some(&submitted), None, false)
        .unwrap();

    assert_eq!(service.load(&slug).values, values(&[("GREETING", "")]));
}
