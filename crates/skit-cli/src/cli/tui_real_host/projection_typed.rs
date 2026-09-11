//! Typed action, effect, and screen projection.

use std::path::Path;

use serde_json::Value;
use skit_domain::StorageMode;
use skit_ui::{
    Action, AddAction, AddEffect, Effect, HealthAction, PreferencesAction, PreferencesEffect,
    RunnerEditorAction, RunnerManagerAction, Screen, SettingsAction,
};

use super::{
    projection::PathMap,
    projection_cause::{
        deserialize_canonical, external_payload_mut, external_tag, project_nested_shape,
        require_external_unit, require_internal_tag, review_default_name, screen_tag,
    },
    stable_sandbox::{names_the_same_path, ordinary_path},
};

enum TypedActionProjection<'a> {
    Unit(&'static str),
    Payload(&'static str),
    LibraryScan,
    LibrarySurface(&'static str),
    Complete,
    PromptRunnerRequired,
    AddCompleted,
    Status,
    RunnerEditorSaveFailed,
    RunnerManagerClosed,
    Present(&'a Screen),
    Add(&'a AddAction),
    Health(&'a HealthAction),
    Runners(&'a RunnerManagerAction),
    RunnerEditor(&'a RunnerEditorAction),
    Preferences(&'a PreferencesAction),
    Settings(&'a SettingsAction),
}

enum TypedEffectProjection<'a> {
    Unit(&'static str),
    Payload(&'static str),
    CountRunGlob,
    Add(&'a [AddEffect]),
    Preferences(&'a PreferencesEffect),
}

pub(super) enum NestedProjection {
    Unit(&'static str),
    Payload(&'static str),
}

enum AddActionProjection<'a> {
    Unit(&'static str),
    Payload(&'static str),
    PickedSourcePath(&'a str),
    SourceResult(&'static str),
    DraftEdited,
    DraftDeleted,
    CommitFinished,
}

enum AddEffectProjection {
    Unit(&'static str),
    Payload(&'static str),
    InspectSource,
    DeleteDraft,
    EditSource,
    Commit,
    ConsumeDraft,
    DraftKept,
}

impl PathMap {
    pub(super) fn project_typed_action(&mut self, action: &mut Action) -> Result<(), String> {
        let mut value = serde_json::to_value(&*action).map_err(|error| error.to_string())?;
        self.project_typed_action_value(&*action, &mut value)?;
        *action = deserialize_canonical(value, "Action")?;
        Ok(())
    }

    pub(super) fn project_typed_action_value(
        &mut self,
        action: &Action,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match action {
            Action::Previous => TypedActionProjection::Unit("previous"),
            Action::Next => TypedActionProjection::Unit("next"),
            Action::PagePrevious => TypedActionProjection::Unit("page_previous"),
            Action::PageNext => TypedActionProjection::Unit("page_next"),
            Action::Home => TypedActionProjection::Unit("home"),
            Action::End => TypedActionProjection::Unit("end"),
            Action::SelectVisible(_) => TypedActionProjection::Payload("select_visible"),
            Action::BeginSearch => TypedActionProjection::Unit("begin_search"),
            Action::FinishSearch => TypedActionProjection::Unit("finish_search"),
            Action::Input(_) => TypedActionProjection::Payload("input"),
            Action::Backspace => TypedActionProjection::Unit("backspace"),
            Action::Paste(_) => TypedActionProjection::Payload("paste"),
            Action::SetSearchQuery(_) => TypedActionProjection::Payload("set_search_query"),
            Action::ClearSearch => TypedActionProjection::Unit("clear_search"),
            Action::Replace { .. } => TypedActionProjection::LibraryScan,
            Action::ReplaceSurface { .. } => {
                TypedActionProjection::LibrarySurface("replace_surface")
            }
            Action::ReplaceRerunnable(_) => TypedActionProjection::Payload("replace_rerunnable"),
            Action::Reload => TypedActionProjection::Unit("reload"),
            Action::Rerun => TypedActionProjection::Unit("rerun"),
            Action::OpenRun => TypedActionProjection::Unit("open_run"),
            Action::OpenAdd => TypedActionProjection::Unit("open_add"),
            Action::OpenSettings => TypedActionProjection::Unit("open_settings"),
            Action::OpenPreferences => TypedActionProjection::Unit("open_preferences"),
            Action::OpenHealth => TypedActionProjection::Unit("open_health"),
            Action::OpenRunners => TypedActionProjection::Unit("open_runners"),
            Action::OpenPresets => TypedActionProjection::Unit("open_presets"),
            Action::OpenRename => TypedActionProjection::Unit("open_rename"),
            Action::Edit => TypedActionProjection::Unit("edit"),
            Action::AskRemove => TypedActionProjection::Unit("ask_remove"),
            Action::OpenHelp => TypedActionProjection::Unit("open_help"),
            Action::ToggleDetail { .. } => TypedActionProjection::Payload("toggle_detail"),
            Action::FocusNext => TypedActionProjection::Unit("focus_next"),
            Action::FocusPrevious => TypedActionProjection::Unit("focus_previous"),
            Action::FocusField(_) => TypedActionProjection::Payload("focus_field"),
            Action::SetFieldValue { .. } => TypedActionProjection::Payload("set_field_value"),
            Action::SetRunGlobCount { .. } => TypedActionProjection::Payload("set_run_glob_count"),
            Action::OpenRunPresetSave => TypedActionProjection::Unit("open_run_preset_save"),
            Action::OpenRunTokenMenu => TypedActionProjection::Unit("open_run_token_menu"),
            Action::OpenRunTokenMenuFor(_) => {
                TypedActionProjection::Payload("open_run_token_menu_for")
            }
            Action::OpenRunEnvironmentPicker(_) => {
                TypedActionProjection::Payload("open_run_environment_picker")
            }
            Action::SetRunEnvironmentQuery(_) => {
                TypedActionProjection::Payload("set_run_environment_query")
            }
            Action::OpenRunFilePicker(_) => TypedActionProjection::Payload("open_run_file_picker"),
            Action::OpenFocusedRunFilePicker => {
                TypedActionProjection::Unit("open_focused_run_file_picker")
            }
            Action::SetRunFieldValueAndCloseModal { .. } => {
                TypedActionProjection::Payload("set_run_field_value_and_close_modal")
            }
            Action::SetRunPickedPathAndCloseModal { .. } => {
                TypedActionProjection::Payload("set_run_picked_path_and_close_modal")
            }
            Action::ResetFocusedRunField => TypedActionProjection::Unit("reset_focused_run_field"),
            Action::OpenRunRunnerEditor => TypedActionProjection::Unit("open_run_runner_editor"),
            Action::Add(nested) => TypedActionProjection::Add(nested),
            Action::OpenAddRunnerEditor => TypedActionProjection::Unit("open_add_runner_editor"),
            Action::Health(nested) => TypedActionProjection::Health(nested),
            Action::Runners(nested) => TypedActionProjection::Runners(nested),
            Action::RunnerEditor(nested) => TypedActionProjection::RunnerEditor(nested),
            Action::RunnerEditorSaved { .. } => {
                TypedActionProjection::Payload("runner_editor_saved")
            }
            Action::RunnerEditorSaveFailed { .. } => TypedActionProjection::RunnerEditorSaveFailed,
            Action::RunnerManagerClosed { .. } => TypedActionProjection::RunnerManagerClosed,
            Action::Preferences(nested) => TypedActionProjection::Preferences(nested),
            Action::Settings(nested) => TypedActionProjection::Settings(nested),
            Action::PreferencesSaved { .. } => TypedActionProjection::Payload("preferences_saved"),
            Action::KeepEditing => TypedActionProjection::Unit("keep_editing"),
            Action::DiscardChanges => TypedActionProjection::Unit("discard_changes"),
            Action::SetModalInput(_) => TypedActionProjection::Payload("set_modal_input"),
            Action::RunPresetSaved { .. } => TypedActionProjection::Payload("run_preset_saved"),
            Action::ToggleField(_) => TypedActionProjection::Payload("toggle_field"),
            Action::SelectFieldOption { .. } => {
                TypedActionProjection::Payload("select_field_option")
            }
            Action::ResetRunField(_) => TypedActionProjection::Payload("reset_run_field"),
            Action::Submit => TypedActionProjection::Unit("submit"),
            Action::Back => TypedActionProjection::Unit("back"),
            Action::Present(screen) => TypedActionProjection::Present(screen),
            Action::PromptRunnerRequired { .. } => TypedActionProjection::PromptRunnerRequired,
            Action::AddCompleted { .. } => TypedActionProjection::AddCompleted,
            Action::AddCancelled => TypedActionProjection::Unit("add_cancelled"),
            Action::Complete { .. } => TypedActionProjection::Complete,
            Action::SetStatus(_) => TypedActionProjection::Status,
            Action::ClearStatus => TypedActionProjection::Unit("clear_status"),
            Action::Quit => TypedActionProjection::Unit("quit"),
        };
        match projection {
            TypedActionProjection::Unit(tag) => require_external_unit(value, tag)?,
            TypedActionProjection::Payload(tag) => {
                let _ = external_payload_mut(value, tag)?;
            }
            TypedActionProjection::LibraryScan => {
                let payload = external_payload_mut(value, "replace")?;
                self.normalize_library_scan(payload)?;
            }
            TypedActionProjection::LibrarySurface(tag) => {
                let payload = external_payload_mut(value, tag)?;
                self.normalize_library_surface(&mut payload["surface"])?;
            }
            TypedActionProjection::Complete => {
                let payload = external_payload_mut(value, "complete")?;
                if !payload["surface"].is_null() {
                    self.normalize_library_surface(&mut payload["surface"])?;
                }
            }
            TypedActionProjection::PromptRunnerRequired => {
                let payload = external_payload_mut(value, "prompt_runner_required")?;
                self.normalize_run_form(&mut payload["form"])?;
            }
            TypedActionProjection::AddCompleted => {
                let payload = external_payload_mut(value, "add_completed")?;
                self.normalize_library_surface(&mut payload["surface"])?;
            }
            TypedActionProjection::Status => {
                let _ = external_payload_mut(value, "set_status")?;
            }
            TypedActionProjection::RunnerEditorSaveFailed => {
                let payload = external_payload_mut(value, "runner_editor_save_failed")?;
                self.normalize_host_text_pointer(payload, "/message")?;
            }
            TypedActionProjection::RunnerManagerClosed => {
                let payload = external_payload_mut(value, "runner_manager_closed")?;
                self.normalize_preferences_view(&mut payload["preferences"])?;
            }
            TypedActionProjection::Present(screen) => {
                self.project_typed_screen(screen, external_payload_mut(value, "present")?)?;
            }
            TypedActionProjection::Add(nested) => {
                self.project_typed_add_action(nested, external_payload_mut(value, "add")?)?
            }
            TypedActionProjection::Health(nested) => {
                self.project_typed_health_action(nested, external_payload_mut(value, "health")?)?
            }
            TypedActionProjection::Runners(nested) => self.project_typed_runner_manager_action(
                nested,
                external_payload_mut(value, "runners")?,
            )?,
            TypedActionProjection::RunnerEditor(nested) => self
                .project_typed_runner_editor_action(
                    nested,
                    external_payload_mut(value, "runner_editor")?,
                )?,
            TypedActionProjection::Preferences(nested) => self.project_typed_preferences_action(
                nested,
                external_payload_mut(value, "preferences")?,
            )?,
            TypedActionProjection::Settings(nested) => self
                .project_typed_settings_action(nested, external_payload_mut(value, "settings")?)?,
        }
        Ok(())
    }

    pub(super) fn project_typed_effect(&mut self, effect: &mut Effect) -> Result<(), String> {
        let mut value = serde_json::to_value(&*effect).map_err(|error| error.to_string())?;
        self.project_typed_effect_value(&*effect, &mut value)?;
        *effect = deserialize_canonical(value, "Effect")?;
        Ok(())
    }

    pub(super) fn project_typed_effect_value(
        &mut self,
        effect: &Effect,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match effect {
            Effect::None => TypedEffectProjection::Unit("none"),
            Effect::Reload => TypedEffectProjection::Unit("reload"),
            Effect::Quit => TypedEffectProjection::Unit("quit"),
            Effect::Rerun { .. } => TypedEffectProjection::Payload("rerun"),
            Effect::Open { .. } => TypedEffectProjection::Payload("open"),
            Effect::Submit { .. } => TypedEffectProjection::Payload("submit"),
            Effect::CountRunGlob { .. } => TypedEffectProjection::CountRunGlob,
            Effect::SaveRunPreset { .. } => TypedEffectProjection::Payload("save_run_preset"),
            Effect::Add(nested) => TypedEffectProjection::Add(nested),
            Effect::HealthRebuild => TypedEffectProjection::Unit("health_rebuild"),
            Effect::SaveRunner { .. } => TypedEffectProjection::Payload("save_runner"),
            Effect::RemoveRunner(_) => TypedEffectProjection::Payload("remove_runner"),
            Effect::RefreshPreferencesAfterRunners => {
                TypedEffectProjection::Unit("refresh_preferences_after_runners")
            }
            Effect::Preferences(nested) => TypedEffectProjection::Preferences(nested),
            Effect::Edit { .. } => TypedEffectProjection::Payload("edit"),
            Effect::Remove { .. } => TypedEffectProjection::Payload("remove"),
        };
        match projection {
            TypedEffectProjection::Unit(tag) => require_external_unit(value, tag)?,
            TypedEffectProjection::Payload(tag) => {
                let _ = external_payload_mut(value, tag)?;
            }
            TypedEffectProjection::CountRunGlob => {
                let payload = external_payload_mut(value, "count_run_glob")?;
                self.normalize_path_pointer(payload, "/request/cwd")?;
            }
            TypedEffectProjection::Add(effects) => {
                let values = external_payload_mut(value, "add")?
                    .as_array_mut()
                    .ok_or("an Add Effect payload is not an array".to_owned())?;
                if effects.len() != values.len() {
                    return Err("an Add Effect payload changed its length".to_owned());
                }
                for (effect, value) in effects.iter().zip(values) {
                    self.project_typed_add_effect(effect, value)?;
                }
            }
            TypedEffectProjection::Preferences(nested) => self.project_typed_preferences_effect(
                nested,
                external_payload_mut(value, "preferences")?,
            )?,
        }
        Ok(())
    }

    pub(super) fn project_typed_screen(
        &mut self,
        screen: &Screen,
        value: &mut Value,
    ) -> Result<(), String> {
        let expected = screen_tag(screen);
        let actual = external_tag(value)?;
        if actual != expected {
            return Err(format!(
                "expected serialized Screen tag {expected}, but found {actual}"
            ));
        }
        self.project_serialized_screen(value)
    }

    pub(super) fn project_serialized_screen(&mut self, value: &mut Value) -> Result<(), String> {
        let tag = external_tag(value)?.to_owned();
        match tag.as_str() {
            "library" => require_external_unit(value, "library"),
            "run" => {
                let _ = external_payload_mut(value, "run")?;
                self.normalize_run_screen(value)
            }
            "preferences" => {
                let _ = external_payload_mut(value, "preferences")?;
                self.normalize_preferences_screen(value)
            }
            "add" => {
                let _ = external_payload_mut(value, "add")?;
                self.normalize_add_screen(value)
            }
            "health" => {
                let health = external_payload_mut(value, "health")?;
                self.normalize_health_view(health)
            }
            "runners" => {
                let runners = external_payload_mut(value, "runners")?;
                self.normalize_runner_manager_view(runners)
            }
            "settings" => {
                let _ = external_payload_mut(value, "settings")?;
                self.normalize_settings_screen(value);
                Ok(())
            }
            "form" => external_payload_mut(value, "form").map(|_| ()),
            "report" => external_payload_mut(value, "report").map(|_| ()),
            _ => Err(format!("unknown Screen tag: {tag}")),
        }
    }

    pub(super) fn project_typed_add_action(
        &mut self,
        action: &AddAction,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match action {
            AddAction::SetSourcePath(_) => AddActionProjection::Payload("set_source_path"),
            AddAction::PickedSourcePath(raw) => AddActionProjection::PickedSourcePath(raw),
            AddAction::SetCommandTemplate(_) => {
                AddActionProjection::Payload("set_command_template")
            }
            AddAction::SetCommandName(_) => AddActionProjection::Payload("set_command_name"),
            AddAction::SetCommandDescription(_) => {
                AddActionProjection::Payload("set_command_description")
            }
            AddAction::SelectDraft(_) => AddActionProjection::Payload("select_draft"),
            AddAction::HighlightDraft(_) => AddActionProjection::Payload("highlight_draft"),
            AddAction::Continue => AddActionProjection::Unit("continue"),
            AddAction::SourceInspected { .. } => {
                AddActionProjection::SourceResult("source_inspected")
            }
            AddAction::PickKind(_) => AddActionProjection::Payload("pick_kind"),
            AddAction::NewDraft(_) => AddActionProjection::Payload("new_draft"),
            AddAction::DraftEdited { .. } => AddActionProjection::DraftEdited,
            AddAction::DeleteSelectedDraft => AddActionProjection::Unit("delete_selected_draft"),
            AddAction::ConfirmDraftDelete(_) => {
                AddActionProjection::Payload("confirm_draft_delete")
            }
            AddAction::DraftDeleted { .. } => AddActionProjection::DraftDeleted,
            AddAction::SetReviewName(_) => AddActionProjection::Payload("set_review_name"),
            AddAction::SetReviewDescription(_) => {
                AddActionProjection::Payload("set_review_description")
            }
            AddAction::SetReviewStorage(_) => AddActionProjection::Payload("set_review_storage"),
            AddAction::SetReviewDependencies(_) => {
                AddActionProjection::Payload("set_review_dependencies")
            }
            AddAction::SetReviewPython(_) => AddActionProjection::Payload("set_review_python"),
            AddAction::SetReviewCandidate { .. } => {
                AddActionProjection::Payload("set_review_candidate")
            }
            AddAction::SetPromptInterpolation(_) => {
                AddActionProjection::Payload("set_prompt_interpolation")
            }
            AddAction::SetPromptCandidate { .. } => {
                AddActionProjection::Payload("set_prompt_candidate")
            }
            AddAction::SetPromptCandidates(_) => {
                AddActionProjection::Payload("set_prompt_candidates")
            }
            AddAction::SetPromptRunner { .. } => AddActionProjection::Payload("set_prompt_runner"),
            AddAction::PromptRunnerAdded(_) => AddActionProjection::Payload("prompt_runner_added"),
            AddAction::EditSource => AddActionProjection::Unit("edit_source"),
            AddAction::SourceEdited { .. } => AddActionProjection::SourceResult("source_edited"),
            AddAction::Save => AddActionProjection::Unit("save"),
            AddAction::CommitFinished { .. } => AddActionProjection::CommitFinished,
            AddAction::Cancel => AddActionProjection::Unit("cancel"),
        };
        match projection {
            AddActionProjection::Unit(tag) => require_external_unit(value, tag),
            AddActionProjection::Payload(tag) => external_payload_mut(value, tag).map(|_| ()),
            AddActionProjection::PickedSourcePath(raw) => {
                let payload = external_payload_mut(value, "picked_source_path")?;
                if payload.as_str() != Some(raw) {
                    return Err(
                        "the serialized picked Add source path does not match its typed action"
                            .to_owned(),
                    );
                }
                let path = self.require_exact_picker_file(raw)?;
                *payload = Value::String(self.normalize_path(path));
                Ok(())
            }
            AddActionProjection::SourceResult(tag) => {
                let payload = external_payload_mut(value, tag)?;
                if let Some(source) = payload.pointer_mut("/result/Ok") {
                    self.normalize_source_snapshot(source)?;
                }
                self.normalize_host_text_pointer(payload, "/result/Err")?;
                Ok(())
            }
            AddActionProjection::DraftEdited => {
                let payload = external_payload_mut(value, "draft_edited")?;
                if let Some(source) = payload.pointer_mut("/result/Ok")
                    && !source.is_null()
                {
                    self.normalize_source_snapshot(source)?;
                }
                self.normalize_host_text_pointer(payload, "/result/Err")?;
                Ok(())
            }
            AddActionProjection::DraftDeleted => {
                let payload = external_payload_mut(value, "draft_deleted")?;
                if let Some(draft) = payload.pointer_mut("/result/Ok/changed") {
                    self.normalize_draft_summary(draft)?;
                }
                self.normalize_host_text_pointer(payload, "/result/Err")?;
                Ok(())
            }
            AddActionProjection::CommitFinished => {
                let payload = external_payload_mut(value, "commit_finished")?;
                self.normalize_host_text_pointer(payload, "/result/Err")
            }
        }
    }

    fn project_typed_add_effect(
        &mut self,
        effect: &AddEffect,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match effect {
            AddEffect::InspectSource { .. } => AddEffectProjection::InspectSource,
            AddEffect::AuthorDraft { .. } => AddEffectProjection::Payload("author_draft"),
            AddEffect::DeleteDraft { .. } => AddEffectProjection::DeleteDraft,
            AddEffect::EditSource { .. } => AddEffectProjection::EditSource,
            AddEffect::Commit { .. } => AddEffectProjection::Commit,
            AddEffect::ConsumeDraft(_) => AddEffectProjection::ConsumeDraft,
            AddEffect::DraftKept(_) => AddEffectProjection::DraftKept,
            AddEffect::RememberRunner(_) => AddEffectProjection::Payload("remember_runner"),
            AddEffect::Complete(_) => AddEffectProjection::Payload("complete"),
            AddEffect::Cancel => AddEffectProjection::Unit("cancel"),
        };
        match projection {
            AddEffectProjection::Unit(tag) => require_external_unit(value, tag),
            AddEffectProjection::Payload(tag) => external_payload_mut(value, tag).map(|_| ()),
            AddEffectProjection::InspectSource => {
                let payload = external_payload_mut(value, "inspect_source")?;
                self.normalize_path_pointer(payload, "/path")
            }
            AddEffectProjection::DeleteDraft => {
                let payload = external_payload_mut(value, "delete_draft")?;
                self.normalize_draft_summary(&mut payload["draft"])?;
                Ok(())
            }
            AddEffectProjection::EditSource => {
                let payload = external_payload_mut(value, "edit_source")?;
                self.normalize_path_pointer(payload, "/path")
            }
            AddEffectProjection::Commit => {
                let payload = external_payload_mut(value, "commit")?;
                let paired_source = payload
                    .pointer("/source/source_record")
                    .and_then(Value::as_str)
                    .zip(payload.pointer("/entry/source").and_then(Value::as_str))
                    .filter(|(source, entry)| source == entry)
                    .map(|(source, _)| source.to_owned());
                self.project_commit_review_name(effect, payload)?;
                if !payload["source"].is_null() {
                    self.normalize_source_snapshot(&mut payload["source"])?;
                }
                if paired_source
                    .as_deref()
                    .is_some_and(|source| self.is_registered_path(Path::new(source)))
                {
                    self.normalize_path_pointer(payload, "/entry/source")?;
                }
                Ok(())
            }
            AddEffectProjection::ConsumeDraft => {
                let source = external_payload_mut(value, "consume_draft")?;
                self.normalize_source_snapshot(source)
            }
            AddEffectProjection::DraftKept => {
                let path = external_payload_mut(value, "draft_kept")?;
                self.normalize_path_value(path);
                Ok(())
            }
        }
    }

    fn project_commit_review_name(
        &self,
        effect: &AddEffect,
        payload: &mut Value,
    ) -> Result<(), String> {
        let Some(projection) = self.add_provenance.review_name.as_ref() else {
            return Ok(());
        };
        debug_assert!(
            !self.add_provenance.review_name_edited,
            "an edited Add review name retained derived provenance",
        );
        let Some(origin) = self.draft_paths.get(&projection.source_path) else {
            return Err("an Add Commit review-name source is not registered".to_owned());
        };
        if review_default_name(&projection.source_path, &projection.kind) != projection.raw_name
            || review_default_name(Path::new(&origin.stable_path), &projection.kind)
                != projection.stable_name
        {
            return Err("an Add Commit review-name projection is inconsistent".to_owned());
        }
        let Some(current_source) = self.add_provenance.review_source_path.as_ref() else {
            return Err("an Add Commit has no current review source provenance".to_owned());
        };
        if !self.draft_paths.contains_key(current_source) {
            return Err("an Add Commit review source is not registered".to_owned());
        }
        let AddEffect::Commit {
            entry,
            source: Some(source),
            ..
        } = effect
        else {
            return Err("a derived Add Commit has no source".to_owned());
        };
        if !source.is_draft
            || source.path != *current_source
            || !names_the_same_path(
                &source.source_record,
                &ordinary_path(&self.resolved_path(current_source)),
            )
            || !names_the_same_path(
                &entry.source,
                &ordinary_path(&self.resolved_path(current_source)),
            )
            || entry.mode != StorageMode::Copy
            || entry.kind.as_str() != projection.kind
            || entry.name != projection.raw_name
        {
            return Err(
                "an Add Commit does not match its derived review-name provenance".to_owned(),
            );
        }
        payload["entry"]["name"] = Value::String(projection.stable_name.clone());
        Ok(())
    }

    fn project_typed_health_action(
        &mut self,
        action: &HealthAction,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match action {
            HealthAction::Previous => NestedProjection::Unit("previous"),
            HealthAction::Next => NestedProjection::Unit("next"),
            HealthAction::PagePrevious(_) => NestedProjection::Payload("page_previous"),
            HealthAction::PageNext(_) => NestedProjection::Payload("page_next"),
            HealthAction::Home => NestedProjection::Unit("home"),
            HealthAction::End => NestedProjection::Unit("end"),
            HealthAction::SelectIssue(_) => NestedProjection::Payload("select_issue"),
            HealthAction::Jump => NestedProjection::Unit("jump"),
            HealthAction::ActivateIssue(_) => NestedProjection::Payload("activate_issue"),
            HealthAction::Rebuild => NestedProjection::Unit("rebuild"),
            HealthAction::Rebuilt { .. } => NestedProjection::Payload("rebuilt"),
            HealthAction::Back => NestedProjection::Unit("back"),
        };
        project_nested_shape(value, projection)?;
        if matches!(action, HealthAction::Rebuilt { .. }) {
            let payload = external_payload_mut(value, "rebuilt")?;
            self.normalize_health_rebuilt_action(payload)?;
        }
        Ok(())
    }

    fn project_typed_preferences_action(
        &mut self,
        action: &PreferencesAction,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match action {
            PreferencesAction::SetLanguage(_) => NestedProjection::Payload("set_language"),
            PreferencesAction::SetEditor(_) => NestedProjection::Payload("set_editor"),
            PreferencesAction::SetInteractiveForm(_) => {
                NestedProjection::Payload("set_interactive_form")
            }
            PreferencesAction::SetAfterRun(_) => NestedProjection::Payload("set_after_run"),
            PreferencesAction::SetJavascript(_) => NestedProjection::Payload("set_javascript"),
            PreferencesAction::SetBashPath(_) => NestedProjection::Payload("set_bash_path"),
            PreferencesAction::SetMirrorMaster(_) => NestedProjection::Payload("set_mirror_master"),
            PreferencesAction::ChooseMirror { .. } => NestedProjection::Payload("choose_mirror"),
            PreferencesAction::SetMirrorUrl { .. } => NestedProjection::Payload("set_mirror_url"),
            PreferencesAction::Focus(_) => NestedProjection::Payload("focus"),
            PreferencesAction::Previous => NestedProjection::Unit("previous"),
            PreferencesAction::Next => NestedProjection::Unit("next"),
            PreferencesAction::Save => NestedProjection::Unit("save"),
            PreferencesAction::Close => NestedProjection::Unit("close"),
            PreferencesAction::ManageAgents => NestedProjection::Unit("manage_agents"),
            PreferencesAction::InstallAgentSkill => NestedProjection::Unit("install_agent_skill"),
            PreferencesAction::PresentAgentSkillTargets(_) => {
                NestedProjection::Payload("present_agent_skill_targets")
            }
            PreferencesAction::SelectAgentSkillTarget(_) => {
                NestedProjection::Payload("select_agent_skill_target")
            }
            PreferencesAction::ActivateAgentSkillTarget(_) => {
                NestedProjection::Payload("activate_agent_skill_target")
            }
            PreferencesAction::ConfirmAgentSkillTarget => {
                NestedProjection::Unit("confirm_agent_skill_target")
            }
            PreferencesAction::CloseAgentSkillTargets => {
                NestedProjection::Unit("close_agent_skill_targets")
            }
            PreferencesAction::AgentSkillInstalled { .. } => {
                NestedProjection::Payload("agent_skill_installed")
            }
            PreferencesAction::ValidationFailed(_) => {
                NestedProjection::Payload("validation_failed")
            }
        };
        project_nested_shape(value, projection)?;
        match action {
            PreferencesAction::PresentAgentSkillTargets(_) => {
                let targets = external_payload_mut(value, "present_agent_skill_targets")?;
                self.normalize_path_pointer(targets, "/*/base")?;
            }
            PreferencesAction::AgentSkillInstalled { .. } => {
                let payload = external_payload_mut(value, "agent_skill_installed")?;
                let message = payload
                    .get_mut("message")
                    .expect("a serialized AgentSkillInstalled action has its message");
                let _ = self.normalize_agent_skill_install_text(message)?;
            }
            PreferencesAction::SetLanguage(_)
            | PreferencesAction::SetEditor(_)
            | PreferencesAction::SetInteractiveForm(_)
            | PreferencesAction::SetAfterRun(_)
            | PreferencesAction::SetJavascript(_)
            | PreferencesAction::SetBashPath(_)
            | PreferencesAction::SetMirrorMaster(_)
            | PreferencesAction::ChooseMirror { .. }
            | PreferencesAction::SetMirrorUrl { .. }
            | PreferencesAction::Focus(_)
            | PreferencesAction::Previous
            | PreferencesAction::Next
            | PreferencesAction::Save
            | PreferencesAction::Close
            | PreferencesAction::ManageAgents
            | PreferencesAction::InstallAgentSkill
            | PreferencesAction::SelectAgentSkillTarget(_)
            | PreferencesAction::ActivateAgentSkillTarget(_)
            | PreferencesAction::ConfirmAgentSkillTarget
            | PreferencesAction::CloseAgentSkillTargets
            | PreferencesAction::ValidationFailed(_) => {}
        }
        Ok(())
    }

    fn project_typed_preferences_effect(
        &mut self,
        effect: &PreferencesEffect,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match effect {
            PreferencesEffect::None => NestedProjection::Unit("none"),
            PreferencesEffect::Save(_) => NestedProjection::Payload("save"),
            PreferencesEffect::Close => NestedProjection::Unit("close"),
            PreferencesEffect::ConfirmDiscard => NestedProjection::Unit("confirm_discard"),
            PreferencesEffect::ManageAgents => NestedProjection::Unit("manage_agents"),
            PreferencesEffect::DiscoverAgentSkillTargets => {
                NestedProjection::Unit("discover_agent_skill_targets")
            }
            PreferencesEffect::InstallAgentSkill { .. } => {
                NestedProjection::Payload("install_agent_skill")
            }
        };
        project_nested_shape(value, projection)?;
        if matches!(effect, PreferencesEffect::InstallAgentSkill { .. }) {
            let payload = external_payload_mut(value, "install_agent_skill")?;
            self.normalize_path_pointer(payload, "/skills_dir")?;
        }
        Ok(())
    }

    fn project_typed_runner_editor_action(
        &mut self,
        action: &RunnerEditorAction,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match action {
            RunnerEditorAction::SetName(_) => NestedProjection::Payload("set_name"),
            RunnerEditorAction::SetCommand(_) => NestedProjection::Payload("set_command"),
            RunnerEditorAction::Focus(_) => NestedProjection::Payload("focus"),
            RunnerEditorAction::FocusNext => NestedProjection::Unit("focus_next"),
            RunnerEditorAction::FocusPrevious => NestedProjection::Unit("focus_previous"),
            RunnerEditorAction::Submit => NestedProjection::Unit("submit"),
            RunnerEditorAction::Cancel => NestedProjection::Unit("cancel"),
            RunnerEditorAction::MutationFailed(_) => NestedProjection::Payload("mutation_failed"),
        };
        project_nested_shape(value, projection)
    }

    fn project_typed_runner_manager_action(
        &mut self,
        action: &RunnerManagerAction,
        value: &mut Value,
    ) -> Result<(), String> {
        let projection = match action {
            RunnerManagerAction::Previous => NestedProjection::Unit("previous"),
            RunnerManagerAction::Next => NestedProjection::Unit("next"),
            RunnerManagerAction::PagePrevious(_) => NestedProjection::Payload("page_previous"),
            RunnerManagerAction::PageNext(_) => NestedProjection::Payload("page_next"),
            RunnerManagerAction::Home => NestedProjection::Unit("home"),
            RunnerManagerAction::End => NestedProjection::Unit("end"),
            RunnerManagerAction::Select(_) => NestedProjection::Payload("select"),
            RunnerManagerAction::ActivateSelected => NestedProjection::Unit("activate_selected"),
            RunnerManagerAction::ActivateRow(_) => NestedProjection::Payload("activate_row"),
            RunnerManagerAction::New => NestedProjection::Unit("new"),
            RunnerManagerAction::EditSelected => NestedProjection::Unit("edit_selected"),
            RunnerManagerAction::RemoveSelected => NestedProjection::Unit("remove_selected"),
            RunnerManagerAction::CloseActions => NestedProjection::Unit("close_actions"),
            RunnerManagerAction::Editor(_) => NestedProjection::Payload("editor"),
            RunnerManagerAction::CancelEditor => NestedProjection::Unit("cancel_editor"),
            RunnerManagerAction::ConfirmRemove => NestedProjection::Unit("confirm_remove"),
            RunnerManagerAction::CancelRemove => NestedProjection::Unit("cancel_remove"),
            RunnerManagerAction::MutationSucceeded { .. } => {
                NestedProjection::Payload("mutation_succeeded")
            }
            RunnerManagerAction::MutationFailed(_) => NestedProjection::Payload("mutation_failed"),
            RunnerManagerAction::Back => NestedProjection::Unit("back"),
        };
        if let RunnerManagerAction::Editor(editor) = action {
            let payload = external_payload_mut(value, "editor")?;
            return self.project_typed_runner_editor_action(editor, payload);
        }
        project_nested_shape(value, projection)?;
        if matches!(action, RunnerManagerAction::MutationFailed(_)) {
            let payload = external_payload_mut(value, "mutation_failed")?;
            self.normalize_host_text_value(payload);
        }
        Ok(())
    }

    fn project_typed_settings_action(
        &mut self,
        action: &SettingsAction,
        value: &mut Value,
    ) -> Result<(), String> {
        let tag = match action {
            SettingsAction::SetField { .. } => "set_field",
            SettingsAction::Focus { .. } => "focus",
            SettingsAction::FocusNext => "focus_next",
            SettingsAction::FocusPrevious => "focus_previous",
            SettingsAction::Resync => "resync",
            SettingsAction::SetPromptCandidates { .. } => "set_prompt_candidates",
            SettingsAction::Save => "save",
            SettingsAction::Close => "close",
            SettingsAction::NewRunner => "new_runner",
        };
        require_internal_tag(value, "action", tag)
    }
}
