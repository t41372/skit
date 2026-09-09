use std::{
    cell::Cell,
    collections::BTreeMap,
    env,
    ffi::OsString,
    io::{self, IsTerminal as _},
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use skit_application::{
    AgentRoots, LibraryService, form_feedback::GlobCountPort as _, form_state::FormStateService,
    health::HealthService, tokens::TokenContext,
};
use skit_i18n::{Locale, format_text, requested_locale, system_locale, text};
use skit_runtime::{InterpreterPlatform, InterpreterPolicy};
use skit_store::{
    AgentSkillInstallPoint, FileFormStateStore, FileGlobExpander, FileStore,
    LaunchSnapshotAllocator, LaunchSnapshotAttempt, LaunchSnapshotRequest, LaunchSnapshotStem,
    SystemLaunchSnapshotAllocator,
};
use skit_ui::{Action as UiAction, Effect as UiEffect, HealthAction, LibraryState, Screen};
use time::{OffsetDateTime, UtcOffset};

use crate::run::{RunClock, RunInvocation, RunPorts, RunServices};

use super::{
    AddHostContext, CliError, CliHealthInspector, EditRuntime, HumanStyle, PreferenceHost,
    active_locale, edit_with_runtime, entry_parameters, human_output_width, paint_for_output,
    refuse_empty_preset_schema, remove_with_state_dir, resolve_editor_argv_with, tui_add_effect_at,
    tui_complete_at, tui_open_with_context, tui_preferences_effect_at,
    tui_preferences_view_with_context, tui_preflight_effect_with_probe, tui_remove_runner_at,
    tui_rerun_with_services, tui_rerunnable, tui_save_runner_at, tui_submit_at, user_home,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ProductRoots {
    pub(super) data: PathBuf,
    pub(super) state: PathBuf,
    pub(super) config: PathBuf,
    pub(super) home: Option<PathBuf>,
    pub(super) cwd: PathBuf,
}

impl ProductRoots {
    pub(super) fn new(
        data: impl Into<PathBuf>,
        state: impl Into<PathBuf>,
        config: impl Into<PathBuf>,
        home: Option<PathBuf>,
        cwd: impl Into<PathBuf>,
    ) -> Self {
        Self {
            data: data.into(),
            state: state.into(),
            config: config.into(),
            home,
            cwd: cwd.into(),
        }
    }
}

pub(super) trait HostEnvironment: std::fmt::Debug {
    fn platform(&self) -> InterpreterPlatform;
    fn variable(&self, name: &str) -> Option<OsString>;
    fn variables(&self) -> BTreeMap<String, String>;
    fn local_offset(&self) -> UtcOffset;
    fn system_locale(&self) -> Locale;
}

/// Start the editor command that production planning selected.
pub(super) trait EditorLauncher: std::fmt::Debug {
    fn launch(&self, argv: &[String], path: &Path) -> io::Result<()>;
}

/// Publish host-side human output without coupling a walker to process streams.
pub(crate) trait HostOutput: std::fmt::Debug {
    fn success(&self, message: &str);
    fn detail(&self, message: &str);
    fn plain(&self, message: &str);
    fn stdout(&self, message: &str);
    fn diagnostic(&self, message: &str);
}

/// Report whether the process streams support an interactive exchange.
pub(super) trait TerminalCapability: std::fmt::Debug {
    fn stdin_is_terminal(&self) -> bool;
    fn stdout_is_terminal(&self) -> bool;
}

/// Inspect preference paths and install the embedded Agent Skill.
pub(super) trait PreferenceFiles: std::fmt::Debug {
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    fn agent_skill_checkpoint(&self, point: AgentSkillInstallPoint, path: &Path) -> io::Result<()>;
}

/// Select one fixed temporary-file name pattern without exposing its prefix to a caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TemporaryFilePurpose {
    AuthoredDraft,
    InjectedSource,
}

/// Select the operating-system temp location or one explicit directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TempLocation<'a> {
    System,
    Directory(&'a Path),
}

impl TemporaryFilePurpose {
    pub(crate) const fn prefix(self) -> &'static str {
        match self {
            Self::AuthoredDraft => "skit-new-",
            Self::InjectedSource => ".injected-",
        }
    }

    #[cfg(test)]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::AuthoredDraft => "authored_draft",
            Self::InjectedSource => "injected_source",
        }
    }

    #[cfg(test)]
    pub(crate) const fn width(self) -> usize {
        6
    }
}

/// Select one fixed private-directory name pattern without exposing its prefix to a caller.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrivateDirectoryPurpose {
    DraftQuarantine,
}

impl PrivateDirectoryPurpose {
    pub(crate) const fn prefix(self) -> &'static str {
        match self {
            Self::DraftQuarantine => ".skit-quarantine-",
        }
    }

    #[cfg(test)]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::DraftQuarantine => "draft_quarantine",
        }
    }

    #[cfg(test)]
    pub(crate) const fn width(self) -> usize {
        6
    }
}

/// Allocate raw files and directories for fixed product-owned name patterns.
pub(crate) trait FileAllocator: std::fmt::Debug + LaunchSnapshotAllocator {
    fn temporary_file(
        &self,
        purpose: TemporaryFilePurpose,
        location: TempLocation<'_>,
        suffix: &str,
    ) -> io::Result<tempfile::NamedTempFile>;

    fn private_directory(
        &self,
        purpose: PrivateDirectoryPurpose,
        location: &Path,
    ) -> io::Result<PathBuf>;
}

#[derive(Debug)]
pub(crate) struct SystemFileAllocator;

pub(super) fn set_private_file_mode(file: &std::fs::File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(())
    }
}

pub(super) fn set_private_directory_mode(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

impl FileAllocator for SystemFileAllocator {
    fn temporary_file(
        &self,
        purpose: TemporaryFilePurpose,
        location: TempLocation<'_>,
        suffix: &str,
    ) -> io::Result<tempfile::NamedTempFile> {
        let mut builder = tempfile::Builder::new();
        builder.prefix(purpose.prefix()).suffix(suffix);
        let temporary = match location {
            TempLocation::System => builder.tempfile(),
            TempLocation::Directory(path) => builder.tempfile_in(path),
        }?;
        set_private_file_mode(temporary.as_file())?;
        Ok(temporary)
    }

    fn private_directory(
        &self,
        purpose: PrivateDirectoryPurpose,
        location: &Path,
    ) -> io::Result<PathBuf> {
        let temporary = tempfile::Builder::new()
            .prefix(purpose.prefix())
            .tempdir_in(location)?;
        set_private_directory_mode(temporary.path())?;
        Ok(temporary.keep())
    }
}

impl LaunchSnapshotAllocator for SystemFileAllocator {
    fn next_stem(&self, request: LaunchSnapshotRequest<'_>) -> io::Result<LaunchSnapshotStem> {
        SystemLaunchSnapshotAllocator.next_stem(request)
    }

    fn record_attempt(&self, evidence: LaunchSnapshotAttempt<'_>) {
        SystemLaunchSnapshotAllocator.record_attempt(evidence);
    }

    fn retry_collisions(&self) -> bool {
        SystemLaunchSnapshotAllocator.retry_collisions()
    }
}

#[derive(Debug)]
pub(super) struct SystemPreferenceFiles;

impl PreferenceFiles for SystemPreferenceFiles {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn agent_skill_checkpoint(
        &self,
        _point: AgentSkillInstallPoint,
        _path: &Path,
    ) -> io::Result<()> {
        Ok(())
    }
}

/// Low-level platform adapters used by the production TUI host.
#[derive(Clone, Copy, Debug)]
pub(super) struct TuiPlatform<'a> {
    environment: &'a dyn HostEnvironment,
    terminal: &'a dyn TerminalCapability,
    editor: &'a dyn EditorLauncher,
    run: RunPorts<'a>,
    allocator: &'a dyn FileAllocator,
    output: &'a dyn HostOutput,
    preference_files: &'a dyn PreferenceFiles,
}

impl<'a> TuiPlatform<'a> {
    pub(super) const fn new(
        environment: &'a dyn HostEnvironment,
        terminal: &'a dyn TerminalCapability,
        editor: &'a dyn EditorLauncher,
        run: RunPorts<'a>,
        allocator: &'a dyn FileAllocator,
        output: &'a dyn HostOutput,
        preference_files: &'a dyn PreferenceFiles,
    ) -> Self {
        Self {
            environment,
            terminal,
            editor,
            run,
            allocator,
            output,
            preference_files,
        }
    }

    #[cfg(test)]
    const fn test(
        environment: &'a dyn HostEnvironment,
        terminal: &'a dyn TerminalCapability,
        editor: &'a dyn EditorLauncher,
        run: RunPorts<'a>,
        output: &'a dyn HostOutput,
    ) -> Self {
        Self::new(
            environment,
            terminal,
            editor,
            run,
            &SYSTEM_FILE_ALLOCATOR,
            output,
            &SYSTEM_PREFERENCE_FILES,
        )
    }
}

#[derive(Debug)]
pub(super) struct SystemClock;

impl RunClock for SystemClock {
    fn now_utc(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[derive(Debug)]
pub(super) struct SystemEnvironment;

impl HostEnvironment for SystemEnvironment {
    fn platform(&self) -> InterpreterPlatform {
        InterpreterPlatform::current()
    }

    fn variable(&self, name: &str) -> Option<OsString> {
        env::var_os(name)
    }

    fn variables(&self) -> BTreeMap<String, String> {
        env::vars().collect()
    }

    fn local_offset(&self) -> UtcOffset {
        UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
    }

    fn system_locale(&self) -> Locale {
        system_locale()
    }
}

#[derive(Debug)]
pub(super) struct SystemEditorLauncher;

impl EditorLauncher for SystemEditorLauncher {
    fn launch(&self, argv: &[String], path: &Path) -> io::Result<()> {
        ProcessCommand::new(&argv[0])
            .args(&argv[1..])
            .arg(path)
            .status()
            .map(|_| ())
    }
}

#[derive(Debug)]
pub(super) struct SystemTerminalCapability;

impl TerminalCapability for SystemTerminalCapability {
    fn stdin_is_terminal(&self) -> bool {
        io::stdin().is_terminal()
    }

    fn stdout_is_terminal(&self) -> bool {
        io::stdout().is_terminal()
    }
}

#[derive(Debug)]
pub(crate) struct SystemHostOutput;

impl HostOutput for SystemHostOutput {
    fn success(&self, message: &str) {
        println!(
            "{}",
            paint_for_output(message, HumanStyle::Green, human_output_width())
        );
    }

    fn detail(&self, message: &str) {
        println!(
            "{}",
            paint_for_output(message, HumanStyle::Dim, human_output_width())
        );
    }

    fn plain(&self, message: &str) {
        println!(
            "{}",
            paint_for_output(message, HumanStyle::Plain, human_output_width())
        );
    }

    fn stdout(&self, message: &str) {
        println!("{message}");
    }

    fn diagnostic(&self, message: &str) {
        eprintln!("{message}");
    }
}

static SYSTEM_CLOCK: SystemClock = SystemClock;
static SYSTEM_ENVIRONMENT: SystemEnvironment = SystemEnvironment;
pub(super) static SYSTEM_EDITOR: SystemEditorLauncher = SystemEditorLauncher;
pub(super) static SYSTEM_TERMINAL: SystemTerminalCapability = SystemTerminalCapability;
pub(crate) static SYSTEM_OUTPUT: SystemHostOutput = SystemHostOutput;
pub(crate) static SYSTEM_FILE_ALLOCATOR: SystemFileAllocator = SystemFileAllocator;
pub(super) static SYSTEM_PREFERENCE_FILES: SystemPreferenceFiles = SystemPreferenceFiles;

#[derive(Debug, Eq, PartialEq)]
pub(super) struct TuiHostDataRootMismatch {
    pub(super) expected: PathBuf,
    pub(super) actual: PathBuf,
}

#[derive(Debug)]
pub(super) struct TuiHost<'a, C: RunClock> {
    service: &'a LibraryService<FileStore>,
    roots: ProductRoots,
    locale: Cell<Locale>,
    clock: &'a C,
    platform: TuiPlatform<'a>,
}

impl<'a> TuiHost<'a, SystemClock> {
    pub(super) fn system(
        service: &'a LibraryService<FileStore>,
        state_dir: &Path,
        config_dir: &Path,
    ) -> Self {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let home = user_home();
        Self::new(
            service,
            ProductRoots::new(
                service.repository().data_dir(),
                state_dir,
                config_dir,
                home,
                cwd,
            ),
            active_locale(),
            &SYSTEM_CLOCK,
            TuiPlatform::new(
                &SYSTEM_ENVIRONMENT,
                &SYSTEM_TERMINAL,
                &SYSTEM_EDITOR,
                RunPorts::system(),
                &SYSTEM_FILE_ALLOCATOR,
                &SYSTEM_OUTPUT,
                &SYSTEM_PREFERENCE_FILES,
            ),
        )
        .expect("system product roots use the service repository data root")
    }
}

impl<'a, C: RunClock> TuiHost<'a, C> {
    pub(super) fn new(
        service: &'a LibraryService<FileStore>,
        roots: ProductRoots,
        locale: Locale,
        clock: &'a C,
        platform: TuiPlatform<'a>,
    ) -> Result<Self, TuiHostDataRootMismatch> {
        if service.repository().data_dir() != roots.data {
            return Err(TuiHostDataRootMismatch {
                expected: roots.data,
                actual: service.repository().data_dir().to_path_buf(),
            });
        }
        Ok(Self {
            service,
            roots,
            locale: Cell::new(locale),
            clock,
            platform,
        })
    }

    pub(super) const fn locale(&self) -> Locale {
        self.locale.get()
    }

    pub(super) fn initial_state(&self) -> Result<LibraryState, CliError> {
        let surface = crate::library::library_surface_at(
            self.service.repository(),
            &self.roots.state,
            &self.roots.config,
            self.clock.now_utc(),
        )?;
        let rerunnable = tui_rerunnable(&surface.scan, &self.roots.state);
        let mut state = LibraryState::from_library_surface(surface);
        let _ = state.update(UiAction::ReplaceRerunnable(rerunnable));
        Ok(state)
    }

    pub(super) fn preflight(&self, effect: &UiEffect) -> Result<(), CliError> {
        tui_preflight_effect_with_probe(
            self.service,
            self.service.repository(),
            effect,
            self.platform.run.probe(),
        )
    }

    fn run_services(&self) -> RunServices<'a> {
        RunServices::new(
            RunInvocation {
                tokens: self.token_context(),
                base_environment: self.platform.environment.variables(),
                locale: self.locale(),
                interpreter_policy: InterpreterPolicy::new_with_windows_environment(
                    self.platform.environment.platform(),
                    None,
                    self.platform.environment.variable("COMSPEC"),
                    self.platform.environment.variable("SystemRoot"),
                ),
            },
            self.platform.run,
            self.platform.allocator,
            self.platform.output,
            self.clock,
        )
    }

    fn editor_fallback(&self) -> Option<String> {
        ["VISUAL", "EDITOR"].into_iter().find_map(|name| {
            self.platform
                .environment
                .variable(name)
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.is_empty())
        })
    }

    fn editor_argv(&self) -> Vec<String> {
        let visual = self
            .platform
            .environment
            .variable("VISUAL")
            .and_then(|value| value.into_string().ok());
        let editor = self
            .platform
            .environment
            .variable("EDITOR")
            .and_then(|value| value.into_string().ok());
        resolve_editor_argv_with(&self.roots.config, visual.as_deref(), editor.as_deref())
    }

    fn token_context(&self) -> TokenContext {
        let local = self
            .clock
            .now_utc()
            .to_offset(self.platform.environment.local_offset());
        let time = local.time();
        TokenContext {
            cwd: self.roots.cwd.display().to_string(),
            home: self
                .roots
                .home
                .as_ref()
                .map(|path| path.display().to_string()),
            env: self.platform.environment.variables(),
            today: local.date().to_string(),
            now: format!(
                "{:02}-{:02}-{:02}",
                time.hour(),
                time.minute(),
                time.second()
            ),
        }
    }

    fn automatic_locale(&self) -> Locale {
        ["SKIT_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .find_map(|name| {
                self.platform
                    .environment
                    .variable(name)
                    .and_then(|value| requested_locale(value.to_str()))
            })
            .unwrap_or_else(|| self.platform.environment.system_locale())
    }

    pub(super) fn serve(&self, effect: UiEffect) -> Result<UiAction, CliError> {
        let store = self.service.repository();
        let locale = self.locale();
        match effect {
            UiEffect::None | UiEffect::Quit => Ok(UiAction::ClearStatus),
            UiEffect::Reload => {
                let surface = crate::library::library_surface_at(
                    store,
                    &self.roots.state,
                    &self.roots.config,
                    self.clock.now_utc(),
                )?;
                let rerunnable = tui_rerunnable(&surface.scan, &self.roots.state);
                Ok(UiAction::ReplaceSurface {
                    surface,
                    rerunnable,
                })
            }
            UiEffect::Rerun { selector } => {
                let run_services = self.run_services();
                tui_rerun_with_services(
                    self.service,
                    store,
                    &self.roots.state,
                    &self.roots.config,
                    &selector,
                    locale,
                    self.token_context(),
                    self.editor_fallback(),
                    &run_services,
                )
            }
            UiEffect::Open { request, selector } => {
                let screen = tui_open_with_context(
                    self.service,
                    store,
                    &self.roots.state,
                    &self.roots.config,
                    request,
                    selector,
                    locale,
                    self.token_context(),
                    self.editor_fallback(),
                    self.platform.run.probe(),
                )?;
                Ok(match screen {
                    Screen::Run(form)
                        if form
                            .context()
                            .is_some_and(|context| context.entry_kind == "prompt")
                            && !form.has_runner_picker() =>
                    {
                        UiAction::PromptRunnerRequired {
                            form,
                            cancel_status: text(
                                locale,
                                "A prompt needs a configured agent to run with.",
                            )
                            .into_owned(),
                        }
                    }
                    screen => UiAction::Present(screen),
                })
            }
            UiEffect::Preferences(effect) => {
                let action = tui_preferences_effect_at(
                    self.service,
                    &self.roots.config,
                    effect,
                    locale,
                    self.automatic_locale(),
                    AgentRoots {
                        home: self.roots.home.clone(),
                        cwd: self.roots.cwd.clone(),
                    },
                    PreferenceHost {
                        home: self.roots.home.as_deref(),
                        platform: self.platform.environment.platform(),
                        files: self.platform.preference_files,
                    },
                )?;
                if let UiAction::PreferencesSaved { locale, .. } = &action
                    && let Some(locale) = requested_locale(Some(locale))
                {
                    self.locale.set(locale);
                }
                Ok(action)
            }
            UiEffect::CountRunGlob {
                field,
                value,
                request,
                ..
            } => {
                let count = FileGlobExpander::new(&request.cwd).count_matches(&request);
                Ok(UiAction::SetRunGlobCount {
                    field,
                    value,
                    count,
                })
            }
            UiEffect::SaveRunPreset {
                selector,
                name,
                mut values,
                secret_names,
            } => {
                let entry = self.service.show(&selector)?;
                let declarations = entry_parameters(store, &entry);
                refuse_empty_preset_schema(&declarations)?;
                values.retain(|key, _| !secret_names.contains(key));
                let state = FormStateService::new(FileFormStateStore::new(&self.roots.state));
                state.save_preset(&entry.slug, &name, &declarations, &values)?;
                let presets = state.load(&entry.slug).presets;
                Ok(UiAction::RunPresetSaved {
                    message: format_text(locale, "Preset \"{}\" saved.", &[&name]),
                    name,
                    presets,
                })
            }
            UiEffect::HealthRebuild => {
                let rebuilt = HealthService::new(CliHealthInspector::new(
                    self.service,
                    store,
                    &self.roots.config,
                    locale,
                    self.platform.run.probe(),
                ))
                .rebuild()?;
                Ok(UiAction::Health(HealthAction::Rebuilt {
                    snapshot: Box::new(rebuilt.snapshot),
                    outcome: rebuilt.outcome,
                }))
            }
            UiEffect::SaveRunner { request, owner } => {
                tui_save_runner_at(self.service, &self.roots.config, request, owner, locale)
            }
            UiEffect::RemoveRunner(request) => {
                tui_remove_runner_at(self.service, &self.roots.config, request, locale)
            }
            UiEffect::RefreshPreferencesAfterRunners => Ok(UiAction::RunnerManagerClosed {
                preferences: Box::new(tui_preferences_view_with_context(
                    &self.roots.config,
                    locale,
                    self.editor_fallback(),
                )?),
            }),
            UiEffect::Add(effects) => {
                let pathext = self.platform.environment.variable("PATHEXT");
                tui_add_effect_at(
                    self.service,
                    store,
                    &self.roots.state,
                    &self.roots.config,
                    effects,
                    locale,
                    self.clock.now_utc(),
                    self.platform.editor,
                    &self.editor_argv(),
                    AddHostContext {
                        home: self.roots.home.as_deref(),
                        platform: self.platform.environment.platform(),
                        windows_pathext: pathext.as_deref(),
                        allocator: self.platform.allocator,
                    },
                )
            }
            UiEffect::Edit { selector } => {
                let argv = self.editor_argv();
                edit_with_runtime(
                    self.service,
                    store,
                    &selector,
                    true,
                    EditRuntime::new(
                        &argv,
                        self.platform.editor,
                        self.platform.output,
                        self.platform.terminal,
                        locale,
                    ),
                )?;
                tui_complete_at(
                    self.service,
                    &self.roots.state,
                    &self.roots.config,
                    self.clock.now_utc(),
                    "Source saved",
                )
            }
            UiEffect::Remove { selector } => {
                let held = self.service.show(&selector)?;
                let name = remove_with_state_dir(self.service, &held, &self.roots.state)?;
                self.platform
                    .output
                    .success(&format_text(locale, "Removed: {}", &[&name]));
                tui_complete_at(
                    self.service,
                    &self.roots.state,
                    &self.roots.config,
                    self.clock.now_utc(),
                    "Entry removed",
                )
            }
            UiEffect::Submit {
                purpose,
                selector,
                values,
            } => {
                let run_services =
                    (purpose == skit_ui::FormPurpose::Run).then(|| self.run_services());
                tui_submit_at(
                    self.service,
                    store,
                    &self.roots.state,
                    &self.roots.config,
                    purpose,
                    selector,
                    &values,
                    locale,
                    self.clock.now_utc(),
                    self.platform.output,
                    run_services.as_ref(),
                    PreferenceHost {
                        home: self.roots.home.as_deref(),
                        platform: self.platform.environment.platform(),
                        files: self.platform.preference_files,
                    },
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::{BTreeMap, VecDeque},
        ffi::OsString,
        io::{self, IsTerminal as _},
        path::{Path, PathBuf},
        time::{Duration, SystemTime},
    };

    use skit_application::{
        AgentScope, AgentTarget, CreateEntry, EntryPayload, LibraryService, SourcePermissions,
        form_state::{FormStateService, LastRunState},
        health::HealthIssueKind,
        library_detail::LibraryRunAge,
        preferences::PreferencesChangeSet,
        prompt_selection::PromptSelectionService,
    };
    use skit_domain::{
        EntryKind, EntrySettings, Slug, StorageMode,
        parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterValue},
    };
    use skit_i18n::{Locale, format_text, system_locale, text};
    use skit_language::write_managed_params;
    use skit_runtime::{
        DependencyCommand, DependencyCommandOutput, DependencyCommandRunner, InjectedCommand,
        InjectedCommandOutput, InjectedCommandRunner, InjectedCommandUnavailable,
        InterpreterPlatform, JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateRunner,
        JavaScriptSyntaxGateUnavailable, LaunchError, LaunchPlan, LaunchProcessOutput,
        LaunchRunner, ProgramProbe, UvArchiveFetchError, UvArchiveFetcher, UvDownloadConsent,
    };
    use skit_store::{
        AgentSkillInstallPoint, FileConfigStore, FileFormStateStore, FilePromptSelectionStore,
        FileStore,
    };
    use skit_ui::{
        Action as UiAction, AddAction, AddRequestId, AddWorkflowState, DraftKind, Effect,
        FieldValue, FormPurpose, HealthView, HostRequest, KnownEntryKind, LibrarySurface,
        PreferencesAction, PreferencesEffect, PreferencesView, ReviewDefaults, RunFormView,
        RunnerManagerAction, RunnerSaveOwner, RunnerSaveRequest, RunnerSaveTarget, Screen,
        SourceSnapshot,
    };
    use tempfile::TempDir;
    use time::{Date, Month, OffsetDateTime, Time, UtcOffset};

    use super::super::{
        CliError, expand_user_path_with_home, tui_add_source, user_home_from_environment,
    };
    use super::{
        EditorLauncher, HostEnvironment, HostOutput, PreferenceFiles, ProductRoots,
        SYSTEM_FILE_ALLOCATOR, SYSTEM_PREFERENCE_FILES, SystemEnvironment,
        SystemTerminalCapability, TerminalCapability, TuiHost, TuiPlatform,
    };
    use crate::run::{RunClock, RunError, RunPorts};

    /// Read the run form of one presented screen. Another action returns nothing.
    fn presented_run_form(action: UiAction) -> Option<Box<RunFormView>> {
        match action {
            UiAction::Present(Screen::Run(form)) => Some(form),
            _ => None,
        }
    }

    /// Read the Preferences view of one presented screen. Another action returns nothing.
    fn presented_preferences(action: UiAction) -> Option<Box<PreferencesView>> {
        match action {
            UiAction::Present(Screen::Preferences(view)) => Some(view),
            _ => None,
        }
    }

    /// Read the Health view of one presented screen. Another action returns nothing.
    fn presented_health(action: UiAction) -> Option<Box<HealthView>> {
        match action {
            UiAction::Present(Screen::Health(health)) => Some(health),
            _ => None,
        }
    }

    /// Read one completion that carries a full refreshed surface. Another action returns nothing.
    fn refreshed_completion(action: UiAction) -> Option<(LibrarySurface, Vec<Slug>, String)> {
        match action {
            UiAction::Complete {
                surface: Some(surface),
                rerunnable: Some(rerunnable),
                message,
            } => Some((surface, rerunnable, message)),
            _ => None,
        }
    }

    /// Read the message of one completion. Another action returns nothing.
    fn completion_message(action: UiAction) -> Option<String> {
        match action {
            UiAction::Complete { message, .. } => Some(message),
            _ => None,
        }
    }

    /// Read one host status line. Another action returns nothing.
    fn status_message(action: UiAction) -> Option<String> {
        match action {
            UiAction::SetStatus(message) => Some(message),
            _ => None,
        }
    }

    /// Read the discovered Agent Skill targets. Another action returns nothing.
    fn agent_skill_targets(action: UiAction) -> Option<Vec<AgentTarget>> {
        match action {
            UiAction::Preferences(PreferencesAction::PresentAgentSkillTargets(targets)) => {
                Some(targets)
            }
            _ => None,
        }
    }

    /// Read one accepted source inspection. Another action returns nothing.
    fn inspected_source(action: UiAction) -> Option<(AddRequestId, SourceSnapshot)> {
        match action {
            UiAction::Add(AddAction::SourceInspected {
                request,
                result: Ok(snapshot),
            }) => Some((request, snapshot)),
            _ => None,
        }
    }

    /// Report whether one error says that the library has no configured prompt runner.
    fn no_runners_configured(error: &CliError) -> bool {
        matches!(error, CliError::Run(RunError::NoRunnersConfigured))
    }

    #[test]
    fn every_shape_reader_refuses_one_other_action_and_one_other_error() {
        assert!(presented_run_form(UiAction::ClearStatus).is_none());
        assert!(presented_preferences(UiAction::ClearStatus).is_none());
        assert!(presented_health(UiAction::ClearStatus).is_none());
        assert!(refreshed_completion(UiAction::ClearStatus).is_none());
        assert!(completion_message(UiAction::ClearStatus).is_none());
        assert!(status_message(UiAction::ClearStatus).is_none());
        assert!(agent_skill_targets(UiAction::ClearStatus).is_none());
        assert!(inspected_source(UiAction::ClearStatus).is_none());
        assert!(!no_runners_configured(&CliError::Run(
            RunError::RunnerUnsupported
        )));
    }

    #[derive(Debug, Default)]
    struct RecordingEditor {
        calls: RefCell<Vec<(Vec<String>, PathBuf)>>,
    }

    impl EditorLauncher for RecordingEditor {
        fn launch(&self, argv: &[String], path: &Path) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push((argv.to_vec(), path.to_path_buf()));
            std::fs::write(path, b"print('edited')\n")
        }
    }

    #[derive(Debug)]
    struct PromptEditor;

    impl EditorLauncher for PromptEditor {
        fn launch(&self, _argv: &[String], path: &Path) -> io::Result<()> {
            std::fs::write(path, b"Review {{fresh}}\n")
        }
    }

    #[derive(Debug)]
    struct FixedTerminal(bool);

    impl TerminalCapability for FixedTerminal {
        fn stdin_is_terminal(&self) -> bool {
            self.0
        }

        fn stdout_is_terminal(&self) -> bool {
            self.0
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    enum OutputEvent {
        Success(String),
        Detail(String),
        Plain(String),
        Stdout(String),
        Diagnostic(String),
    }

    #[derive(Debug, Default)]
    struct RecordingOutput(RefCell<Vec<OutputEvent>>);

    impl HostOutput for RecordingOutput {
        fn success(&self, message: &str) {
            self.0
                .borrow_mut()
                .push(OutputEvent::Success(message.to_owned()));
        }

        fn detail(&self, message: &str) {
            self.0
                .borrow_mut()
                .push(OutputEvent::Detail(message.to_owned()));
        }

        fn plain(&self, message: &str) {
            self.0
                .borrow_mut()
                .push(OutputEvent::Plain(message.to_owned()));
        }

        fn stdout(&self, message: &str) {
            self.0
                .borrow_mut()
                .push(OutputEvent::Stdout(message.to_owned()));
        }

        fn diagnostic(&self, message: &str) {
            self.0
                .borrow_mut()
                .push(OutputEvent::Diagnostic(message.to_owned()));
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum PreferenceFileCall {
        File(PathBuf),
        Directory(PathBuf),
        AgentSkillCheckpoint {
            point: AgentSkillInstallPoint,
            path: PathBuf,
        },
    }

    #[derive(Debug, Default)]
    struct RecordingPreferenceFiles {
        calls: RefCell<Vec<PreferenceFileCall>>,
        fail_point: Cell<Option<AgentSkillInstallPoint>>,
    }

    impl PreferenceFiles for RecordingPreferenceFiles {
        fn is_file(&self, path: &Path) -> bool {
            self.calls
                .borrow_mut()
                .push(PreferenceFileCall::File(path.to_path_buf()));
            SYSTEM_PREFERENCE_FILES.is_file(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.calls
                .borrow_mut()
                .push(PreferenceFileCall::Directory(path.to_path_buf()));
            SYSTEM_PREFERENCE_FILES.is_dir(path)
        }

        fn agent_skill_checkpoint(
            &self,
            point: AgentSkillInstallPoint,
            path: &Path,
        ) -> io::Result<()> {
            self.calls
                .borrow_mut()
                .push(PreferenceFileCall::AgentSkillCheckpoint {
                    point,
                    path: path.to_path_buf(),
                });
            if self.fail_point.get() == Some(point) {
                return Err(io::Error::other("injected preference filesystem failure"));
            }
            Ok(())
        }
    }

    #[derive(Debug)]
    struct NoopEditor;

    impl EditorLauncher for NoopEditor {
        fn launch(&self, _argv: &[String], _path: &Path) -> io::Result<()> {
            panic!("this contract must not launch an editor")
        }
    }

    #[derive(Debug)]
    struct NoopLaunchRunner;

    impl LaunchRunner for NoopLaunchRunner {
        fn run(&self, _plan: &LaunchPlan) -> io::Result<LaunchProcessOutput> {
            panic!("this contract must not launch a child process")
        }
    }

    #[derive(Debug)]
    enum RecordedLaunchOutcome {
        Exit(i32),
        Failure(io::ErrorKind, &'static str),
    }

    #[derive(Debug)]
    struct RecordingLaunchRunner {
        plans: RefCell<Vec<LaunchPlan>>,
        source_files: RefCell<Vec<(PathBuf, Vec<u8>)>>,
        outcomes: RefCell<VecDeque<RecordedLaunchOutcome>>,
    }

    impl RecordingLaunchRunner {
        fn new(outcomes: impl IntoIterator<Item = RecordedLaunchOutcome>) -> Self {
            Self {
                plans: RefCell::new(Vec::new()),
                source_files: RefCell::new(Vec::new()),
                outcomes: RefCell::new(outcomes.into_iter().collect()),
            }
        }
    }

    impl LaunchRunner for RecordingLaunchRunner {
        fn run(&self, plan: &LaunchPlan) -> io::Result<LaunchProcessOutput> {
            self.plans.borrow_mut().push(plan.clone());
            for argument in &plan.args {
                let path = PathBuf::from(argument);
                if path.is_file() {
                    self.source_files
                        .borrow_mut()
                        .push((path.clone(), std::fs::read(path)?));
                }
            }
            match self
                .outcomes
                .borrow_mut()
                .pop_front()
                .expect("the contract must provide one result per launch")
            {
                RecordedLaunchOutcome::Exit(exit_code) => Ok(LaunchProcessOutput {
                    exit_code: Some(exit_code),
                    signal: None,
                }),
                RecordedLaunchOutcome::Failure(kind, message) => Err(io::Error::new(kind, message)),
            }
        }
    }

    #[derive(Debug)]
    struct NoopDependencyRunner;

    impl DependencyCommandRunner for NoopDependencyRunner {
        fn run(&self, _command: &DependencyCommand) -> io::Result<DependencyCommandOutput> {
            panic!("this contract must not run a dependency command")
        }
    }

    #[derive(Debug)]
    struct RecordingDependencyRunner {
        commands: RefCell<Vec<DependencyCommand>>,
        success: bool,
    }

    impl RecordingDependencyRunner {
        fn new(success: bool) -> Self {
            Self {
                commands: RefCell::new(Vec::new()),
                success,
            }
        }
    }

    impl DependencyCommandRunner for RecordingDependencyRunner {
        fn run(&self, command: &DependencyCommand) -> io::Result<DependencyCommandOutput> {
            self.commands.borrow_mut().push(command.clone());
            if self.success {
                std::fs::create_dir_all(command.cwd.join("node_modules"))?;
            } else {
                std::fs::write(command.cwd.join("package.json"), b"partial manifest\n")?;
                std::fs::write(command.cwd.join("package-lock.json"), b"partial lock\n")?;
                std::fs::create_dir_all(command.cwd.join("node_modules"))?;
                std::fs::write(
                    command.cwd.join("node_modules/partial.txt"),
                    b"partial install\n",
                )?;
            }
            Ok(DependencyCommandOutput {
                success: self.success,
                exit_code: Some(if self.success { 0 } else { 9 }),
                stderr: if self.success {
                    Vec::new()
                } else {
                    b"offline dependency failure".to_vec()
                },
            })
        }
    }

    #[derive(Debug)]
    struct NoopInjectedRunner;

    impl InjectedCommandRunner for NoopInjectedRunner {
        fn run(
            &self,
            _command: &InjectedCommand,
        ) -> Result<InjectedCommandOutput, InjectedCommandUnavailable> {
            panic!("this contract must not run an injected-source command")
        }
    }

    #[derive(Debug)]
    struct RecordingInjectedRunner {
        calls: RefCell<Vec<(InjectedCommand, Vec<u8>)>>,
        success: bool,
    }

    impl InjectedCommandRunner for RecordingInjectedRunner {
        fn run(
            &self,
            command: &InjectedCommand,
        ) -> Result<InjectedCommandOutput, InjectedCommandUnavailable> {
            let source = command
                .args
                .last()
                .map(PathBuf::from)
                .and_then(|path| std::fs::read(path).ok())
                .unwrap_or_default();
            self.calls.borrow_mut().push((command.clone(), source));
            Ok(InjectedCommandOutput {
                success: self.success,
                stderr: if self.success {
                    Vec::new()
                } else {
                    b"syntax rejected".to_vec()
                },
            })
        }
    }

    #[derive(Debug)]
    struct NoopJavaScriptGate;

    impl JavaScriptSyntaxGateRunner for NoopJavaScriptGate {
        fn check(
            &self,
            _program: &Path,
            _source: &Path,
            _timeout: Duration,
        ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
            panic!("this contract must not run a JavaScript syntax gate")
        }
    }

    #[derive(Debug)]
    struct RecordingJavaScriptGate {
        calls: RefCell<Vec<JavaScriptGateCall>>,
        success: bool,
    }

    type JavaScriptGateCall = (PathBuf, PathBuf, Duration, Vec<u8>);

    impl JavaScriptSyntaxGateRunner for RecordingJavaScriptGate {
        fn check(
            &self,
            program: &Path,
            source: &Path,
            timeout: Duration,
        ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
            self.calls.borrow_mut().push((
                program.to_path_buf(),
                source.to_path_buf(),
                timeout,
                std::fs::read(source).unwrap(),
            ));
            Ok(JavaScriptSyntaxGateOutput {
                success: self.success,
                stderr: if self.success {
                    Vec::new()
                } else {
                    b"syntax rejected".to_vec()
                },
            })
        }
    }

    #[derive(Debug)]
    struct NoopUvConsent;

    impl UvDownloadConsent for NoopUvConsent {
        fn allow_download(&self, _version: &str, _destination: &Path) -> bool {
            panic!("this contract must not request a uv download")
        }
    }

    #[derive(Debug)]
    struct NoopUvFetcher;

    impl UvArchiveFetcher for NoopUvFetcher {
        fn fetch(&self, _url: &str, _limit: u64) -> Result<Vec<u8>, UvArchiveFetchError> {
            panic!("this contract must not fetch a uv archive")
        }
    }

    #[derive(Debug, Default)]
    struct RejectingUvFetcher(RefCell<Vec<(String, u64)>>);

    impl UvArchiveFetcher for RejectingUvFetcher {
        fn fetch(&self, url: &str, limit: u64) -> Result<Vec<u8>, UvArchiveFetchError> {
            self.0.borrow_mut().push((url.to_owned(), limit));
            Err(UvArchiveFetchError::new("network disabled by walker"))
        }
    }

    #[derive(Debug)]
    struct FixedProbe;

    impl ProgramProbe for FixedProbe {
        fn find_program(&self, _name: &str) -> Option<PathBuf> {
            None
        }

        fn is_file(&self, path: &Path) -> bool {
            path.is_file()
        }

        fn is_dir(&self, path: &Path) -> bool {
            path.is_dir()
        }

        fn is_executable(&self, _path: &Path) -> bool {
            false
        }
    }

    #[derive(Debug, Default)]
    struct RecordingProbe {
        programs: BTreeMap<String, PathBuf>,
        calls: RefCell<Vec<String>>,
    }

    impl ProgramProbe for RecordingProbe {
        fn find_program(&self, name: &str) -> Option<PathBuf> {
            self.calls.borrow_mut().push(format!("find:{name}"));
            self.programs.get(name).cloned()
        }

        fn is_file(&self, path: &Path) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("file:{}", path.display()));
            path.is_file()
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("dir:{}", path.display()));
            path.is_dir()
        }

        fn is_executable(&self, path: &Path) -> bool {
            self.calls
                .borrow_mut()
                .push(format!("executable:{}", path.display()));
            false
        }
    }

    static NOOP_EDITOR: NoopEditor = NoopEditor;
    static NOOP_LAUNCH: NoopLaunchRunner = NoopLaunchRunner;
    static NOOP_DEPENDENCIES: NoopDependencyRunner = NoopDependencyRunner;
    static NOOP_INJECTED: NoopInjectedRunner = NoopInjectedRunner;
    static NOOP_JAVASCRIPT_GATE: NoopJavaScriptGate = NoopJavaScriptGate;
    static NOOP_UV_CONSENT: NoopUvConsent = NoopUvConsent;
    static NOOP_UV_FETCHER: NoopUvFetcher = NoopUvFetcher;
    static INTERACTIVE_TERMINAL: FixedTerminal = FixedTerminal(true);
    static NONINTERACTIVE_TERMINAL: FixedTerminal = FixedTerminal(false);
    static FIXED_PROBE: FixedProbe = FixedProbe;

    fn test_run_ports(probe: &dyn ProgramProbe) -> RunPorts<'_> {
        run_ports(probe, &NOOP_LAUNCH)
    }

    fn run_ports<'a>(probe: &'a dyn ProgramProbe, launch: &'a dyn LaunchRunner) -> RunPorts<'a> {
        RunPorts::new(
            probe,
            launch,
            &NOOP_DEPENDENCIES,
            &NOOP_INJECTED,
            &NOOP_JAVASCRIPT_GATE,
            &NOOP_UV_CONSENT,
            &NOOP_UV_FETCHER,
        )
    }

    fn test_platform<'a>(
        environment: &'a dyn HostEnvironment,
        output: &'a RecordingOutput,
    ) -> TuiPlatform<'a> {
        TuiPlatform::test(
            environment,
            &NONINTERACTIVE_TERMINAL,
            &NOOP_EDITOR,
            test_run_ports(&FIXED_PROBE),
            output,
        )
    }

    #[derive(Debug)]
    struct FixedClock(OffsetDateTime);

    impl RunClock for FixedClock {
        fn now_utc(&self) -> OffsetDateTime {
            self.0
        }
    }

    #[derive(Debug)]
    struct MutableClock(Cell<OffsetDateTime>);

    impl RunClock for MutableClock {
        fn now_utc(&self) -> OffsetDateTime {
            self.0.get()
        }
    }

    #[derive(Debug)]
    struct AdvancingLaunchRunner<'a> {
        inner: &'a RecordingLaunchRunner,
        clock: &'a MutableClock,
        completed_at: Cell<OffsetDateTime>,
    }

    impl LaunchRunner for AdvancingLaunchRunner<'_> {
        fn run(&self, plan: &LaunchPlan) -> io::Result<LaunchProcessOutput> {
            let result = self.inner.run(plan);
            self.clock.0.set(self.completed_at.get());
            result
        }
    }

    #[derive(Debug)]
    struct FixedEnvironment {
        variables: BTreeMap<String, OsString>,
        system_locale: Locale,
        platform: InterpreterPlatform,
    }

    impl HostEnvironment for FixedEnvironment {
        fn platform(&self) -> InterpreterPlatform {
            self.platform
        }

        fn variable(&self, name: &str) -> Option<OsString> {
            self.variables.get(name).cloned()
        }

        fn variables(&self) -> BTreeMap<String, String> {
            self.variables
                .iter()
                .filter_map(|(key, value)| {
                    value.to_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        }

        fn local_offset(&self) -> UtcOffset {
            UtcOffset::UTC
        }

        fn system_locale(&self) -> Locale {
            self.system_locale
        }
    }

    #[derive(Debug)]
    struct NoRunSnapshotEnvironment;

    impl HostEnvironment for NoRunSnapshotEnvironment {
        fn platform(&self) -> InterpreterPlatform {
            InterpreterPlatform::Other
        }

        fn variable(&self, _name: &str) -> Option<OsString> {
            None
        }

        fn variables(&self) -> BTreeMap<String, String> {
            panic!("a non-run submit must not read the run environment snapshot")
        }

        fn local_offset(&self) -> UtcOffset {
            UtcOffset::UTC
        }

        fn system_locale(&self) -> Locale {
            Locale::En
        }
    }

    #[cfg(unix)]
    #[derive(Debug)]
    struct PermissionRestore {
        path: PathBuf,
        permissions: std::fs::Permissions,
    }

    #[cfg(unix)]
    impl Drop for PermissionRestore {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.path, self.permissions.clone()).unwrap();
        }
    }

    fn fixed_clock() -> FixedClock {
        FixedClock(
            Date::from_calendar_date(2026, Month::August, 28)
                .unwrap()
                .with_time(Time::from_hms(12, 34, 56).unwrap())
                .assume_utc(),
        )
    }

    fn add_command(service: &LibraryService<FileStore>, name: &str) -> skit_domain::Entry {
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings {
                    template: "printf ok".to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn add_parameter_command(
        service: &LibraryService<FileStore>,
        name: &str,
    ) -> skit_domain::Entry {
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings {
                    template: "printf {value}".to_owned(),
                    params: vec!["value".to_owned()],
                    parameters: vec![ParamDecl::new("value")],
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn add_prompt(service: &LibraryService<FileStore>, name: &str) -> skit_domain::Entry {
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("prompt").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"Review this".to_vec(),
                    stored_name: Some("prompt.md".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    params: vec!["gone".to_owned()],
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn add_python(service: &LibraryService<FileStore>, name: &str) -> skit_domain::Entry {
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"print('before')\n".to_vec(),
                    stored_name: Some("script.py".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap()
    }

    fn add_command_with_need(
        service: &LibraryService<FileStore>,
        name: &str,
        need: &str,
    ) -> skit_domain::Entry {
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings {
                    template: "printf ok".to_owned(),
                    needs: vec![need.to_owned()],
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn add_javascript(service: &LibraryService<FileStore>, name: &str) -> skit_domain::Entry {
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse("js").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"console.log('ok');\n".to_vec(),
                    stored_name: Some("script.js".to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    dependencies: vec!["left-pad".to_owned()],
                    interpreter: "node".to_owned(),
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn add_injected_script(
        service: &LibraryService<FileStore>,
        kind: &str,
        name: &str,
        source: &str,
        stored_name: &str,
    ) -> skit_domain::Entry {
        let mut declaration = ParamDecl::new("TOKEN");
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.default = Some(ParameterValue::String("before".to_owned()));
        let source =
            write_managed_params(kind, source, std::slice::from_ref(&declaration)).unwrap();
        service
            .add(CreateEntry {
                name: name.to_owned(),
                kind: EntryKind::parse(kind).unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: source.into_bytes(),
                    stored_name: Some(stored_name.to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    parameters: vec![declaration],
                    ..EntrySettings::default()
                },
            })
            .unwrap()
    }

    fn roots(root: &Path) -> ProductRoots {
        ProductRoots::new(
            root.join("data"),
            root.join("state"),
            root.join("config"),
            Some(root.join("home")),
            root.join("cwd"),
        )
    }

    fn tree_snapshot(root: &Path) -> Vec<(PathBuf, Option<Vec<u8>>)> {
        fn visit(root: &Path, directory: &Path, items: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
            let mut children = std::fs::read_dir(directory)
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            children.sort_by_key(std::fs::DirEntry::file_name);
            for child in children {
                let path = child.path();
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                if path.is_dir() {
                    items.push((relative, None));
                    visit(root, &path, items);
                } else {
                    items.push((relative, Some(std::fs::read(path).unwrap())));
                }
            }
        }

        let mut items = Vec::new();
        visit(root, root, &mut items);
        items
    }

    fn seed_future_cleanup_sentinels(entry_dir: &Path) -> (PathBuf, PathBuf) {
        let injected = entry_dir.join(".injected-future.sh");
        let launch = entry_dir.join(".run-future.sh");
        std::fs::write(&injected, b"future injected sentinel\n").unwrap();
        std::fs::write(&launch, b"future launch sentinel\n").unwrap();
        let modified = SystemTime::UNIX_EPOCH
            + Duration::from_secs(
                u64::try_from(
                    Date::from_calendar_date(2010, Month::January, 1)
                        .unwrap()
                        .midnight()
                        .assume_utc()
                        .unix_timestamp(),
                )
                .unwrap(),
            );
        for path in [&injected, &launch] {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(modified))
                .unwrap();
        }
        (injected, launch)
    }

    fn cleanup_test_clock() -> FixedClock {
        FixedClock(
            Date::from_calendar_date(2000, Month::January, 1)
                .unwrap()
                .midnight()
                .assume_utc(),
        )
    }

    fn poison_environment(root: &Path) -> FixedEnvironment {
        FixedEnvironment {
            variables: BTreeMap::from([
                (
                    "SKIT_DATA_DIR".to_owned(),
                    root.join("poison-data").into_os_string(),
                ),
                (
                    "SKIT_STATE_DIR".to_owned(),
                    root.join("poison-state").into_os_string(),
                ),
                (
                    "SKIT_CONFIG_DIR".to_owned(),
                    root.join("poison-config").into_os_string(),
                ),
                ("SKIT_LANG".to_owned(), OsString::from("zh-CN")),
                ("LANG".to_owned(), OsString::from("zh_CN.UTF-8")),
                ("VISUAL".to_owned(), OsString::from("poison-editor")),
                ("SENTINEL".to_owned(), OsString::from("explicit-host")),
                (
                    "NPM_CONFIG_REGISTRY".to_owned(),
                    OsString::from("https://host.example/npm"),
                ),
            ]),
            system_locale: Locale::ZhCn,
            platform: InterpreterPlatform::Other,
        }
    }

    #[test]
    fn explicit_roots_and_locale_own_reload_open_remove_and_complete() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        std::fs::create_dir_all(roots.home.as_ref().unwrap()).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let kept = add_command(&service, "Kept");
        let removed = add_command(&service, "Removed");
        FileConfigStore::new(&roots.config)
            .set_many(&BTreeMap::from([
                ("lang".to_owned(), "zh-TW".to_owned()),
                ("editor".to_owned(), "configured-editor".to_owned()),
            ]))
            .unwrap();
        FormStateService::new(FileFormStateStore::new(&roots.state))
            .record_run(
                &removed.slug,
                0,
                "2026-08-20T00:00:00+00:00",
                &[],
                Some(&BTreeMap::new()),
            )
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::ZhTw,
            &clock,
            test_platform(&environment, &output),
        )
        .unwrap();

        assert_eq!(host.locale(), Locale::ZhTw);
        let first = host.serve(Effect::Reload).unwrap();
        let second = host.serve(Effect::Reload).unwrap();
        assert_eq!(first, second, "a fixed instant must stabilize the surface");

        let opened = host
            .serve(Effect::Open {
                request: HostRequest::Run,
                selector: Some(kept.slug.as_str().to_owned()),
            })
            .unwrap();
        let form = presented_run_form(opened).expect("the production host must open the run form");
        let context = form.context().unwrap();
        assert_eq!(context.tokens.cwd, roots.cwd.display().to_string());
        assert_eq!(
            context.tokens.home.as_deref(),
            roots
                .home
                .as_ref()
                .map(|path| path.display().to_string())
                .as_deref()
        );

        FormStateService::new(FileFormStateStore::new(&roots.state))
            .record_run(
                &kept.slug,
                0,
                "2026-08-28T12:30:00+00:00",
                &[],
                Some(&BTreeMap::new()),
            )
            .unwrap();
        let removed_action = host
            .serve(Effect::Remove {
                selector: removed.slug.as_str().to_owned(),
            })
            .unwrap();
        let (surface, rerunnable, message) = refreshed_completion(removed_action)
            .expect("remove must return one complete refreshed surface");
        assert_eq!(message, "Entry removed");
        assert_eq!(surface.scan.entries.len(), 1);
        assert_eq!(surface.scan.entries[0].slug, kept.slug);
        assert_eq!(rerunnable.as_slice(), std::slice::from_ref(&kept.slug));
        assert_eq!(
            surface.details[&kept.slug].last_run.as_ref().unwrap().age,
            LibraryRunAge::Minutes(4)
        );
        assert!(service.show(removed.slug.as_str()).is_err());
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state)).last_run(&removed.slug),
            LastRunState::default()
        );
        assert_eq!(
            output.0.borrow().as_slice(),
            [OutputEvent::Success(format_text(
                Locale::ZhTw,
                "Removed: {}",
                &[&"Removed"]
            ))]
        );
        for poison in ["poison-data", "poison-state", "poison-config"] {
            assert!(!sandbox.path().join(poison).exists());
        }
    }

    #[test]
    fn explicit_locale_does_not_read_ambient_language_or_editor_values() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        FileConfigStore::new(&roots.config)
            .set_many(&BTreeMap::from([
                ("lang".to_owned(), "en".to_owned()),
                ("editor".to_owned(), String::new()),
            ]))
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            test_platform(&environment, &output),
        )
        .unwrap();

        assert_eq!(host.locale(), Locale::En);
        let view = presented_preferences(
            host.serve(Effect::Open {
                request: HostRequest::Preferences,
                selector: None,
            })
            .unwrap(),
        )
        .expect("the production host must open Preferences");
        assert_eq!(view.draft().effective_language, "en");
        assert_eq!(
            view.draft().editor_fallback.as_deref(),
            Some("poison-editor")
        );
    }

    #[test]
    fn a_saved_language_becomes_the_persistent_host_locale_immediately() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let command = add_parameter_command(&service, "Parameterized");
        let prompt = add_prompt(&service, "Prompt");
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots,
            Locale::En,
            &clock,
            test_platform(&environment, &output),
        )
        .unwrap();

        let saved = host
            .serve(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([("lang".to_owned(), "zh-TW".to_owned())]),
                },
            )))
            .unwrap();
        assert!(matches!(
            saved,
            skit_ui::Action::PreferencesSaved { ref locale, .. } if locale == "zh-TW"
        ));
        assert_eq!(host.locale(), Locale::ZhTw);

        let preset = host
            .serve(Effect::SaveRunPreset {
                selector: command.slug.as_str().to_owned(),
                name: "常用".to_owned(),
                values: BTreeMap::from([("value".to_owned(), "ok".to_owned())]),
                secret_names: Default::default(),
            })
            .unwrap();
        assert!(matches!(
            preset,
            skit_ui::Action::RunPresetSaved { ref message, .. }
                if message == &format_text(Locale::ZhTw, "Preset \"{}\" saved.", &[&"常用"])
        ));

        let opened = host
            .serve(Effect::Open {
                request: HostRequest::Run,
                selector: Some(prompt.slug.as_str().to_owned()),
            })
            .unwrap();
        let form = presented_run_form(opened).expect("run must open after the locale changes");
        assert_eq!(
            form.drift_lines,
            [format_text(
                Locale::ZhTw,
                "No longer in the prompt (the value would be ignored): {} — edit the body or update parameters with: skit params {}",
                &[&"gone", &"Prompt"],
            )]
        );

        let runner_failure = host
            .serve(Effect::SaveRunner {
                request: RunnerSaveRequest {
                    name: "agent".to_owned(),
                    argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
                    target: RunnerSaveTarget::Named {
                        name: "agent".to_owned(),
                        expected: Vec::new(),
                    },
                },
                owner: RunnerSaveOwner::Manager,
            })
            .unwrap();
        assert!(matches!(
            runner_failure,
            skit_ui::Action::Runners(RunnerManagerAction::MutationFailed(ref message))
                if message == text(
                    Locale::ZhTw,
                    "The runner row changed before it could be saved; inspect again."
                ).as_ref()
        ));
    }

    #[cfg(unix)]
    #[test]
    fn preference_files_own_validation_discovery_and_agent_skill_installation() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(roots.home.as_ref().unwrap()).unwrap();
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let outside = sandbox.path().join("outside");
        let outside_agent = outside.join("agent-home");
        std::fs::create_dir_all(&outside_agent).unwrap();
        let outside_bash = outside.join("bash.exe");
        std::fs::write(&outside_bash, b"outside bash bytes\n").unwrap();
        std::os::unix::fs::symlink(
            &outside_bash,
            roots.home.as_ref().unwrap().join("bash-link"),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside_agent, roots.home.as_ref().unwrap().join(".codex"))
            .unwrap();
        std::fs::create_dir_all(roots.cwd.join(".agents")).unwrap();
        let outside_initial = tree_snapshot(&outside);
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let preference_files = RecordingPreferenceFiles::default();
        let platform = TuiPlatform::new(
            &environment,
            &NONINTERACTIVE_TERMINAL,
            &NOOP_EDITOR,
            test_run_ports(&FIXED_PROBE),
            &SYSTEM_FILE_ALLOCATOR,
            &output,
            &preference_files,
        );
        let host = TuiHost::new(&service, roots.clone(), Locale::En, &clock, platform).unwrap();

        let saved = host
            .serve(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([
                        ("editor".to_owned(), "~/editor".to_owned()),
                        ("lang".to_owned(), "en".to_owned()),
                        ("shell.bash_path".to_owned(), "~/bash-link".to_owned()),
                    ]),
                },
            )))
            .unwrap();
        assert!(
            matches!(saved, skit_ui::Action::PreferencesSaved { .. }),
            "{saved:?}"
        );
        assert_eq!(
            FileConfigStore::new(&roots.config)
                .get("shell.bash_path")
                .unwrap(),
            "~/bash-link"
        );
        assert_eq!(
            FileConfigStore::new(&roots.config).get("editor").unwrap(),
            "~/editor"
        );
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [PreferenceFileCall::File(
                roots.home.as_ref().unwrap().join("bash-link")
            )]
        );

        let bash_directory = roots.home.as_ref().unwrap().join("bash-directory");
        std::fs::create_dir(&bash_directory).unwrap();
        let config_before_refusal = std::fs::read(roots.config.join("config.toml")).unwrap();
        preference_files.calls.borrow_mut().clear();
        assert!(matches!(
            host.serve(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([(
                        "shell.bash_path".to_owned(),
                        "~/bash-directory".to_owned(),
                    )]),
                },
            )))
            .unwrap(),
            skit_ui::Action::Preferences(skit_ui::PreferencesAction::ValidationFailed(_))
        ));
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [PreferenceFileCall::File(bash_directory)]
        );
        assert_eq!(
            std::fs::read(roots.config.join("config.toml")).unwrap(),
            config_before_refusal
        );

        preference_files.calls.borrow_mut().clear();
        assert!(matches!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Preferences,
                selector: None,
                values: BTreeMap::from([(
                    "shell.bash_path".to_owned(),
                    FieldValue::text("~/bash-link"),
                )]),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [PreferenceFileCall::File(
                roots.home.as_ref().unwrap().join("bash-link")
            )]
        );

        preference_files.calls.borrow_mut().clear();
        let discovered = host
            .serve(Effect::Preferences(
                PreferencesEffect::DiscoverAgentSkillTargets,
            ))
            .unwrap();
        let view =
            agent_skill_targets(discovered).expect("discovery must return the target picker");
        assert_eq!(
            view.as_slice(),
            [
                AgentTarget {
                    name: "codex".to_owned(),
                    scope: AgentScope::User,
                    base: roots.home.as_ref().unwrap().join(".codex"),
                },
                AgentTarget {
                    name: "agents".to_owned(),
                    scope: AgentScope::Project,
                    base: roots.cwd.join(".agents"),
                },
            ]
        );
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [
                PreferenceFileCall::Directory(roots.home.as_ref().unwrap().join(".claude")),
                PreferenceFileCall::Directory(roots.home.as_ref().unwrap().join(".codex")),
                PreferenceFileCall::Directory(roots.cwd.join(".claude")),
                PreferenceFileCall::Directory(roots.cwd.join(".codex")),
                PreferenceFileCall::Directory(roots.cwd.join(".agents")),
            ]
        );

        assert_eq!(tree_snapshot(&outside), outside_initial);
        preference_files.calls.borrow_mut().clear();
        let discovered_skills_dir = roots.home.as_ref().unwrap().join(".codex/skills");
        assert!(matches!(
            host.serve(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: discovered_skills_dir.clone(),
            }))
            .unwrap(),
            skit_ui::Action::Preferences(skit_ui::PreferencesAction::AgentSkillInstalled { .. })
        ));
        let embedded = include_bytes!("../../../../skills/skit/SKILL.md");
        assert!(
            std::fs::symlink_metadata(roots.home.as_ref().unwrap().join(".codex"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read(outside_agent.join("skills/skit/SKILL.md")).unwrap(),
            embedded
        );
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::Inspect,
                    path: discovered_skills_dir.join("skit/SKILL.md"),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: discovered_skills_dir.clone(),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: discovered_skills_dir.join("skit"),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeReplace,
                    path: discovered_skills_dir.join("skit/SKILL.md"),
                },
            ]
        );
        let outside_before = tree_snapshot(&outside);

        preference_files.calls.borrow_mut().clear();
        let skills_dir = roots.cwd.join(".agents/skills");
        assert!(!skills_dir.exists());
        assert!(matches!(
            host.serve(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: skills_dir.clone(),
            },))
                .unwrap(),
            skit_ui::Action::Preferences(skit_ui::PreferencesAction::AgentSkillInstalled { .. })
        ));
        assert_eq!(
            std::fs::read(skills_dir.join("skit/SKILL.md")).unwrap(),
            embedded
        );
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::Inspect,
                    path: skills_dir.join("skit/SKILL.md"),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: skills_dir.clone(),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: skills_dir.join("skit"),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeReplace,
                    path: skills_dir.join("skit/SKILL.md"),
                },
            ]
        );

        preference_files.calls.borrow_mut().clear();
        let linked_skills_dir = roots.cwd.join(".codex/skills");
        std::fs::create_dir_all(linked_skills_dir.join("skit")).unwrap();
        let linked_target = roots.cwd.join("linked-skill-target.md");
        std::fs::write(&linked_target, b"old linked skill\n").unwrap();
        let linked_destination = linked_skills_dir.join("skit/SKILL.md");
        std::os::unix::fs::symlink(&linked_target, &linked_destination).unwrap();
        assert!(matches!(
            host.serve(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: linked_skills_dir.clone(),
            },))
                .unwrap(),
            skit_ui::Action::Preferences(skit_ui::PreferencesAction::AgentSkillInstalled { .. })
        ));
        assert!(
            std::fs::symlink_metadata(&linked_destination)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read(&linked_target).unwrap(), embedded);
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::Inspect,
                    path: linked_destination.clone(),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::ReadLink,
                    path: linked_destination,
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::Inspect,
                    path: linked_target.clone(),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeReplace,
                    path: linked_target,
                },
            ]
        );

        preference_files.calls.borrow_mut().clear();
        let blocker = roots.cwd.join("blocker");
        std::fs::write(&blocker, b"blocking bytes\n").unwrap();
        let blocked = host
            .serve(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: blocker.join("skills"),
            }))
            .unwrap();
        let blocked_message =
            status_message(blocked).expect("a blocking parent must return a status");
        let blocked_target = blocker.join("skills/skit/SKILL.md");
        assert!(
            blocked_message.contains("inspect")
                && blocked_message.contains(&blocked_target.display().to_string())
                && blocked_message.contains("Not a directory"),
            "{blocked_message}"
        );
        assert_eq!(std::fs::read(&blocker).unwrap(), b"blocking bytes\n");

        preference_files.calls.borrow_mut().clear();
        preference_files
            .fail_point
            .set(Some(AgentSkillInstallPoint::BeforeReplace));
        let tree_before_fault = tree_snapshot(sandbox.path());
        let fault_skills = roots.cwd.join("fault/skills");
        let failed = host
            .serve(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: fault_skills.clone(),
            }))
            .unwrap();
        let fault_target = fault_skills.join("skit/SKILL.md");
        assert!(matches!(
            failed,
            skit_ui::Action::SetStatus(ref message)
                if message.contains("replace")
                    && message.contains(&fault_target.display().to_string())
                    && message.contains("injected preference filesystem failure")
        ));
        assert_eq!(
            preference_files.calls.borrow().as_slice(),
            [
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::Inspect,
                    path: fault_target.clone(),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: roots.cwd.join("fault"),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: fault_skills.clone(),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeCreateDirectory,
                    path: fault_skills.join("skit"),
                },
                PreferenceFileCall::AgentSkillCheckpoint {
                    point: AgentSkillInstallPoint::BeforeReplace,
                    path: fault_target,
                },
            ]
        );
        assert_eq!(tree_snapshot(sandbox.path()), tree_before_fault);
        assert_eq!(tree_snapshot(&outside), outside_before);
    }

    #[test]
    fn injected_editor_runs_author_draft_source_and_stored_copy_pipelines() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let stored = add_python(&service, "Stored");
        FileConfigStore::new(&roots.config)
            .set("editor", "must-not-run --wait")
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let editor = RecordingEditor::default();
        let output = RecordingOutput::default();
        let platform = TuiPlatform::test(
            &environment,
            &NONINTERACTIVE_TERMINAL,
            &editor,
            test_run_ports(&FIXED_PROBE),
            &output,
        );
        let host = TuiHost::new(&service, roots.clone(), Locale::En, &clock, platform).unwrap();

        let mut author = AddWorkflowState::new(Vec::new());
        let authored = host
            .serve(Effect::Add(
                author.reduce(AddAction::NewDraft(DraftKind::Script)),
            ))
            .unwrap();
        assert!(matches!(
            authored,
            skit_ui::Action::Add(AddAction::DraftEdited {
                result: Ok(Some(ref snapshot)),
                ..
            }) if snapshot.bytes == b"print('edited')\n"
        ));

        let external = sandbox.path().join("external.py");
        std::fs::write(&external, b"print('external')\n").unwrap();
        let review = skit_ui::ReviewState::from_source(
            tui_add_source(&roots.data, &external).unwrap(),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        let mut edit_source = AddWorkflowState::from_review(review);
        let edited = host
            .serve(Effect::Add(edit_source.reduce(AddAction::EditSource)))
            .unwrap();
        assert!(matches!(
            edited,
            skit_ui::Action::Add(AddAction::SourceEdited {
                result: Ok(ref snapshot),
                ..
            }) if snapshot.bytes == b"print('edited')\n"
        ));

        assert!(matches!(
            host.serve(Effect::Edit {
                selector: stored.slug.as_str().to_owned(),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        assert_eq!(
            std::fs::read(store.payload_path(&stored).unwrap()).unwrap(),
            b"print('edited')\n"
        );

        let calls = editor.calls.borrow();
        assert_eq!(calls.len(), 3);
        assert!(
            calls
                .iter()
                .all(|(argv, _)| argv == &["must-not-run", "--wait"])
        );
        assert!(calls[0].1.starts_with(roots.data.join("drafts")));
        assert_eq!(calls[1].1, std::fs::canonicalize(&external).unwrap());
        assert_eq!(calls[2].1, store.payload_path(&stored).unwrap());
        assert_eq!(
            output.0.borrow().as_slice(),
            [
                OutputEvent::Success(format_text(Locale::En, "Saved {}.", &[&"Stored"])),
                OutputEvent::Detail(format_text(
                    Locale::En,
                    "skit reconciles parameter drift at run time; review managed parameters with: skit params {}",
                    &[&"Stored"],
                )),
            ]
        );
    }

    #[test]
    fn reference_python_edit_routes_each_receipt_through_host_output() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let source = sandbox.path().join("reference.py");
        std::fs::write(&source, b"print('before')\n").unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = service
            .add(CreateEntry {
                name: "Referenced".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Reference,
                source: source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        FileConfigStore::new(&roots.config)
            .set("editor", "must-not-run")
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let editor = RecordingEditor::default();
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &editor,
                test_run_ports(&FIXED_PROBE),
                &output,
            ),
        )
        .unwrap();

        host.serve(Effect::Edit {
            selector: entry.slug.as_str().to_owned(),
        })
        .unwrap();

        assert_eq!(
            output.0.borrow().as_slice(),
            [
                OutputEvent::Detail(format_text(
                    Locale::En,
                    "Editing the original file (reference mode): {}",
                    &[&source.display()],
                )),
                OutputEvent::Success(format_text(Locale::En, "Saved {}.", &[&"Referenced"])),
                OutputEvent::Detail(format_text(
                    Locale::En,
                    "skit reconciles parameter drift at run time; review managed parameters with: skit params {}",
                    &[&"Referenced"],
                )),
            ]
        );
    }

    #[test]
    fn injected_terminal_capability_owns_the_reference_prompt_receipt() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let source = sandbox.path().join("reference.md");
        std::fs::write(&source, b"Review {{old}}\n").unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = service
            .add(CreateEntry {
                name: "Prompt".to_owned(),
                kind: EntryKind::parse("prompt").unwrap(),
                mode: StorageMode::Reference,
                source: source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings {
                    interpolate: true,
                    ..EntrySettings::default()
                },
            })
            .unwrap();
        FileConfigStore::new(&roots.config)
            .set("editor", "must-not-run")
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &INTERACTIVE_TERMINAL,
                &PromptEditor,
                test_run_ports(&FIXED_PROBE),
                &output,
            ),
        )
        .unwrap();

        host.serve(Effect::Edit {
            selector: entry.slug.as_str().to_owned(),
        })
        .unwrap();

        assert_eq!(
            output.0.borrow().as_slice(),
            [
                OutputEvent::Detail(format_text(
                    Locale::En,
                    "Editing the original file (reference mode): {}",
                    &[&source.display()],
                )),
                OutputEvent::Success(format_text(Locale::En, "Saved {}.", &[&"Prompt"])),
            ]
        );

        let noninteractive_output = RecordingOutput::default();
        let noninteractive_host = TuiHost::new(
            &service,
            roots,
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &PromptEditor,
                test_run_ports(&FIXED_PROBE),
                &noninteractive_output,
            ),
        )
        .unwrap();
        noninteractive_host
            .serve(Effect::Edit {
                selector: entry.slug.as_str().to_owned(),
            })
            .unwrap();
        assert_eq!(
            noninteractive_output.0.borrow().as_slice(),
            [
                OutputEvent::Detail(format_text(
                    Locale::En,
                    "Editing the original file (reference mode): {}",
                    &[&source.display()],
                )),
                OutputEvent::Success(format_text(Locale::En, "Saved {}.", &[&"Prompt"])),
                OutputEvent::Plain(format_text(
                    Locale::En,
                    "Detected but not yet managed: {} (use --add to manage them)",
                    &[&"fresh"],
                )),
            ]
        );
    }

    #[test]
    fn rename_submit_routes_its_receipt_through_host_output() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = add_command(&service, "Before");
        let clock = fixed_clock();
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots,
            Locale::En,
            &clock,
            test_platform(&NoRunSnapshotEnvironment, &output),
        )
        .unwrap();

        assert!(matches!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Rename,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([("name".to_owned(), FieldValue::text("After"))]),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        let renamed = service.show("After").unwrap();
        assert_eq!(
            output.0.borrow().as_slice(),
            [OutputEvent::Plain(format_text(
                Locale::En,
                "Renamed: {} ({})",
                &[&renamed.meta.name, &renamed.slug],
            ))]
        );
    }

    #[test]
    fn explicit_windows_host_policy_owns_add_inference_and_command_launch() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        std::fs::create_dir_all(roots.home.as_ref().unwrap()).unwrap();
        let source = roots.home.as_ref().unwrap().join("tool.walker");
        std::fs::write(&source, b"opaque executable payload\n").unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = add_command(&service, "Windows command");
        FileConfigStore::new(&roots.config)
            .set("after_run", "stay")
            .unwrap();
        let mut environment = poison_environment(sandbox.path());
        environment.platform = InterpreterPlatform::Windows;
        environment
            .variables
            .insert("PATHEXT".to_owned(), OsString::from(".WALKER"));
        environment.variables.insert(
            "COMSPEC".to_owned(),
            OsString::from("C:\\explicit\\cmd.exe"),
        );
        environment.variables.insert(
            "SystemRoot".to_owned(),
            OsString::from("C:\\poison-system-root"),
        );
        let launch = RecordingLaunchRunner::new([
            RecordedLaunchOutcome::Exit(0),
            RecordedLaunchOutcome::Exit(7),
        ]);
        let output = RecordingOutput::default();
        let clock = fixed_clock();
        let host = TuiHost::new(
            &service,
            roots,
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&FIXED_PROBE, &launch),
                &output,
            ),
        )
        .unwrap();

        let mut add = AddWorkflowState::new(Vec::new());
        let _ = add.reduce(AddAction::SetSourcePath("~/tool.walker".to_owned()));
        let action = host
            .serve(Effect::Add(add.reduce(AddAction::Continue)))
            .unwrap();
        let (request, snapshot) =
            inspected_source(action).expect("the explicit host must inspect the source");
        assert_eq!(snapshot.executable, Some(true));
        let _ = add.reduce(AddAction::SourceInspected {
            request,
            result: Ok(snapshot),
        });
        let committed = host
            .serve(Effect::Add(add.reduce(AddAction::Save)))
            .unwrap();
        assert!(matches!(
            committed,
            skit_ui::Action::Add(AddAction::CommitFinished { result: Ok(_), .. })
        ));
        assert_eq!(
            std::fs::read(&source).unwrap(),
            b"opaque executable payload\n"
        );

        let run_values = BTreeMap::from([
            ("_skit_args".to_owned(), FieldValue::text("")),
            ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
            ("_skit_dry_run".to_owned(), FieldValue::text("false")),
        ]);
        let action = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: run_values.clone(),
            })
            .unwrap();
        assert!(matches!(action, skit_ui::Action::Complete { .. }));
        let mut dry_values = run_values;
        dry_values.insert("_skit_dry_run".to_owned(), FieldValue::text("true"));
        assert!(matches!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: dry_values,
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        assert!(matches!(
            host.serve(Effect::Rerun {
                selector: entry.slug.as_str().to_owned(),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        let plans = launch.plans.borrow();
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].program, PathBuf::from("C:\\explicit\\cmd.exe"));
        assert_eq!(plans[0].args, ["/C", "printf ok"]);
        assert_eq!(plans[1], plans[0]);
    }

    #[test]
    fn explicit_home_resolution_keeps_the_host_platform_order() {
        assert_eq!(
            user_home_from_environment(
                InterpreterPlatform::Windows,
                Some(OsString::from("C:\\wrong-home")),
                Some(OsString::from("D:\\profile")),
                Some(OsString::from("E:")),
                Some(OsString::from("\\drive-profile")),
                Some(PathBuf::from("F:\\fallback")),
            ),
            Some(PathBuf::from("D:\\profile"))
        );
        assert_eq!(
            user_home_from_environment(
                InterpreterPlatform::Windows,
                Some(OsString::from("C:\\wrong-home")),
                None,
                Some(OsString::from("E:")),
                Some(OsString::from("\\drive-profile")),
                Some(PathBuf::from("F:\\fallback")),
            ),
            Some(PathBuf::from("E:\\drive-profile"))
        );
        assert_eq!(
            user_home_from_environment(
                InterpreterPlatform::Other,
                Some(OsString::from("/explicit-home")),
                Some(OsString::from("/wrong-profile")),
                None,
                None,
                Some(PathBuf::from("/fallback")),
            ),
            Some(PathBuf::from("/explicit-home"))
        );
        for (drive, path) in [
            (None, None),
            (Some(OsString::new()), Some(OsString::from("\\profile"))),
            (Some(OsString::from("E:")), Some(OsString::new())),
        ] {
            assert_eq!(
                user_home_from_environment(
                    InterpreterPlatform::Windows,
                    None,
                    Some(OsString::new()),
                    drive,
                    path,
                    Some(PathBuf::from("F:\\fallback")),
                ),
                Some(PathBuf::from("F:\\fallback"))
            );
        }
        assert_eq!(
            expand_user_path_with_home(
                Path::new("~\\literal"),
                Some(Path::new("/explicit-home")),
                InterpreterPlatform::Other,
            ),
            PathBuf::from("~\\literal")
        );
        assert_eq!(
            expand_user_path_with_home(
                Path::new("~\\child"),
                Some(Path::new("C:\\profile")),
                InterpreterPlatform::Windows,
            ),
            PathBuf::from("C:\\profile").join("child")
        );
    }

    #[cfg(unix)]
    #[test]
    fn command_submit_and_rerun_use_the_exact_plan_and_persisted_invocation() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = add_parameter_command(&service, "Command");
        FileConfigStore::new(&roots.config)
            .set_many(&BTreeMap::from([
                ("after_run".to_owned(), "stay".to_owned()),
                ("mirror.npm".to_owned(), "npmmirror".to_owned()),
            ]))
            .unwrap();
        let started_at = fixed_clock().0;
        let completed_at = started_at + time::Duration::minutes(9);
        let other = add_command(&service, "Other");
        FormStateService::new(FileFormStateStore::new(&roots.state))
            .record_run(
                &other.slug,
                0,
                "2026-08-28T12:33:56+00:00",
                &[],
                Some(&BTreeMap::new()),
            )
            .unwrap();
        let clock = MutableClock(Cell::new(started_at));
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([("sh".to_owned(), PathBuf::from("/runtime/sh"))]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([
            RecordedLaunchOutcome::Exit(7),
            RecordedLaunchOutcome::Exit(0),
        ]);
        let advancing_launch = AdvancingLaunchRunner {
            inner: &launch,
            clock: &clock,
            completed_at: Cell::new(completed_at),
        };
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &advancing_launch),
                &output,
            ),
        )
        .unwrap();
        let values = BTreeMap::from([
            (
                "value:value".to_owned(),
                FieldValue::text("~/{env:SENTINEL}/{today}/{now}/{cwd}"),
            ),
            (
                "_skit_args".to_owned(),
                FieldValue::text("--one 'two words'"),
            ),
            ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
            ("_skit_dry_run".to_owned(), FieldValue::text("false")),
        ]);

        let submitted = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values,
            })
            .unwrap();
        assert!(
            matches!(submitted, skit_ui::Action::Complete { .. }),
            "unexpected submit action: {submitted:?}"
        );

        let (surface, _, _) = refreshed_completion(submitted).unwrap();
        assert_eq!(
            surface.details[&other.slug].last_run.as_ref().unwrap().age,
            LibraryRunAge::Minutes(10)
        );

        let expected = LaunchPlan {
            program: PathBuf::from("/runtime/sh"),
            args: vec![
                "-c".to_owned(),
                format!(
                    "printf {}/explicit-host/2026-08-28/12-34-56/{} --one 'two words'",
                    roots.home.as_ref().unwrap().display(),
                    roots.cwd.display(),
                ),
            ],
            env: BTreeMap::new(),
            cwd: roots.cwd.clone(),
            display: format!(
                "/runtime/sh -c 'printf {}/explicit-host/2026-08-28/12-34-56/{} --one '\"'\"'two words'\"'\"''",
                roots.home.as_ref().unwrap().display(),
                roots.cwd.display(),
            ),
            warnings: Vec::new(),
        };
        assert_eq!(
            launch.plans.borrow().as_slice(),
            std::slice::from_ref(&expected)
        );
        assert_eq!(
            output.0.borrow().as_slice(),
            [OutputEvent::Stdout(format_text(
                Locale::En,
                "→ {}",
                &[&expected.display],
            ))]
        );
        let saved = FormStateService::new(FileFormStateStore::new(&roots.state)).load(&entry.slug);
        assert_eq!(
            saved.last_run,
            LastRunState {
                at: Some("2026-08-28T12:43:56+00:00".to_owned()),
                exit: Some(7),
                values: Some(BTreeMap::from([(
                    "value".to_owned(),
                    "~/{env:SENTINEL}/{today}/{now}/{cwd}".to_owned(),
                )])),
            }
        );
        assert_eq!(saved.extra_args, ["--one", "two words"]);

        output.0.borrow_mut().clear();
        advancing_launch
            .completed_at
            .set(completed_at + time::Duration::minutes(9));
        let rerun_action = host
            .serve(Effect::Rerun {
                selector: entry.slug.as_str().to_owned(),
            })
            .unwrap();
        let (surface, _, _) = refreshed_completion(rerun_action).unwrap();
        assert_eq!(
            surface.details[&other.slug].last_run.as_ref().unwrap().age,
            LibraryRunAge::Minutes(19)
        );
        let rerun_expected = LaunchPlan {
            args: vec![
                "-c".to_owned(),
                format!(
                    "printf {}/explicit-host/2026-08-28/12-43-56/{} --one 'two words'",
                    roots.home.as_ref().unwrap().display(),
                    roots.cwd.display(),
                ),
            ],
            display: format!(
                "/runtime/sh -c 'printf {}/explicit-host/2026-08-28/12-43-56/{} --one '\"'\"'two words'\"'\"''",
                roots.home.as_ref().unwrap().display(),
                roots.cwd.display(),
            ),
            ..expected.clone()
        };
        assert_eq!(
            launch.plans.borrow().as_slice(),
            [expected.clone(), rerun_expected.clone()]
        );
        assert_eq!(
            output.0.borrow().as_slice(),
            [
                OutputEvent::Diagnostic(format_text(
                    Locale::En,
                    "Reusing your last arguments: {}",
                    &[&"--one two words"],
                )),
                OutputEvent::Stdout(format_text(Locale::En, "→ {}", &[&rerun_expected.display],)),
            ]
        );
        let rerun = FormStateService::new(FileFormStateStore::new(&roots.state)).load(&entry.slug);
        assert_eq!(rerun.last_run.exit, Some(0));
        assert_eq!(
            rerun.last_run.at.as_deref(),
            Some("2026-08-28T12:52:56+00:00")
        );
        assert_eq!(rerun.last_run.values, saved.last_run.values);
        assert_eq!(rerun.extra_args, saved.extra_args);
    }

    #[test]
    fn prompt_submit_uses_the_selected_real_runner_and_persists_the_pick() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = add_prompt(&service, "Prompt");
        let config = FileConfigStore::new(&roots.config);
        config.set("after_run", "stay").unwrap();
        config
            .set_runner(
                skit_store::PromptRunner {
                    name: "agent".to_owned(),
                    argv: vec![
                        "agent".to_owned(),
                        "run".to_owned(),
                        "{{prompt}}".to_owned(),
                    ],
                },
                true,
            )
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([("agent".to_owned(), PathBuf::from("/runtime/agent"))]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([RecordedLaunchOutcome::Exit(0)]);
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &launch),
                &output,
            ),
        )
        .unwrap();

        let action = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([
                    ("value:gone".to_owned(), FieldValue::text("kept")),
                    ("_skit_args".to_owned(), FieldValue::text("")),
                    ("_skit_runner".to_owned(), FieldValue::text("agent")),
                    ("_skit_runner_picked".to_owned(), FieldValue::text("true")),
                    ("_skit_dry_run".to_owned(), FieldValue::text("false")),
                ]),
            })
            .unwrap();
        assert!(matches!(action, skit_ui::Action::Complete { .. }));
        let expected = LaunchPlan {
            program: PathBuf::from("/runtime/agent"),
            args: vec!["run".to_owned(), "Review this".to_owned()],
            env: BTreeMap::new(),
            cwd: roots.cwd.clone(),
            display: "/runtime/agent run '<rendered prompt omitted; use --dry-run to inspect it>'"
                .to_owned(),
            warnings: Vec::new(),
        };
        assert_eq!(
            launch.plans.borrow().as_slice(),
            std::slice::from_ref(&expected)
        );
        assert_eq!(
            output.0.borrow().as_slice(),
            [OutputEvent::Stdout(format_text(
                Locale::En,
                "→ {}",
                &[&expected.display],
            ))]
        );
        assert_eq!(
            PromptSelectionService::new(FilePromptSelectionStore::new(&roots.state)).last_runner(),
            "agent"
        );
        let saved = FormStateService::new(FileFormStateStore::new(&roots.state)).load(&entry.slug);
        assert_eq!(saved.last_run.exit, Some(0));
        assert_eq!(
            saved.last_run.values,
            Some(BTreeMap::from([("gone".to_owned(), "kept".to_owned(),)]))
        );
        assert_eq!(
            saved.last_run.at.as_deref(),
            Some("2026-08-28T12:34:56+00:00")
        );
    }

    #[test]
    fn python_copy_and_reference_use_real_preparation_with_an_injected_child() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let copied = add_python(&service, "Copied");
        let stale_snapshot = store
            .entry_dir_path(&copied.slug)
            .join(".run-stale-copy.py");
        std::fs::write(&stale_snapshot, b"stale launch bytes").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&stale_snapshot)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::UNIX_EPOCH)
                    .set_accessed(SystemTime::UNIX_EPOCH),
            )
            .unwrap();
        let reference_source = sandbox.path().join("reference.py");
        std::fs::write(&reference_source, b"print('reference')\n").unwrap();
        let referenced = service
            .add(CreateEntry {
                name: "Referenced run".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Reference,
                source: reference_source.display().to_string(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        FileConfigStore::new(&roots.config)
            .set("after_run", "stay")
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/runtime/uv"))]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([
            RecordedLaunchOutcome::Exit(0),
            RecordedLaunchOutcome::Exit(7),
        ]);
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &launch),
                &output,
            ),
        )
        .unwrap();
        let values = || {
            BTreeMap::from([
                ("_skit_args".to_owned(), FieldValue::text("")),
                ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ])
        };

        for entry in [&copied, &referenced] {
            assert!(matches!(
                host.serve(Effect::Submit {
                    purpose: FormPurpose::Run,
                    selector: Some(entry.slug.as_str().to_owned()),
                    values: values(),
                })
                .unwrap(),
                skit_ui::Action::Complete { .. }
            ));
        }

        let plans = launch.plans.borrow();
        assert_eq!(plans.len(), 2);
        for plan in plans.iter() {
            assert_eq!(plan.program, PathBuf::from("/runtime/uv"));
            assert_eq!(&plan.args[..3], ["run", "--no-project", "--script"]);
            assert_eq!(plan.env, BTreeMap::new());
            assert_eq!(plan.cwd, roots.cwd);
            assert!(plan.warnings.is_empty());
            assert_eq!(
                shlex::split(&plan.display).unwrap(),
                std::iter::once("/runtime/uv".to_owned())
                    .chain(plan.args.iter().cloned())
                    .collect::<Vec<_>>()
            );
        }
        let copied_snapshot = PathBuf::from(&plans[0].args[3]);
        assert!(copied_snapshot.starts_with(store.entry_dir_path(&copied.slug)));
        assert_ne!(copied_snapshot, store.payload_path(&copied).unwrap());
        assert!(!copied_snapshot.exists());
        assert!(!stale_snapshot.exists());
        assert_eq!(PathBuf::from(&plans[1].args[3]), reference_source);
        assert_eq!(
            launch.source_files.borrow().as_slice(),
            [
                (copied_snapshot, b"print('before')\n".to_vec()),
                (reference_source.clone(), b"print('reference')\n".to_vec()),
            ]
        );
        assert_eq!(
            std::fs::read(&reference_source).unwrap(),
            b"print('reference')\n"
        );
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state))
                .last_run(&copied.slug)
                .exit,
            Some(0)
        );
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state))
                .last_run(&referenced.slug)
                .exit,
            Some(7)
        );
    }

    #[cfg(unix)]
    #[test]
    fn dry_run_persists_only_its_preset_and_never_calls_the_child() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let entry = add_parameter_command(&service, "Preview");
        FileConfigStore::new(&roots.config)
            .set("after_run", "stay")
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([("sh".to_owned(), PathBuf::from("/runtime/sh"))]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([]);
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &launch),
                &output,
            ),
        )
        .unwrap();

        assert!(matches!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([
                    ("value:value".to_owned(), FieldValue::text("preview")),
                    ("_skit_args".to_owned(), FieldValue::text("")),
                    ("_skit_runner_picked".to_owned(), FieldValue::text("false"),),
                    ("_skit_dry_run".to_owned(), FieldValue::text("true")),
                    ("_skit_save_preset".to_owned(), FieldValue::text("review"),),
                ]),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        assert!(launch.plans.borrow().is_empty());
        let saved = FormStateService::new(FileFormStateStore::new(&roots.state)).load(&entry.slug);
        assert_eq!(saved.last_run, LastRunState::default());
        assert!(saved.values.is_empty());
        assert_eq!(
            saved.presets,
            BTreeMap::from([(
                "review".to_owned(),
                BTreeMap::from([("value".to_owned(), "preview".to_owned())]),
            )])
        );
        assert_eq!(output.0.borrow().len(), 1);
        assert!(matches!(output.0.borrow()[0], OutputEvent::Stdout(_)));
    }

    #[cfg(unix)]
    #[test]
    fn launch_io_failure_keeps_completed_run_state_empty() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let entry = add_python(&service, "Failure");
        let entry_dir = store.entry_dir_path(&entry.slug);
        let source_path = store.payload_path(&entry).unwrap();
        let source_before = std::fs::read(&source_path).unwrap();
        let meta_path = entry_dir.join("meta.toml");
        let meta_before = std::fs::read(&meta_path).unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/runtime/uv"))]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([RecordedLaunchOutcome::Failure(
            io::ErrorKind::PermissionDenied,
            "spawn denied",
        )]);
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &launch),
                &output,
            ),
        )
        .unwrap();

        let error = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([
                    ("_skit_args".to_owned(), FieldValue::text("")),
                    ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                    ("_skit_dry_run".to_owned(), FieldValue::text("false")),
                ]),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            super::super::CliError::Run(crate::run::RunError::Launch(LaunchError::Process {
                operation: "run",
                ref source,
            })) if source.kind() == io::ErrorKind::PermissionDenied
                && source.to_string() == "spawn denied"
        ));
        assert_eq!(launch.plans.borrow().len(), 1);
        assert_eq!(launch.source_files.borrow().len(), 1);
        let failed_snapshot = launch.source_files.borrow()[0].0.clone();
        assert!(!failed_snapshot.exists());
        assert!(std::fs::read_dir(&entry_dir).unwrap().all(|item| {
            !item
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".run-")
        }));
        assert_eq!(std::fs::read(source_path).unwrap(), source_before);
        assert_eq!(std::fs::read(meta_path).unwrap(), meta_before);
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state)).last_run(&entry.slug),
            LastRunState::default()
        );
        assert_eq!(output.0.borrow().len(), 1);
        assert!(matches!(output.0.borrow()[0], OutputEvent::Stdout(_)));
    }

    #[cfg(unix)]
    #[test]
    fn completed_state_write_failure_keeps_old_bytes_and_cleans_the_copy_snapshot() {
        use std::os::unix::fs::PermissionsExt as _;

        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let entry = add_python(&service, "State failure");
        let state = FormStateService::new(FileFormStateStore::new(&roots.state));
        state
            .record_run(&entry.slug, 3, "2026-08-01T00:00:00+00:00", &[], None)
            .unwrap();
        let saved_before = state.load(&entry.slug);
        let values_dir = roots.state.join("values");
        let values_path = values_dir.join(format!("{}.toml", entry.slug.as_str()));
        let state_bytes_before = std::fs::read(&values_path).unwrap();
        let original_permissions = std::fs::metadata(&values_dir).unwrap().permissions();
        let _restore = PermissionRestore {
            path: values_dir.clone(),
            permissions: original_permissions.clone(),
        };
        let mut read_only = original_permissions;
        read_only.set_mode(0o500);
        std::fs::set_permissions(&values_dir, read_only).unwrap();

        let entry_dir = store.entry_dir_path(&entry.slug);
        let source_path = store.payload_path(&entry).unwrap();
        let source_before = std::fs::read(&source_path).unwrap();
        let meta_path = entry_dir.join("meta.toml");
        let meta_before = std::fs::read(&meta_path).unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([("uv".to_owned(), PathBuf::from("/runtime/uv"))]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([RecordedLaunchOutcome::Exit(0)]);
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &launch),
                &output,
            ),
        )
        .unwrap();

        let error = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([
                    ("_skit_args".to_owned(), FieldValue::text("")),
                    ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                    ("_skit_dry_run".to_owned(), FieldValue::text("false")),
                ]),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            super::super::CliError::Run(crate::run::RunError::State(_))
        ));
        assert_eq!(launch.plans.borrow().len(), 1);
        assert_eq!(launch.source_files.borrow().len(), 1);
        let completed_snapshot = launch.source_files.borrow()[0].0.clone();
        assert!(!completed_snapshot.exists());
        assert!(std::fs::read_dir(&entry_dir).unwrap().all(|item| {
            !item
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".run-")
        }));
        assert_eq!(std::fs::read(source_path).unwrap(), source_before);
        assert_eq!(std::fs::read(meta_path).unwrap(), meta_before);
        assert_eq!(std::fs::read(values_path).unwrap(), state_bytes_before);
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state)).load(&entry.slug),
            saved_before
        );
    }

    #[test]
    fn prompt_runner_refusals_preserve_pick_state_without_a_completed_run() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let no_runner = add_prompt(&service, "No runner");
        let missing_program = add_prompt(&service, "Missing program");
        let config = FileConfigStore::new(&roots.config);
        config.ensure_runners_seeded().unwrap();
        for runner in config.runners().unwrap() {
            assert!(config.remove_runner(&runner.name).unwrap());
        }
        assert!(config.runners().unwrap().is_empty());
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe::default();
        let launch = RecordingLaunchRunner::new([]);
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                run_ports(&probe, &launch),
                &output,
            ),
        )
        .unwrap();
        let base_values = |runner: Option<&str>| {
            let mut values = BTreeMap::from([
                ("value:gone".to_owned(), FieldValue::text("kept")),
                ("_skit_args".to_owned(), FieldValue::text("")),
                (
                    "_skit_runner_picked".to_owned(),
                    FieldValue::text(if runner.is_some() { "true" } else { "false" }),
                ),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ]);
            if let Some(runner) = runner {
                values.insert("_skit_runner".to_owned(), FieldValue::text(runner));
            }
            values
        };

        let no_runner_error = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(no_runner.slug.as_str().to_owned()),
                values: base_values(None),
            })
            .unwrap_err();
        assert!(
            no_runners_configured(&no_runner_error),
            "unexpected no-runner error: {no_runner_error:?}"
        );
        config
            .set_runner(
                skit_store::PromptRunner {
                    name: "missing".to_owned(),
                    argv: vec!["missing".to_owned(), "{{prompt}}".to_owned()],
                },
                true,
            )
            .unwrap();
        let required_error = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(no_runner.slug.as_str().to_owned()),
                values: base_values(None),
            })
            .unwrap_err();
        assert!(matches!(
            required_error,
            super::super::CliError::Run(crate::run::RunError::RunnerRequired { .. })
        ));
        let unknown_error = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(missing_program.slug.as_str().to_owned()),
                values: base_values(Some("unknown")),
            })
            .unwrap_err();
        assert!(matches!(
            unknown_error,
            super::super::CliError::Run(crate::run::RunError::RunnerNotFound {
                ref name,
                ref known,
            }) if name == "unknown" && known == "missing"
        ));
        let action = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(missing_program.slug.as_str().to_owned()),
                values: base_values(Some("missing")),
            })
            .unwrap();
        let message = completion_message(action)
            .expect("a selected missing runner must return a completion error status");
        assert_eq!(
            message,
            format_text(
                Locale::En,
                "Error: {}",
                &[&"required program was not found: missing"],
            )
        );
        assert_eq!(
            PromptSelectionService::new(FilePromptSelectionStore::new(&roots.state)).last_runner(),
            "missing"
        );
        assert!(launch.plans.borrow().is_empty());
        for entry in [&no_runner, &missing_program] {
            assert_eq!(
                FormStateService::new(FileFormStateStore::new(&roots.state)).last_run(&entry.slug),
                LastRunState::default()
            );
        }
    }

    #[test]
    fn missing_uv_uses_the_injected_no_network_fetcher_and_cleans_the_launch_snapshot() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let entry = add_python(&service, "Python offline");
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe::default();
        let launch = RecordingLaunchRunner::new([]);
        let fetcher = RejectingUvFetcher::default();
        let consent = skit_runtime::AllowUvDownload;
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                RunPorts::new(
                    &probe,
                    &launch,
                    &NOOP_DEPENDENCIES,
                    &NOOP_INJECTED,
                    &NOOP_JAVASCRIPT_GATE,
                    &consent,
                    &fetcher,
                ),
                &output,
            ),
        )
        .unwrap();

        let error = host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([
                    ("_skit_args".to_owned(), FieldValue::text("")),
                    ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                    ("_skit_dry_run".to_owned(), FieldValue::text("false")),
                ]),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            super::super::CliError::Run(crate::run::RunError::Uv(
                skit_runtime::UvBootstrapError::Download { ref url, ref reason }
            )) if url == &fetcher.0.borrow()[0].0
                && reason == "network disabled by walker"
        ));
        assert_eq!(fetcher.0.borrow().len(), 1);
        assert!(fetcher.0.borrow()[0].0.starts_with("https://"));
        assert!(fetcher.0.borrow()[0].1 > 0);
        assert!(launch.plans.borrow().is_empty());
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state)).last_run(&entry.slug),
            LastRunState::default()
        );
        assert!(!skit_runtime::managed_uv_path(&roots.data).exists());
        assert!(
            std::fs::read_dir(store.entry_dir_path(&entry.slug))
                .unwrap()
                .all(|item| !item
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".run-"))
        );
        assert_eq!(
            output.0.borrow().as_slice(),
            [OutputEvent::Diagnostic(format_text(
                Locale::En,
                "First run — downloading uv {}…",
                &[&skit_runtime::UV_VERSION],
            ))]
        );
    }

    #[test]
    fn javascript_dependency_runner_uses_the_real_transaction_and_restores_on_failure() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let success_entry = add_javascript(&service, "JavaScript success");
        let failed_entry = add_javascript(&service, "JavaScript failure");
        let failed_dir = store.entry_dir_path(&failed_entry.slug);
        std::fs::write(failed_dir.join("package.json"), b"old manifest\n").unwrap();
        std::fs::create_dir_all(failed_dir.join("node_modules")).unwrap();
        std::fs::write(failed_dir.join("node_modules/old.txt"), b"old tree\n").unwrap();
        let failed_tree_before = tree_snapshot(&failed_dir);
        FileConfigStore::new(&roots.config)
            .set("after_run", "stay")
            .unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([
                ("node".to_owned(), PathBuf::from("/runtime/node")),
                ("/runtime/node".to_owned(), PathBuf::from("/runtime/node")),
                ("npm".to_owned(), PathBuf::from("/runtime/npm")),
            ]),
            ..RecordingProbe::default()
        };
        let successful_dependencies = RecordingDependencyRunner::new(true);
        let launch = RecordingLaunchRunner::new([RecordedLaunchOutcome::Exit(0)]);
        let output = RecordingOutput::default();
        let success_host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                RunPorts::new(
                    &probe,
                    &launch,
                    &successful_dependencies,
                    &NOOP_INJECTED,
                    &NOOP_JAVASCRIPT_GATE,
                    &NOOP_UV_CONSENT,
                    &NOOP_UV_FETCHER,
                ),
                &output,
            ),
        )
        .unwrap();
        let values = || {
            BTreeMap::from([
                ("_skit_args".to_owned(), FieldValue::text("")),
                ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ])
        };

        assert!(matches!(
            success_host
                .serve(Effect::Submit {
                    purpose: FormPurpose::Run,
                    selector: Some(success_entry.slug.as_str().to_owned()),
                    values: values(),
                })
                .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        let success_commands = successful_dependencies.commands.borrow();
        assert_eq!(
            success_commands.as_slice(),
            [DependencyCommand {
                program: PathBuf::from("/runtime/npm"),
                args: vec![
                    "install".to_owned(),
                    "--no-audit".to_owned(),
                    "--no-fund".to_owned(),
                    "--ignore-scripts".to_owned(),
                ],
                cwd: store.entry_dir_path(&success_entry.slug),
                environment: BTreeMap::new(),
            }]
        );
        let success_display = launch.plans.borrow()[0].display.clone();
        assert_eq!(
            output.0.borrow().as_slice(),
            [
                OutputEvent::Diagnostic(
                    skit_runtime::javascript_dependency_install_announcement("npm")
                        .localize(Locale::En),
                ),
                OutputEvent::Stdout(format_text(Locale::En, "→ {}", &[&success_display],)),
            ]
        );
        assert_eq!(launch.plans.borrow().len(), 1);
        assert_eq!(
            launch.plans.borrow()[0].program,
            PathBuf::from("/runtime/node")
        );
        assert!(
            store
                .entry_dir_path(&success_entry.slug)
                .join("node_modules")
                .is_dir()
        );

        let failed_dependencies = RecordingDependencyRunner::new(false);
        let failed_launch = RecordingLaunchRunner::new([]);
        let failed_output = RecordingOutput::default();
        let failed_host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                RunPorts::new(
                    &probe,
                    &failed_launch,
                    &failed_dependencies,
                    &NOOP_INJECTED,
                    &NOOP_JAVASCRIPT_GATE,
                    &NOOP_UV_CONSENT,
                    &NOOP_UV_FETCHER,
                ),
                &failed_output,
            ),
        )
        .unwrap();
        let error = failed_host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(failed_entry.slug.as_str().to_owned()),
                values: values(),
            })
            .unwrap_err();
        assert!(matches!(
            error,
            super::super::CliError::Run(crate::run::RunError::Dependencies(
                skit_runtime::DependencyError::InstallFailed { .. }
            ))
        ));
        assert_eq!(
            failed_output.0.borrow().as_slice(),
            [OutputEvent::Diagnostic(
                skit_runtime::javascript_dependency_install_announcement("npm")
                    .localize(Locale::En),
            )]
        );
        assert!(failed_launch.plans.borrow().is_empty());
        assert_eq!(
            std::fs::read(failed_dir.join("package.json")).unwrap(),
            b"old manifest\n"
        );
        assert_eq!(
            std::fs::read(failed_dir.join("node_modules/old.txt")).unwrap(),
            b"old tree\n"
        );
        assert_eq!(tree_snapshot(&failed_dir), failed_tree_before);
        assert!(!failed_dir.join(".skit-deps.backup").exists());
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state))
                .last_run(&failed_entry.slug),
            LastRunState::default()
        );
    }

    #[test]
    fn settings_dependency_cleanup_uses_the_host_clock() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let entry = add_javascript(&service, "Settings cleanup clock");
        let entry_dir = store.entry_dir_path(&entry.slug);
        std::fs::create_dir_all(entry_dir.join("node_modules")).unwrap();
        std::fs::write(
            entry_dir.join("node_modules/left-pad.js"),
            b"module.exports = {};\n",
        )
        .unwrap();
        std::fs::write(
            entry_dir.join("package.json"),
            b"{\"dependencies\":{\"left-pad\":\"*\"}}\n",
        )
        .unwrap();
        let (future_injected, future_launch) = seed_future_cleanup_sentinels(&entry_dir);
        let clock = cleanup_test_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            test_platform(&environment, &output),
        )
        .unwrap();

        assert!(matches!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Settings,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([("dependencies".to_owned(), FieldValue::text(""),)]),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));
        assert!(!entry_dir.join("node_modules").exists());
        assert!(!entry_dir.join("package.json").exists());
        assert!(future_injected.exists());
        assert!(future_launch.exists());
        assert!(
            EntrySettings::from_meta(&service.show(entry.slug.as_str()).unwrap().meta)
                .dependencies
                .is_empty()
        );
    }

    #[test]
    fn successful_shell_injection_reaches_the_exact_plan_and_cleans_owned_sources() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let entry = add_injected_script(
            &service,
            "shell",
            "Injected shell success",
            "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
            "script.sh",
        );
        let entry_dir = store.entry_dir_path(&entry.slug);
        let source_path = store.payload_path(&entry).unwrap();
        let source_before = std::fs::read(&source_path).unwrap();
        let meta_path = entry_dir.join("meta.toml");
        let meta_before = std::fs::read(&meta_path).unwrap();
        let (future_injected, future_launch) = seed_future_cleanup_sentinels(&entry_dir);
        FileConfigStore::new(&roots.config)
            .set("after_run", "stay")
            .unwrap();
        let probe = RecordingProbe {
            programs: BTreeMap::from([("bash".to_owned(), PathBuf::from("/runtime/bash"))]),
            ..RecordingProbe::default()
        };
        let gate = RecordingInjectedRunner {
            calls: RefCell::new(Vec::new()),
            success: true,
        };
        let launch = RecordingLaunchRunner::new([RecordedLaunchOutcome::Exit(0)]);
        let clock = cleanup_test_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                RunPorts::new(
                    &probe,
                    &launch,
                    &NOOP_DEPENDENCIES,
                    &gate,
                    &NOOP_JAVASCRIPT_GATE,
                    &NOOP_UV_CONSENT,
                    &NOOP_UV_FETCHER,
                ),
                &output,
            ),
        )
        .unwrap();

        assert!(matches!(
            host.serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(entry.slug.as_str().to_owned()),
                values: BTreeMap::from([
                    ("value:TOKEN".to_owned(), FieldValue::text("after")),
                    ("_skit_args".to_owned(), FieldValue::text("")),
                    ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                    ("_skit_dry_run".to_owned(), FieldValue::text("false")),
                ]),
            })
            .unwrap(),
            skit_ui::Action::Complete { .. }
        ));

        let calls = gate.calls.borrow();
        assert_eq!(calls.len(), 1);
        let (gate_command, gate_bytes) = &calls[0];
        assert_eq!(gate_command.program, PathBuf::from("/runtime/bash"));
        assert_eq!(
            gate_command.timeout,
            skit_runtime::SHELL_SYNTAX_GATE_TIMEOUT
        );
        assert_eq!(gate_command.args[0], "-n");
        let staged_path = PathBuf::from(&gate_command.args[1]);
        assert!(String::from_utf8_lossy(gate_bytes).contains("after"));
        assert_ne!(gate_bytes, &source_before);
        {
            let plans = launch.plans.borrow();
            assert_eq!(plans.len(), 1);
            let plan = &plans[0];
            assert_eq!(plan.program, PathBuf::from("/runtime/bash"));
            assert_eq!(plan.args, [staged_path.display().to_string()]);
            assert!(plan.env.is_empty());
            assert_eq!(plan.cwd, roots.cwd);
            assert!(plan.warnings.is_empty());
            assert_eq!(
                shlex::split(&plan.display).unwrap(),
                [
                    "/runtime/bash".to_owned(),
                    staged_path.display().to_string()
                ]
            );
        }
        assert_eq!(
            launch.source_files.borrow().as_slice(),
            [(staged_path.clone(), gate_bytes.clone())]
        );
        assert!(!staged_path.exists());
        assert!(future_injected.exists());
        assert!(future_launch.exists());
        let remaining_launch_snapshots = std::fs::read_dir(&entry_dir)
            .unwrap()
            .map(Result::unwrap)
            .map(|item| item.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".run-"))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            remaining_launch_snapshots,
            std::slice::from_ref(&future_launch)
        );
        assert_eq!(std::fs::read(source_path).unwrap(), source_before);
        assert_eq!(std::fs::read(meta_path).unwrap(), meta_before);
        assert_eq!(
            FormStateService::new(FileFormStateStore::new(&roots.state))
                .last_run(&entry.slug)
                .exit,
            Some(0)
        );
    }

    #[test]
    fn javascript_and_shell_injected_syntax_rejections_clean_staged_sources() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store.clone());
        let javascript = add_injected_script(
            &service,
            "js",
            "Injected JavaScript",
            "const TOKEN = \"before\";\nconsole.log(TOKEN);\n",
            "script.js",
        );
        let shell = add_injected_script(
            &service,
            "shell",
            "Injected shell",
            "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
            "script.sh",
        );
        let javascript_before = std::fs::read(store.payload_path(&javascript).unwrap()).unwrap();
        let shell_before = std::fs::read(store.payload_path(&shell).unwrap()).unwrap();
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe {
            programs: BTreeMap::from([
                ("node".to_owned(), PathBuf::from("/runtime/node")),
                ("/runtime/node".to_owned(), PathBuf::from("/runtime/node")),
                ("bash".to_owned(), PathBuf::from("/runtime/bash")),
            ]),
            ..RecordingProbe::default()
        };
        let launch = RecordingLaunchRunner::new([]);
        let javascript_gate = RecordingJavaScriptGate {
            calls: RefCell::new(Vec::new()),
            success: false,
        };
        let shell_gate = RecordingInjectedRunner {
            calls: RefCell::new(Vec::new()),
            success: false,
        };
        let output = RecordingOutput::default();
        let values = || {
            BTreeMap::from([
                ("value:TOKEN".to_owned(), FieldValue::text("after")),
                ("_skit_args".to_owned(), FieldValue::text("")),
                ("_skit_runner_picked".to_owned(), FieldValue::text("false")),
                ("_skit_dry_run".to_owned(), FieldValue::text("false")),
            ])
        };

        let javascript_host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                RunPorts::new(
                    &probe,
                    &launch,
                    &NOOP_DEPENDENCIES,
                    &shell_gate,
                    &javascript_gate,
                    &NOOP_UV_CONSENT,
                    &NOOP_UV_FETCHER,
                ),
                &output,
            ),
        )
        .unwrap();
        assert!(
            javascript_host
                .serve(Effect::Submit {
                    purpose: FormPurpose::Run,
                    selector: Some(javascript.slug.as_str().to_owned()),
                    values: values(),
                })
                .is_err()
        );
        assert_eq!(javascript_gate.calls.borrow().len(), 1);
        let javascript_staged = javascript_gate.calls.borrow()[0].1.clone();
        assert_eq!(
            javascript_gate.calls.borrow()[0].0,
            PathBuf::from("/runtime/node")
        );
        assert!(String::from_utf8_lossy(&javascript_gate.calls.borrow()[0].3).contains("after"));
        assert!(!javascript_staged.exists());

        let shell_host = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            TuiPlatform::test(
                &environment,
                &NONINTERACTIVE_TERMINAL,
                &NOOP_EDITOR,
                RunPorts::new(
                    &probe,
                    &launch,
                    &NOOP_DEPENDENCIES,
                    &shell_gate,
                    &javascript_gate,
                    &NOOP_UV_CONSENT,
                    &NOOP_UV_FETCHER,
                ),
                &output,
            ),
        )
        .unwrap();
        let shell_error = shell_host
            .serve(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some(shell.slug.as_str().to_owned()),
                values: values(),
            })
            .unwrap_err();
        assert_eq!(
            shell_gate.calls.borrow().len(),
            1,
            "unexpected shell refusal: {shell_error:?}"
        );
        let (command, bytes) = &shell_gate.calls.borrow()[0];
        assert_eq!(command.program, PathBuf::from("/runtime/bash"));
        assert_eq!(command.args[0], "-n");
        let shell_staged = PathBuf::from(&command.args[1]);
        assert!(String::from_utf8_lossy(bytes).contains("after"));
        assert!(!shell_staged.exists());

        assert!(launch.plans.borrow().is_empty());
        assert_eq!(
            std::fs::read(store.payload_path(&javascript).unwrap()).unwrap(),
            javascript_before
        );
        assert_eq!(
            std::fs::read(store.payload_path(&shell).unwrap()).unwrap(),
            shell_before
        );
        for entry in [&javascript, &shell] {
            assert_eq!(
                FormStateService::new(FileFormStateStore::new(&roots.state)).last_run(&entry.slug),
                LastRunState::default()
            );
        }
    }

    #[test]
    fn injected_probe_owns_health_and_javascript_preflight() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        std::fs::create_dir_all(&roots.cwd).unwrap();
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let command = add_command_with_need(&service, "Needs tool", "deterministic-tool");
        let javascript = add_javascript(&service, "JavaScript");
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let probe = RecordingProbe::default();
        let output = RecordingOutput::default();
        let platform = TuiPlatform::test(
            &environment,
            &NONINTERACTIVE_TERMINAL,
            &NOOP_EDITOR,
            test_run_ports(&probe),
            &output,
        );
        let host = TuiHost::new(&service, roots, Locale::En, &clock, platform).unwrap();

        let health = presented_health(
            host.serve(Effect::Open {
                request: HostRequest::Health,
                selector: None,
            })
            .unwrap(),
        )
        .expect("health must use the injected probe");
        assert!(health.snapshot().issues.iter().any(|issue| {
            issue.slug == command.slug.as_str()
                && matches!(
                    &issue.kind,
                    HealthIssueKind::MissingNeeds { tools }
                        if tools == &["deterministic-tool"]
                )
        }));

        let preflight = host.preflight(&Effect::Open {
            request: HostRequest::Run,
            selector: Some(javascript.slug.as_str().to_owned()),
        });
        assert!(preflight.is_err(), "the injected probe has no node program");
        let calls = probe.calls.borrow();
        assert!(calls.iter().any(|call| call == "find:deterministic-tool"));
        assert!(calls.iter().any(|call| call == "find:node"));
    }

    #[test]
    fn a_host_rejects_a_library_service_for_another_data_root() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let service = LibraryService::new(FileStore::new(sandbox.path().join("other-data")));
        let clock = fixed_clock();
        let environment = poison_environment(sandbox.path());
        let output = RecordingOutput::default();

        let error = TuiHost::new(
            &service,
            roots.clone(),
            Locale::En,
            &clock,
            test_platform(&environment, &output),
        )
        .expect_err("a mismatched data root must fail in every build profile");
        assert_eq!(error.expected, roots.data);
        assert_eq!(error.actual, service.repository().data_dir());
    }

    #[test]
    fn the_system_adapters_report_the_process_locale_and_the_stdout_terminal_state() {
        assert_eq!(SystemEnvironment.system_locale(), system_locale());
        assert_eq!(
            SystemTerminalCapability.stdout_is_terminal(),
            io::stdout().is_terminal()
        );
    }

    fn refusing_launch_plan() -> LaunchPlan {
        LaunchPlan {
            program: PathBuf::from("/runtime/never"),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: PathBuf::from("/"),
            display: "never".to_owned(),
            warnings: Vec::new(),
        }
    }

    #[test]
    #[should_panic(expected = "this contract must not launch an editor")]
    fn the_shared_editor_port_refuses_every_call() {
        let _ = NOOP_EDITOR.launch(&["editor".to_owned()], Path::new("draft.py"));
    }

    #[test]
    #[should_panic(expected = "this contract must not launch a child process")]
    fn the_shared_launch_port_refuses_every_call() {
        let _ = NOOP_LAUNCH.run(&refusing_launch_plan());
    }

    #[test]
    #[should_panic(expected = "this contract must not run a dependency command")]
    fn the_shared_dependency_port_refuses_every_call() {
        let _ = NOOP_DEPENDENCIES.run(&DependencyCommand {
            program: PathBuf::from("/runtime/never"),
            args: Vec::new(),
            cwd: PathBuf::from("/"),
            environment: BTreeMap::new(),
        });
    }

    #[test]
    #[should_panic(expected = "this contract must not run an injected-source command")]
    fn the_shared_injected_source_port_refuses_every_call() {
        let _ = NOOP_INJECTED.run(&InjectedCommand {
            program: PathBuf::from("/runtime/never"),
            args: Vec::new(),
            timeout: Duration::from_secs(1),
        });
    }

    #[test]
    #[should_panic(expected = "this contract must not run a JavaScript syntax gate")]
    fn the_shared_javascript_gate_port_refuses_every_call() {
        let _ = NOOP_JAVASCRIPT_GATE.check(
            Path::new("/runtime/never"),
            Path::new("/runtime/never.js"),
            Duration::from_secs(1),
        );
    }

    #[test]
    #[should_panic(expected = "this contract must not request a uv download")]
    fn the_shared_uv_consent_port_refuses_every_call() {
        let _ = NOOP_UV_CONSENT.allow_download("0.0.0", Path::new("/runtime/uv"));
    }

    #[test]
    #[should_panic(expected = "this contract must not fetch a uv archive")]
    fn the_shared_uv_fetcher_port_refuses_every_call() {
        let _ = NOOP_UV_FETCHER.fetch("https://example.invalid/uv.tar.gz", 1);
    }

    #[test]
    fn the_shared_probe_finds_no_program_and_answers_from_the_filesystem() {
        let sandbox = TempDir::new().unwrap();
        let source = sandbox.path().join("present.txt");
        std::fs::write(&source, b"present\n").unwrap();

        assert_eq!(FIXED_PROBE.find_program("node"), None);
        assert!(FIXED_PROBE.is_file(&source));
        assert!(!FIXED_PROBE.is_file(sandbox.path()));
        assert!(!FIXED_PROBE.is_executable(&source));
    }

    #[test]
    fn the_no_snapshot_environment_answers_with_fixed_values() {
        assert_eq!(NoRunSnapshotEnvironment.variable("PATH"), None);
        assert_eq!(NoRunSnapshotEnvironment.local_offset(), UtcOffset::UTC);
        assert_eq!(NoRunSnapshotEnvironment.system_locale(), Locale::En);
    }

    #[test]
    #[should_panic(expected = "a non-run submit must not read the run environment snapshot")]
    fn the_no_snapshot_environment_refuses_the_run_environment() {
        let _ = NoRunSnapshotEnvironment.variables();
    }

    #[test]
    fn the_recording_javascript_gate_reports_an_accepted_syntax_check() {
        let sandbox = TempDir::new().unwrap();
        let source = sandbox.path().join("accepted.js");
        std::fs::write(&source, b"const value = 1;\n").unwrap();
        let gate = RecordingJavaScriptGate {
            calls: RefCell::new(Vec::new()),
            success: true,
        };

        let output = gate
            .check(Path::new("/runtime/node"), &source, Duration::from_secs(5))
            .unwrap();

        assert!(output.success);
        assert!(output.stderr.is_empty());
        assert_eq!(gate.calls.borrow().len(), 1);
        assert_eq!(gate.calls.borrow()[0].3, b"const value = 1;\n");
    }

    #[test]
    fn an_automatic_language_follows_the_system_locale_without_a_locale_variable() {
        let sandbox = TempDir::new().unwrap();
        let roots = roots(sandbox.path());
        let store = FileStore::new(&roots.data);
        let service = LibraryService::new(store);
        let clock = fixed_clock();
        let environment = FixedEnvironment {
            variables: BTreeMap::new(),
            system_locale: Locale::ZhTw,
            platform: InterpreterPlatform::Other,
        };
        let output = RecordingOutput::default();
        let host = TuiHost::new(
            &service,
            roots,
            Locale::En,
            &clock,
            test_platform(&environment, &output),
        )
        .unwrap();

        let saved = host
            .serve(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([("lang".to_owned(), "auto".to_owned())]),
                },
            )))
            .unwrap();

        assert!(matches!(
            saved,
            UiAction::PreferencesSaved { ref locale, .. } if locale == "zh-TW"
        ));
        assert_eq!(host.locale(), Locale::ZhTw);
    }

    #[test]
    fn an_expanded_user_path_needs_a_home_and_answers_one_bare_tilde() {
        assert_eq!(
            expand_user_path_with_home(Path::new("~/notes"), None, InterpreterPlatform::Other),
            PathBuf::from("~/notes")
        );
        assert_eq!(
            expand_user_path_with_home(
                Path::new("~"),
                Some(Path::new("/explicit-home")),
                InterpreterPlatform::Other,
            ),
            PathBuf::from("/explicit-home")
        );
    }

    /// The borrowed probe is the production wrapper that lends one probe to a nested plan. Each
    /// method must reach the owner, including the `exists` answer that the trait can default.
    #[test]
    fn the_borrowed_probe_sends_every_question_to_its_owner() {
        let sandbox = TempDir::new().unwrap();
        let source = sandbox.path().join("present.txt");
        std::fs::write(&source, b"present\n").unwrap();
        let probe = RecordingProbe {
            programs: BTreeMap::from([("node".to_owned(), PathBuf::from("/runtime/node"))]),
            ..RecordingProbe::default()
        };
        let borrowed = super::super::BorrowedProgramProbe(&probe);

        assert_eq!(
            borrowed.find_program("node"),
            Some(PathBuf::from("/runtime/node"))
        );
        assert!(borrowed.is_file(&source));
        assert!(borrowed.is_dir(sandbox.path()));
        assert!(borrowed.exists(&source));
        assert!(!borrowed.is_executable(&source));
        assert_eq!(
            probe.calls.borrow().clone(),
            vec![
                "find:node".to_owned(),
                format!("file:{}", source.display()),
                format!("dir:{}", sandbox.path().display()),
                format!("file:{}", source.display()),
                format!("executable:{}", source.display()),
            ]
        );
    }
}
