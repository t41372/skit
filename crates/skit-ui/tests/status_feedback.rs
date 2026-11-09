//! Contract tests for the status line: every status carries the outcome it reports, and the
//! outcome sets how long the line stays.
//!
//! Ways these tests can fail: a reducer arm that sets a status keeps the old untyped field, so
//! the kind stays `Info`; a clear leaves the old kind behind; a serialized state or action grows a
//! field when nothing changed, which a version 0.5 state file or a walker fixture would reject; a
//! preset choice applies values without a receipt, or a receipt appears for a choice that changed
//! nothing.

use std::collections::BTreeMap;
use std::time::Duration;

use skit_application::LibraryScan;
use skit_application::library_detail::LibrarySurface;
use skit_domain::parameters::ParamDecl;
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode};
use skit_ui::{
    Action, AddWorkflowState, LibraryState, RunFieldRole, RunFormView, Screen, StatusKind,
    TRANSIENT_STATUS_TIME,
};

fn entry(slug: &str, name: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).unwrap(),
        name: name.to_owned(),
        kind: EntryKind::parse("command").unwrap(),
        mode: StorageMode::Copy,
        description: String::new(),
        target: None,
    }
}

fn surface() -> LibrarySurface {
    LibrarySurface {
        scan: LibraryScan {
            entries: vec![entry("alpha", "Alpha"), entry("beta", "Beta")],
            diagnostics: Vec::new(),
        },
        details: BTreeMap::new(),
    }
}

fn state() -> LibraryState {
    LibraryState::from_library_surface(surface())
}

fn preset_form() -> RunFormView {
    let presets = BTreeMap::from([(
        "triple".to_owned(),
        BTreeMap::from([("count".to_owned(), "3".to_owned())]),
    )]);
    RunFormView::from_declarations(
        "alpha",
        "Alpha",
        &[ParamDecl::new("count")],
        &BTreeMap::from([("count".to_owned(), "1".to_owned())]),
        &[],
        "",
        &presets,
        "",
    )
}

fn run_state() -> (LibraryState, usize) {
    let mut state = state();
    let form = preset_form();
    let preset = form
        .fields()
        .iter()
        .position(|field| matches!(field.role, RunFieldRole::Preset))
        .unwrap();
    state.update(Action::Present(Screen::Run(Box::new(form))));
    (state, preset)
}

#[test]
fn a_receipt_or_a_note_clears_itself_and_a_problem_stays_until_the_user_acts() {
    assert_eq!(TRANSIENT_STATUS_TIME, Duration::from_secs(4));
    assert_eq!(
        StatusKind::Success.display_time(),
        Some(TRANSIENT_STATUS_TIME)
    );
    assert_eq!(StatusKind::Info.display_time(), Some(TRANSIENT_STATUS_TIME));
    assert_eq!(StatusKind::Warning.display_time(), None);
    assert_eq!(StatusKind::Error.display_time(), None);
    assert_eq!(StatusKind::default(), StatusKind::Info);
}

#[test]
fn a_host_status_keeps_its_kind_and_a_clear_resets_it() {
    let mut state = state();
    assert_eq!(state.status(), None);
    assert_eq!(state.status_kind(), StatusKind::Info);

    state.update(Action::SetStatus {
        kind: StatusKind::Error,
        message: "failed".to_owned(),
    });
    assert_eq!(state.status(), Some("failed"));
    assert_eq!(state.status_kind(), StatusKind::Error);

    state.update(Action::SetStatus {
        kind: StatusKind::Info,
        message: "note".to_owned(),
    });
    assert_eq!(state.status_kind(), StatusKind::Info);

    state.update(Action::SetStatus {
        kind: StatusKind::Warning,
        message: "check".to_owned(),
    });
    state.update(Action::ClearStatus);
    assert_eq!(state.status(), None);
    assert_eq!(state.status_kind(), StatusKind::Info);
}

#[test]
fn a_completion_reports_the_kind_the_host_gives_it() {
    for kind in [
        StatusKind::Success,
        StatusKind::Warning,
        StatusKind::Error,
        StatusKind::Info,
    ] {
        let mut state = state();
        state.update(Action::Complete {
            surface: None,
            rerunnable: None,
            message: "done".to_owned(),
            kind,
        });
        assert_eq!(state.status(), Some("done"));
        assert_eq!(state.status_kind(), kind);
    }
}

#[test]
fn every_receipt_action_is_a_success() {
    let mut state = state();
    state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(Vec::new()),
    ))));
    state.update(Action::AddCompleted {
        surface: surface(),
        rerunnable: Vec::new(),
        slug: Slug::parse("beta").unwrap(),
        message: "Entry added".to_owned(),
        kind: StatusKind::Success,
    });
    assert_eq!(state.status(), Some("Entry added"));
    assert_eq!(state.status_kind(), StatusKind::Success);

    let (mut run, _) = run_state();
    run.update(Action::RunPresetSaved {
        name: "triple".to_owned(),
        presets: BTreeMap::new(),
        message: "Preset \"triple\" saved.".to_owned(),
    });
    assert_eq!(run.status_kind(), StatusKind::Success);

    let mut preferences = state.clone();
    preferences.update(Action::PreferencesSaved {
        locale: "en".to_owned(),
        message: "Preferences saved".to_owned(),
        theme: None,
        accent: None,
    });
    assert_eq!(preferences.status_kind(), StatusKind::Success);
}

#[test]
fn a_requested_reload_shows_its_receipt_and_a_plain_refresh_shows_nothing() {
    let mut state = state();
    state.update(Action::ReplaceSurface {
        surface: surface(),
        rerunnable: Vec::new(),
        message: None,
    });
    assert_eq!(state.status(), None);

    state.update(Action::ReplaceSurface {
        surface: surface(),
        rerunnable: Vec::new(),
        message: Some("Library reloaded (2 entries)".to_owned()),
    });
    assert_eq!(state.status(), Some("Library reloaded (2 entries)"));
    assert_eq!(state.status_kind(), StatusKind::Success);
}

#[test]
fn choosing_a_preset_applies_it_and_says_so() {
    let (mut state, preset) = run_state();
    state.update(Action::SelectFieldOption {
        field: preset,
        value: "triple".to_owned(),
    });
    assert_eq!(state.status(), Some("Preset applied."));
    assert_eq!(state.status_kind(), StatusKind::Success);

    state.update(Action::ClearStatus);
    state.update(Action::SelectFieldOption {
        field: preset,
        value: String::new(),
    });
    assert_eq!(state.status(), Some("Last values restored."));
    assert_eq!(state.status_kind(), StatusKind::Success);
}

#[test]
fn a_choice_that_is_not_a_preset_or_not_an_option_gives_no_receipt() {
    let (mut state, preset) = run_state();
    state.update(Action::SelectFieldOption {
        field: preset,
        value: "missing".to_owned(),
    });
    assert_eq!(state.status(), None);

    let count = preset + 1;
    state.update(Action::SelectFieldOption {
        field: count,
        value: "3".to_owned(),
    });
    assert_eq!(state.status(), None);
}

#[test]
fn serialization_adds_a_kind_only_when_a_status_reports_more_than_information() {
    let mut state = state();
    let quiet = serde_json::to_value(&state).unwrap();
    assert!(quiet.get("status_kind").is_none());

    state.update(Action::SetStatus {
        kind: StatusKind::Info,
        message: "note".to_owned(),
    });
    assert!(
        serde_json::to_value(&state)
            .unwrap()
            .get("status_kind")
            .is_none()
    );

    state.update(Action::SetStatus {
        kind: StatusKind::Error,
        message: "failed".to_owned(),
    });
    let encoded = serde_json::to_value(&state).unwrap();
    assert_eq!(encoded["status"], "failed");
    assert_eq!(encoded["status_kind"], "error");
    let decoded: LibraryState = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, state);

    // A version 0.5 action without a kind is still a valid, canonical information status.
    let legacy = serde_json::json!({"set_status": {"message": "status"}});
    let action: Action = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(&action).unwrap(), legacy);
    let complete = serde_json::json!({
        "complete": {"surface": null, "rerunnable": null, "message": "complete"}
    });
    let action: Action = serde_json::from_value(complete.clone()).unwrap();
    assert_eq!(serde_json::to_value(&action).unwrap(), complete);
}
