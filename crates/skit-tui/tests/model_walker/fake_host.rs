use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::{Path, PathBuf},
};

use skit_application::{
    AgentTarget, CreateEntry, EntryPayload,
    form_state::remembered_values,
    health::HealthRebuildOutcome,
    preferences::{
        AfterRunChoice, InteractiveFormChoice, JavascriptChoice, PreferencesChangeSet,
        PreferencesDraft,
    },
    runner_management::{
        EditableArgvDialect, join_editable_argv, split_editable_argv, validate_runner_argv,
    },
};
use skit_domain::{
    EntrySummary, Slug,
    parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue},
};
use skit_language::{
    detect_candidates, managed_params, placeholder_params, validate_pep440_specifiers,
    validate_pep508_requirement,
};
use skit_ui::{
    Action, AddAction, AddEffect, AddRequestId, AddWorkflowState, DraftDeleteOutcome, DraftKind,
    DraftSummary, Effect, FieldValue, FormField, FormPurpose, FormView, HealthAction, HealthView,
    HostRequest, LibraryState, PreferencesAction, PreferencesEffect, PreferencesView,
    ReviewDefaults, RunFormView, RunnerManagerAction, RunnerManagerView, RunnerRemoveRequest,
    RunnerRow, RunnerRowIdentity, RunnerSaveOwner, RunnerSaveRequest, RunnerSaveTarget, Screen,
    SettingsSectionId, SettingsView, SourceSnapshot, TypedValue,
};

use super::fixtures::{self, EntryFixture};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct HostSnapshot {
    entries: BTreeMap<String, EntryFixture>,
    rerunnable: BTreeSet<String>,
    kept_drafts: Vec<DraftSummary>,
    sources: BTreeMap<PathBuf, SourceSnapshot>,
    runners: Vec<RunnerRow>,
    preferences: skit_application::preferences::PreferencesSnapshot,
    health: skit_application::health::HealthSnapshot,
    preference_settings: BTreeMap<String, String>,
    installed_skills: BTreeSet<PathBuf>,
    agent_targets: Vec<AgentTarget>,
    virtual_directories: BTreeSet<PathBuf>,
    virtual_paths: BTreeSet<PathBuf>,
    virtual_files: BTreeSet<PathBuf>,
    remembered_values: BTreeMap<String, BTreeMap<String, String>>,
    extra_args: BTreeMap<String, Vec<String>>,
    last_runs: BTreeMap<String, BTreeMap<String, FieldValue>>,
    last_runner: Option<String>,
    draft_sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HostError {
    message: String,
}

impl HostError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for HostError {}

#[derive(Clone, Debug)]
pub(super) struct FakeHost {
    model: HostSnapshot,
}

impl Default for FakeHost {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHost {
    pub(super) fn new() -> Self {
        let fixtures = fixtures::fixture_set();
        Self {
            model: HostSnapshot {
                entries: fixtures
                    .entries
                    .into_iter()
                    .map(|entry| (entry.summary.slug.as_str().to_owned(), entry))
                    .collect(),
                rerunnable: BTreeSet::from(["python-tool".to_owned()]),
                kept_drafts: fixtures.kept_drafts,
                sources: fixtures.sources,
                runners: fixtures.runners,
                preferences: fixtures.preferences,
                health: fixtures.health,
                preference_settings: BTreeMap::new(),
                installed_skills: BTreeSet::new(),
                agent_targets: fixtures.agent_targets,
                virtual_directories: fixtures.virtual_directories,
                virtual_paths: fixtures.virtual_paths,
                virtual_files: fixtures.virtual_files,
                remembered_values: BTreeMap::new(),
                extra_args: BTreeMap::new(),
                last_runs: BTreeMap::from([("python-tool".to_owned(), BTreeMap::new())]),
                last_runner: Some("codex".to_owned()),
                draft_sequence: 0,
            },
        }
    }

    pub(super) fn initial_state(&self) -> LibraryState {
        let mut state = LibraryState::from_library_surface(self.surface());
        let _ = state.update(Action::ReplaceRerunnable(self.rerunnable_slugs()));
        state
    }

    pub(super) fn file_picker_tree(&self) -> (PathBuf, BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
        let files = self
            .all_virtual_paths()
            .difference(&self.model.virtual_directories)
            .cloned()
            .collect();
        (
            PathBuf::from("/fixtures"),
            self.model.virtual_directories.clone(),
            files,
        )
    }

    fn all_virtual_paths(&self) -> BTreeSet<PathBuf> {
        let mut paths = self.model.virtual_directories.clone();
        paths.extend(self.model.virtual_paths.iter().cloned());
        paths.extend(self.model.virtual_files.iter().cloned());
        paths.extend(self.model.sources.keys().cloned());
        paths
    }

    pub(super) fn serve(&mut self, effect: Effect) -> Result<Action, HostError> {
        match effect {
            Effect::None => Err(HostError::new(
                "protocol error: Effect::None reached the host",
            )),
            Effect::Quit => Err(HostError::new(
                "protocol error: Effect::Quit reached the host",
            )),
            Effect::Reload => Ok(Action::ReplaceSurface {
                surface: self.surface(),
                rerunnable: self.rerunnable_slugs(),
            }),
            Effect::Rerun { selector } => self.rerun(&selector),
            Effect::Open { request, selector } => self.open(request, selector.as_deref()),
            Effect::Submit {
                purpose,
                selector,
                values,
            } => self.submit(purpose, selector.as_deref(), &values),
            Effect::CountRunGlob {
                selector,
                field,
                value,
                request,
            } => self.count_run_glob(&selector, field, value, &request),
            Effect::SaveRunPreset {
                selector,
                name,
                values,
                secret_names,
            } => self.save_preset(&selector, name, values, &secret_names),
            Effect::Add(effects) => self.serve_add(effects),
            Effect::HealthRebuild => Ok(Action::Health(HealthAction::Rebuilt {
                snapshot: Box::new(self.current_health()),
                outcome: HealthRebuildOutcome {
                    entry_count: self.model.entries.len(),
                    problems: Vec::new(),
                },
            })),
            Effect::SaveRunner { request, owner } => self.save_runner(request, owner),
            Effect::RemoveRunner(request) => self.remove_runner(request),
            Effect::RefreshPreferencesAfterRunners => Ok(Action::RunnerManagerClosed {
                preferences: Box::new(self.preferences_view()),
            }),
            Effect::Preferences(effect) => self.serve_preferences(effect),
            Effect::Edit { selector } => {
                if self.model.entries.contains_key(&selector) {
                    self.complete("Source saved")
                } else {
                    Ok(Action::SetStatus(format!("entry not found: {selector}")))
                }
            }
            Effect::Remove { selector } => self.remove_entry(&selector),
        }
    }

    pub(super) fn contains_selector(&self, selector: &str) -> bool {
        self.model.entries.contains_key(selector)
    }

    pub(super) fn validate_effect_sanity(&self, effect: &Effect) -> Result<(), HostError> {
        let selector = match effect {
            Effect::Rerun { selector }
            | Effect::CountRunGlob { selector, .. }
            | Effect::SaveRunPreset { selector, .. }
            | Effect::Edit { selector }
            | Effect::Remove { selector } => Some(selector.as_str()),
            Effect::Open {
                selector: Some(selector),
                ..
            }
            | Effect::Submit {
                selector: Some(selector),
                ..
            } => Some(selector.as_str()),
            Effect::None
            | Effect::Quit
            | Effect::Reload
            | Effect::Open { selector: None, .. }
            | Effect::Submit { selector: None, .. }
            | Effect::Add(_)
            | Effect::HealthRebuild
            | Effect::SaveRunner { .. }
            | Effect::RemoveRunner(_)
            | Effect::RefreshPreferencesAfterRunners
            | Effect::Preferences(_) => None,
        };
        if let Some(selector) = selector
            && !self.contains_selector(selector)
        {
            return Err(HostError::new(format!(
                "effect selector is absent from the fake model: {selector}"
            )));
        }
        Ok(())
    }

    fn surface(&self) -> skit_ui::LibrarySurface {
        fixtures::surface(self.model.entries.values().cloned())
    }

    fn rerunnable_slugs(&self) -> Vec<Slug> {
        self.model
            .rerunnable
            .iter()
            .filter_map(|selector| Slug::parse(selector).ok())
            .collect()
    }

    fn current_health(&self) -> skit_application::health::HealthSnapshot {
        let mut snapshot = self.model.health.clone();
        snapshot.entry_count = self.model.entries.len();
        let available = BTreeSet::from(["git", "curl", "make", "rg"]);
        snapshot.issues = self
            .model
            .entries
            .values()
            .filter_map(|entry| {
                let tools = entry
                    .settings
                    .needs
                    .iter()
                    .filter(|tool| !available.contains(tool.as_str()))
                    .cloned()
                    .collect::<Vec<_>>();
                if !tools.is_empty() {
                    Some(skit_application::health::HealthIssue {
                        slug: entry.summary.slug.as_str().to_owned(),
                        name: entry.summary.name.clone(),
                        kind: skit_application::health::HealthIssueKind::MissingNeeds { tools },
                    })
                } else {
                    let runner = &entry.settings.runner;
                    (entry.summary.kind.as_str() == "prompt"
                        && !runner.is_empty()
                        && !self.runner_names().contains(runner))
                    .then(|| skit_application::health::HealthIssue {
                        slug: entry.summary.slug.as_str().to_owned(),
                        name: entry.summary.name.clone(),
                        kind: skit_application::health::HealthIssueKind::LaunchBlocked {
                            reason: format!("configured runner is missing: {runner}"),
                        },
                    })
                }
            })
            .collect();
        snapshot.invalid_runner_rows = self
            .model
            .runners
            .iter()
            .filter_map(|row| row.reason.clone())
            .collect();
        let pypi = self
            .model
            .preference_settings
            .get("mirror.pypi")
            .map_or("off", String::as_str);
        let github = self
            .model
            .preference_settings
            .get("mirror.github")
            .map_or("off", String::as_str);
        let npm = self
            .model
            .preference_settings
            .get("mirror.npm")
            .map_or("off", String::as_str);
        snapshot.mirror = if [pypi, github, npm].into_iter().all(|axis| axis == "off") {
            skit_application::health::MirrorHealth::Off
        } else {
            let axes = format!("pypi={pypi} · github={github} · npm={npm}");
            if self.model.preferences.mirror.enabled {
                skit_application::health::MirrorHealth::On { axes }
            } else {
                skit_application::health::MirrorHealth::Paused { axes }
            }
        };
        snapshot
    }

    fn require_entry(&self, selector: &str) -> Result<&EntryFixture, HostError> {
        self.model
            .entries
            .get(selector)
            .ok_or_else(|| HostError::new(format!("entry not found: {selector}")))
    }

    fn require_entry_mut(&mut self, selector: &str) -> Result<&mut EntryFixture, HostError> {
        self.model
            .entries
            .get_mut(selector)
            .ok_or_else(|| HostError::new(format!("entry not found: {selector}")))
    }

    fn rerun(&mut self, selector: &str) -> Result<Action, HostError> {
        if !self.model.entries.contains_key(selector) {
            return Ok(Action::SetStatus(format!("entry not found: {selector}")));
        }
        if !self.model.rerunnable.contains(selector) {
            return Ok(Action::SetStatus(format!(
                "entry has no saved run: {selector}"
            )));
        }
        if self.model.entries.get(selector).is_some_and(|entry| {
            entry.summary.kind.as_str() == "prompt"
                && (entry.settings.runner.is_empty()
                    || !self.runner_names().contains(&entry.settings.runner))
        }) {
            if self.model.entries.get(selector).is_some_and(|entry| {
                !entry.settings.runner.is_empty()
                    && !self.runner_names().contains(&entry.settings.runner)
            }) {
                return Ok(Action::SetStatus(format!(
                    "configured runner is missing: {}",
                    self.model.entries[selector].settings.runner
                )));
            }
            return self
                .open(HostRequest::Run, Some(selector))
                .map(|action| match action {
                    Action::PromptRunnerRequired { form, .. } => Action::Present(Screen::Run(form)),
                    action => action,
                });
        }
        self.after_run_action()
    }

    fn open(&self, request: HostRequest, selector: Option<&str>) -> Result<Action, HostError> {
        let screen = match request {
            HostRequest::Run => {
                let entry = self.require_selected(request, selector)?;
                let runners = if entry.summary.kind.as_str() == "prompt" {
                    self.runner_names()
                } else {
                    Vec::new()
                };
                let extra_args = self
                    .model
                    .extra_args
                    .get(entry.summary.slug.as_str())
                    .map_or_else(String::new, |arguments| {
                        join_editable_argv(arguments, EditableArgvDialect::host())
                    });
                Screen::Run(Box::new(
                    RunFormView::from_declarations(
                        entry.summary.slug.as_str(),
                        &entry.summary.name,
                        effective_declarations(entry),
                        self.model
                            .remembered_values
                            .get(entry.summary.slug.as_str())
                            .unwrap_or(&BTreeMap::new()),
                        &runners,
                        &entry.settings.runner,
                        &entry.presets,
                        &extra_args,
                    )
                    .with_context(self.run_context(entry)),
                ))
            }
            HostRequest::Add => {
                self.require_unselected(request, selector)?;
                Screen::Add(Box::new(
                    AddWorkflowState::new(self.model.kept_drafts.clone()).with_review_defaults(
                        ReviewDefaults {
                            runner_names: self.runner_names(),
                            last_runner: self.model.last_runner.clone(),
                            ..ReviewDefaults::default()
                        },
                    ),
                ))
            }
            HostRequest::Settings | HostRequest::Presets => {
                let entry = self.require_selected(request, selector)?;
                let mut inputs = entry.settings.clone();
                inputs.name = entry.summary.name.clone();
                inputs.description = entry.summary.description.clone();
                inputs.configured_runners = self.runner_names();
                inputs.presets = entry.presets.clone();
                if request == HostRequest::Presets {
                    inputs.revealed = Some(SettingsSectionId::Presets);
                }
                Screen::Settings(Box::new(SettingsView::from_inputs(&inputs)))
            }
            HostRequest::Preferences => {
                self.require_unselected(request, selector)?;
                Screen::Preferences(Box::new(self.preferences_view()))
            }
            HostRequest::Health => {
                self.require_unselected(request, selector)?;
                Screen::Health(Box::new(HealthView::new(self.current_health())))
            }
            HostRequest::Runners => {
                self.require_unselected(request, selector)?;
                Screen::Runners(Box::new(RunnerManagerView::new(self.model.runners.clone())))
            }
            HostRequest::Rename => {
                let entry = self.require_selected(request, selector)?;
                Screen::Form(FormView {
                    purpose: FormPurpose::Rename,
                    title: "Rename {}".to_owned(),
                    title_arguments: vec![entry.summary.name.clone()],
                    translate_title: true,
                    selector: Some(entry.summary.slug.as_str().to_owned()),
                    fields: vec![FormField::text("name", "Name", &entry.summary.name)],
                    focused: 0,
                    submit_label: "Rename".to_owned(),
                })
            }
        };
        Ok(match screen {
            Screen::Run(form)
                if form
                    .context()
                    .is_some_and(|context| context.entry_kind == "prompt")
                    && !form.has_runner_picker() =>
            {
                Action::PromptRunnerRequired {
                    form,
                    cancel_status: "A prompt needs a configured agent to run with.".to_owned(),
                }
            }
            screen => Action::Present(screen),
        })
    }

    fn require_selected(
        &self,
        request: HostRequest,
        selector: Option<&str>,
    ) -> Result<&EntryFixture, HostError> {
        let selector = selector
            .ok_or_else(|| HostError::new(format!("{request:?} requires an entry selector")))?;
        self.require_entry(selector)
    }

    fn run_context(&self, entry: &EntryFixture) -> skit_ui::RunFormContext {
        let mut context = fixtures::run_context(entry.summary.kind.as_str());
        let Some(path) = &mut context.path else {
            return context;
        };
        path.workdir = match entry.settings.workdir.as_str() {
            "" => "/fixtures/work".to_owned(),
            "invoke" => "/fixtures/invoke".to_owned(),
            "store" => format!("/fixtures/library/{}", entry.summary.slug.as_str()),
            "origin" => Path::new(&entry.settings.source)
                .parent()
                .filter(|parent| self.model.virtual_directories.contains(*parent))
                .map_or_else(
                    || "/fixtures/invoke".to_owned(),
                    |parent| parent.display().to_string(),
                ),
            custom => custom.to_owned(),
        };
        context
    }

    fn require_unselected(
        &self,
        request: HostRequest,
        selector: Option<&str>,
    ) -> Result<(), HostError> {
        if selector.is_some() {
            Err(HostError::new(format!(
                "{request:?} does not accept an entry selector"
            )))
        } else {
            Ok(())
        }
    }

    fn submit(
        &mut self,
        purpose: FormPurpose,
        selector: Option<&str>,
        values: &BTreeMap<String, FieldValue>,
    ) -> Result<Action, HostError> {
        match purpose {
            FormPurpose::Run => {
                let selector = self.require_submit_selector(purpose, selector)?;
                let entry = self
                    .model
                    .entries
                    .get(&selector)
                    .expect("the selector was validated");
                let prompt = entry.summary.kind.as_str() == "prompt";
                let declarations = effective_declarations(entry).to_vec();
                for key in values.keys() {
                    let unknown_parameter = key.strip_prefix("value:").is_some_and(|name| {
                        !declarations
                            .iter()
                            .any(|declaration| declaration.name == name)
                    });
                    let unknown_reserved = key.starts_with("_skit_")
                        && !matches!(
                            key.as_str(),
                            "_skit_args"
                                | "_skit_dry_run"
                                | "_skit_preset"
                                | "_skit_runner"
                                | "_skit_runner_picked"
                                | "_skit_save_preset"
                        );
                    if unknown_parameter || unknown_reserved {
                        return Ok(Action::SetStatus(format!("unknown run field: {key}")));
                    }
                }
                if let Some(value) = values.get("_skit_dry_run")
                    && let Err(error) = settings_bool_value(value, "_skit_dry_run")
                {
                    return Ok(Action::SetStatus(error.message));
                }
                let runner = values
                    .get("_skit_runner")
                    .map(FieldValue::as_text)
                    .unwrap_or_default();
                if prompt && !self.runner_names().contains(&runner) {
                    return Ok(Action::SetStatus(format!("runner not found: {runner}")));
                }
                let runner_was_picked = match values
                    .get("_skit_runner_picked")
                    .map(|value| settings_bool_value(value, "_skit_runner_picked"))
                    .transpose()
                {
                    Ok(value) => value.unwrap_or(false),
                    Err(error) => return Ok(Action::SetStatus(error.message)),
                };
                let extra_args = match split_editable_argv(
                    values
                        .get("_skit_args")
                        .map(FieldValue::as_text)
                        .unwrap_or_default()
                        .as_str(),
                    EditableArgvDialect::host(),
                ) {
                    Ok(arguments) => arguments,
                    Err(error) => {
                        return Ok(Action::SetStatus(format!("invalid arguments: {error:?}")));
                    }
                };
                let public = declarations
                    .iter()
                    .filter(|declaration| !declaration.secret)
                    .filter_map(|declaration| {
                        values
                            .get(&declaration.name)
                            .or_else(|| values.get(&format!("value:{}", declaration.name)))
                            .map(|value| (declaration.name.clone(), value.clone()))
                    })
                    .collect::<BTreeMap<_, _>>();
                let submitted = public
                    .iter()
                    .map(|(name, value)| (name.clone(), value.as_text().to_owned()))
                    .collect::<BTreeMap<_, _>>();
                let remembered = remembered_values(&declarations, &submitted);
                self.model
                    .remembered_values
                    .insert(selector.clone(), remembered.clone());
                self.model.extra_args.insert(selector.clone(), extra_args);
                if prompt && runner_was_picked {
                    self.model.last_runner = Some(runner);
                }
                self.model
                    .last_runs
                    .insert(selector.clone(), public.clone());
                self.model.rerunnable.insert(selector.to_owned());
                let runners = self.runner_names();
                let entry = self
                    .model
                    .entries
                    .get_mut(&selector)
                    .expect("the selector was validated");
                refresh_entry_projection(entry, &runners, Some(&remembered), true);
                self.after_run_action()
            }
            FormPurpose::Settings => {
                let selector = self.require_submit_selector(purpose, selector)?;
                match self.apply_settings(&selector, values) {
                    Ok(()) => self.complete("Entry settings saved"),
                    Err(error) => Ok(Action::SetStatus(format!("Error: {error}"))),
                }
            }
            FormPurpose::Rename => {
                let selector = self.require_submit_selector(purpose, selector)?;
                let name = values
                    .get("name")
                    .map(FieldValue::as_text)
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty());
                let Some(name) = name else {
                    return Ok(Action::SetStatus("rename requires a name".to_owned()));
                };
                if let Err(error) = self.validate_entry_name_available(&selector, &name) {
                    return Ok(Action::SetStatus(error.message));
                }
                let entry = self.require_entry_mut(&selector)?;
                entry.summary.name = name.clone();
                entry.settings.name = name;
                self.complete("Entry renamed")
            }
            FormPurpose::Add | FormPurpose::Preferences | FormPurpose::Runners => {
                Err(HostError::new(format!(
                    "protocol error: generic {purpose:?} submit is unreachable"
                )))
            }
        }
    }

    fn after_run_action(&self) -> Result<Action, HostError> {
        if self.model.preferences.after_run == AfterRunChoice::Exit {
            Ok(Action::Quit)
        } else {
            self.complete("Run finished with exit status 0")
        }
    }

    fn require_submit_selector(
        &self,
        purpose: FormPurpose,
        selector: Option<&str>,
    ) -> Result<String, HostError> {
        let selector = selector
            .ok_or_else(|| HostError::new(format!("{purpose:?} requires an entry selector")))?;
        self.require_entry(selector)?;
        Ok(selector.to_owned())
    }

    fn apply_settings(
        &mut self,
        selector: &str,
        values: &BTreeMap<String, FieldValue>,
    ) -> Result<(), HostError> {
        let mut next = self.require_entry(selector)?.clone();
        validate_settings_keys(values, &next)?;

        if let Some(name) = settings_text(values, skit_ui::NAME_KEY) {
            let name = name.trim();
            if name.is_empty() {
                return Err(HostError::new("entry settings require a name"));
            }
            self.validate_entry_name_available(selector, name)?;
            next.summary.name = name.to_owned();
            next.settings.name = name.to_owned();
        }
        if let Some(description) = settings_text(values, skit_ui::DESCRIPTION_KEY) {
            next.summary.description = description.clone();
            next.settings.description = description;
        }
        if let Some(workdir) = settings_text(values, skit_ui::WORKDIR_KEY) {
            if !matches!(workdir.as_str(), "invoke" | "store" | "origin")
                && !Path::new(&workdir).is_absolute()
            {
                return Err(HostError::new(
                    "working directory must be invoke, store, origin, or an absolute path",
                ));
            }
            next.settings.workdir = workdir;
        }
        if let Some(interpreter) = settings_text(values, skit_ui::INTERPRETER_KEY) {
            if !interpreter.is_empty() && !next.settings.pinnable_interpreter {
                return Err(HostError::new(
                    "the entry does not use a pinnable interpreter",
                ));
            }
            next.settings.interpreter = interpreter;
        }
        if let Some(runner) = settings_text(values, skit_ui::RUNNER_KEY) {
            if next.summary.kind.as_str() != "prompt" {
                return Err(HostError::new("only a prompt entry accepts a runner"));
            }
            next.settings.runner = runner.trim().to_owned();
        }
        if values.contains_key(skit_ui::DEPENDENCIES_KEY) {
            if next.settings.dependency_flavor.is_none() {
                return Err(HostError::new(
                    "package dependencies do not apply to this entry",
                ));
            }
            next.settings.effective_dependencies = settings_list(values, skit_ui::DEPENDENCIES_KEY);
        }
        if let Some(python) = settings_text(values, skit_ui::PYTHON_KEY) {
            if next.settings.dependency_flavor != Some(skit_ui::DependencyFlavor::Uv)
                && !python.is_empty()
            {
                return Err(HostError::new(
                    "a Python constraint applies only to Python entries",
                ));
            }
            next.settings.effective_requires_python = python.trim().to_owned();
        }
        if next.summary.kind.as_str() == "python" {
            for requirement in &next.settings.effective_dependencies {
                validate_pep508_requirement(requirement).map_err(|_| {
                    HostError::new(format!("invalid Python requirement: {requirement}"))
                })?;
            }
            let python = next.settings.effective_requires_python.trim();
            if !python.is_empty() && !matches!(python.to_ascii_lowercase().as_str(), "-" | "none") {
                validate_pep440_specifiers(python)
                    .map_err(|_| HostError::new(format!("invalid Python constraint: {python}")))?;
            }
        }
        if values.contains_key(skit_ui::NEEDS_KEY) {
            next.settings.needs = settings_list(values, skit_ui::NEEDS_KEY);
        }
        if let Some(template) = settings_text(values, skit_ui::TEMPLATE_KEY) {
            if next.summary.kind.as_str() != "command" {
                return Err(HostError::new("only a command entry accepts a template"));
            }
            next.settings.template = template;
        }
        if let Some(interpolate) = settings_bool(values, skit_ui::INTERPOLATE_KEY)? {
            if next.summary.kind.as_str() != "prompt" {
                return Err(HostError::new(
                    "only a prompt entry accepts interpolation settings",
                ));
            }
            next.settings.interpolate = interpolate;
        }

        for (key, value) in values {
            if let Some(name) = key.strip_prefix(skit_ui::PRESET_PREFIX) {
                if settings_bool_value(value, key)? {
                    if !next.presets.contains_key(name) {
                        return Err(HostError::new(format!("preset not found: {name}")));
                    }
                } else if next.presets.remove(name).is_none() {
                    return Err(HostError::new(format!("preset not found: {name}")));
                }
            }
        }

        if settings_bool(values, skit_ui::RESYNC_KEY)?.unwrap_or(false) {
            next.resync_count = next.resync_count.saturating_add(1);
        }
        let mut declarations = next.declarations.clone();
        let mut candidates = next.settings.candidates.clone();

        for name in settings_list(values, skit_ui::MANAGE_KEY) {
            let Some(index) = candidates.iter().position(|candidate| candidate == &name) else {
                return Err(HostError::new(format!(
                    "source candidate not found: {name}"
                )));
            };
            if declarations.iter().any(|item| item.name == name) {
                return Err(HostError::new(format!("parameter already exists: {name}")));
            }
            candidates.remove(index);
            let mut declaration = ParamDecl::new(name);
            declaration.binding = ParameterBinding::Const;
            declaration.delivery = ParameterDelivery::Inject;
            declarations.push(declaration);
        }
        for name in settings_list(values, "source:unmanage") {
            let Some(index) = declarations.iter().position(|item| item.name == name) else {
                return Err(HostError::new(format!("parameter not found: {name}")));
            };
            declarations.remove(index);
            if !candidates.contains(&name) {
                candidates.push(name);
            }
        }
        for name in settings_list(values, skit_ui::ADD_PARAMETER_KEY) {
            let name = name.trim().to_owned();
            if name.is_empty() || declarations.iter().any(|item| item.name == name) {
                return Err(HostError::new(format!("parameter already exists: {name}")));
            }
            let mut declaration = ParamDecl::new(name.clone());
            if matches!(next.summary.kind.as_str(), "command" | "prompt") {
                declaration.delivery = ParameterDelivery::Env;
            }
            declarations.push(declaration);
            candidates.retain(|candidate| candidate != &name);
        }
        for name in settings_list(values, skit_ui::PROMPT_CANDIDATES_KEY) {
            let name = name.trim().to_owned();
            if name.is_empty() || declarations.iter().any(|item| item.name == name) {
                return Err(HostError::new(format!("parameter already exists: {name}")));
            }
            let mut declaration = ParamDecl::new(name.clone());
            declaration.delivery = ParameterDelivery::Placeholder;
            declarations.push(declaration);
            candidates.retain(|candidate| candidate != &name);
        }
        for name in settings_list(values, "parameter:remove") {
            let Some(index) = declarations.iter().position(|item| item.name == name) else {
                return Err(HostError::new(format!("parameter not found: {name}")));
            };
            declarations.remove(index);
        }

        apply_parameter_values(values, &mut declarations)?;
        if next.summary.kind.as_str() == "command" {
            declarations = reconcile_command_parameters(&next.settings.template, &declarations);
        }
        for name in settings_list(values, skit_ui::NORMALIZE_KEY) {
            if !declarations.iter().any(|item| item.name == name) {
                return Err(HostError::new(format!(
                    "normalization target not found: {name}"
                )));
            }
            next.normalized.insert(name);
        }

        let secret_names = declarations
            .iter()
            .filter(|declaration| declaration.secret)
            .map(|declaration| declaration.name.clone())
            .collect::<BTreeSet<_>>();
        for preset in next.presets.values_mut() {
            preset.retain(|name, _| !secret_names.contains(name));
        }
        next.presets.retain(|_, preset| !preset.is_empty());
        let mut next_remembered = self.model.remembered_values.get(selector).cloned();
        if let Some(remembered) = &mut next_remembered {
            remembered.retain(|name, _| !secret_names.contains(name));
        }
        let mut next_last_run = self.model.last_runs.get(selector).cloned();
        if let Some(last_run) = &mut next_last_run {
            last_run.retain(|name, _| !secret_names.contains(name));
        }
        next.declarations = declarations;
        next.settings.managed = next.declarations.clone();
        next.settings.candidates = candidates;
        next.settings.presets = next.presets.clone();
        refresh_entry_projection(
            &mut next,
            &self.runner_names(),
            next_remembered.as_ref(),
            self.model.rerunnable.contains(selector),
        );
        self.model.entries.insert(selector.to_owned(), next);
        if let Some(last_run) = next_last_run {
            self.model.last_runs.insert(selector.to_owned(), last_run);
        }
        if let Some(remembered) = next_remembered {
            self.model
                .remembered_values
                .insert(selector.to_owned(), remembered);
        }
        self.refresh_runner_pin_counts();
        Ok(())
    }

    fn validate_entry_name_available(&self, selector: &str, name: &str) -> Result<(), HostError> {
        if self
            .model
            .entries
            .iter()
            .any(|(candidate, entry)| candidate != selector && entry.summary.name == name)
        {
            Err(HostError::new(format!("entry name already exists: {name}")))
        } else {
            Ok(())
        }
    }

    fn count_run_glob(
        &self,
        selector: &str,
        field: usize,
        value: String,
        request: &skit_application::form_feedback::GlobCountRequest,
    ) -> Result<Action, HostError> {
        let Some(entry) = self.model.entries.get(selector) else {
            return Ok(Action::SetStatus(format!("entry not found: {selector}")));
        };
        let form = RunFormView::from_declarations(
            selector,
            &entry.summary.name,
            effective_declarations(entry),
            &BTreeMap::new(),
            &[],
            "",
            &entry.presets,
            "",
        );
        if field >= form.fields().len() {
            return Ok(Action::SetStatus(format!("run field not found: {field}")));
        }
        let cwd = Path::new(&request.cwd);
        let paths = self.all_virtual_paths();
        let count = request
            .pieces
            .iter()
            .map(|piece| virtual_glob_count(cwd, piece, &paths))
            .sum();
        Ok(Action::SetRunGlobCount {
            field,
            value,
            count,
        })
    }

    fn save_preset(
        &mut self,
        selector: &str,
        name: String,
        mut values: BTreeMap<String, String>,
        secret_names: &BTreeSet<String>,
    ) -> Result<Action, HostError> {
        if name.trim().is_empty() {
            return Ok(Action::SetStatus("preset name is empty".to_owned()));
        }
        let declarations = effective_declarations(self.require_entry(selector)?).to_vec();
        if declarations.is_empty() {
            return Ok(Action::SetStatus(
                "cannot save a preset because the entry has no form fields".to_owned(),
            ));
        }
        values.retain(|key, _| !secret_names.contains(key));
        let values = declarations
            .iter()
            .filter(|declaration| !declaration.secret)
            .filter_map(|declaration| {
                values
                    .get(&declaration.name)
                    .map(|value| (declaration.name.clone(), value.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let entry = self.require_entry_mut(selector)?;
        entry.presets.insert(name.clone(), values);
        entry.settings.presets = entry.presets.clone();
        entry.detail.presets = entry.presets.keys().cloned().collect();
        Ok(Action::RunPresetSaved {
            name: name.clone(),
            presets: entry.presets.clone(),
            message: format!("Preset \"{name}\" saved."),
        })
    }

    fn serve_add(&mut self, effects: Vec<AddEffect>) -> Result<Action, HostError> {
        if effects.is_empty() {
            return Err(HostError::new(
                "protocol error: an empty Add effect reached the host",
            ));
        }
        let mut warnings = Vec::new();
        for effect in effects {
            match effect {
                AddEffect::InspectSource { request, path } => {
                    return Ok(Action::Add(AddAction::SourceInspected {
                        request,
                        result: self
                            .model
                            .sources
                            .get(&path)
                            .cloned()
                            .ok_or_else(|| format!("source not found: {}", path.display())),
                    }));
                }
                AddEffect::AuthorDraft { request, kind } => {
                    return self.author_draft(request, kind);
                }
                AddEffect::DeleteDraft { request, draft } => {
                    return Ok(Action::Add(AddAction::DraftDeleted {
                        request,
                        result: self.delete_draft(&draft).map_err(|error| error.message),
                    }));
                }
                AddEffect::EditSource { request, path } => {
                    return Ok(Action::Add(AddAction::SourceEdited {
                        request,
                        result: self
                            .model
                            .sources
                            .get(&path)
                            .cloned()
                            .ok_or_else(|| format!("source not found: {}", path.display())),
                    }));
                }
                AddEffect::Commit {
                    request,
                    entry,
                    source,
                } => {
                    return Ok(Action::Add(AddAction::CommitFinished {
                        request,
                        result: self
                            .commit_entry(*entry, source.as_ref())
                            .map_err(|error| error.message),
                    }));
                }
                AddEffect::ConsumeDraft(source) => {
                    if let Err(error) = self.consume_draft(&source) {
                        warnings.push(error.message);
                    }
                }
                AddEffect::DraftKept(path) => {
                    if let Err(error) = self.keep_draft(&path) {
                        warnings.push(error.message);
                    }
                }
                AddEffect::RememberRunner(name) => {
                    if !self.runner_names().contains(&name) {
                        warnings.push(format!("runner not found: {name}"));
                        continue;
                    }
                    self.model.last_runner = Some(name);
                }
                AddEffect::Complete(selector) => {
                    let message = add_completion_message(&warnings);
                    return Ok(match Slug::parse(&selector) {
                        Ok(slug) => Action::AddCompleted {
                            surface: self.surface(),
                            rerunnable: self.rerunnable_slugs(),
                            slug,
                            message,
                        },
                        Err(_) => Action::Complete {
                            surface: None,
                            rerunnable: None,
                            message,
                        },
                    });
                }
                AddEffect::Cancel => return Ok(Action::AddCancelled),
            }
        }
        Ok(Action::ClearStatus)
    }

    fn delete_draft(&mut self, draft: &DraftSummary) -> Result<DraftDeleteOutcome, HostError> {
        let Some(index) = self
            .model
            .kept_drafts
            .iter()
            .position(|candidate| candidate.path == draft.path)
        else {
            return Err(HostError::new(format!(
                "kept draft not found: {}",
                draft.path.display()
            )));
        };
        if self.model.kept_drafts[index] != *draft {
            return Err(HostError::new(format!(
                "kept draft changed: {}",
                draft.path.display()
            )));
        }
        if draft.identity.is_none() {
            return Err(HostError::new(format!(
                "the kept draft has no filesystem identity: {}",
                draft.path.display()
            )));
        }
        let Some(current) = self.model.sources.get(&draft.path).cloned() else {
            self.model.kept_drafts.remove(index);
            return Ok(DraftDeleteOutcome::AlreadyMissing);
        };
        if !current.is_draft {
            return Err(HostError::new(format!(
                "source is not a kept draft: {}",
                draft.path.display()
            )));
        }
        let current_fact = fixtures::draft_summary(&current);
        if current_fact != *draft {
            self.model.kept_drafts[index] = current_fact.clone();
            return Ok(DraftDeleteOutcome::Changed(current_fact));
        }
        self.model.kept_drafts.remove(index);
        self.model.sources.remove(&draft.path);
        Ok(DraftDeleteOutcome::Removed)
    }

    fn author_draft(
        &mut self,
        request: AddRequestId,
        kind: DraftKind,
    ) -> Result<Action, HostError> {
        self.model.draft_sequence = self.model.draft_sequence.saturating_add(1);
        let suffix = match kind {
            DraftKind::Script => "py",
            DraftKind::Prompt => "prompt.md",
        };
        let path = PathBuf::from(format!(
            "/fixtures/drafts/skit-new-{}.{}",
            self.model.draft_sequence, suffix
        ));
        let source = SourceSnapshot {
            path: path.clone(),
            source_record: path.display().to_string(),
            bytes: match kind {
                DraftKind::Script => b"#!/usr/bin/env python3\nprint('draft')\n".to_vec(),
                DraftKind::Prompt => b"Write about {{TOPIC}}.\n".to_vec(),
            },
            permissions: skit_application::SourcePermissions {
                readonly: false,
                unix_mode: Some(0o700),
            },
            executable: Some(matches!(kind, DraftKind::Script)),
            is_regular: true,
            is_directory: false,
            is_draft: true,
            identity: Some(skit_application::SourceIdentity::unix(
                7,
                100 + self.model.draft_sequence,
                1_776_981_600,
                0,
            )),
        };
        self.model.sources.insert(path, source.clone());
        self.model
            .kept_drafts
            .push(fixtures::draft_summary(&source));
        Ok(Action::Add(AddAction::DraftEdited {
            request,
            result: Ok(Some(source)),
        }))
    }

    fn consume_draft(&mut self, source: &SourceSnapshot) -> Result<(), HostError> {
        if source.identity.is_none() || !source.is_draft {
            return Err(HostError::new(format!(
                "draft changed: {}",
                source.path.display()
            )));
        }
        let Some(current) = self.model.sources.get(&source.path) else {
            self.model
                .kept_drafts
                .retain(|draft| draft.path != source.path);
            return Ok(());
        };
        if current != source {
            return Err(HostError::new(format!(
                "draft changed: {}",
                source.path.display()
            )));
        }
        self.model.sources.remove(&source.path);
        self.model
            .kept_drafts
            .retain(|draft| draft.path != source.path);
        self.refresh_original_source_projection(&source.path);
        Ok(())
    }

    fn refresh_original_source_projection(&mut self, source_path: &Path) {
        let sources = &self.model.sources;
        for entry in self.model.entries.values_mut().filter(|entry| {
            entry.summary.kind.as_str() != "command"
                && Path::new(&entry.settings.source) == source_path
        }) {
            let preserved = sources.contains_key(source_path);
            entry.detail.original_file_preserved = preserved;
        }
    }

    fn keep_draft(&mut self, path: &Path) -> Result<(), HostError> {
        let source = self
            .model
            .sources
            .get(path)
            .ok_or_else(|| HostError::new(format!("source not found: {}", path.display())))?;
        if !source.is_draft {
            return Err(HostError::new(format!(
                "source is not a kept draft: {}",
                path.display()
            )));
        }
        if !self
            .model
            .kept_drafts
            .iter()
            .any(|draft| draft.path == path)
        {
            self.model.kept_drafts.push(fixtures::draft_summary(source));
        }
        Ok(())
    }

    fn commit_entry(
        &mut self,
        entry: CreateEntry,
        source: Option<&SourceSnapshot>,
    ) -> Result<String, HostError> {
        if let Some(source) = source {
            if source.identity.is_none() {
                return Err(HostError::new(format!(
                    "source has no filesystem identity: {}",
                    source.path.display()
                )));
            }
            let current = self.model.sources.get(&source.path).ok_or_else(|| {
                HostError::new(format!("source not found: {}", source.path.display()))
            })?;
            if current != source {
                return Err(HostError::new(format!(
                    "source changed: {}",
                    source.path.display()
                )));
            }
        }
        if self
            .model
            .entries
            .values()
            .any(|current| current.summary.name == entry.name)
        {
            return Err(HostError::new(format!(
                "entry name already exists: {}",
                entry.name
            )));
        }
        let base = Slug::from_display_name(&entry.name);
        let selector = if !self.model.entries.contains_key(base.as_str()) {
            base.as_str().to_owned()
        } else {
            (2_u64..)
                .map(|suffix| format!("{}-{suffix}", base.as_str()))
                .find(|candidate| !self.model.entries.contains_key(candidate))
                .expect("the slug suffix space is not bounded")
        };
        let verified_source = source.map(|source| source.bytes.as_slice());
        let mut fixture = fixture_from_create(entry, verified_source);
        fixture.summary.slug = Slug::parse(&selector).expect("the allocated slug is valid");
        fixture.settings.selector = selector.clone();
        refresh_entry_projection(&mut fixture, &self.runner_names(), None, false);
        self.model.entries.insert(selector.clone(), fixture);
        self.model
            .virtual_directories
            .insert(PathBuf::from(format!("/fixtures/library/{selector}")));
        self.refresh_runner_pin_counts();
        Ok(selector)
    }

    fn save_runner(
        &mut self,
        mut request: RunnerSaveRequest,
        owner: RunnerSaveOwner,
    ) -> Result<Action, HostError> {
        request.name = request.name.trim().to_owned();
        if request.name.is_empty() {
            return Ok(runner_save_failure(
                owner,
                "runner name and command are required".to_owned(),
            ));
        }
        if let Err(error) = validate_runner_argv(&request.argv) {
            return Ok(runner_save_failure(
                owner,
                format!("invalid runner command: {error:?}"),
            ));
        }
        let next = match &request.target {
            RunnerSaveTarget::New => {
                if self
                    .model
                    .runners
                    .iter()
                    .any(|row| row.name.as_deref() == Some(request.name.as_str()))
                {
                    return Ok(runner_save_failure(
                        owner,
                        format!("runner already exists: {}", request.name),
                    ));
                }
                let mut rows = self.model.runners.clone();
                rows.push(new_runner_row(&request.name, &request.argv));
                rows
            }
            RunnerSaveTarget::Named { name, expected } => {
                if name != &request.name {
                    return Ok(runner_save_failure(
                        owner,
                        "runner save target has a different name".to_owned(),
                    ));
                }
                let current = self.named_runner_identities(name);
                if current.is_empty() || &current != expected {
                    return Ok(runner_save_failure(
                        owner,
                        format!("runner changed: {name}"),
                    ));
                }
                let mut rows = self.model.runners.clone();
                let first = rows
                    .iter()
                    .position(|row| row.name.as_deref() == Some(name))
                    .expect("validated runner has a row");
                rows[first] = new_runner_row(name, &request.argv);
                let mut seen = false;
                rows.retain(|row| {
                    if row.name.as_deref() != Some(name) {
                        true
                    } else if !seen {
                        seen = true;
                        true
                    } else {
                        false
                    }
                });
                rows
            }
            RunnerSaveTarget::RawRow { expected } => {
                let index = match self.exact_runner_index(expected) {
                    Ok(index) => index,
                    Err(error) => return Ok(runner_save_failure(owner, error.message)),
                };
                if !self.model.runners[index].is_editable() {
                    return Ok(runner_save_failure(
                        owner,
                        "runner row cannot be repaired".to_owned(),
                    ));
                }
                if self
                    .model
                    .runners
                    .iter()
                    .enumerate()
                    .any(|(row_index, row)| {
                        row_index != index && row.name.as_deref() == Some(request.name.as_str())
                    })
                {
                    return Ok(runner_save_failure(
                        owner,
                        format!("runner already exists: {}", request.name),
                    ));
                }
                let mut rows = self.model.runners.clone();
                rows[index] = new_runner_row(&request.name, &request.argv);
                rows
            }
        };
        self.commit_runner_rows(next);
        let message = format!("Runner {} saved.", request.name);
        Ok(match owner {
            RunnerSaveOwner::Manager => Action::Runners(RunnerManagerAction::MutationSucceeded {
                rows: self.model.runners.clone(),
                selected_name: Some(request.name),
                message,
            }),
            RunnerSaveOwner::Editor(owner) => Action::RunnerEditorSaved {
                owner,
                name: request.name,
                message,
            },
        })
    }

    fn remove_runner(&mut self, request: RunnerRemoveRequest) -> Result<Action, HostError> {
        let next = match &request {
            RunnerRemoveRequest::Named {
                name,
                expected,
                expected_pinned_count,
            } => {
                let current = self.named_runner_identities(name);
                let pinned = self.runner_pin_count(name);
                if current.is_empty() || &current != expected || pinned != *expected_pinned_count {
                    return Ok(Action::Runners(RunnerManagerAction::MutationFailed(
                        format!("runner changed: {name}"),
                    )));
                }
                self.model
                    .runners
                    .iter()
                    .filter(|row| row.name.as_deref() != Some(name))
                    .cloned()
                    .collect()
            }
            RunnerRemoveRequest::RawRow { expected } => {
                let index = match self.exact_runner_index(expected) {
                    Ok(index) => index,
                    Err(error) => {
                        return Ok(Action::Runners(RunnerManagerAction::MutationFailed(
                            error.message,
                        )));
                    }
                };
                let mut rows = self.model.runners.clone();
                rows.remove(index);
                rows
            }
        };
        self.commit_runner_rows(next);
        Ok(Action::Runners(RunnerManagerAction::MutationSucceeded {
            rows: self.model.runners.clone(),
            selected_name: None,
            message: "Runner removed.".to_owned(),
        }))
    }

    fn named_runner_identities(&self, name: &str) -> Vec<RunnerRowIdentity> {
        self.model
            .runners
            .iter()
            .filter(|row| row.name.as_deref() == Some(name))
            .map(|row| row.identity.clone())
            .collect()
    }

    fn exact_runner_index(&self, expected: &RunnerRowIdentity) -> Result<usize, HostError> {
        let matches = self
            .model
            .runners
            .iter()
            .enumerate()
            .filter(|(_, row)| &row.identity == expected)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => Ok(*index),
            [] => Err(HostError::new("runner row changed")),
            _ => Err(HostError::new("runner row identity is ambiguous")),
        }
    }

    fn runner_pin_count(&self, name: &str) -> usize {
        self.model
            .entries
            .values()
            .filter(|entry| {
                entry.summary.kind.as_str() == "prompt" && entry.settings.runner == name
            })
            .count()
    }

    fn commit_runner_rows(&mut self, rows: Vec<RunnerRow>) {
        self.model.runners = rows;
        for (index, row) in self.model.runners.iter_mut().enumerate() {
            let raw_index = if row.identity.index.is_none() && row.name.is_none() {
                None
            } else {
                Some(index)
            };
            row.identity = RunnerRowIdentity {
                index: raw_index,
                snapshot_token: runner_snapshot_token(index, row),
            };
        }
        let names = self.runner_names();
        let identities = self
            .model
            .runners
            .iter()
            .map(|row| row.identity.clone())
            .collect::<Vec<_>>();
        let key_identities = self
            .model
            .runners
            .iter()
            .map(|row| {
                row.name.as_ref().map_or_else(Vec::new, |name| {
                    self.model
                        .runners
                        .iter()
                        .enumerate()
                        .filter(|(_, candidate)| candidate.name.as_ref() == Some(name))
                        .map(|(candidate, _)| identities[candidate].clone())
                        .collect()
                })
            })
            .collect::<Vec<_>>();
        let pins = self
            .model
            .runners
            .iter()
            .map(|row| {
                row.name
                    .as_deref()
                    .map_or(0, |name| self.runner_pin_count(name))
            })
            .collect::<Vec<_>>();
        for (index, row) in self.model.runners.iter_mut().enumerate() {
            row.key_identities = key_identities[index].clone();
            row.pinned_count = pins[index];
        }
        self.model.preferences.runner_names = names;
        self.refresh_prompt_runner_details();
    }

    fn refresh_runner_pin_counts(&mut self) {
        let pins = self
            .model
            .runners
            .iter()
            .map(|row| {
                row.name
                    .as_deref()
                    .map_or(0, |name| self.runner_pin_count(name))
            })
            .collect::<Vec<_>>();
        for (row, count) in self.model.runners.iter_mut().zip(pins) {
            row.pinned_count = count;
        }
    }

    fn refresh_prompt_runner_details(&mut self) {
        let configured = self.runner_names();
        for entry in self.model.entries.values_mut() {
            if entry.summary.kind.as_str() != "prompt" {
                continue;
            }
            let runner = entry.settings.runner.clone();
            entry.detail.prompt_runner = Some(if runner.is_empty() {
                skit_ui::LibraryPromptRunner::PickOnRunForm
            } else if configured.contains(&runner) {
                skit_ui::LibraryPromptRunner::Configured(runner)
            } else {
                skit_ui::LibraryPromptRunner::Missing(runner)
            });
            entry.settings.configured_runners = configured.clone();
        }
    }

    fn runner_names(&self) -> Vec<String> {
        self.model
            .runners
            .iter()
            .filter(|row| row.is_valid())
            .filter_map(|row| row.name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    fn preferences_view(&self) -> PreferencesView {
        let mut snapshot = self.model.preferences.clone();
        snapshot.runner_names = self.runner_names();
        PreferencesView::new(PreferencesDraft::from_snapshot(snapshot))
    }

    fn serve_preferences(&mut self, effect: PreferencesEffect) -> Result<Action, HostError> {
        match effect {
            PreferencesEffect::None => Err(HostError::new(
                "protocol error: PreferencesEffect::None reached the host",
            )),
            PreferencesEffect::Close => Err(HostError::new(
                "protocol error: PreferencesEffect::Close reached the host",
            )),
            PreferencesEffect::ConfirmDiscard => Err(HostError::new(
                "protocol error: PreferencesEffect::ConfirmDiscard reached the host",
            )),
            PreferencesEffect::Save(change) => self.save_preferences(change),
            PreferencesEffect::ManageAgents => Ok(Action::Present(Screen::Runners(Box::new(
                RunnerManagerView::new(self.model.runners.clone()),
            )))),
            PreferencesEffect::DiscoverAgentSkillTargets => Ok(Action::Preferences(
                PreferencesAction::PresentAgentSkillTargets(self.model.agent_targets.clone()),
            )),
            PreferencesEffect::InstallAgentSkill { skills_dir } => {
                if !self
                    .model
                    .agent_targets
                    .iter()
                    .any(|target| target.skills_dir() == skills_dir)
                {
                    return Ok(Action::SetStatus(format!(
                        "Error: Agent Skill target not found: {}",
                        skills_dir.display()
                    )));
                }
                self.model.installed_skills.insert(skills_dir.clone());
                Ok(Action::Preferences(
                    PreferencesAction::AgentSkillInstalled {
                        message: format!(
                            "Installed the skit Agent Skill: {}",
                            skills_dir.join("skit/SKILL.md").display()
                        ),
                    },
                ))
            }
        }
    }

    fn save_preferences(&mut self, change: PreferencesChangeSet) -> Result<Action, HostError> {
        const KEYS: &[&str] = &[
            "lang",
            "editor",
            "form",
            "after_run",
            "js.runner",
            "shell.bash_path",
            "mirror",
            "mirror.pypi",
            "mirror.github",
            "mirror.npm",
        ];
        if let Some(key) = change
            .settings
            .keys()
            .find(|key| !KEYS.contains(&key.as_str()))
        {
            return Ok(Action::SetStatus(format!(
                "Error: preference key not found: {key}"
            )));
        }
        if let Err(error) = change.validate_files(|path| self.model.virtual_files.contains(path)) {
            return Ok(Action::Preferences(PreferencesAction::ValidationFailed(
                error,
            )));
        }
        let mut next = self.model.preferences.clone();
        for (key, value) in &change.settings {
            match key.as_str() {
                "lang" => {
                    if !value.is_empty() && !next.available_languages.contains(value) {
                        return Ok(Action::SetStatus(format!(
                            "Error: language not found: {value}"
                        )));
                    }
                    next.language = value.clone();
                    next.effective_language = if value.is_empty() {
                        "en".to_owned()
                    } else {
                        value.clone()
                    };
                }
                "editor" => next.editor = value.clone(),
                "form" => {
                    next.form = match value.as_str() {
                        "tui" => InteractiveFormChoice::Tui,
                        "plain" => InteractiveFormChoice::Plain,
                        _ => {
                            return Ok(Action::SetStatus(format!(
                                "Error: form choice not found: {value}"
                            )));
                        }
                    };
                }
                "after_run" => {
                    next.after_run = match value.as_str() {
                        "exit" => AfterRunChoice::Exit,
                        "stay" => AfterRunChoice::Stay,
                        _ => {
                            return Ok(Action::SetStatus(format!(
                                "Error: after-run choice not found: {value}"
                            )));
                        }
                    };
                }
                "js.runner" => {
                    next.javascript = match value.as_str() {
                        "" => JavascriptChoice::Automatic,
                        "deno" => JavascriptChoice::Deno,
                        "bun" => JavascriptChoice::Bun,
                        "node" => JavascriptChoice::Node,
                        _ => {
                            return Ok(Action::SetStatus(format!(
                                "Error: JavaScript runner not found: {value}"
                            )));
                        }
                    };
                }
                "shell.bash_path" => next.bash_path = Some(value.clone()),
                "mirror" => match value.as_str() {
                    "on" => next.mirror.enabled = true,
                    "off" => next.mirror.enabled = false,
                    _ => {
                        return Ok(Action::SetStatus(format!(
                            "Error: mirror choice not found: {value}"
                        )));
                    }
                },
                "mirror.pypi" => {
                    let value = match resolve_mirror_axis(value, MirrorAxis::Pypi) {
                        Ok(value) => value,
                        Err(error) => {
                            return Ok(Action::Preferences(PreferencesAction::ValidationFailed(
                                error,
                            )));
                        }
                    };
                    next.mirror.pypi = value;
                }
                "mirror.github" => {
                    let value = match resolve_mirror_axis(value, MirrorAxis::Github) {
                        Ok(value) => value,
                        Err(error) => {
                            return Ok(Action::Preferences(PreferencesAction::ValidationFailed(
                                error,
                            )));
                        }
                    };
                    if value.is_empty() {
                        next.mirror.python_install.clear();
                        next.mirror.uv_binary.clear();
                    } else {
                        next.mirror.python_install =
                            format!("{value}/astral-sh/python-build-standalone/");
                        next.mirror.uv_binary = format!("{value}/astral-sh/uv");
                    }
                }
                "mirror.npm" => {
                    let value = match resolve_mirror_axis(value, MirrorAxis::Npm) {
                        Ok(value) => value,
                        Err(error) => {
                            return Ok(Action::Preferences(PreferencesAction::ValidationFailed(
                                error,
                            )));
                        }
                    };
                    next.mirror.npm = value;
                }
                _ => unreachable!("preference keys were validated"),
            }
        }
        self.model.preferences = next;
        self.model.preference_settings.extend(change.settings);
        Ok(Action::PreferencesSaved {
            locale: self.model.preferences.effective_language.clone(),
            message: "Preferences saved".to_owned(),
        })
    }

    fn remove_entry(&mut self, selector: &str) -> Result<Action, HostError> {
        if !self.model.entries.contains_key(selector) {
            return Ok(Action::SetStatus(format!("entry not found: {selector}")));
        }
        self.model.entries.remove(selector);
        self.model
            .virtual_directories
            .remove(&PathBuf::from(format!("/fixtures/library/{selector}")));
        self.refresh_runner_pin_counts();
        self.model.rerunnable.remove(selector);
        self.model.last_runs.remove(selector);
        self.model.remembered_values.remove(selector);
        self.model.extra_args.remove(selector);
        self.model.health = self.current_health();
        self.complete("Entry removed")
    }

    fn complete(&self, message: &str) -> Result<Action, HostError> {
        Ok(Action::Complete {
            surface: Some(self.surface()),
            rerunnable: Some(self.rerunnable_slugs()),
            message: message.to_owned(),
        })
    }

    #[cfg(test)]
    fn snapshot(&self) -> HostSnapshot {
        self.model.clone()
    }

    #[cfg(test)]
    fn is_rerunnable(&self, selector: &str) -> bool {
        self.model.rerunnable.contains(selector)
    }

    #[cfg(test)]
    fn kept_draft_count(&self) -> usize {
        self.model.kept_drafts.len()
    }

    #[cfg(test)]
    fn kept_drafts(&self) -> &[DraftSummary] {
        &self.model.kept_drafts
    }

    #[cfg(test)]
    fn runner_rows(&self) -> &[RunnerRow] {
        &self.model.runners
    }

    #[cfg(test)]
    fn preferences_snapshot(&self) -> &skit_application::preferences::PreferencesSnapshot {
        &self.model.preferences
    }

    #[cfg(test)]
    fn last_runner(&self) -> Option<&str> {
        self.model.last_runner.as_deref()
    }

    #[cfg(test)]
    fn source_fixture(&self, path: &str) -> Option<&SourceSnapshot> {
        self.model.sources.get(Path::new(path))
    }

    #[cfg(test)]
    fn preset(&self, selector: &str, name: &str) -> Option<&BTreeMap<String, String>> {
        self.model.entries.get(selector)?.presets.get(name)
    }
}

fn validate_settings_keys(
    values: &BTreeMap<String, FieldValue>,
    entry: &EntryFixture,
) -> Result<(), HostError> {
    const EXACT: &[&str] = &[
        skit_ui::NAME_KEY,
        skit_ui::DESCRIPTION_KEY,
        skit_ui::WORKDIR_KEY,
        skit_ui::INTERPRETER_KEY,
        skit_ui::RUNNER_KEY,
        skit_ui::DEPENDENCIES_KEY,
        skit_ui::PYTHON_KEY,
        skit_ui::NEEDS_KEY,
        skit_ui::TEMPLATE_KEY,
        skit_ui::INTERPOLATE_KEY,
        skit_ui::RESYNC_KEY,
        skit_ui::MANAGE_KEY,
        "source:unmanage",
        skit_ui::NORMALIZE_KEY,
        skit_ui::ADD_PARAMETER_KEY,
        skit_ui::PROMPT_CANDIDATES_KEY,
        "parameter:remove",
    ];
    const PARAMETER_FIELDS: &[&str] = &[
        "keep",
        "type",
        "default",
        "choices",
        "help",
        "flag",
        "required",
        "prompt",
        "secret",
        "env_source",
    ];
    let additions = settings_list(values, skit_ui::ADD_PARAMETER_KEY)
        .into_iter()
        .chain(settings_list(values, skit_ui::PROMPT_CANDIDATES_KEY))
        .collect::<BTreeSet<_>>();
    for key in values.keys() {
        if EXACT.contains(&key.as_str()) {
            continue;
        }
        if let Some(name) = key.strip_prefix(skit_ui::PRESET_PREFIX) {
            if entry.presets.contains_key(name) {
                continue;
            }
            return Err(HostError::new(format!("preset not found: {name}")));
        }
        if let Some(suffix) = key.strip_prefix("parameter:")
            && let Some((name, field)) = suffix.rsplit_once(':')
            && PARAMETER_FIELDS.contains(&field)
            && (entry.declarations.iter().any(|item| item.name == name) || additions.contains(name))
        {
            continue;
        }
        return Err(HostError::new(format!("settings field not found: {key}")));
    }
    Ok(())
}

fn settings_text(values: &BTreeMap<String, FieldValue>, key: &str) -> Option<String> {
    values.get(key).map(FieldValue::as_text)
}

fn settings_list(values: &BTreeMap<String, FieldValue>, key: &str) -> Vec<String> {
    match values.get(key) {
        Some(FieldValue::Explicit(TypedValue::Choices(items) | TypedValue::Arguments(items))) => {
            items.clone()
        }
        Some(value) => value
            .as_text()
            .split(|character: char| character == ',' || character.is_whitespace())
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect(),
        None => Vec::new(),
    }
}

fn settings_bool(
    values: &BTreeMap<String, FieldValue>,
    key: &str,
) -> Result<Option<bool>, HostError> {
    values
        .get(key)
        .map(|value| settings_bool_value(value, key))
        .transpose()
}

fn settings_bool_value(value: &FieldValue, key: &str) -> Result<bool, HostError> {
    match value.as_text().trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" => Ok(true),
        "" | "false" | "no" | "0" | "off" => Ok(false),
        value => Err(HostError::new(format!(
            "invalid Boolean value for {key}: {value}"
        ))),
    }
}

fn apply_parameter_values(
    values: &BTreeMap<String, FieldValue>,
    declarations: &mut Vec<ParamDecl>,
) -> Result<(), HostError> {
    let mut remove = BTreeSet::new();
    for declaration in declarations.iter_mut() {
        let prefix = format!("parameter:{}", declaration.name);
        let get = |field: &str| settings_text(values, &format!("{prefix}:{field}"));
        if let Some(keep) = values.get(&format!("{prefix}:keep"))
            && !settings_bool_value(keep, &format!("{prefix}:keep"))?
        {
            remove.insert(declaration.name.clone());
            continue;
        }
        if let Some(value) = get("type") {
            declaration.parameter_type = match value.trim() {
                "" | "str" => ParameterType::Str,
                "int" => ParameterType::Int,
                "float" => ParameterType::Float,
                "bool" => ParameterType::Bool,
                "choice" => ParameterType::Choice,
                "path" => ParameterType::Path,
                value => {
                    return Err(HostError::new(format!("parameter type not found: {value}")));
                }
            };
        }
        if let Some(value) = get("choices") {
            declaration.choices = value
                .split(',')
                .map(str::trim)
                .filter(|choice| !choice.is_empty())
                .map(str::to_owned)
                .collect();
        }
        if let Some(value) = get("default") {
            declaration.default = parse_parameter_default(&value, declaration.parameter_type)?;
        }
        if let Some(value) = get("help") {
            declaration.help = value;
        }
        if let Some(value) = get("flag") {
            declaration.flag = value;
        }
        if let Some(value) = values.get(&format!("{prefix}:required")) {
            declaration.required = settings_bool_value(value, &format!("{prefix}:required"))?;
        }
        if let Some(value) = get("prompt") {
            declaration.prompt = value;
        }
        let was_secret = declaration.secret;
        if let Some(value) = values.get(&format!("{prefix}:secret")) {
            declaration.secret = settings_bool_value(value, &format!("{prefix}:secret"))?;
        }
        if let Some(value) = get("env_source") {
            declaration.env_source = value;
        }
        if declaration.secret && !was_secret {
            declaration.default = None;
        }
        if !declaration.secret {
            declaration.env_source.clear();
        }
        if declaration.validate().is_some() {
            return Err(HostError::new(format!(
                "parameter {} has incompatible settings",
                declaration.name
            )));
        }
    }
    declarations.retain(|item| !remove.contains(&item.name));
    Ok(())
}

fn parse_parameter_default(
    value: &str,
    parameter_type: ParameterType,
) -> Result<Option<ParameterValue>, HostError> {
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = match parameter_type {
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => {
            ParameterValue::String(value.to_owned())
        }
        ParameterType::Int => ParameterValue::Integer(
            value
                .parse()
                .map_err(|_| HostError::new(format!("invalid integer default: {value}")))?,
        ),
        ParameterType::Float => {
            let number = value
                .parse::<f64>()
                .map_err(|_| HostError::new(format!("invalid float default: {value}")))?;
            if !number.is_finite() {
                return Err(HostError::new(format!("invalid float default: {value}")));
            }
            ParameterValue::Float(number)
        }
        ParameterType::Bool => ParameterValue::Bool(match value.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => true,
            "false" | "no" | "0" | "off" => false,
            _ => return Err(HostError::new(format!("invalid Boolean default: {value}"))),
        }),
    };
    Ok(Some(parsed))
}

fn reconcile_command_parameters(template: &str, current: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut parameters = placeholder_params("command", template)
        .into_iter()
        .map(|detected| {
            current
                .iter()
                .find(|item| {
                    item.name == detected.name && item.delivery == ParameterDelivery::Placeholder
                })
                .cloned()
                .unwrap_or(detected)
        })
        .collect::<Vec<_>>();
    parameters.extend(
        current
            .iter()
            .filter(|item| item.delivery != ParameterDelivery::Placeholder)
            .cloned(),
    );
    parameters
}

fn refresh_entry_projection(
    entry: &mut EntryFixture,
    configured_runners: &[String],
    remembered: Option<&BTreeMap<String, String>>,
    has_run: bool,
) {
    entry.settings.name = entry.summary.name.clone();
    entry.settings.description = entry.summary.description.clone();
    entry.settings.managed = entry.declarations.clone();
    entry.settings.presets = entry.presets.clone();
    entry.settings.configured_runners = configured_runners.to_vec();
    entry.detail.template = (entry.summary.kind.as_str() == "command"
        && !entry.settings.template.is_empty())
    .then(|| entry.settings.template.clone());
    entry.detail.prompt_runner = (entry.summary.kind.as_str() == "prompt").then(|| {
        if entry.settings.runner.is_empty() {
            skit_ui::LibraryPromptRunner::PickOnRunForm
        } else if configured_runners.contains(&entry.settings.runner) {
            skit_ui::LibraryPromptRunner::Configured(entry.settings.runner.clone())
        } else {
            skit_ui::LibraryPromptRunner::Missing(entry.settings.runner.clone())
        }
    });
    let declarations = effective_declarations(entry).to_vec();
    entry.detail.parameters = declarations
        .iter()
        .map(|declaration| skit_ui::LibraryParameterDetail {
            key: declaration.name.clone(),
            value: if declaration.secret {
                String::new()
            } else {
                remembered
                    .and_then(|values| values.get(&declaration.name))
                    .cloned()
                    .or_else(|| declaration.default.as_ref().map(parameter_value_text))
                    .unwrap_or_default()
            },
            secret: declaration.secret,
        })
        .collect();
    entry.detail.presets = entry.presets.keys().cloned().collect();
    entry.detail.dependencies = entry.settings.effective_dependencies.clone();
    entry.detail.last_run = has_run.then(|| skit_ui::LibraryLastRun {
        at: "2026-08-27T12:00:00Z".to_owned(),
        age: skit_ui::LibraryRunAge::JustNow,
        exit: Some(0),
    });
}

fn effective_declarations(entry: &EntryFixture) -> &[ParamDecl] {
    if entry.summary.kind.as_str() == "prompt" && !entry.settings.interpolate {
        &[]
    } else {
        &entry.declarations
    }
}

fn parameter_value_text(value: &ParameterValue) -> String {
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => value.to_string(),
        ParameterValue::Bool(value) => value.to_string(),
    }
}

fn fixture_from_create(entry: CreateEntry, verified_source: Option<&[u8]>) -> EntryFixture {
    let slug = Slug::from_display_name(&entry.name);
    let selector = slug.as_str().to_owned();
    let bytes = entry
        .payload
        .as_ref()
        .map(|payload| payload.bytes.as_slice())
        .or(verified_source)
        .unwrap_or_default();
    let text = String::from_utf8_lossy(bytes);
    let source = entry.source.clone();
    let target = (entry.mode == skit_domain::StorageMode::Reference).then_some(source.clone());
    let summary = EntrySummary {
        slug,
        name: entry.name.clone(),
        kind: entry.kind.clone(),
        mode: entry.mode,
        description: entry.description.clone(),
        target,
    };
    let kind = entry.kind.as_str();
    let settings = entry.settings.clone();
    let source_owned = matches!(kind, "python" | "shell" | "fish" | "js" | "ts");
    let declarations = if kind == "prompt" {
        let mut declarations = placeholder_params("prompt", &text)
            .into_iter()
            .filter(|declaration| settings.params.contains(&declaration.name))
            .collect::<Vec<_>>();
        for explicit in &settings.parameters {
            if let Some(declaration) = declarations
                .iter_mut()
                .find(|declaration| declaration.name == explicit.name)
            {
                *declaration = explicit.clone();
            } else {
                declarations.push(explicit.clone());
            }
        }
        declarations
    } else if source_owned {
        managed_params(kind, &text)
    } else {
        settings.parameters.clone()
    };
    let candidates = if kind == "prompt" {
        placeholder_params("prompt", &text)
    } else if source_owned {
        detect_candidates(kind, &text)
    } else {
        Vec::new()
    }
    .into_iter()
    .map(|candidate| candidate.name)
    .filter(|name| {
        !declarations
            .iter()
            .any(|declaration| declaration.name == *name)
    })
    .collect();
    let has_stored_name = entry
        .payload
        .as_ref()
        .and_then(|payload| payload.stored_name.as_ref())
        .is_some();
    let supports_original_file = kind != "command";
    let original_file_preserved = supports_original_file && !entry.source.is_empty();
    let detail = skit_ui::LibraryEntryDetail {
        added_at: "2026-08-27T12:00:00Z".to_owned(),
        original_file_preserved,
        ..skit_ui::LibraryEntryDetail::default()
    };
    EntryFixture {
        settings: skit_ui::SettingsInputs {
            selector,
            kind: kind.to_owned(),
            name: entry.name,
            description: entry.description,
            source,
            reference_mode: entry.mode == skit_domain::StorageMode::Reference,
            workdir: entry.workdir,
            interpreter: settings.interpreter.clone(),
            runner: settings.runner.clone(),
            supports_modes: skit_application::supports_storage_modes(&entry.kind),
            has_original_file: supports_original_file,
            has_stored_name,
            pinnable_interpreter: matches!(
                kind,
                "shell" | "fish" | "powershell" | "ruby" | "perl" | "lua" | "r" | "js" | "ts"
            ),
            has_analyzer: matches!(kind, "python" | "shell" | "fish" | "js" | "ts"),
            declared_schema: matches!(
                kind,
                "command" | "prompt" | "exe" | "powershell" | "ruby" | "perl" | "lua" | "r"
            ),
            managed: declarations.clone(),
            candidates,
            template: settings.template,
            interpolate: settings.interpolate,
            dependency_flavor: match kind {
                "python" => Some(skit_ui::DependencyFlavor::Uv),
                "js" | "ts" => Some(skit_ui::DependencyFlavor::Npm),
                _ => None,
            },
            effective_dependencies: settings.dependencies,
            effective_requires_python: settings.requires_python,
            needs: settings.needs,
            ..skit_ui::SettingsInputs::default()
        },
        summary,
        detail,
        declarations,
        presets: BTreeMap::new(),
        normalized: BTreeSet::new(),
        resync_count: 0,
    }
}

fn new_runner_row(name: &str, argv: &[String]) -> RunnerRow {
    RunnerRow {
        identity: RunnerRowIdentity {
            index: None,
            snapshot_token: String::new(),
        },
        name: Some(name.to_owned()),
        argv: Some(argv.to_vec()),
        reason: None,
        descriptor: name.to_owned(),
        key_identities: Vec::new(),
        pinned_count: 0,
    }
}

fn runner_save_failure(owner: RunnerSaveOwner, message: String) -> Action {
    match owner {
        RunnerSaveOwner::Manager => Action::Runners(RunnerManagerAction::MutationFailed(message)),
        RunnerSaveOwner::Editor(owner) => Action::RunnerEditorSaveFailed { owner, message },
    }
}

fn runner_snapshot_token(index: usize, row: &RunnerRow) -> String {
    format!(
        "row:{index}:name={:?}:argv={:?}:reason={:?}:descriptor={:?}",
        row.name, row.argv, row.reason, row.descriptor
    )
}

#[derive(Clone, Copy)]
enum MirrorAxis {
    Pypi,
    Github,
    Npm,
}

fn resolve_mirror_axis(
    value: &str,
    axis: MirrorAxis,
) -> Result<String, skit_application::preferences::PreferencesError> {
    let preset = match (axis, value) {
        (MirrorAxis::Pypi, "tsinghua") => Some("https://pypi.tuna.tsinghua.edu.cn/simple"),
        (MirrorAxis::Pypi, "aliyun") => Some("https://mirrors.aliyun.com/pypi/simple"),
        (MirrorAxis::Pypi, "ustc") => Some("https://pypi.mirrors.ustc.edu.cn/simple"),
        (MirrorAxis::Github, "nju") => Some("https://mirror.nju.edu.cn/github-release"),
        (MirrorAxis::Npm, "npmmirror") => Some("https://registry.npmmirror.com"),
        _ => None,
    };
    if let Some(value) = preset {
        return Ok(value.to_owned());
    }
    if value == "off" {
        return Ok(String::new());
    }
    let value = value.trim_end_matches('/');
    let valid_token = (value.starts_with("https://") || value.starts_with("http://"))
        && !value.chars().any(char::is_whitespace)
        && !value.contains('·');
    let field = match axis {
        MirrorAxis::Pypi => skit_application::preferences::PreferencesField::PypiMirror,
        MirrorAxis::Github => skit_application::preferences::PreferencesField::GithubMirror,
        MirrorAxis::Npm => skit_application::preferences::PreferencesField::NpmMirror,
    };
    if !valid_token {
        return Err(skit_application::preferences::PreferencesError::CustomUrlRequired { field });
    }
    if matches!(axis, MirrorAxis::Github) && !value.starts_with("https://") {
        return Err(
            skit_application::preferences::PreferencesError::GithubHttpsRequired {
                url: value.to_owned(),
            },
        );
    }
    Ok(value.to_owned())
}

fn add_completion_message(warnings: &[String]) -> String {
    let mut message = "Entry added".to_owned();
    for warning in warnings {
        message.push_str("\nwarning: ");
        message.push_str(warning);
    }
    message
}

fn virtual_glob_count(cwd: &Path, piece: &str, paths: &BTreeSet<PathBuf>) -> usize {
    if !piece.contains(['*', '?', '[']) {
        return 1;
    }
    let input = Path::new(piece);
    let absolute = input.is_absolute();
    let pattern_path = if absolute {
        input.to_path_buf()
    } else {
        cwd.join(input)
    };
    let Ok(pattern) = glob::Pattern::new(&pattern_path.to_string_lossy()) else {
        return 1;
    };
    let options = glob::MatchOptions {
        case_sensitive: !cfg!(windows),
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    paths
        .iter()
        .filter(|path| pattern.matches_path_with(path, options))
        .filter(|path| hidden_segments_are_explicit(&pattern_path, path))
        .count()
        .max(1)
}

fn path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect()
}

fn hidden_segments_are_explicit(pattern: &Path, candidate: &Path) -> bool {
    hidden_parts_match(&path_components(pattern), &path_components(candidate))
}

fn hidden_parts_match(pattern: &[String], candidate: &[String]) -> bool {
    match (pattern.split_first(), candidate.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((part, rest)), _) if part == "**" => {
            hidden_parts_match(rest, candidate)
                || candidate.first().is_some_and(|value| {
                    !value.starts_with('.') && hidden_parts_match(pattern, &candidate[1..])
                })
        }
        (Some((part, rest)), Some((value, values))) => {
            (!value.starts_with('.') || part.starts_with('.')) && hidden_parts_match(rest, values)
        }
        (Some(_), None) => false,
    }
}

#[cfg(test)]
fn request_id(value: u64) -> AddRequestId {
    serde_json::from_value(serde_json::json!(value)).expect("request ID fixture is valid")
}

#[cfg(test)]
fn create_entry_fixture(name: &str, kind: &str) -> CreateEntry {
    CreateEntry {
        name: name.to_owned(),
        kind: skit_domain::EntryKind::parse(kind).expect("entry kind fixture is valid"),
        mode: skit_domain::StorageMode::Copy,
        source: "/fixtures/new.py".to_owned(),
        workdir: "origin".to_owned(),
        description: "Added fixture".to_owned(),
        payload: Some(EntryPayload {
            bytes: b"print('new')\n".to_vec(),
            stored_name: Some("new.py".to_owned()),
            permissions: skit_application::SourcePermissions::default(),
        }),
        settings: skit_domain::EntrySettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use skit_application::preferences::{MirrorChoice, PreferencesField};
    use skit_domain::parameters::ParamDecl;
    use skit_ui::{
        Action, AddAction, AddEffect, Effect, FieldValue, FormPurpose, HostRequest, KnownEntryKind,
        ModalState, PreferencesAction, PreferencesEffect, RunnerEditorAction, RunnerEditorOwner,
        RunnerManagerAction, RunnerRemoveRequest, RunnerSaveOwner, RunnerSaveRequest,
        RunnerSaveTarget, Screen, SettingsAction, TypedValue,
    };

    use super::*;

    #[test]
    fn fixtures_cover_each_required_host_surface() {
        let host = FakeHost::new();
        let state = host.initial_state();

        assert_eq!(state.entry_count(), 3);
        assert!(host.contains_selector("python-tool"));
        assert!(host.contains_selector("prompt-tool"));
        assert!(host.contains_selector("command-tool"));
        assert!(host.is_rerunnable("python-tool"));
        for entry in host.model.entries.values() {
            assert_eq!(
                host.model.rerunnable.contains(entry.summary.slug.as_str()),
                entry.detail.last_run.is_some(),
                "{}",
                entry.summary.slug
            );
        }
        assert_eq!(host.kept_draft_count(), 1);
        assert_eq!(host.runner_rows().len(), 2);
        assert!(host.runner_rows().iter().any(skit_ui::RunnerRow::is_valid));
        assert!(host.runner_rows().iter().any(|row| !row.is_valid()));
        assert!(
            host.model
                .sources
                .values()
                .all(|source| source.identity.is_some())
        );
        assert_eq!(
            host.model.entries["python-tool"]
                .detail
                .parameters
                .iter()
                .map(|parameter| parameter.key.as_str())
                .collect::<Vec<_>>(),
            ["NAME", "TOKEN", "OLD"]
        );
        assert_eq!(
            host.model.entries["command-tool"]
                .detail
                .parameters
                .iter()
                .map(|parameter| parameter.key.as_str())
                .collect::<Vec<_>>(),
            ["TARGET"]
        );
        assert_eq!(
            host.model.entries["prompt-tool"].settings.candidates,
            ["AUDIENCE"]
        );
        let projected = fixture_from_create(create_entry_fixture("Empty command", "command"), None);
        assert!(projected.settings.declared_schema);
        assert!(projected.declarations.is_empty());

        for (kind, has_analyzer) in [
            ("python", true),
            ("shell", true),
            ("fish", true),
            ("js", true),
            ("ts", true),
            ("powershell", false),
            ("ruby", false),
            ("perl", false),
            ("lua", false),
            ("r", false),
            ("command", false),
        ] {
            let mut create = create_entry_fixture("Analyzer contract", kind);
            create.source = "/fixtures/source".to_owned();
            let projected = fixture_from_create(create, None);
            assert_eq!(projected.settings.has_analyzer, has_analyzer, "{kind}");
            assert_eq!(
                projected.settings.has_original_file,
                kind != "command",
                "{kind}"
            );
            assert_eq!(
                projected.detail.original_file_preserved,
                kind != "command",
                "{kind}"
            );
        }
    }

    #[test]
    fn preserved_sources_and_found_tools_exist_in_the_same_virtual_snapshot() {
        let mut host = FakeHost::new();
        let (_, _, files) = host.file_picker_tree();
        for entry in host.model.entries.values().filter(|entry| {
            entry.summary.mode == skit_domain::StorageMode::Copy
                && entry.summary.kind.as_str() != "command"
        }) {
            assert_eq!(
                entry.detail.original_file_preserved,
                host.model
                    .sources
                    .contains_key(Path::new(&entry.settings.source)),
                "{}",
                entry.summary.slug
            );
        }
        let preserved = host
            .model
            .entries
            .values()
            .filter(|entry| entry.detail.original_file_preserved)
            .map(|entry| PathBuf::from(&entry.settings.source))
            .collect::<Vec<_>>();
        assert!(!preserved.is_empty());
        for (index, path) in preserved.into_iter().enumerate() {
            assert!(
                files.contains(&path),
                "missing picker source: {}",
                path.display()
            );
            let response = host
                .serve(Effect::Add(vec![AddEffect::InspectSource {
                    request: request_id(60 + index as u64),
                    path: path.clone(),
                }]))
                .unwrap();
            assert!(matches!(
                response,
                Action::Add(AddAction::SourceInspected {
                    request,
                    result: Ok(ref source),
                }) if request == request_id(60 + index as u64) && source.path == path
            ));
        }
        if let skit_application::health::UvHealth::Found(path) = &host.model.health.uv {
            assert!(
                files.contains(Path::new(path)),
                "missing health tool: {path}"
            );
        }
    }

    #[test]
    fn opens_every_screen_from_the_current_model() {
        let mut host = FakeHost::new();
        let cases = [
            (HostRequest::Run, Some("python-tool"), "run"),
            (HostRequest::Add, None, "add"),
            (HostRequest::Settings, Some("python-tool"), "settings"),
            (HostRequest::Preferences, None, "preferences"),
            (HostRequest::Health, None, "health"),
            (HostRequest::Runners, None, "runners"),
            (HostRequest::Presets, Some("python-tool"), "settings"),
            (HostRequest::Rename, Some("python-tool"), "form"),
        ];

        for (request, selector, expected) in cases {
            let action = host
                .serve(Effect::Open {
                    request,
                    selector: selector.map(str::to_owned),
                })
                .unwrap();
            let Action::Present(screen) = action else {
                panic!("open must present a screen: {action:?}");
            };
            let actual = match screen {
                Screen::Run(_) => "run",
                Screen::Add(_) => "add",
                Screen::Settings(_) => "settings",
                Screen::Preferences(_) => "preferences",
                Screen::Health(_) => "health",
                Screen::Runners(_) => "runners",
                Screen::Form(_) => "form",
                Screen::Library | Screen::Report(_) => "other",
            };
            assert_eq!(actual, expected, "wrong screen for {request:?}");
        }
    }

    #[test]
    fn invalid_selector_and_request_pairs_do_not_mutate_the_model() {
        let mut host = FakeHost::new();
        let before = host.snapshot();

        assert!(
            host.serve(Effect::Open {
                request: HostRequest::Run,
                selector: Some("missing".to_owned()),
            })
            .is_err()
        );
        assert!(
            host.serve(Effect::Open {
                request: HostRequest::Add,
                selector: Some("python-tool".to_owned()),
            })
            .is_err()
        );
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn remove_and_draft_delete_commit_only_after_exact_validation() {
        let mut host = FakeHost::new();
        let before = host.snapshot();
        assert!(matches!(
            host.serve(Effect::Remove {
                selector: "missing".to_owned(),
            })
            .unwrap(),
            Action::SetStatus(_)
        ));
        assert_eq!(host.snapshot(), before);

        let draft = host.kept_drafts()[0].clone();
        let response = host
            .serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: request_id(7),
                draft: draft.clone(),
            }]))
            .unwrap();
        assert!(matches!(response, Action::Add(_)));
        assert!(host.kept_drafts().is_empty());

        let after_delete = host.snapshot();
        assert!(matches!(
            host.serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: request_id(8),
                draft,
            }]))
            .unwrap(),
            Action::Add(AddAction::DraftDeleted { result: Err(_), .. })
        ));
        assert_eq!(host.snapshot(), after_delete);

        host.serve(Effect::Remove {
            selector: "command-tool".to_owned(),
        })
        .unwrap();
        assert!(!host.contains_selector("command-tool"));
    }

    #[test]
    fn ordered_add_effects_apply_prefix_mutations_and_return_at_the_terminal_effect() {
        let mut host = FakeHost::new();
        let source = host.source_fixture("/fixtures/new.py").unwrap().clone();
        let kept = host.kept_drafts()[0].path.clone();
        let create = create_entry_fixture("Added by host", "python");

        let action = host
            .serve(Effect::Add(vec![
                AddEffect::RememberRunner("codex".to_owned()),
                AddEffect::DraftKept(kept),
                AddEffect::Commit {
                    request: request_id(9),
                    entry: Box::new(create),
                    source: Some(source),
                },
                AddEffect::Complete("must-not-run".to_owned()),
            ]))
            .unwrap();

        assert!(matches!(action, Action::Add(_)));
        assert_eq!(host.last_runner(), Some("codex"));
        assert!(host.contains_selector("added-by-host"));
        assert!(!host.contains_selector("must-not-run"));
    }

    #[test]
    fn stale_runner_cas_targets_refuse_without_mutation() {
        let mut host = FakeHost::new();
        let before = host.snapshot();
        let request = RunnerSaveRequest {
            name: "codex".to_owned(),
            argv: vec!["codex".to_owned(), "{{prompt}}".to_owned()],
            target: RunnerSaveTarget::Named {
                name: "codex".to_owned(),
                expected: Vec::new(),
            },
        };

        assert!(matches!(
            host.serve(Effect::SaveRunner {
                request,
                owner: RunnerSaveOwner::Manager,
            })
            .unwrap(),
            Action::Runners(RunnerManagerAction::MutationFailed(_))
        ));
        assert_eq!(host.snapshot(), before);

        let stale_remove = RunnerRemoveRequest::Named {
            name: "codex".to_owned(),
            expected: Vec::new(),
            expected_pinned_count: 1,
        };
        assert!(matches!(
            host.serve(Effect::RemoveRunner(stale_remove)).unwrap(),
            Action::Runners(RunnerManagerAction::MutationFailed(_))
        ));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn valid_runner_save_and_remove_update_every_projection() {
        let mut host = FakeHost::new();
        let identity = host
            .runner_rows()
            .iter()
            .find(|row| row.name.as_deref() == Some("codex"))
            .unwrap()
            .key_identities
            .clone();
        let response = host
            .serve(Effect::SaveRunner {
                request: RunnerSaveRequest {
                    name: "codex".to_owned(),
                    argv: vec![
                        "codex".to_owned(),
                        "exec".to_owned(),
                        "{{prompt}}".to_owned(),
                    ],
                    target: RunnerSaveTarget::Named {
                        name: "codex".to_owned(),
                        expected: identity,
                    },
                },
                owner: RunnerSaveOwner::Manager,
            })
            .unwrap();
        assert!(matches!(
            response,
            Action::Runners(RunnerManagerAction::MutationSucceeded { .. })
        ));

        let current = host
            .runner_rows()
            .iter()
            .find(|row| row.name.as_deref() == Some("codex"))
            .unwrap();
        let remove = RunnerRemoveRequest::Named {
            name: "codex".to_owned(),
            expected: current.key_identities.clone(),
            expected_pinned_count: current.pinned_count,
        };
        host.serve(Effect::RemoveRunner(remove)).unwrap();
        assert!(
            host.runner_rows()
                .iter()
                .all(|row| row.name.as_deref() != Some("codex"))
        );
        assert!(
            !host
                .preferences_snapshot()
                .runner_names
                .contains(&"codex".to_owned())
        );
    }

    #[test]
    fn preset_and_preferences_saves_persist_real_model_values() {
        let mut host = FakeHost::new();
        host.serve(Effect::SaveRunPreset {
            selector: "python-tool".to_owned(),
            name: "nightly".to_owned(),
            values: BTreeMap::from([
                ("NAME".to_owned(), "Ada".to_owned()),
                ("TOKEN".to_owned(), "do-not-store".to_owned()),
            ]),
            secret_names: ["TOKEN".to_owned()].into_iter().collect(),
        })
        .unwrap();
        assert_eq!(
            host.preset("python-tool", "nightly").unwrap(),
            &BTreeMap::from([("NAME".to_owned(), "Ada".to_owned())])
        );

        host.serve(Effect::Preferences(PreferencesEffect::Save(
            skit_application::preferences::PreferencesChangeSet {
                settings: BTreeMap::from([
                    ("lang".to_owned(), "zh-TW".to_owned()),
                    ("editor".to_owned(), "micro".to_owned()),
                ]),
            },
        )))
        .unwrap();
        assert_eq!(host.preferences_snapshot().language, "zh-TW");
        assert_eq!(host.preferences_snapshot().editor, "micro");
    }

    #[test]
    fn generic_submit_rejects_wrong_purpose_without_mutation() {
        let mut host = FakeHost::new();
        let before = host.snapshot();
        assert!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Add,
                selector: Some("python-tool".to_owned()),
                values: BTreeMap::new(),
            })
            .is_err()
        );
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn add_and_runner_refusals_return_to_the_exact_reducer_owner() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let open = state.update(Action::OpenAdd);
        let response = host.serve(open).expect("the add screen opens");
        assert_eq!(state.update(response), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SetSourcePath(
                "/fixtures/missing.py".to_owned(),
            ))),
            Effect::None
        );
        let inspect = state.update(Action::Add(AddAction::Continue));
        let request = match &inspect {
            Effect::Add(effects) => match effects.as_slice() {
                [AddEffect::InspectSource { request, .. }] => *request,
                _ => panic!("unexpected add effects: {effects:?}"),
            },
            _ => panic!("unexpected add effect: {inspect:?}"),
        };
        let before = host.snapshot();
        let response = host
            .serve(inspect)
            .expect("an inspection refusal belongs to Add");
        assert!(matches!(
            response,
            Action::Add(AddAction::SourceInspected {
                request: echoed,
                result: Err(_),
            }) if echoed == request
        ));
        assert_eq!(state.update(response), Effect::None);
        assert!(state.add_workflow().unwrap().problem().is_some());
        assert_eq!(host.snapshot(), before);

        let open = state.update(Action::Back);
        assert_eq!(open, Effect::None);
        let open = state.update(Action::OpenRunners);
        let response = host.serve(open).expect("the runner manager opens");
        assert_eq!(state.update(response), Effect::None);
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::New)),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::Editor(
                RunnerEditorAction::SetName("codex".to_owned()),
            ))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::Editor(
                RunnerEditorAction::SetCommand("codex {{prompt}}".to_owned()),
            ))),
            Effect::None
        );
        let save = state.update(Action::Runners(RunnerManagerAction::Editor(
            RunnerEditorAction::Submit,
        )));
        assert!(matches!(
            save,
            Effect::SaveRunner {
                owner: RunnerSaveOwner::Manager,
                ..
            }
        ));
        let before = host.snapshot();
        let response = host
            .serve(save)
            .expect("a duplicate runner refusal belongs to the manager");
        assert!(matches!(
            response,
            Action::Runners(RunnerManagerAction::MutationFailed(_))
        ));
        assert_eq!(state.update(response), Effect::None);
        let Screen::Runners(manager) = state.screen() else {
            panic!("the manager must keep ownership");
        };
        assert!(manager.editor().unwrap().host_error().is_some());
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn settings_validate_the_complete_transaction_before_commit() {
        let mut host = FakeHost::new();
        let values = BTreeMap::from([
            (
                skit_ui::NAME_KEY.to_owned(),
                FieldValue::text("Python saved"),
            ),
            (
                skit_ui::DESCRIPTION_KEY.to_owned(),
                FieldValue::text("Saved description"),
            ),
            (
                skit_ui::WORKDIR_KEY.to_owned(),
                FieldValue::text("/fixtures/work"),
            ),
            (
                skit_ui::DEPENDENCIES_KEY.to_owned(),
                FieldValue::Explicit(TypedValue::Arguments(vec![
                    "requests>=2".to_owned(),
                    "rich".to_owned(),
                ])),
            ),
            (skit_ui::PYTHON_KEY.to_owned(), FieldValue::text(">=3.12")),
            (
                skit_ui::NEEDS_KEY.to_owned(),
                FieldValue::Explicit(TypedValue::Choices(vec!["git".to_owned()])),
            ),
            (skit_ui::RESYNC_KEY.to_owned(), FieldValue::boolean(true)),
            (
                skit_ui::MANAGE_KEY.to_owned(),
                FieldValue::Explicit(TypedValue::Choices(vec!["SOURCE_CONST".to_owned()])),
            ),
            (
                "source:unmanage".to_owned(),
                FieldValue::Explicit(TypedValue::Choices(vec!["OLD".to_owned()])),
            ),
            (
                skit_ui::NORMALIZE_KEY.to_owned(),
                FieldValue::Explicit(TypedValue::Choices(vec!["NAME".to_owned()])),
            ),
            (
                skit_ui::ADD_PARAMETER_KEY.to_owned(),
                FieldValue::Explicit(TypedValue::Choices(vec!["EXTRA".to_owned()])),
            ),
            (
                "parameter:remove".to_owned(),
                FieldValue::Explicit(TypedValue::Choices(vec!["TOKEN".to_owned()])),
            ),
            ("parameter:NAME:type".to_owned(), FieldValue::text("choice")),
            (
                "parameter:NAME:choices".to_owned(),
                FieldValue::text("Ada, Grace"),
            ),
            ("parameter:NAME:default".to_owned(), FieldValue::text("Ada")),
            (
                "parameter:NAME:required".to_owned(),
                FieldValue::boolean(true),
            ),
            (
                "parameter:NAME:prompt".to_owned(),
                FieldValue::text("Person"),
            ),
            (
                "parameter:NAME:help".to_owned(),
                FieldValue::text("Choose a person."),
            ),
            (skit_ui::preset_key("friendly"), FieldValue::boolean(false)),
        ]);
        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Settings,
                selector: Some("python-tool".to_owned()),
                values,
            })
            .expect("valid settings save");
        assert!(matches!(response, Action::Complete { .. }));

        let saved = host.model.entries.get("python-tool").unwrap();
        assert_eq!(saved.summary.name, "Python saved");
        assert_eq!(saved.settings.workdir, "/fixtures/work");
        assert_eq!(saved.settings.interpreter, "");
        assert_eq!(
            saved.settings.effective_dependencies,
            ["requests>=2", "rich"]
        );
        assert_eq!(saved.settings.effective_requires_python, ">=3.12");
        assert_eq!(saved.settings.needs, ["git"]);
        assert_eq!(
            saved
                .declarations
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            ["NAME", "SOURCE_CONST", "EXTRA"]
        );
        let name = &saved.declarations[0];
        assert_eq!(
            name.parameter_type,
            skit_domain::parameters::ParameterType::Choice
        );
        assert_eq!(name.choices, ["Ada", "Grace"]);
        assert!(name.required);
        assert!(!saved.presets.contains_key("friendly"));

        let action = host
            .serve(Effect::Open {
                request: HostRequest::Settings,
                selector: Some("python-tool".to_owned()),
            })
            .unwrap();
        let Action::Present(Screen::Settings(view)) = action else {
            panic!("settings must reopen");
        };
        assert_eq!(
            view.field(skit_ui::NAME_KEY).unwrap().value().as_text(),
            "Python saved"
        );
        assert_eq!(
            view.field(skit_ui::WORKDIR_PATH_KEY)
                .unwrap()
                .value()
                .as_text(),
            "/fixtures/work"
        );

        let before = host.snapshot();
        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Settings,
                selector: Some("python-tool".to_owned()),
                values: BTreeMap::from([
                    (
                        skit_ui::NAME_KEY.to_owned(),
                        FieldValue::text("Must roll back"),
                    ),
                    ("zz-unknown".to_owned(), FieldValue::text("late")),
                ]),
            })
            .expect("a settings refusal is a typed host response");
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn draft_delete_checks_identity_content_and_request_before_mutation() {
        let exact_id = request_id(31);
        let mut exact = FakeHost::new();
        let draft = exact.kept_drafts()[0].clone();
        assert!(draft.identity.is_some());
        let response = exact
            .serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: exact_id,
                draft,
            }]))
            .unwrap();
        assert!(matches!(
            response,
            Action::Add(AddAction::DraftDeleted {
                request,
                result: Ok(DraftDeleteOutcome::Removed),
            }) if request == exact_id
        ));

        let mut absent = FakeHost::new();
        let draft = absent.kept_drafts()[0].clone();
        absent.model.sources.remove(&draft.path);
        let response = absent
            .serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: request_id(32),
                draft,
            }]))
            .unwrap();
        assert!(matches!(
            response,
            Action::Add(AddAction::DraftDeleted {
                request,
                result: Ok(DraftDeleteOutcome::AlreadyMissing),
            }) if request == request_id(32)
        ));
        assert!(absent.kept_drafts().is_empty());

        let mut changed = FakeHost::new();
        let claimed = changed.kept_drafts()[0].clone();
        let current = changed.model.sources.get_mut(&claimed.path).unwrap();
        current.bytes.push(b'!');
        current.identity = Some(skit_application::SourceIdentity::unix(9, 9, 9, 9));
        let response = changed
            .serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: request_id(33),
                draft: claimed.clone(),
            }]))
            .unwrap();
        let Action::Add(AddAction::DraftDeleted {
            request,
            result: Ok(DraftDeleteOutcome::Changed(refreshed)),
        }) = response
        else {
            panic!("changed draft must return a refreshed row");
        };
        assert_eq!(request, request_id(33));
        assert_ne!(refreshed.identity, claimed.identity);
        assert_eq!(changed.kept_drafts(), &[refreshed]);
        assert!(changed.model.sources.contains_key(&claimed.path));

        let mut legacy = FakeHost::new();
        legacy.model.kept_drafts[0].identity = None;
        let draft = legacy.kept_drafts()[0].clone();
        let before = legacy.snapshot();
        let response = legacy
            .serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: request_id(34),
                draft,
            }]))
            .unwrap();
        assert!(matches!(
            response,
            Action::Add(AddAction::DraftDeleted {
                request,
                result: Err(_),
            }) if request == request_id(34)
        ));
        assert_eq!(legacy.snapshot(), before);
    }

    #[test]
    fn add_cleanup_failure_keeps_the_committed_entry_and_draft() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let open = state.update(Action::OpenAdd);
        let response = host.serve(open).unwrap();
        assert_eq!(state.update(response), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SelectDraft(0))),
            Effect::None
        );
        let inspect = state.update(Action::Add(AddAction::Continue));
        let response = host.serve(inspect).unwrap();
        assert_eq!(state.update(response), Effect::None);
        assert!(state.add_workflow().unwrap().review().is_some());
        let commit = state.update(Action::Add(AddAction::Save));
        let response = host.serve(commit).unwrap();
        let cleanup = state.update(response);
        let expected = match &cleanup {
            Effect::Add(effects) => match effects.as_slice() {
                [AddEffect::ConsumeDraft(source), AddEffect::Complete(_)] => source.clone(),
                _ => panic!("unexpected cleanup effects: {effects:?}"),
            },
            _ => panic!("unexpected cleanup effect: {cleanup:?}"),
        };
        host.model
            .sources
            .get_mut(&expected.path)
            .unwrap()
            .bytes
            .push(b'!');
        let response = host
            .serve(cleanup)
            .expect("cleanup failure must not hide commit completion");
        let message = match &response {
            Action::AddCompleted { message, .. } | Action::Complete { message, .. } => message,
            _ => panic!("unexpected completion: {response:?}"),
        };
        assert!(message.contains("warning:"));
        assert!(host.model.sources.contains_key(&expected.path));
        assert!(host.contains_selector("skit-new-kept"));
        assert!(
            host.model.entries["skit-new-kept"]
                .detail
                .original_file_preserved
        );
        assert_eq!(state.update(response), Effect::None);
        assert_eq!(state.entry_count(), 4);
    }

    #[test]
    fn add_cleanup_success_recomputes_the_original_source_projection() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let open = state.update(Action::OpenAdd);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SelectDraft(0))),
            Effect::None
        );
        let inspect = state.update(Action::Add(AddAction::Continue));
        assert_eq!(round_trip(&mut host, &mut state, inspect), Effect::None);
        let commit = state.update(Action::Add(AddAction::Save));
        let response = host.serve(commit).unwrap();
        let cleanup = state.update(response);
        let response = host.serve(cleanup).unwrap();
        assert_eq!(state.update(response), Effect::None);
        assert!(
            !host
                .model
                .sources
                .contains_key(Path::new("/fixtures/drafts/skit-new-kept.py"))
        );
        assert!(host.contains_selector("skit-new-kept"));
        assert!(
            !host.model.entries["skit-new-kept"]
                .detail
                .original_file_preserved
        );
        assert!(
            host.model.entries["skit-new-kept"]
                .settings
                .has_original_file
        );
    }

    #[test]
    fn picker_tree_tracks_author_delete_consume_and_refused_cleanup() {
        let mut authored = FakeHost::new();
        let response = authored
            .serve(Effect::Add(vec![AddEffect::AuthorDraft {
                request: request_id(50),
                kind: DraftKind::Script,
            }]))
            .unwrap();
        let Action::Add(AddAction::DraftEdited {
            request,
            result: Ok(Some(source)),
        }) = response
        else {
            panic!("the author effect must return its source snapshot");
        };
        assert_eq!(request, request_id(50));
        assert!(authored.file_picker_tree().2.contains(&source.path));
        let draft = authored
            .kept_drafts()
            .iter()
            .find(|draft| draft.path == source.path)
            .unwrap()
            .clone();
        let response = authored
            .serve(Effect::Add(vec![AddEffect::DeleteDraft {
                request: request_id(51),
                draft,
            }]))
            .unwrap();
        assert!(matches!(
            response,
            Action::Add(AddAction::DraftDeleted {
                request,
                result: Ok(DraftDeleteOutcome::Removed),
            }) if request == request_id(51)
        ));
        assert!(!authored.file_picker_tree().2.contains(&source.path));

        let mut consumed = FakeHost::new();
        let source = consumed
            .model
            .sources
            .get(Path::new("/fixtures/drafts/skit-new-kept.py"))
            .unwrap()
            .clone();
        assert!(consumed.file_picker_tree().2.contains(&source.path));
        consumed
            .serve(Effect::Add(vec![
                AddEffect::ConsumeDraft(source.clone()),
                AddEffect::Complete("skit-new-kept".to_owned()),
            ]))
            .unwrap();
        assert!(!consumed.file_picker_tree().2.contains(&source.path));

        let mut refused = FakeHost::new();
        let claimed = refused
            .model
            .sources
            .get(Path::new("/fixtures/drafts/skit-new-kept.py"))
            .unwrap()
            .clone();
        refused
            .model
            .sources
            .get_mut(&claimed.path)
            .unwrap()
            .bytes
            .push(b'!');
        let before = refused.file_picker_tree();
        refused
            .serve(Effect::Add(vec![
                AddEffect::ConsumeDraft(claimed),
                AddEffect::Complete("skit-new-kept".to_owned()),
            ]))
            .unwrap();
        assert_eq!(refused.file_picker_tree(), before);
    }

    #[test]
    fn dynamic_sources_drive_glob_counts_from_the_same_virtual_snapshot() {
        let mut host = FakeHost::new();
        let count = |host: &FakeHost| {
            virtual_glob_count(
                Path::new("/fixtures/drafts"),
                "skit-new-*.py",
                &host.all_virtual_paths(),
            )
        };
        assert_eq!(count(&host), 1);
        let response = host
            .serve(Effect::Add(vec![AddEffect::AuthorDraft {
                request: request_id(70),
                kind: DraftKind::Script,
            }]))
            .unwrap();
        let Action::Add(AddAction::DraftEdited {
            result: Ok(Some(source)),
            ..
        }) = response
        else {
            panic!("the draft must be authored");
        };
        assert_eq!(count(&host), 2);
        let draft = host
            .kept_drafts()
            .iter()
            .find(|draft| draft.path == source.path)
            .unwrap()
            .clone();
        host.serve(Effect::Add(vec![AddEffect::DeleteDraft {
            request: request_id(71),
            draft,
        }]))
        .unwrap();
        assert_eq!(count(&host), 1);
    }

    #[test]
    fn add_allocates_slug_suffixes_but_refuses_exact_display_name_duplicates() {
        let mut host = FakeHost::new();
        let before = host.snapshot();
        let exact = host
            .serve(Effect::Add(vec![AddEffect::Commit {
                request: request_id(72),
                entry: Box::new(create_entry_fixture("Python tool", "python")),
                source: None,
            }]))
            .unwrap();
        assert!(matches!(
            exact,
            Action::Add(AddAction::CommitFinished { result: Err(_), .. })
        ));
        assert_eq!(host.snapshot(), before);

        let allocated = host
            .serve(Effect::Add(vec![AddEffect::Commit {
                request: request_id(73),
                entry: Box::new(create_entry_fixture("Python-tool", "python")),
                source: None,
            }]))
            .unwrap();
        assert!(matches!(
            allocated,
            Action::Add(AddAction::CommitFinished {
                request,
                result: Ok(ref slug),
            }) if request == request_id(73) && slug == "python-tool-2"
        ));
        assert!(host.contains_selector("python-tool-2"));
    }

    #[test]
    fn health_rebuild_uses_current_needs_runners_and_mirror_state() {
        let mut host = FakeHost::new();
        assert!(host.current_health().issues.iter().any(|issue| {
            issue.slug == "command-tool"
                && matches!(
                    issue.kind,
                    skit_application::health::HealthIssueKind::MissingNeeds { .. }
                )
        }));
        host.serve(Effect::Submit {
            purpose: FormPurpose::Settings,
            selector: Some("command-tool".to_owned()),
            values: BTreeMap::from([(skit_ui::NEEDS_KEY.to_owned(), FieldValue::text(""))]),
        })
        .unwrap();
        assert!(!host.current_health().issues.iter().any(|issue| {
            issue.slug == "command-tool"
                && matches!(
                    issue.kind,
                    skit_application::health::HealthIssueKind::MissingNeeds { .. }
                )
        }));

        host.model
            .entries
            .get_mut("prompt-tool")
            .unwrap()
            .settings
            .runner = "missing".to_owned();
        assert!(host.current_health().issues.iter().any(|issue| {
            issue.slug == "prompt-tool"
                && matches!(
                    issue.kind,
                    skit_application::health::HealthIssueKind::LaunchBlocked { .. }
                )
        }));
        host.model
            .entries
            .get_mut("prompt-tool")
            .unwrap()
            .settings
            .needs = vec!["missing-prompt-tool".to_owned()];
        let prompt_issues = host
            .current_health()
            .issues
            .into_iter()
            .filter(|issue| issue.slug == "prompt-tool")
            .collect::<Vec<_>>();
        assert_eq!(prompt_issues.len(), 1);
        assert!(matches!(
            prompt_issues[0].kind,
            skit_application::health::HealthIssueKind::MissingNeeds { .. }
        ));
        host.serve(Effect::Preferences(PreferencesEffect::Save(
            PreferencesChangeSet {
                settings: BTreeMap::from([
                    ("mirror".to_owned(), "off".to_owned()),
                    (
                        "mirror.pypi".to_owned(),
                        "https://mirror.invalid/simple".to_owned(),
                    ),
                    ("mirror.github".to_owned(), "off".to_owned()),
                    ("mirror.npm".to_owned(), "off".to_owned()),
                ]),
            },
        )))
        .unwrap();
        assert!(matches!(
            host.current_health().mirror,
            skit_application::health::MirrorHealth::Paused { .. }
        ));
    }

    #[test]
    fn run_refuses_unknown_parameter_and_reserved_keys_without_mutation() {
        let mut host = FakeHost::new();
        for values in [
            BTreeMap::from([("value:UNKNOWN".to_owned(), FieldValue::text("must not run"))]),
            BTreeMap::from([("_skit_unknown".to_owned(), FieldValue::text("must not run"))]),
            BTreeMap::from([("_skit_dry_run".to_owned(), FieldValue::text("sometimes"))]),
        ] {
            let before = host.snapshot();
            let response = host
                .serve(Effect::Submit {
                    purpose: FormPurpose::Run,
                    selector: Some("python-tool".to_owned()),
                    values,
                })
                .unwrap();
            assert!(matches!(response, Action::SetStatus(_)));
            assert_eq!(host.snapshot(), before);
        }
    }

    #[test]
    fn reducer_run_hidden_keys_round_trip_for_each_entry_surface() {
        let mut host = FakeHost::new();
        for selector in ["python-tool", "prompt-tool", "command-tool"] {
            let mut state = host.initial_state();
            select_entry(&mut state, selector);
            let open = state.update(Action::OpenRun);
            assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
            if selector == "python-tool" {
                let preset = state
                    .run_form()
                    .unwrap()
                    .fields()
                    .iter()
                    .position(|field| field.key == "_skit_preset")
                    .unwrap();
                assert_eq!(
                    state.update(Action::SelectFieldOption {
                        field: preset,
                        value: "friendly".to_owned(),
                    }),
                    Effect::None
                );
            }
            let submit = state.update(Action::Submit);
            let Effect::Submit { values, .. } = &submit else {
                panic!("run submit must use the host form contract: {submit:?}");
            };
            assert!(values.contains_key("_skit_args"));
            assert!(values.contains_key("_skit_save_preset"));
            assert!(values.contains_key("_skit_dry_run"));
            if selector == "python-tool" {
                assert_eq!(
                    values.get("_skit_preset").map(FieldValue::as_text),
                    Some("friendly".to_owned())
                );
            }
            if selector == "prompt-tool" {
                assert_eq!(
                    values.get("_skit_runner").map(FieldValue::as_text),
                    Some("codex".to_owned())
                );
            }
            let response = host.serve(submit).unwrap();
            assert!(!matches!(response, Action::SetStatus(_)));
            assert!(host.model.rerunnable.contains(selector));
        }
    }

    #[test]
    fn entry_mutations_refresh_runner_pins_without_rewriting_raw_identity() {
        let mut settings = FakeHost::new();
        let raw_identity = settings.model.runners[1].identity.clone();
        settings
            .serve(Effect::Submit {
                purpose: FormPurpose::Settings,
                selector: Some("prompt-tool".to_owned()),
                values: BTreeMap::from([(skit_ui::RUNNER_KEY.to_owned(), FieldValue::text(""))]),
            })
            .unwrap();
        assert_eq!(settings.model.runners[0].pinned_count, 0);
        assert_eq!(settings.model.runners[1].identity, raw_identity);

        let mut host = FakeHost::new();
        let mut create = create_entry_fixture("Second prompt", "prompt");
        create.settings.runner = "codex".to_owned();
        host.serve(Effect::Add(vec![AddEffect::Commit {
            request: request_id(74),
            entry: Box::new(create),
            source: None,
        }]))
        .unwrap();
        assert_eq!(host.model.runners[0].pinned_count, 2);
        host.serve(Effect::Remove {
            selector: "second-prompt".to_owned(),
        })
        .unwrap();
        assert_eq!(host.model.runners[0].pinned_count, 1);
        host.serve(Effect::Remove {
            selector: "prompt-tool".to_owned(),
        })
        .unwrap();
        assert_eq!(host.model.runners[0].pinned_count, 0);
    }

    #[test]
    fn add_prompt_candidate_projection_keeps_only_unmanaged_placeholders() {
        for (selected, interpolate, expected) in [
            (false, true, vec!["TOPIC"]),
            (true, true, Vec::new()),
            (false, false, vec!["TOPIC", "AUDIENCE"]),
        ] {
            let mut host = FakeHost::new();
            let mut state = host.initial_state();
            let open = state.update(Action::OpenAdd);
            assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
            assert_eq!(
                state.update(Action::Add(AddAction::SetSourcePath(
                    "/fixtures/prompt.md".to_owned(),
                ))),
                Effect::None
            );
            let inspect = state.update(Action::Add(AddAction::Continue));
            assert_eq!(round_trip(&mut host, &mut state, inspect), Effect::None);
            assert_eq!(
                state.update(Action::Add(AddAction::PickKind(Some(
                    KnownEntryKind::Prompt,
                )))),
                Effect::None
            );
            assert_eq!(
                state.update(Action::Add(AddAction::SetReviewName(
                    "Prompt candidate copy".to_owned(),
                ))),
                Effect::None
            );
            assert_eq!(
                state.update(Action::Add(AddAction::SetPromptCandidate {
                    name: "TOPIC".to_owned(),
                    selected,
                })),
                Effect::None
            );
            assert_eq!(
                state.update(Action::Add(AddAction::SetPromptRunner {
                    name: "codex".to_owned(),
                    picked: true,
                })),
                Effect::None
            );
            if !interpolate {
                assert_eq!(
                    state.update(Action::Add(AddAction::SetPromptInterpolation(false))),
                    Effect::None
                );
            }
            let mut effect = state.update(Action::Add(AddAction::Save));
            assert!(
                !matches!(effect, Effect::None),
                "save did not emit a host effect: {:?}",
                state.add_workflow()
            );
            while !matches!(effect, Effect::None) {
                effect = round_trip(&mut host, &mut state, effect);
            }
            let created = host
                .model
                .entries
                .values()
                .find(|entry| entry.summary.name == "Prompt candidate copy")
                .unwrap_or_else(|| panic!("created entries: {:?}", host.model.entries.keys()));
            assert_eq!(
                created
                    .settings
                    .candidates
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn add_python_candidate_projection_redetects_an_unmanaged_source_binding() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let open = state.update(Action::OpenAdd);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SetSourcePath(
                "/fixtures/python_tool.py".to_owned(),
            ))),
            Effect::None
        );
        let inspect = state.update(Action::Add(AddAction::Continue));
        assert_eq!(round_trip(&mut host, &mut state, inspect), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::PickKind(Some(
                KnownEntryKind::Python,
            )))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Add(AddAction::SetReviewName(
                "Python candidate copy".to_owned(),
            ))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Add(AddAction::SetReviewCandidate {
                name: "NAME".to_owned(),
                selected: false,
            })),
            Effect::None
        );
        let mut effect = state.update(Action::Add(AddAction::Save));
        assert!(
            !matches!(effect, Effect::None),
            "save did not emit a host effect: {:?}",
            state.add_workflow()
        );
        while !matches!(effect, Effect::None) {
            effect = round_trip(&mut host, &mut state, effect);
        }
        assert!(
            host.model
                .entries
                .values()
                .find(|entry| entry.summary.name == "Python candidate copy")
                .unwrap()
                .settings
                .candidates
                .contains(&"NAME".to_owned())
        );
    }

    #[test]
    fn prompt_interpolation_controls_only_effective_run_and_detail_declarations() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::INTERPOLATE_KEY.to_owned(),
                value: FieldValue::boolean(false),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        assert!(
            host.model.entries["prompt-tool"]
                .detail
                .parameters
                .is_empty()
        );
        assert_eq!(host.model.entries["prompt-tool"].declarations.len(), 1);

        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert!(
            state
                .run_form()
                .unwrap()
                .fields()
                .iter()
                .all(|field| field.key != "value:TOPIC")
        );
        assert_eq!(state.update(Action::OpenRunPresetSave), Effect::None);
        assert!(state.modal().is_none());
        assert_eq!(state.update(Action::Back), Effect::None);

        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::INTERPOLATE_KEY.to_owned(),
                value: FieldValue::boolean(true),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        assert_eq!(host.model.entries["prompt-tool"].detail.parameters.len(), 1);
        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert!(
            state
                .run_form()
                .unwrap()
                .fields()
                .iter()
                .any(|field| field.key == "value:TOPIC")
        );
    }

    #[test]
    fn prompt_without_a_runner_returns_the_owner_preserving_action() {
        let mut host = FakeHost::new();
        let row = host
            .runner_rows()
            .iter()
            .find(|row| row.name.as_deref() == Some("codex"))
            .unwrap()
            .clone();
        host.serve(Effect::RemoveRunner(RunnerRemoveRequest::Named {
            name: "codex".to_owned(),
            expected: row.key_identities,
            expected_pinned_count: row.pinned_count,
        }))
        .unwrap();
        let response = host
            .serve(Effect::Open {
                request: HostRequest::Run,
                selector: Some("prompt-tool".to_owned()),
            })
            .unwrap();
        assert!(matches!(response, Action::PromptRunnerRequired { .. }));
    }

    #[test]
    fn preset_schema_is_authoritative_and_empty_schema_refuses_without_writing() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        for (key, value) in [("value:NAME", "Ada"), ("value:TOKEN", "secret")] {
            let field = state
                .run_form()
                .unwrap()
                .fields()
                .iter()
                .position(|field| field.key == key)
                .unwrap();
            assert_eq!(
                state.update(Action::SetFieldValue {
                    field,
                    value: value.to_owned(),
                }),
                Effect::None
            );
        }
        assert_eq!(state.update(Action::OpenRunPresetSave), Effect::None);
        assert_eq!(
            state.update(Action::SetModalInput("schema-race".to_owned())),
            Effect::None
        );
        let save = state.update(Action::Submit);
        assert!(matches!(save, Effect::SaveRunPreset { .. }));
        let declarations = &mut host
            .model
            .entries
            .get_mut("python-tool")
            .unwrap()
            .declarations;
        declarations
            .iter_mut()
            .find(|declaration| declaration.name == "NAME")
            .unwrap()
            .secret = true;
        declarations
            .iter_mut()
            .find(|declaration| declaration.name == "TOKEN")
            .unwrap()
            .secret = false;
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        assert_eq!(
            host.preset("python-tool", "schema-race").unwrap(),
            &BTreeMap::from([("OLD".to_owned(), String::new())])
        );

        let mut host = FakeHost::new();
        host.model
            .entries
            .get_mut("command-tool")
            .unwrap()
            .declarations
            .clear();
        let before = host.snapshot();
        let response = host
            .serve(Effect::SaveRunPreset {
                selector: "command-tool".to_owned(),
                name: "empty".to_owned(),
                values: BTreeMap::new(),
                secret_names: BTreeSet::new(),
            })
            .expect("empty schema is a typed refusal");
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn create_projection_runner_identity_run_glob_and_preferences_are_model_backed() {
        let mut host = FakeHost::new();
        let mut create = create_entry_fixture("Projected entry", "prompt");
        create.workdir = "/fixtures/work".to_owned();
        create.settings.runner = "codex".to_owned();
        create.settings.template = "ignored".to_owned();
        create.settings.dependencies = vec!["dep".to_owned()];
        create.settings.requires_python = ">=3.12".to_owned();
        create.settings.needs = vec!["git".to_owned()];
        create.settings.parameters = vec![ParamDecl::new("TOPIC")];
        let response = host
            .serve(Effect::Add(vec![AddEffect::Commit {
                request: request_id(40),
                entry: Box::new(create),
                source: None,
            }]))
            .unwrap();
        assert!(matches!(
            response,
            Action::Add(AddAction::CommitFinished {
                request,
                result: Ok(ref slug),
            }) if request == request_id(40) && slug == "projected-entry"
        ));
        let projected = host.model.entries.get("projected-entry").unwrap();
        assert_eq!(projected.settings.workdir, "/fixtures/work");
        assert_eq!(projected.settings.runner, "codex");
        assert_eq!(projected.settings.needs, ["git"]);
        assert_eq!(projected.detail.parameters.len(), 1);
        assert_eq!(
            projected.detail.prompt_runner,
            Some(skit_ui::LibraryPromptRunner::Configured("codex".to_owned()))
        );

        let malformed_before = host.runner_rows()[1].identity.clone();
        let codex = host.runner_rows()[0].clone();
        host.serve(Effect::SaveRunner {
            request: RunnerSaveRequest {
                name: "codex".to_owned(),
                argv: vec!["codex".to_owned(), "{{prompt}}".to_owned()],
                target: RunnerSaveTarget::Named {
                    name: "codex".to_owned(),
                    expected: codex.key_identities,
                },
            },
            owner: RunnerSaveOwner::Manager,
        })
        .unwrap();
        assert_eq!(host.runner_rows()[1].identity, malformed_before);
        assert_eq!(host.runner_rows()[0].pinned_count, 2);
        host.model
            .entries
            .get_mut("python-tool")
            .unwrap()
            .settings
            .runner = "codex".to_owned();
        host.commit_runner_rows(host.model.runners.clone());
        assert_eq!(host.runner_rows()[0].pinned_count, 2);

        let before = host.snapshot();
        let invalid = host
            .serve(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([(
                        "shell.bash_path".to_owned(),
                        "/fixtures/missing-bash".to_owned(),
                    )]),
                },
            )))
            .unwrap();
        assert!(matches!(
            invalid,
            Action::Preferences(PreferencesAction::ValidationFailed(
                skit_application::preferences::PreferencesError::BashPathMissing { .. }
            ))
        ));
        assert_eq!(host.snapshot(), before);

        host.model.agent_targets.clear();
        let discovered = host
            .serve(Effect::Preferences(
                PreferencesEffect::DiscoverAgentSkillTargets,
            ))
            .unwrap();
        assert_eq!(
            discovered,
            Action::Preferences(PreferencesAction::PresentAgentSkillTargets(Vec::new()))
        );
    }

    #[test]
    fn run_state_after_run_and_protocol_only_effects_are_honest() {
        let mut host = FakeHost::new();
        host.model.preferences.after_run = AfterRunChoice::Exit;
        let values = BTreeMap::from([("NAME".to_owned(), FieldValue::text("Ada"))]);
        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("python-tool".to_owned()),
                values: values.clone(),
            })
            .unwrap();
        assert_eq!(response, Action::Quit);
        assert_eq!(host.model.last_runs.get("python-tool"), Some(&values));
        assert_eq!(
            host.serve(Effect::Rerun {
                selector: "python-tool".to_owned(),
            })
            .unwrap(),
            Action::Quit
        );

        let before = host.snapshot();
        assert!(host.serve(Effect::None).is_err());
        assert!(host.serve(Effect::Quit).is_err());
        assert!(
            host.serve(Effect::Preferences(PreferencesEffect::None))
                .is_err()
        );
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn standalone_runner_refusal_preserves_its_exact_modal_owner() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let prompt = state
            .visible_entries()
            .position(|entry| entry.slug.as_str() == "prompt-tool")
            .unwrap();
        assert_eq!(state.update(Action::SelectVisible(prompt)), Effect::None);
        let open = state.update(Action::OpenSettings);
        let response = host.serve(open).unwrap();
        assert_eq!(state.update(response), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::NewRunner)),
            Effect::None
        );
        assert_eq!(
            state.update(Action::RunnerEditor(RunnerEditorAction::SetName(
                "codex".to_owned(),
            ))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::RunnerEditor(RunnerEditorAction::SetCommand(
                "codex {{prompt}}".to_owned(),
            ))),
            Effect::None
        );
        let save = state.update(Action::RunnerEditor(RunnerEditorAction::Submit));
        let owner = RunnerEditorOwner::Settings {
            selector: "prompt-tool".to_owned(),
        };
        assert!(matches!(
            save,
            Effect::SaveRunner {
                owner: RunnerSaveOwner::Editor(ref actual),
                ..
            } if actual == &owner
        ));
        let before = host.snapshot();
        let response = host.serve(save).unwrap();
        assert!(matches!(
            response,
            Action::RunnerEditorSaveFailed {
                owner: ref actual,
                ..
            } if actual == &owner
        ));
        assert_eq!(state.update(response), Effect::None);
        let Some(ModalState::RunnerEditor {
            owner: actual,
            view,
            ..
        }) = state.modal()
        else {
            panic!("the standalone editor must remain open");
        };
        assert_eq!(actual, &owner);
        assert!(view.host_error().is_some());
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn raw_runner_identity_must_resolve_to_one_exact_row() {
        let mut host = FakeHost::new();
        let duplicate = host.model.runners[1].clone();
        let expected = duplicate.identity.clone();
        host.model.runners.push(duplicate);
        let before = host.snapshot();
        let response = host
            .serve(Effect::RemoveRunner(RunnerRemoveRequest::RawRow {
                expected,
            }))
            .unwrap();
        assert!(matches!(
            response,
            Action::Runners(RunnerManagerAction::MutationFailed(_))
        ));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn reducer_raw_runner_repair_refuses_a_valid_name_collision_without_mutation() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let open = state.update(Action::OpenRunners);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::Select(1))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::ActivateSelected)),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::EditSelected)),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::Editor(
                RunnerEditorAction::SetName("codex".to_owned()),
            ))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Runners(RunnerManagerAction::Editor(
                RunnerEditorAction::SetCommand("other {{prompt}}".to_owned()),
            ))),
            Effect::None
        );
        let save = state.update(Action::Runners(RunnerManagerAction::Editor(
            RunnerEditorAction::Submit,
        )));
        assert!(matches!(
            save,
            Effect::SaveRunner {
                owner: RunnerSaveOwner::Manager,
                request: RunnerSaveRequest {
                    target: RunnerSaveTarget::RawRow { .. },
                    ..
                },
            }
        ));
        let before = host.snapshot();
        let response = host.serve(save).unwrap();
        assert!(matches!(
            response,
            Action::Runners(RunnerManagerAction::MutationFailed(_))
        ));
        assert_eq!(state.update(response), Effect::None);
        assert!(matches!(
            state.screen(),
            Screen::Runners(view) if view.editor().is_some()
        ));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn reducer_raw_runner_repair_refuses_a_malformed_named_row_collision() {
        let mut host = FakeHost::new();
        let mut rows = host.model.runners.clone();
        rows.push(RunnerRow {
            identity: RunnerRowIdentity {
                index: None,
                snapshot_token: String::new(),
            },
            name: Some("collision".to_owned()),
            argv: None,
            reason: Some("command_missing".to_owned()),
            descriptor: "malformed named collision".to_owned(),
            key_identities: Vec::new(),
            pinned_count: 0,
        });
        host.commit_runner_rows(rows);
        assert!(!host.model.runners[2].is_valid());

        let mut state = host.initial_state();
        let open = state.update(Action::OpenRunners);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        for action in [
            RunnerManagerAction::Select(1),
            RunnerManagerAction::ActivateSelected,
            RunnerManagerAction::EditSelected,
            RunnerManagerAction::Editor(RunnerEditorAction::SetName("collision".to_owned())),
            RunnerManagerAction::Editor(RunnerEditorAction::SetCommand(
                "other {{prompt}}".to_owned(),
            )),
        ] {
            assert_eq!(state.update(Action::Runners(action)), Effect::None);
        }
        let save = state.update(Action::Runners(RunnerManagerAction::Editor(
            RunnerEditorAction::Submit,
        )));
        assert!(matches!(
            save,
            Effect::SaveRunner {
                owner: RunnerSaveOwner::Manager,
                request: RunnerSaveRequest {
                    target: RunnerSaveTarget::RawRow { .. },
                    ..
                },
            }
        ));
        let before = host.snapshot();
        let response = host.serve(save).unwrap();
        assert!(matches!(
            response,
            Action::Runners(RunnerManagerAction::MutationFailed(_))
        ));
        assert_eq!(state.update(response), Effect::None);
        assert!(matches!(
            state.screen(),
            Screen::Runners(view) if view.editor().is_some()
        ));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn forged_invalid_runner_commands_preserve_each_typed_owner() {
        for owner in [
            RunnerSaveOwner::Manager,
            RunnerSaveOwner::Editor(RunnerEditorOwner::Settings {
                selector: "prompt-tool".to_owned(),
            }),
        ] {
            let mut host = FakeHost::new();
            let before = host.snapshot();
            let response = host
                .serve(Effect::SaveRunner {
                    request: RunnerSaveRequest {
                        name: " invalid ".to_owned(),
                        argv: vec!["agent".to_owned(), String::new(), "{{prompt}}".to_owned()],
                        target: RunnerSaveTarget::New,
                    },
                    owner: owner.clone(),
                })
                .unwrap();
            assert!(match (&owner, response) {
                (
                    RunnerSaveOwner::Manager,
                    Action::Runners(RunnerManagerAction::MutationFailed(_)),
                ) => true,
                (
                    RunnerSaveOwner::Editor(expected),
                    Action::RunnerEditorSaveFailed { owner, .. },
                ) => &owner == expected,
                _ => false,
            });
            assert_eq!(host.snapshot(), before);
        }
    }

    #[test]
    fn prompt_run_saves_picked_runner_and_extra_arguments_atomically() {
        let mut host = FakeHost::new();
        host.serve(Effect::SaveRunner {
            request: RunnerSaveRequest {
                name: "other".to_owned(),
                argv: vec!["other".to_owned(), "{{prompt}}".to_owned()],
                target: RunnerSaveTarget::New,
            },
            owner: RunnerSaveOwner::Manager,
        })
        .unwrap();
        let mut state = host.initial_state();
        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let runner = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .position(|field| field.key == "_skit_runner")
            .unwrap();
        assert_eq!(
            state.update(Action::SelectFieldOption {
                field: runner,
                value: "other".to_owned(),
            }),
            Effect::None
        );
        let expected_args = vec!["--flag".to_owned(), "two words".to_owned()];
        let editable_args = join_editable_argv(&expected_args, EditableArgvDialect::host());
        let args = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .position(|field| field.key == "_skit_args")
            .unwrap();
        assert_eq!(
            state.update(Action::SetFieldValue {
                field: args,
                value: editable_args.clone(),
            }),
            Effect::None
        );
        let run = state.update(Action::Submit);
        assert!(matches!(
            &run,
            Effect::Submit { values, .. }
                if values.get("_skit_runner_picked").is_some_and(|value| value.as_text() == "true")
        ));
        assert_eq!(round_trip(&mut host, &mut state, run), Effect::None);
        assert_eq!(host.model.last_runner.as_deref(), Some("other"));
        assert_eq!(host.model.extra_args["prompt-tool"], expected_args);

        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let args = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .find(|field| field.key == "_skit_args")
            .unwrap();
        assert_eq!(args.control.value(), editable_args);

        assert_eq!(state.update(Action::Back), Effect::None);
        let open = state.update(Action::OpenAdd);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SetSourcePath(
                "/fixtures/prompt.md".to_owned(),
            ))),
            Effect::None
        );
        let inspect = state.update(Action::Add(AddAction::Continue));
        assert_eq!(round_trip(&mut host, &mut state, inspect), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::PickKind(Some(
                KnownEntryKind::Prompt,
            )))),
            Effect::None
        );
        assert_eq!(
            state.add_workflow().unwrap().review().unwrap().runner(),
            "other"
        );
        assert_eq!(state.update(Action::Back), Effect::None);

        let before = host.snapshot();
        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("prompt-tool".to_owned()),
                values: BTreeMap::from([
                    ("_skit_runner".to_owned(), FieldValue::text("missing")),
                    ("_skit_args".to_owned(), FieldValue::text("'unfinished")),
                ]),
            })
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);

        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("prompt-tool".to_owned()),
                values: BTreeMap::from([
                    ("_skit_runner".to_owned(), FieldValue::text("codex")),
                    ("_skit_args".to_owned(), FieldValue::text("'unfinished")),
                ]),
            })
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn rerun_refuses_a_missing_nonempty_prompt_pin_without_mutation() {
        let mut host = FakeHost::new();
        host.model.rerunnable.insert("prompt-tool".to_owned());
        host.model
            .entries
            .get_mut("prompt-tool")
            .unwrap()
            .settings
            .runner = "missing".to_owned();
        let before = host.snapshot();
        let response = host
            .serve(Effect::Rerun {
                selector: "prompt-tool".to_owned(),
            })
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn prompt_and_command_settings_update_their_own_surfaces_atomically() {
        let mut host = FakeHost::new();
        host.serve(Effect::Submit {
            purpose: FormPurpose::Settings,
            selector: Some("prompt-tool".to_owned()),
            values: BTreeMap::from([
                (skit_ui::RUNNER_KEY.to_owned(), FieldValue::text("")),
                (
                    skit_ui::INTERPOLATE_KEY.to_owned(),
                    FieldValue::boolean(false),
                ),
            ]),
        })
        .unwrap();
        let prompt = host.model.entries.get("prompt-tool").unwrap();
        assert!(!prompt.settings.interpolate);
        assert_eq!(prompt.settings.runner, "");
        assert_eq!(
            prompt.detail.prompt_runner,
            Some(skit_ui::LibraryPromptRunner::PickOnRunForm)
        );

        host.serve(Effect::Submit {
            purpose: FormPurpose::Settings,
            selector: Some("command-tool".to_owned()),
            values: BTreeMap::from([(
                skit_ui::TEMPLATE_KEY.to_owned(),
                FieldValue::text("echo {{TARGET}}"),
            )]),
        })
        .unwrap();
        let command = host.model.entries.get("command-tool").unwrap();
        assert_eq!(command.settings.template, "echo {{TARGET}}");
        assert_eq!(command.detail.template.as_deref(), Some("echo {{TARGET}}"));

        let before = host.snapshot();
        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Settings,
                selector: Some("python-tool".to_owned()),
                values: BTreeMap::from([
                    (skit_ui::NAME_KEY.to_owned(), FieldValue::text("No commit")),
                    (
                        skit_ui::RESYNC_KEY.to_owned(),
                        FieldValue::text("sometimes"),
                    ),
                ]),
            })
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn forged_preferences_globs_and_agent_targets_refuse_without_mutation() {
        let mut host = FakeHost::new();
        let before = host.snapshot();
        let response = host
            .serve(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([("lang".to_owned(), "not-a-locale".to_owned())]),
                },
            )))
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);

        let response = host
            .serve(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: PathBuf::from("/fixtures/unknown/skills"),
            }))
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    fn select_entry(state: &mut LibraryState, selector: &str) {
        let index = state
            .visible_entries()
            .position(|entry| entry.slug.as_str() == selector)
            .unwrap();
        assert_eq!(state.update(Action::SelectVisible(index)), Effect::None);
    }

    fn round_trip(host: &mut FakeHost, state: &mut LibraryState, effect: Effect) -> Effect {
        let response = host.serve(effect).expect("the fake host serves the effect");
        state.update(response)
    }

    #[test]
    fn reducer_glob_requests_use_the_same_virtual_root_and_production_counts() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let field = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .position(|field| field.key == "value:NAME")
            .unwrap();

        assert_eq!(
            state.update(Action::SetFieldValue {
                field,
                value: "literal.py".to_owned(),
            }),
            Effect::None,
            "a literal does not request glob feedback"
        );
        assert_eq!(
            state.update(Action::SetFieldValue {
                field,
                value: "'unfinished*".to_owned(),
            }),
            Effect::None,
            "invalid shell splitting does not request glob feedback"
        );

        let cases = [
            ("*", 6),
            ("*.py", 2),
            ("***.py", 1),
            ("none-*.zzz", 1),
            ("[ab]lpha.py", 1),
            ("[broken", 1),
            ("**/*.py", 3),
            (".hidden*.py", 1),
            ("nested/*.py", 1),
            ("nested/.*.py", 1),
            ("unicodé-?.rs", 1),
        ];
        for (value, expected) in cases {
            let effect = state.update(Action::SetFieldValue {
                field,
                value: value.to_owned(),
            });
            let request_cwd = match &effect {
                Effect::CountRunGlob { request, .. } => request.cwd.clone(),
                _ => panic!("{value} must make a glob request: {effect:?}"),
            };
            assert_eq!(
                Path::new(&request_cwd),
                Path::new(&fixtures::run_context("python").path.unwrap().invoke_cwd)
            );
            assert_eq!(round_trip(&mut host, &mut state, effect), Effect::None);
            assert_eq!(
                state.run_form().unwrap().fields()[field]
                    .feedback
                    .glob_count,
                Some(expected),
                "wrong production-style count for {value}"
            );
        }
    }

    #[test]
    fn preferences_reducer_tokens_resolve_presets_custom_urls_and_paused_state() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        let open = state.update(Action::OpenPreferences);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        for action in [
            PreferencesAction::SetMirrorMaster(true),
            PreferencesAction::ChooseMirror {
                field: PreferencesField::PypiMirror,
                choice: MirrorChoice::Preset("tsinghua".to_owned()),
            },
            PreferencesAction::ChooseMirror {
                field: PreferencesField::GithubMirror,
                choice: MirrorChoice::Preset("nju".to_owned()),
            },
            PreferencesAction::ChooseMirror {
                field: PreferencesField::NpmMirror,
                choice: MirrorChoice::Preset("npmmirror".to_owned()),
            },
        ] {
            assert_eq!(state.update(Action::Preferences(action)), Effect::None);
        }
        let save = state.update(Action::Preferences(PreferencesAction::Save));
        assert!(matches!(
            save,
            Effect::Preferences(PreferencesEffect::Save(_))
        ));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        let mirror = &host.preferences_snapshot().mirror;
        assert!(mirror.enabled);
        assert_eq!(mirror.pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
        assert_eq!(
            mirror.python_install,
            "https://mirror.nju.edu.cn/github-release/astral-sh/python-build-standalone/"
        );
        assert_eq!(
            mirror.uv_binary,
            "https://mirror.nju.edu.cn/github-release/astral-sh/uv"
        );
        assert_eq!(mirror.npm, "https://registry.npmmirror.com");

        let open = state.update(Action::OpenPreferences);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Preferences(PreferencesAction::SetMirrorMaster(
                false
            ))),
            Effect::None
        );
        let save = state.update(Action::Preferences(PreferencesAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        let mirror = &host.preferences_snapshot().mirror;
        assert!(!mirror.enabled);
        assert_eq!(mirror.pypi, "https://pypi.tuna.tsinghua.edu.cn/simple");
        assert_eq!(mirror.npm, "https://registry.npmmirror.com");

        let open = state.update(Action::OpenPreferences);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        for action in [
            PreferencesAction::SetMirrorMaster(true),
            PreferencesAction::ChooseMirror {
                field: PreferencesField::PypiMirror,
                choice: MirrorChoice::Custom,
            },
            PreferencesAction::SetMirrorUrl {
                field: PreferencesField::PypiMirror,
                value: "https://custom.invalid/pypi/".to_owned(),
            },
            PreferencesAction::ChooseMirror {
                field: PreferencesField::GithubMirror,
                choice: MirrorChoice::Custom,
            },
            PreferencesAction::SetMirrorUrl {
                field: PreferencesField::GithubMirror,
                value: "https://custom.invalid/releases/".to_owned(),
            },
            PreferencesAction::ChooseMirror {
                field: PreferencesField::NpmMirror,
                choice: MirrorChoice::Custom,
            },
            PreferencesAction::SetMirrorUrl {
                field: PreferencesField::NpmMirror,
                value: "https://custom.invalid/npm/".to_owned(),
            },
        ] {
            assert_eq!(state.update(Action::Preferences(action)), Effect::None);
        }
        let save = state.update(Action::Preferences(PreferencesAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        let mirror = &host.preferences_snapshot().mirror;
        assert!(mirror.enabled);
        assert_eq!(mirror.pypi, "https://custom.invalid/pypi");
        assert_eq!(
            mirror.python_install,
            "https://custom.invalid/releases/astral-sh/python-build-standalone/"
        );
        assert_eq!(
            mirror.uv_binary,
            "https://custom.invalid/releases/astral-sh/uv"
        );
        assert_eq!(mirror.npm, "https://custom.invalid/npm");

        let open = state.update(Action::OpenPreferences);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        for field in [
            PreferencesField::PypiMirror,
            PreferencesField::GithubMirror,
            PreferencesField::NpmMirror,
        ] {
            assert_eq!(
                state.update(Action::Preferences(PreferencesAction::ChooseMirror {
                    field,
                    choice: MirrorChoice::Off,
                })),
                Effect::None
            );
        }
        let save = state.update(Action::Preferences(PreferencesAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        let mirror = &host.preferences_snapshot().mirror;
        assert!(!mirror.enabled);
        assert_eq!(mirror.pypi, "");
        assert_eq!(mirror.python_install, "");
        assert_eq!(mirror.uv_binary, "");
        assert_eq!(mirror.npm, "");
    }

    #[test]
    fn every_pypi_preset_token_from_the_reducer_resolves_to_its_store_value() {
        for (token, expected) in [
            ("tsinghua", "https://pypi.tuna.tsinghua.edu.cn/simple"),
            ("aliyun", "https://mirrors.aliyun.com/pypi/simple"),
            ("ustc", "https://pypi.mirrors.ustc.edu.cn/simple"),
        ] {
            let mut host = FakeHost::new();
            let mut state = host.initial_state();
            let open = state.update(Action::OpenPreferences);
            assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
            assert_eq!(
                state.update(Action::Preferences(PreferencesAction::ChooseMirror {
                    field: PreferencesField::PypiMirror,
                    choice: MirrorChoice::Preset(token.to_owned()),
                })),
                Effect::None
            );
            let save = state.update(Action::Preferences(PreferencesAction::Save));
            assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
            assert_eq!(host.preferences_snapshot().mirror.pypi, expected);
        }
    }

    #[test]
    fn reducer_preset_and_secret_transition_scrub_every_persistent_value_surface() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let fields = state.run_form().unwrap().fields();
        let name = fields
            .iter()
            .position(|field| field.key == "value:NAME")
            .unwrap();
        let token = fields
            .iter()
            .position(|field| field.key == "value:TOKEN")
            .unwrap();
        assert_eq!(
            state.update(Action::SetFieldValue {
                field: name,
                value: "Visible".to_owned(),
            }),
            Effect::None
        );
        assert_eq!(
            state.update(Action::SetFieldValue {
                field: token,
                value: "never-store".to_owned(),
            }),
            Effect::None
        );
        assert_eq!(state.update(Action::OpenRunPresetSave), Effect::None);
        assert_eq!(
            state.update(Action::SetModalInput("before-secret".to_owned())),
            Effect::None
        );
        let save_preset = state.update(Action::Submit);
        assert!(matches!(save_preset, Effect::SaveRunPreset { .. }));
        assert_eq!(round_trip(&mut host, &mut state, save_preset), Effect::None);
        assert_eq!(
            host.preset("python-tool", "before-secret").unwrap(),
            &BTreeMap::from([
                ("NAME".to_owned(), "Visible".to_owned()),
                ("OLD".to_owned(), String::new()),
            ])
        );
        let run = state.update(Action::Submit);
        assert!(matches!(
            run,
            Effect::Submit {
                purpose: FormPurpose::Run,
                ..
            }
        ));
        assert_eq!(round_trip(&mut host, &mut state, run), Effect::None);
        assert!(host.model.last_runs["python-tool"].contains_key("NAME"));
        assert!(host.model.remembered_values["python-tool"].contains_key("NAME"));
        assert!(host.model.last_runs["python-tool"].contains_key("OLD"));
        assert!(!host.model.last_runs["python-tool"].contains_key("TOKEN"));

        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: "parameter:NAME:secret".to_owned(),
                value: FieldValue::boolean(true),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert!(matches!(
            save,
            Effect::Submit {
                purpose: FormPurpose::Settings,
                ..
            }
        ));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        assert!(host.preset("python-tool", "friendly").is_none());
        assert!(
            !host
                .preset("python-tool", "before-secret")
                .unwrap()
                .contains_key("NAME")
        );
        assert!(!host.model.last_runs["python-tool"].contains_key("NAME"));
        assert!(!host.model.remembered_values["python-tool"].contains_key("NAME"));
        let detail = &host.model.entries["python-tool"].detail;
        let name = detail
            .parameters
            .iter()
            .find(|parameter| parameter.key == "NAME")
            .unwrap();
        assert!(name.secret);
        assert_eq!(name.value, "");
    }

    #[test]
    fn settings_reducer_uses_production_python_validation_and_rolls_back_secret_scrub() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert!(
            state
                .settings_view()
                .unwrap()
                .field(skit_ui::INTERPRETER_KEY)
                .is_none()
        );
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: "parameter:NAME:secret".to_owned(),
                value: FieldValue::boolean(true),
            })),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::DEPENDENCIES_KEY.to_owned(),
                value: FieldValue::text("@@@"),
            })),
            Effect::None
        );
        let before = host.snapshot();
        let save = state.update(Action::Settings(SettingsAction::Save));
        let response = host.serve(save).unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(state.update(response), Effect::None);
        assert_eq!(host.snapshot(), before);

        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::DEPENDENCIES_KEY.to_owned(),
                value: FieldValue::text("requests>=2"),
            })),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::PYTHON_KEY.to_owned(),
                value: FieldValue::text("not-a-version"),
            })),
            Effect::None
        );
        let before = host.snapshot();
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert!(matches!(host.serve(save).unwrap(), Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }

    #[test]
    fn command_settings_reconcile_body_placeholders_and_environment_riders() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "command-tool");
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::TEMPLATE_KEY.to_owned(),
                value: FieldValue::text("echo {SECOND} {TARGET}"),
            })),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::ADD_PARAMETER_KEY.to_owned(),
                value: FieldValue::text("EXTRA"),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        let command = &host.model.entries["command-tool"];
        assert_eq!(
            command
                .declarations
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter.delivery,))
                .collect::<Vec<_>>(),
            [
                ("SECOND", ParameterDelivery::Placeholder),
                ("TARGET", ParameterDelivery::Placeholder),
                ("EXTRA", ParameterDelivery::Env),
            ]
        );
        assert_eq!(
            command
                .detail
                .parameters
                .iter()
                .map(|parameter| parameter.key.as_str())
                .collect::<Vec<_>>(),
            ["SECOND", "TARGET", "EXTRA"]
        );
    }

    #[test]
    fn run_and_rerun_refresh_public_detail_and_reopen_an_unpinned_prompt() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let name = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .position(|field| field.key == "value:NAME")
            .unwrap();
        assert_eq!(
            state.update(Action::SetFieldValue {
                field: name,
                value: "Latest".to_owned(),
            }),
            Effect::None
        );
        let run = state.update(Action::Submit);
        assert_eq!(round_trip(&mut host, &mut state, run), Effect::None);
        let slug = Slug::parse("python-tool").unwrap();
        let detail = state.entry_detail(&slug).unwrap();
        assert_eq!(
            detail
                .parameters
                .iter()
                .find(|parameter| parameter.key == "NAME")
                .unwrap()
                .value,
            "Latest"
        );
        assert!(detail.last_run.is_some());

        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let reopened = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .find(|field| field.key == "value:NAME")
            .unwrap();
        assert_eq!(reopened.control.value(), "Latest");

        assert_eq!(state.update(Action::Back), Effect::None);

        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let run = state.update(Action::Submit);
        assert_eq!(round_trip(&mut host, &mut state, run), Effect::None);
        select_entry(&mut state, "prompt-tool");
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::RUNNER_KEY.to_owned(),
                value: FieldValue::text(""),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        select_entry(&mut state, "prompt-tool");
        let rerun = state.update(Action::Rerun);
        let response = host.serve(rerun).unwrap();
        assert!(matches!(response, Action::Present(Screen::Run(_))));
        assert_eq!(state.update(response), Effect::None);
        assert!(matches!(state.screen(), Screen::Run(_)));
    }

    #[test]
    fn changed_defaults_do_not_reuse_an_exact_last_run_as_remembered_prefill() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "command-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        let name = state
            .run_form()
            .unwrap()
            .fields()
            .iter()
            .position(|field| field.key == "value:TARGET")
            .unwrap();
        assert_eq!(
            state.run_form().unwrap().fields()[name].control.value(),
            "A"
        );
        let run = state.update(Action::Submit);
        assert_eq!(round_trip(&mut host, &mut state, run), Effect::None);
        assert_eq!(
            host.model.last_runs["command-tool"]["TARGET"].as_text(),
            "A"
        );
        assert!(host.model.remembered_values["command-tool"].is_empty());

        select_entry(&mut state, "command-tool");
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: "parameter:TARGET:default".to_owned(),
                value: FieldValue::text("B"),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        let detail = &host.model.entries["command-tool"].detail;
        assert_eq!(
            detail
                .parameters
                .iter()
                .find(|parameter| parameter.key == "TARGET")
                .unwrap()
                .value,
            "B",
            "the detail uses the current prefill"
        );

        select_entry(&mut state, "command-tool");
        let open = state.update(Action::OpenRun);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state
                .run_form()
                .unwrap()
                .fields()
                .iter()
                .find(|field| field.key == "value:TARGET")
                .unwrap()
                .control
                .value(),
            "B",
            "the prefill uses the new default, not the exact last run"
        );
    }

    #[test]
    fn settings_workdir_projects_into_run_context_and_glob_requests() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        for (saved, expected) in [
            ("invoke", "/fixtures/invoke"),
            ("/fixtures/invoke/nested", "/fixtures/invoke/nested"),
        ] {
            select_entry(&mut state, "python-tool");
            let open = state.update(Action::OpenSettings);
            assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
            assert_eq!(
                state.update(Action::Settings(SettingsAction::SetField {
                    key: skit_ui::WORKDIR_KEY.to_owned(),
                    value: FieldValue::text(saved),
                })),
                Effect::None
            );
            let save = state.update(Action::Settings(SettingsAction::Save));
            assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
            select_entry(&mut state, "python-tool");
            let open = state.update(Action::OpenRun);
            assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
            let form = state.run_form().unwrap();
            assert_eq!(
                form.context().unwrap().path.as_ref().unwrap().workdir,
                expected
            );
            let field = form
                .fields()
                .iter()
                .position(|field| field.key == "value:NAME")
                .unwrap();
            let effect = state.update(Action::SetFieldValue {
                field,
                value: "*.py".to_owned(),
            });
            assert!(matches!(
                effect,
                Effect::CountRunGlob { ref request, .. } if request.cwd == "/fixtures/invoke"
            ));
            assert_eq!(round_trip(&mut host, &mut state, effect), Effect::None);
            assert_eq!(state.update(Action::Back), Effect::None);
        }

        for invalid in ["", "relative/path"] {
            select_entry(&mut state, "python-tool");
            let open = state.update(Action::OpenSettings);
            assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
            assert_eq!(
                state.update(Action::Settings(SettingsAction::SetField {
                    key: skit_ui::WORKDIR_KEY.to_owned(),
                    value: FieldValue::text(invalid),
                })),
                Effect::None
            );
            let before = host.snapshot();
            let save = state.update(Action::Settings(SettingsAction::Save));
            let response = host.serve(save).unwrap();
            assert!(matches!(response, Action::SetStatus(_)));
            assert_eq!(state.update(response), Effect::None);
            assert_eq!(host.snapshot(), before);
            assert_eq!(state.update(Action::Back), Effect::None);
        }
    }

    #[test]
    fn rename_refusal_keeps_the_real_form_owner_and_lists_accept_whitespace() {
        let mut host = FakeHost::new();
        let mut state = host.initial_state();
        select_entry(&mut state, "python-tool");
        let open = state.update(Action::OpenRename);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::SetFieldValue {
                field: 0,
                value: "   ".to_owned(),
            }),
            Effect::None
        );
        let before = host.snapshot();
        let submit = state.update(Action::Submit);
        let response = host
            .serve(submit)
            .expect("rename refusal is an owner-preserving action");
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(state.update(response), Effect::None);
        assert!(matches!(state.screen(), Screen::Form(_)));
        assert_eq!(host.snapshot(), before);

        assert_eq!(
            state.update(Action::SetFieldValue {
                field: 0,
                value: "Prompt tool".to_owned(),
            }),
            Effect::None
        );
        let before = host.snapshot();
        let submit = state.update(Action::Submit);
        let response = host.serve(submit).unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(state.update(response), Effect::None);
        assert!(matches!(state.screen(), Screen::Form(_)));
        assert_eq!(host.snapshot(), before);

        assert_eq!(state.update(Action::Back), Effect::None);
        let open = state.update(Action::OpenSettings);
        assert_eq!(round_trip(&mut host, &mut state, open), Effect::None);
        assert_eq!(
            state.update(Action::Settings(SettingsAction::SetField {
                key: skit_ui::NEEDS_KEY.to_owned(),
                value: FieldValue::text("git\tcurl  make,\nrg"),
            })),
            Effect::None
        );
        let save = state.update(Action::Settings(SettingsAction::Save));
        assert_eq!(round_trip(&mut host, &mut state, save), Effect::None);
        assert_eq!(
            host.model.entries["python-tool"].settings.needs,
            ["git", "curl", "make", "rg"]
        );
    }

    #[test]
    fn settings_name_collision_rolls_back_the_complete_transaction() {
        let mut host = FakeHost::new();
        let before = host.snapshot();
        let response = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Settings,
                selector: Some("python-tool".to_owned()),
                values: BTreeMap::from([
                    (
                        skit_ui::NAME_KEY.to_owned(),
                        FieldValue::text("Prompt tool"),
                    ),
                    (
                        "parameter:NAME:secret".to_owned(),
                        FieldValue::boolean(true),
                    ),
                ]),
            })
            .unwrap();
        assert!(matches!(response, Action::SetStatus(_)));
        assert_eq!(host.snapshot(), before);
    }
}
