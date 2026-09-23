//! Recording adapters for every port of the production host.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use skit_i18n::Locale;
use skit_runtime::{
    DependencyCommand, DependencyCommandOutput, DependencyCommandRunner, InjectedCommand,
    InjectedCommandOutput, InjectedCommandRunner, InjectedCommandUnavailable, InterpreterPlatform,
    JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateRunner, JavaScriptSyntaxGateUnavailable,
    LaunchPlan, LaunchProcessOutput, LaunchRunner, ProgramProbe, UvArchiveFetchError,
    UvArchiveFetcher, UvDownloadConsent,
};
use skit_store::AgentSkillInstallPoint;
use time::{Date, Month, OffsetDateTime, Time, UtcOffset};

use super::{
    observation::{byte_value, encode_hex, escaped_os, io_error_value, readable_plan_files},
    projection::PathMap,
};
use crate::{
    cli::tui_host::{
        EditorLauncher, FileAllocator, HostEnvironment, HostOutput, PreferenceFiles,
        PrivateDirectoryPurpose, TempLocation, TemporaryFilePurpose, TerminalCapability,
        TuiPlatform, set_private_directory_mode, set_private_file_mode,
    },
    run::{RunClock, RunPorts},
};

#[derive(Clone, Debug)]
pub(super) struct FixedClock {
    at: OffsetDateTime,
    events: Rc<RefCell<Vec<PortEvent>>>,
}

impl FixedClock {
    pub(super) fn new(events: Rc<RefCell<Vec<PortEvent>>>) -> Self {
        Self {
            at: fixed_instant(),
            events,
        }
    }
}

pub(super) fn fixed_instant() -> OffsetDateTime {
    Date::from_calendar_date(2026, Month::August, 28)
        .expect("the fixed month has this date")
        .with_time(Time::from_hms(12, 34, 56).expect("the fixed time is valid"))
        .assume_utc()
}

impl RunClock for FixedClock {
    fn now_utc(&self) -> OffsetDateTime {
        self.events.borrow_mut().push(PortEvent::Clock(self.at));
        self.at
    }
}

#[derive(Clone, Debug)]
pub(super) enum PortEvent {
    Clock(OffsetDateTime),
    Platform(InterpreterPlatform),
    EnvironmentVariable {
        name: String,
        result: Option<OsString>,
    },
    EnvironmentSnapshot(BTreeMap<String, String>),
    LocalOffset(UtcOffset),
    SystemLocale(Locale),
    Terminal {
        stream: String,
        result: bool,
    },
    Allocation {
        purpose: AllocationPurpose,
        attempt: u128,
        location: AllocationLocation,
        path: Option<PathBuf>,
        outcome: AllocationOutcome,
    },
    Probe {
        operation: String,
        path: PathBuf,
        result: ProbeResult,
    },
    Editor {
        argv: Vec<String>,
        path: PathBuf,
        outcome: Value,
    },
    Launch {
        plan: LaunchPlan,
        readable_files: Vec<(PathBuf, Vec<u8>)>,
        outcome: Value,
    },
    Dependency {
        command: DependencyCommand,
        outcome: Value,
    },
    Injected {
        command: InjectedCommand,
        readable_files: Vec<(PathBuf, Vec<u8>)>,
        outcome: Value,
    },
    JavaScriptGate {
        program: PathBuf,
        source: PathBuf,
        timeout: Duration,
        bytes: Option<Vec<u8>>,
        outcome: Value,
    },
    UvConsent {
        version: String,
        destination: PathBuf,
        result: bool,
    },
    UvFetch {
        url: String,
        limit: u64,
        outcome: Value,
    },
    Preference {
        operation: String,
        path: PathBuf,
        outcome: Value,
    },
    Output {
        stream: String,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AllocationPurpose {
    TemporaryFile(TemporaryFilePurpose),
    PrivateDirectory(PrivateDirectoryPurpose),
}

impl AllocationPurpose {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::TemporaryFile(purpose) => purpose.label(),
            Self::PrivateDirectory(purpose) => purpose.label(),
        }
    }
}

impl From<TemporaryFilePurpose> for AllocationPurpose {
    fn from(value: TemporaryFilePurpose) -> Self {
        Self::TemporaryFile(value)
    }
}

impl From<PrivateDirectoryPurpose> for AllocationPurpose {
    fn from(value: PrivateDirectoryPurpose) -> Self {
        Self::PrivateDirectory(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AllocationLocation {
    System(PathBuf),
    Directory(PathBuf),
}

impl AllocationLocation {
    pub(super) fn path(&self) -> &Path {
        match self {
            Self::System(path) | Self::Directory(path) => path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AllocationFailure {
    pub(super) kind: io::ErrorKind,
    pub(super) reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum AllocationOutcome {
    Accepted,
    Rejected(AllocationFailure),
}

impl From<&io::Error> for AllocationFailure {
    fn from(error: &io::Error) -> Self {
        Self {
            kind: error.kind(),
            reason: if error.kind() == io::ErrorKind::AlreadyExists {
                "the allocation candidate already exists".to_owned()
            } else {
                error.to_string()
            },
        }
    }
}

#[derive(Debug)]
struct AllocationCounter {
    next: Cell<Option<u128>>,
    maximum: u128,
}

impl AllocationCounter {
    pub(super) const fn new(maximum: u128) -> Self {
        Self {
            next: Cell::new(Some(0)),
            maximum,
        }
    }

    pub(super) fn take(&self, purpose: AllocationPurpose) -> Result<u128, (u128, io::Error)> {
        let Some(candidate) = self.next.get() else {
            return Err((
                self.maximum.checked_add(1).unwrap_or(self.maximum),
                io::Error::other(format!("{} allocation counter overflow", purpose.label())),
            ));
        };
        self.next.set(
            candidate
                .checked_add(1)
                .filter(|next| *next <= self.maximum),
        );
        Ok(candidate)
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct AllocationMaximums {
    pub(super) authored_draft: u128,
    pub(super) draft_quarantine: u128,
    pub(super) injected_source: u128,
}

impl Default for AllocationMaximums {
    fn default() -> Self {
        Self {
            authored_draft: 0x00ff_ffff,
            draft_quarantine: 0x00ff_ffff,
            injected_source: 0x00ff_ffff,
        }
    }
}

#[derive(Debug)]
struct AllocationCounters {
    authored_draft: AllocationCounter,
    draft_quarantine: AllocationCounter,
    injected_source: AllocationCounter,
}

impl AllocationCounters {
    pub(super) fn new(maximums: AllocationMaximums) -> Self {
        Self {
            authored_draft: AllocationCounter::new(maximums.authored_draft),
            draft_quarantine: AllocationCounter::new(maximums.draft_quarantine),
            injected_source: AllocationCounter::new(maximums.injected_source),
        }
    }

    const fn for_purpose(&self, purpose: AllocationPurpose) -> &AllocationCounter {
        match purpose {
            AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft) => {
                &self.authored_draft
            }
            AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource) => {
                &self.injected_source
            }
            AllocationPurpose::PrivateDirectory(PrivateDirectoryPurpose::DraftQuarantine) => {
                &self.draft_quarantine
            }
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum ProbeResult {
    Path(Option<PathBuf>),
    Bool(bool),
}

#[derive(Clone, Debug)]
pub(super) enum LaunchScript {
    Success(LaunchProcessOutput),
    Failure { kind: io::ErrorKind, reason: String },
}

#[derive(Clone, Debug, Default)]
pub(super) enum EditorScript {
    #[default]
    Success,
    Write(Vec<u8>),
    Failure {
        kind: io::ErrorKind,
        reason: String,
    },
}

#[derive(Clone, Debug, Default)]
pub(super) enum DependencyScript {
    #[default]
    Success,
    Completed(DependencyCommandOutput),
    Failure {
        kind: io::ErrorKind,
        reason: String,
    },
}

#[derive(Clone, Debug, Default)]
pub(super) enum InjectedScript {
    #[default]
    Success,
    Completed(InjectedCommandOutput),
    Unavailable(InjectedCommandUnavailable),
}

#[derive(Clone, Debug, Default)]
pub(super) enum JavaScriptGateScript {
    #[default]
    Success,
    Completed(JavaScriptSyntaxGateOutput),
    Unavailable(JavaScriptSyntaxGateUnavailable),
}

#[derive(Clone, Debug)]
pub(super) enum UvFetchScript {
    Failure(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug)]
pub(super) struct PreferenceFailure {
    pub(super) point: AgentSkillInstallPoint,
    pub(super) kind: io::ErrorKind,
    pub(super) reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PrivateModeTarget {
    File,
    Directory,
}

#[derive(Clone, Debug)]
pub(super) struct PrivateModeFailure {
    pub(super) target: PrivateModeTarget,
    pub(super) kind: io::ErrorKind,
    pub(super) reason: String,
}

impl Default for LaunchScript {
    fn default() -> Self {
        Self::Success(LaunchProcessOutput {
            exit_code: Some(0),
            signal: None,
        })
    }
}

#[derive(Debug)]
pub(super) struct RecordingAdapters {
    pub(super) events: Rc<RefCell<Vec<PortEvent>>>,
    pub(super) system_temp: PathBuf,
    allocation_counters: AllocationCounters,
    pub(super) private_mode_failure: RefCell<Option<PrivateModeFailure>>,
    pub(super) variables: BTreeMap<String, String>,
    pub(super) programs: BTreeMap<String, PathBuf>,
    pub(super) launch_script: RefCell<LaunchScript>,
    pub(super) editor_write_queue: RefCell<VecDeque<Vec<u8>>>,
    pub(super) editor_script: RefCell<EditorScript>,
    pub(super) dependency_script: RefCell<DependencyScript>,
    pub(super) injected_script: RefCell<InjectedScript>,
    pub(super) javascript_gate_script: RefCell<JavaScriptGateScript>,
    pub(super) uv_consent: Cell<bool>,
    pub(super) uv_fetch_script: RefCell<UvFetchScript>,
    pub(super) preference_failure: RefCell<Option<PreferenceFailure>>,
    pub(super) stdin_terminal: Cell<bool>,
    pub(super) stdout_terminal: Cell<bool>,
    pub(super) system_locale: Cell<Locale>,
}

impl RecordingAdapters {
    pub(super) fn new(events: Rc<RefCell<Vec<PortEvent>>>, system_temp: PathBuf) -> Self {
        Self::new_with_maximums(events, system_temp, AllocationMaximums::default())
    }

    fn new_with_maximums(
        events: Rc<RefCell<Vec<PortEvent>>>,
        system_temp: PathBuf,
        maximums: AllocationMaximums,
    ) -> Self {
        Self::new_with_maximums_and_editor_writes(events, system_temp, maximums, Vec::new())
    }

    pub(super) fn new_with_maximums_and_editor_writes(
        events: Rc<RefCell<Vec<PortEvent>>>,
        system_temp: PathBuf,
        maximums: AllocationMaximums,
        editor_writes: Vec<Vec<u8>>,
    ) -> Self {
        Self {
            events,
            system_temp,
            allocation_counters: AllocationCounters::new(maximums),
            private_mode_failure: RefCell::new(None),
            variables: BTreeMap::from([
                ("HOME".to_owned(), "/explicit/home".to_owned()),
                ("LANG".to_owned(), "en_US.UTF-8".to_owned()),
                ("SENTINEL".to_owned(), "walker".to_owned()),
            ]),
            programs: ["agent", "bash", "node", "npm", "sh", "uv"]
                .into_iter()
                .map(|name| {
                    (
                        name.to_owned(),
                        PathBuf::from(format!("/virtual/bin/{name}")),
                    )
                })
                .collect(),
            launch_script: RefCell::new(LaunchScript::default()),
            editor_write_queue: RefCell::new(editor_writes.into()),
            editor_script: RefCell::new(EditorScript::default()),
            dependency_script: RefCell::new(DependencyScript::default()),
            injected_script: RefCell::new(InjectedScript::default()),
            javascript_gate_script: RefCell::new(JavaScriptGateScript::default()),
            uv_consent: Cell::new(false),
            uv_fetch_script: RefCell::new(UvFetchScript::Failure(
                "network is disabled in the real walker host".to_owned(),
            )),
            preference_failure: RefCell::new(None),
            stdin_terminal: Cell::new(false),
            stdout_terminal: Cell::new(false),
            system_locale: Cell::new(Locale::En),
        }
    }

    pub(super) fn private_mode_failure(&self, target: PrivateModeTarget) -> io::Result<()> {
        let failure = self.private_mode_failure.borrow_mut().take();
        if let Some(failure) = failure {
            assert_eq!(failure.target, target);
            return Err(io::Error::new(failure.kind, failure.reason));
        }
        Ok(())
    }

    fn set_private_file_mode(&self, file: &fs::File) -> io::Result<()> {
        self.private_mode_failure(PrivateModeTarget::File)?;
        set_private_file_mode(file)
    }

    pub(super) fn set_private_directory_mode(&self, path: &Path) -> io::Result<()> {
        self.private_mode_failure(PrivateModeTarget::Directory)?;
        set_private_directory_mode(path)
    }

    pub(super) fn platform(&self) -> TuiPlatform<'_> {
        TuiPlatform::new(
            self,
            self,
            self,
            RunPorts::new(self, self, self, self, self, self, self),
            self,
            self,
            self,
        )
    }

    pub(super) fn push(&self, event: PortEvent) {
        self.events.borrow_mut().push(event);
    }

    fn allocate_with<T>(
        &self,
        purpose: AllocationPurpose,
        prefix: &str,
        width: usize,
        location: AllocationLocation,
        suffix: &str,
        mut create: impl FnMut(&Path) -> io::Result<T>,
    ) -> Result<(T, PathBuf), (PathBuf, io::Error)> {
        loop {
            let attempt = match self.allocation_counters.for_purpose(purpose).take(purpose) {
                Ok(attempt) => attempt,
                Err((overflow_attempt, error)) => {
                    self.push(PortEvent::Allocation {
                        purpose,
                        attempt: overflow_attempt,
                        location: location.clone(),
                        path: None,
                        outcome: AllocationOutcome::Rejected(AllocationFailure::from(&error)),
                    });
                    return Err((location.path().to_path_buf(), error));
                }
            };
            let name = format!("{prefix}{attempt:0width$x}{suffix}");
            let path = location.path().join(name);
            match create(&path) {
                Ok(value) => {
                    self.push(PortEvent::Allocation {
                        purpose,
                        attempt,
                        location,
                        path: Some(path.clone()),
                        outcome: AllocationOutcome::Accepted,
                    });
                    return Ok((value, path));
                }
                Err(error) => {
                    let collision = error.kind() == io::ErrorKind::AlreadyExists;
                    self.push(PortEvent::Allocation {
                        purpose,
                        attempt,
                        location: location.clone(),
                        path: Some(path.clone()),
                        outcome: AllocationOutcome::Rejected(AllocationFailure::from(&error)),
                    });
                    if !collision {
                        return Err((path, error));
                    }
                }
            }
        }
    }

    pub(super) fn transcript(&self, paths: &mut PathMap) -> Vec<Value> {
        self.register_pending_artifacts(paths);
        Self::render_transcript(&self.events.borrow(), paths)
    }

    pub(super) fn pending_events(&self) -> Vec<PortEvent> {
        self.events.borrow().clone()
    }

    pub(super) fn drain_event_prefix(&self, count: usize) -> Result<(), String> {
        let mut events = self.events.borrow_mut();
        if events.len() < count {
            return Err("the walker port transcript changed during checkpoint capture".to_owned());
        }
        events.drain(..count);
        Ok(())
    }

    fn register_pending_artifacts(&self, paths: &mut PathMap) {
        for event in self.events.borrow().iter() {
            paths.register_event_artifacts(event);
        }
    }

    pub(super) fn render_transcript(events: &[PortEvent], paths: &mut PathMap) -> Vec<Value> {
        events
            .iter()
            .map(|event| match event {
                PortEvent::Clock(at) => json!({ "clock": skit_store::iso_stamp(*at) }),
                PortEvent::Platform(platform) => {
                    json!({ "platform": format!("{platform:?}") })
                }
                PortEvent::EnvironmentVariable { name, result } => json!({
                    "environment_variable": name,
                    "result": result.as_deref().map(escaped_os),
                }),
                PortEvent::EnvironmentSnapshot(result) => {
                    json!({ "environment_snapshot": result })
                }
                PortEvent::LocalOffset(result) => {
                    json!({ "local_offset_seconds": result.whole_seconds() })
                }
                PortEvent::SystemLocale(result) => {
                    json!({ "system_locale": format!("{result:?}") })
                }
                PortEvent::Terminal { stream, result } => json!({
                    "terminal": stream,
                    "result": result,
                }),
                PortEvent::Allocation {
                    purpose,
                    attempt,
                    location,
                    path,
                    outcome,
                } => json!({
                    "allocation": {
                        "purpose": purpose.label(),
                        "attempt": u64::try_from(*attempt)
                            .map_or_else(|_| Value::String(attempt.to_string()), Value::from),
                        "location": match location {
                            AllocationLocation::System(_) => json!({ "kind": "system" }),
                            AllocationLocation::Directory(path) => json!({
                                "kind": "directory",
                                "path": paths.normalize_path(path),
                            }),
                        },
                        "path": path.as_ref().map(|path| paths.normalize_path(path)),
                        "outcome": match outcome {
                            AllocationOutcome::Accepted => json!({ "accepted": true }),
                            AllocationOutcome::Rejected(error) => json!({ "rejected": {
                                "kind": format!("{:?}", error.kind),
                                "reason": error.reason,
                            }}),
                        },
                    }
                }),
                PortEvent::Probe { operation, path, result } => json!({
                    "probe": operation,
                    "path": paths.normalize_path(path),
                    "result": match result {
                        ProbeResult::Path(Some(path)) => Value::String(paths.normalize_path(path)),
                        ProbeResult::Path(None) => Value::Null,
                        ProbeResult::Bool(result) => Value::Bool(*result),
                    },
                }),
                PortEvent::Editor { argv, path, outcome } => json!({
                    "editor": argv.iter().map(|argument| paths.normalize_argument(argument)).collect::<Vec<_>>(),
                    "path": paths.normalize_path(path),
                    "outcome": outcome,
                }),
                PortEvent::Launch { plan, readable_files, outcome } => json!({
                    "launch": {
                        "program": paths.normalize_path(&plan.program),
                        "args": plan.args.iter().map(|arg| paths.normalize_argument(arg)).collect::<Vec<_>>(),
                        "environment": plan.env.iter().map(|(key, value)| (key.clone(), paths.normalize_argument(value))).collect::<BTreeMap<_, _>>(),
                        "cwd": paths.normalize_path(&plan.cwd),
                        "display": paths.normalize_host_text(&plan.display),
                        "warnings": plan.warnings.iter().map(|warning| format!("{warning:?}")).collect::<Vec<_>>(),
                        "readable_files": readable_files.iter().map(|(path, bytes)| json!({
                            "path": paths.normalize_path(path),
                            "content": paths.byte_view(path, bytes),
                        })).collect::<Vec<_>>(),
                        "outcome": outcome,
                    }
                }),
                PortEvent::Dependency { command, outcome } => json!({
                    "dependency": {
                        "program": paths.normalize_path(&command.program),
                        "args": command.args.iter().map(|arg| paths.normalize_argument(arg)).collect::<Vec<_>>(),
                        "cwd": paths.normalize_path(&command.cwd),
                        "environment": command.environment,
                        "outcome": outcome,
                    }
                }),
                PortEvent::Injected { command, readable_files, outcome } => json!({
                    "injected": {
                        "program": paths.normalize_path(&command.program),
                        "args": command.args.iter().map(|arg| paths.normalize_os_argument(arg)).collect::<Vec<_>>(),
                        "timeout_ms": command.timeout.as_millis(),
                        "readable_files": readable_files.iter().map(|(path, bytes)| json!({
                            "path": paths.normalize_path(path),
                            "content": paths.byte_view(path, bytes),
                        })).collect::<Vec<_>>(),
                        "outcome": outcome,
                    }
                }),
                PortEvent::JavaScriptGate { program, source, timeout, bytes, outcome } => json!({
                    "javascript_gate": {
                        "program": paths.normalize_path(program),
                        "source": paths.normalize_path(source),
                        "timeout_ms": timeout.as_millis(),
                        "content": bytes.as_ref().map(|bytes| paths.byte_view(source, bytes)),
                        "outcome": outcome,
                    }
                }),
                PortEvent::UvConsent { version, destination, result } => json!({
                    "uv_consent": {
                        "version": version,
                        "destination": paths.normalize_path(destination),
                        "result": result,
                    }
                }),
                PortEvent::UvFetch { url, limit, outcome } => {
                    json!({ "uv_fetch": { "url": url, "limit": limit, "outcome": outcome } })
                }
                PortEvent::Preference { operation, path, outcome } => json!({
                    "preference": operation,
                    "path": paths.normalize_path(path),
                    "outcome": outcome,
                }),
                PortEvent::Output { stream, text } => json!({
                    "output": stream,
                    "text": paths.normalize_host_text(text),
                }),
            })
            .collect()
    }
}

fn create_private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

fn create_private_directory(path: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

impl FileAllocator for RecordingAdapters {
    fn temporary_file(
        &self,
        purpose: TemporaryFilePurpose,
        location: TempLocation<'_>,
        suffix: &str,
    ) -> io::Result<tempfile::NamedTempFile> {
        let event_purpose = AllocationPurpose::from(purpose);
        let location = match location {
            TempLocation::System => AllocationLocation::System(self.system_temp.clone()),
            TempLocation::Directory(path) => AllocationLocation::Directory(path.to_path_buf()),
        };
        self.allocate_with(
            event_purpose,
            purpose.prefix(),
            purpose.width(),
            location,
            suffix,
            |path| {
                let file = create_private_file(path)?;
                let temp_path = tempfile::TempPath::try_from_path(path.to_path_buf())
                    .expect("walker allocation paths are absolute");
                let temporary = tempfile::NamedTempFile::from_parts(file, temp_path);
                self.set_private_file_mode(temporary.as_file())?;
                Ok(temporary)
            },
        )
        .map(|(file, _)| file)
        .map_err(|(_, error)| error)
    }

    fn private_directory(
        &self,
        purpose: PrivateDirectoryPurpose,
        location: &Path,
    ) -> io::Result<PathBuf> {
        self.allocate_with(
            AllocationPurpose::from(purpose),
            purpose.prefix(),
            purpose.width(),
            AllocationLocation::Directory(location.to_path_buf()),
            "",
            |path| {
                create_private_directory(path)?;
                if let Err(error) = self.set_private_directory_mode(path) {
                    let _ = fs::remove_dir(path);
                    return Err(error);
                }
                Ok(())
            },
        )
        .map(|((), path)| path)
        .map_err(|(_, error)| error)
    }
}

impl HostEnvironment for RecordingAdapters {
    fn platform(&self) -> InterpreterPlatform {
        let result = InterpreterPlatform::Other;
        self.push(PortEvent::Platform(result));
        result
    }

    fn variable(&self, name: &str) -> Option<OsString> {
        let result = self.variables.get(name).map(OsString::from);
        self.push(PortEvent::EnvironmentVariable {
            name: name.to_owned(),
            result: result.clone(),
        });
        result
    }

    fn variables(&self) -> BTreeMap<String, String> {
        let result = self.variables.clone();
        self.push(PortEvent::EnvironmentSnapshot(result.clone()));
        result
    }

    fn local_offset(&self) -> UtcOffset {
        let result = UtcOffset::UTC;
        self.push(PortEvent::LocalOffset(result));
        result
    }

    fn system_locale(&self) -> Locale {
        let result = self.system_locale.get();
        self.push(PortEvent::SystemLocale(result));
        result
    }
}

impl TerminalCapability for RecordingAdapters {
    fn stdin_is_terminal(&self) -> bool {
        let result = self.stdin_terminal.get();
        self.push(PortEvent::Terminal {
            stream: "stdin".to_owned(),
            result,
        });
        result
    }

    fn stdout_is_terminal(&self) -> bool {
        let result = self.stdout_terminal.get();
        self.push(PortEvent::Terminal {
            stream: "stdout".to_owned(),
            result,
        });
        result
    }
}

impl EditorLauncher for RecordingAdapters {
    fn launch(&self, argv: &[String], path: &Path) -> io::Result<()> {
        let script = self
            .editor_write_queue
            .borrow_mut()
            .pop_front()
            .map(EditorScript::Write)
            .unwrap_or_else(|| self.editor_script.replace(EditorScript::default()));
        let result = match script {
            EditorScript::Success => Ok(()),
            EditorScript::Write(bytes) => fs::write(path, bytes),
            EditorScript::Failure { kind, reason } => Err(io::Error::new(kind, reason)),
        };
        let outcome = match &result {
            Ok(()) => json!({ "ok": true }),
            Err(error) => json!({
                "error": { "kind": format!("{:?}", error.kind()), "reason": error.to_string() }
            }),
        };
        self.push(PortEvent::Editor {
            argv: argv.to_vec(),
            path: path.to_path_buf(),
            outcome,
        });
        result
    }
}

impl ProgramProbe for RecordingAdapters {
    fn find_program(&self, name: &str) -> Option<PathBuf> {
        let requested = PathBuf::from(name);
        let result = self
            .programs
            .get(name)
            .cloned()
            .or_else(|| {
                requested
                    .starts_with("/virtual/bin")
                    .then_some(requested.clone())
            })
            .or_else(|| {
                fixture_program_is_explicit(&requested)
                    .then(|| fixture_path_is_executable(&requested))
                    .filter(|executable| *executable)
                    .map(|_| requested.clone())
            });
        self.push(PortEvent::Probe {
            operation: "find_program".to_owned(),
            path: requested,
            result: ProbeResult::Path(result.clone()),
        });
        result
    }

    fn is_file(&self, path: &Path) -> bool {
        let result = path.starts_with("/virtual/bin") || path.is_file();
        self.push(PortEvent::Probe {
            operation: "is_file".to_owned(),
            path: path.to_path_buf(),
            result: ProbeResult::Bool(result),
        });
        result
    }

    fn is_dir(&self, path: &Path) -> bool {
        let result = path.is_dir();
        self.push(PortEvent::Probe {
            operation: "is_dir".to_owned(),
            path: path.to_path_buf(),
            result: ProbeResult::Bool(result),
        });
        result
    }

    fn exists(&self, path: &Path) -> bool {
        let result = path.starts_with("/virtual/bin") || path.exists();
        self.push(PortEvent::Probe {
            operation: "exists".to_owned(),
            path: path.to_path_buf(),
            result: ProbeResult::Bool(result),
        });
        result
    }

    fn is_executable(&self, path: &Path) -> bool {
        let result = path.starts_with("/virtual/bin") || fixture_path_is_executable(path);
        self.push(PortEvent::Probe {
            operation: "is_executable".to_owned(),
            path: path.to_path_buf(),
            result: ProbeResult::Bool(result),
        });
        result
    }
}

fn fixture_program_is_explicit(path: &Path) -> bool {
    path.is_absolute() || path.components().count() > 1
}

#[cfg(unix)]
fn fixture_path_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn fixture_path_is_executable(path: &Path) -> bool {
    path.is_file()
}

impl LaunchRunner for RecordingAdapters {
    fn run(&self, plan: &LaunchPlan) -> io::Result<LaunchProcessOutput> {
        let script = self.launch_script.replace(LaunchScript::default());
        let outcome = match &script {
            LaunchScript::Success(output) => json!({
                "ok": {
                    "exit_code": output.exit_code,
                    "signal": output.signal,
                }
            }),
            LaunchScript::Failure { kind, reason } => json!({
                "error": {
                    "kind": format!("{kind:?}"),
                    "reason": reason,
                }
            }),
        };
        self.push(PortEvent::Launch {
            plan: plan.clone(),
            readable_files: readable_plan_files(plan),
            outcome,
        });
        match script {
            LaunchScript::Success(output) => Ok(output),
            LaunchScript::Failure { kind, reason } => Err(io::Error::new(kind, reason)),
        }
    }
}

impl DependencyCommandRunner for RecordingAdapters {
    fn run(&self, command: &DependencyCommand) -> io::Result<DependencyCommandOutput> {
        let script = self.dependency_script.replace(DependencyScript::default());
        let result =
            match script {
                DependencyScript::Success => fs::create_dir_all(command.cwd.join("node_modules"))
                    .map(|()| DependencyCommandOutput {
                        success: true,
                        exit_code: Some(0),
                        stderr: Vec::new(),
                    }),
                DependencyScript::Completed(output) => {
                    if output.success {
                        fs::create_dir_all(command.cwd.join("node_modules")).map(|()| output)
                    } else {
                        Ok(output)
                    }
                }
                DependencyScript::Failure { kind, reason } => Err(io::Error::new(kind, reason)),
            };
        let outcome = match &result {
            Ok(output) => json!({
                "ok": {
                    "success": output.success,
                    "exit_code": output.exit_code,
                    "stderr": byte_value(&output.stderr),
                }
            }),
            Err(error) => io_error_value(error),
        };
        self.push(PortEvent::Dependency {
            command: command.clone(),
            outcome,
        });
        result
    }
}

impl InjectedCommandRunner for RecordingAdapters {
    fn run(
        &self,
        command: &InjectedCommand,
    ) -> Result<InjectedCommandOutput, InjectedCommandUnavailable> {
        let script = self.injected_script.replace(InjectedScript::default());
        let result = match script {
            InjectedScript::Success => Ok(InjectedCommandOutput {
                success: true,
                stderr: Vec::new(),
            }),
            InjectedScript::Completed(output) => Ok(output),
            InjectedScript::Unavailable(error) => Err(error),
        };
        let outcome = match &result {
            Ok(output) => json!({
                "ok": {
                    "success": output.success,
                    "stderr": byte_value(&output.stderr),
                }
            }),
            Err(error) => json!({ "unavailable": error.to_string() }),
        };
        self.push(PortEvent::Injected {
            command: command.clone(),
            readable_files: command
                .args
                .iter()
                .filter_map(|argument| {
                    let path = PathBuf::from(argument);
                    fs::read(&path).ok().map(|bytes| (path, bytes))
                })
                .collect(),
            outcome,
        });
        result
    }
}

impl JavaScriptSyntaxGateRunner for RecordingAdapters {
    fn check(
        &self,
        program: &Path,
        source: &Path,
        timeout: Duration,
    ) -> Result<JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable> {
        let script = self
            .javascript_gate_script
            .replace(JavaScriptGateScript::default());
        let result = match script {
            JavaScriptGateScript::Success => Ok(JavaScriptSyntaxGateOutput {
                success: true,
                stderr: Vec::new(),
            }),
            JavaScriptGateScript::Completed(output) => Ok(output),
            JavaScriptGateScript::Unavailable(error) => Err(error),
        };
        let outcome = match &result {
            Ok(output) => json!({
                "ok": {
                    "success": output.success,
                    "stderr": byte_value(&output.stderr),
                }
            }),
            Err(error) => json!({ "unavailable": error.to_string() }),
        };
        self.push(PortEvent::JavaScriptGate {
            program: program.to_path_buf(),
            source: source.to_path_buf(),
            timeout,
            bytes: fs::read(source).ok(),
            outcome,
        });
        result
    }
}

impl UvDownloadConsent for RecordingAdapters {
    fn allow_download(&self, version: &str, destination: &Path) -> bool {
        let result = self.uv_consent.get();
        self.push(PortEvent::UvConsent {
            version: version.to_owned(),
            destination: destination.to_path_buf(),
            result,
        });
        result
    }
}

impl UvArchiveFetcher for RecordingAdapters {
    fn fetch(&self, url: &str, limit: u64) -> Result<Vec<u8>, UvArchiveFetchError> {
        let script = self.uv_fetch_script.replace(UvFetchScript::Failure(
            "network is disabled in the real walker host".to_owned(),
        ));
        let outcome = match &script {
            UvFetchScript::Failure(reason) => json!({ "error": reason }),
            UvFetchScript::Bytes(bytes) => json!({
                "ok": {
                    "length": bytes.len(),
                    "sha256": encode_hex(&Sha256::digest(bytes)),
                }
            }),
        };
        self.push(PortEvent::UvFetch {
            url: url.to_owned(),
            limit,
            outcome,
        });
        match script {
            UvFetchScript::Failure(reason) => Err(UvArchiveFetchError::new(reason)),
            UvFetchScript::Bytes(bytes) => Ok(bytes),
        }
    }
}

impl PreferenceFiles for RecordingAdapters {
    fn is_file(&self, path: &Path) -> bool {
        let result = path.is_file();
        self.push(PortEvent::Preference {
            operation: "is_file".to_owned(),
            path: path.to_path_buf(),
            outcome: json!(result),
        });
        result
    }

    fn is_dir(&self, path: &Path) -> bool {
        let result = path.is_dir();
        self.push(PortEvent::Preference {
            operation: "is_dir".to_owned(),
            path: path.to_path_buf(),
            outcome: json!(result),
        });
        result
    }

    fn agent_skill_checkpoint(&self, point: AgentSkillInstallPoint, path: &Path) -> io::Result<()> {
        let failure = self.preference_failure.borrow().clone();
        let outcome = failure
            .as_ref()
            .filter(|failure| failure.point == point)
            .map_or_else(
                || json!({ "ok": true }),
                |failure| {
                    json!({
                        "error": {
                            "kind": format!("{:?}", failure.kind),
                            "reason": failure.reason,
                        }
                    })
                },
            );
        self.push(PortEvent::Preference {
            operation: format!("agent_skill::{point:?}"),
            path: path.to_path_buf(),
            outcome,
        });
        if let Some(failure) = failure.filter(|failure| failure.point == point) {
            return Err(io::Error::new(failure.kind, failure.reason));
        }
        Ok(())
    }
}

impl HostOutput for RecordingAdapters {
    fn success(&self, message: &str) {
        self.record_output("success", message);
    }

    fn detail(&self, message: &str) {
        self.record_output("detail", message);
    }

    fn plain(&self, message: &str) {
        self.record_output("plain", message);
    }

    fn stdout(&self, message: &str) {
        self.record_output("stdout", message);
    }

    fn diagnostic(&self, message: &str) {
        self.record_output("diagnostic", message);
    }
}

impl RecordingAdapters {
    fn record_output(&self, stream: &str, text: &str) {
        self.push(PortEvent::Output {
            stream: stream.to_owned(),
            text: text.to_owned(),
        });
    }
}
