use skit_ui::{
    Action, AddAction, AddEffect, AddStage, AddWorkflowState, DraftSummary, LibraryState,
    ModalState, ReviewLane, RunFormView, RunnerEditorMode, RunnerEditorOwner, RunnerManagerView,
    RunnerRemoveRequest, RunnerRow, RunnerRowIdentity, RunnerSaveTarget, Screen, SettingsSectionId,
    SourceSnapshot,
};

/// Check every named invariant over one reducer state.
///
/// The state passes a JSON round trip first. The library indices, the active screen, and the
/// modal are checked next. The error names the rule that the state breaks.
pub fn check_state(state: &LibraryState) -> Result<(), String> {
    let encoded = serde_json::to_vec(state).map_err(|error| error.to_string())?;
    let decoded: LibraryState =
        serde_json::from_slice(&encoded).map_err(|error| error.to_string())?;
    let round_trip = (state == &decoded).then_some(());
    round_trip.ok_or("the UI state changed during its JSON round trip")?;

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
    let raw_selected = value
        .get("selected")
        .ok_or("LIBRARY_SCHEMA selected is missing")?;
    let selected = match raw_selected {
        serde_json::Value::Null => None,
        value => Some(
            value
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .ok_or_else(|| format!("LIBRARY_SELECTED_TYPE value={value}"))?,
        ),
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
    if (fields == 0 && focused == 0) || focused < fields {
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
            // The editor reads its mode from this target. The message is built first so every
            // line of the rule runs.
            let mode = editor.mode();
            let refusal = format!("RUNNER_MANAGER_EDITOR_MODE target=new mode={mode:?}");
            (mode == RunnerEditorMode::New)
                .then_some(())
                .ok_or(refusal)?;
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
    // The replay reopens a picker the caller already proved reachable. The result is collected
    // into an option so every line of the rule runs.
    let modal = replay.modal();
    let mut replayed = None;
    if let Some(ModalState::RunEnvironmentPicker { visible, .. }) = modal {
        replayed = Some(visible.clone());
    }
    replayed.ok_or_else(|| format!("RUN_ENV_REPLAY field={field} query={query:?} modal={modal:?}"))
}

fn expected_file_contract(
    state: &LibraryState,
    field: usize,
) -> Result<(skit_ui::RunPathContext, skit_ui::RunPathInsertMode), String> {
    let mut replay = state.clone();
    let _ = replay.update(Action::Back);
    let _ = replay.update(Action::OpenRunFilePicker(field));
    // The replay reopens a picker the caller already proved reachable. The result is collected
    // into an option so every line of the rule runs.
    let modal = replay.modal();
    let mut replayed = None;
    if let Some(ModalState::RunFilePicker { context, mode, .. }) = modal {
        replayed = Some((context.clone(), *mode));
    }
    replayed.ok_or_else(|| format!("RUN_FILE_REPLAY field={field} modal={modal:?}"))
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
