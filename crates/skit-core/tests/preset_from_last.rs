use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use skit_core::{
    Delivery, FormField, FormPlan, LibraryRoots, ParamDecl, PlanSource, PresetFromLastError,
    StateStore, save_preset_from_last,
};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn plan() -> FormPlan {
    FormPlan {
        source: PlanSource::Declared,
        fields: vec![
            FormField::from_decl(&ParamDecl {
                name: "CITY".to_owned(),
                delivery: Delivery::Env,
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "API_KEY".to_owned(),
                delivery: Delivery::Env,
                secret: true,
                ..ParamDecl::default()
            }),
        ],
        ..FormPlan::default()
    }
}

#[test]
fn from_last_uses_exact_run_snapshot_filters_removed_fields_and_scrubs_secrets()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let path = state.values_path("demo");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &path,
        r#"[values]
CITY = "old-last-used"
API_KEY = "old-plaintext"

[last_run]
at = "2026-08-08T05:00:00+00:00"
exit = 0

[last_run.values]
CITY = "Taipei"
REMOVED = "gone"
API_KEY = "run-secret"
"#,
    )?;

    let saved = save_preset_from_last(&state, "demo", "prod", &plan())?;
    assert_eq!(
        saved,
        BTreeMap::from([("CITY".to_owned(), "Taipei".to_owned())])
    );

    let loaded = state.load("demo");
    assert_eq!(loaded.presets["prod"], saved);
    assert!(!loaded.presets["prod"].contains_key("REMOVED"));
    assert!(!loaded.values.contains_key("API_KEY"));
    assert!(
        loaded
            .last_run
            .as_ref()
            .is_some_and(|run| run.values_recorded && !run.values.contains_key("API_KEY"))
    );
    assert!(
        loaded
            .last_run
            .as_ref()
            .is_some_and(|run| run.values.get("REMOVED").is_some_and(|value| value == "gone"))
    );
    let text = fs::read_to_string(path)?;
    assert!(!text.contains("old-plaintext"));
    assert!(!text.contains("run-secret"));
    Ok(())
}

#[test]
fn exact_empty_run_snapshot_survives_roundtrip_and_can_save_an_empty_preset()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let empty = BTreeMap::new();
    state.record_run(
        "demo",
        0,
        "2026-08-08T05:00:00+00:00",
        Some(&empty),
        &Default::default(),
    )?;

    let loaded = state.load("demo");
    assert!(
        loaded
            .last_run
            .as_ref()
            .is_some_and(|run| run.values_recorded && run.values.is_empty())
    );
    let saved = save_preset_from_last(&state, "demo", "empty", &plan())?;
    assert!(saved.is_empty());
    assert!(state.load("demo").presets.contains_key("empty"));
    Ok(())
}

#[test]
fn legacy_last_used_values_are_the_narrow_fallback_when_no_run_stamp_exists()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let values = BTreeMap::from([("CITY".to_owned(), "Tainan".to_owned())]);
    state.save_last("demo", Some(&values), None, &Default::default())?;

    let saved = save_preset_from_last(&state, "demo", "legacy", &plan())?;
    assert_eq!(saved, values);
    assert_eq!(state.load("demo").presets["legacy"], values);
    Ok(())
}

#[test]
fn legacy_run_stamp_without_snapshot_refuses_even_if_last_used_values_exist()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let state = StateStore::new(roots(root.path()));
    let path = state.values_path("demo");
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &path,
        r#"[values]
CITY = "last-used-is-not-the-exact-run"

[last_run]
at = "2026-07-01T00:00:00+00:00"
exit = 0
"#,
    )?;

    let loaded = state.load("demo");
    assert!(
        loaded
            .last_run
            .as_ref()
            .is_some_and(|run| !run.values_recorded)
    );
    let result = save_preset_from_last(&state, "demo", "bad-history", &plan());
    assert!(matches!(
        result,
        Err(PresetFromLastError::NoRememberedValues)
    ));
    assert!(state.load("demo").presets.is_empty());
    Ok(())
}

#[test]
fn no_fields_and_never_run_are_named_refusals() {
    let Ok(root) = tempdir() else {
        panic!("temporary directory creation failed");
    };
    let state = StateStore::new(roots(root.path()));
    let no_fields = save_preset_from_last(&state, "demo", "x", &FormPlan::default());
    assert!(matches!(no_fields, Err(PresetFromLastError::NoFields)));

    let never_run = save_preset_from_last(&state, "demo", "x", &plan());
    assert!(matches!(
        never_run,
        Err(PresetFromLastError::NoRememberedValues)
    ));
}
