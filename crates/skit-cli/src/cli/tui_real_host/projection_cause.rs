//! Checkpoint cause projection, provenance, and session projection.

#[cfg(any(unix, windows))]
use std::ffi::OsString;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use skit_tui::AGENT_REVIEW_SNAPSHOT_VERSION;
use skit_tui_walker_support::engine::CheckpointCauseProjection;
use skit_ui::{
    Action, AddAction, Effect, LibraryState, PreferencesAction, RunnerManagerAction, Screen,
};

use super::{
    observation::escaped_os,
    projection::{
        AddProjectionCause, AddProjectionProvenance, PathMap, ReviewNameProjection,
        StatusProjectionCause, draft_source_path,
    },
    projection_typed::NestedProjection,
};

pub(super) fn add_projection_cause(action: &AddAction) -> AddProjectionCause {
    match action {
        AddAction::SetSourcePath(_) => AddProjectionCause::SourcePathEdited,
        AddAction::PickedSourcePath(path) => AddProjectionCause::PickedSourcePath(path.clone()),
        AddAction::SelectDraft(index) => AddProjectionCause::SelectDraft(*index),
        AddAction::SourceInspected { result, .. } => {
            AddProjectionCause::ReviewSource(result.as_ref().ok().and_then(draft_source_path))
        }
        AddAction::DraftEdited { result, .. } => AddProjectionCause::ReviewSource(
            result
                .as_ref()
                .ok()
                .and_then(Option::as_ref)
                .and_then(draft_source_path),
        ),
        AddAction::SetReviewName(_) => AddProjectionCause::ReviewNameEdited,
        AddAction::SourceEdited {
            result: Ok(source), ..
        } => AddProjectionCause::ReviewSourceEdited(draft_source_path(source)),
        AddAction::SetCommandTemplate(_)
        | AddAction::SetCommandName(_)
        | AddAction::SetCommandDescription(_)
        | AddAction::HighlightDraft(_)
        | AddAction::Continue
        | AddAction::PickKind(_)
        | AddAction::NewDraft(_)
        | AddAction::DeleteSelectedDraft
        | AddAction::ConfirmDraftDelete(_)
        | AddAction::DraftDeleted { .. }
        | AddAction::SetReviewDescription(_)
        | AddAction::SetReviewStorage(_)
        | AddAction::SetReviewDependencies(_)
        | AddAction::SetReviewPython(_)
        | AddAction::SetReviewCandidate { .. }
        | AddAction::SetPromptInterpolation(_)
        | AddAction::SetPromptCandidate { .. }
        | AddAction::SetPromptCandidates(_)
        | AddAction::SetPromptRunner { .. }
        | AddAction::PromptRunnerAdded(_)
        | AddAction::EditSource
        | AddAction::SourceEdited { result: Err(_), .. }
        | AddAction::Save
        | AddAction::CommitFinished { .. }
        | AddAction::Cancel => AddProjectionCause::None,
    }
}

impl PathMap {
    pub(super) fn project_checkpoint_cause(
        &mut self,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<(), String> {
        match cause {
            CheckpointCauseProjection::Observation => {
                self.add_cause = AddProjectionCause::None;
                self.status_cause = StatusProjectionCause::None;
                Ok(())
            }
            CheckpointCauseProjection::Reducer { action, emitted } => {
                self.record_add_projection_cause(action);
                self.record_status_projection_cause(action);
                self.record_runner_status_provenance(action);
                self.project_typed_action(action)?;
                self.project_typed_effect(emitted)
            }
            CheckpointCauseProjection::Host {
                request,
                response,
                emitted,
            } => {
                self.record_add_projection_cause(response);
                self.record_status_projection_cause(response);
                self.record_runner_status_provenance(response);
                self.project_typed_effect(request)?;
                self.project_typed_action(response)?;
                self.project_typed_effect(emitted)
            }
        }
    }

    pub(super) fn project_typed_library_state(
        &mut self,
        state: &mut LibraryState,
    ) -> Result<(), String> {
        let value = serde_json::to_value(&*state).map_err(|error| error.to_string())?;
        self.reconcile_add_provenance(&value)?;
        self.reconcile_status_provenance(&value)?;
        self.reconcile_runner_status_provenance(&value);
        let value = self.normalize_library_json(value)?;
        *state = deserialize_canonical(value, "LibraryState")?;
        Ok(())
    }

    pub(super) fn record_add_projection_cause(&mut self, action: &Action) {
        self.add_cause = match action {
            Action::Present(Screen::Add(_))
            | Action::AddCompleted { .. }
            | Action::AddCancelled => AddProjectionCause::Reset,
            Action::Add(nested) => add_projection_cause(nested),
            _ => AddProjectionCause::None,
        };
    }

    fn record_status_projection_cause(&mut self, action: &Action) {
        self.status_cause = match action {
            Action::Preferences(PreferencesAction::AgentSkillInstalled { message }) => {
                StatusProjectionCause::AgentSkillInstalled(message.clone())
            }
            Action::RunnerEditorSaved { .. }
            | Action::PreferencesSaved { .. }
            | Action::RunPresetSaved { .. }
            | Action::AddCompleted { .. }
            | Action::Complete { .. }
            | Action::SetStatus(_)
            | Action::ClearStatus => StatusProjectionCause::Replaced,
            _ => StatusProjectionCause::None,
        };
    }

    pub(super) fn reconcile_status_provenance(&mut self, state: &Value) -> Result<(), String> {
        let raw_status = state.get("status").and_then(Value::as_str);
        match std::mem::take(&mut self.status_cause) {
            StatusProjectionCause::AgentSkillInstalled(message) => {
                if raw_status != Some(message.as_str()) {
                    return Err(
                        "the Agent Skill install status does not match its cause".to_owned()
                    );
                }
                self.status_provenance = Some(message);
            }
            StatusProjectionCause::Replaced => self.status_provenance = None,
            StatusProjectionCause::None => {
                if self.status_provenance.as_deref() != raw_status {
                    self.status_provenance = None;
                }
            }
        }
        Ok(())
    }

    fn record_runner_status_provenance(&mut self, action: &Action) {
        match action {
            Action::Runners(RunnerManagerAction::MutationFailed(message)) => {
                self.runner_failure_status = Some(message.clone());
            }
            Action::Runners(RunnerManagerAction::MutationSucceeded { .. })
            | Action::Present(Screen::Runners(_))
            | Action::RunnerManagerClosed { .. } => self.runner_failure_status = None,
            _ => {}
        }
    }

    fn reconcile_runner_status_provenance(&mut self, state: &Value) {
        let raw = state
            .pointer("/workflow/active/runners/status")
            .and_then(Value::as_str);
        if self.runner_failure_status.as_deref() != raw {
            self.runner_failure_status = None;
        }
    }

    pub(super) fn reconcile_add_provenance(&mut self, state: &Value) -> Result<(), String> {
        let cause = std::mem::take(&mut self.add_cause);
        if matches!(cause, AddProjectionCause::Reset) {
            self.add_provenance = AddProjectionProvenance::default();
        }
        let Some(add) = state.pointer("/workflow/active/add") else {
            self.add_provenance = AddProjectionProvenance::default();
            return if matches!(cause, AddProjectionCause::PickedSourcePath(_)) {
                Err("a picked Add source path has no active Add state".to_owned())
            } else {
                Ok(())
            };
        };
        match cause {
            AddProjectionCause::None | AddProjectionCause::Reset => {}
            AddProjectionCause::SelectDraft(index) => {
                let path = add_draft_path_at(add, index)?;
                if !self.draft_paths.contains_key(&path) {
                    return Err("a selected Add draft path is not registered".to_owned());
                }
                if add
                    .pointer("/source/selected_draft")
                    .and_then(Value::as_u64)
                    != u64::try_from(index).ok()
                {
                    return Err(
                        "a selected Add draft index does not match the raw state".to_owned()
                    );
                }
                if add.pointer("/source/path").and_then(Value::as_str)
                    != Some(path.to_string_lossy().as_ref())
                {
                    return Err("a selected Add draft path does not match the raw state".to_owned());
                }
                self.add_provenance.selected_source_path = Some(path);
                self.add_provenance.picked_source_path = None;
            }
            AddProjectionCause::PickedSourcePath(raw) => {
                let _ = self.require_exact_picker_file(&raw)?;
                if add.pointer("/source/path").and_then(Value::as_str) != Some(raw.as_str()) {
                    return Err("a picked Add source path does not match the raw state".to_owned());
                }
                self.add_provenance.selected_source_path = None;
                self.add_provenance.picked_source_path = Some(raw);
            }
            AddProjectionCause::SourcePathEdited => {
                self.add_provenance.selected_source_path = None;
                self.add_provenance.picked_source_path = None;
            }
            AddProjectionCause::ReviewSource(path) => {
                self.add_provenance.review_source_path = path;
                self.add_provenance.review_name = None;
                self.add_provenance.review_name_edited = false;
            }
            AddProjectionCause::ReviewNameEdited => {
                self.add_provenance.review_name_yank = self
                    .add_provenance
                    .review_name
                    .take()
                    .or_else(|| self.add_provenance.review_name_yank.take());
                self.add_provenance.review_name_edited = true;
            }
            AddProjectionCause::ReviewSourceEdited(path) => {
                self.add_provenance.review_source_path = path;
            }
        }
        self.reconcile_selected_source(add);
        self.reconcile_review_name(add)
    }

    fn reconcile_selected_source(&mut self, add: &Value) {
        let source = add.pointer("/source/path").and_then(Value::as_str);
        if self
            .add_provenance
            .selected_source_path
            .as_ref()
            .is_some_and(|path| source != Some(path.to_string_lossy().as_ref()))
        {
            self.add_provenance.selected_source_path = None;
        }
        if self
            .add_provenance
            .picked_source_path
            .as_ref()
            .is_some_and(|path| source != Some(path.as_str()))
        {
            self.add_provenance.picked_source_path = None;
        }
    }

    pub(super) fn require_exact_picker_file(&self, raw: &str) -> Result<&Path, String> {
        let path = Path::new(raw);
        self.file_picker_files
            .iter()
            .find(|candidate| candidate.as_os_str() == path.as_os_str())
            .map(PathBuf::as_path)
            .ok_or_else(|| "a picked Add source path is not an exact seeded file".to_owned())
    }

    pub(super) fn add_source_path_projection(&self) -> Result<Option<(String, String)>, String> {
        debug_assert!(
            self.add_provenance.selected_source_path.is_none()
                || self.add_provenance.picked_source_path.is_none(),
            "Add draft and picker source provenance must not overlap",
        );
        if let Some(path) = self.add_provenance.selected_source_path.as_ref() {
            return self
                .draft_paths
                .get(path)
                .map(|draft| {
                    Some((
                        draft.raw_path.to_string_lossy().into_owned(),
                        draft.stable_path.clone(),
                    ))
                })
                .ok_or_else(|| "a selected Add draft path is not registered".to_owned());
        }
        let Some(raw) = self.add_provenance.picked_source_path.as_ref() else {
            return Ok(None);
        };
        let exact = self.require_exact_picker_file(raw)?;
        Ok(Some((raw.clone(), self.normalize_path(exact))))
    }

    fn reconcile_review_name(&mut self, add: &Value) -> Result<(), String> {
        let pending_matches = self
            .add_provenance
            .review_source_path
            .as_ref()
            .is_some_and(|path| {
                add.pointer("/pending_source/path")
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| Path::new(candidate) == path)
            });
        if pending_matches {
            self.add_provenance.review_name = None;
            return Ok(());
        }
        let Some(raw_name) = add.pointer("/review/name").and_then(Value::as_str) else {
            self.add_provenance.review_source_path = None;
            self.add_provenance.review_name = None;
            self.add_provenance.review_name_edited = false;
            return Ok(());
        };
        if let Some(projection) = self.add_provenance.review_name.as_ref() {
            if raw_name == projection.raw_name {
                return Ok(());
            }
            self.add_provenance.review_name_yank = self.add_provenance.review_name.take();
            self.add_provenance.review_name_edited = true;
            return Ok(());
        }
        if self.add_provenance.review_name_edited {
            return Ok(());
        }
        if add
            .pointer("/review_defaults/name")
            .is_some_and(|name| !name.is_null())
        {
            self.add_provenance.review_name_edited = true;
            return Ok(());
        }
        let Some(path) = self.add_provenance.review_source_path.clone() else {
            return Ok(());
        };
        if !add
            .pointer("/review/source/path")
            .and_then(Value::as_str)
            .is_some_and(|candidate| Path::new(candidate) == path)
        {
            self.add_provenance.review_source_path = None;
            self.add_provenance.review_name_edited = true;
            return Ok(());
        }
        let Some(path_projection) = self.draft_paths.get(&path) else {
            return Err("an auto-derived Add review source path is not registered".to_owned());
        };
        let kind = add
            .pointer("/review/kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let expected_raw = review_default_name(&path, kind);
        if raw_name == expected_raw {
            self.add_provenance.review_name = Some(ReviewNameProjection {
                source_path: path,
                kind: kind.to_owned(),
                raw_name: expected_raw,
                stable_name: review_default_name(Path::new(&path_projection.stable_path), kind),
            });
        } else {
            self.add_provenance.review_name_edited = true;
        }
        Ok(())
    }

    pub(super) fn project_session_value(&mut self, session: &mut Value) -> Result<(), String> {
        self.project_agent_review_session(session)
    }

    fn project_agent_review_session(&self, session: &mut Value) -> Result<(), String> {
        let schema = session
            .as_object()
            .ok_or("the agent-review session is not an object".to_owned())?
            .get("schema_version")
            .and_then(Value::as_u64);
        if schema != Some(u64::from(AGENT_REVIEW_SNAPSHOT_VERSION)) {
            return Err(format!(
                "the agent-review session schema_version must be {AGENT_REVIEW_SNAPSHOT_VERSION}"
            ));
        }

        {
            let fields = top_agent_review_fields_mut(session, "preferences", "preferences")?;
            self.project_agent_signature(fields)?;
        }
        {
            let fields = top_agent_review_fields_mut(session, "add", "add")?;
            self.project_add_session_paths(fields)?;
            self.project_add_input_mirrors(fields)?;
        }
        {
            let fields =
                top_agent_review_fields_mut(session, "path_suggestions", "path_suggestions")?;
            self.project_path_suggestion_paths(fields)?;
        }
        {
            let fields = top_agent_review_fields_mut(session, "run_modal", "run_modal")?;
            self.project_run_modal_signature_paths(fields)?;
            let file = required_field_mut(fields, "file", "run_modal")?;
            if !file.is_null() {
                self.project_file_picker_node(file)?;
            }
            let source = required_field_mut(fields, "file_picker_source", "run_modal")?;
            if !source.is_null() {
                self.project_memory_file_picker_node(source)?;
            }
        }

        let add_overlay = required_session_owner_mut(session, "add_overlay")?;
        if !add_overlay.is_null() {
            let kind = agent_review_kind(add_overlay)?.to_owned();
            match kind.as_str() {
                "add_file_overlay" => {
                    let fields = agent_review_fields_mut(add_overlay, "add_file_overlay")?;
                    let file = fields
                        .get_mut("session")
                        .ok_or("the add_file_overlay node is missing session".to_owned())?;
                    self.project_file_picker_node(file)?;
                }
                "add_prompt_overlay" => {
                    let _ = agent_review_fields_mut(add_overlay, "add_prompt_overlay")?;
                }
                kind => return Err(format!("the add_overlay node has unknown kind {kind}")),
            }
        }

        let source = required_session_owner_mut(session, "file_picker_source")?;
        if !source.is_null() {
            self.project_memory_file_picker_node(source)?;
        }
        Ok(())
    }

    fn project_agent_signature(&self, fields: &mut Value) -> Result<(), String> {
        let signature = required_field_mut(fields, "agent_signature", "preferences")?;
        if signature.is_null() {
            return Ok(());
        }
        let targets = signature
            .as_array_mut()
            .ok_or("the preferences agent_signature is not an array".to_owned())?;
        for target in targets {
            let base = required_field_mut(target, "base", "preferences agent target")?;
            self.normalize_session_path_value(base)?;
        }
        Ok(())
    }

    fn project_add_session_paths(&self, fields: &mut Value) -> Result<(), String> {
        let picker_root = required_field_mut(fields, "picker_root", "add session")?;
        if !picker_root.is_null() {
            self.normalize_session_path_value(picker_root)?;
        }
        let advertised = required_field_mut(fields, "advertised", "add session")?
            .as_array_mut()
            .ok_or("the add session advertised field is not an array".to_owned())?;
        for row in advertised {
            let event = required_field_mut(row, "event", "add advertised row")?;
            match event {
                Value::String(tag)
                    if matches!(
                        tag.as_str(),
                        "open_prompt_candidates" | "open_runner_editor" | "changed"
                    ) => {}
                Value::Object(object) if object.len() == 1 => {
                    if object.contains_key("action") {
                        continue;
                    }
                    let contract = object.get_mut("open_path_picker").ok_or(
                        "an add advertised event has an unknown object variant".to_owned(),
                    )?;
                    let start = required_field_mut(contract, "start_dir", "add advertised picker")?;
                    self.normalize_session_path_value(start)?;
                    let policy =
                        required_field_mut(contract, "output_policy", "add advertised picker")?;
                    self.project_session_output_policy(policy)?;
                }
                Value::String(_) | Value::Object(_) => {
                    return Err("an add advertised event has a malformed variant".to_owned());
                }
                _ => return Err("an add advertised event has a malformed shape".to_owned()),
            }
        }
        Ok(())
    }

    fn project_path_suggestion_paths(&self, fields: &mut Value) -> Result<(), String> {
        let expected = required_field_mut(fields, "expected", "path_suggestions")?;
        if !expected.is_null() {
            let request = required_field_mut(expected, "request", "path suggestion expected")?;
            let context = required_field_mut(request, "context", "path suggestion request")?;
            let workdir = required_field_mut(context, "workdir", "path suggestion context")?;
            self.normalize_session_path_value(workdir)?;
            let tokens = required_field_mut(context, "tokens", "path suggestion context")?;
            let cwd = required_field_mut(tokens, "cwd", "path suggestion tokens")?;
            self.normalize_session_path_value(cwd)?;
            let home = required_field_mut(tokens, "home", "path suggestion tokens")?;
            if !home.is_null() {
                self.normalize_session_path_value(home)?;
            }
            let env = required_field_mut(tokens, "env", "path suggestion tokens")?
                .as_object_mut()
                .ok_or("the path suggestion token env is not an object".to_owned())?;
            for value in env.values_mut() {
                self.normalize_session_path_value(value)?;
            }
        }
        let visible = required_field_mut(fields, "visible", "path_suggestions")?;
        if !visible.is_null() {
            let suggestion = required_field_mut(visible, "suggestion", "visible path suggestion")?;
            self.normalize_session_path_value(suggestion)?;
        }
        Ok(())
    }

    fn project_run_modal_signature_paths(&self, fields: &mut Value) -> Result<(), String> {
        let signature = required_field_mut(fields, "signature", "run_modal")?;
        match signature {
            Value::Null => {}
            Value::String(tag) if matches!(tag.as_str(), "preset" | "other") => {}
            Value::Object(object) if object.len() == 1 => {
                let tag = object.keys().next().expect("one signature member").clone();
                match tag.as_str() {
                    "file" => {
                        let file = object.get_mut("file").expect("the file member exists");
                        let context =
                            required_field_mut(file, "context", "run modal file signature")?;
                        for field in ["workdir", "invoke_cwd"] {
                            let path =
                                required_field_mut(context, field, "run modal file context")?;
                            self.normalize_session_path_value(path)?;
                        }
                    }
                    "token" => {
                        let token = object.get_mut("token").expect("the token member exists");
                        let options =
                            required_field_mut(token, "options", "run modal token signature")?
                                .as_array_mut()
                                .ok_or("the run modal token options are not an array".to_owned())?;
                        for option in options {
                            if let Some(fixed) = option.get_mut("fixed_directory") {
                                let path =
                                    required_field_mut(fixed, "path", "fixed directory option")?;
                                self.normalize_session_path_value(path)?;
                            }
                        }
                    }
                    "environment" => {}
                    _ => return Err("the run modal signature has an unknown variant".to_owned()),
                }
            }
            Value::String(_) | Value::Object(_) => {
                return Err("the run modal signature has a malformed variant".to_owned());
            }
            _ => return Err("the run modal signature has a malformed shape".to_owned()),
        }
        Ok(())
    }

    fn project_session_output_policy(&self, policy: &mut Value) -> Result<(), String> {
        match policy {
            Value::String(value) if value == "absolute" => Ok(()),
            Value::Object(object) if object.len() == 1 => {
                let relative = object
                    .get_mut("relative_to")
                    .ok_or("a session output policy has an unknown variant".to_owned())?;
                self.normalize_session_path_value(relative)
            }
            Value::String(_) | Value::Object(_) => {
                Err("a session output policy has a malformed variant".to_owned())
            }
            _ => Err("a session output policy has a malformed shape".to_owned()),
        }
    }

    fn project_add_input_mirrors(&self, fields: &mut Value) -> Result<(), String> {
        let stage = add_session_stage(fields)?;
        let source_owned = matches!(stage, Some(skit_ui::AddStage::Source));
        let review_owned = matches!(stage, Some(skit_ui::AddStage::Review));
        let source_projection = source_owned
            .then(|| self.add_source_path_projection())
            .transpose()?
            .flatten();
        let Some(inputs) = fields.get_mut("inputs").and_then(Value::as_array_mut) else {
            return Err("the add session fields are missing the inputs array".to_owned());
        };
        let mut source_seen = false;
        let mut review_seen = false;
        for input in inputs {
            let id = input.get("id").and_then(Value::as_str);
            let mut cut_projection = None;
            let projection = match id {
                Some("SourcePath") => {
                    if source_seen {
                        return Err("the add session has duplicate source_path inputs".to_owned());
                    }
                    source_seen = true;
                    source_projection.clone()
                }
                Some("ReviewName") => {
                    if review_seen {
                        return Err("the add session has duplicate review_name inputs".to_owned());
                    }
                    review_seen = true;
                    cut_projection = review_owned
                        .then(|| {
                            self.add_provenance
                                .review_name_yank
                                .as_ref()
                                .map(|projection| {
                                    (projection.raw_name.clone(), projection.stable_name.clone())
                                })
                        })
                        .flatten();
                    review_owned
                        .then(|| {
                            self.add_provenance.review_name.as_ref().map(|projection| {
                                (projection.raw_name.clone(), projection.stable_name.clone())
                            })
                        })
                        .flatten()
                }
                Some(_) | None => None,
            };
            if projection.is_none() && cut_projection.is_none() {
                continue;
            }
            let state = input
                .get_mut("state")
                .ok_or("an auto-derived Add input is missing state".to_owned())?;
            let cut = cut_projection
                .as_ref()
                .map(|(raw, stable)| (raw.as_str(), stable.as_str()));
            if let Some((raw, stable)) = projection {
                project_default_line_input(state, &raw, &stable, cut)?;
            } else {
                project_default_line_input_cut(state, cut)?;
            }
        }
        if source_projection.is_some() && !source_seen {
            return Err("the add session is missing its derived source_path input".to_owned());
        }
        if review_owned && self.add_provenance.review_name.is_some() && !review_seen {
            return Err("the add session is missing its derived review_name input".to_owned());
        }
        Ok(())
    }

    fn project_file_picker_node(&self, node: &mut Value) -> Result<(), String> {
        let fields = agent_review_fields_mut(node, "file_picker")?;
        let contract = required_field_mut(fields, "contract", "file_picker")?;
        let start = required_field_mut(contract, "start_dir", "file_picker contract")?;
        self.normalize_session_path_value(start)?;
        let policy = required_field_mut(contract, "output_policy", "file_picker contract")?;
        self.project_session_output_policy(policy)?;
        let explorer = required_field_mut(fields, "explorer", "file_picker")?;
        let current = required_field_mut(explorer, "current_dir", "file_picker explorer")?;
        self.normalize_session_path_value(current)?;
        let entries = required_field_mut(explorer, "entries", "file_picker explorer")?
            .as_array_mut()
            .ok_or("the file_picker explorer is missing its entries array".to_owned())?;
        for entry in entries {
            let raw_path = entry
                .get("path")
                .ok_or("the file_picker entry value is missing path".to_owned())
                .and_then(decode_session_path_value)?;
            if let Some(projection) = self.draft_paths.get(&raw_path) {
                let name = entry
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("a draft file-picker entry is missing its text name".to_owned())?;
                if name != projection.raw_file_picker_entry_name {
                    return Err("a draft file-picker entry name does not match its path".to_owned());
                }
                entry["name"] = Value::String(projection.stable_basename.clone());
            }
            let path = required_field_mut(entry, "path", "file_picker entry")?;
            self.normalize_session_path_value(path)?;
            if let Some(symlink) = entry.pointer_mut("/entry_type/symlink") {
                let target = required_field_mut(symlink, "target", "file_picker symlink")?;
                if !target.is_null() {
                    self.normalize_session_path_value(target)?;
                }
            }
        }
        let selected = required_field_mut(explorer, "selected_files", "file_picker explorer")?
            .as_array_mut()
            .ok_or("the file_picker selected_files field is not an array".to_owned())?;
        for path in selected {
            self.normalize_session_path_value(path)?;
        }
        let io_error = required_field_mut(fields, "io_error", "file_picker")?;
        match io_error {
            Value::Null => {}
            Value::String(_) => self.normalize_host_text_value(io_error),
            _ => return Err("the file_picker io_error is not null or text".to_owned()),
        }
        let memory = fields
            .get_mut("memory_source")
            .ok_or("the file_picker node is missing memory_source".to_owned())?;
        if memory.is_null() {
            return Err("the file_picker memory_source is null".to_owned());
        }
        self.project_memory_file_picker_node(memory)
    }

    fn project_memory_file_picker_node(&self, node: &mut Value) -> Result<(), String> {
        let fields = agent_review_fields_mut(node, "memory_file_picker_source")?;
        let root = required_field_mut(fields, "root", "memory_file_picker_source")?;
        self.normalize_session_path_value(root)?;
        for field in ["directories", "files"] {
            let paths = required_field_mut(fields, field, "memory_file_picker_source")?
                .as_array_mut()
                .ok_or_else(|| {
                    format!("the memory_file_picker_source {field} field is not an array")
                })?;
            for path in paths {
                self.normalize_session_path_value(path)?;
            }
        }
        Ok(())
    }
}

pub(super) fn draft_projection_suffix(path: &Path) -> String {
    let basename = path.file_name().map(escaped_os).unwrap_or_default();
    let lowercase = basename.to_ascii_lowercase();
    for semantic_suffix in [".prompt.md", ".prompt"] {
        if lowercase.ends_with(semantic_suffix) {
            return basename[basename.len() - semantic_suffix.len()..].to_owned();
        }
    }
    path.extension()
        .map(|value| format!(".{}", escaped_os(value)))
        .unwrap_or_default()
}

pub(super) fn review_default_name(path: &Path, kind: &str) -> String {
    let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or("entry");
    if kind == "prompt" {
        stem.strip_suffix(".prompt").unwrap_or(stem).to_owned()
    } else {
        stem.to_owned()
    }
}

fn add_draft_path_at(add: &Value, index: usize) -> Result<PathBuf, String> {
    add.pointer(&format!("/source/drafts/{index}/path"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| "a selected Add draft is missing its text path".to_owned())
}

fn required_session_owner_mut<'a>(
    session: &'a mut Value,
    owner: &str,
) -> Result<&'a mut Value, String> {
    session
        .as_object_mut()
        .and_then(|object| object.get_mut(owner))
        .ok_or_else(|| format!("the agent-review session is missing {owner}"))
}

fn required_field_mut<'a>(
    value: &'a mut Value,
    field: &str,
    owner: &str,
) -> Result<&'a mut Value, String> {
    value
        .as_object_mut()
        .ok_or_else(|| format!("the {owner} value is not an object"))?
        .get_mut(field)
        .ok_or_else(|| format!("the {owner} value is missing {field}"))
}

fn top_agent_review_fields_mut<'a>(
    session: &'a mut Value,
    owner: &str,
    expected_kind: &str,
) -> Result<&'a mut Value, String> {
    let node = required_session_owner_mut(session, owner)?;
    agent_review_fields_mut(node, expected_kind)
}

fn agent_review_kind(node: &Value) -> Result<&str, String> {
    node.as_object()
        .ok_or("an agent-review node is not an object".to_owned())?
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("an agent-review node is missing its text kind".to_owned())
}

fn agent_review_fields_mut<'a>(
    node: &'a mut Value,
    expected_kind: &str,
) -> Result<&'a mut Value, String> {
    let object = node
        .as_object_mut()
        .ok_or_else(|| format!("the {expected_kind} agent-review node is not an object"))?;
    match object.get("kind") {
        Some(Value::String(kind)) if kind == expected_kind => {}
        Some(Value::String(kind)) => {
            return Err(format!(
                "expected agent-review node kind {expected_kind}, but found {kind}"
            ));
        }
        Some(_) => {
            return Err(format!(
                "the {expected_kind} agent-review node kind is not text"
            ));
        }
        None => {
            return Err(format!(
                "the {expected_kind} agent-review node kind is missing"
            ));
        }
    }
    if !object.contains_key("fields") {
        return Err(format!(
            "the {expected_kind} agent-review node fields are missing"
        ));
    }
    if object.len() != 2 {
        return Err(format!(
            "the {expected_kind} agent-review node must contain only kind and fields"
        ));
    }
    let fields = object
        .get_mut("fields")
        .ok_or_else(|| format!("the {expected_kind} agent-review node fields are missing"))?;
    if !fields.is_object() {
        return Err(format!(
            "the {expected_kind} agent-review node fields are not an object"
        ));
    }
    Ok(fields)
}

fn add_session_stage(fields: &Value) -> Result<Option<skit_ui::AddStage>, String> {
    let signature = fields
        .get("signature")
        .ok_or("the add session is missing signature".to_owned())?;
    if signature.is_null() {
        return Ok(None);
    }
    let stage = signature
        .as_object()
        .ok_or("the add session signature is not an object".to_owned())?
        .get("stage")
        .ok_or("the add session signature is missing stage".to_owned())?;
    serde_json::from_value(stage.clone())
        .map(Some)
        .map_err(|_| "the add session signature has an invalid stage".to_owned())
}

fn project_default_line_input(
    state: &mut Value,
    raw: &str,
    stable: &str,
    cut: Option<(&str, &str)>,
) -> Result<(), String> {
    let object = default_line_input_object(state)?;
    if object.get("value").and_then(Value::as_str) != Some(raw) {
        return Err("an auto-derived Add input value does not match its state".to_owned());
    }
    if object
        .get("cursor")
        .and_then(Value::as_u64)
        .and_then(|cursor| usize::try_from(cursor).ok())
        != Some(raw.chars().count())
    {
        return Err("an auto-derived Add input cursor is not at the end".to_owned());
    }
    if !project_derived_cut(object, cut)? && object.get("yank").and_then(Value::as_str) != Some("")
    {
        return Err("an auto-derived Add input has nondefault yank state".to_owned());
    }
    if object.get("last_was_cut").and_then(Value::as_bool) != Some(false) {
        return Err("an auto-derived Add input has nondefault cut state".to_owned());
    }
    object.insert("value".to_owned(), Value::String(stable.to_owned()));
    object.insert("cursor".to_owned(), Value::from(stable.chars().count()));
    Ok(())
}

/// Project only the cut derived name that one Add input still holds.
///
/// The value is user text again, so it keeps its exact bytes. The mirror still
/// owns the cut buffer while that buffer holds the name the host derived.
fn project_default_line_input_cut(
    state: &mut Value,
    cut: Option<(&str, &str)>,
) -> Result<(), String> {
    let object = default_line_input_object(state)?;
    project_derived_cut(object, cut).map(|_| ())
}

fn default_line_input_object(state: &mut Value) -> Result<&mut Map<String, Value>, String> {
    let object = state
        .as_object_mut()
        .ok_or("an auto-derived Add input state is not an object".to_owned())?;
    if object.len() != 4
        || !["value", "cursor", "yank", "last_was_cut"]
            .into_iter()
            .all(|field| object.contains_key(field))
    {
        return Err("an auto-derived Add input state is not a default LineInput shape".to_owned());
    }
    Ok(object)
}

/// Replace one cut buffer that holds the exact derived name the host proved.
///
/// It reports whether it replaced the buffer. Every other cut text is user
/// text and keeps its exact bytes.
fn project_derived_cut(
    object: &mut Map<String, Value>,
    cut: Option<(&str, &str)>,
) -> Result<bool, String> {
    let yank = object
        .get("yank")
        .and_then(Value::as_str)
        .ok_or("an auto-derived Add input yank is not text".to_owned())?;
    let Some(stable) = cut.and_then(|(raw, stable)| (yank == raw).then_some(stable)) else {
        return Ok(false);
    };
    object.insert("yank".to_owned(), Value::String(stable.to_owned()));
    Ok(true)
}

pub(super) fn decode_session_path_value(value: &Value) -> Result<PathBuf, String> {
    match value {
        Value::String(path) => Ok(PathBuf::from(path)),
        Value::Null => Err("a required agent-review path is null".to_owned()),
        Value::Object(_) => decode_session_native_path(value),
        _ => Err("an agent-review path has a malformed shape".to_owned()),
    }
}

#[cfg(unix)]
fn decode_session_native_path(value: &Value) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt as _;

    let object = value
        .as_object()
        .ok_or("an agent-review native path is not an object".to_owned())?;
    if object.contains_key("windows_wide") {
        return Err("an agent-review native path uses the wrong platform shape".to_owned());
    }
    let bytes = object
        .get("unix_bytes")
        .and_then(Value::as_array)
        .ok_or("an agent-review native path is missing unix_bytes".to_owned())?;
    if object.len() != 1 {
        return Err("an agent-review native path has extra fields".to_owned());
    }
    let bytes = bytes
        .iter()
        .map(|byte| {
            byte.as_u64()
                .and_then(|byte| u8::try_from(byte).ok())
                .ok_or("an agent-review unix path byte is invalid".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let path = PathBuf::from(OsString::from_vec(bytes));
    if path.as_os_str().to_str().is_some() {
        return Err("an agent-review Unix path object is not canonical UTF-8".to_owned());
    }
    Ok(path)
}

#[cfg(windows)]
fn decode_session_native_path(value: &Value) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt as _;

    let object = value
        .as_object()
        .ok_or("an agent-review native path is not an object".to_owned())?;
    if object.contains_key("unix_bytes") {
        return Err("an agent-review native path uses the wrong platform shape".to_owned());
    }
    let units = object
        .get("windows_wide")
        .and_then(Value::as_array)
        .ok_or("an agent-review native path is missing windows_wide".to_owned())?;
    if object.len() != 1 {
        return Err("an agent-review native path has extra fields".to_owned());
    }
    let units = units
        .iter()
        .map(|unit| {
            unit.as_u64()
                .and_then(|unit| u16::try_from(unit).ok())
                .ok_or("an agent-review Windows path unit is invalid".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let path = PathBuf::from(OsString::from_wide(&units));
    if path.as_os_str().to_str().is_some() {
        return Err("an agent-review Windows path object is not canonical Unicode".to_owned());
    }
    Ok(path)
}

#[cfg(not(any(unix, windows)))]
fn decode_session_native_path(_value: &Value) -> Result<PathBuf, String> {
    Err("agent-review native paths are unsupported on this platform".to_owned())
}

pub(super) fn require_external_unit(value: &Value, expected: &str) -> Result<(), String> {
    match value {
        Value::String(tag) if tag == expected => Ok(()),
        Value::String(tag) => Err(format!(
            "expected serialized tag {expected}, but found {tag}"
        )),
        Value::Object(object) if object.len() != 1 => Err(format!(
            "serialized tag {expected} must have exactly one object member"
        )),
        Value::Object(_) => Err(format!("serialized tag {expected} must not have a payload")),
        _ => Err(format!("serialized tag {expected} has a malformed shape")),
    }
}

pub(super) fn external_tag(value: &Value) -> Result<&str, String> {
    match value {
        Value::String(tag) => Ok(tag),
        Value::Object(object) if object.len() == 1 => object
            .keys()
            .next()
            .map(String::as_str)
            .ok_or("a serialized external tag object is empty".to_owned()),
        Value::Object(object) if object.is_empty() => {
            Err("a serialized external tag object is empty".to_owned())
        }
        Value::Object(_) => {
            Err("a serialized external tag object has more than one member".to_owned())
        }
        _ => Err("a serialized external tag has a malformed shape".to_owned()),
    }
}

pub(super) fn screen_tag(screen: &Screen) -> &'static str {
    match screen {
        Screen::Library => "library",
        Screen::Run(_) => "run",
        Screen::Preferences(_) => "preferences",
        Screen::Add(_) => "add",
        Screen::Health(_) => "health",
        Screen::Runners(_) => "runners",
        Screen::Settings(_) => "settings",
        Screen::Form(_) => "form",
        Screen::Report(_) => "report",
    }
}

pub(super) fn external_payload_mut<'a>(
    value: &'a mut Value,
    expected: &str,
) -> Result<&'a mut Value, String> {
    let Value::Object(object) = value else {
        return Err(format!("serialized tag {expected} must have one payload"));
    };
    if object.len() != 1 {
        return Err(format!(
            "serialized tag {expected} must have exactly one object member"
        ));
    }
    object
        .get_mut(expected)
        .ok_or_else(|| format!("serialized tag {expected} is missing"))
}

pub(super) fn require_internal_tag(
    value: &Value,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("serialized {expected} value must be an object"))?;
    match object.get(field) {
        Some(Value::String(tag)) if tag == expected => Ok(()),
        Some(Value::String(tag)) => Err(format!(
            "expected serialized {field} tag {expected}, but found {tag}"
        )),
        Some(_) => Err(format!("serialized {field} tag {expected} is not text")),
        None => Err(format!("serialized {field} tag {expected} is missing")),
    }
}

pub(super) fn project_nested_shape(
    value: &mut Value,
    projection: NestedProjection,
) -> Result<(), String> {
    match projection {
        NestedProjection::Unit(tag) => require_external_unit(value, tag),
        NestedProjection::Payload(tag) => external_payload_mut(value, tag).map(|_| ()),
    }
}

pub(super) fn deserialize_canonical<T>(value: Value, label: &str) -> Result<T, String>
where
    T: DeserializeOwned + Serialize,
{
    let parsed: T = serde_json::from_value(value.clone())
        .map_err(|error| format!("a serialized {label} is invalid: {error}"))?;
    let encoded = serde_json::to_value(&parsed).map_err(|error| error.to_string())?;
    if encoded != value {
        return Err(format!("a serialized {label} is not canonical"));
    }
    Ok(parsed)
}
