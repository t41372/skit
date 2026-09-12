//! The path map: registration, path normalization, and state normalization.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use skit_application::LibraryService;
use skit_domain::Slug;
use skit_i18n::{Locale, format_text};
use skit_store::FileStore;
use skit_tui_walker_support::{
    canonical_json_bytes,
    leak_oracle::accept_host_path_token,
    projection::{replace_longest_text_tokens, rewrite_json_pointer_matches},
    sentinels::{AscendingRankAllocator, SourceIdentitySentinels},
};
use skit_ui::Action;

use super::{
    adapters::{AllocationOutcome, AllocationPurpose, PortEvent},
    observation::{
        ArtifactLeakOracleFact, ByteView, LeakOracleFacts, ModifiedLeakOraclePair,
        RendererDraftFact, RendererReviewNameFact, SourceIdentityLeakOraclePair, encode_hex,
        escaped_os, escaped_path, sorted_tui_drafts,
    },
    projection_cause::{decode_session_path_value, deserialize_canonical, draft_projection_suffix},
    stable_sandbox::ordinary_path,
};
use crate::cli::tui_host::{PrivateDirectoryPurpose, TemporaryFilePurpose};

#[derive(Debug)]
pub(super) struct PathMap {
    profile_root: PathBuf,
    resolved_profile_root: PathBuf,
    ambient_paths: BTreeSet<String>,
    pub(super) profile_label: String,
    pub(super) file_picker_files: BTreeSet<PathBuf>,
    entry_ids: BTreeMap<String, String>,
    entry_id_generations: BTreeMap<String, u64>,
    entry_generations: BTreeMap<Slug, u64>,
    pub(super) added_at: BTreeMap<String, String>,
    artifact_facts: BTreeMap<String, ArtifactLeakOracleFact>,
    pub(super) draft_paths: BTreeMap<PathBuf, DraftPathProjection>,
    pub(super) next_draft: usize,
    pub(super) transient_paths: BTreeMap<PathBuf, String>,
    next_transient: usize,
    pub(super) text_artifacts: BTreeMap<String, String>,
    pub(super) modified_ranks: AscendingRankAllocator,
    source_identities: SourceIdentitySentinels,
    pub(super) add_provenance: AddProjectionProvenance,
    pub(super) add_cause: AddProjectionCause,
    pub(super) status_provenance: Option<String>,
    pub(super) status_cause: StatusProjectionCause,
    pub(super) runner_failure_status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DraftPathProjection {
    pub(super) raw_path: PathBuf,
    pub(super) stable_path: String,
    raw_kind_picker_filename: String,
    pub(super) raw_file_picker_entry_name: String,
    pub(super) stable_basename: String,
    observed_review_names: BTreeSet<RendererReviewNameFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ReviewNameProjection {
    pub(super) source_path: PathBuf,
    pub(super) kind: String,
    pub(super) raw_name: String,
    pub(super) stable_name: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct AddProjectionProvenance {
    pub(super) selected_source_path: Option<PathBuf>,
    pub(super) picked_source_path: Option<String>,
    pub(super) review_source_path: Option<PathBuf>,
    pub(super) review_name: Option<ReviewNameProjection>,
    pub(super) review_name_yank: Option<ReviewNameProjection>,
    pub(super) review_name_edited: bool,
}

#[derive(Debug, Default)]
pub(super) enum AddProjectionCause {
    #[default]
    None,
    Reset,
    SelectDraft(usize),
    PickedSourcePath(String),
    SourcePathEdited,
    ReviewSource(Option<PathBuf>),
    ReviewNameEdited,
    ReviewSourceEdited(Option<PathBuf>),
}

#[derive(Debug, Default)]
pub(super) enum StatusProjectionCause {
    #[default]
    None,
    AgentSkillInstalled(String),
    Replaced,
}

pub(super) fn draft_source_path(source: &skit_ui::SourceSnapshot) -> Option<PathBuf> {
    source.is_draft.then(|| source.path.clone())
}

/// Keep both environment text and the spelling that filesystem operations return.
pub(super) fn ambient_path_spellings(paths: impl IntoIterator<Item = PathBuf>) -> BTreeSet<String> {
    let mut spellings = BTreeSet::new();
    for path in paths {
        if path.as_os_str().is_empty() {
            continue;
        }
        spellings.insert(path.display().to_string());
        if let Ok(resolved) = fs::canonicalize(path) {
            spellings.insert(resolved.display().to_string());
            spellings.insert(ordinary_path(&resolved).display().to_string());
        }
    }
    spellings
}

impl PathMap {
    pub(super) fn new(
        profile: &str,
        profile_root: &Path,
        service: &LibraryService<FileStore>,
        file_picker_files: &BTreeSet<PathBuf>,
    ) -> Result<Self, String> {
        let mut ambient_roots = vec![
            crate::cli::tui_walker_bundle::checkout_root()?,
            std::env::temp_dir(),
        ];
        ambient_roots.extend(std::env::current_dir());
        ambient_roots.extend(["HOME", "USERPROFILE"].into_iter().filter_map(|name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        }));
        let profile_label = format!("<profile:{profile}>");
        let mut text_artifacts = BTreeMap::new();
        text_artifacts.insert(profile_root.display().to_string(), profile_label.clone());
        let mut paths = Self {
            profile_root: profile_root.to_path_buf(),
            resolved_profile_root: fs::canonicalize(profile_root)
                .map_err(|error| error.to_string())?,
            ambient_paths: ambient_path_spellings(ambient_roots),
            profile_label,
            file_picker_files: file_picker_files.clone(),
            entry_ids: BTreeMap::new(),
            entry_id_generations: BTreeMap::new(),
            entry_generations: BTreeMap::new(),
            added_at: BTreeMap::new(),
            artifact_facts: BTreeMap::new(),
            draft_paths: BTreeMap::new(),
            next_draft: 0,
            transient_paths: BTreeMap::new(),
            next_transient: 0,
            text_artifacts,
            modified_ranks: AscendingRankAllocator::default(),
            source_identities: SourceIdentitySentinels::default(),
            add_provenance: AddProjectionProvenance::default(),
            add_cause: AddProjectionCause::default(),
            status_provenance: None,
            status_cause: StatusProjectionCause::default(),
            runner_failure_status: None,
        };
        paths.register_path_text(profile_root, paths.profile_label.clone());
        paths.artifact_facts.insert(
            paths.profile_label.clone(),
            ArtifactLeakOracleFact {
                raw_path_spellings: paths.text_artifacts.keys().cloned().collect(),
                ..ArtifactLeakOracleFact::default()
            },
        );
        paths.refresh(service)?;
        Ok(paths)
    }

    pub(super) fn leak_oracle_facts(&self) -> LeakOracleFacts {
        let renderer_drafts = self
            .draft_paths
            .values()
            .map(|draft| {
                (
                    draft.stable_path.clone(),
                    RendererDraftFact {
                        raw_path: draft.raw_path.display().to_string(),
                        projected_path: draft.stable_path.clone(),
                        raw_kind_picker_basename: draft.raw_kind_picker_filename.clone(),
                        raw_lossy_basename: draft.raw_file_picker_entry_name.clone(),
                        projected_basename: draft.stable_basename.clone(),
                        review_names: draft.observed_review_names.clone(),
                    },
                )
            })
            .collect();
        LeakOracleFacts {
            artifacts: self.artifact_facts.clone(),
            renderer_drafts,
            ambient_paths: self.ambient_paths.clone(),
        }
    }

    pub(super) fn refresh(&mut self, service: &LibraryService<FileStore>) -> Result<(), String> {
        let mut entries = service
            .repository()
            .scan_entries()
            .map_err(|error| error.to_string())?;
        entries.sort_by(|left, right| left.slug.cmp(&right.slug));
        for entry in entries {
            let generation = if let Some(id) = entry.meta.id.as_ref() {
                let raw = id.as_str();
                if let Some(generation) = self.entry_id_generations.get(raw) {
                    *generation
                } else {
                    let generation = self
                        .entry_generations
                        .entry(entry.slug.clone())
                        .or_default();
                    *generation += 1;
                    let token = generation_token("entry-id", entry.slug.as_str(), *generation);
                    self.entry_ids.insert(raw.to_owned(), token);
                    self.entry_id_generations
                        .insert(raw.to_owned(), *generation);
                    *generation
                }
            } else {
                *self
                    .entry_generations
                    .entry(entry.slug.clone())
                    .or_insert(1)
            };
            if !entry.meta.added_at.is_empty() && !self.added_at.contains_key(&entry.meta.added_at)
            {
                self.added_at.insert(
                    entry.meta.added_at,
                    generation_token("added-at", entry.slug.as_str(), generation),
                );
            }
        }
        let drafts = sorted_tui_drafts(service.repository().data_dir());
        let mut modified = drafts
            .iter()
            .map(|draft| draft.modified)
            .collect::<Vec<_>>();
        modified.sort_unstable();
        self.modified_ranks.assign_sorted(&modified)?;
        for draft in drafts {
            if self.draft_paths.contains_key(&draft.path) {
                continue;
            }
            self.register_draft_path(&draft.path);
        }
        Ok(())
    }

    pub(super) fn register_event_artifacts(&mut self, event: &PortEvent) {
        match event {
            PortEvent::Allocation {
                purpose,
                path: Some(path),
                outcome: AllocationOutcome::Accepted,
                ..
            } => {
                match purpose {
                    AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft) => {
                        self.register_draft_path(path)
                    }
                    AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource) => {
                        self.register_owned_transient_path(path, "injected-source", ".injected-")
                    }
                    AllocationPurpose::PrivateDirectory(
                        PrivateDirectoryPurpose::DraftQuarantine,
                    ) => {
                        self.register_owned_transient_path(path, "quarantine", ".skit-quarantine-")
                    }
                }
                self.register_text_path(path);
            }
            PortEvent::Editor { path, .. } => {
                self.register_text_path(path);
            }
            PortEvent::Launch {
                plan,
                readable_files,
                ..
            } => {
                self.register_text_path(&plan.program);
                self.register_text_path(&plan.cwd);
                for argument in &plan.args {
                    self.register_text_path(Path::new(argument));
                }
                for (path, _) in readable_files {
                    self.register_text_path(path);
                }
            }
            PortEvent::Dependency { command, .. } => {
                self.register_text_path(&command.program);
                self.register_text_path(&command.cwd);
                for argument in &command.args {
                    self.register_text_path(Path::new(argument));
                }
            }
            PortEvent::Injected {
                command,
                readable_files,
                ..
            } => {
                self.register_text_path(&command.program);
                for argument in &command.args {
                    self.register_text_path(Path::new(argument));
                }
                for (path, _) in readable_files {
                    self.register_owned_transient_path(path, "injected-source", ".injected-");
                    self.register_text_path(path);
                }
            }
            PortEvent::JavaScriptGate {
                program, source, ..
            } => {
                self.register_owned_transient_path(source, "injected-source", ".injected-");
                self.register_text_path(program);
                self.register_text_path(source);
            }
            PortEvent::Probe { path, .. }
            | PortEvent::Preference { path, .. }
            | PortEvent::UvConsent {
                destination: path, ..
            } => self.register_text_path(path),
            PortEvent::Clock(_)
            | PortEvent::Platform(_)
            | PortEvent::EnvironmentVariable { .. }
            | PortEvent::EnvironmentSnapshot(_)
            | PortEvent::LocalOffset(_)
            | PortEvent::SystemLocale(_)
            | PortEvent::Terminal { .. }
            | PortEvent::Allocation { .. }
            | PortEvent::UvFetch { .. }
            | PortEvent::Output { .. } => {}
        }
    }

    /// Keep the root's resolved spelling even after an owned source is removed.
    pub(super) fn resolved_path<'p>(&self, path: &'p Path) -> Cow<'p, Path> {
        match path.strip_prefix(&self.profile_root) {
            Ok(relative) if relative.as_os_str().is_empty() => {
                Cow::Owned(self.resolved_profile_root.clone())
            }
            Ok(relative) => Cow::Owned(self.resolved_profile_root.join(relative)),
            Err(_) => Cow::Borrowed(path),
        }
    }

    fn fixture_path<'p>(&self, path: &'p Path) -> Cow<'p, Path> {
        let ordinary = ordinary_path(path);
        match ordinary.strip_prefix(ordinary_path(&self.resolved_profile_root)) {
            Ok(relative) => Cow::Owned(self.profile_root.join(relative)),
            Err(_) => ordinary,
        }
    }

    fn register_path_text(&mut self, path: &Path, stable: String) {
        let resolved = self.resolved_path(path).into_owned();
        for spelling in [path, &resolved, ordinary_path(&resolved).as_ref()] {
            self.text_artifacts
                .insert(spelling.display().to_string(), stable.clone());
        }
    }

    pub(super) fn register_draft_path(&mut self, path: &Path) {
        if self.draft_paths.contains_key(path) {
            return;
        }
        let extension = draft_projection_suffix(path);
        let stable = format!(
            "{}/data/.drafts/<draft:{}>{extension}",
            self.profile_label, self.next_draft
        );
        self.next_draft += 1;
        let raw_kind_picker_filename = path
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_owned();
        let stable_basename = Path::new(&stable)
            .file_name()
            .map(escaped_os)
            .unwrap_or_default();
        let projection = DraftPathProjection {
            raw_path: path.to_path_buf(),
            stable_path: stable.clone(),
            raw_kind_picker_filename,
            raw_file_picker_entry_name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            stable_basename,
            observed_review_names: BTreeSet::new(),
        };
        self.draft_paths.insert(path.to_path_buf(), projection);
        self.register_path_text(path, stable);
    }

    fn register_text_path(&mut self, path: &Path) {
        if !self.is_registered_path(path) || path == self.profile_root {
            return;
        }
        let raw = path.display().to_string();
        if self.text_artifacts.contains_key(&raw) {
            return;
        }
        let stable = self.normalize_path(path);
        self.register_path_text(path, stable);
    }

    pub(super) fn register_owned_transient_path(&mut self, path: &Path, kind: &str, prefix: &str) {
        if self.transient_paths.contains_key(path) {
            return;
        }
        let Some(name) = path.file_name().map(escaped_os) else {
            return;
        };
        if !name.starts_with(prefix) {
            return;
        }
        let extension = path
            .extension()
            .map(|value| format!(".{}", escaped_os(value)))
            .unwrap_or_default();
        let parent = path.parent().expect("a path with a file name has a parent");
        let parent = if parent.starts_with(&self.profile_root) {
            self.normalize_path(parent)
        } else {
            "<transient>".to_owned()
        };
        let stable = format!("{parent}/<{kind}:{}>{extension}", self.next_transient);
        self.next_transient += 1;
        self.transient_paths
            .insert(path.to_path_buf(), stable.clone());
        self.register_path_text(path, stable);
    }

    pub(super) fn normalize_library_json(&mut self, mut value: Value) -> Result<Value, String> {
        self.normalize_library_surface(&mut value)?;
        self.normalize_workflow_artifacts(&mut value)?;
        self.normalize_modal_artifacts(&mut value)?;
        self.normalize_install_completion_status(&mut value)?;
        Ok(value)
    }

    pub(super) fn normalize_library_surface(&mut self, value: &mut Value) -> Result<(), String> {
        self.normalize_library_details(value);
        self.normalize_library_scan(value)
    }

    pub(super) fn normalize_library_scan(&self, value: &mut Value) -> Result<(), String> {
        self.normalize_entry_summary_targets(value);
        for pointer in ["/diagnostics/*/message", "/scan/diagnostics/*/message"] {
            self.normalize_host_text_pointer(value, pointer)?;
        }
        Ok(())
    }

    pub(super) fn normalize_path_pointer(
        &self,
        value: &mut Value,
        pointer: &str,
    ) -> Result<(), String> {
        rewrite_json_pointer_matches(value, pointer, |matched| {
            self.normalize_path_value(matched);
            Ok(())
        })
    }

    pub(super) fn normalize_session_path_value(&self, value: &mut Value) -> Result<(), String> {
        let path = decode_session_path_value(value)?;
        if self.is_registered_path(&path) {
            *value = Value::String(self.normalize_path(&path));
        }
        Ok(())
    }

    pub(super) fn normalize_host_text_pointer(
        &self,
        value: &mut Value,
        pointer: &str,
    ) -> Result<(), String> {
        rewrite_json_pointer_matches(value, pointer, |matched| {
            self.normalize_host_text_value(matched);
            Ok(())
        })
    }

    pub(super) fn normalize_host_text_value(&self, value: &mut Value) {
        if let Value::String(text) = value {
            *text = self.normalize_host_text(text);
        }
    }

    pub(super) fn normalize_action_json(&mut self, mut value: Value) -> Result<Value, String> {
        let mut action: Action = deserialize_canonical(value, "Action")?;
        self.project_typed_action(&mut action)?;
        value = serde_json::to_value(action).map_err(|error| error.to_string())?;
        Ok(value)
    }

    pub(super) fn normalize_draft_list(&mut self, mut value: Value) -> Result<Value, String> {
        if let Some(drafts) = value.as_array_mut() {
            for draft in drafts {
                self.normalize_artifact_object(draft)?;
            }
        }
        Ok(value)
    }

    fn normalize_library_details(&self, value: &mut Value) {
        let Some(details) = value.get_mut("details").and_then(Value::as_object_mut) else {
            return;
        };
        for detail in details.values_mut().filter_map(Value::as_object_mut) {
            if let Some(Value::String(added_at)) = detail.get_mut("added_at")
                && let Some(stable) = self.added_at.get(added_at)
            {
                *added_at = stable.clone();
            }
            if let Some(missing_target) = detail.get_mut("missing_target") {
                self.normalize_path_value(missing_target);
            }
        }
    }

    fn normalize_entry_summary_targets(&self, value: &mut Value) {
        if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
            self.normalize_summary_array(entries);
        }
        if let Some(entries) = value
            .get_mut("scan")
            .and_then(|scan| scan.get_mut("entries"))
            .and_then(Value::as_array_mut)
        {
            self.normalize_summary_array(entries);
        }
    }

    fn normalize_summary_array(&self, entries: &mut [Value]) {
        for entry in entries.iter_mut().filter_map(Value::as_object_mut) {
            if entry.contains_key("slug")
                && entry.contains_key("kind")
                && let Some(Value::String(target)) = entry.get_mut("target")
            {
                *target = self.normalize_registered_path_text(target);
            }
        }
    }

    fn normalize_workflow_artifacts(&mut self, value: &mut Value) -> Result<(), String> {
        let Some(workflow) = value.get_mut("workflow").and_then(Value::as_object_mut) else {
            return Ok(());
        };
        if let Some(active) = workflow.get_mut("active") {
            self.project_serialized_screen(active)?;
        }
        if let Some(history) = workflow.get_mut("history").and_then(Value::as_array_mut) {
            for screen in history {
                self.project_serialized_screen(screen)?;
            }
        }
        Ok(())
    }

    pub(super) fn normalize_run_screen(&self, screen: &mut Value) -> Result<(), String> {
        let Some(run) = screen.get_mut("run") else {
            return Ok(());
        };
        self.normalize_run_form(run)
    }

    pub(super) fn normalize_run_form(&self, run: &mut Value) -> Result<(), String> {
        for pointer in [
            "/context/path/workdir",
            "/context/path/invoke_cwd",
            "/context/tokens/cwd",
            "/context/tokens/home",
            "/context/tokens/env/*",
        ] {
            self.normalize_path_pointer(run, pointer)?;
        }
        self.normalize_host_text_pointer(run, "/fields/*/feedback/expanded")
    }

    pub(super) fn normalize_settings_screen(&self, screen: &mut Value) {
        let Some(sections) = screen
            .get_mut("settings")
            .and_then(|settings| settings.get_mut("sections"))
            .and_then(Value::as_array_mut)
        else {
            return;
        };
        for item in sections
            .iter_mut()
            .filter_map(|section| section.get_mut("items"))
            .filter_map(Value::as_array_mut)
            .flat_map(|items| items.iter_mut())
        {
            let is_source_note = item.get("item").and_then(Value::as_str) == Some("note")
                && item
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| {
                        matches!(
                            text,
                            "Linked to the original: {}"
                                | "Keep a copy — your original file is never modified. Source: {}"
                        )
                    });
            if !is_source_note {
                continue;
            }
            if let Some(arguments) = item.get_mut("arguments").and_then(Value::as_array_mut) {
                for argument in arguments {
                    self.normalize_path_value(argument);
                }
            }
        }
    }

    pub(super) fn normalize_preferences_screen(&self, screen: &mut Value) -> Result<(), String> {
        let Some(preferences) = screen.get_mut("preferences") else {
            return Ok(());
        };
        self.normalize_preferences_view(preferences)
    }

    pub(super) fn normalize_preferences_view(&self, preferences: &mut Value) -> Result<(), String> {
        self.normalize_path_pointer(preferences, "/agent_skill_install/targets/*/base")
    }

    pub(super) fn normalize_health_view(&self, health: &mut Value) -> Result<(), String> {
        for pointer in ["/snapshot/library_path", "/snapshot/uv/found"] {
            self.normalize_path_pointer(health, pointer)?;
        }
        for pointer in [
            "/snapshot/issues/*/kind/launch_blocked/reason",
            "/snapshot/diagnostics/*",
            "/rebuilt/problems/*",
        ] {
            self.normalize_host_text_pointer(health, pointer)?;
        }
        Ok(())
    }

    pub(super) fn normalize_health_rebuilt_action(
        &self,
        payload: &mut Value,
    ) -> Result<(), String> {
        for pointer in ["/snapshot/library_path", "/snapshot/uv/found"] {
            self.normalize_path_pointer(payload, pointer)?;
        }
        for pointer in [
            "/snapshot/issues/*/kind/launch_blocked/reason",
            "/snapshot/diagnostics/*",
            "/outcome/problems/*",
        ] {
            self.normalize_host_text_pointer(payload, pointer)?;
        }
        Ok(())
    }

    pub(super) fn normalize_runner_manager_view(&self, runners: &mut Value) -> Result<(), String> {
        if runners.pointer("/status").and_then(Value::as_str)
            == self.runner_failure_status.as_deref()
            && self.runner_failure_status.is_some()
        {
            self.normalize_host_text_pointer(runners, "/status")?;
        }
        self.normalize_host_text_pointer(runners, "/overlay/editor/host_error")
    }

    fn normalize_modal_artifacts(&self, value: &mut Value) -> Result<(), String> {
        let Some(modal) = value.get_mut("modal") else {
            return Ok(());
        };
        if let Some(file_picker) = modal.get_mut("run_file_picker") {
            for pointer in ["/context/workdir", "/context/invoke_cwd"] {
                self.normalize_path_pointer(file_picker, pointer)?;
            }
        }
        if let Some(options) = modal
            .get_mut("run_token_menu")
            .and_then(|menu| menu.get_mut("options"))
            .and_then(Value::as_array_mut)
        {
            for option in options {
                self.normalize_path_pointer(option, "/fixed_directory/path")?;
            }
        }
        self.normalize_host_text_pointer(modal, "/runner_editor/view/host_error")?;
        Ok(())
    }

    fn normalize_install_completion_status(&self, value: &mut Value) -> Result<(), String> {
        let Some(status) = value.get_mut("status") else {
            return Ok(());
        };
        if let Some(provenance) = self.status_provenance.as_deref() {
            if status.as_str() != Some(provenance) {
                return Err("the proven Agent Skill install status changed".to_owned());
            }
            if !self.normalize_agent_skill_install_text(status)? {
                return Err(
                    "the proven Agent Skill install status is not a complete template".to_owned(),
                );
            }
        }
        Ok(())
    }

    pub(super) fn normalize_agent_skill_install_text(
        &self,
        value: &mut Value,
    ) -> Result<bool, String> {
        let Value::String(status) = value else {
            return Ok(false);
        };
        const MARKER: &str = "<walker-agent-skill-path>";
        let mut fragments = Vec::new();
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            let rendered = format_text(locale, "Installed the skit Agent Skill: {}", &[&MARKER]);
            let (prefix, suffix) = rendered
                .split_once(MARKER)
                .expect("the Agent Skill completion template contains its path argument");
            fragments.push((prefix.to_owned(), suffix.to_owned()));
            let Some(path) = status
                .strip_prefix(prefix)
                .and_then(|remaining| remaining.strip_suffix(suffix))
            else {
                continue;
            };
            let normalized = self.normalize_registered_path_text(path);
            if normalized != path {
                *status = format!("{prefix}{normalized}{suffix}");
            } else if self.contains_raw_text_artifact(path) {
                return Err(
                    "an Agent Skill install message has text outside its one path slot".to_owned(),
                );
            }
            return Ok(true);
        }
        let has_template_fragment = fragments.iter().any(|(prefix, suffix)| {
            (!prefix.is_empty() && status.contains(prefix))
                || (!suffix.is_empty() && status.contains(suffix))
        });
        if has_template_fragment && self.contains_raw_text_artifact(status) {
            return Err("an Agent Skill install message is not a complete template".to_owned());
        }
        Ok(false)
    }

    fn contains_raw_text_artifact(&self, text: &str) -> bool {
        self.text_artifacts
            .keys()
            .any(|raw| !raw.is_empty() && text.contains(raw))
    }

    pub(super) fn normalize_path_value(&self, value: &mut Value) {
        if let Value::String(path) = value {
            *path = self.normalize_registered_path_text(path);
        }
    }

    pub(super) fn normalize_add_screen(&mut self, screen: &mut Value) -> Result<(), String> {
        let Some(add) = screen.get_mut("add") else {
            return Ok(());
        };
        self.normalize_add_derived_fields(add)?;
        if let Some(drafts) = add
            .get_mut("source")
            .and_then(|source| source.get_mut("drafts"))
            .and_then(Value::as_array_mut)
        {
            for draft in drafts {
                self.normalize_draft_summary(draft)?;
            }
        }
        for pointer in ["/pending_source", "/review/source", "/delete_candidate"] {
            if let Some(artifact) = add.pointer_mut(pointer) {
                if pointer == "/delete_candidate" {
                    self.normalize_draft_summary(artifact)?;
                } else {
                    self.normalize_source_snapshot(artifact)?;
                }
            }
        }
        if let Some(draft) = add
            .get_mut("pending_delete")
            .and_then(Value::as_array_mut)
            .and_then(|tuple| tuple.get_mut(1))
        {
            self.normalize_draft_summary(draft)?;
        }
        for pointer in [
            "/problem/source_unavailable/path",
            "/problem/draft_changed/path",
            "/notice/draft_kept",
            "/notice/draft_deleted",
        ] {
            self.normalize_path_pointer(add, pointer)?;
        }
        for pointer in [
            "/problem/source_unavailable/reason",
            "/problem/source_edit/reason",
            "/problem/commit_failed/reason",
            "/problem/edit_failed/reason",
            "/problem/draft_delete_failed/reason",
        ] {
            self.normalize_host_text_pointer(add, pointer)?;
        }
        Ok(())
    }

    fn normalize_add_derived_fields(&mut self, add: &mut Value) -> Result<(), String> {
        if let Some((raw, stable)) = self.add_source_path_projection()?
            && add.pointer("/source/path").and_then(Value::as_str) == Some(raw.as_str())
        {
            add["source"]["path"] = Value::String(stable);
        }
        if let Some(raw_path) = add
            .pointer("/pending_source/path")
            .and_then(Value::as_str)
            .map(Path::new)
            && let Some(projection) = self.draft_paths.get(raw_path)
            && let Some(kind_picker) = add.get_mut("kind_picker").filter(|value| !value.is_null())
        {
            let filename = kind_picker
                .get("filename")
                .and_then(Value::as_str)
                .ok_or("an Add kind-picker filename is not text".to_owned())?;
            if filename != projection.raw_kind_picker_filename {
                return Err("an Add kind-picker filename does not match its draft path".to_owned());
            }
            kind_picker["filename"] = Value::String(projection.stable_basename.clone());
        }
        if let Some(projection) = self.add_provenance.review_name.clone() {
            if !self.draft_paths.contains_key(&projection.source_path) {
                return Err("an Add review-name source path is not registered".to_owned());
            }
            let name = add
                .pointer_mut("/review/name")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or("an Add review name is not text".to_owned())?;
            if name != projection.raw_name {
                return Err("an Add review name does not match its draft default".to_owned());
            }
            add["review"]["name"] = Value::String(projection.stable_name.clone());
            let draft = self
                .draft_paths
                .get_mut(&projection.source_path)
                .expect("the review-name source path was checked");
            draft.observed_review_names.insert(RendererReviewNameFact {
                kind: projection.kind,
                raw_source_path: projection.source_path.display().to_string(),
                projected_source_path: draft.stable_path.clone(),
                raw_name: projection.raw_name,
                projected_name: projection.stable_name,
            });
        }
        Ok(())
    }

    pub(super) fn normalize_source_snapshot(&mut self, value: &mut Value) -> Result<(), String> {
        self.normalize_artifact_common(value)?;
        if let Some(source_record) = value.get_mut("source_record") {
            self.normalize_path_value(source_record);
        }
        Ok(())
    }

    pub(super) fn normalize_draft_summary(&mut self, value: &mut Value) -> Result<(), String> {
        let registered = self.normalize_artifact_common(value)?;
        if !registered {
            return Ok(());
        }
        let Some(modified) = value.get("modified").and_then(Value::as_u64) else {
            return Ok(());
        };
        let rank = self.modified_ranks.lookup(modified).ok_or_else(|| {
            format!(
                "the sorted scan did not assign a rank to this draft modified value: {modified}"
            )
        })?;
        value["modified"] = Value::from(rank);
        let projected_path = value
            .get("path")
            .and_then(Value::as_str)
            .expect("a registered artifact retains its projected text path")
            .to_owned();
        self.artifact_facts
            .entry(projected_path)
            .or_default()
            .modified_values
            .insert(ModifiedLeakOraclePair {
                raw: modified,
                projected: rank,
            });
        Ok(())
    }

    fn normalize_artifact_common(&mut self, value: &mut Value) -> Result<bool, String> {
        let Some(object) = value.as_object_mut() else {
            return Ok(false);
        };
        let Some(raw_path) = object
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .filter(|path| self.is_registered_path(Path::new(path)))
        else {
            return Ok(false);
        };
        let path = PathBuf::from(&raw_path);
        let stable_path = self.normalize_path(&path);
        let identity_projection = object
            .get("identity")
            .filter(|identity| !identity.is_null())
            .map(|identity| {
                let raw = canonical_json_bytes(identity)
                    .expect("a serde_json Value has canonical JSON bytes");
                let projected = self.source_identities.construct(identity)?;
                let projected_bytes = canonical_json_bytes(&projected)
                    .expect("a serde_json Value has canonical JSON bytes");
                Ok::<_, String>((projected, raw, projected_bytes))
            })
            .transpose()?;
        if let Some(Value::String(stored)) = object.get_mut("path") {
            *stored = stable_path.clone();
        }
        if let Some((projected, _, _)) = identity_projection.as_ref()
            && let Some(identity) = object.get_mut("identity")
        {
            *identity = projected.clone();
        }
        let facts = self.artifact_facts.entry(stable_path).or_default();
        facts.raw_path_spellings.insert(raw_path);
        if let Some((_, raw, projected)) = identity_projection {
            facts
                .source_identities
                .insert(SourceIdentityLeakOraclePair { raw, projected });
        }
        Ok(true)
    }

    pub(super) fn normalize_artifact_object(&mut self, value: &mut Value) -> Result<(), String> {
        if value.get("modified").is_some() {
            self.normalize_draft_summary(value)
        } else {
            self.normalize_source_snapshot(value)
        }
    }

    /// Whether this fixture owns one path.
    ///
    /// A path that the product recorded can arrive in the Windows verbatim form, so the lookup
    /// uses the ordinary spelling of it.
    pub(super) fn is_registered_path(&self, path: &Path) -> bool {
        let ordinary = self.fixture_path(path);
        self.draft_paths.contains_key(&*ordinary)
            || self.transient_paths.contains_key(&*ordinary)
            || ordinary.starts_with(&self.profile_root)
    }

    fn normalize_registered_path_text(&self, text: &str) -> String {
        let path = Path::new(text);
        if self.is_registered_path(path) {
            self.normalize_path(path)
        } else {
            text.to_owned()
        }
    }

    pub(super) fn normalize_argument(&self, text: &str) -> String {
        self.normalize_registered_path_text(text)
    }

    pub(super) fn normalize_host_text(&self, text: &str) -> String {
        replace_longest_text_tokens(text, &self.text_artifacts, accept_host_path_token)
    }

    pub(super) fn normalize_os_argument(&self, value: &OsStr) -> String {
        let path = Path::new(value);
        if self.is_registered_path(path) {
            self.normalize_path(path)
        } else {
            escaped_os(value)
        }
    }

    /// Get the stable text of one path.
    ///
    /// A path that the product recorded can arrive in the Windows verbatim form, so the lookup
    /// uses the ordinary spelling of it. A path that this fixture does not own keeps its exact
    /// spelling.
    pub(super) fn normalize_path(&self, path: &Path) -> String {
        let ordinary = self.fixture_path(path);
        if let Some(projection) = self.draft_paths.get(&*ordinary) {
            return projection.stable_path.clone();
        }
        if let Some(stable) = self.transient_paths.get(&*ordinary) {
            return stable.clone();
        }
        if let Ok(relative) = ordinary.strip_prefix(&self.profile_root) {
            let suffix = escaped_path(relative);
            if suffix.is_empty() {
                return self.profile_label.clone();
            }
            return format!("{}/{suffix}", self.profile_label);
        }
        escaped_os(path.as_os_str())
    }

    pub(super) fn byte_view(&self, path: &Path, bytes: &[u8]) -> ByteView {
        let registry = path == self.profile_root.join("data/registry.toml");
        let entry_meta = path.file_name() == Some(OsStr::new("meta.toml"))
            && path
                .parent()
                .and_then(Path::parent)
                .is_some_and(|parent| parent == self.profile_root.join("data/scripts"));
        let text = std::str::from_utf8(bytes).ok().map(|text| {
            if registry {
                self.normalize_registry_document(text)
            } else if entry_meta {
                self.normalize_entry_meta_document(text)
            } else {
                text.to_owned()
            }
        });
        match text {
            Some(text)
                if text
                    .chars()
                    .all(|character| !character.is_control() || "\n\r\t".contains(character)) =>
            {
                ByteView::Utf8(text)
            }
            Some(text) => ByteView::Hex(encode_hex(text.as_bytes())),
            None => ByteView::Hex(encode_hex(bytes)),
        }
    }

    fn normalize_entry_meta_document(&self, text: &str) -> String {
        let Ok(mut document) = text.parse::<toml_edit::DocumentMut>() else {
            return text.to_owned();
        };
        if let Some(id) = document.get("id").and_then(toml_edit::Item::as_str)
            && let Some(stable) = self.entry_ids.get(id)
        {
            document["id"] = toml_edit::value(stable);
        }
        if let Some(added_at) = document.get("added_at").and_then(toml_edit::Item::as_str)
            && let Some(stable) = self.added_at.get(added_at)
        {
            document["added_at"] = toml_edit::value(stable);
        }
        if let Some(source) = document.get("source").and_then(toml_edit::Item::as_str) {
            let stable = self.normalize_registered_path_text(source);
            if stable != source {
                document["source"] = toml_edit::value(stable);
            }
        }
        document.to_string()
    }

    fn normalize_registry_document(&self, text: &str) -> String {
        let Ok(mut document) = text.parse::<toml_edit::DocumentMut>() else {
            return text.to_owned();
        };
        let Some(entries) = document
            .get_mut("entries")
            .and_then(toml_edit::Item::as_table_mut)
        else {
            return text.to_owned();
        };
        for (_, row) in entries.iter_mut() {
            let Some(row) = row.as_table_mut() else {
                continue;
            };
            if row
                .get("mtime_ns")
                .and_then(toml_edit::Item::as_integer)
                .is_some()
            {
                row["mtime_ns"] = toml_edit::value("<mtime-ns>");
            }
            if let Some(target) = row.get("target").and_then(toml_edit::Item::as_str) {
                let stable = self.normalize_registered_path_text(target);
                if stable != target {
                    row["target"] = toml_edit::value(stable);
                }
            }
            let Some(cache) = row
                .get_mut("skit_cache")
                .and_then(toml_edit::Item::as_table_mut)
            else {
                continue;
            };
            if !valid_registry_cache(cache) {
                continue;
            }
            for key in [
                "file_id",
                "modified_ns",
                "changed_ns",
                "metadata_hash",
                "projection_hash",
            ] {
                cache[key] = toml_edit::value(format!("<{}>", key.replace('_', "-")));
            }
        }
        document.to_string()
    }
}

fn valid_registry_cache(cache: &toml_edit::Table) -> bool {
    cache.get("schema").and_then(toml_edit::Item::as_integer) == Some(1)
        && cache
            .get("platform")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(cache_platform_matches_current_host)
        && cache
            .get("file_id")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(|value| !value.is_empty())
        && cache
            .get("file_size")
            .and_then(toml_edit::Item::as_str)
            .is_some_and(canonical_u64)
        && ["modified_ns", "changed_ns"].into_iter().all(|key| {
            cache
                .get(key)
                .and_then(toml_edit::Item::as_str)
                .is_some_and(canonical_integer)
        })
        && ["metadata_hash", "projection_hash"].into_iter().all(|key| {
            cache
                .get(key)
                .and_then(toml_edit::Item::as_str)
                .is_some_and(valid_sha256)
        })
}

#[cfg(unix)]
fn cache_platform_matches_current_host(value: &str) -> bool {
    value == "unix"
}

#[cfg(windows)]
fn cache_platform_matches_current_host(value: &str) -> bool {
    value == "windows"
}

#[cfg(not(any(unix, windows)))]
fn cache_platform_matches_current_host(_value: &str) -> bool {
    false
}

fn canonical_u64(value: &str) -> bool {
    value
        .parse::<u64>()
        .is_ok_and(|parsed| parsed.to_string() == value)
}

fn canonical_integer(value: &str) -> bool {
    value
        .parse::<i128>()
        .is_ok_and(|parsed| parsed.to_string() == value)
}

fn valid_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn generation_token(kind: &str, slug: &str, generation: u64) -> String {
    if generation == 1 {
        format!("<{kind}:{slug}>")
    } else {
        format!("<{kind}:{slug}:{generation}>")
    }
}
