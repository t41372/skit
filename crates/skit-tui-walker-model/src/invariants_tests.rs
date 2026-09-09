use std::{collections::BTreeMap, path::PathBuf};

use skit_application::{
    AgentScope, AgentTarget, LibraryScan, SourceIdentity, SourcePermissions,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, MirrorConfiguration,
        PreferencesDraft, PreferencesSnapshot,
    },
    tokens::TokenContext,
};
use skit_domain::{EntryKind, EntrySummary, Slug, StorageMode, parameters::ParamDecl};
use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, Effect, FieldValue,
    HealthIssue, HealthIssueKind, HealthSnapshot, HealthView, KnownEntryKind, LibraryEntryDetail,
    LibraryState, LibrarySurface, MirrorHealth, ModalState, NAME_KEY, PreferencesAction,
    PreferencesView, RunFormContext, RunFormView, RunPathContext, RunnerEditorAction,
    RunnerEditorOwner, RunnerEditorView, RunnerManagerAction, RunnerManagerView, RunnerRow,
    RunnerRowIdentity, Screen, SettingsAction, SettingsInputs, SettingsView, SourceSnapshot,
    UvHealth,
};

use crate::invariants::check_state;

fn summary(slug: &str, name: &str, kind: &str, description: &str) -> EntrySummary {
    EntrySummary {
        slug: Slug::parse(slug).expect("fixture slug is valid"),
        name: name.to_owned(),
        kind: EntryKind::parse(kind).expect("fixture kind is valid"),
        mode: StorageMode::Copy,
        description: description.to_owned(),
        target: None,
    }
}

fn preserved_detail(added_at: &str) -> LibraryEntryDetail {
    LibraryEntryDetail {
        added_at: added_at.to_owned(),
        original_file_preserved: true,
        ..LibraryEntryDetail::default()
    }
}

/// Two entries. Each one keeps its original file, which the remove confirmation reports.
/// Two rows are also what a `selected = 1` move and a `visible = [1, 0]` order need.
fn library_surface() -> LibrarySurface {
    let entries = vec![
        (
            summary(
                "python-tool",
                "Python tool",
                "python",
                "A parameter fixture",
            ),
            preserved_detail("2026-08-20T12:00:00Z"),
        ),
        (
            summary("prompt-tool", "Prompt tool", "prompt", "A prompt fixture"),
            preserved_detail("2026-08-19T12:00:00Z"),
        ),
    ];
    LibrarySurface {
        scan: LibraryScan {
            entries: entries.iter().map(|(summary, _)| summary.clone()).collect(),
            diagnostics: Vec::new(),
        },
        details: entries
            .into_iter()
            .map(|(summary, detail)| (summary.slug, detail))
            .collect(),
    }
}

/// The inspected source of the Add Kind stage. It carries plain bytes and no shebang.
fn inspected_source() -> SourceSnapshot {
    SourceSnapshot {
        path: PathBuf::from("notes.bin"),
        source_record: "notes.bin".to_owned(),
        bytes: b"plain bytes\n".to_vec(),
        permissions: SourcePermissions {
            readonly: false,
            unix_mode: Some(0o700),
        },
        executable: Some(false),
        is_regular: true,
        is_directory: false,
        is_draft: true,
        identity: Some(SourceIdentity::unix(7, 11, 1_776_981_600, 0)),
    }
}

/// Preferences with the mirror off. The mirror keeps the index URL controls hidden.
fn preferences_snapshot() -> PreferencesSnapshot {
    PreferencesSnapshot {
        language: String::new(),
        available_languages: vec!["en".to_owned(), "zh-CN".to_owned(), "zh-TW".to_owned()],
        effective_language: "en".to_owned(),
        editor: String::new(),
        editor_fallback: Some("vi".to_owned()),
        form: InteractiveFormChoice::Tui,
        after_run: AfterRunChoice::Stay,
        javascript: JavascriptChoice::Automatic,
        bash_path: None,
        runner_names: vec!["codex".to_owned()],
        mirror: MirrorConfiguration::default(),
    }
}

fn agent_targets() -> Vec<AgentTarget> {
    vec![AgentTarget {
        name: "codex".to_owned(),
        scope: AgentScope::Project,
        base: PathBuf::from("/fixtures/project/.codex"),
    }]
}

fn draft() -> DraftSummary {
    DraftSummary {
        path: PathBuf::from("/model/drafts/kept.py"),
        modified: 7,
        identity: None,
        permissions: Default::default(),
        content_hash: Some("draft-hash".to_owned()),
    }
}

fn add_delete_confirmation() -> LibraryState {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(vec![draft()]),
    ))));
    let _ = state.update(Action::Add(AddAction::HighlightDraft(0)));
    let _ = state.update(Action::Add(AddAction::DeleteSelectedDraft));
    assert!(matches!(
        state.screen(),
        Screen::Add(add) if add.stage() == AddStage::ConfirmDraftDelete
    ));
    state
}

fn run_state(selector: &str) -> LibraryState {
    let mut state = LibraryState::default();
    let form = RunFormView::from_declarations(
        selector,
        "Model run",
        &[ParamDecl::new("value")],
        &BTreeMap::new(),
        &["codex".to_owned()],
        "codex",
        &BTreeMap::new(),
        "",
    )
    .with_context(RunFormContext {
        entry_kind: "prompt".to_owned(),
        path: Some(RunPathContext {
            workdir: "/model/work".to_owned(),
            invoke_cwd: "/model/invoke".to_owned(),
        }),
        tokens: TokenContext {
            cwd: "/model/invoke".to_owned(),
            home: Some("/model/home".to_owned()),
            env: BTreeMap::from([("MODEL_VALUE".to_owned(), "one".to_owned())]),
            today: "2026-08-27".to_owned(),
            now: "12-00-00".to_owned(),
        },
    });
    let _ = state.update(Action::Present(Screen::Run(Box::new(form))));
    state
}

fn runner(name: &str) -> RunnerRow {
    let identity = RunnerRowIdentity {
        index: Some(0),
        snapshot_token: "runner-snapshot".to_owned(),
    };
    RunnerRow {
        identity: identity.clone(),
        name: Some(name.to_owned()),
        argv: Some(vec!["agent".to_owned(), "{{prompt}}".to_owned()]),
        reason: None,
        descriptor: name.to_owned(),
        key_identities: vec![identity],
        pinned_count: 0,
    }
}

#[test]
fn accepts_the_in_flight_draft_delete_before_the_host_answers() {
    let mut state = add_delete_confirmation();
    let effect = state.update(Action::Add(AddAction::ConfirmDraftDelete(true)));
    assert!(matches!(effect, Effect::Add(_)));
    check_state(&state).unwrap();
}

#[test]
fn rejects_the_e29db41_draft_subject_regression_shape() {
    let state = add_delete_confirmation();
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["delete_candidate"] = serde_json::Value::Null;
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    let error = check_state(&invalid).unwrap_err();
    assert!(error.contains("draft delete confirmation has no candidate"));
}

#[test]
fn rejects_a_run_modal_that_names_a_missing_field() {
    let mut state = run_state("prompt");
    let _ = state.update(Action::OpenRunTokenMenuFor(1));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunTokenMenu { .. })
    ));
    let mut value = serde_json::to_value(state).unwrap();
    value["modal"]["run_token_menu"]["field"] = serde_json::json!(999);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    let error = check_state(&invalid).unwrap_err();
    assert!(error.contains("run modal field 999 is out of bounds"));
}

#[test]
fn rejects_a_runner_editor_owned_by_a_different_run_form() {
    let mut state = run_state("prompt");
    let _ = state.update(Action::OpenRunRunnerEditor);
    let mut value = serde_json::to_value(state).unwrap();
    value["modal"]["runner_editor"]["owner"]["run"]["selector"] = serde_json::json!("other");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    let error = check_state(&invalid).unwrap_err();
    assert!(error.contains("runner editor owner does not match the run form"));
}

#[test]
fn rejects_nested_management_indices_that_name_no_row() {
    let mut runners = LibraryState::default();
    let _ = runners.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![runner("codex")]),
    ))));
    let mut runner_value = serde_json::to_value(runners).unwrap();
    runner_value["workflow"]["active"]["runners"]["selected"] = serde_json::json!(8);
    let invalid_runners: LibraryState = serde_json::from_value(runner_value).unwrap();
    assert!(
        check_state(&invalid_runners)
            .unwrap_err()
            .contains("runner selection 8 is out of bounds")
    );

    let mut health = LibraryState::default();
    let _ = health.update(Action::Present(Screen::Health(Box::new(HealthView::new(
        HealthSnapshot {
            uv: UvHealth::NotRequired,
            entry_count: 1,
            issues: vec![HealthIssue {
                slug: "missing".to_owned(),
                name: "Missing".to_owned(),
                kind: HealthIssueKind::MissingTarget,
            }],
            invalid_runner_rows: Vec::new(),
            mirror: MirrorHealth::Off,
            library_path: "/model/library".to_owned(),
            library_size: "1 KiB".to_owned(),
            diagnostics: Vec::new(),
        },
    )))));
    let mut health_value = serde_json::to_value(health).unwrap();
    health_value["workflow"]["active"]["health"]["selected_issue"] = serde_json::json!(3);
    let invalid_health: LibraryState = serde_json::from_value(health_value).unwrap();
    assert!(
        check_state(&invalid_health)
            .unwrap_err()
            .contains("health selection 3 is out of bounds")
    );
}

#[test]
fn rejects_a_discard_guard_for_clean_settings() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Settings(Box::new(
        SettingsView::from_inputs(&SettingsInputs {
            selector: "python-tool".to_owned(),
            kind: "python".to_owned(),
            name: "Python tool".to_owned(),
            ..SettingsInputs::default()
        }),
    ))));
    let mut value = serde_json::to_value(state).unwrap();
    value["modal"] = serde_json::json!("confirm_discard_changes");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("clean settings")
    );
}

#[test]
fn rejects_stale_run_modal_subjects() {
    let mut preset = run_state("prompt");
    let _ = preset.update(Action::OpenRunPresetSave);
    let mut preset_value = serde_json::to_value(preset).unwrap();
    preset_value["modal"]["run_preset_name"]["existing"] =
        serde_json::json!(["not-a-current-preset"]);
    let invalid_preset: LibraryState = serde_json::from_value(preset_value).unwrap();
    assert!(
        check_state(&invalid_preset)
            .unwrap_err()
            .contains("preset names")
    );

    let mut token = run_state("prompt");
    let _ = token.update(Action::OpenRunTokenMenuFor(1));
    let mut token_value = serde_json::to_value(token).unwrap();
    token_value["modal"]["run_token_menu"]["options"] = serde_json::json!([]);
    let invalid_token: LibraryState = serde_json::from_value(token_value).unwrap();
    assert!(
        check_state(&invalid_token)
            .unwrap_err()
            .contains("token options")
    );
}

#[test]
fn rejects_stale_runner_removal_identity() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![runner("codex")]),
    ))));
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["runners"]["rows"][0]["identity"]["snapshot_token"] =
        serde_json::json!("changed-after-confirmation");
    value["workflow"]["active"]["runners"]["rows"][0]["key_identities"][0]["snapshot_token"] =
        serde_json::json!("changed-after-confirmation");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("RUNNER_REMOVAL_TARGET")
    );
}

#[test]
fn rejects_remove_confirmation_with_stale_detail_facts() {
    let mut state = LibraryState::from_library_surface(library_surface());
    let _ = state.update(Action::AskRemove);
    let mut value = serde_json::to_value(state).unwrap();
    value["modal"]["confirm_remove"]["original_file_preserved"] = serde_json::json!(false);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("preserved-file fact")
    );
}

#[test]
fn accepts_raw_removal_of_a_pinned_named_duplicate() {
    let mut stable = runner("codex");
    stable.pinned_count = 3;
    let mut duplicate = stable.clone();
    duplicate.identity = RunnerRowIdentity {
        index: Some(1),
        snapshot_token: "duplicate-row".to_owned(),
    };
    duplicate.reason = Some("duplicate runner key".to_owned());
    duplicate.descriptor = "duplicate codex".to_owned();
    let identities = vec![stable.identity.clone(), duplicate.identity.clone()];
    stable.key_identities = identities.clone();
    duplicate.key_identities = identities;
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![stable, duplicate]),
    ))));
    let _ = state.update(Action::Runners(RunnerManagerAction::Select(1)));
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));

    check_state(&state).unwrap();
}

#[test]
fn rejects_jointly_truncated_named_runner_removal_cas() {
    let mut stable = runner("codex");
    stable.pinned_count = 2;
    let mut duplicate = stable.clone();
    duplicate.identity = RunnerRowIdentity {
        index: Some(1),
        snapshot_token: "duplicate-row".to_owned(),
    };
    duplicate.reason = Some("duplicate runner key".to_owned());
    duplicate.descriptor = "duplicate codex".to_owned();
    let identities = vec![stable.identity.clone(), duplicate.identity.clone()];
    stable.key_identities = identities.clone();
    duplicate.key_identities = identities;
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![stable, duplicate]),
    ))));
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    let mut value = serde_json::to_value(state).unwrap();
    let first = value["workflow"]["active"]["runners"]["rows"][0]["identity"].clone();
    value["workflow"]["active"]["runners"]["rows"][0]["key_identities"] =
        serde_json::json!([first.clone()]);
    value["workflow"]["active"]["runners"]["rows"][1]["key_identities"] =
        serde_json::json!([first.clone()]);
    value["workflow"]["active"]["runners"]["overlay"]["removal"]["request"]["named"]["expected"] =
        serde_json::json!([first]);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("RUNNER_REMOVAL_TARGET")
    );
}

#[test]
fn rejects_remove_confirmation_with_a_duplicate_or_unselected_selector() {
    let mut state = LibraryState::from_library_surface(library_surface());
    let _ = state.update(Action::AskRemove);
    let mut duplicate = serde_json::to_value(&state).unwrap();
    let mut extra = duplicate["entries"][0].clone();
    extra["name"] = serde_json::json!("Different name");
    duplicate["entries"].as_array_mut().unwrap().push(extra);
    let invalid_duplicate: LibraryState = serde_json::from_value(duplicate).unwrap();
    assert!(
        check_state(&invalid_duplicate)
            .unwrap_err()
            .contains("REMOVE_SUBJECT")
    );

    let mut unselected = serde_json::to_value(state).unwrap();
    unselected["selected"] = serde_json::json!(1);
    let invalid_selection: LibraryState = serde_json::from_value(unselected).unwrap();
    assert!(
        check_state(&invalid_selection)
            .unwrap_err()
            .contains("REMOVE_SUBJECT")
    );
}

#[test]
fn rejects_stale_or_wrong_add_delete_subjects() {
    let confirmation = add_delete_confirmation();
    let mut stale = serde_json::to_value(&confirmation).unwrap();
    stale["workflow"]["active"]["add"]["stage"] = serde_json::json!("source");
    let invalid_stale: LibraryState = serde_json::from_value(stale).unwrap();
    assert!(check_state(&invalid_stale).is_err());

    let other = DraftSummary {
        path: PathBuf::from("/model/drafts/other.py"),
        modified: 8,
        identity: None,
        permissions: Default::default(),
        content_hash: Some("other-hash".to_owned()),
    };
    let mut wrong = serde_json::to_value(&confirmation).unwrap();
    wrong["workflow"]["active"]["add"]["delete_candidate"] = serde_json::to_value(&other).unwrap();
    let invalid_wrong: LibraryState = serde_json::from_value(wrong).unwrap();
    assert!(check_state(&invalid_wrong).is_err());

    let mut pending = confirmation;
    let _ = pending.update(Action::Add(AddAction::ConfirmDraftDelete(true)));
    let mut pending_value = serde_json::to_value(pending).unwrap();
    pending_value["workflow"]["active"]["add"]["pending_delete"][1] =
        serde_json::to_value(other).unwrap();
    let invalid_pending: LibraryState = serde_json::from_value(pending_value).unwrap();
    assert!(check_state(&invalid_pending).is_err());
}

#[test]
fn rejects_an_add_draft_selection_that_names_no_draft() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(vec![draft()]),
    ))));
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["source"]["selected_draft"] = serde_json::json!(999);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();
    assert!(check_state(&invalid).is_err());
}

#[test]
fn rejects_a_nonempty_agent_picker_without_selection_or_a_hidden_focus() {
    let mut state = LibraryState::default();
    let view = PreferencesView::new(PreferencesDraft::from_snapshot(preferences_snapshot()));
    let _ = state.update(Action::Present(Screen::Preferences(Box::new(view))));
    let _ = state.update(Action::Preferences(
        PreferencesAction::PresentAgentSkillTargets(agent_targets()),
    ));
    let mut picker = serde_json::to_value(&state).unwrap();
    picker["workflow"]["active"]["preferences"]["agent_skill_install"]["selected"] =
        serde_json::Value::Null;
    let invalid_picker: LibraryState = serde_json::from_value(picker).unwrap();
    assert_eq!(
        check_state(&invalid_picker).unwrap_err(),
        "AGENT_TARGET_SELECTION_PRESENCE selected=None targets=1"
    );

    let mut hidden = serde_json::to_value(state).unwrap();
    hidden["workflow"]["active"]["preferences"]["focused"] = serde_json::json!("pypi_url");
    let invalid_focus: LibraryState = serde_json::from_value(hidden).unwrap();
    assert_eq!(
        check_state(&invalid_focus).unwrap_err(),
        "PREFERENCES_FOCUS_HIDDEN focused=PypiUrl"
    );
}

#[test]
fn rejects_nonempty_management_surfaces_without_a_selection() {
    let mut runners = LibraryState::default();
    let _ = runners.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![runner("codex")]),
    ))));
    let mut runner_value = serde_json::to_value(runners).unwrap();
    runner_value["workflow"]["active"]["runners"]["selected"] = serde_json::Value::Null;
    let invalid_runners: LibraryState = serde_json::from_value(runner_value).unwrap();
    assert!(check_state(&invalid_runners).is_err());

    let mut health = LibraryState::default();
    let _ = health.update(Action::Present(Screen::Health(Box::new(HealthView::new(
        HealthSnapshot {
            uv: UvHealth::NotRequired,
            entry_count: 1,
            issues: vec![HealthIssue {
                slug: "missing".to_owned(),
                name: "Missing".to_owned(),
                kind: HealthIssueKind::MissingTarget,
            }],
            invalid_runner_rows: Vec::new(),
            mirror: MirrorHealth::Off,
            library_path: "/model/library".to_owned(),
            library_size: "1 KiB".to_owned(),
            diagnostics: Vec::new(),
        },
    )))));
    let mut health_value = serde_json::to_value(health).unwrap();
    health_value["workflow"]["active"]["health"]["selected_issue"] = serde_json::Value::Null;
    let invalid_health: LibraryState = serde_json::from_value(health_value).unwrap();
    assert!(check_state(&invalid_health).is_err());
}

#[test]
fn rejects_stale_manager_editor_targets() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![runner("codex")]),
    ))));
    let _ = state.update(Action::Runners(RunnerManagerAction::EditSelected));
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["runners"]["overlay"]["editor"]["target"]["named"]["expected"][0]
        ["snapshot_token"] = serde_json::json!("stale-editor-target");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();
    assert!(check_state(&invalid).is_err());
}

#[test]
fn rejects_a_kind_stage_without_its_inspected_source() {
    let mut add = AddWorkflowState::new(Vec::new());
    let _ = add.reduce(AddAction::SetSourcePath("notes.bin".to_owned()));
    let request = match add.reduce(AddAction::Continue).as_slice() {
        [AddEffect::InspectSource { request, .. }] => *request,
        effects => panic!("unexpected source effects: {effects:?}"),
    };
    let _ = add.reduce(AddAction::SourceInspected {
        request,
        result: Ok(inspected_source()),
    });
    assert_eq!(add.stage(), AddStage::Kind);
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Add(Box::new(add))));
    check_state(&state).unwrap();

    let mut stale = serde_json::to_value(&state).unwrap();
    stale["workflow"]["active"]["add"]["stage"] = serde_json::json!("source");
    stale["workflow"]["active"]["add"]["kind_picker"] = serde_json::Value::Null;
    let invalid_stale: LibraryState = serde_json::from_value(stale).unwrap();
    assert!(
        check_state(&invalid_stale)
            .unwrap_err()
            .contains("ADD_KIND_SOURCE_LIFETIME")
    );

    let mut wrong = serde_json::to_value(&state).unwrap();
    wrong["workflow"]["active"]["add"]["pending_source"]["bytes"] =
        serde_json::to_value(b"#!/usr/bin/env python3\nprint('different')\n".to_vec()).unwrap();
    let invalid_wrong: LibraryState = serde_json::from_value(wrong).unwrap();
    assert!(
        check_state(&invalid_wrong)
            .unwrap_err()
            .contains("ADD_KIND_SUBJECT")
    );

    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["pending_source"] = serde_json::Value::Null;
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("ADD_KIND_SOURCE_LIFETIME")
    );
}

#[test]
fn accepts_a_named_manager_editor_for_an_editable_invalid_duplicate() {
    let mut stable = runner("codex");
    let mut duplicate = stable.clone();
    duplicate.identity = RunnerRowIdentity {
        index: Some(1),
        snapshot_token: "duplicate-row".to_owned(),
    };
    duplicate.reason = Some("duplicate runner key".to_owned());
    duplicate.descriptor = "duplicate codex".to_owned();
    let identities = vec![stable.identity.clone(), duplicate.identity.clone()];
    stable.key_identities = identities.clone();
    duplicate.key_identities = identities;
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![stable, duplicate]),
    ))));
    let _ = state.update(Action::Runners(RunnerManagerAction::Select(1)));
    let _ = state.update(Action::Runners(RunnerManagerAction::EditSelected));

    check_state(&state).unwrap();
}

#[test]
fn rejects_runner_overlays_not_owned_by_the_selected_row() {
    let mut second = runner("other");
    second.identity.index = Some(1);
    second.identity.snapshot_token = "other-row".to_owned();
    second.key_identities = vec![second.identity.clone()];
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![runner("codex"), second]),
    ))));
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    let mut removal = serde_json::to_value(&state).unwrap();
    removal["workflow"]["active"]["runners"]["selected"] = serde_json::json!(1);
    let invalid_removal: LibraryState = serde_json::from_value(removal).unwrap();
    assert!(check_state(&invalid_removal).is_err());

    let mut raw = runner("codex");
    raw.name = None;
    raw.reason = Some("invalid row".to_owned());
    let mut repair = LibraryState::default();
    let _ = repair.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(vec![raw]),
    ))));
    let _ = repair.update(Action::Runners(RunnerManagerAction::EditSelected));
    let _ = repair.update(Action::Runners(RunnerManagerAction::Editor(
        RunnerEditorAction::MutationFailed("retry".to_owned()),
    )));
    check_state(&repair).unwrap();
    let mut stale = serde_json::to_value(repair).unwrap();
    stale["workflow"]["active"]["runners"]["overlay"]["editor"]["target"]["raw_row"]["expected"]
        ["snapshot_token"] = serde_json::json!("stale-raw-row");
    let invalid_repair: LibraryState = serde_json::from_value(stale).unwrap();
    assert!(check_state(&invalid_repair).is_err());
}

#[test]
fn rejects_a_settings_runner_editor_without_a_runner_section() {
    let selector = "python-tool".to_owned();
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Settings(Box::new(
        SettingsView::from_inputs(&SettingsInputs {
            selector: selector.clone(),
            kind: "python".to_owned(),
            name: "Python tool".to_owned(),
            ..SettingsInputs::default()
        }),
    ))));
    let mut value = serde_json::to_value(state).unwrap();
    value["modal"] = serde_json::to_value(ModalState::RunnerEditor {
        owner: RunnerEditorOwner::Settings { selector },
        view: Box::new(RunnerEditorView::new()),
        cancel_status: None,
    })
    .unwrap();
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("SETTINGS_CAPABILITY")
    );
}

#[test]
fn rejects_out_of_bounds_workflow_focus() {
    let mut run = run_state("prompt");
    let mut run_value = serde_json::to_value(run).unwrap();
    run_value["workflow"]["active"]["run"]["focused"] = serde_json::json!(999);
    run = serde_json::from_value(run_value).unwrap();
    assert!(check_state(&run).unwrap_err().contains("run focus"));

    let mut form = LibraryState::default();
    let _ = form.update(Action::Present(Screen::Form(skit_ui::FormView {
        purpose: skit_ui::FormPurpose::Rename,
        title: "Rename".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: Some("python-tool".to_owned()),
        fields: Vec::new(),
        focused: 999,
        submit_label: "Save".to_owned(),
    })));
    assert!(check_state(&form).unwrap_err().contains("form focus"));
}

#[test]
fn rejects_environment_and_file_picker_contract_drift() {
    let mut environment = run_state("prompt");
    let _ = environment.update(Action::OpenRunTokenMenuFor(1));
    let _ = environment.update(Action::OpenRunEnvironmentPicker(1));
    let mut environment_value = serde_json::to_value(environment).unwrap();
    environment_value["modal"]["run_environment_picker"]["visible"] = serde_json::json!(["STALE"]);
    let invalid_environment: LibraryState = serde_json::from_value(environment_value).unwrap();
    assert!(
        check_state(&invalid_environment)
            .unwrap_err()
            .contains("RUN_ENV_SUBJECT")
    );

    let mut file = run_state("prompt");
    let _ = file.update(Action::OpenRunFilePicker(1));
    let mut file_value = serde_json::to_value(file).unwrap();
    file_value["modal"]["run_file_picker"]["context"]["workdir"] = serde_json::json!("/stale/work");
    let invalid_file: LibraryState = serde_json::from_value(file_value).unwrap();
    assert!(
        check_state(&invalid_file)
            .unwrap_err()
            .contains("RUN_FILE_SUBJECT")
    );
}

#[test]
fn rejects_runner_editor_mode_and_recovery_drift() {
    let mut recovery = run_state("prompt");
    let _ = recovery.update(Action::OpenRunRunnerEditor);
    let mut recovery_value = serde_json::to_value(recovery).unwrap();
    recovery_value["modal"]["runner_editor"]["cancel_status"] = serde_json::json!("stale recovery");
    let invalid_recovery: LibraryState = serde_json::from_value(recovery_value).unwrap();
    assert!(
        check_state(&invalid_recovery)
            .unwrap_err()
            .contains("RUNNER_EDITOR_RUN_CAPABILITY")
    );

    let mut mode = run_state("prompt");
    let _ = mode.update(Action::OpenRunRunnerEditor);
    let mut mode_value = serde_json::to_value(mode).unwrap();
    mode_value["modal"]["runner_editor"]["view"]["target"] = serde_json::json!({
        "named": {
            "name": "codex",
            "expected": []
        }
    });
    let invalid_mode: LibraryState = serde_json::from_value(mode_value).unwrap();
    assert!(
        check_state(&invalid_mode)
            .unwrap_err()
            .contains("RUNNER_EDITOR_MODE")
    );
}

#[test]
fn rejects_noncanonical_visible_library_indices() {
    let state = LibraryState::from_library_surface(library_surface());
    let mut value = serde_json::to_value(state).unwrap();
    value["visible"] = serde_json::json!([1, 0]);
    value["selected"] = serde_json::json!(0);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("LIBRARY_VISIBLE_ORDER")
    );
}

fn kind_stage_state() -> LibraryState {
    let mut add = AddWorkflowState::new(Vec::new());
    let _ = add.reduce(AddAction::SetSourcePath("notes.bin".to_owned()));
    let request = match add.reduce(AddAction::Continue).as_slice() {
        [AddEffect::InspectSource { request, .. }] => *request,
        effects => panic!("unexpected source effects: {effects:?}"),
    };
    let _ = add.reduce(AddAction::SourceInspected {
        request,
        result: Ok(inspected_source()),
    });
    assert_eq!(add.stage(), AddStage::Kind);
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Add(Box::new(add))));
    state
}

fn prompt_settings_state() -> LibraryState {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Settings(Box::new(
        SettingsView::from_inputs(&SettingsInputs {
            selector: "prompt-tool".to_owned(),
            kind: "prompt".to_owned(),
            name: "Prompt tool".to_owned(),
            configured_runners: vec!["codex".to_owned()],
            ..SettingsInputs::default()
        }),
    ))));
    state
}

fn preferences_state() -> LibraryState {
    let mut state = LibraryState::default();
    let view = PreferencesView::new(PreferencesDraft::from_snapshot(preferences_snapshot()));
    let _ = state.update(Action::Present(Screen::Preferences(Box::new(view))));
    state
}

fn runner_manager_state(rows: Vec<RunnerRow>) -> LibraryState {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Runners(Box::new(
        RunnerManagerView::new(rows),
    ))));
    state
}

#[test]
fn rejects_a_visible_library_row_that_names_no_entry() {
    let state = LibraryState::from_library_surface(library_surface());
    let mut bounds = serde_json::to_value(state).unwrap();
    bounds["visible"] = serde_json::json!([0, 5]);
    let invalid_bounds: LibraryState = serde_json::from_value(bounds).unwrap();
    assert!(
        check_state(&invalid_bounds)
            .unwrap_err()
            .contains("LIBRARY_VISIBLE_BOUNDS index=5 entries=2")
    );
}

#[test]
fn rejects_a_library_selection_that_disagrees_with_the_visible_rows() {
    let state = LibraryState::from_library_surface(library_surface());
    let mut empty = serde_json::to_value(&state).unwrap();
    empty["visible"] = serde_json::json!([]);
    let invalid_empty: LibraryState = serde_json::from_value(empty).unwrap();
    assert!(
        check_state(&invalid_empty)
            .unwrap_err()
            .contains("LIBRARY_SELECTED_EMPTY")
    );

    let mut missing = serde_json::to_value(&state).unwrap();
    missing["selected"] = serde_json::Value::Null;
    let invalid_missing: LibraryState = serde_json::from_value(missing).unwrap();
    assert!(
        check_state(&invalid_missing)
            .unwrap_err()
            .contains("LIBRARY_SELECTED_MISSING")
    );

    let mut bounds = serde_json::to_value(state).unwrap();
    bounds["visible"] = serde_json::json!([0]);
    bounds["selected"] = serde_json::json!(3);
    let invalid_bounds: LibraryState = serde_json::from_value(bounds).unwrap();
    assert!(
        check_state(&invalid_bounds)
            .unwrap_err()
            .contains("LIBRARY_SELECTED_BOUNDS selected=3 visible=1")
    );
}

#[test]
fn rejects_a_runner_action_row_that_names_no_row() {
    let state = runner_manager_state(vec![runner("codex")]);
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["runners"]["overlay"] = serde_json::json!({ "actions": 8 });
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("runner action row 8 is out of bounds")
    );
}

#[test]
fn rejects_an_agent_target_selection_that_names_no_target() {
    let mut state = preferences_state();
    let _ = state.update(Action::Preferences(
        PreferencesAction::PresentAgentSkillTargets(agent_targets()),
    ));
    check_state(&state).unwrap();

    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["preferences"]["agent_skill_install"]["selected"] =
        serde_json::json!(5);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("agent target selection 5 is out of bounds")
    );
}

#[test]
fn rejects_a_kind_stage_without_its_picker() {
    let state = kind_stage_state();
    check_state(&state).unwrap();

    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["kind_picker"] = serde_json::Value::Null;
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("ADD_KIND_SUBJECT stage=Kind picker=false")
    );
}

#[test]
fn rejects_a_kind_stage_whose_source_path_does_not_replay() {
    let state = kind_stage_state();
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["source"]["path"] = serde_json::json!("");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("ADD_KIND_SOURCE_REPLAY")
    );
}

#[test]
fn rejects_a_review_stage_without_its_review_subject() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Add(Box::new(
        AddWorkflowState::new(vec![draft()]),
    ))));
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["stage"] = serde_json::json!("review");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("add review stage has no review subject")
    );
}

#[test]
fn rejects_a_raw_runner_removal_that_names_no_row() {
    let mut raw = runner("codex");
    raw.name = None;
    raw.reason = Some("invalid row".to_owned());
    raw.key_identities = Vec::new();
    let mut state = runner_manager_state(vec![raw]);
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    check_state(&state).unwrap();

    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["runners"]["overlay"]["removal"]["request"]["raw_row"]["expected"]
        ["snapshot_token"] = serde_json::json!("stale-raw-removal");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("RUNNER_REMOVAL_TARGET raw=")
    );
}

#[test]
fn accepts_a_new_runner_manager_editor_and_rejects_a_stale_named_target() {
    let mut state = runner_manager_state(vec![runner("codex")]);
    let _ = state.update(Action::Runners(RunnerManagerAction::New));
    check_state(&state).unwrap();

    let mut named = runner_manager_state(vec![runner("codex")]);
    let _ = named.update(Action::Runners(RunnerManagerAction::EditSelected));
    let mut value = serde_json::to_value(named).unwrap();
    value["workflow"]["active"]["runners"]["overlay"]["editor"]["name"] =
        serde_json::json!("renamed");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("RUNNER_MANAGER_EDITOR_TARGET")
    );
}

#[test]
fn accepts_the_help_modal_over_the_library() {
    let mut state = LibraryState::from_library_surface(library_surface());
    let _ = state.update(Action::OpenHelp);
    assert!(matches!(state.modal(), Some(ModalState::Help)));

    check_state(&state).unwrap();
}

#[test]
fn accepts_a_remove_confirmation_and_rejects_one_outside_the_library() {
    let mut state = LibraryState::from_library_surface(library_surface());
    let _ = state.update(Action::AskRemove);
    assert!(matches!(
        state.modal(),
        Some(ModalState::ConfirmRemove { .. })
    ));
    check_state(&state).unwrap();

    let modal = serde_json::to_value(state.modal()).unwrap();
    let mut run = serde_json::to_value(run_state("prompt")).unwrap();
    run["modal"] = modal;
    let invalid: LibraryState = serde_json::from_value(run).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("remove confirmation is not owned by the library")
    );
}

#[test]
fn accepts_a_discard_guard_for_dirty_settings_and_dirty_preferences() {
    let mut settings = prompt_settings_state();
    let _ = settings.update(Action::Settings(SettingsAction::SetField {
        key: NAME_KEY.to_owned(),
        value: FieldValue::text("Renamed prompt"),
    }));
    let _ = settings.update(Action::Settings(SettingsAction::Close));
    assert!(matches!(
        settings.modal(),
        Some(ModalState::ConfirmDiscardChanges)
    ));
    check_state(&settings).unwrap();

    let mut preferences = preferences_state();
    let _ = preferences.update(Action::Preferences(PreferencesAction::SetEditor(
        "vim".to_owned(),
    )));
    let _ = preferences.update(Action::Preferences(PreferencesAction::Close));
    assert!(matches!(
        preferences.modal(),
        Some(ModalState::ConfirmDiscardChanges)
    ));
    check_state(&preferences).unwrap();
}

#[test]
fn rejects_a_discard_guard_for_clean_preferences_or_a_plain_workflow() {
    let mut clean = serde_json::to_value(preferences_state()).unwrap();
    clean["modal"] = serde_json::json!("confirm_discard_changes");
    let invalid_clean: LibraryState = serde_json::from_value(clean).unwrap();
    assert!(
        check_state(&invalid_clean)
            .unwrap_err()
            .contains("clean preferences")
    );

    let mut library =
        serde_json::to_value(LibraryState::from_library_surface(library_surface())).unwrap();
    library["modal"] = serde_json::json!("confirm_discard_changes");
    let invalid_library: LibraryState = serde_json::from_value(library).unwrap();
    assert!(
        check_state(&invalid_library)
            .unwrap_err()
            .contains("discard confirmation has no editable workflow")
    );
}

#[test]
fn accepts_every_live_run_modal() {
    let mut preset = run_state("prompt");
    let _ = preset.update(Action::OpenRunPresetSave);
    assert!(matches!(
        preset.modal(),
        Some(ModalState::RunPresetName { .. })
    ));
    check_state(&preset).unwrap();

    let mut token = run_state("prompt");
    let _ = token.update(Action::OpenRunTokenMenuFor(1));
    check_state(&token).unwrap();

    let mut environment = run_state("prompt");
    let _ = environment.update(Action::OpenRunTokenMenuFor(1));
    let _ = environment.update(Action::OpenRunEnvironmentPicker(1));
    assert!(matches!(
        environment.modal(),
        Some(ModalState::RunEnvironmentPicker { .. })
    ));
    check_state(&environment).unwrap();

    let mut file = run_state("prompt");
    let _ = file.update(Action::OpenRunFilePicker(1));
    assert!(matches!(
        file.modal(),
        Some(ModalState::RunFilePicker { .. })
    ));
    check_state(&file).unwrap();
}

#[test]
fn rejects_run_pickers_moved_onto_a_field_without_the_capability() {
    let mut environment = run_state("prompt");
    let _ = environment.update(Action::OpenRunTokenMenuFor(1));
    let _ = environment.update(Action::OpenRunEnvironmentPicker(1));
    let mut environment_value = serde_json::to_value(environment).unwrap();
    environment_value["modal"]["run_environment_picker"]["field"] = serde_json::json!(0);
    environment_value["workflow"]["active"]["run"]["focused"] = serde_json::json!(0);
    let invalid_environment: LibraryState = serde_json::from_value(environment_value).unwrap();
    assert!(
        check_state(&invalid_environment)
            .unwrap_err()
            .contains("RUN_ENV_CAPABILITY field=0")
    );

    let mut file = run_state("prompt");
    let _ = file.update(Action::OpenRunFilePicker(1));
    let mut file_value = serde_json::to_value(file).unwrap();
    file_value["modal"]["run_file_picker"]["field"] = serde_json::json!(0);
    file_value["workflow"]["active"]["run"]["focused"] = serde_json::json!(0);
    let invalid_file: LibraryState = serde_json::from_value(file_value).unwrap();
    assert!(
        check_state(&invalid_file)
            .unwrap_err()
            .contains("RUN_FILE_CAPABILITY field=0")
    );
}

#[test]
fn rejects_a_run_modal_that_is_not_on_the_focused_field() {
    let mut state = run_state("prompt");
    let _ = state.update(Action::OpenRunTokenMenuFor(1));
    let mut value = serde_json::to_value(state).unwrap();
    value["modal"]["run_token_menu"]["field"] = serde_json::json!(0);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("RUN_MODAL_FOCUS field=0 focused=1")
    );
}

#[test]
fn rejects_a_run_modal_without_a_run_form() {
    let mut value =
        serde_json::to_value(LibraryState::from_library_surface(library_surface())).unwrap();
    value["modal"] = serde_json::to_value(ModalState::RunTokenMenu {
        field: 0,
        options: Vec::new(),
    })
    .unwrap();
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("run modal has no run form")
    );
}

#[test]
fn accepts_a_run_owned_runner_editor() {
    let mut state = run_state("prompt");
    let _ = state.update(Action::OpenRunRunnerEditor);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor { .. })
    ));

    check_state(&state).unwrap();
}

#[test]
fn accepts_a_settings_runner_editor_and_rejects_its_status_and_owner_drift() {
    let mut state = prompt_settings_state();
    let _ = state.update(Action::Settings(SettingsAction::NewRunner));
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor { .. })
    ));
    check_state(&state).unwrap();

    let mut status = serde_json::to_value(&state).unwrap();
    status["modal"]["runner_editor"]["cancel_status"] = serde_json::json!("stale recovery");
    let invalid_status: LibraryState = serde_json::from_value(status).unwrap();
    assert!(
        check_state(&invalid_status)
            .unwrap_err()
            .contains("RUNNER_EDITOR_SETTINGS_STATUS")
    );

    let mut owner = serde_json::to_value(state).unwrap();
    owner["modal"]["runner_editor"]["owner"]["settings"]["selector"] = serde_json::json!("other");
    let invalid_owner: LibraryState = serde_json::from_value(owner).unwrap();
    assert!(
        check_state(&invalid_owner)
            .unwrap_err()
            .contains("runner editor owner does not match settings")
    );
}

#[test]
fn accepts_an_add_runner_editor_and_rejects_one_without_a_prompt_review() {
    let mut state = kind_stage_state();
    let _ = state.update(Action::Add(AddAction::PickKind(Some(
        KnownEntryKind::Prompt,
    ))));
    let _ = state.update(Action::OpenAddRunnerEditor);
    assert!(matches!(
        state.modal(),
        Some(ModalState::RunnerEditor {
            owner: RunnerEditorOwner::Add,
            ..
        })
    ));
    check_state(&state).unwrap();

    let modal = serde_json::to_value(state.modal()).unwrap();
    let mut kind = serde_json::to_value(kind_stage_state()).unwrap();
    kind["modal"] = modal;
    let invalid: LibraryState = serde_json::from_value(kind).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("add runner editor has no prompt review")
    );
}

#[test]
fn rejects_a_runner_editor_without_its_owner_workflow() {
    let mut value =
        serde_json::to_value(LibraryState::from_library_surface(library_surface())).unwrap();
    value["modal"] = serde_json::to_value(ModalState::RunnerEditor {
        owner: RunnerEditorOwner::Run {
            selector: "prompt".to_owned(),
        },
        view: Box::new(RunnerEditorView::new()),
        cancel_status: None,
    })
    .unwrap();
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("runner editor has no matching owner workflow")
    );
}

fn named_removal_state() -> LibraryState {
    let mut state = runner_manager_state(vec![runner("codex")]);
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    state
}

fn pinned_duplicate_rows() -> Vec<RunnerRow> {
    let mut stable = runner("codex");
    stable.pinned_count = 3;
    let mut duplicate = stable.clone();
    duplicate.identity = RunnerRowIdentity {
        index: Some(1),
        snapshot_token: "duplicate-row".to_owned(),
    };
    duplicate.reason = Some("duplicate runner key".to_owned());
    duplicate.descriptor = "duplicate codex".to_owned();
    let identities = vec![stable.identity.clone(), duplicate.identity.clone()];
    stable.key_identities = identities.clone();
    duplicate.key_identities = identities;
    vec![stable, duplicate]
}

fn raw_removal_state() -> LibraryState {
    let mut state = runner_manager_state(pinned_duplicate_rows());
    let _ = state.update(Action::Runners(RunnerManagerAction::Select(1)));
    let _ = state.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    state
}

fn second_runner(name: &str, token: &str) -> RunnerRow {
    let mut row = runner(name);
    row.identity = RunnerRowIdentity {
        index: Some(1),
        snapshot_token: token.to_owned(),
    };
    row.key_identities = vec![row.identity.clone()];
    row
}

fn raw_repair_row() -> RunnerRow {
    let mut raw = runner("codex");
    raw.name = None;
    raw.reason = Some("invalid row".to_owned());
    raw
}

/// Return the refusal for one hand-forged edit of a valid state.
fn forged_refusal(state: &LibraryState, edit: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value = serde_json::to_value(state).unwrap();
    edit(&mut value);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();
    check_state(&invalid).unwrap_err()
}

#[test]
fn accepts_a_named_runner_removal_of_the_selected_valid_row() {
    check_state(&named_removal_state()).unwrap();
}

#[test]
fn rejects_each_named_runner_removal_fact_on_its_own() {
    let state = named_removal_state();
    let identity = serde_json::to_value(state.clone()).unwrap()["workflow"]["active"]["runners"]
        ["rows"][0]["identity"]
        .clone();
    let duplicate_identity = serde_json::json!({
        "index": 1,
        "snapshot_token": "second-codex-row"
    });

    let mut duplicated = serde_json::to_value(&state).unwrap();
    let mut extra = duplicated["workflow"]["active"]["runners"]["rows"][0].clone();
    extra["identity"] = duplicate_identity.clone();
    duplicated["workflow"]["active"]["runners"]["rows"]
        .as_array_mut()
        .unwrap()
        .push(extra);
    let both = serde_json::json!([identity, duplicate_identity]);
    duplicated["workflow"]["active"]["runners"]["rows"][0]["key_identities"] = both.clone();
    duplicated["workflow"]["active"]["runners"]["rows"][1]["key_identities"] = both.clone();
    duplicated["workflow"]["active"]["runners"]["overlay"]["removal"]["request"]["named"]["expected"] =
        both;
    let two_valid: LibraryState = serde_json::from_value(duplicated).unwrap();
    assert!(
        check_state(&two_valid)
            .unwrap_err()
            .contains("RUNNER_REMOVAL_TARGET name=\"codex\" valid=2")
    );

    for edit in [
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["request"]["named"]["expected"] =
                serde_json::json!([]);
        }) as Box<dyn FnOnce(&mut serde_json::Value)>,
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["rows"][0]["key_identities"] =
                serde_json::json!([]);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["rows"][0]["pinned_count"] =
                serde_json::json!(7);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["name"] =
                serde_json::json!("other");
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["pinned_count"] =
                serde_json::json!(5);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["invalid_row"] =
                serde_json::json!(true);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["container"] =
                serde_json::json!(true);
        }),
    ] {
        assert!(forged_refusal(&state, edit).contains("RUNNER_REMOVAL_TARGET name="));
    }

    let mut moved =
        runner_manager_state(vec![runner("codex"), second_runner("other", "other-row")]);
    let _ = moved.update(Action::Runners(RunnerManagerAction::RemoveSelected));
    assert!(
        forged_refusal(&moved, |value| {
            value["workflow"]["active"]["runners"]["selected"] = serde_json::json!(1);
        })
        .contains("RUNNER_REMOVAL_TARGET name=")
    );
}

#[test]
fn rejects_each_raw_runner_removal_fact_on_its_own() {
    let state = raw_removal_state();
    check_state(&state).unwrap();

    for edit in [
        Box::new(|value: &mut serde_json::Value| {
            let extra = value["workflow"]["active"]["runners"]["rows"][1].clone();
            value["workflow"]["active"]["runners"]["rows"]
                .as_array_mut()
                .unwrap()
                .push(extra);
        }) as Box<dyn FnOnce(&mut serde_json::Value)>,
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["rows"][1]["reason"] = serde_json::Value::Null;
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["selected"] = serde_json::json!(0);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["pinned_count"] =
                serde_json::json!(3);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["invalid_row"] =
                serde_json::json!(false);
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["overlay"]["removal"]["container"] =
                serde_json::json!(true);
        }),
    ] {
        assert!(forged_refusal(&state, edit).contains("RUNNER_REMOVAL_TARGET raw="));
    }
}

#[test]
fn rejects_a_named_manager_editor_whose_selection_moved_to_another_runner() {
    let mut state =
        runner_manager_state(vec![runner("codex"), second_runner("other", "other-row")]);
    let _ = state.update(Action::Runners(RunnerManagerAction::EditSelected));
    check_state(&state).unwrap();

    assert!(
        forged_refusal(&state, |value| {
            value["workflow"]["active"]["runners"]["selected"] = serde_json::json!(1);
        })
        .contains("RUNNER_MANAGER_EDITOR_TARGET name=")
    );
}

#[test]
fn rejects_each_raw_manager_editor_fact_on_its_own() {
    let mut state = runner_manager_state(vec![raw_repair_row()]);
    let _ = state.update(Action::Runners(RunnerManagerAction::EditSelected));
    check_state(&state).unwrap();

    for edit in [
        Box::new(|value: &mut serde_json::Value| {
            let extra = value["workflow"]["active"]["runners"]["rows"][0].clone();
            value["workflow"]["active"]["runners"]["rows"]
                .as_array_mut()
                .unwrap()
                .push(extra);
        }) as Box<dyn FnOnce(&mut serde_json::Value)>,
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["rows"][0]["name"] = serde_json::json!("codex");
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["rows"][0]["reason"] = serde_json::Value::Null;
        }),
        Box::new(|value: &mut serde_json::Value| {
            value["workflow"]["active"]["runners"]["rows"][0]["argv"] = serde_json::Value::Null;
        }),
    ] {
        assert!(forged_refusal(&state, edit).contains("RUNNER_MANAGER_EDITOR_TARGET raw="));
    }

    let mut moved =
        runner_manager_state(vec![raw_repair_row(), second_runner("other", "other-row")]);
    let _ = moved.update(Action::Runners(RunnerManagerAction::EditSelected));
    assert!(
        forged_refusal(&moved, |value| {
            value["workflow"]["active"]["runners"]["selected"] = serde_json::json!(1);
        })
        .contains("RUNNER_MANAGER_EDITOR_TARGET raw=")
    );
}

#[test]
fn rejects_a_kind_picker_that_does_not_match_the_replayed_source() {
    let state = kind_stage_state();
    let mut value = serde_json::to_value(state).unwrap();
    value["workflow"]["active"]["add"]["kind_picker"]["filename"] = serde_json::json!("other.bin");
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    let error = check_state(&invalid).unwrap_err();
    assert!(error.contains("ADD_KIND_SUBJECT"));
    assert!(error.contains("replay_stage=Kind"));
}

#[test]
fn accepts_a_health_screen_with_its_selected_issue() {
    let mut state = LibraryState::default();
    let _ = state.update(Action::Present(Screen::Health(Box::new(HealthView::new(
        HealthSnapshot {
            uv: UvHealth::NotRequired,
            entry_count: 1,
            issues: vec![HealthIssue {
                slug: "missing".to_owned(),
                name: "Missing".to_owned(),
                kind: HealthIssueKind::MissingTarget,
            }],
            invalid_runner_rows: Vec::new(),
            mirror: MirrorHealth::Off,
            library_path: "/model/library".to_owned(),
            library_size: "1 KiB".to_owned(),
            diagnostics: Vec::new(),
        },
    )))));

    check_state(&state).unwrap();
}

#[test]
fn accepts_an_empty_form_and_rejects_a_focus_at_the_field_count() {
    let mut empty = LibraryState::default();
    let _ = empty.update(Action::Present(Screen::Form(skit_ui::FormView {
        purpose: skit_ui::FormPurpose::Rename,
        title: "Rename".to_owned(),
        title_arguments: Vec::new(),
        translate_title: true,
        selector: Some("python-tool".to_owned()),
        fields: Vec::new(),
        focused: 0,
        submit_label: "Save".to_owned(),
    })));
    check_state(&empty).unwrap();

    let mut value = serde_json::to_value(run_state("prompt")).unwrap();
    value["workflow"]["active"]["run"]["focused"] = serde_json::json!(3);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();
    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("RUN_FOCUS_BOUNDS run focus=3 fields=3")
    );
}

#[test]
fn rejects_a_remove_confirmation_whose_selection_only_shares_the_entry_name() {
    let mut state = LibraryState::from_library_surface(library_surface());
    let _ = state.update(Action::AskRemove);
    let mut value = serde_json::to_value(state).unwrap();
    value["entries"][1]["name"] = serde_json::json!("Python tool");
    value["selected"] = serde_json::json!(1);
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("REMOVE_SUBJECT")
    );
}

#[test]
fn rejects_an_add_runner_editor_over_a_review_that_is_not_a_prompt() {
    let mut prompt = kind_stage_state();
    let _ = prompt.update(Action::Add(AddAction::PickKind(Some(
        KnownEntryKind::Prompt,
    ))));
    let _ = prompt.update(Action::OpenAddRunnerEditor);
    let modal = serde_json::to_value(prompt.modal()).unwrap();

    let mut script = kind_stage_state();
    let _ = script.update(Action::Add(AddAction::PickKind(Some(
        KnownEntryKind::Python,
    ))));
    let mut value = serde_json::to_value(script).unwrap();
    value["modal"] = modal;
    let invalid: LibraryState = serde_json::from_value(value).unwrap();

    assert!(
        check_state(&invalid)
            .unwrap_err()
            .contains("add runner editor has no prompt review")
    );
}
