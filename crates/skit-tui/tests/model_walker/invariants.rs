use std::{collections::BTreeMap, path::PathBuf};

use skit_application::{preferences::PreferencesDraft, tokens::TokenContext};
use skit_domain::parameters::ParamDecl;
use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, Effect, HealthIssue,
    HealthIssueKind, HealthSnapshot, HealthView, LibraryState, MirrorHealth, ModalState,
    PreferencesAction, PreferencesView, ReviewLane, RunFormContext, RunFormView, RunPathContext,
    RunnerEditorAction, RunnerEditorMode, RunnerEditorOwner, RunnerEditorView, RunnerManagerAction,
    RunnerManagerView, RunnerRemoveRequest, RunnerRow, RunnerRowIdentity, RunnerSaveTarget, Screen,
    SettingsInputs, SettingsSectionId, SettingsView, SourceSnapshot, UvHealth,
};

pub(super) fn check_state(state: &LibraryState) -> Result<(), String> {
    let encoded = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    let decoded: LibraryState =
        serde_json::from_slice(&encoded).map_err(|error| error.to_string())?;
    if state != &decoded {
        return Err("the UI state changed during its JSON round trip".to_owned());
    }

    check_library_indices(&serde_json::from_slice(&encoded).map_err(|error| error.to_string())?)?;
    check_screen(state)?;
    check_modal(state)
}

fn check_library_indices(value: &serde_json::Value) -> Result<(), String> {
    let entries = value
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .ok_or("LIBRARY_SCHEMA entries is not an array")?;
    let visible = value
        .get("visible")
        .and_then(serde_json::Value::as_array)
        .ok_or("LIBRARY_SCHEMA visible is not an array")?;
    let mut previous = None;
    for raw in visible {
        let index = raw
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| format!("LIBRARY_VISIBLE_TYPE value={raw}"))?;
        if index >= entries.len() {
            return Err(format!(
                "LIBRARY_VISIBLE_BOUNDS index={index} entries={}",
                entries.len()
            ));
        }
        if previous.is_some_and(|previous| previous >= index) {
            return Err(format!(
                "LIBRARY_VISIBLE_ORDER previous={previous:?} current={index}"
            ));
        }
        previous = Some(index);
    }
    let selected = match value.get("selected") {
        Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| format!("LIBRARY_SELECTED_TYPE value={value}"))?,
        ),
        None => return Err("LIBRARY_SCHEMA selected is missing".to_owned()),
    };
    if visible.is_empty() && selected.is_some() {
        return Err(format!("LIBRARY_SELECTED_EMPTY selected={selected:?}"));
    }
    if !visible.is_empty() && selected.is_none() {
        return Err("LIBRARY_SELECTED_MISSING visible is not empty".to_owned());
    }
    if let Some(selected) = selected
        && selected >= visible.len()
    {
        return Err(format!(
            "LIBRARY_SELECTED_BOUNDS selected={selected} visible={}",
            visible.len()
        ));
    }
    Ok(())
}

fn check_screen(state: &LibraryState) -> Result<(), String> {
    match state.screen() {
        Screen::Add(add) => check_add(add),
        Screen::Health(health) => {
            if health.selected_issue().is_some() != !health.snapshot().issues.is_empty() {
                return Err(format!(
                    "HEALTH_SELECTION_PRESENCE selected={:?} issues={}",
                    health.selected_issue(),
                    health.snapshot().issues.len()
                ));
            }
            if let Some(selected) = health.selected_issue()
                && selected >= health.snapshot().issues.len()
            {
                return Err(format!(
                    "health selection {selected} is out of bounds for {} issues",
                    health.snapshot().issues.len()
                ));
            }
            Ok(())
        }
        Screen::Run(run) => check_focus("run", run.focused(), run.fields().len()),
        Screen::Form(form) => check_focus("form", form.focused, form.fields.len()),
        Screen::Runners(runners) => {
            if runners.selected().is_some() != !runners.rows().is_empty() {
                return Err(format!(
                    "RUNNER_SELECTION_PRESENCE selected={:?} rows={}",
                    runners.selected(),
                    runners.rows().len()
                ));
            }
            if let Some(selected) = runners.selected()
                && selected >= runners.rows().len()
            {
                return Err(format!(
                    "runner selection {selected} is out of bounds for {} rows",
                    runners.rows().len()
                ));
            }
            if let Some(index) = runners.action_row()
                && index >= runners.rows().len()
            {
                return Err(format!(
                    "runner action row {index} is out of bounds for {} rows",
                    runners.rows().len()
                ));
            }
            if let Some(removal) = runners.removal() {
                check_runner_removal(runners, removal)?;
            }
            if let Some(editor) = runners.editor() {
                check_runner_editor(runners, editor)?;
            }
            Ok(())
        }
        Screen::Preferences(preferences) => {
            if !preferences.has_control(preferences.focused()) {
                return Err(format!(
                    "PREFERENCES_FOCUS_HIDDEN focused={:?}",
                    preferences.focused()
                ));
            }
            if let Some(picker) = preferences.agent_skill_install() {
                if picker.selected().is_some() != !picker.targets().is_empty() {
                    return Err(format!(
                        "AGENT_TARGET_SELECTION_PRESENCE selected={:?} targets={}",
                        picker.selected(),
                        picker.targets().len()
                    ));
                }
                if let Some(selected) = picker.selected()
                    && selected >= picker.targets().len()
                {
                    return Err(format!(
                        "agent target selection {selected} is out of bounds for {} targets",
                        picker.targets().len()
                    ));
                }
            }
            Ok(())
        }
        Screen::Library | Screen::Settings(_) | Screen::Report(_) => Ok(()),
    }
}

fn check_add(add: &AddWorkflowState) -> Result<(), String> {
    let value = serde_json::to_value(add).map_err(|error| format!("ADD_SCHEMA encode={error}"))?;
    let pending_source = value
        .get("pending_source")
        .ok_or("ADD_SCHEMA pending_source is missing")?;
    let pending_delete = value
        .get("pending_delete")
        .ok_or("ADD_SCHEMA pending_delete is missing")?;

    if add.source().selected_draft.is_some() && add.source().selected_draft().is_none() {
        return Err(format!(
            "ADD_DRAFT_SELECTION_BOUNDS selected={:?} listed={}",
            add.source().selected_draft,
            add.source().listed_drafts().len()
        ));
    }
    if (add.stage() == AddStage::Kind) != add.kind_picker().is_some() {
        return Err(format!(
            "ADD_KIND_SUBJECT stage={:?} picker={}",
            add.stage(),
            add.kind_picker().is_some()
        ));
    }
    if (add.stage() == AddStage::Kind) == pending_source.is_null() {
        return Err(format!(
            "ADD_KIND_SOURCE_LIFETIME stage={:?} pending={}",
            add.stage(),
            !pending_source.is_null()
        ));
    }
    if add.stage() == AddStage::Kind {
        let source: SourceSnapshot = serde_json::from_value(pending_source.clone())
            .map_err(|error| format!("ADD_KIND_SOURCE_SCHEMA source={error}"))?;
        let mut replay = AddWorkflowState::new(add.source().listed_drafts().to_vec())
            .with_review_defaults(add.review_defaults().clone());
        let _ = replay.reduce(AddAction::SetSourcePath(add.source().path.clone()));
        let request = match replay.reduce(AddAction::Continue).as_slice() {
            [AddEffect::InspectSource { request, .. }] => *request,
            effects => {
                return Err(format!(
                    "ADD_KIND_SOURCE_REPLAY path={:?} effects={effects:?}",
                    add.source().path
                ));
            }
        };
        let _ = replay.reduce(AddAction::SourceInspected {
            request,
            result: Ok(source),
        });
        if replay.stage() != AddStage::Kind || replay.kind_picker() != add.kind_picker() {
            return Err(format!(
                "ADD_KIND_SUBJECT current={:?} replay={:?} replay_stage={:?}",
                add.kind_picker(),
                replay.kind_picker(),
                replay.stage()
            ));
        }
    }
    if add.stage() == AddStage::Review && add.review().is_none() {
        return Err("add review stage has no review subject".to_owned());
    }

    let confirming = add.stage() == AddStage::ConfirmDraftDelete;
    if confirming != add.delete_candidate().is_some() {
        return Err(if confirming {
            "draft delete confirmation has no candidate".to_owned()
        } else {
            format!(
                "ADD_DELETE_LIFETIME stage={:?} candidate={:?}",
                add.stage(),
                add.delete_candidate()
            )
        });
    }
    if let Some(candidate) = add.delete_candidate()
        && add.source().selected_draft() != Some(candidate)
    {
        return Err(format!(
            "ADD_DELETE_SUBJECT candidate={candidate:?} selected={:?}",
            add.source().selected_draft()
        ));
    }
    if !pending_delete.is_null() {
        let values = pending_delete
            .as_array()
            .ok_or("ADD_SCHEMA pending_delete is not a tuple")?;
        let pending_candidate: DraftSummary = serde_json::from_value(
            values
                .get(1)
                .cloned()
                .ok_or("ADD_SCHEMA pending_delete has no draft")?,
        )
        .map_err(|error| format!("ADD_SCHEMA pending_delete draft={error}"))?;
        if add.delete_candidate() != Some(&pending_candidate) {
            return Err(format!(
                "ADD_DELETE_PENDING pending={pending_candidate:?} candidate={:?}",
                add.delete_candidate()
            ));
        }
    }
    Ok(())
}

fn check_focus(label: &str, focused: usize, fields: usize) -> Result<(), String> {
    if (fields == 0 && focused == 0) || (fields > 0 && focused < fields) {
        Ok(())
    } else {
        Err(format!(
            "{}_FOCUS_BOUNDS {label} focus={focused} fields={fields}",
            label.to_ascii_uppercase()
        ))
    }
}

fn check_runner_removal(
    runners: &RunnerManagerView,
    removal: &skit_ui::RunnerRemovalView,
) -> Result<(), String> {
    let value = serde_json::to_value(removal)
        .map_err(|error| format!("RUNNER_REMOVAL_SCHEMA encode={error}"))?;
    let request: RunnerRemoveRequest = serde_json::from_value(
        value
            .get("request")
            .cloned()
            .ok_or("RUNNER_REMOVAL_SCHEMA request is missing")?,
    )
    .map_err(|error| format!("RUNNER_REMOVAL_SCHEMA request={error}"))?;
    match request {
        RunnerRemoveRequest::Named {
            name,
            expected,
            expected_pinned_count,
        } => {
            let identities = named_runner_identities(runners.rows(), &name);
            let candidates = named_runner_rows(runners.rows(), &name);
            let valid = candidates.iter().filter(|row| row.is_valid()).count();
            let caches_match = candidates
                .iter()
                .all(|row| row.key_identities == identities);
            let pins_match = candidates
                .iter()
                .all(|row| row.pinned_count == expected_pinned_count);
            let selected_matches = runners.selected().is_some_and(|selected| {
                runners
                    .rows()
                    .get(selected)
                    .is_some_and(|row| row.is_valid() && row.name.as_deref() == Some(name.as_str()))
            });
            if valid != 1
                || expected != identities
                || !caches_match
                || !pins_match
                || !selected_matches
                || removal.name != name
                || removal.pinned_count != expected_pinned_count
                || removal.invalid_row
                || removal.container
            {
                return Err(format!(
                    "RUNNER_REMOVAL_TARGET name={name:?} valid={valid} candidates={} expected={expected:?} current={identities:?} pins={expected_pinned_count} selected={:?}",
                    candidates.len(),
                    runners.selected()
                ));
            }
        }
        RunnerRemoveRequest::RawRow { expected } => {
            let candidates = runners
                .rows()
                .iter()
                .filter(|row| row.identity == expected)
                .collect::<Vec<_>>();
            let row = candidates.first();
            let selected_matches = runners.selected().is_some_and(|selected| {
                runners
                    .rows()
                    .get(selected)
                    .is_some_and(|row| row.identity == expected)
            });
            if candidates.len() != 1
                || row.is_none_or(|row| row.is_valid() || row.label() != removal.name)
                || !selected_matches
                || removal.pinned_count != 0
                || !removal.invalid_row
                || removal.container != expected.index.is_none()
            {
                return Err(format!(
                    "RUNNER_REMOVAL_TARGET raw={expected:?} matches={} invalid={} container={}",
                    candidates.len(),
                    removal.invalid_row,
                    removal.container
                ));
            }
        }
    }
    Ok(())
}

fn named_runner_rows<'a>(rows: &'a [RunnerRow], name: &str) -> Vec<&'a RunnerRow> {
    rows.iter()
        .filter(|row| row.name.as_deref() == Some(name))
        .collect()
}

fn named_runner_identities(rows: &[RunnerRow], name: &str) -> Vec<RunnerRowIdentity> {
    named_runner_rows(rows, name)
        .into_iter()
        .map(|row| row.identity.clone())
        .collect()
}

fn check_runner_editor(
    runners: &RunnerManagerView,
    editor: &skit_ui::RunnerEditorView,
) -> Result<(), String> {
    let value = serde_json::to_value(editor)
        .map_err(|error| format!("RUNNER_EDITOR_SCHEMA encode={error}"))?;
    let target: RunnerSaveTarget = serde_json::from_value(
        value
            .get("target")
            .cloned()
            .ok_or("RUNNER_EDITOR_SCHEMA target is missing")?,
    )
    .map_err(|error| format!("RUNNER_EDITOR_SCHEMA target={error}"))?;
    match target {
        RunnerSaveTarget::New => {
            if editor.mode() != RunnerEditorMode::New {
                return Err(format!(
                    "RUNNER_MANAGER_EDITOR_MODE target=new mode={:?}",
                    editor.mode()
                ));
            }
        }
        RunnerSaveTarget::Named { name, expected } => {
            let identities = named_runner_identities(runners.rows(), &name);
            let candidates = named_runner_rows(runners.rows(), &name);
            let selected_matches = runners.selected().is_some_and(|selected| {
                runners.rows().get(selected).is_some_and(|row| {
                    row.is_editable() && row.name.as_deref() == Some(name.as_str())
                })
            });
            if editor.mode() != RunnerEditorMode::Edit
                || editor.name() != name
                || expected != identities
                || candidates.is_empty()
                || !selected_matches
                || candidates
                    .iter()
                    .any(|row| row.key_identities != identities)
            {
                return Err(format!(
                    "RUNNER_MANAGER_EDITOR_TARGET name={name:?} editor_name={:?} expected={expected:?} current={identities:?} candidates={} selected={:?}",
                    editor.name(),
                    candidates.len(),
                    runners.selected()
                ));
            }
        }
        RunnerSaveTarget::RawRow { expected } => {
            let candidates = runners
                .rows()
                .iter()
                .filter(|row| row.identity == expected)
                .collect::<Vec<_>>();
            let selected_matches = runners.selected().is_some_and(|selected| {
                runners
                    .rows()
                    .get(selected)
                    .is_some_and(|row| row.identity == expected)
            });
            if editor.mode() != RunnerEditorMode::Repair
                || candidates.len() != 1
                || candidates
                    .first()
                    .is_none_or(|row| row.name.is_some() || row.is_valid() || !row.is_editable())
                || !selected_matches
            {
                return Err(format!(
                    "RUNNER_MANAGER_EDITOR_TARGET raw={expected:?} matches={} selected={:?}",
                    candidates.len(),
                    runners.selected()
                ));
            }
        }
    }
    Ok(())
}

fn check_modal(state: &LibraryState) -> Result<(), String> {
    let Some(modal) = state.modal() else {
        return Ok(());
    };
    match modal {
        ModalState::Help => Ok(()),
        ModalState::ConfirmRemove {
            selector,
            name,
            original_file_preserved,
        } => {
            if !matches!(state.screen(), Screen::Library) {
                return Err("remove confirmation is not owned by the library".to_owned());
            }
            let value = serde_json::to_value(state).map_err(|error| error.to_string())?;
            let entries = value
                .get("entries")
                .and_then(serde_json::Value::as_array)
                .ok_or("REMOVE_SCHEMA entries is not an array")?;
            let slug_matches = entries
                .iter()
                .filter(|entry| entry["slug"].as_str() == Some(selector))
                .collect::<Vec<_>>();
            let selected_matches = state
                .selected()
                .is_some_and(|entry| entry.slug.as_str() == selector && entry.name == *name);
            if slug_matches.len() != 1
                || slug_matches[0]["name"].as_str() != Some(name)
                || !selected_matches
            {
                return Err(format!(
                    "REMOVE_SUBJECT selector={selector:?} name={name:?} slug_matches={} selected={:?}",
                    slug_matches.len(),
                    state.selected()
                ));
            }
            let details = value
                .get("details")
                .and_then(serde_json::Value::as_object)
                .ok_or("REMOVE_SCHEMA details is not an object")?;
            let current_preserved = details
                .get(selector)
                .and_then(|detail| detail.get("original_file_preserved"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if current_preserved != *original_file_preserved {
                return Err(format!(
                    "REMOVE_PRESERVED_FACT selector={selector:?} modal={original_file_preserved} detail={current_preserved}; preserved-file fact is stale"
                ));
            }
            Ok(())
        }
        ModalState::ConfirmDiscardChanges => match state.screen() {
            Screen::Preferences(preferences) if preferences.dirty() => Ok(()),
            Screen::Settings(settings) if settings.is_dirty() => Ok(()),
            Screen::Settings(_) => {
                Err("DISCARD_CLEAN_SETTINGS guard owns clean settings".to_owned())
            }
            Screen::Preferences(_) => Err("discard confirmation owns clean preferences".to_owned()),
            Screen::Library
            | Screen::Run(_)
            | Screen::Add(_)
            | Screen::Health(_)
            | Screen::Runners(_)
            | Screen::Form(_)
            | Screen::Report(_) => Err("discard confirmation has no editable workflow".to_owned()),
        },
        ModalState::RunPresetName { existing, .. } => {
            let run = require_run_form(state)?;
            let current = run.preset_names().map(str::to_owned).collect();
            if !run.has_parameters() || existing != &current {
                return Err(format!(
                    "RUN_PRESET_SUBJECT has_parameters={} modal={existing:?} current={current:?}; preset names are stale",
                    run.has_parameters()
                ));
            }
            Ok(())
        }
        ModalState::RunTokenMenu { field, options } => {
            let run = require_run_modal_field(state, *field)?;
            let current = run.focused_token_options();
            if current.as_ref() != Some(options) {
                return Err(format!(
                    "RUN_TOKEN_OPTIONS field={field} modal={options:?} current={current:?}; token options are stale"
                ));
            }
            Ok(())
        }
        ModalState::RunEnvironmentPicker {
            field,
            names,
            query,
            visible,
        } => {
            let run = require_run_modal_field(state, *field)?;
            if run.focused_token_options().is_none() {
                return Err(format!("RUN_ENV_CAPABILITY field={field}"));
            }
            let current_names = run
                .context()
                .ok_or_else(|| format!("RUN_ENV_CONTEXT field={field}"))?
                .tokens
                .env
                .keys()
                .cloned()
                .collect::<Vec<_>>();
            let expected_visible = expected_environment_visible(state, *field, query)?;
            if names != &current_names || visible != &expected_visible {
                return Err(format!(
                    "RUN_ENV_SUBJECT field={field} names={names:?} current={current_names:?} visible={visible:?} expected={expected_visible:?}"
                ));
            }
            Ok(())
        }
        ModalState::RunFilePicker {
            field,
            context,
            mode,
        } => {
            let run = require_run_modal_field(state, *field)?;
            if !run.can_browse_field(*field) {
                return Err(format!("RUN_FILE_CAPABILITY field={field}"));
            }
            let (expected_context, expected_mode) = expected_file_contract(state, *field)?;
            if context != &expected_context || mode != &expected_mode {
                return Err(format!(
                    "RUN_FILE_SUBJECT field={field} modal=({context:?},{mode:?}) current=({expected_context:?},{expected_mode:?})"
                ));
            }
            Ok(())
        }
        ModalState::RunnerEditor {
            owner,
            view,
            cancel_status,
        } => {
            if view.mode() != RunnerEditorMode::New {
                return Err(format!(
                    "RUNNER_EDITOR_MODE owner={owner:?} mode={:?}",
                    view.mode()
                ));
            }
            match (owner, state.screen()) {
                (RunnerEditorOwner::Run { selector }, Screen::Run(run))
                    if selector == run.selector() =>
                {
                    if cancel_status.is_some() == run.has_runner_picker() {
                        return Err(format!(
                            "RUNNER_EDITOR_RUN_CAPABILITY selector={selector:?} picker={} cancel_status={cancel_status:?}",
                            run.has_runner_picker()
                        ));
                    }
                    Ok(())
                }
                (RunnerEditorOwner::Run { .. }, Screen::Run(_)) => {
                    Err("runner editor owner does not match the run form".to_owned())
                }
                (RunnerEditorOwner::Settings { selector }, Screen::Settings(settings))
                    if selector == &settings.selector =>
                {
                    if cancel_status.is_none() && settings.has_section(SettingsSectionId::Runner) {
                        Ok(())
                    } else if !settings.has_section(SettingsSectionId::Runner) {
                        Err("RUNNER_EDITOR_SETTINGS_CAPABILITY has no runner section".to_owned())
                    } else {
                        Err("RUNNER_EDITOR_SETTINGS_STATUS has cancel status".to_owned())
                    }
                }
                (RunnerEditorOwner::Settings { .. }, Screen::Settings(_)) => {
                    Err("runner editor owner does not match settings".to_owned())
                }
                (RunnerEditorOwner::Add, Screen::Add(add))
                    if add.stage() == AddStage::Review
                        && add
                            .review()
                            .is_some_and(|review| review.lane() == ReviewLane::Prompt)
                        && cancel_status.is_none() =>
                {
                    Ok(())
                }
                (RunnerEditorOwner::Add, Screen::Add(_)) => {
                    Err("add runner editor has no prompt review".to_owned())
                }
                (RunnerEditorOwner::Run { .. }, _)
                | (RunnerEditorOwner::Settings { .. }, _)
                | (RunnerEditorOwner::Add, _) => {
                    Err("runner editor has no matching owner workflow".to_owned())
                }
            }
        }
    }
}

fn require_run_modal_field(state: &LibraryState, field: usize) -> Result<&RunFormView, String> {
    let run = require_run_form(state)?;
    if field >= run.fields().len() {
        return Err(format!(
            "run modal field {field} is out of bounds for {} fields",
            run.fields().len()
        ));
    }
    if run.focused() != field {
        return Err(format!(
            "RUN_MODAL_FOCUS field={field} focused={}",
            run.focused()
        ));
    }
    Ok(run)
}

fn expected_environment_visible(
    state: &LibraryState,
    field: usize,
    query: &str,
) -> Result<Vec<String>, String> {
    let mut replay = state.clone();
    let _ = replay.update(Action::Back);
    let _ = replay.update(Action::OpenRunTokenMenuFor(field));
    let _ = replay.update(Action::OpenRunEnvironmentPicker(field));
    let _ = replay.update(Action::SetRunEnvironmentQuery(query.to_owned()));
    match replay.modal() {
        Some(ModalState::RunEnvironmentPicker { visible, .. }) => Ok(visible.clone()),
        modal => Err(format!(
            "RUN_ENV_REPLAY field={field} query={query:?} modal={modal:?}"
        )),
    }
}

fn expected_file_contract(
    state: &LibraryState,
    field: usize,
) -> Result<(skit_ui::RunPathContext, skit_ui::RunPathInsertMode), String> {
    let mut replay = state.clone();
    let _ = replay.update(Action::Back);
    let _ = replay.update(Action::OpenRunFilePicker(field));
    match replay.modal() {
        Some(ModalState::RunFilePicker { context, mode, .. }) => Ok((context.clone(), *mode)),
        modal => Err(format!("RUN_FILE_REPLAY field={field} modal={modal:?}")),
    }
}

fn require_run_form(state: &LibraryState) -> Result<&RunFormView, String> {
    match state.screen() {
        Screen::Run(run) => Ok(run),
        Screen::Library
        | Screen::Preferences(_)
        | Screen::Add(_)
        | Screen::Health(_)
        | Screen::Runners(_)
        | Screen::Settings(_)
        | Screen::Form(_)
        | Screen::Report(_) => Err("run modal has no run form".to_owned()),
    }
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
    let fixtures = super::fixtures::fixture_set();
    let mut state =
        LibraryState::from_library_surface(super::fixtures::surface(fixtures.entries.into_iter()));
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
    let fixtures = super::fixtures::fixture_set();
    let mut state =
        LibraryState::from_library_surface(super::fixtures::surface(fixtures.entries.into_iter()));
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
    let fixtures = super::fixtures::fixture_set();
    let mut state = LibraryState::default();
    let view = PreferencesView::new(PreferencesDraft::from_snapshot(fixtures.preferences));
    let _ = state.update(Action::Present(Screen::Preferences(Box::new(view))));
    let _ = state.update(Action::Preferences(
        PreferencesAction::PresentAgentSkillTargets(fixtures.agent_targets),
    ));
    let mut picker = serde_json::to_value(&state).unwrap();
    picker["workflow"]["active"]["preferences"]["agent_skill_install"]["selected"] =
        serde_json::Value::Null;
    let invalid_picker: LibraryState = serde_json::from_value(picker).unwrap();
    assert!(check_state(&invalid_picker).is_err());

    let mut hidden = serde_json::to_value(state).unwrap();
    hidden["workflow"]["active"]["preferences"]["focused"] = serde_json::json!("pypi_url");
    let invalid_focus: LibraryState = serde_json::from_value(hidden).unwrap();
    assert!(check_state(&invalid_focus).is_err());
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
    let mut source = super::fixtures::fixture_set()
        .sources
        .into_values()
        .next()
        .expect("fixture source");
    source.path = PathBuf::from("notes.bin");
    source.source_record = "notes.bin".to_owned();
    source.bytes = b"plain bytes\n".to_vec();
    source.executable = Some(false);
    let _ = add.reduce(AddAction::SourceInspected {
        request,
        result: Ok(source),
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
    let fixtures = super::fixtures::fixture_set();
    let state =
        LibraryState::from_library_surface(super::fixtures::surface(fixtures.entries.into_iter()));
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
