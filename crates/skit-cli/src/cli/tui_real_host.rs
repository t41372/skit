//! Real production-host fixture for the local UI walker.
//!
//! This module is test-only. It composes the production stores and `TuiHost` with low-level
//! recording adapters. It does not model product behavior.

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use skit_application::{
    CreateEntry, LibraryService,
    form_state::{FormStateService, PersistedFormState},
    prompt_selection::PromptSelectionService,
};
use skit_domain::{Slug, StorageMode};
use skit_i18n::{Locale, Localize, Message, format_text, requested_locale};
use skit_runtime::{
    DependencyCommand, DependencyCommandOutput, DependencyCommandRunner, InjectedCommand,
    InjectedCommandOutput, InjectedCommandRunner, InjectedCommandUnavailable, InterpreterPlatform,
    JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateRunner, JavaScriptSyntaxGateUnavailable,
    LaunchPlan, LaunchProcessOutput, LaunchRunner, ProgramProbe, UvArchiveFetchError,
    UvArchiveFetcher, UvDownloadConsent,
};
use skit_store::{
    AgentSkillInstallPoint, EntryCreateClock, FileConfigStore, FileFormStateStore,
    FilePromptSelectionStore, FileStore, PromptRunner,
};
use skit_tui::AGENT_REVIEW_SNAPSHOT_VERSION;
// Only the stable-marker contract reads the sandbox roots.
#[cfg(any(target_os = "linux", target_os = "windows"))]
use skit_tui_walker_support::sandbox::SandboxRoots;
use skit_tui_walker_support::{
    ArtifactError, canonical_json_bytes,
    engine::CheckpointCauseProjection,
    leak_oracle::accept_host_path_token,
    projection::{replace_longest_text_tokens, rewrite_json_pointer_matches},
    sandbox::{
        LEASE_DIRECTORY, NAMESPACE_MARKER_FILE, NamespaceMarker, ParentInitLockMarker,
        ProfileLeaseMarker, QuarantineDecision, SANDBOX_DIRECTORY, SANDBOX_MARKER_FILE,
        STABLE_SANDBOX_NAMESPACE, SafeProfileId, SandboxEvidenceState, SandboxMarker,
        SandboxMetadata, SandboxPlatform, decide_quarantine, profile_cleanup_path,
        profile_lease_path, profile_sandbox_path,
    },
    sentinels::{AscendingRankAllocator, SourceIdentitySentinels},
    validate_stable_namespace_root,
};
use skit_ui::{
    Action, AddAction, AddEffect, Effect, HealthAction, LibraryState, PreferencesAction,
    PreferencesEffect, RunnerEditorAction, RunnerManagerAction, Screen, SettingsAction,
};
use tempfile::TempDir;
use time::{Date, Month, OffsetDateTime, Time, UtcOffset};

use super::tui_host::{
    EditorLauncher, FileAllocator, HostEnvironment, HostOutput, PreferenceFiles,
    PrivateDirectoryPurpose, ProductRoots, TempLocation, TemporaryFilePurpose, TerminalCapability,
    TuiHost, TuiPlatform, set_private_directory_mode, set_private_file_mode,
};
use super::tui_real_sandbox_fs::{
    ChildName, EntryTicket, InitializationGuard, NamespaceHandles, NodeKind as SandboxNodeKind,
    PinnedDirectory, PinnedFile, ProfileLease, SandboxFsError, ValidatedCleanupSandbox,
    ValidatedOriginalSandbox, combine_release,
};
use crate::{
    library::library_surface_at,
    run::{RunClock, RunPorts},
};

#[derive(Clone, Debug, Default)]
pub(super) struct WalkerSeedSpec {
    pub(super) profile: String,
    pub(super) entries: Vec<CreateEntry>,
    pub(super) settings: BTreeMap<String, String>,
    pub(super) runners: Vec<PromptRunner>,
    pub(super) forms: Vec<WalkerFormSeed>,
    pub(super) prompt_runner: String,
    pub(super) external: Vec<WalkerExternalSeed>,
    pub(super) external_references: Vec<WalkerExternalReferenceSeed>,
    pub(super) directories: Vec<WalkerDirectorySeed>,
    pub(super) editor_writes: Vec<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WalkerDirectoryRoot {
    Home,
    Cwd,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WalkerDirectorySeed {
    pub(super) root: WalkerDirectoryRoot,
    pub(super) path: PathBuf,
}

trait DirectorySeedIo {
    fn create_dir(&self, path: &Path) -> io::Result<()>;
    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata>;
    fn set_private_directory_mode(&self, path: &Path) -> io::Result<()>;
}

#[derive(Debug)]
struct SystemDirectorySeedIo;

impl DirectorySeedIo for SystemDirectorySeedIo {
    fn create_dir(&self, path: &Path) -> io::Result<()> {
        fs::create_dir(path)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
        fs::symlink_metadata(path)
    }

    fn set_private_directory_mode(&self, path: &Path) -> io::Result<()> {
        set_private_directory_mode(path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum WalkerExternalSeed {
    File {
        path: PathBuf,
        bytes: Vec<u8>,
        readonly: bool,
        unix_mode: u32,
    },
    Symlink {
        path: PathBuf,
        target: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WalkerFilePickerTree {
    pub(super) root: PathBuf,
    pub(super) directories: BTreeSet<PathBuf>,
    pub(super) files: BTreeSet<PathBuf>,
}

#[derive(Clone, Debug)]
pub(super) struct WalkerExternalReferenceSeed {
    pub(super) request: CreateEntry,
    pub(super) source: PathBuf,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct WalkerFormSeed {
    pub(super) selector: String,
    pub(super) values: BTreeMap<String, String>,
    pub(super) extra_args: Vec<String>,
    pub(super) extra_args_raw: bool,
    pub(super) preset: Option<String>,
    pub(super) last_run: Option<WalkerLastRunSeed>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct WalkerLastRunSeed {
    pub(super) exit: i64,
    pub(super) at: String,
    pub(super) values: Option<BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct HostObservation {
    pub(super) surface: Value,
    pub(super) state: Value,
    pub(super) config: Value,
    pub(super) form_state: BTreeMap<String, Value>,
    pub(super) prompt_runner: String,
    pub(super) drafts: Value,
    pub(super) tree: Vec<TreeRecord>,
    pub(super) transcript: Vec<Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct LeakOracleFacts {
    pub(super) artifacts: BTreeMap<String, ArtifactLeakOracleFact>,
    pub(super) renderer_drafts: BTreeMap<String, RendererDraftFact>,
    /// Paths from the process that created this host. These facts never enter recorded JSON.
    pub(super) ambient_paths: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ArtifactLeakOracleFact {
    pub(super) raw_path_spellings: BTreeSet<String>,
    pub(super) source_identities: BTreeSet<SourceIdentityLeakOraclePair>,
    pub(super) modified_values: BTreeSet<ModifiedLeakOraclePair>,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct SourceIdentityLeakOraclePair {
    pub(super) raw: Vec<u8>,
    pub(super) projected: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ModifiedLeakOraclePair {
    pub(super) raw: u64,
    pub(super) projected: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct RendererDraftFact {
    pub(super) raw_path: String,
    pub(super) projected_path: String,
    pub(super) raw_kind_picker_basename: String,
    pub(super) raw_lossy_basename: String,
    pub(super) projected_basename: String,
    pub(super) review_names: BTreeSet<RendererReviewNameFact>,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct RendererReviewNameFact {
    pub(super) kind: String,
    pub(super) raw_source_path: String,
    pub(super) projected_source_path: String,
    pub(super) raw_name: String,
    pub(super) projected_name: String,
}

struct PendingHostObservation {
    surface: Value,
    config: Value,
    form_state: BTreeMap<String, Value>,
    prompt_runner: String,
    drafts: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObservationNodeKind {
    File,
    Directory,
    Symlink,
}

impl ObservationNodeKind {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        if metadata.file_type().is_symlink() {
            Self::Symlink
        } else if metadata.is_file() {
            Self::File
        } else {
            Self::Directory
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
        }
    }
}

fn has_store_owned_copy_payload(entry: &skit_domain::Entry) -> bool {
    entry.meta.mode == skit_domain::StorageMode::Copy
        && !matches!(entry.meta.kind.as_str(), "command" | "exe")
}

#[derive(Debug, Default)]
struct ObservationModeProvenance {
    exact: BTreeMap<ObservationModeKey, ObservationNodeKind>,
    deferred_error: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ObservationModeKey(PathBuf);

impl ObservationModeKey {
    fn from_path(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "an observation mode provenance path is not absolute: {}",
                path.display()
            ));
        }
        let Some((parent, name)) = path.parent().zip(path.file_name()) else {
            return Err(format!(
                "an observation mode provenance path has no final component: {}",
                path.display()
            ));
        };
        let parent = fs::canonicalize(parent).map_err(|error| {
            format!(
                "could not resolve observation mode provenance parent {}: {error}",
                parent.display()
            )
        })?;
        Ok(Self(parent.join(name)))
    }
}

impl ObservationModeProvenance {
    fn from_seed(
        sandbox: &SandboxOwner,
        roots: &ProductRoots,
        external_root: &Path,
        system_temp: &Path,
        seeded_directories: &[PathBuf],
        external: &[WalkerExternalSeed],
        seeded_copy_paths: &[PathBuf],
    ) -> Result<Self, String> {
        let mut provenance = Self::default();
        if sandbox.stable().is_some() {
            for path in [
                roots.data.as_path(),
                roots.state.as_path(),
                roots.config.as_path(),
                roots
                    .home
                    .as_deref()
                    .expect("the walker profile has a home"),
                roots.cwd.as_path(),
                external_root,
                system_temp,
            ] {
                provenance.register(path, ObservationNodeKind::Directory)?;
            }
        }
        for seed in external {
            match seed {
                WalkerExternalSeed::File { path, .. } => {
                    provenance.register(&external_root.join(path), ObservationNodeKind::File)?;
                }
                WalkerExternalSeed::Symlink { path, .. } => {
                    provenance.register(&external_root.join(path), ObservationNodeKind::Symlink)?;
                }
            }
        }
        for path in seeded_directories {
            provenance.register(path, ObservationNodeKind::Directory)?;
        }
        for path in seeded_copy_paths {
            provenance.register(path, ObservationNodeKind::File)?;
        }
        Ok(provenance)
    }

    fn register(&mut self, path: &Path, kind: ObservationNodeKind) -> Result<(), String> {
        let key = ObservationModeKey::from_path(path)?;
        if let Some(previous) = self.exact.get(&key) {
            if previous != &kind {
                return Err(format!(
                    "observation mode provenance conflicts at {}: {} and {}",
                    path.display(),
                    previous.label(),
                    kind.label()
                ));
            }
            return Ok(());
        }
        self.exact.insert(key, kind);
        Ok(())
    }

    fn register_if_parent_present(
        &mut self,
        path: &Path,
        kind: ObservationNodeKind,
    ) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return self.register(path, kind);
        };
        if matches!(
            fs::symlink_metadata(parent),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ) {
            return Ok(());
        }
        self.register(path, kind)
    }

    fn expected_kind(&self, path: &Path) -> Result<Option<ObservationNodeKind>, String> {
        let key = ObservationModeKey::from_path(path)?;
        Ok(self.exact.get(&key).copied())
    }

    fn register_existing(&mut self, path: &Path) -> Result<(), String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                let kind = ObservationNodeKind::from_metadata(&metadata);
                self.register(path, kind)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "could not inspect observation mode provenance at {}: {error}",
                path.display()
            )),
        }
    }

    fn refresh_live_sources(&mut self, service: &LibraryService<FileStore>) -> Result<(), String> {
        if let Some(error) = &self.deferred_error {
            return Err(error.clone());
        }
        let mut entries = service
            .repository()
            .scan_entries()
            .map_err(|error| error.to_string())?;
        entries.sort_by(|left, right| left.slug.cmp(&right.slug));
        for entry in entries {
            if has_store_owned_copy_payload(&entry) {
                if let Ok(path) = service.repository().payload_path(&entry) {
                    self.register(&path, ObservationNodeKind::File)?;
                }
            } else if entry.meta.kind.as_str() != "command"
                && let Ok(path) = service.repository().payload_path(&entry)
            {
                self.register_existing(&path)?;
            }
        }
        for draft in sorted_tui_drafts(service.repository().data_dir()) {
            self.register(&draft.path, ObservationNodeKind::File)?;
        }
        Ok(())
    }

    fn record_created_copy(&mut self, service: &LibraryService<FileStore>, action: &Action) {
        let result = self.try_record_created_copy(service, action);
        if let Err(error) = result
            && self.deferred_error.is_none()
        {
            self.deferred_error = Some(error);
        }
    }

    fn try_record_created_copy(
        &mut self,
        service: &LibraryService<FileStore>,
        action: &Action,
    ) -> Result<(), String> {
        let Action::Add(AddAction::CommitFinished {
            result: Ok(slug), ..
        }) = action
        else {
            return Ok(());
        };
        let entry = service.show(slug).map_err(|error| error.to_string())?;
        if !has_store_owned_copy_payload(&entry) {
            return Ok(());
        }
        let path = service
            .repository()
            .payload_path(&entry)
            .map_err(|error| error.to_string())?;
        self.register(&path, ObservationNodeKind::File)
    }

    fn register_event(&mut self, event: &PortEvent, roots: &ProductRoots) -> Result<(), String> {
        match event {
            PortEvent::Allocation {
                purpose,
                path: Some(path),
                outcome: AllocationOutcome::Accepted,
                ..
            } => {
                let kind = match purpose {
                    AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft)
                    | AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource) => {
                        ObservationNodeKind::File
                    }
                    AllocationPurpose::PrivateDirectory(
                        PrivateDirectoryPurpose::DraftQuarantine,
                    ) => ObservationNodeKind::Directory,
                };
                self.register_if_parent_present(path, kind)?;
            }
            PortEvent::Probe {
                operation,
                path,
                result: ProbeResult::Bool(true),
            } if matches!(operation.as_str(), "is_file" | "is_executable") => {
                self.register_managed_executable(path, roots)?;
            }
            PortEvent::Probe {
                result: ProbeResult::Path(Some(path)),
                ..
            } => {
                self.register_managed_executable(path, roots)?;
            }
            PortEvent::Launch { plan, .. } => {
                self.register_managed_executable(&plan.program, roots)?;
            }
            PortEvent::Clock(_)
            | PortEvent::Platform(_)
            | PortEvent::EnvironmentVariable { .. }
            | PortEvent::EnvironmentSnapshot(_)
            | PortEvent::LocalOffset(_)
            | PortEvent::SystemLocale(_)
            | PortEvent::Terminal { .. }
            | PortEvent::Allocation { .. }
            | PortEvent::Probe { .. }
            | PortEvent::Editor { .. }
            | PortEvent::Dependency { .. }
            | PortEvent::Injected { .. }
            | PortEvent::JavaScriptGate { .. }
            | PortEvent::UvConsent { .. }
            | PortEvent::UvFetch { .. }
            | PortEvent::Preference { .. }
            | PortEvent::Output { .. } => {}
        }
        Ok(())
    }

    fn register_managed_executable(
        &mut self,
        path: &Path,
        roots: &ProductRoots,
    ) -> Result<(), String> {
        if path == skit_runtime::managed_uv_path(&roots.data) {
            self.register(path, ObservationNodeKind::File)?;
        }
        Ok(())
    }

    fn mode(&self, path: &Path, metadata: &fs::Metadata) -> Result<Option<u32>, String> {
        let actual = ObservationNodeKind::from_metadata(metadata);
        let expected = self.expected_kind(path)?;
        if let Some(expected) = expected
            && expected != actual
        {
            return Err(format!(
                "observation mode provenance expected {} at {}, but found {}",
                expected.label(),
                path.display(),
                actual.label()
            ));
        }
        let raw = portable_mode(metadata);
        if expected.is_some()
            || actual == ObservationNodeKind::Symlink
            || raw.is_some_and(|mode| mode & 0o7000 != 0)
        {
            Ok(raw)
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(super) struct TreeRecord {
    pub(super) path: String,
    pub(super) kind: String,
    pub(super) readonly: bool,
    pub(super) mode: Option<u32>,
    pub(super) target: Option<String>,
    pub(super) content: Option<ByteView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub(super) enum ByteView {
    Utf8(String),
    Hex(String),
}

const EMPTY_LOCK_RETRIES: usize = 256;

#[derive(Debug)]
pub(super) enum SandboxError {
    UnsupportedPlatform {
        platform: &'static str,
    },
    Unsupported {
        operation: &'static str,
        path: PathBuf,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    NativeStatus {
        operation: &'static str,
        path: PathBuf,
        status: i32,
    },
    Busy {
        profile: SafeProfileId,
        path: PathBuf,
    },
    InvalidEvidence {
        path: PathBuf,
        reason: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    InjectedFault {
        point: SandboxFaultPoint,
    },
    Dual {
        primary: Box<SandboxError>,
        release: Box<SandboxError>,
    },
}

impl SandboxError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }

    fn invalid(path: &Path, reason: impl Into<String>) -> Self {
        Self::InvalidEvidence {
            path: path.to_path_buf(),
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform { platform } => {
                write!(
                    formatter,
                    "stable walker sandboxes are not supported on {platform}"
                )
            }
            Self::Unsupported {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} walker sandbox path {} because the filesystem operation is unsupported: {source}",
                path.display()
            ),
            Self::NativeStatus {
                operation,
                path,
                status,
            } => write!(
                formatter,
                "could not {operation} walker sandbox path {}: native status 0x{status:08x}",
                path.display()
            ),
            Self::Busy { profile, path } => write!(
                formatter,
                "walker sandbox profile {profile} is busy at {}",
                path.display()
            ),
            Self::InvalidEvidence { path, reason } => write!(
                formatter,
                "walker sandbox evidence is invalid at {}: {reason}",
                path.display()
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} walker sandbox path {}: {source}",
                path.display()
            ),
            Self::InjectedFault { point } => {
                write!(formatter, "injected walker sandbox fault at {point:?}")
            }
            Self::Dual { primary, release } => {
                write!(
                    formatter,
                    "{primary}; sandbox release also failed: {release}"
                )
            }
        }
    }
}

impl std::error::Error for SandboxError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported { source, .. } => Some(source.as_ref()),
            Self::Io { source, .. } => Some(source),
            Self::Dual { primary, .. } => Some(primary),
            Self::UnsupportedPlatform { .. }
            | Self::NativeStatus { .. }
            | Self::Busy { .. }
            | Self::InvalidEvidence { .. }
            | Self::InjectedFault { .. } => None,
        }
    }
}

impl From<SandboxFsError> for SandboxError {
    fn from(value: SandboxFsError) -> Self {
        match value {
            SandboxFsError::UnsupportedPlatform => Self::UnsupportedPlatform {
                platform: std::env::consts::OS,
            },
            SandboxFsError::Unsupported {
                operation,
                path,
                source,
            } => Self::Unsupported {
                operation,
                path,
                source,
            },
            SandboxFsError::NativeStatus {
                operation,
                path,
                status,
            } => Self::NativeStatus {
                operation,
                path,
                status,
            },
            SandboxFsError::Invalid { path, reason } => Self::InvalidEvidence { path, reason },
            SandboxFsError::Io {
                operation: _,
                path,
                source,
            } if sandbox_io_is_link_loop(&source) => Self::InvalidEvidence {
                path,
                reason: "the path is a link or reparse point".to_owned(),
            },
            SandboxFsError::Io {
                operation,
                path,
                source,
            } => Self::Io {
                operation,
                path,
                source,
            },
            SandboxFsError::Dual { primary, release } => Self::Dual {
                primary: Box::new(Self::from(*primary)),
                release: Box::new(Self::from(*release)),
            },
        }
    }
}

fn sandbox_io_is_link_loop(error: &io::Error) -> bool {
    #[cfg(target_os = "linux")]
    {
        error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = error;
        false
    }
}

#[derive(Debug)]
pub(super) enum StableHostPrimaryError {
    Seed(String),
}

impl std::fmt::Display for StableHostPrimaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Seed(error) => {
                write!(formatter, "could not seed the stable walker host: {error}")
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum StableHostError {
    Sandbox(SandboxError),
    Primary {
        primary: StableHostPrimaryError,
        cleanup: Option<SandboxError>,
    },
}

impl std::fmt::Display for StableHostError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sandbox(error) => error.fmt(formatter),
            Self::Primary {
                primary,
                cleanup: None,
            } => primary.fmt(formatter),
            Self::Primary {
                primary,
                cleanup: Some(cleanup),
            } => write!(
                formatter,
                "{primary}; sandbox cleanup also failed: {cleanup}"
            ),
        }
    }
}

impl std::error::Error for StableHostError {}

impl From<SandboxError> for StableHostError {
    fn from(value: SandboxError) -> Self {
        Self::Sandbox(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SandboxFaultPoint {
    ParentInitLockIo,
    ProfileLeaseLockIo,
    CreateNamespaceIo,
    InspectNamespaceIo,
    CreateLeaseDirectoryIo,
    CreateSandboxDirectoryIo,
    CreateSandboxRootIo,
    CreateSandboxChildIo,
    BeforeNamespaceMarker,
    AfterNamespaceMarker,
    BeforeSandboxMarker,
    AfterSandboxMarker,
    RenameOriginal,
    AfterRenameIdentity,
    RemoveChild,
    AfterChildRemoval,
    RemoveMarker,
    RemoveDirectory,
}

#[derive(Debug, Default)]
struct SandboxFaults(Cell<Option<SandboxFaultPoint>>);

impl SandboxFaults {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    const fn at(point: SandboxFaultPoint) -> Self {
        Self(Cell::new(Some(point)))
    }

    fn check(&self, point: SandboxFaultPoint) -> Result<(), SandboxError> {
        if self.take(point) {
            return Err(SandboxError::InjectedFault { point });
        }
        Ok(())
    }

    fn check_io(
        &self,
        point: SandboxFaultPoint,
        operation: &'static str,
        path: &Path,
    ) -> Result<(), SandboxError> {
        if self.take(point) {
            return Err(SandboxError::io(
                operation,
                path,
                io::Error::other(format!("injected {point:?} failure")),
            ));
        }
        Ok(())
    }

    fn take(&self, point: SandboxFaultPoint) -> bool {
        if self.0.get() != Some(point) {
            return false;
        }
        self.0.set(None);
        true
    }
}

fn current_sandbox_platform() -> Result<SandboxPlatform, SandboxError> {
    #[cfg(target_os = "linux")]
    {
        Ok(SandboxPlatform::Linux)
    }
    #[cfg(target_os = "macos")]
    {
        Ok(SandboxPlatform::Macos)
    }
    #[cfg(target_os = "windows")]
    {
        Ok(SandboxPlatform::Windows)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(SandboxError::UnsupportedPlatform {
            platform: std::env::consts::OS,
        })
    }
}

fn default_stable_namespace_root() -> Result<PathBuf, SandboxError> {
    stable_namespace_root_for(current_sandbox_platform()?, &std::env::temp_dir())
}

fn stable_namespace_root_for(
    platform: SandboxPlatform,
    windows_temp: &Path,
) -> Result<PathBuf, SandboxError> {
    match platform {
        SandboxPlatform::Linux => Ok(PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE)),
        SandboxPlatform::Windows => Ok(windows_temp.join(STABLE_SANDBOX_NAMESPACE)),
        SandboxPlatform::Macos => Err(SandboxError::UnsupportedPlatform { platform: "macos" }),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StableSandboxNamespace {
    platform: SandboxPlatform,
    path: PathBuf,
    literal: String,
}

impl StableSandboxNamespace {
    pub(super) fn system() -> Result<Self, SandboxError> {
        Self::explicit(default_stable_namespace_root()?)
    }

    pub(super) fn explicit(path: PathBuf) -> Result<Self, SandboxError> {
        let platform = current_sandbox_platform()?;
        let literal = path
            .to_str()
            .ok_or_else(|| SandboxError::invalid(&path, "the literal path is not valid UTF-8"))?
            .to_owned();
        map_contract_result(&path, validate_stable_namespace_root(platform, &literal))?;
        Ok(Self {
            platform,
            path,
            literal,
        })
    }

    fn into_parts(self) -> (SandboxPlatform, PathBuf, String) {
        (self.platform, self.path, self.literal)
    }
}

fn literal_path(path: &Path) -> Result<String, SandboxError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| SandboxError::invalid(path, "the literal path is not valid UTF-8"))
}

fn map_contract_result<T, E: std::fmt::Display>(
    path: &Path,
    result: Result<T, E>,
) -> Result<T, SandboxError> {
    result.map_err(|error| SandboxError::invalid(path, error.to_string()))
}

#[derive(Debug)]
struct StableSandboxPaths {
    platform: SandboxPlatform,
    profile: SafeProfileId,
    namespace_root: PathBuf,
    init_lock: PathBuf,
    leases_root: PathBuf,
    sandboxes_root: PathBuf,
    lease_path: PathBuf,
    sandbox_root: PathBuf,
    cleanup_root: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    roots: SandboxRoots,
    parent_lock_bytes: Vec<u8>,
    namespace_marker_bytes: Vec<u8>,
    lease_marker_bytes: Vec<u8>,
    sandbox_marker_bytes: Vec<u8>,
}

struct StableContractData {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    roots: SandboxRoots,
    parent_lock_bytes: Vec<u8>,
    namespace_marker_bytes: Vec<u8>,
    lease_marker_bytes: Vec<u8>,
    sandbox_marker_bytes: Vec<u8>,
}

fn derive_stable_contract_data(
    platform: SandboxPlatform,
    namespace_literal: &str,
    profile: &SafeProfileId,
) -> Result<StableContractData, ArtifactError> {
    ParentInitLockMarker::new(platform, namespace_literal.to_owned())
        .and_then(|marker| marker.canonical_bytes())
        .and_then(|parent_lock_bytes| {
            NamespaceMarker::new(platform, namespace_literal.to_owned())
                .and_then(|marker| marker.canonical_bytes())
                .map(|namespace_marker_bytes| (parent_lock_bytes, namespace_marker_bytes))
        })
        .and_then(|(parent_lock_bytes, namespace_marker_bytes)| {
            ProfileLeaseMarker::new(platform, profile.clone(), namespace_literal.to_owned())
                .and_then(|marker| marker.canonical_bytes())
                .map(|lease_marker_bytes| {
                    (
                        parent_lock_bytes,
                        namespace_marker_bytes,
                        lease_marker_bytes,
                    )
                })
        })
        .and_then(
            |(parent_lock_bytes, namespace_marker_bytes, lease_marker_bytes)| {
                SandboxMarker::new(platform, profile.clone(), namespace_literal.to_owned())
                    .and_then(|marker| {
                        #[cfg(any(target_os = "linux", target_os = "windows"))]
                        let roots = marker.roots().clone();
                        marker
                            .canonical_bytes()
                            .map(|sandbox_marker_bytes| StableContractData {
                                #[cfg(any(target_os = "linux", target_os = "windows"))]
                                roots,
                                parent_lock_bytes,
                                namespace_marker_bytes,
                                lease_marker_bytes,
                                sandbox_marker_bytes,
                            })
                    })
            },
        )
}

impl StableSandboxPaths {
    fn new(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
    ) -> Result<Self, SandboxError> {
        Self::new_with_contract_builder(namespace, profile, derive_stable_contract_data)
    }

    fn new_with_contract_builder<E: std::fmt::Display>(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
        derive: impl FnOnce(SandboxPlatform, &str, &SafeProfileId) -> Result<StableContractData, E>,
    ) -> Result<Self, SandboxError> {
        let (platform, namespace_root, namespace_literal) = namespace.into_parts();
        let parent = namespace_root
            .parent()
            .expect("a validated absolute namespace root has a parent");
        let init_lock = parent.join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
        let leases_root = namespace_root.join(LEASE_DIRECTORY);
        let sandboxes_root = namespace_root.join(SANDBOX_DIRECTORY);
        let lease_path = namespace_root.join(profile_lease_path(&profile));
        let sandbox_root = namespace_root.join(profile_sandbox_path(&profile));
        let cleanup_root = namespace_root.join(profile_cleanup_path(&profile));
        let contracts = map_contract_result(
            &namespace_root,
            derive(platform, &namespace_literal, &profile),
        )?;
        let StableContractData {
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            roots,
            parent_lock_bytes,
            namespace_marker_bytes,
            lease_marker_bytes,
            sandbox_marker_bytes,
        } = contracts;
        Ok(Self {
            platform,
            profile,
            namespace_root,
            init_lock,
            leases_root,
            sandboxes_root,
            lease_path,
            sandbox_root,
            cleanup_root,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            roots,
            parent_lock_bytes,
            namespace_marker_bytes,
            lease_marker_bytes,
            sandbox_marker_bytes,
        })
    }

    fn parent_lock_bytes(&self) -> &[u8] {
        &self.parent_lock_bytes
    }

    fn namespace_marker_bytes(&self) -> &[u8] {
        &self.namespace_marker_bytes
    }

    fn lease_marker_bytes(&self) -> &[u8] {
        &self.lease_marker_bytes
    }

    fn sandbox_marker_bytes(&self) -> &[u8] {
        &self.sandbox_marker_bytes
    }

    fn namespace_name(&self) -> ChildName {
        ChildName::literal(STABLE_SANDBOX_NAMESPACE)
    }

    fn init_lock_name(&self) -> ChildName {
        ChildName::new(
            self.init_lock
                .file_name()
                .expect("the typed init lock has a filename"),
        )
        .expect("the typed init lock filename is one component")
    }

    fn leases_name(&self) -> ChildName {
        ChildName::literal(LEASE_DIRECTORY)
    }

    fn sandboxes_name(&self) -> ChildName {
        ChildName::literal(SANDBOX_DIRECTORY)
    }

    fn lease_name(&self) -> ChildName {
        ChildName::new(
            self.lease_path
                .file_name()
                .expect("the typed lease has a filename"),
        )
        .expect("the typed lease filename is one component")
    }

    fn sandbox_name(&self) -> ChildName {
        ChildName::new(self.profile.as_str()).expect("a safe profile is one filename")
    }

    fn cleanup_name(&self) -> ChildName {
        ChildName::new(
            self.cleanup_root
                .file_name()
                .expect("the typed cleanup root has a filename"),
        )
        .expect("the typed cleanup filename is one component")
    }

    fn namespace_marker_name(&self) -> ChildName {
        ChildName::literal(NAMESPACE_MARKER_FILE)
    }

    fn sandbox_marker_name(&self) -> ChildName {
        ChildName::literal(SANDBOX_MARKER_FILE)
    }

    fn sandbox_child_names(&self) -> [ChildName; 7] {
        [
            ChildName::literal("data"),
            ChildName::literal("state"),
            ChildName::literal("config"),
            ChildName::literal("home"),
            ChildName::literal("cwd"),
            ChildName::literal("external"),
            ChildName::literal("system-temp"),
        ]
    }
}

fn ticket_named(
    directory: &PinnedDirectory,
    name: &ChildName,
) -> Result<Option<EntryTicket>, SandboxError> {
    Ok(directory
        .tickets()
        .map_err(SandboxError::from)?
        .into_iter()
        .find(|ticket| ticket.name() == name))
}

fn require_ticket(
    directory: &PinnedDirectory,
    name: &ChildName,
    expected_kind: SandboxNodeKind,
    expected_identity: super::tui_real_sandbox_fs::NodeIdentity,
) -> Result<(), SandboxError> {
    let ticket = ticket_named(directory, name)?.ok_or_else(|| {
        SandboxError::invalid(
            &directory.path().join(name.as_os_str()),
            "the retained directory entry is missing",
        )
    })?;
    if ticket.kind() != expected_kind || ticket.identity() != expected_identity {
        return Err(SandboxError::invalid(
            &directory.path().join(name.as_os_str()),
            "the retained directory entry changed identity or kind",
        ));
    }
    Ok(())
}

fn publish_relative_marker(
    directory: &PinnedDirectory,
    name: &ChildName,
    bytes: &[u8],
) -> Result<PinnedFile, SandboxError> {
    let marker = directory.create_file(name).map_err(SandboxError::from)?;
    marker
        .publish_bytes(directory, name, bytes)
        .map_err(SandboxError::from)?;
    Ok(marker)
}

fn validate_relative_marker(
    directory: &PinnedDirectory,
    name: &ChildName,
    expected: &[u8],
) -> Result<PinnedFile, SandboxError> {
    validate_relative_marker_with_hook(directory, name, expected, |_, _, _| {})
}

fn validate_relative_marker_with_hook(
    directory: &PinnedDirectory,
    name: &ChildName,
    expected: &[u8],
    after_read: impl FnOnce(&PinnedDirectory, &PinnedFile, &ChildName),
) -> Result<PinnedFile, SandboxError> {
    let marker = match directory.open_file(name, false) {
        Ok(marker) => marker,
        Err(SandboxFsError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
            return Err(SandboxError::invalid(
                &directory.path().join(name.as_os_str()),
                "the ownership marker is missing",
            ));
        }
        Err(error) => return Err(SandboxError::from(error)),
    };
    let actual = marker
        .read_all(directory, name)
        .map_err(SandboxError::from)?;
    if actual != expected {
        return Err(SandboxError::invalid(
            marker.path(),
            "the marker bytes are invalid",
        ));
    }
    after_read(directory, &marker, name);
    require_ticket(
        directory,
        name,
        SandboxNodeKind::RegularFile,
        marker.identity(),
    )?;
    Ok(marker)
}

fn require_exact_names(
    directory: &PinnedDirectory,
    expected: &[ChildName],
) -> Result<(), SandboxError> {
    let mut actual = directory
        .tickets()
        .map_err(SandboxError::from)?
        .into_iter()
        .map(|ticket| ticket.name().clone())
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = expected.to_vec();
    expected.sort();
    if actual != expected {
        return Err(SandboxError::invalid(
            directory.path(),
            "the directory has unexpected or missing children",
        ));
    }
    Ok(())
}

fn create_relative_directory_with_fault(
    parent: &PinnedDirectory,
    name: &ChildName,
    path: &Path,
    operation: &'static str,
    faults: &SandboxFaults,
    point: SandboxFaultPoint,
) -> Result<PinnedDirectory, SandboxError> {
    if faults.take(point) {
        return Err(SandboxError::io(
            operation,
            path,
            io::Error::other(format!("injected {point:?} failure")),
        ));
    }
    parent.create_directory(name).map_err(SandboxError::from)
}

fn dual_error(primary: SandboxError, release: SandboxFsError) -> SandboxError {
    combine_sandbox_release(Err(primary), Err(SandboxError::from(release)))
        .expect_err("two failures combine into one typed dual error")
}

fn combine_sandbox_release(
    primary: Result<(), SandboxError>,
    release: Result<(), SandboxError>,
) -> Result<(), SandboxError> {
    combine_release(primary, release, |primary, release| SandboxError::Dual {
        primary: Box::new(primary),
        release: Box::new(release),
    })
}

fn release_failed_initialization(
    guard: InitializationGuard,
    primary: SandboxError,
) -> SandboxError {
    match release_initialization_guard(guard) {
        Ok(_) => primary,
        Err(release) => dual_error(primary, release),
    }
}

fn release_initialization_guard(
    guard: InitializationGuard,
) -> Result<PinnedDirectory, SandboxFsError> {
    guard.release(|_| Ok(()))
}

fn acquire_initialization_guard(
    paths: &StableSandboxPaths,
    faults: &SandboxFaults,
) -> Result<InitializationGuard, SandboxError> {
    acquire_initialization_guard_with_hooks(paths, faults, |_| {}, release_initialization_guard)
}

fn acquire_initialization_guard_with_hooks(
    paths: &StableSandboxPaths,
    faults: &SandboxFaults,
    after_first_read: impl FnOnce(&InitializationGuard),
    release_empty: impl FnOnce(InitializationGuard) -> Result<PinnedDirectory, SandboxFsError>,
) -> Result<InitializationGuard, SandboxError> {
    let parent_path = paths
        .namespace_root
        .parent()
        .expect("the validated namespace has a parent");
    let expected = paths.parent_lock_bytes();
    let mut after_first_read = Some(after_first_read);
    let mut release_empty = Some(release_empty);
    for _ in 0..EMPTY_LOCK_RETRIES {
        let guard = InitializationGuard::acquire(parent_path, paths.init_lock_name())
            .map_err(SandboxError::from)?;
        let existing = !guard.created();
        if let Err(error) = faults.check_io(
            SandboxFaultPoint::ParentInitLockIo,
            "lock",
            &paths.init_lock,
        ) {
            return Err(release_failed_initialization(guard, error));
        }
        let evidence = if guard.created() {
            guard
                .file()
                .publish_bytes(guard.parent(), guard.name(), expected)
                .map_err(SandboxError::from)
        } else {
            guard
                .file()
                .read_all(guard.parent(), guard.name())
                .map_err(SandboxError::from)
                .and_then(|actual| {
                    if actual == expected || actual.is_empty() {
                        Ok(())
                    } else {
                        Err(SandboxError::invalid(
                            &paths.init_lock,
                            "the parent initialization lock marker bytes are invalid",
                        ))
                    }
                })
        };
        if let Err(error) = evidence {
            return Err(release_failed_initialization(guard, error));
        }
        if existing && let Some(after_first_read) = after_first_read.take() {
            after_first_read(&guard);
        }
        let actual = match guard.file().read_all(guard.parent(), guard.name()) {
            Ok(actual) => actual,
            Err(error) => {
                return Err(release_failed_initialization(
                    guard,
                    SandboxError::from(error),
                ));
            }
        };
        if actual == expected {
            return Ok(guard);
        }
        let release = if let Some(release_empty) = release_empty.take() {
            release_empty(guard)
        } else {
            release_initialization_guard(guard)
        };
        match release {
            Ok(_) => std::thread::sleep(Duration::from_millis(1)),
            Err(error) => return Err(SandboxError::from(error)),
        }
    }
    Err(SandboxError::invalid(
        &paths.init_lock,
        "the parent initialization lock marker bytes are invalid",
    ))
}

fn validate_namespace_opened(
    paths: &StableSandboxPaths,
    parent: &PinnedDirectory,
    namespace: &PinnedDirectory,
    leases: &PinnedDirectory,
    sandboxes: &PinnedDirectory,
) -> Result<PinnedFile, SandboxError> {
    validate_namespace_opened_with_hook(
        paths,
        parent,
        namespace,
        leases,
        sandboxes,
        |_, _, _, _| {},
    )
}

fn validate_namespace_opened_with_hook(
    paths: &StableSandboxPaths,
    parent: &PinnedDirectory,
    namespace: &PinnedDirectory,
    leases: &PinnedDirectory,
    sandboxes: &PinnedDirectory,
    after_opened_verify: impl FnOnce(
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedDirectory,
    ),
) -> Result<PinnedFile, SandboxError> {
    NamespaceHandles::verify_opened(
        parent,
        namespace,
        leases,
        sandboxes,
        &paths.namespace_name(),
        &paths.leases_name(),
        &paths.sandboxes_name(),
    )
    .map_err(SandboxError::from)?;
    after_opened_verify(parent, namespace, leases, sandboxes);
    let marker = validate_relative_marker(
        namespace,
        &paths.namespace_marker_name(),
        paths.namespace_marker_bytes(),
    )?;
    require_ticket(
        namespace,
        &paths.leases_name(),
        SandboxNodeKind::Directory,
        leases.identity(),
    )?;
    require_ticket(
        namespace,
        &paths.sandboxes_name(),
        SandboxNodeKind::Directory,
        sandboxes.identity(),
    )?;
    require_exact_names(
        namespace,
        &[
            paths.namespace_marker_name(),
            paths.leases_name(),
            paths.sandboxes_name(),
        ],
    )?;
    Ok(marker)
}

fn open_or_create_namespace_locked_with_hook(
    paths: &StableSandboxPaths,
    guard: &InitializationGuard,
    faults: &SandboxFaults,
    before_marker_publish: impl FnOnce(&PinnedDirectory, &ChildName),
) -> Result<
    (
        PinnedDirectory,
        PinnedDirectory,
        PinnedDirectory,
        PinnedFile,
    ),
    SandboxError,
> {
    if faults.take(SandboxFaultPoint::InspectNamespaceIo) {
        return Err(SandboxError::io(
            "inspect namespace",
            &paths.namespace_root,
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected InspectNamespaceIo failure",
            ),
        ));
    }
    if ticket_named(guard.parent(), &paths.namespace_name())?.is_some() {
        let namespace = guard
            .parent()
            .open_directory(&paths.namespace_name())
            .map_err(SandboxError::from)?;
        let leases = namespace
            .open_directory(&paths.leases_name())
            .map_err(SandboxError::from)?;
        let sandboxes = namespace
            .open_directory(&paths.sandboxes_name())
            .map_err(SandboxError::from)?;
        let marker =
            validate_namespace_opened(paths, guard.parent(), &namespace, &leases, &sandboxes)?;
        return Ok((namespace, leases, sandboxes, marker));
    }

    let namespace = create_relative_directory_with_fault(
        guard.parent(),
        &paths.namespace_name(),
        &paths.namespace_root,
        "create namespace",
        faults,
        SandboxFaultPoint::CreateNamespaceIo,
    )?;
    guard.parent().sync().map_err(SandboxError::from)?;
    let leases = create_relative_directory_with_fault(
        &namespace,
        &paths.leases_name(),
        &paths.leases_root,
        "create lease directory",
        faults,
        SandboxFaultPoint::CreateLeaseDirectoryIo,
    )?;
    let sandboxes = create_relative_directory_with_fault(
        &namespace,
        &paths.sandboxes_name(),
        &paths.sandboxes_root,
        "create sandbox directory",
        faults,
        SandboxFaultPoint::CreateSandboxDirectoryIo,
    )?;
    leases.sync().map_err(SandboxError::from)?;
    sandboxes.sync().map_err(SandboxError::from)?;
    namespace.sync().map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::BeforeNamespaceMarker)?;
    let marker_name = paths.namespace_marker_name();
    before_marker_publish(&namespace, &marker_name);
    let marker = publish_relative_marker(&namespace, &marker_name, paths.namespace_marker_bytes())?;
    namespace.sync().map_err(SandboxError::from)?;
    validate_namespace_opened(paths, guard.parent(), &namespace, &leases, &sandboxes)?;
    faults.check(SandboxFaultPoint::AfterNamespaceMarker)?;
    Ok((namespace, leases, sandboxes, marker))
}

fn initialize_namespace_retained(
    paths: &StableSandboxPaths,
    faults: &SandboxFaults,
) -> Result<(NamespaceHandles, PinnedFile), SandboxError> {
    initialize_namespace_retained_with_hooks(
        paths,
        faults,
        no_namespace_marker_publish_hook,
        no_before_namespace_release_hook,
        no_after_namespace_release_hook,
    )
}

fn no_namespace_marker_publish_hook(_: &PinnedDirectory, _: &ChildName) {}

fn no_before_namespace_release_hook(
    _: &InitializationGuard,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedFile,
) {
}

fn no_after_namespace_release_hook(
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedFile,
) {
}

#[cfg(target_os = "linux")]
fn initialize_namespace_retained_with_hook(
    paths: &StableSandboxPaths,
    faults: &SandboxFaults,
    before_release: impl FnOnce(
        &InitializationGuard,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedFile,
    ),
) -> Result<(NamespaceHandles, PinnedFile), SandboxError> {
    initialize_namespace_retained_with_hooks(
        paths,
        faults,
        no_namespace_marker_publish_hook,
        before_release,
        no_after_namespace_release_hook,
    )
}

fn initialize_namespace_retained_with_hooks(
    paths: &StableSandboxPaths,
    faults: &SandboxFaults,
    before_marker_publish: impl FnOnce(&PinnedDirectory, &ChildName),
    before_release: impl FnOnce(
        &InitializationGuard,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedFile,
    ),
    after_release: impl FnOnce(
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedDirectory,
        &PinnedFile,
    ),
) -> Result<(NamespaceHandles, PinnedFile), SandboxError> {
    let guard = acquire_initialization_guard(paths, faults)?;
    let initialized =
        open_or_create_namespace_locked_with_hook(paths, &guard, faults, before_marker_publish);
    let (namespace, leases, sandboxes, marker) = match initialized {
        Ok(initialized) => initialized,
        Err(primary) => return Err(release_failed_initialization(guard, primary)),
    };
    before_release(&guard, &namespace, &leases, &sandboxes, &marker);
    let expected_marker_identity = marker.identity();
    let parent = guard
        .release(|guard| {
            NamespaceHandles::verify_opened(
                guard.parent(),
                &namespace,
                &leases,
                &sandboxes,
                &paths.namespace_name(),
                &paths.leases_name(),
                &paths.sandboxes_name(),
            )?;
            marker.verify_handle()?;
            marker.verify_at(&namespace, &paths.namespace_marker_name())
        })
        .map_err(SandboxError::from)?;
    after_release(&parent, &namespace, &leases, &sandboxes, &marker);
    let handles = NamespaceHandles::new(parent, namespace, leases, sandboxes);
    handles
        .verify(
            &paths.namespace_name(),
            &paths.leases_name(),
            &paths.sandboxes_name(),
        )
        .map_err(SandboxError::from)?;
    let marker = validate_relative_marker(
        handles.namespace(),
        &paths.namespace_marker_name(),
        paths.namespace_marker_bytes(),
    )?;
    if marker.identity() != expected_marker_identity {
        return Err(SandboxError::invalid(
            marker.path(),
            "the namespace marker identity changed after initialization release",
        ));
    }
    Ok((handles, marker))
}

fn release_failed_profile_lease(
    lease: ProfileLease,
    handles: &NamespaceHandles,
    primary: SandboxError,
    release: &mut impl FnMut(ProfileLease, &PinnedDirectory) -> Result<(), SandboxFsError>,
) -> SandboxError {
    match release(lease, handles.leases()) {
        Ok(()) => primary,
        Err(release) => dual_error(primary, release),
    }
}

fn acquire_profile_lease_retained(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
) -> Result<ProfileLease, SandboxError> {
    acquire_profile_lease_retained_with_hooks(
        paths,
        handles,
        faults,
        no_profile_lease_hook,
        publish_profile_lease_marker,
        no_profile_lease_hook,
        release_profile_lease,
    )
}

fn no_profile_lease_hook(_: &ProfileLease, _: &PinnedDirectory) {}

fn publish_profile_lease_marker(
    file: &PinnedFile,
    parent: &PinnedDirectory,
    name: &ChildName,
    bytes: &[u8],
) -> Result<(), SandboxFsError> {
    file.publish_bytes(parent, name, bytes)
}

fn release_profile_lease(
    lease: ProfileLease,
    leases: &PinnedDirectory,
) -> Result<(), SandboxFsError> {
    lease.release(leases)
}

fn acquire_profile_lease_retained_with_hooks(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
    mut after_try_lock: impl FnMut(&ProfileLease, &PinnedDirectory),
    mut publish: impl FnMut(
        &PinnedFile,
        &PinnedDirectory,
        &ChildName,
        &[u8],
    ) -> Result<(), SandboxFsError>,
    mut after_publish: impl FnMut(&ProfileLease, &PinnedDirectory),
    mut release: impl FnMut(ProfileLease, &PinnedDirectory) -> Result<(), SandboxFsError>,
) -> Result<ProfileLease, SandboxError> {
    let expected = paths.lease_marker_bytes();
    for _ in 0..EMPTY_LOCK_RETRIES {
        let (mut lease, created) =
            ProfileLease::open(handles.leases(), paths.lease_name()).map_err(SandboxError::from)?;
        if let Err(primary) = faults.check_io(
            SandboxFaultPoint::ProfileLeaseLockIo,
            "lock",
            &paths.lease_path,
        ) {
            return Err(release_failed_profile_lease(
                lease,
                handles,
                primary,
                &mut release,
            ));
        }
        let lock_result = map_profile_lock_result(lease.try_lock(), paths);
        if let Err(primary) = lock_result {
            return Err(release_failed_profile_lease(
                lease,
                handles,
                primary,
                &mut release,
            ));
        }
        after_try_lock(&lease, handles.leases());
        if let Err(error) = lease.validate(handles.leases()) {
            return Err(release_failed_profile_lease(
                lease,
                handles,
                SandboxError::from(error),
                &mut release,
            ));
        }
        let evidence = if created {
            publish(
                lease.file(),
                handles.leases(),
                &paths.lease_name(),
                expected,
            )
        } else {
            Ok(())
        };
        if let Err(error) = evidence {
            return Err(release_failed_profile_lease(
                lease,
                handles,
                SandboxError::from(error),
                &mut release,
            ));
        }
        after_publish(&lease, handles.leases());
        let actual = match lease.file().read_all(handles.leases(), &paths.lease_name()) {
            Ok(actual) => actual,
            Err(error) => {
                return Err(release_failed_profile_lease(
                    lease,
                    handles,
                    SandboxError::from(error),
                    &mut release,
                ));
            }
        };
        if actual == expected {
            return Ok(lease);
        }
        if actual.is_empty() {
            release(lease, handles.leases()).map_err(SandboxError::from)?;
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        return Err(release_failed_profile_lease(
            lease,
            handles,
            SandboxError::invalid(
                &paths.lease_path,
                "the profile lease marker bytes are invalid",
            ),
            &mut release,
        ));
    }
    Err(SandboxError::invalid(
        &paths.lease_path,
        "the profile lease marker bytes are invalid",
    ))
}

fn map_profile_lock_result(
    result: Result<(), fs::TryLockError>,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    match result {
        Ok(()) => Ok(()),
        Err(fs::TryLockError::WouldBlock) => Err(SandboxError::Busy {
            profile: paths.profile.clone(),
            path: paths.lease_path.clone(),
        }),
        Err(fs::TryLockError::Error(error)) => {
            Err(SandboxError::io("lock", &paths.lease_path, error))
        }
    }
}

#[derive(Debug)]
enum Evidence<T> {
    Absent,
    Valid(T),
    Invalid { reason: String },
}

impl<T> Evidence<T> {
    const fn state(&self) -> SandboxEvidenceState {
        match self {
            Self::Absent => SandboxEvidenceState::Absent,
            Self::Valid(_) => SandboxEvidenceState::Valid,
            Self::Invalid { .. } => SandboxEvidenceState::Invalid,
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            Self::Invalid { reason } => Some(reason),
            Self::Absent | Self::Valid(_) => None,
        }
    }

    fn into_valid(self) -> Option<T> {
        match self {
            Self::Valid(value) => Some(value),
            Self::Absent | Self::Invalid { .. } => None,
        }
    }
}

#[derive(Debug)]
struct CandidateParts {
    root: PinnedDirectory,
    marker: PinnedFile,
    children: BTreeMap<ChildName, PinnedDirectory>,
    tickets: Vec<EntryTicket>,
}

fn open_candidate_parts(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<CandidateParts, SandboxError> {
    open_candidate_parts_with_hooks(
        parent,
        name,
        paths,
        no_candidate_tickets_hook,
        no_candidate_root_hook,
    )
}

fn no_candidate_tickets_hook(_: &PinnedDirectory, _: &[EntryTicket]) {}

fn no_candidate_root_hook(_: &PinnedDirectory, _: &PinnedDirectory, _: &ChildName) {}

fn open_candidate_parts_with_hooks(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
    after_tickets: impl FnOnce(&PinnedDirectory, &[EntryTicket]),
    before_final_root_ticket: impl FnOnce(&PinnedDirectory, &PinnedDirectory, &ChildName),
) -> Result<CandidateParts, SandboxError> {
    let root = parent.open_directory(name).map_err(SandboxError::from)?;
    require_ticket(parent, name, SandboxNodeKind::Directory, root.identity())?;
    let tickets = root.tickets().map_err(SandboxError::from)?;
    after_tickets(&root, &tickets);
    let mut children = BTreeMap::new();
    for child_name in paths.sandbox_child_names() {
        let ticket = tickets.iter().find(|ticket| ticket.name() == &child_name);
        let Some(ticket) = ticket else {
            continue;
        };
        if ticket.kind() != SandboxNodeKind::Directory {
            return Err(SandboxError::invalid(
                &root.path().join(child_name.as_os_str()),
                "the sandbox child root is not an ordinary directory",
            ));
        }
        let child = root
            .open_directory(&child_name)
            .map_err(SandboxError::from)?;
        if child.identity() != ticket.identity() {
            return Err(SandboxError::invalid(
                child.path(),
                "the sandbox child root changed after enumeration",
            ));
        }
        children.insert(child_name, child);
    }
    let marker = validate_relative_marker(
        &root,
        &paths.sandbox_marker_name(),
        paths.sandbox_marker_bytes(),
    )?;
    root.verify_at(parent, name).map_err(SandboxError::from)?;
    marker
        .verify_at(&root, &paths.sandbox_marker_name())
        .map_err(SandboxError::from)?;
    before_final_root_ticket(parent, &root, name);
    require_ticket(parent, name, SandboxNodeKind::Directory, root.identity())?;
    Ok(CandidateParts {
        root,
        marker,
        children,
        tickets,
    })
}

fn validate_original_candidate(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    let CandidateParts {
        root,
        marker,
        children,
        tickets: _,
    } = open_candidate_parts(parent, name, paths)?;
    let expected = paths.sandbox_child_names();
    let sandbox = ValidatedOriginalSandbox::new_original(root, marker, expected.clone(), children)
        .map_err(SandboxError::from)?;
    let mut allowed = expected.to_vec();
    allowed.push(paths.sandbox_marker_name());
    require_exact_names(sandbox.root(), &allowed)?;
    Ok(sandbox)
}

fn validate_cleanup_candidate(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<ValidatedCleanupSandbox, SandboxError> {
    let CandidateParts {
        root,
        marker,
        children,
        tickets,
    } = open_candidate_parts(parent, name, paths)?;
    let mut allowed = paths.sandbox_child_names().to_vec();
    allowed.push(paths.sandbox_marker_name());
    if tickets
        .iter()
        .any(|ticket| !allowed.iter().any(|allowed| allowed == ticket.name()))
    {
        return Err(SandboxError::invalid(
            root.path(),
            "the cleanup directory has an unexpected child",
        ));
    }
    Ok(ValidatedCleanupSandbox::new_cleanup(root, marker, children))
}

fn classify_retained_sandbox<T>(
    parent: &PinnedDirectory,
    name: &ChildName,
    validate: impl FnOnce(&PinnedDirectory, &ChildName) -> Result<T, SandboxError>,
) -> Result<Evidence<T>, SandboxError> {
    if ticket_named(parent, name)?.is_none() {
        return Ok(Evidence::Absent);
    }
    match validate(parent, name) {
        Ok(sandbox) => Ok(Evidence::Valid(sandbox)),
        Err(error @ SandboxError::InvalidEvidence { .. }) => Ok(Evidence::Invalid {
            reason: error.to_string(),
        }),
        Err(error) => Err(error),
    }
}

fn create_fresh_sandbox_retained(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    create_fresh_sandbox_retained_with_hooks(
        paths,
        handles,
        faults,
        no_fresh_sandbox_before_marker_hook,
        no_fresh_sandbox_after_marker_hook,
    )
}

fn no_fresh_sandbox_before_marker_hook(_: &PinnedDirectory, _: &ChildName) {}

fn no_fresh_sandbox_after_marker_hook(_: &PinnedDirectory) {}

fn create_fresh_sandbox_retained_with_hooks(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
    before_marker_publish: impl FnOnce(&PinnedDirectory, &ChildName),
    after_marker_publish: impl FnOnce(&PinnedDirectory),
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    let root = create_relative_directory_with_fault(
        handles.sandboxes(),
        &paths.sandbox_name(),
        &paths.sandbox_root,
        "create sandbox",
        faults,
        SandboxFaultPoint::CreateSandboxRootIo,
    )?;
    let mut children = BTreeMap::new();
    for child_name in paths.sandbox_child_names() {
        let child_path = root.path().join(child_name.as_os_str());
        let child = create_relative_directory_with_fault(
            &root,
            &child_name,
            &child_path,
            "create sandbox child",
            faults,
            SandboxFaultPoint::CreateSandboxChildIo,
        )?;
        child.sync().map_err(SandboxError::from)?;
        children.insert(child_name, child);
    }
    root.sync().map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::BeforeSandboxMarker)?;
    let marker_name = paths.sandbox_marker_name();
    before_marker_publish(&root, &marker_name);
    let marker = publish_relative_marker(&root, &marker_name, paths.sandbox_marker_bytes())?;
    root.sync().map_err(SandboxError::from)?;
    after_marker_publish(&root);
    let sandbox =
        ValidatedOriginalSandbox::new_original(root, marker, paths.sandbox_child_names(), children)
            .expect("seven created child roots produce original sandbox evidence");
    validate_same_original(&sandbox, handles.sandboxes(), &paths.sandbox_name(), paths)?;
    faults.check(SandboxFaultPoint::AfterSandboxMarker)?;
    Ok(sandbox)
}

fn validate_same_candidate_core(
    root: &PinnedDirectory,
    marker: &PinnedFile,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    root.verify_handle().map_err(SandboxError::from)?;
    root.verify_at(parent, name).map_err(SandboxError::from)?;
    require_ticket(parent, name, SandboxNodeKind::Directory, root.identity())?;
    marker.verify_handle().map_err(SandboxError::from)?;
    marker
        .verify_at(root, &paths.sandbox_marker_name())
        .map_err(SandboxError::from)?;
    let marker_bytes = marker
        .read_all(root, &paths.sandbox_marker_name())
        .map_err(SandboxError::from)?;
    if marker_bytes != paths.sandbox_marker_bytes() {
        return Err(SandboxError::invalid(
            marker.path(),
            "the marker bytes are invalid",
        ));
    }
    Ok(())
}

fn validate_same_original(
    sandbox: &ValidatedOriginalSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    validate_same_original_with_hook(sandbox, parent, name, paths, no_candidate_child_hook)
}

fn no_candidate_child_hook(_: &PinnedDirectory, _: &ChildName, _: &PinnedDirectory) {}

fn validate_same_original_with_hook(
    sandbox: &ValidatedOriginalSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
    mut after_child_verify: impl FnMut(&PinnedDirectory, &ChildName, &PinnedDirectory),
) -> Result<(), SandboxError> {
    validate_same_candidate_core(sandbox.root(), sandbox.marker(), parent, name, paths)?;
    for (child_name, child) in sandbox.children() {
        child.verify_handle().map_err(SandboxError::from)?;
        child
            .verify_at(sandbox.root(), child_name)
            .map_err(SandboxError::from)?;
        after_child_verify(sandbox.root(), child_name, child);
        require_ticket(
            sandbox.root(),
            child_name,
            SandboxNodeKind::Directory,
            child.identity(),
        )?;
    }
    let mut allowed = sandbox
        .children()
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    allowed.push(paths.sandbox_marker_name());
    require_exact_names(sandbox.root(), &allowed)
}

fn validate_same_cleanup(
    sandbox: &ValidatedCleanupSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    validate_same_cleanup_with_hook(sandbox, parent, name, paths, no_candidate_child_hook)
}

fn validate_same_cleanup_with_hook(
    sandbox: &ValidatedCleanupSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
    mut after_child_verify: impl FnMut(&PinnedDirectory, &ChildName, &PinnedDirectory),
) -> Result<(), SandboxError> {
    validate_same_candidate_core(sandbox.root(), sandbox.marker(), parent, name, paths)?;
    for (child_name, child) in sandbox.children() {
        child.verify_handle().map_err(SandboxError::from)?;
        child
            .verify_at(sandbox.root(), child_name)
            .map_err(SandboxError::from)?;
        after_child_verify(sandbox.root(), child_name, child);
        require_ticket(
            sandbox.root(),
            child_name,
            SandboxNodeKind::Directory,
            child.identity(),
        )?;
    }
    let mut allowed = sandbox.children().keys().cloned().collect::<Vec<_>>();
    allowed.push(paths.sandbox_marker_name());
    require_exact_names(sandbox.root(), &allowed)
}

fn remove_cleanup_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    remove_cleanup_sandbox_with_hooks(
        paths,
        handles,
        sandbox,
        faults,
        CleanupHooks {
            after_plan: no_cleanup_plan_hook,
            after_children: no_cleanup_marker_hook,
            after_marker_read: no_cleanup_marker_hook,
            before_root_ticket: no_cleanup_root_hook,
        },
    )
}

fn no_cleanup_plan_hook(_: &PinnedDirectory) {}

fn no_cleanup_marker_hook(_: &PinnedDirectory, _: &PinnedFile) {}

fn no_cleanup_root_hook(_: &PinnedDirectory, _: &PinnedDirectory, _: &ChildName) {}

struct CleanupHooks<AfterPlan, AfterChildren, AfterMarkerRead, BeforeRootTicket> {
    after_plan: AfterPlan,
    after_children: AfterChildren,
    after_marker_read: AfterMarkerRead,
    before_root_ticket: BeforeRootTicket,
}

fn remove_cleanup_sandbox_with_hooks<AfterPlan, AfterChildren, AfterMarkerRead, BeforeRootTicket>(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
    hooks: CleanupHooks<AfterPlan, AfterChildren, AfterMarkerRead, BeforeRootTicket>,
) -> Result<(), SandboxError>
where
    AfterPlan: FnOnce(&PinnedDirectory),
    AfterChildren: FnOnce(&PinnedDirectory, &PinnedFile),
    AfterMarkerRead: FnOnce(&PinnedDirectory, &PinnedFile),
    BeforeRootTicket: FnOnce(&PinnedDirectory, &PinnedDirectory, &ChildName),
{
    let CleanupHooks {
        after_plan,
        after_children,
        after_marker_read,
        before_root_ticket,
    } = hooks;
    let cleanup_name = paths.cleanup_name();
    let sandbox = sandbox.into_cleanup_plan();
    after_plan(sandbox.root());
    let marker_name = paths.sandbox_marker_name();
    for ticket in sandbox.root().tickets().map_err(SandboxError::from)? {
        if ticket.name() == &marker_name {
            continue;
        }
        let expected = sandbox.children().get(ticket.name()).ok_or_else(|| {
            SandboxError::invalid(
                &sandbox.root().path().join(ticket.name().as_os_str()),
                "the cleanup child is not one retained sandbox root",
            )
        })?;
        if ticket.kind() != SandboxNodeKind::Directory || ticket.identity() != *expected {
            return Err(SandboxError::invalid(
                &sandbox.root().path().join(ticket.name().as_os_str()),
                "the cleanup child changed after validation",
            ));
        }
        faults.check(SandboxFaultPoint::RemoveChild)?;
        sandbox
            .root()
            .remove_tree(ticket)
            .map_err(SandboxError::from)?;
        faults.check(SandboxFaultPoint::AfterChildRemoval)?;
    }
    require_exact_names(sandbox.root(), std::slice::from_ref(&marker_name))?;
    after_children(sandbox.root(), sandbox.marker());
    sandbox
        .marker()
        .verify_handle()
        .map_err(SandboxError::from)?;
    let marker_bytes = sandbox
        .marker()
        .read_all(sandbox.root(), &marker_name)
        .map_err(SandboxError::from)?;
    if marker_bytes != paths.sandbox_marker_bytes() {
        return Err(SandboxError::invalid(
            sandbox.marker().path(),
            "the marker bytes are invalid",
        ));
    }
    after_marker_read(sandbox.root(), sandbox.marker());
    let marker_ticket = ticket_named(sandbox.root(), &marker_name)?.ok_or_else(|| {
        SandboxError::invalid(sandbox.marker().path(), "the sandbox marker is missing")
    })?;
    if marker_ticket.kind() != SandboxNodeKind::RegularFile
        || marker_ticket.identity() != sandbox.marker().identity()
    {
        return Err(SandboxError::invalid(
            sandbox.marker().path(),
            "the sandbox marker changed before removal",
        ));
    }
    faults.check(SandboxFaultPoint::RemoveMarker)?;
    let root = sandbox.into_root();
    root.remove_ticket(marker_ticket)
        .map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::RemoveDirectory)?;
    root.verify_handle().map_err(SandboxError::from)?;
    before_root_ticket(handles.sandboxes(), &root, &cleanup_name);
    let root_ticket = ticket_named(handles.sandboxes(), &cleanup_name)?
        .ok_or_else(|| SandboxError::invalid(&paths.cleanup_root, "the cleanup root is missing"))?;
    if root_ticket.kind() != SandboxNodeKind::Directory || root_ticket.identity() != root.identity()
    {
        return Err(SandboxError::invalid(
            &paths.cleanup_root,
            "the cleanup root changed before removal",
        ));
    }
    drop(root);
    handles
        .sandboxes()
        .remove_ticket(root_ticket)
        .map_err(SandboxError::from)
}

fn quarantine_original_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    source_name: ChildName,
    sandbox: ValidatedOriginalSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    quarantine_original_sandbox_with_hook(
        paths,
        handles,
        source_name,
        sandbox,
        faults,
        no_quarantine_after_rename_hook,
    )
}

fn no_quarantine_after_rename_hook(_: &PinnedDirectory, _: &ChildName) {}

fn quarantine_original_sandbox_with_hook(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    source_name: ChildName,
    sandbox: ValidatedOriginalSandbox,
    faults: &SandboxFaults,
    after_rename: impl FnOnce(&PinnedDirectory, &ChildName),
) -> Result<(), SandboxError> {
    let identity = sandbox.root().identity();
    let cleanup_name = paths.cleanup_name();
    faults.check(SandboxFaultPoint::RenameOriginal)?;
    let ticket = sandbox.into_rename_ticket(source_name, cleanup_name.clone());
    let moved = handles
        .sandboxes()
        .rename_noreplace(ticket)
        .map_err(SandboxError::from)?;
    drop(moved);
    handles.sandboxes().sync().map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::AfterRenameIdentity)?;
    after_rename(handles.sandboxes(), &cleanup_name);
    let cleanup = validate_cleanup_candidate(handles.sandboxes(), &cleanup_name, paths)?;
    if cleanup.root().identity() != identity {
        return Err(SandboxError::invalid(
            &paths.cleanup_root,
            "the reopened cleanup identity does not match the rename source",
        ));
    }
    remove_cleanup_sandbox(paths, handles, cleanup, faults)
}

fn recover_cleanup_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    recover_cleanup_sandbox_with_hook(paths, handles, sandbox, faults, no_recover_cleanup_hook)
}

fn no_recover_cleanup_hook(_: &ValidatedCleanupSandbox) {}

fn recover_cleanup_sandbox_with_hook(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
    before_validate: impl FnOnce(&ValidatedCleanupSandbox),
) -> Result<(), SandboxError> {
    before_validate(&sandbox);
    validate_same_cleanup(&sandbox, handles.sandboxes(), &paths.cleanup_name(), paths)?;
    remove_cleanup_sandbox(paths, handles, sandbox, faults)
}

fn refuse_retained_quarantine_state<O, C>(
    paths: &StableSandboxPaths,
    original: &Evidence<O>,
    cleanup: &Evidence<C>,
) -> SandboxError {
    SandboxError::invalid(
        &paths.sandboxes_root,
        format!(
            "refused original {:?} and cleanup {:?}; original detail: {}; cleanup detail: {}",
            original.state(),
            cleanup.state(),
            original.reason().unwrap_or("none"),
            cleanup.reason().unwrap_or("none"),
        ),
    )
}

fn prepare_fresh_sandbox_retained(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    let original = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.sandbox_name(),
        |parent, name| validate_original_candidate(parent, name, paths),
    )?;
    let cleanup = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.cleanup_name(),
        |parent, name| validate_cleanup_candidate(parent, name, paths),
    )?;
    match decide_quarantine(original.state(), cleanup.state()) {
        QuarantineDecision::FreshCreate => {}
        QuarantineDecision::CleanupOriginal => quarantine_original_sandbox(
            paths,
            handles,
            paths.sandbox_name(),
            original
                .into_valid()
                .expect("the cleanup-original decision has valid original evidence"),
            faults,
        )?,
        QuarantineDecision::RecoverCleanup => recover_cleanup_sandbox(
            paths,
            handles,
            cleanup
                .into_valid()
                .expect("the recover-cleanup decision has valid cleanup evidence"),
            faults,
        )?,
        QuarantineDecision::RefuseRetainAll => {
            return Err(refuse_retained_quarantine_state(paths, &original, &cleanup));
        }
    }
    create_fresh_sandbox_retained(paths, handles, faults)
}

fn close_retained_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedOriginalSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    validate_same_original(&sandbox, handles.sandboxes(), &paths.sandbox_name(), paths)?;
    let cleanup = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.cleanup_name(),
        |parent, name| validate_cleanup_candidate(parent, name, paths),
    )?;
    if cleanup.state() != SandboxEvidenceState::Absent {
        let original = Evidence::Valid(());
        return Err(refuse_retained_quarantine_state(paths, &original, &cleanup));
    }
    quarantine_original_sandbox(paths, handles, paths.sandbox_name(), sandbox, faults)
}

#[derive(Debug)]
struct StableSandbox {
    paths: StableSandboxPaths,
    namespace: Option<NamespaceHandles>,
    namespace_marker: Option<PinnedFile>,
    lease: Option<ProfileLease>,
    sandbox: Option<ValidatedOriginalSandbox>,
    faults: SandboxFaults,
    armed: bool,
}

fn no_live_namespace_marker_hook(_: &PinnedDirectory, _: &PinnedFile, _: &ChildName) {}

fn no_acquisition_after_prepare_hook(_: &StableSandbox) {}

fn combine_acquisition_abort(
    primary: SandboxError,
    cleanup: Result<(), SandboxError>,
    release: Result<(), SandboxError>,
) -> SandboxError {
    let primary = match cleanup {
        Ok(()) => primary,
        Err(cleanup) => SandboxError::Dual {
            primary: Box::new(primary),
            release: Box::new(cleanup),
        },
    };
    match release {
        Ok(()) => primary,
        Err(release) => SandboxError::Dual {
            primary: Box::new(primary),
            release: Box::new(release),
        },
    }
}

impl StableSandbox {
    fn acquire(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
        faults: SandboxFaults,
    ) -> Result<Self, SandboxError> {
        Self::acquire_with_hooks(
            namespace,
            profile,
            faults,
            no_acquisition_after_prepare_hook,
            release_profile_lease,
        )
    }

    fn acquire_with_hooks(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
        faults: SandboxFaults,
        after_prepare: impl FnOnce(&StableSandbox),
        abort_release: impl FnOnce(ProfileLease, &PinnedDirectory) -> Result<(), SandboxFsError>,
    ) -> Result<Self, SandboxError> {
        let paths = StableSandboxPaths::new(namespace, profile)?;
        let (namespace, namespace_marker) = initialize_namespace_retained(&paths, &faults)?;
        let lease = acquire_profile_lease_retained(&paths, &namespace, &faults)?;
        let mut sandbox = Self {
            paths,
            namespace: Some(namespace),
            namespace_marker: Some(namespace_marker),
            lease: Some(lease),
            sandbox: None,
            faults,
            armed: true,
        };
        let prepared = match prepare_fresh_sandbox_retained(
            &sandbox.paths,
            sandbox.handles(),
            &sandbox.faults,
        ) {
            Ok(prepared) => prepared,
            Err(primary) => {
                return Err(sandbox.abort_acquire(primary, abort_release));
            }
        };
        sandbox.sandbox = Some(prepared);
        after_prepare(&sandbox);
        let validation = sandbox
            .validate_namespace()
            .and_then(|()| sandbox.validate_lease())
            .and_then(|()| sandbox.validate_live_sandbox());
        if let Err(primary) = validation {
            return Err(sandbox.abort_acquire(primary, abort_release));
        }
        Ok(sandbox)
    }

    fn abort_acquire(
        mut self,
        primary: SandboxError,
        release: impl FnOnce(ProfileLease, &PinnedDirectory) -> Result<(), SandboxFsError>,
    ) -> SandboxError {
        self.armed = false;
        let cleanup = match self.sandbox.take() {
            Some(sandbox) => {
                close_retained_sandbox(&self.paths, self.handles(), sandbox, &self.faults)
            }
            None => Ok(()),
        };
        let release = release(
            self.lease
                .take()
                .expect("an armed partial sandbox retains its profile lease"),
            self.handles().leases(),
        )
        .map_err(SandboxError::from);
        combine_acquisition_abort(primary, cleanup, release)
    }

    fn path(&self) -> &Path {
        &self.paths.sandbox_root
    }

    fn metadata(&self, review_profile: &SafeProfileId) -> Result<SandboxMetadata, SandboxError> {
        if review_profile != &self.paths.profile {
            return Err(SandboxError::invalid(
                &self.paths.sandbox_root,
                "the requested review profile does not own this sandbox",
            ));
        }
        self.validate_namespace()?;
        self.validate_lease()?;
        self.validate_live_sandbox()?;
        SandboxMetadata::stable(
            self.paths.platform,
            literal_path(&self.paths.namespace_root)?,
            BTreeSet::from([review_profile.clone()]),
        )
        .map_err(|error| SandboxError::invalid(&self.paths.namespace_root, error.to_string()))
    }

    fn handles(&self) -> &NamespaceHandles {
        self.namespace
            .as_ref()
            .expect("an armed stable sandbox retains its namespace handles")
    }

    fn validate_namespace(&self) -> Result<(), SandboxError> {
        self.validate_namespace_with_hook(no_live_namespace_marker_hook)
    }

    fn validate_namespace_with_hook(
        &self,
        after_marker_read: impl FnOnce(&PinnedDirectory, &PinnedFile, &ChildName),
    ) -> Result<(), SandboxError> {
        let handles = self.handles();
        handles
            .verify(
                &self.paths.namespace_name(),
                &self.paths.leases_name(),
                &self.paths.sandboxes_name(),
            )
            .map_err(SandboxError::from)?;
        let marker = self
            .namespace_marker
            .as_ref()
            .expect("an armed stable sandbox retains its namespace marker");
        marker.verify_handle().map_err(SandboxError::from)?;
        marker
            .verify_at(handles.namespace(), &self.paths.namespace_marker_name())
            .map_err(SandboxError::from)?;
        let bytes = marker
            .read_all(handles.namespace(), &self.paths.namespace_marker_name())
            .map_err(SandboxError::from)?;
        if bytes != self.paths.namespace_marker_bytes() {
            return Err(SandboxError::invalid(
                marker.path(),
                "the namespace marker bytes are invalid",
            ));
        }
        let marker_name = self.paths.namespace_marker_name();
        after_marker_read(handles.namespace(), marker, &marker_name);
        require_ticket(
            handles.namespace(),
            &marker_name,
            SandboxNodeKind::RegularFile,
            marker.identity(),
        )?;
        require_exact_names(
            handles.namespace(),
            &[
                self.paths.namespace_marker_name(),
                self.paths.leases_name(),
                self.paths.sandboxes_name(),
            ],
        )
    }

    fn validate_live_sandbox(&self) -> Result<(), SandboxError> {
        validate_same_original(
            self.sandbox
                .as_ref()
                .expect("an armed stable sandbox retains its validated root"),
            self.handles().sandboxes(),
            &self.paths.sandbox_name(),
            &self.paths,
        )
    }

    fn validate_lease(&self) -> Result<(), SandboxError> {
        let lease = self
            .lease
            .as_ref()
            .expect("an armed stable sandbox retains its profile lease");
        lease
            .validate(self.handles().leases())
            .map_err(SandboxError::from)?;
        if lease
            .file()
            .read_all(self.handles().leases(), &self.paths.lease_name())
            .map_err(SandboxError::from)?
            != self.paths.lease_marker_bytes()
        {
            return Err(SandboxError::invalid(
                &self.paths.lease_path,
                "the held profile lease marker bytes are invalid",
            ));
        }
        Ok(())
    }

    fn close(mut self) -> Result<(), SandboxError> {
        self.armed = false;
        let primary = (|| {
            self.validate_namespace()?;
            self.validate_lease()?;
            self.validate_live_sandbox()?;
            let sandbox = self
                .sandbox
                .take()
                .expect("an armed stable sandbox retains its validated root");
            close_retained_sandbox(&self.paths, self.handles(), sandbox, &self.faults)
        })();
        let release = self
            .lease
            .take()
            .expect("an armed stable sandbox retains its profile lease")
            .release(self.handles().leases())
            .map_err(SandboxError::from);
        combine_sandbox_release(primary, release)
    }
}

impl Drop for StableSandbox {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            let can_cleanup = self.validate_namespace().is_ok()
                && self.validate_lease().is_ok()
                && self.validate_live_sandbox().is_ok();
            if can_cleanup && let Some(sandbox) = self.sandbox.take() {
                let _ = close_retained_sandbox(&self.paths, self.handles(), sandbox, &self.faults);
            }
            if let Some(lease) = self.lease.take() {
                let _ = lease.release(self.handles().leases());
            }
        }
    }
}

#[derive(Debug)]
enum SandboxOwner {
    Random { directory: TempDir, root: PathBuf },
    Stable(Box<StableSandbox>),
}

/// The prefix that Windows `fs::canonicalize` puts in front of a resolved drive path.
const VERBATIM_PREFIX: &str = r"\\?\";

/// The prefix that the verbatim form puts in front of a resolved network-share path.
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

/// Whether one text starts with a drive letter, a colon, and a separator.
fn starts_with_drive(text: &str) -> bool {
    let mut characters = text.chars();
    matches!(
        (characters.next(), characters.next(), characters.next()),
        (Some(letter), Some(':'), Some('\\')) if letter.is_ascii_alphabetic()
    )
}

/// Rewrite one Windows verbatim path into the ordinary spelling of the same location.
///
/// `\\?\C:\a\b` becomes `C:\a\b`, and `\\?\UNC\server\share\a` becomes `\\server\share\a`. Every
/// other text stays as it is, because only these two verbatim forms have an ordinary spelling. A
/// device path such as `\\?\Volume{...}` has none.
///
/// The rule reads text only, so it runs and it is testable on every platform. It is correct for
/// the output of `fs::canonicalize`. Do not use it on a hand-written verbatim path: the verbatim
/// form keeps a trailing dot or space that the ordinary form loses.
fn ordinary_windows_path(text: &str) -> Cow<'_, str> {
    if let Some(share) = text.strip_prefix(VERBATIM_UNC_PREFIX) {
        return Cow::Owned(format!(r"\\{share}"));
    }
    match text.strip_prefix(VERBATIM_PREFIX) {
        Some(rest) if starts_with_drive(rest) => Cow::Borrowed(rest),
        _ => Cow::Borrowed(text),
    }
}

/// Get one path in the spelling that this fixture looks up and compares.
///
/// The fixture speaks the ordinary spelling. A path that the product recorded can arrive in the
/// verbatim form, so it drops that prefix first. A path that is not Unicode keeps its exact bytes.
fn ordinary_path(path: &Path) -> Cow<'_, Path> {
    match path.to_str().map(ordinary_windows_path) {
        Some(Cow::Borrowed(text)) => Cow::Borrowed(Path::new(text)),
        Some(Cow::Owned(text)) => Cow::Owned(PathBuf::from(text)),
        None => Cow::Borrowed(path),
    }
}

/// Whether one path that the product recorded names the same location as one fixture path.
///
/// The product resolves every path it records, and Windows `fs::canonicalize` returns the
/// verbatim form. The recorded text therefore differs from the fixture spelling on Windows even
/// when both name one file, so the comparison drops the verbatim prefix first.
fn names_the_same_path(recorded: &str, fixture: &Path) -> bool {
    *ordinary_path(Path::new(recorded)) == *fixture
}

/// Get the random sandbox root in the spelling that the product records.
///
/// The product resolves every source path it records, so the fixture root resolves too. If it does
/// not, the projection cannot recognize the stored path. A Unix temporary directory can sit behind
/// a symlink (macOS does this for every one; Linux does when `TMPDIR` points at a link). A Windows
/// temporary directory can carry an 8.3 short name: the GitHub runner gives
/// `C:\Users\RUNNER~1\AppData\Local\Temp`, and `fs::canonicalize` expands it to `runneradmin`.
///
/// Windows must resolve the root too, but it must not keep the resolved spelling. Windows
/// `fs::canonicalize` returns the `\\?\` verbatim form, and that spelling must stay out of the
/// recorded interface. The root therefore drops the prefix, and `ordinary_path` drops it again
/// from every path that the product recorded. Both sides then speak the ordinary spelling.
///
/// A resolved root hides one product behavior: for an owned draft the product keeps two spellings
/// on purpose (`skit-ui/src/add.rs`: `path` keeps skit's own spelling, `source_record` holds the
/// resolved one). The walker oracle `project_commit_review_name` requires the two to name one
/// location, so no walker run exercises the two-spelling case.
fn resolved_random_root(root: &Path) -> PathBuf {
    let resolved = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    ordinary_path(&resolved).into_owned()
}

impl SandboxOwner {
    fn random() -> Result<Self, SandboxError> {
        Self::random_with(TempDir::new)
    }

    fn random_with(create: impl FnOnce() -> io::Result<TempDir>) -> Result<Self, SandboxError> {
        create()
            .map(|directory| {
                let root = resolved_random_root(directory.path());
                Self::Random { directory, root }
            })
            .map_err(|error| {
                SandboxError::io("create random sandbox", Path::new("<system-temp>"), error)
            })
    }

    fn path(&self) -> &Path {
        match self {
            Self::Random { root, .. } => root.as_path(),
            Self::Stable(sandbox) => sandbox.path(),
        }
    }

    fn stable(&self) -> Option<&StableSandbox> {
        match self {
            Self::Random { .. } => None,
            Self::Stable(sandbox) => Some(sandbox),
        }
    }

    fn metadata(&self, review_profile: &SafeProfileId) -> Result<SandboxMetadata, SandboxError> {
        match self {
            Self::Random { root, .. } => {
                let platform = current_sandbox_platform()?;
                let literal = literal_path(root)?;
                SandboxMetadata::random(platform, literal, review_profile.clone())
                    .map_err(|error| SandboxError::invalid(root, error.to_string()))
            }
            Self::Stable(sandbox) => sandbox.metadata(review_profile),
        }
    }

    fn close(self) -> Result<(), SandboxError> {
        match self {
            Self::Random { directory, root } => directory
                .close()
                .map_err(|error| SandboxError::io("close random sandbox", &root, error)),
            Self::Stable(sandbox) => StableSandbox::close(*sandbox),
        }
    }
}

#[derive(Debug)]
pub(super) struct SeededTuiHost {
    _sandbox: SandboxOwner,
    roots: ProductRoots,
    external_root: PathBuf,
    file_picker_tree: WalkerFilePickerTree,
    service: LibraryService<FileStore>,
    locale: Cell<Locale>,
    clock: FixedClock,
    adapters: RecordingAdapters,
    path_map: PathMap,
    observation_modes: RefCell<ObservationModeProvenance>,
    profile: String,
}

pub(super) type RealWalkerHost = SeededTuiHost;

impl SeededTuiHost {
    pub(super) fn spawn(spec: WalkerSeedSpec) -> Result<Self, String> {
        Self::spawn_with_allocation_maximums(spec, AllocationMaximums::default())
    }

    pub(super) fn spawn_stable_in(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
    ) -> Result<Self, StableHostError> {
        Self::spawn_stable_in_with_faults(spec, review_profile, namespace, SandboxFaults::default())
    }

    fn spawn_stable_in_with_faults(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
        faults: SandboxFaults,
    ) -> Result<Self, StableHostError> {
        Self::spawn_stable_in_with_faults_and_directory_seed_io(
            spec,
            review_profile,
            namespace,
            faults,
            &SystemDirectorySeedIo,
        )
    }

    #[cfg(test)]
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn spawn_stable_in_with_directory_seed_io(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, StableHostError> {
        Self::spawn_stable_in_with_faults_and_directory_seed_io(
            spec,
            review_profile,
            namespace,
            SandboxFaults::default(),
            directory_seed_io,
        )
    }

    fn spawn_stable_in_with_faults_and_directory_seed_io(
        spec: WalkerSeedSpec,
        review_profile: SafeProfileId,
        namespace: StableSandboxNamespace,
        faults: SandboxFaults,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, StableHostError> {
        validate_external_spec(&spec).map_err(|primary| StableHostError::Primary {
            primary: StableHostPrimaryError::Seed(primary),
            cleanup: None,
        })?;
        let sandbox = SandboxOwner::Stable(Box::new(StableSandbox::acquire(
            namespace,
            review_profile,
            faults,
        )?));
        match Self::spawn_in_sandbox(
            spec,
            sandbox,
            AllocationMaximums::default(),
            directory_seed_io,
        ) {
            Ok(host) => Ok(host),
            Err((sandbox, primary)) => {
                let cleanup = sandbox.close().err();
                Err(StableHostError::Primary {
                    primary: StableHostPrimaryError::Seed(primary),
                    cleanup,
                })
            }
        }
    }

    fn spawn_with_allocation_maximums(
        spec: WalkerSeedSpec,
        allocation_maximums: AllocationMaximums,
    ) -> Result<Self, String> {
        Self::spawn_with_allocation_maximums_and_directory_seed_io(
            spec,
            allocation_maximums,
            &SystemDirectorySeedIo,
        )
    }

    #[cfg(test)]
    fn spawn_with_directory_seed_io(
        spec: WalkerSeedSpec,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, String> {
        Self::spawn_with_allocation_maximums_and_directory_seed_io(
            spec,
            AllocationMaximums::default(),
            directory_seed_io,
        )
    }

    fn spawn_with_allocation_maximums_and_directory_seed_io(
        spec: WalkerSeedSpec,
        allocation_maximums: AllocationMaximums,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, String> {
        validate_external_spec(&spec)?;
        let sandbox = SandboxOwner::random().map_err(|error| error.to_string())?;
        match Self::spawn_in_sandbox(spec, sandbox, allocation_maximums, directory_seed_io) {
            Ok(host) => Ok(host),
            Err((sandbox, primary)) => Self::failed_random_spawn(sandbox, primary),
        }
    }

    fn failed_random_spawn(sandbox: SandboxOwner, primary: String) -> Result<Self, String> {
        match sandbox.close() {
            Ok(()) => Err(primary),
            Err(cleanup) => Err(format!(
                "{primary}; random sandbox cleanup also failed: {cleanup}"
            )),
        }
    }

    fn spawn_in_sandbox(
        spec: WalkerSeedSpec,
        sandbox: SandboxOwner,
        allocation_maximums: AllocationMaximums,
        directory_seed_io: &dyn DirectorySeedIo,
    ) -> Result<Self, (SandboxOwner, String)> {
        let built = (|| -> Result<_, String> {
            let roots = ProductRoots::new(
                sandbox.path().join("data"),
                sandbox.path().join("state"),
                sandbox.path().join("config"),
                Some(sandbox.path().join("home")),
                sandbox.path().join("cwd"),
            );
            let external_root = sandbox.path().join("external");
            let system_temp = sandbox.path().join("system-temp");
            for root in [
                &roots.data,
                &roots.state,
                &roots.config,
                roots.home.as_ref().expect("the walker profile has a home"),
                &roots.cwd,
                &external_root,
                &system_temp,
            ] {
                if let Some(stable) = sandbox.stable() {
                    stable
                        .validate_namespace()
                        .map_err(|error| error.to_string())?;
                    stable.validate_lease().map_err(|error| error.to_string())?;
                    stable
                        .validate_live_sandbox()
                        .map_err(|error| error.to_string())?;
                } else {
                    fs::create_dir_all(root).map_err(|error| error.to_string())?;
                }
            }
            let seeded_directories =
                seed_profile_directories(&roots, &spec.directories, directory_seed_io)?;
            let file_picker_tree = seed_external_world(&external_root, &spec.external)?;

            let create_clock = Arc::new(WalkerCreateClock::default());
            let (expected, seeded_copy_paths) = {
                let service = LibraryService::new(FileStore::with_create_clock(
                    &roots.data,
                    create_clock.clone(),
                ));
                let seeded_copy_paths = seed_profile(&service, &roots, &external_root, &spec)?;
                (read_seed_snapshot(&service, &roots)?, seeded_copy_paths)
            };
            let service =
                LibraryService::new(FileStore::with_create_clock(&roots.data, create_clock));
            let reopened = read_seed_snapshot(&service, &roots)?;
            (reopened == expected)
                .then_some(())
                .ok_or("the production stores changed after the seed was reopened".to_owned())?;
            let configured = FileConfigStore::new(&roots.config)
                .get("lang")
                .map_err(|error| error.to_string())?;
            let locale = requested_locale(Some(&configured)).unwrap_or(Locale::En);
            let transcript = Rc::new(RefCell::new(Vec::new()));
            let profile = if spec.profile.is_empty() {
                "default".to_owned()
            } else {
                spec.profile.clone()
            };
            let path_map =
                PathMap::new(&profile, sandbox.path(), &service, &file_picker_tree.files)?;
            let observation_modes = ObservationModeProvenance::from_seed(
                &sandbox,
                &roots,
                &external_root,
                &system_temp,
                &seeded_directories,
                &spec.external,
                &seeded_copy_paths,
            )?;
            Ok((
                roots,
                external_root,
                file_picker_tree,
                service,
                locale,
                transcript,
                system_temp,
                path_map,
                observation_modes,
                profile,
            ))
        })();
        let (
            roots,
            external_root,
            file_picker_tree,
            service,
            locale,
            transcript,
            system_temp,
            path_map,
            observation_modes,
            profile,
        ) = match built {
            Ok(built) => built,
            Err(error) => return Err((sandbox, error)),
        };
        Ok(Self {
            _sandbox: sandbox,
            roots,
            external_root,
            file_picker_tree,
            service,
            locale: Cell::new(locale),
            clock: FixedClock::new(Rc::clone(&transcript)),
            adapters: RecordingAdapters::new_with_maximums_and_editor_writes(
                transcript,
                system_temp,
                allocation_maximums,
                spec.editor_writes,
            ),
            path_map,
            observation_modes: RefCell::new(observation_modes),
            profile,
        })
    }

    pub(super) fn sandbox_root(&self) -> &Path {
        self._sandbox.path()
    }

    pub(super) fn sandbox_metadata(
        &self,
        review_profile: &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError> {
        self._sandbox.metadata(review_profile)
    }

    pub(super) fn leak_oracle_facts(&self) -> LeakOracleFacts {
        self.path_map.leak_oracle_facts()
    }

    pub(super) fn close(self) -> Result<(), SandboxError> {
        let Self {
            _sandbox,
            roots,
            external_root,
            file_picker_tree,
            service,
            locale,
            clock,
            adapters,
            path_map,
            observation_modes,
            profile,
        } = self;
        drop((
            profile,
            observation_modes,
            path_map,
            adapters,
            clock,
            locale,
            service,
            file_picker_tree,
            external_root,
            roots,
        ));
        _sandbox.close()
    }

    pub(super) fn roots(&self) -> &ProductRoots {
        &self.roots
    }

    pub(super) fn file_picker_tree(&self) -> WalkerFilePickerTree {
        self.file_picker_tree.clone()
    }

    pub(super) fn initial_state(&self) -> Result<LibraryState, super::CliError> {
        self.with_host(|host| host.initial_state())
    }

    pub(super) fn preflight(&self, effect: &Effect) -> Result<(), super::CliError> {
        self.with_host(|host| host.preflight(effect))
    }

    fn serve_after_preflight(&self, effect: Effect) -> Result<Action, super::CliError> {
        self.with_host(|host| host.serve(effect))
    }

    pub(super) fn dispatch(&self, effect: Effect) -> Result<Action, super::CliError> {
        match self.preflight(&effect) {
            Ok(()) => {
                let action = self.serve_after_preflight(effect)?;
                self.observation_modes
                    .borrow_mut()
                    .record_created_copy(&self.service, &action);
                Ok(action)
            }
            Err(error) => Ok(Action::SetStatus(
                Message::new("Error: {}")
                    .nested(error.message())
                    .localize(self.locale.get()),
            )),
        }
    }

    fn pending_observation(&self) -> Result<PendingHostObservation, String> {
        let surface = library_surface_at(
            self.service.repository(),
            &self.roots.state,
            &self.roots.config,
            self.clock.now_utc(),
        )
        .map_err(|error| error.to_string())?;
        let scan = self.service.list().map_err(|error| error.to_string())?;
        let forms = FormStateService::new(FileFormStateStore::new(&self.roots.state));
        let form_state = scan
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.slug.as_str().to_owned(),
                    persisted_form_json(&forms.load(&entry.slug)),
                )
            })
            .collect();
        let config = FileConfigStore::new(&self.roots.config);
        let runner_rows = config
            .runner_rows()
            .map_err(|error| error.to_string())?
            .into_iter()
            .map(|runner| {
                json!({
                    "index": runner.index,
                    "name": runner.name,
                    "argv": runner.argv,
                    "reason": runner.reason,
                    "descriptor": runner.descriptor,
                    "snapshot": runner.snapshot_token(),
                })
            })
            .collect::<Vec<_>>();
        let mirror = config.mirror().map_err(|error| error.to_string())?;
        let config_path = self.roots.config.join("config.toml");
        let config = json!({
            "settings": config.settings().map_err(|error| error.to_string())?,
            "runner_rows": runner_rows,
            "mirror": {
                "enabled": mirror.enabled,
                "pypi": mirror.pypi,
                "python_install": mirror.python_install,
                "uv_binary": mirror.uv_binary,
                "npm": mirror.npm,
            },
            "document_ref": config_path.is_file().then_some("tree:config/config.toml"),
        });
        let prompt_runner =
            PromptSelectionService::new(FilePromptSelectionStore::new(&self.roots.state))
                .last_runner();
        let drafts = sorted_tui_drafts(&self.roots.data);
        let drafts = serde_json::to_value(drafts).map_err(|error| error.to_string())?;
        let surface = serde_json::to_value(surface).map_err(|error| error.to_string())?;
        Ok(PendingHostObservation {
            surface,
            config,
            form_state,
            prompt_runner,
            drafts,
        })
    }

    pub(super) fn capture_checkpoint_parts(
        &mut self,
        state: &mut LibraryState,
        session: &mut Value,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<HostObservation, String> {
        self.capture_checkpoint_parts_internal(state, Some(session), cause)
    }

    fn capture_checkpoint_parts_internal(
        &mut self,
        state: &mut LibraryState,
        session: Option<&mut Value>,
        cause: CheckpointCauseProjection<'_, Action, Effect>,
    ) -> Result<HostObservation, String> {
        let pending = self.pending_observation()?;
        let events = self.adapters.pending_events();

        self.path_map.refresh(&self.service)?;
        let observation_modes = self.observation_modes.get_mut();
        observation_modes.refresh_live_sources(&self.service)?;
        for event in &events {
            self.path_map.register_event_artifacts(event);
            observation_modes.register_event(event, &self.roots)?;
        }
        self.path_map.project_checkpoint_cause(cause)?;
        self.path_map.project_typed_library_state(state)?;
        let state = serde_json::to_value(&*state).map_err(|error| error.to_string())?;
        if let Some(session) = session {
            self.path_map.project_session_value(session)?;
        }

        let surface = self.path_map.normalize_library_json(pending.surface)?;
        let drafts = self.path_map.normalize_draft_list(pending.drafts)?;
        ensure_system_temp_empty(&self.adapters.system_temp, &self.path_map)?;
        let tree = snapshot_tree(
            &self.roots,
            &self.external_root,
            &mut self.path_map,
            observation_modes,
        )?;
        let transcript = RecordingAdapters::render_transcript(&events, &mut self.path_map);
        let observation = HostObservation {
            surface,
            state,
            config: pending.config,
            form_state: pending.form_state,
            prompt_runner: pending.prompt_runner,
            drafts,
            tree,
            transcript,
        };
        self.adapters.drain_event_prefix(events.len())?;
        Ok(observation)
    }

    pub(super) fn observe(&mut self, state: &LibraryState) -> Result<HostObservation, String> {
        let mut state = state.clone();
        self.capture_checkpoint_parts_internal(
            &mut state,
            None,
            CheckpointCauseProjection::Observation,
        )
    }

    pub(super) fn canonical_action(&mut self, action: &Action) -> Result<Value, String> {
        self.path_map.refresh(&self.service)?;
        let mut action = action.clone();
        self.path_map.project_typed_action(&mut action)?;
        serde_json::to_value(action).map_err(|error| error.to_string())
    }

    pub(super) fn clear_transcript(&self) {
        self.adapters.events.borrow_mut().clear();
    }

    pub(super) fn fail_next_launch(&self, kind: io::ErrorKind, reason: &str) {
        self.adapters.launch_script.replace(LaunchScript::Failure {
            kind,
            reason: reason.to_owned(),
        });
    }

    fn with_host<T>(
        &self,
        operation: impl FnOnce(&TuiHost<'_, FixedClock>) -> Result<T, super::CliError>,
    ) -> Result<T, super::CliError> {
        let host = TuiHost::new(
            &self.service,
            self.roots.clone(),
            self.locale.get(),
            &self.clock,
            self.adapters.platform(),
        )
        .expect("the walker service and roots share the same data directory");
        let result = operation(&host);
        self.locale.set(host.locale());
        result
    }
}

#[derive(Debug, Default)]
struct WalkerCreateClock(AtomicU64);

impl EntryCreateClock for WalkerCreateClock {
    fn now_utc(&self) -> OffsetDateTime {
        let seconds = self.0.fetch_add(1, Ordering::Relaxed);
        let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
        fixed_instant() + time::Duration::seconds(seconds)
    }
}

fn seed_profile(
    service: &LibraryService<FileStore>,
    roots: &ProductRoots,
    external_root: &Path,
    spec: &WalkerSeedSpec,
) -> Result<Vec<PathBuf>, String> {
    let mut copy_paths = Vec::new();
    for request in &spec.entries {
        let entry = service
            .add(request.clone())
            .map_err(|error| error.to_string())?;
        if has_store_owned_copy_payload(&entry) {
            let path = service
                .repository()
                .payload_path(&entry)
                .map_err(|error| error.to_string())?;
            copy_paths.push(path);
        }
    }
    for seed in &spec.external_references {
        let mut request = seed.request.clone();
        request.source = external_root.join(&seed.source).display().to_string();
        let entry = service.add(request).map_err(|error| error.to_string())?;
        if has_store_owned_copy_payload(&entry) {
            let path = service
                .repository()
                .payload_path(&entry)
                .map_err(|error| error.to_string())?;
            copy_paths.push(path);
        }
    }
    let config = FileConfigStore::new(&roots.config);
    config
        .set_many(&spec.settings)
        .map_err(|error| error.to_string())?;
    for runner in &spec.runners {
        config
            .set_runner(runner.clone(), true)
            .map_err(|error| error.to_string())?;
    }
    let forms = FormStateService::new(FileFormStateStore::new(&roots.state));
    for seed in &spec.forms {
        let entry = service
            .show(&seed.selector)
            .map_err(|error| error.to_string())?;
        let declarations = super::entry_parameters(service.repository(), &entry);
        forms
            .save_last(
                &entry.slug,
                &declarations,
                Some(&seed.values),
                Some(seed.extra_args.clone()),
                seed.extra_args_raw,
            )
            .map_err(|error| error.to_string())?;
        if let Some(preset) = &seed.preset {
            forms
                .save_preset(&entry.slug, preset, &declarations, &seed.values)
                .map_err(|error| error.to_string())?;
        }
        if let Some(last_run) = &seed.last_run {
            forms
                .record_run(
                    &entry.slug,
                    last_run.exit,
                    &last_run.at,
                    &declarations,
                    last_run.values.as_ref(),
                )
                .map_err(|error| error.to_string())?;
        }
    }
    if !spec.prompt_runner.is_empty() {
        PromptSelectionService::new(FilePromptSelectionStore::new(&roots.state))
            .remember_runner(&spec.prompt_runner)
            .map_err(|error| error.to_string())?;
    }
    Ok(copy_paths)
}

fn validate_external_spec(spec: &WalkerSeedSpec) -> Result<(), String> {
    for directory in &spec.directories {
        validate_directory_seed_path(&directory.path)?;
    }
    let _ = validated_unique_external_seeds(&spec.external)?;
    for reference in &spec.external_references {
        validate_fixture_path(&reference.source)?;
    }
    Ok(())
}

fn validated_unique_external_seeds(
    seeds: &[WalkerExternalSeed],
) -> Result<Vec<&WalkerExternalSeed>, String> {
    let mut unique: Vec<&WalkerExternalSeed> = Vec::with_capacity(seeds.len());
    'seeds: for seed in seeds {
        let path = match seed {
            WalkerExternalSeed::File { path, .. } => {
                validate_fixture_path(path)?;
                path.as_path()
            }
            WalkerExternalSeed::Symlink { path, target } => {
                validate_fixture_path(path)?;
                validate_fixture_path(target)?;
                path.as_path()
            }
        };
        for previous in &unique {
            let previous_path = match previous {
                WalkerExternalSeed::File { path, .. }
                | WalkerExternalSeed::Symlink { path, .. } => path,
            };
            if path == previous_path {
                if seed == *previous {
                    continue 'seeds;
                }
                let same_kind = matches!(
                    (seed, *previous),
                    (
                        WalkerExternalSeed::File { .. },
                        WalkerExternalSeed::File { .. }
                    ) | (
                        WalkerExternalSeed::Symlink { .. },
                        WalkerExternalSeed::Symlink { .. }
                    )
                );
                if same_kind {
                    return Err(format!(
                        "walker external fixture seed payloads conflict at {}",
                        path.display()
                    ));
                }
            }
            if path == previous_path
                || path.starts_with(previous_path)
                || previous_path.starts_with(path)
            {
                let mut paths = [
                    previous_path.display().to_string(),
                    path.display().to_string(),
                ];
                paths.sort();
                return Err(format!(
                    "walker external fixture leaf paths overlap: {} and {}",
                    paths[0], paths[1]
                ));
            }
        }
        unique.push(seed);
    }
    Ok(unique)
}

fn validate_directory_seed_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    if path.as_os_str().is_empty()
        || !components.all(|component| matches!(component, std::path::Component::Normal(_)))
        || !directory_seed_spelling_is_normal(path)
    {
        return Err(format!(
            "walker profile directory seed paths must be relative descendants: {}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn directory_seed_spelling_is_normal(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    !bytes
        .split(|byte| *byte == b'/')
        .any(|segment| segment.is_empty() || segment == b"." || segment == b"..")
}

#[cfg(windows)]
fn directory_seed_spelling_is_normal(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt as _;
    let units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    !units
        .split(|unit| *unit == u16::from(b'/') || *unit == u16::from(b'\\'))
        .any(|segment| {
            segment.is_empty()
                || segment == [u16::from(b'.')]
                || segment == [u16::from(b'.'), u16::from(b'.')]
        })
}

#[cfg(not(any(unix, windows)))]
fn directory_seed_spelling_is_normal(path: &Path) -> bool {
    let path = path.to_string_lossy();
    !path
        .split(std::path::MAIN_SEPARATOR)
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
}

fn validate_fixture_path(path: &Path) -> Result<(), String> {
    let mut components = path.components();
    if path.as_os_str().is_empty()
        || !components.all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(format!(
            "walker external fixture paths must be relative descendants: {}",
            path.display()
        ));
    }
    Ok(())
}

fn seed_profile_directories(
    roots: &ProductRoots,
    seeds: &[WalkerDirectorySeed],
    directory_seed_io: &dyn DirectorySeedIo,
) -> Result<Vec<PathBuf>, String> {
    let mut seeded = BTreeSet::new();
    for seed in seeds {
        let root = match seed.root {
            WalkerDirectoryRoot::Home => roots
                .home
                .as_deref()
                .expect("the walker profile has a home"),
            WalkerDirectoryRoot::Cwd => roots.cwd.as_path(),
        };
        let mut path = root.to_path_buf();
        for name in &seed.path {
            path.push(name);
            match directory_seed_io.create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = directory_seed_io.symlink_metadata(&path).map_err(|error| {
                        format!(
                            "could not inspect walker profile directory seed {}: {error}",
                            path.display()
                        )
                    })?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(format!(
                            "walker profile directory seed is not a directory: {}",
                            path.display()
                        ));
                    }
                }
                Err(error) => {
                    return Err(format!(
                        "could not create walker profile directory seed {}: {error}",
                        path.display()
                    ));
                }
            }
            directory_seed_io
                .set_private_directory_mode(&path)
                .map_err(|error| {
                    format!(
                        "could not set private mode on walker profile directory seed {}: {error}",
                        path.display()
                    )
                })?;
            seeded.insert(path.clone());
        }
    }
    Ok(seeded.into_iter().collect())
}

fn seed_external_world(
    root: &Path,
    seeds: &[WalkerExternalSeed],
) -> Result<WalkerFilePickerTree, String> {
    let mut tree = WalkerFilePickerTree {
        root: root.to_path_buf(),
        directories: BTreeSet::from([root.to_path_buf()]),
        files: BTreeSet::new(),
    };
    for seed in validated_unique_external_seeds(seeds)? {
        let seeded_path = match seed {
            WalkerExternalSeed::File {
                path,
                bytes,
                readonly,
                unix_mode,
            } => {
                let path = root.join(path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
                }
                fs::write(&path, bytes).map_err(|error| error.to_string())?;
                let mut permissions = fs::metadata(&path)
                    .map_err(|error| error.to_string())?
                    .permissions();
                permissions.set_readonly(*readonly);
                fs::set_permissions(&path, permissions).map_err(|error| error.to_string())?;
                set_unix_mode(&path, *unix_mode)?;
                tree.files.insert(path.clone());
                path
            }
            WalkerExternalSeed::Symlink { path, target } => {
                let path = root.join(path);
                create_fixture_symlink(&root.join(target), &path)?;
                path
            }
        };
        tree.directories.extend(
            seeded_path
                .ancestors()
                .skip(1)
                .take_while(|directory| directory.starts_with(root))
                .map(Path::to_path_buf),
        );
    }
    Ok(tree)
}

#[cfg(unix)]
fn set_unix_mode(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn set_unix_mode(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn create_fixture_symlink(target: &Path, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::os::unix::fs::symlink(target, link).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn create_fixture_symlink(target: &Path, link: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link).map_err(|error| error.to_string())
    } else {
        std::os::windows::fs::symlink_file(target, link).map_err(|error| error.to_string())
    }
}

#[cfg(not(any(unix, windows)))]
fn create_fixture_symlink(_target: &Path, _link: &Path) -> Result<(), String> {
    Err("symbolic-link fixtures are not supported on this platform".to_owned())
}

#[derive(Debug, PartialEq)]
struct SeedSnapshot {
    scan: skit_application::LibraryScan,
    entries: Vec<skit_domain::Entry>,
    payloads: BTreeMap<Slug, SeedPayloadSnapshot>,
    settings: BTreeMap<String, String>,
    runners: Vec<PromptRunner>,
    runner_rows: Vec<skit_store::PromptRunnerRow>,
    mirror: skit_store::MirrorSettings,
    config_bytes: Option<Vec<u8>>,
    forms: BTreeMap<Slug, PersistedFormState>,
    prompt_runner: String,
}

#[derive(Debug, Eq, PartialEq)]
enum SeedPayloadSnapshot {
    NoPayload,
    Present {
        path: PathBuf,
        bytes: Vec<u8>,
        readonly: bool,
        mode: Option<u32>,
        symlink_target: Option<PathBuf>,
    },
}

fn read_seed_snapshot(
    service: &LibraryService<FileStore>,
    roots: &ProductRoots,
) -> Result<SeedSnapshot, String> {
    let scan = service.list().map_err(|error| error.to_string())?;
    let config = FileConfigStore::new(&roots.config);
    let forms = FormStateService::new(FileFormStateStore::new(&roots.state));
    let mut entries = service
        .repository()
        .scan_entries()
        .map_err(|error| error.to_string())?;
    entries.sort_by(|left, right| left.slug.cmp(&right.slug));
    let payloads: BTreeMap<Slug, SeedPayloadSnapshot> = entries
        .iter()
        .map(|entry| {
            let payload = if entry.meta.kind.as_str() == "command" {
                SeedPayloadSnapshot::NoPayload
            } else {
                let path = service
                    .repository()
                    .payload_path(entry)
                    .map_err(|error| error.to_string())?;
                let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
                let symlink_target = if metadata.file_type().is_symlink() {
                    Some(fs::read_link(&path).map_err(|error| error.to_string())?)
                } else {
                    None
                };
                SeedPayloadSnapshot::Present {
                    bytes: fs::read(&path).map_err(|error| error.to_string())?,
                    readonly: metadata.permissions().readonly(),
                    mode: portable_mode(&metadata),
                    symlink_target,
                    path,
                }
            };
            Ok((entry.slug.clone(), payload))
        })
        .collect::<Result<_, String>>()?;
    let form_state = scan
        .entries
        .iter()
        .map(|entry| (entry.slug.clone(), forms.load(&entry.slug)))
        .collect();
    Ok(SeedSnapshot {
        scan,
        entries,
        payloads,
        settings: config.settings().map_err(|error| error.to_string())?,
        runners: config.runners().map_err(|error| error.to_string())?,
        runner_rows: config.runner_rows().map_err(|error| error.to_string())?,
        mirror: config.mirror().map_err(|error| error.to_string())?,
        config_bytes: fs::read(roots.config.join("config.toml")).ok(),
        forms: form_state,
        prompt_runner: PromptSelectionService::new(FilePromptSelectionStore::new(&roots.state))
            .last_runner(),
    })
}

fn persisted_form_json(state: &PersistedFormState) -> Value {
    json!({
        "values": state.values,
        "extra_args": state.extra_args,
        "extra_args_raw": state.extra_args_raw,
        "presets": state.presets,
        "last_run": {
            "at": state.last_run.at,
            "exit": state.last_run.exit,
            "values": state.last_run.values,
        },
    })
}

#[derive(Clone, Debug)]
struct FixedClock {
    at: OffsetDateTime,
    events: Rc<RefCell<Vec<PortEvent>>>,
}

impl FixedClock {
    fn new(events: Rc<RefCell<Vec<PortEvent>>>) -> Self {
        Self {
            at: fixed_instant(),
            events,
        }
    }
}

fn fixed_instant() -> OffsetDateTime {
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
enum PortEvent {
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
enum AllocationPurpose {
    TemporaryFile(TemporaryFilePurpose),
    PrivateDirectory(PrivateDirectoryPurpose),
}

impl AllocationPurpose {
    const fn label(self) -> &'static str {
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
enum AllocationLocation {
    System(PathBuf),
    Directory(PathBuf),
}

impl AllocationLocation {
    fn path(&self) -> &Path {
        match self {
            Self::System(path) | Self::Directory(path) => path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AllocationFailure {
    kind: io::ErrorKind,
    reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AllocationOutcome {
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
    const fn new(maximum: u128) -> Self {
        Self {
            next: Cell::new(Some(0)),
            maximum,
        }
    }

    fn take(&self, purpose: AllocationPurpose) -> Result<u128, (u128, io::Error)> {
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
struct AllocationMaximums {
    authored_draft: u128,
    draft_quarantine: u128,
    injected_source: u128,
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
    fn new(maximums: AllocationMaximums) -> Self {
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
enum ProbeResult {
    Path(Option<PathBuf>),
    Bool(bool),
}

#[derive(Clone, Debug)]
enum LaunchScript {
    Success(LaunchProcessOutput),
    Failure { kind: io::ErrorKind, reason: String },
}

#[derive(Clone, Debug, Default)]
enum EditorScript {
    #[default]
    Success,
    Write(Vec<u8>),
    Failure {
        kind: io::ErrorKind,
        reason: String,
    },
}

#[derive(Clone, Debug, Default)]
enum DependencyScript {
    #[default]
    Success,
    Completed(DependencyCommandOutput),
    Failure {
        kind: io::ErrorKind,
        reason: String,
    },
}

#[derive(Clone, Debug, Default)]
enum InjectedScript {
    #[default]
    Success,
    Completed(InjectedCommandOutput),
    Unavailable(InjectedCommandUnavailable),
}

#[derive(Clone, Debug, Default)]
enum JavaScriptGateScript {
    #[default]
    Success,
    Completed(JavaScriptSyntaxGateOutput),
    Unavailable(JavaScriptSyntaxGateUnavailable),
}

#[derive(Clone, Debug)]
enum UvFetchScript {
    Failure(String),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug)]
struct PreferenceFailure {
    point: AgentSkillInstallPoint,
    kind: io::ErrorKind,
    reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrivateModeTarget {
    File,
    Directory,
}

#[derive(Clone, Debug)]
struct PrivateModeFailure {
    target: PrivateModeTarget,
    kind: io::ErrorKind,
    reason: String,
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
struct RecordingAdapters {
    events: Rc<RefCell<Vec<PortEvent>>>,
    system_temp: PathBuf,
    allocation_counters: AllocationCounters,
    private_mode_failure: RefCell<Option<PrivateModeFailure>>,
    variables: BTreeMap<String, String>,
    programs: BTreeMap<String, PathBuf>,
    launch_script: RefCell<LaunchScript>,
    editor_write_queue: RefCell<VecDeque<Vec<u8>>>,
    editor_script: RefCell<EditorScript>,
    dependency_script: RefCell<DependencyScript>,
    injected_script: RefCell<InjectedScript>,
    javascript_gate_script: RefCell<JavaScriptGateScript>,
    uv_consent: Cell<bool>,
    uv_fetch_script: RefCell<UvFetchScript>,
    preference_failure: RefCell<Option<PreferenceFailure>>,
    stdin_terminal: Cell<bool>,
    stdout_terminal: Cell<bool>,
    system_locale: Cell<Locale>,
}

impl RecordingAdapters {
    fn new(events: Rc<RefCell<Vec<PortEvent>>>, system_temp: PathBuf) -> Self {
        Self::new_with_maximums(events, system_temp, AllocationMaximums::default())
    }

    fn new_with_maximums(
        events: Rc<RefCell<Vec<PortEvent>>>,
        system_temp: PathBuf,
        maximums: AllocationMaximums,
    ) -> Self {
        Self::new_with_maximums_and_editor_writes(events, system_temp, maximums, Vec::new())
    }

    fn new_with_maximums_and_editor_writes(
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

    fn private_mode_failure(&self, target: PrivateModeTarget) -> io::Result<()> {
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

    fn set_private_directory_mode(&self, path: &Path) -> io::Result<()> {
        self.private_mode_failure(PrivateModeTarget::Directory)?;
        set_private_directory_mode(path)
    }

    fn platform(&self) -> TuiPlatform<'_> {
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

    fn push(&self, event: PortEvent) {
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

    fn transcript(&self, paths: &mut PathMap) -> Vec<Value> {
        self.register_pending_artifacts(paths);
        Self::render_transcript(&self.events.borrow(), paths)
    }

    fn pending_events(&self) -> Vec<PortEvent> {
        self.events.borrow().clone()
    }

    fn drain_event_prefix(&self, count: usize) -> Result<(), String> {
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

    fn render_transcript(events: &[PortEvent], paths: &mut PathMap) -> Vec<Value> {
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

fn sorted_tui_drafts(data_dir: &Path) -> Vec<skit_ui::DraftSummary> {
    let mut drafts = super::tui_drafts(data_dir);
    drafts.sort_by(|left, right| {
        fs::read(&left.path)
            .unwrap_or_default()
            .cmp(&fs::read(&right.path).unwrap_or_default())
            .then_with(|| left.path.extension().cmp(&right.path.extension()))
            .then_with(|| left.path.cmp(&right.path))
    });
    drafts
}

#[derive(Debug)]
struct PathMap {
    profile_root: PathBuf,
    resolved_profile_root: PathBuf,
    ambient_paths: BTreeSet<String>,
    profile_label: String,
    file_picker_files: BTreeSet<PathBuf>,
    entry_ids: BTreeMap<String, String>,
    entry_id_generations: BTreeMap<String, u64>,
    entry_generations: BTreeMap<Slug, u64>,
    added_at: BTreeMap<String, String>,
    artifact_facts: BTreeMap<String, ArtifactLeakOracleFact>,
    draft_paths: BTreeMap<PathBuf, DraftPathProjection>,
    next_draft: usize,
    transient_paths: BTreeMap<PathBuf, String>,
    next_transient: usize,
    text_artifacts: BTreeMap<String, String>,
    modified_ranks: AscendingRankAllocator,
    source_identities: SourceIdentitySentinels,
    add_provenance: AddProjectionProvenance,
    add_cause: AddProjectionCause,
    status_provenance: Option<String>,
    status_cause: StatusProjectionCause,
    runner_failure_status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DraftPathProjection {
    raw_path: PathBuf,
    stable_path: String,
    raw_kind_picker_filename: String,
    raw_file_picker_entry_name: String,
    stable_basename: String,
    observed_review_names: BTreeSet<RendererReviewNameFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ReviewNameProjection {
    source_path: PathBuf,
    kind: String,
    raw_name: String,
    stable_name: String,
}

#[derive(Debug, Default, Eq, PartialEq)]
struct AddProjectionProvenance {
    selected_source_path: Option<PathBuf>,
    picked_source_path: Option<String>,
    review_source_path: Option<PathBuf>,
    review_name: Option<ReviewNameProjection>,
    review_name_yank: Option<ReviewNameProjection>,
    review_name_edited: bool,
}

#[derive(Debug, Default)]
enum AddProjectionCause {
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
enum StatusProjectionCause {
    #[default]
    None,
    AgentSkillInstalled(String),
    Replaced,
}

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

enum NestedProjection {
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

fn add_projection_cause(action: &AddAction) -> AddProjectionCause {
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

fn draft_source_path(source: &skit_ui::SourceSnapshot) -> Option<PathBuf> {
    source.is_draft.then(|| source.path.clone())
}

/// Keep both environment text and the spelling that filesystem operations return.
fn ambient_path_spellings(paths: impl IntoIterator<Item = PathBuf>) -> BTreeSet<String> {
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
    fn new(
        profile: &str,
        profile_root: &Path,
        service: &LibraryService<FileStore>,
        file_picker_files: &BTreeSet<PathBuf>,
    ) -> Result<Self, String> {
        let mut ambient_roots = vec![
            super::tui_walker_bundle::checkout_root()?,
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

    fn leak_oracle_facts(&self) -> LeakOracleFacts {
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

    fn refresh(&mut self, service: &LibraryService<FileStore>) -> Result<(), String> {
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

    fn register_event_artifacts(&mut self, event: &PortEvent) {
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
    fn resolved_path<'p>(&self, path: &'p Path) -> Cow<'p, Path> {
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

    fn register_draft_path(&mut self, path: &Path) {
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

    fn register_owned_transient_path(&mut self, path: &Path, kind: &str, prefix: &str) {
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

    fn normalize_library_json(&mut self, mut value: Value) -> Result<Value, String> {
        self.normalize_library_surface(&mut value)?;
        self.normalize_workflow_artifacts(&mut value)?;
        self.normalize_modal_artifacts(&mut value)?;
        self.normalize_install_completion_status(&mut value)?;
        Ok(value)
    }

    fn normalize_library_surface(&mut self, value: &mut Value) -> Result<(), String> {
        self.normalize_library_details(value);
        self.normalize_library_scan(value)
    }

    fn normalize_library_scan(&self, value: &mut Value) -> Result<(), String> {
        self.normalize_entry_summary_targets(value);
        for pointer in ["/diagnostics/*/message", "/scan/diagnostics/*/message"] {
            self.normalize_host_text_pointer(value, pointer)?;
        }
        Ok(())
    }

    fn normalize_path_pointer(&self, value: &mut Value, pointer: &str) -> Result<(), String> {
        rewrite_json_pointer_matches(value, pointer, |matched| {
            self.normalize_path_value(matched);
            Ok(())
        })
    }

    fn normalize_session_path_value(&self, value: &mut Value) -> Result<(), String> {
        let path = decode_session_path_value(value)?;
        if self.is_registered_path(&path) {
            *value = Value::String(self.normalize_path(&path));
        }
        Ok(())
    }

    fn normalize_host_text_pointer(&self, value: &mut Value, pointer: &str) -> Result<(), String> {
        rewrite_json_pointer_matches(value, pointer, |matched| {
            self.normalize_host_text_value(matched);
            Ok(())
        })
    }

    fn normalize_host_text_value(&self, value: &mut Value) {
        if let Value::String(text) = value {
            *text = self.normalize_host_text(text);
        }
    }

    fn project_checkpoint_cause(
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

    fn project_typed_library_state(&mut self, state: &mut LibraryState) -> Result<(), String> {
        let value = serde_json::to_value(&*state).map_err(|error| error.to_string())?;
        self.reconcile_add_provenance(&value)?;
        self.reconcile_status_provenance(&value)?;
        self.reconcile_runner_status_provenance(&value);
        let value = self.normalize_library_json(value)?;
        *state = deserialize_canonical(value, "LibraryState")?;
        Ok(())
    }

    fn record_add_projection_cause(&mut self, action: &Action) {
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

    fn reconcile_status_provenance(&mut self, state: &Value) -> Result<(), String> {
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

    fn reconcile_add_provenance(&mut self, state: &Value) -> Result<(), String> {
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

    fn require_exact_picker_file(&self, raw: &str) -> Result<&Path, String> {
        let path = Path::new(raw);
        self.file_picker_files
            .iter()
            .find(|candidate| candidate.as_os_str() == path.as_os_str())
            .map(PathBuf::as_path)
            .ok_or_else(|| "a picked Add source path is not an exact seeded file".to_owned())
    }

    fn add_source_path_projection(&self) -> Result<Option<(String, String)>, String> {
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

    fn project_session_value(&mut self, session: &mut Value) -> Result<(), String> {
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

    fn project_typed_action(&mut self, action: &mut Action) -> Result<(), String> {
        let mut value = serde_json::to_value(&*action).map_err(|error| error.to_string())?;
        self.project_typed_action_value(&*action, &mut value)?;
        *action = deserialize_canonical(value, "Action")?;
        Ok(())
    }

    fn project_typed_action_value(
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

    fn project_typed_effect(&mut self, effect: &mut Effect) -> Result<(), String> {
        let mut value = serde_json::to_value(&*effect).map_err(|error| error.to_string())?;
        self.project_typed_effect_value(&*effect, &mut value)?;
        *effect = deserialize_canonical(value, "Effect")?;
        Ok(())
    }

    fn project_typed_effect_value(
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

    fn project_typed_screen(&mut self, screen: &Screen, value: &mut Value) -> Result<(), String> {
        let expected = screen_tag(screen);
        let actual = external_tag(value)?;
        if actual != expected {
            return Err(format!(
                "expected serialized Screen tag {expected}, but found {actual}"
            ));
        }
        self.project_serialized_screen(value)
    }

    fn project_serialized_screen(&mut self, value: &mut Value) -> Result<(), String> {
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

    fn project_typed_add_action(
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

    fn normalize_action_json(&mut self, mut value: Value) -> Result<Value, String> {
        let mut action: Action = deserialize_canonical(value, "Action")?;
        self.project_typed_action(&mut action)?;
        value = serde_json::to_value(action).map_err(|error| error.to_string())?;
        Ok(value)
    }

    fn normalize_draft_list(&mut self, mut value: Value) -> Result<Value, String> {
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

    fn normalize_run_screen(&self, screen: &mut Value) -> Result<(), String> {
        let Some(run) = screen.get_mut("run") else {
            return Ok(());
        };
        self.normalize_run_form(run)
    }

    fn normalize_run_form(&self, run: &mut Value) -> Result<(), String> {
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

    fn normalize_settings_screen(&self, screen: &mut Value) {
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

    fn normalize_preferences_screen(&self, screen: &mut Value) -> Result<(), String> {
        let Some(preferences) = screen.get_mut("preferences") else {
            return Ok(());
        };
        self.normalize_preferences_view(preferences)
    }

    fn normalize_preferences_view(&self, preferences: &mut Value) -> Result<(), String> {
        self.normalize_path_pointer(preferences, "/agent_skill_install/targets/*/base")
    }

    fn normalize_health_view(&self, health: &mut Value) -> Result<(), String> {
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

    fn normalize_health_rebuilt_action(&self, payload: &mut Value) -> Result<(), String> {
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

    fn normalize_runner_manager_view(&self, runners: &mut Value) -> Result<(), String> {
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

    fn normalize_agent_skill_install_text(&self, value: &mut Value) -> Result<bool, String> {
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

    fn normalize_path_value(&self, value: &mut Value) {
        if let Value::String(path) = value {
            *path = self.normalize_registered_path_text(path);
        }
    }

    fn normalize_add_screen(&mut self, screen: &mut Value) -> Result<(), String> {
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

    fn normalize_source_snapshot(&mut self, value: &mut Value) -> Result<(), String> {
        self.normalize_artifact_common(value)?;
        if let Some(source_record) = value.get_mut("source_record") {
            self.normalize_path_value(source_record);
        }
        Ok(())
    }

    fn normalize_draft_summary(&mut self, value: &mut Value) -> Result<(), String> {
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

    fn normalize_artifact_object(&mut self, value: &mut Value) -> Result<(), String> {
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
    fn is_registered_path(&self, path: &Path) -> bool {
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

    fn normalize_argument(&self, text: &str) -> String {
        self.normalize_registered_path_text(text)
    }

    fn normalize_host_text(&self, text: &str) -> String {
        replace_longest_text_tokens(text, &self.text_artifacts, accept_host_path_token)
    }

    fn normalize_os_argument(&self, value: &OsStr) -> String {
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
    fn normalize_path(&self, path: &Path) -> String {
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

    fn byte_view(&self, path: &Path, bytes: &[u8]) -> ByteView {
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

fn draft_projection_suffix(path: &Path) -> String {
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

fn review_default_name(path: &Path, kind: &str) -> String {
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

fn decode_session_path_value(value: &Value) -> Result<PathBuf, String> {
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

fn require_external_unit(value: &Value, expected: &str) -> Result<(), String> {
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

fn external_tag(value: &Value) -> Result<&str, String> {
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

fn screen_tag(screen: &Screen) -> &'static str {
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

fn external_payload_mut<'a>(value: &'a mut Value, expected: &str) -> Result<&'a mut Value, String> {
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

fn require_internal_tag(value: &Value, field: &str, expected: &str) -> Result<(), String> {
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

fn project_nested_shape(value: &mut Value, projection: NestedProjection) -> Result<(), String> {
    match projection {
        NestedProjection::Unit(tag) => require_external_unit(value, tag),
        NestedProjection::Payload(tag) => external_payload_mut(value, tag).map(|_| ()),
    }
}

fn deserialize_canonical<T>(value: Value, label: &str) -> Result<T, String>
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

fn snapshot_tree(
    roots: &ProductRoots,
    external_root: &Path,
    paths: &mut PathMap,
    modes: &ObservationModeProvenance,
) -> Result<Vec<TreeRecord>, String> {
    let mut output = Vec::new();
    for root in [
        roots.data.as_path(),
        roots.state.as_path(),
        roots.config.as_path(),
        roots
            .home
            .as_deref()
            .expect("the walker profile has a home"),
        roots.cwd.as_path(),
        external_root,
    ] {
        snapshot_path(root, paths, modes, &mut output)?;
    }
    output.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(output)
}

fn ensure_system_temp_empty(system_temp: &Path, paths: &PathMap) -> Result<(), String> {
    let entries = fs::read_dir(system_temp)
        .map_err(|error| format!("could not inspect walker system temp: {error}"))?
        .map(|entry| entry.map(|entry| entry.path()));
    if let Some(residual) = first_system_temp_residual(entries)? {
        return Err(format!(
            "walker system temp retained an artifact: {}",
            paths.normalize_path(&residual)
        ));
    }
    Ok(())
}

fn first_system_temp_residual(
    entries: impl IntoIterator<Item = io::Result<PathBuf>>,
) -> Result<Option<PathBuf>, String> {
    let mut residuals = entries
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not inspect walker system temp: {error}"))?;
    residuals.sort();
    Ok(residuals.into_iter().next())
}

fn snapshot_path(
    path: &Path,
    paths: &mut PathMap,
    modes: &ObservationModeProvenance,
    output: &mut Vec<TreeRecord>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let mapped = paths.normalize_path(path);
    let normalized_path = mapped
        .strip_prefix(&format!("{}/", paths.profile_label))
        .expect("a tree root stays below the walker profile")
        .to_owned();
    let mode = modes.mode(path, &metadata)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|error| error.to_string())?;
        output.push(TreeRecord {
            path: normalized_path,
            kind: "symlink".to_owned(),
            readonly: metadata.permissions().readonly(),
            mode,
            target: Some(paths.normalize_path(&target)),
            content: None,
        });
        return Ok(());
    }
    if metadata.is_file() {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        output.push(TreeRecord {
            path: normalized_path,
            kind: "file".to_owned(),
            readonly: metadata.permissions().readonly(),
            mode,
            target: None,
            content: Some(paths.byte_view(path, &bytes)),
        });
        return Ok(());
    }
    output.push(TreeRecord {
        path: normalized_path,
        kind: "directory".to_owned(),
        readonly: metadata.permissions().readonly(),
        mode,
        target: None,
        content: None,
    });
    let mut children = fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        snapshot_path(&child.path(), paths, modes, output)?;
    }
    Ok(())
}

#[cfg(unix)]
fn portable_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
fn portable_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
fn escaped_os(value: &OsStr) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    value
        .as_bytes()
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte).map(char::from))
        .collect()
}

fn escaped_wide_units(units: &[u16]) -> String {
    use std::fmt::Write as _;

    // Windows reserves `\` as a separator, so `\uXXXX` cannot collide with a literal path
    // component. Fixed-width units also keep distinct unpaired surrogates injective.
    let mut escaped = String::with_capacity(units.len().saturating_mul(6));
    for unit in units {
        write!(escaped, "\\u{unit:04x}").expect("writing to a String cannot fail");
    }
    escaped
}

#[cfg(windows)]
fn escaped_os(value: &OsStr) -> String {
    use std::os::windows::ffi::OsStrExt as _;

    value.to_str().map_or_else(
        || escaped_wide_units(&value.encode_wide().collect::<Vec<_>>()),
        str::to_owned,
    )
}

#[cfg(not(any(unix, windows)))]
fn escaped_os(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn escaped_path(path: &Path) -> String {
    path.components()
        .map(|component| escaped_os(component.as_os_str()))
        .collect::<Vec<_>>()
        .join("/")
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn readable_plan_files(plan: &LaunchPlan) -> Vec<(PathBuf, Vec<u8>)> {
    std::iter::once(plan.program.as_os_str())
        .chain(plan.args.iter().map(OsStr::new))
        .filter_map(|argument| {
            let path = PathBuf::from(argument);
            fs::read(&path).ok().map(|bytes| (path, bytes))
        })
        .collect()
}

fn byte_value(bytes: &[u8]) -> Value {
    match std::str::from_utf8(bytes) {
        Ok(text) => json!({ "encoding": "utf8", "data": text }),
        Err(_) => json!({ "encoding": "hex", "data": encode_hex(bytes) }),
    }
}

fn io_error_value(error: &io::Error) -> Value {
    json!({
        "error": {
            "kind": format!("{:?}", error.kind()),
            "reason": error.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::{BTreeMap, BTreeSet},
        fs,
        fs::OpenOptions,
        io,
        path::{Path, PathBuf},
        rc::Rc,
    };

    use ratatui_core::{backend::TestBackend, terminal::Terminal};
    use ratatui_crossterm::crossterm::event::Event;
    use serde_json::{Value, json};
    use skit_application::{
        CreateEntry, EntryPayload, SourcePermissions, form_state::FormStateService,
        preferences::PreferencesChangeSet,
    };
    use skit_domain::{
        EntryKind, EntrySettings, Slug, StorageMode,
        parameters::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterValue},
    };
    use skit_i18n::{Locale, Localize, Message, format_text};
    use skit_language::write_managed_params;
    use skit_runtime::{
        DependencyCommandOutput, InjectedCommandOutput, InjectedCommandUnavailable,
        JavaScriptSyntaxGateOutput, JavaScriptSyntaxGateUnavailable,
    };
    use skit_store::{AgentSkillInstallPoint, FileConfigStore, FileFormStateStore, PromptRunner};
    use skit_tui::{
        AddControlId, EventHandling, LocalActionTarget, TuiSession, ViewGeometry,
        render_with_session,
    };
    use skit_tui_walker_support::engine::CheckpointCauseProjection;
    use skit_tui_walker_support::sandbox::{
        STABLE_SANDBOX_NAMESPACE, SafeProfileId, SandboxEvidenceState, SandboxMode,
        SandboxPlatform, profile_lease_path,
    };
    // The stable sandbox supports Linux and Windows only. Its tests use these items.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use skit_tui_walker_support::sandbox::{
        NAMESPACE_MARKER_FILE, NamespaceMarker, ParentInitLockMarker, ProfileLeaseMarker,
        SANDBOX_MARKER_FILE, SandboxMarker, SandboxRoots, profile_cleanup_path,
        profile_sandbox_path,
    };
    use skit_ui::{
        Action, AddAction, AddWorkflowState, DraftKind, Effect, FieldValue, FormPurpose,
        HostRequest, LibraryState, PreferencesAction, PreferencesEffect, ReviewDefaults,
        SourceSnapshot,
    };
    // Only the Linux non-UTF-8 contract picks a kind through the typed serde boundary.
    #[cfg(target_os = "linux")]
    use skit_ui::KnownEntryKind;

    use super::{
        AllocationMaximums, HostObservation, PortEvent, PrivateModeFailure, PrivateModeTarget,
        RealWalkerHost, RecordingAdapters, WalkerDirectoryRoot, WalkerDirectorySeed,
        WalkerExternalReferenceSeed, WalkerExternalSeed, WalkerFormSeed, WalkerLastRunSeed,
        WalkerSeedSpec, encode_hex,
    };
    use crate::cli::tui_host::{
        FileAllocator as _, PrivateDirectoryPurpose, SYSTEM_FILE_ALLOCATOR, TempLocation,
        TemporaryFilePurpose,
    };
    use crate::run::{StageWriteFaultGuard, new_injected_file_with_allocator};

    #[cfg(unix)]
    fn restrictive_umask_child_profile() -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt as _;

        let template = std::env::var_os("LLVM_PROFILE_FILE")?;
        let parent = Path::new(&template)
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let template_prefix = Path::new(&template)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let profile_prefix = format!(
            "{}umask-walker-",
            template_prefix.split('%').next().unwrap_or_default()
        );
        let profile = tempfile::Builder::new()
            .prefix(&profile_prefix)
            .suffix(".profraw")
            .tempfile_in(parent)
            .unwrap();
        fs::set_permissions(profile.path(), fs::Permissions::from_mode(0o600)).unwrap();
        let (file, path) = profile.keep().unwrap();
        drop(file);
        Some(path)
    }

    #[cfg(not(unix))]
    fn restrictive_umask_child_profile() -> Option<PathBuf> {
        None
    }

    #[cfg(windows)]
    fn windows_file_identity(path: &Path) -> (u64, u128) {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .unwrap();
        let identity = fs_id::FileID::new(&file).unwrap();
        (identity.storage_id(), identity.internal_file_id())
    }

    fn expect_authored_draft_source(action: Action) -> SourceSnapshot {
        match action {
            Action::Add(AddAction::DraftEdited {
                result: Ok(Some(source)),
                ..
            }) => source,
            _ => panic!("the writing editor must retain the authored draft"),
        }
    }

    fn expect_add_action(action: Action) -> AddAction {
        match action {
            Action::Add(action) => action,
            _ => panic!("the response must be an Add action"),
        }
    }

    #[test]
    #[should_panic(expected = "the writing editor must retain the authored draft")]
    fn authored_draft_source_extraction_refuses_other_actions() {
        let _ = expect_authored_draft_source(Action::SetStatus("not a draft".to_owned()));
    }

    #[test]
    #[should_panic(expected = "the response must be an Add action")]
    fn c2_add_action_extraction_refuses_other_actions() {
        let _ = expect_add_action(Action::SetStatus("not Add".to_owned()));
    }

    fn recording_allocator(system_temp: &Path) -> RecordingAdapters {
        fs::create_dir_all(system_temp).unwrap();
        RecordingAdapters::new(Rc::new(RefCell::new(Vec::new())), system_temp.to_path_buf())
    }

    fn safe_profile(value: &str) -> SafeProfileId {
        SafeProfileId::try_from(value).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn stable_paths(
        namespace_root: PathBuf,
        profile: SafeProfileId,
    ) -> Result<super::StableSandboxPaths, super::SandboxError> {
        super::StableSandboxNamespace::explicit(namespace_root)
            .and_then(|namespace| super::StableSandboxPaths::new(namespace, profile))
    }

    #[test]
    fn sandbox_errors_keep_typed_causes_and_stable_diagnostics() {
        let path = PathBuf::from("/tmp/skit-ui-walker-v1/leases/error.lock");
        let unsupported = super::SandboxError::UnsupportedPlatform { platform: "macos" };
        assert_eq!(
            unsupported.to_string(),
            "stable walker sandboxes are not supported on macos"
        );
        assert!(std::error::Error::source(&unsupported).is_none());
        let unsupported = super::SandboxError::from(super::SandboxFsError::Unsupported {
            operation: "rename",
            path: path.clone(),
            source: Box::new(io::Error::new(io::ErrorKind::Unsupported, "not supported")),
        });
        assert!(unsupported.to_string().contains("not supported"));
        assert!(std::error::Error::source(&unsupported).is_some());
        let native = super::SandboxError::from(super::SandboxFsError::NativeStatus {
            operation: "rename",
            path: path.clone(),
            status: -1,
        });
        assert!(native.to_string().contains("native status 0xffffffff"));
        assert!(std::error::Error::source(&native).is_none());
        let platform = super::SandboxError::from(super::SandboxFsError::UnsupportedPlatform);
        assert!(platform.to_string().contains(std::env::consts::OS));
        let dual = super::SandboxError::from(super::SandboxFsError::Dual {
            primary: Box::new(super::SandboxFsError::Invalid {
                path: path.clone(),
                reason: "primary evidence".to_owned(),
            }),
            release: Box::new(super::SandboxFsError::Io {
                operation: "unlock",
                path: path.clone(),
                source: io::Error::new(io::ErrorKind::PermissionDenied, "release denied"),
            }),
        });
        assert!(dual.to_string().contains("sandbox release also failed"));
        assert!(std::error::Error::source(&dual).is_some());

        let busy = super::SandboxError::Busy {
            profile: safe_profile("error"),
            path: path.clone(),
        };
        assert!(busy.to_string().contains("profile error is busy"));
        let invalid = super::SandboxError::invalid(&path, "invalid evidence");
        assert!(invalid.to_string().contains("invalid evidence"));
        let contract_error =
            super::map_contract_result::<(), _>(&path, Err("contract error")).unwrap_err();
        assert!(matches!(
            contract_error,
            super::SandboxError::InvalidEvidence { reason, .. }
                if reason == "contract error"
        ));
        let io = super::SandboxError::io(
            "lock",
            &path,
            io::Error::new(io::ErrorKind::PermissionDenied, "lock denied"),
        );
        assert!(
            io.to_string()
                .contains("could not lock walker sandbox path")
        );
        assert!(std::error::Error::source(&io).is_some());
        let injected = super::SandboxError::InjectedFault {
            point: super::SandboxFaultPoint::RemoveMarker,
        };
        assert!(injected.to_string().contains("RemoveMarker"));

        let seed = super::StableHostPrimaryError::Seed("seed denied".to_owned());
        assert!(seed.to_string().contains("seed denied"));
        let sandbox = super::StableHostError::Sandbox(busy);
        assert!(sandbox.to_string().contains("is busy"));
        let primary = super::StableHostError::Primary {
            primary: super::StableHostPrimaryError::Seed("seed only".to_owned()),
            cleanup: None,
        };
        assert_eq!(
            primary.to_string(),
            "could not seed the stable walker host: seed only"
        );
        let combined = super::StableHostError::Primary {
            primary: super::StableHostPrimaryError::Seed("primary".to_owned()),
            cleanup: Some(injected),
        };
        assert!(combined.to_string().contains("sandbox cleanup also failed"));
    }

    #[test]
    fn retained_evidence_and_release_results_keep_each_typed_state() {
        let absent = super::Evidence::<u8>::Absent;
        assert_eq!(absent.state(), SandboxEvidenceState::Absent);
        assert_eq!(absent.reason(), None);
        assert_eq!(absent.into_valid(), None);

        let valid = super::Evidence::Valid(7_u8);
        assert_eq!(valid.state(), SandboxEvidenceState::Valid);
        assert_eq!(valid.reason(), None);
        assert_eq!(valid.into_valid(), Some(7));

        let invalid = super::Evidence::<u8>::Invalid {
            reason: "invalid evidence".to_owned(),
        };
        assert_eq!(invalid.state(), SandboxEvidenceState::Invalid);
        assert_eq!(invalid.reason(), Some("invalid evidence"));
        assert_eq!(invalid.into_valid(), None);

        let primary = || super::SandboxError::invalid(Path::new("primary"), "primary");
        let release = || super::SandboxError::invalid(Path::new("release"), "release");
        assert!(super::combine_sandbox_release(Ok(()), Ok(())).is_ok());
        assert!(matches!(
            super::combine_sandbox_release(Err(primary()), Ok(())),
            Err(super::SandboxError::InvalidEvidence { path, .. })
                if path == Path::new("primary")
        ));
        assert!(matches!(
            super::combine_sandbox_release(Ok(()), Err(release())),
            Err(super::SandboxError::InvalidEvidence { path, .. })
                if path == Path::new("release")
        ));
        assert!(matches!(
            super::combine_sandbox_release(Err(primary()), Err(release())),
            Err(super::SandboxError::Dual { .. })
        ));
    }

    #[test]
    fn namespace_shape_validation_fails_closed() {
        assert!(
            super::StableSandboxNamespace::explicit(PathBuf::from(STABLE_SANDBOX_NAMESPACE))
                .is_err()
        );
        assert!(
            super::StableSandboxNamespace::explicit(PathBuf::from("/tmp/wrong-namespace")).is_err()
        );
        assert!(
            super::StableSandboxNamespace::explicit(PathBuf::from("/tmp/../skit-ui-walker-v1",))
                .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn invalid_namespace_literals_return_typed_errors_without_panics() {
        use std::os::unix::ffi::OsStringExt as _;

        let parent = tempfile::TempDir::new().unwrap();
        let parent_literal = parent.path().to_str().unwrap();
        let mut paths = vec![
            PathBuf::from(format!(
                "{parent_literal}/newline\n/{STABLE_SANDBOX_NAMESPACE}"
            )),
            PathBuf::from(format!(
                "{parent_literal}/carriage\r/{STABLE_SANDBOX_NAMESPACE}"
            )),
            PathBuf::from(format!("{parent_literal}/nul\0/{STABLE_SANDBOX_NAMESPACE}")),
            PathBuf::from(format!("{parent_literal}//{STABLE_SANDBOX_NAMESPACE}")),
            PathBuf::from(format!("{parent_literal}/{STABLE_SANDBOX_NAMESPACE}/")),
            PathBuf::from(format!("{parent_literal}/./{STABLE_SANDBOX_NAMESPACE}")),
            PathBuf::from(format!(
                "{parent_literal}/parent/../{STABLE_SANDBOX_NAMESPACE}"
            )),
        ];
        paths.push(
            parent
                .path()
                .join(std::ffi::OsString::from_vec(b"non-utf8-\xff".to_vec()))
                .join(STABLE_SANDBOX_NAMESPACE),
        );

        for path in paths {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                super::StableSandboxNamespace::explicit(path.clone())
            }));
            assert!(outcome.is_ok(), "panicked for namespace path {path:?}");
            assert!(
                outcome.unwrap().is_err(),
                "accepted namespace path {path:?}"
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_paths_cache_exact_typed_marker_bytes() {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile("marker-derivation");
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            profile.clone(),
        )
        .unwrap();
        let namespace = paths.namespace_root.to_str().unwrap();
        assert_eq!(
            paths.parent_lock_bytes(),
            ParentInitLockMarker::new(paths.platform, namespace)
                .unwrap()
                .canonical_bytes()
                .unwrap()
        );
        assert_eq!(
            paths.namespace_marker_bytes(),
            NamespaceMarker::new(paths.platform, namespace)
                .unwrap()
                .canonical_bytes()
                .unwrap()
        );
        assert_eq!(
            paths.lease_marker_bytes(),
            ProfileLeaseMarker::new(paths.platform, profile.clone(), namespace)
                .unwrap()
                .canonical_bytes()
                .unwrap()
        );
        let sandbox_marker = SandboxMarker::new(paths.platform, profile, namespace).unwrap();
        assert_eq!(&paths.roots, sandbox_marker.roots());
        assert_eq!(
            paths.sandbox_marker_bytes(),
            sandbox_marker.canonical_bytes().unwrap()
        );

        let rejected_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let rejected = super::StableSandboxPaths::new_with_contract_builder(
            super::StableSandboxNamespace::explicit(rejected_root.clone()).unwrap(),
            safe_profile("marker-builder-error"),
            |_, _, _| Err("injected marker contract failure"),
        )
        .unwrap_err();
        assert!(matches!(
            rejected,
            super::SandboxError::InvalidEvidence { path, reason }
                if path == rejected_root && reason == "injected marker contract failure"
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn marker_and_ticket_reverse_checks_refuse_real_namespace_replacements() {
        let temporary = tempfile::TempDir::new().unwrap();
        let directory = super::PinnedDirectory::open_ambient_parent(temporary.path()).unwrap();
        let marker_name = super::ChildName::literal("marker");
        let marker =
            super::publish_relative_marker(&directory, &marker_name, b"marker bytes").unwrap();
        let original_identity = marker.identity();
        drop(marker);
        let parked = temporary.path().join("parked-marker");
        let replacement_identity = std::cell::Cell::new(None);

        let replaced = super::validate_relative_marker_with_hook(
            &directory,
            &marker_name,
            b"marker bytes",
            |directory, marker, name| {
                fs::rename(marker.path(), &parked).unwrap();
                let replacement = directory.create_file(name).unwrap();
                replacement
                    .publish_bytes(directory, name, b"marker bytes")
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
        );

        assert!(matches!(
            replaced,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert_eq!(
            directory
                .open_file(&super::ChildName::literal("parked-marker"), false)
                .unwrap()
                .identity(),
            original_identity
        );
        assert_ne!(replacement_identity.get().unwrap(), original_identity);
        assert_eq!(fs::read(&parked).unwrap(), b"marker bytes");
        assert_eq!(
            fs::read(temporary.path().join("marker")).unwrap(),
            b"marker bytes"
        );

        let removed_name = super::ChildName::literal("removed-marker");
        drop(super::publish_relative_marker(&directory, &removed_name, b"removed bytes").unwrap());
        let removed = super::validate_relative_marker_with_hook(
            &directory,
            &removed_name,
            b"removed bytes",
            |_, marker, _| {
                fs::remove_file(marker.path()).unwrap();
            },
        );
        assert!(matches!(
            removed,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert!(!temporary.path().join("removed-marker").exists());

        let directory_marker = super::ChildName::literal("directory-marker");
        drop(directory.create_directory(&directory_marker).unwrap());
        assert!(matches!(
            super::validate_relative_marker(&directory, &directory_marker, b"marker bytes"),
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let outside = temporary.path().join("outside-marker");
        fs::write(&outside, b"outside bytes").unwrap();
        let linked_marker = super::ChildName::literal("linked-marker");
        std::os::unix::fs::symlink(&outside, temporary.path().join("linked-marker")).unwrap();
        assert!(matches!(
            super::validate_relative_marker(&directory, &linked_marker, b"marker bytes"),
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert_eq!(fs::read(outside).unwrap(), b"outside bytes");

        let first_name = super::ChildName::literal("first-directory");
        let first = directory.create_directory(&first_name).unwrap();
        let second_name = super::ChildName::literal("second-directory");
        let second = directory.create_directory(&second_name).unwrap();
        assert!(
            super::require_ticket(
                &directory,
                &super::ChildName::literal("missing-directory"),
                super::SandboxNodeKind::Directory,
                first.identity(),
            )
            .is_err()
        );
        assert!(
            super::require_ticket(
                &directory,
                &first_name,
                super::SandboxNodeKind::RegularFile,
                first.identity(),
            )
            .is_err()
        );
        assert!(
            super::require_ticket(
                &directory,
                &first_name,
                super::SandboxNodeKind::Directory,
                second.identity(),
            )
            .is_err()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn initialization_reverse_read_and_empty_release_fail_closed() {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("init-reverse"),
        )
        .unwrap();
        let guard =
            super::acquire_initialization_guard(&paths, &super::SandboxFaults::default()).unwrap();
        let original_identity = guard.file().identity();
        guard.release(|_| Ok(())).unwrap();
        let parked = parent.path().join("parked-init.lock");
        let replacement_identity = std::cell::Cell::new(None);

        let replaced = super::acquire_initialization_guard_with_hooks(
            &paths,
            &super::SandboxFaults::default(),
            |guard| {
                fs::rename(&paths.init_lock, &parked).unwrap();
                let replacement = guard.parent().create_file(guard.name()).unwrap();
                replacement
                    .publish_bytes(guard.parent(), guard.name(), paths.parent_lock_bytes())
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
            super::release_initialization_guard,
        );

        assert!(replaced.is_err());
        assert_eq!(fs::read(&parked).unwrap(), paths.parent_lock_bytes());
        assert_eq!(
            fs::read(&paths.init_lock).unwrap(),
            paths.parent_lock_bytes()
        );
        assert_ne!(replacement_identity.get().unwrap(), original_identity);

        let empty_parent = tempfile::TempDir::new().unwrap();
        let empty_paths = stable_paths(
            empty_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("empty-release"),
        )
        .unwrap();
        let directory = super::PinnedDirectory::open_ambient_parent(empty_parent.path()).unwrap();
        drop(
            directory
                .create_file(&empty_paths.init_lock_name())
                .unwrap(),
        );

        let release = super::acquire_initialization_guard_with_hooks(
            &empty_paths,
            &super::SandboxFaults::default(),
            |_| {},
            |guard| {
                guard.release_with_ops(
                    |_| Ok(()),
                    |_| {
                        Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "injected init unlock failure",
                        ))
                    },
                    |_| Ok(()),
                )
            },
        );

        assert!(matches!(
            release,
            Err(super::SandboxError::Io {
                operation: "unlock initialization file",
                ..
            })
        ));
        assert_eq!(fs::read(&empty_paths.init_lock).unwrap(), b"");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn namespace_marker_publish_collision_retains_the_unmarked_skeleton() {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("marker-publish-collision"),
        )
        .unwrap();
        let inserted_identity = std::cell::Cell::new(None);

        let result = super::initialize_namespace_retained_with_hooks(
            &paths,
            &super::SandboxFaults::default(),
            |namespace, marker_name| {
                let inserted = namespace.create_directory(marker_name).unwrap();
                inserted_identity.set(Some(inserted.identity()));
            },
            super::no_before_namespace_release_hook,
            super::no_after_namespace_release_hook,
        );

        assert!(matches!(
            result,
            Err(super::SandboxError::Io { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists
        ));
        assert!(paths.leases_root.is_dir());
        assert!(paths.sandboxes_root.is_dir());
        let parent_handle = super::PinnedDirectory::open_ambient_parent(parent.path()).unwrap();
        let namespace = parent_handle
            .open_directory(&paths.namespace_name())
            .unwrap();
        let inserted = namespace
            .open_directory(&paths.namespace_marker_name())
            .unwrap();
        assert_eq!(inserted.identity(), inserted_identity.get().unwrap());
        assert!(!paths.namespace_root.join(NAMESPACE_MARKER_FILE).is_file());

        assert!(
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).is_err()
        );
        assert_eq!(
            namespace
                .open_directory(&paths.namespace_marker_name())
                .unwrap()
                .identity(),
            inserted_identity.get().unwrap()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn initialization_final_validation_refuses_namespace_and_marker_replacements() {
        let namespace_parent = tempfile::TempDir::new().unwrap();
        let namespace_paths = stable_paths(
            namespace_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("post-release-namespace"),
        )
        .unwrap();
        let parked_namespace = namespace_parent.path().join("parked-namespace");
        let original_namespace_identity = std::cell::Cell::new(None);
        let replacement_namespace_identity = std::cell::Cell::new(None);

        let namespace_result = super::initialize_namespace_retained_with_hooks(
            &namespace_paths,
            &super::SandboxFaults::default(),
            |_, _| {},
            |_, _, _, _, _| {},
            |parent, namespace, _, _, _| {
                original_namespace_identity.set(Some(namespace.identity()));
                fs::rename(namespace.path(), &parked_namespace).unwrap();
                let replacement = parent
                    .create_directory(&namespace_paths.namespace_name())
                    .unwrap();
                replacement_namespace_identity.set(Some(replacement.identity()));
            },
        );

        assert!(matches!(
            namespace_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let parent_handle =
            super::PinnedDirectory::open_ambient_parent(namespace_parent.path()).unwrap();
        let parked = parent_handle
            .open_directory(&super::ChildName::literal("parked-namespace"))
            .unwrap();
        let replacement = parent_handle
            .open_directory(&namespace_paths.namespace_name())
            .unwrap();
        assert_eq!(
            parked.identity(),
            original_namespace_identity.get().unwrap()
        );
        assert_eq!(
            replacement.identity(),
            replacement_namespace_identity.get().unwrap()
        );
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(
            fs::read(parked_namespace.join(NAMESPACE_MARKER_FILE)).unwrap(),
            namespace_paths.namespace_marker_bytes()
        );
        assert!(replacement.tickets().unwrap().is_empty());

        let marker_parent = tempfile::TempDir::new().unwrap();
        let marker_paths = stable_paths(
            marker_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("post-release-marker"),
        )
        .unwrap();
        let parked_marker = marker_paths.namespace_root.join("parked-marker");
        let original_marker_identity = std::cell::Cell::new(None);
        let replacement_marker_identity = std::cell::Cell::new(None);

        let marker_result = super::initialize_namespace_retained_with_hooks(
            &marker_paths,
            &super::SandboxFaults::default(),
            |_, _| {},
            |_, _, _, _, _| {},
            |_, namespace, _, _, marker| {
                original_marker_identity.set(Some(marker.identity()));
                fs::rename(marker.path(), &parked_marker).unwrap();
                let replacement = namespace
                    .create_file(&marker_paths.namespace_marker_name())
                    .unwrap();
                replacement
                    .publish_bytes(
                        namespace,
                        &marker_paths.namespace_marker_name(),
                        marker_paths.namespace_marker_bytes(),
                    )
                    .unwrap();
                replacement_marker_identity.set(Some(replacement.identity()));
            },
        );

        assert!(matches!(
            marker_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let parent_handle =
            super::PinnedDirectory::open_ambient_parent(marker_parent.path()).unwrap();
        let namespace = parent_handle
            .open_directory(&marker_paths.namespace_name())
            .unwrap();
        let parked = namespace
            .open_file(&super::ChildName::literal("parked-marker"), false)
            .unwrap();
        let replacement = namespace
            .open_file(&marker_paths.namespace_marker_name(), false)
            .unwrap();
        assert_eq!(parked.identity(), original_marker_identity.get().unwrap());
        assert_eq!(
            replacement.identity(),
            replacement_marker_identity.get().unwrap()
        );
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(
            fs::read(&parked_marker).unwrap(),
            marker_paths.namespace_marker_bytes()
        );
        assert_eq!(
            fs::read(marker_paths.namespace_root.join(NAMESPACE_MARKER_FILE)).unwrap(),
            marker_paths.namespace_marker_bytes()
        );
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&parked_marker).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(marker_paths.namespace_root.join(NAMESPACE_MARKER_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let corrupt_parent = tempfile::TempDir::new().unwrap();
        let corrupt_paths = stable_paths(
            corrupt_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("post-release-corrupt-marker"),
        )
        .unwrap();
        let expected_identity = std::cell::Cell::new(None);
        let corrupt_bytes = b"corrupt marker bytes";
        let corrupt_result = super::initialize_namespace_retained_with_hooks(
            &corrupt_paths,
            &super::SandboxFaults::default(),
            super::no_namespace_marker_publish_hook,
            super::no_before_namespace_release_hook,
            |_, _, _, _, marker| {
                use std::io::{Seek as _, Write as _};

                expected_identity.set(Some(marker.identity()));
                let mut file = marker.file().try_clone().unwrap();
                file.set_len(0).unwrap();
                file.seek(io::SeekFrom::Start(0)).unwrap();
                file.write_all(corrupt_bytes).unwrap();
                file.sync_all().unwrap();
            },
        );

        assert!(matches!(
            corrupt_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let parent_handle =
            super::PinnedDirectory::open_ambient_parent(corrupt_parent.path()).unwrap();
        let namespace = parent_handle
            .open_directory(&corrupt_paths.namespace_name())
            .unwrap();
        let marker = namespace
            .open_file(&corrupt_paths.namespace_marker_name(), false)
            .unwrap();
        assert_eq!(marker.identity(), expected_identity.get().unwrap());
        assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
        assert!(
            super::initialize_namespace_retained(&corrupt_paths, &super::SandboxFaults::default(),)
                .is_err()
        );
        assert_eq!(
            namespace
                .open_file(&corrupt_paths.namespace_marker_name(), false)
                .unwrap()
                .identity(),
            expected_identity.get().unwrap()
        );
        assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn profile_lease_validation_refuses_replacement_after_lock() {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("lease-after-lock"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap();
        let original = super::acquire_profile_lease_retained(
            &paths,
            &handles,
            &super::SandboxFaults::default(),
        )
        .unwrap();
        let original_identity = original.file().identity();
        original.release(handles.leases()).unwrap();
        let parked_name = super::ChildName::literal("parked-lease.lock");
        let parked_path = paths.leases_root.join(parked_name.as_os_str());
        let replacement_identity = std::cell::Cell::new(None);

        let result = super::acquire_profile_lease_retained_with_hooks(
            &paths,
            &handles,
            &super::SandboxFaults::default(),
            |lease, leases| {
                fs::rename(lease.file().path(), &parked_path).unwrap();
                let replacement = leases.create_file(&paths.lease_name()).unwrap();
                replacement
                    .publish_bytes(leases, &paths.lease_name(), paths.lease_marker_bytes())
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
            super::publish_profile_lease_marker,
            super::no_profile_lease_hook,
            super::release_profile_lease,
        );

        let error = result.unwrap_err();
        assert!(matches!(
            error,
            super::SandboxError::Dual { primary, release }
                if matches!(*primary, super::SandboxError::InvalidEvidence { .. })
                    && matches!(*release, super::SandboxError::InvalidEvidence { .. })
        ));
        let parked = handles.leases().open_file(&parked_name, false).unwrap();
        let replacement = handles
            .leases()
            .open_file(&paths.lease_name(), false)
            .unwrap();
        assert_eq!(parked.identity(), original_identity);
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(fs::read(parked.path()).unwrap(), paths.lease_marker_bytes());
        assert_eq!(
            fs::read(replacement.path()).unwrap(),
            paths.lease_marker_bytes()
        );
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn profile_lease_publish_and_release_failures_retain_created_evidence() {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum Failure {
            TruncateAndUnlock,
            Write,
            Sync,
        }

        for failure in [Failure::TruncateAndUnlock, Failure::Write, Failure::Sync] {
            let parent = tempfile::TempDir::new().unwrap();
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                safe_profile(match failure {
                    Failure::TruncateAndUnlock => "lease-truncate-unlock",
                    Failure::Write => "lease-write",
                    Failure::Sync => "lease-sync",
                }),
            )
            .unwrap();
            let (handles, namespace_marker) =
                super::initialize_namespace_retained(&paths, &super::SandboxFaults::default())
                    .unwrap();
            let created_identity = std::cell::Cell::new(None);

            let result = super::acquire_profile_lease_retained_with_hooks(
                &paths,
                &handles,
                &super::SandboxFaults::default(),
                super::no_profile_lease_hook,
                |file, parent, name, bytes| {
                    created_identity.set(Some(file.identity()));
                    match failure {
                        Failure::TruncateAndUnlock => file.publish_bytes_with_ops(
                            parent,
                            name,
                            bytes,
                            |_| {
                                Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "injected lease truncate failure",
                                ))
                            },
                            std::io::Write::write_all,
                            fs::File::sync_all,
                        ),
                        Failure::Write => file.publish_bytes_with_ops(
                            parent,
                            name,
                            bytes,
                            |file| file.set_len(0),
                            |_, _| {
                                Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "injected lease write failure",
                                ))
                            },
                            fs::File::sync_all,
                        ),
                        Failure::Sync => file.publish_bytes_with_ops(
                            parent,
                            name,
                            bytes,
                            |file| file.set_len(0),
                            std::io::Write::write_all,
                            |_| {
                                Err(io::Error::new(
                                    io::ErrorKind::PermissionDenied,
                                    "injected lease sync failure",
                                ))
                            },
                        ),
                    }
                },
                super::no_profile_lease_hook,
                |lease, leases| {
                    if failure == Failure::TruncateAndUnlock {
                        lease.release_with_ops(leases, |_| {
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "injected lease unlock failure",
                            ))
                        })
                    } else {
                        lease.release(leases)
                    }
                },
            );

            let error = result.unwrap_err();
            match failure {
                Failure::TruncateAndUnlock => assert!(matches!(
                    error,
                    super::SandboxError::Dual { primary, release }
                        if matches!(*primary, super::SandboxError::Io {
                            operation: "truncate child file",
                            ..
                        }) && matches!(*release, super::SandboxError::Io {
                            operation: "unlock profile lease",
                            ..
                        })
                )),
                Failure::Write => assert!(matches!(
                    error,
                    super::SandboxError::Io {
                        operation: "write child file",
                        ..
                    }
                )),
                Failure::Sync => assert!(matches!(
                    error,
                    super::SandboxError::Io {
                        operation: "sync child file",
                        ..
                    }
                )),
            }
            let retained = handles
                .leases()
                .open_file(&paths.lease_name(), false)
                .unwrap();
            assert_eq!(retained.identity(), created_identity.get().unwrap());
            let expected = if failure == Failure::Sync {
                paths.lease_marker_bytes()
            } else {
                b""
            };
            assert_eq!(fs::read(retained.path()).unwrap(), expected);
            drop(namespace_marker);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn profile_lease_final_read_refuses_post_publish_replacement() {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("lease-after-publish"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap();
        let parked_name = super::ChildName::literal("parked-published-lease.lock");
        let parked_path = paths.leases_root.join(parked_name.as_os_str());
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);

        let result = super::acquire_profile_lease_retained_with_hooks(
            &paths,
            &handles,
            &super::SandboxFaults::default(),
            super::no_profile_lease_hook,
            super::publish_profile_lease_marker,
            |lease, leases| {
                original_identity.set(Some(lease.file().identity()));
                fs::rename(lease.file().path(), &parked_path).unwrap();
                let replacement = leases.create_file(&paths.lease_name()).unwrap();
                replacement
                    .publish_bytes(leases, &paths.lease_name(), paths.lease_marker_bytes())
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
            super::release_profile_lease,
        );

        let error = result.unwrap_err();
        assert!(matches!(
            error,
            super::SandboxError::Dual { primary, release }
                if matches!(*primary, super::SandboxError::InvalidEvidence { .. })
                    && matches!(*release, super::SandboxError::InvalidEvidence { .. })
        ));
        let parked = handles.leases().open_file(&parked_name, false).unwrap();
        let replacement = handles
            .leases()
            .open_file(&paths.lease_name(), false)
            .unwrap();
        assert_eq!(parked.identity(), original_identity.get().unwrap());
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(fs::read(parked.path()).unwrap(), paths.lease_marker_bytes());
        assert_eq!(
            fs::read(replacement.path()).unwrap(),
            paths.lease_marker_bytes()
        );
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn candidate_open_refuses_fixed_child_kinds_and_identity_replacements() {
        for use_link in [false, true] {
            let parent = tempfile::TempDir::new().unwrap();
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                safe_profile(if use_link {
                    "candidate-child-link"
                } else {
                    "candidate-child-file"
                }),
            )
            .unwrap();
            let (handles, namespace_marker) =
                super::initialize_namespace_retained(&paths, &super::SandboxFaults::default())
                    .unwrap();
            create_valid_sandbox_evidence(&paths.sandbox_root, &paths);
            fs::remove_dir(paths.sandbox_root.join("data")).unwrap();
            let outside = parent.path().join("outside-data");
            if use_link {
                fs::write(&outside, b"outside bytes").unwrap();
                std::os::unix::fs::symlink(&outside, paths.sandbox_root.join("data")).unwrap();
            } else {
                write_private_test_file(&paths.sandbox_root.join("data"), b"owned file");
            }

            assert!(matches!(
                super::validate_original_candidate(
                    handles.sandboxes(),
                    &paths.sandbox_name(),
                    &paths,
                ),
                Err(super::SandboxError::InvalidEvidence { .. })
            ));
            if use_link {
                assert_eq!(fs::read(&outside).unwrap(), b"outside bytes");
                assert!(
                    fs::symlink_metadata(paths.sandbox_root.join("data"))
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
            } else {
                assert_eq!(
                    fs::read(paths.sandbox_root.join("data")).unwrap(),
                    b"owned file"
                );
            }
            drop(namespace_marker);
        }

        let child_parent = tempfile::TempDir::new().unwrap();
        let child_paths = stable_paths(
            child_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("candidate-child-swap"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&child_paths, &super::SandboxFaults::default())
                .unwrap();
        create_valid_sandbox_evidence(&child_paths.sandbox_root, &child_paths);
        let parked_child = child_paths.sandbox_root.join("parked-data");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let child_result = super::open_candidate_parts_with_hooks(
            handles.sandboxes(),
            &child_paths.sandbox_name(),
            &child_paths,
            |root, tickets| {
                let data_name = super::ChildName::literal("data");
                let ticket = tickets
                    .iter()
                    .find(|ticket| ticket.name() == &data_name)
                    .unwrap();
                original_identity.set(Some(ticket.identity()));
                fs::rename(root.path().join("data"), &parked_child).unwrap();
                let replacement = root.create_directory(&data_name).unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
            super::no_candidate_root_hook,
        );
        assert!(matches!(
            child_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert_eq!(
            handles
                .sandboxes()
                .open_directory(&child_paths.sandbox_name())
                .unwrap()
                .open_directory(&super::ChildName::literal("parked-data"))
                .unwrap()
                .identity(),
            original_identity.get().unwrap()
        );
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );
        drop(namespace_marker);

        let root_parent = tempfile::TempDir::new().unwrap();
        let root_paths = stable_paths(
            root_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("candidate-root-swap"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&root_paths, &super::SandboxFaults::default())
                .unwrap();
        create_valid_sandbox_evidence(&root_paths.sandbox_root, &root_paths);
        let parked_name = super::ChildName::literal("parked-candidate-root");
        let parked_root = root_paths.sandboxes_root.join(parked_name.as_os_str());
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let root_result = super::open_candidate_parts_with_hooks(
            handles.sandboxes(),
            &root_paths.sandbox_name(),
            &root_paths,
            super::no_candidate_tickets_hook,
            |parent, root, name| {
                original_identity.set(Some(root.identity()));
                fs::rename(root.path(), &parked_root).unwrap();
                let replacement = parent.create_directory(name).unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
        );
        assert!(matches!(
            root_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let parked = handles.sandboxes().open_directory(&parked_name).unwrap();
        let replacement = handles
            .sandboxes()
            .open_directory(&root_paths.sandbox_name())
            .unwrap();
        assert_eq!(parked.identity(), original_identity.get().unwrap());
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(
            fs::read(parked_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            root_paths.sandbox_marker_bytes()
        );
        assert!(replacement.tickets().unwrap().is_empty());
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn candidate_classification_propagates_real_open_io() {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("candidate-open-io"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap();
        create_valid_sandbox_evidence(&paths.sandbox_root, &paths);
        let parked_root = paths.sandboxes_root.join("parked-open-io");

        let result = super::classify_retained_sandbox(
            handles.sandboxes(),
            &paths.sandbox_name(),
            |parent, name| {
                fs::rename(&paths.sandbox_root, &parked_root).unwrap();
                super::validate_original_candidate(parent, name, &paths)
            },
        );

        assert!(matches!(
            result,
            Err(super::SandboxError::Io { source, .. })
                if source.kind() == io::ErrorKind::NotFound
        ));
        assert_eq!(
            fs::read(parked_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        assert!(!paths.sandbox_root.exists());
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fresh_candidate_refuses_marker_collision_and_post_publish_child_replacement() {
        let marker_parent = tempfile::TempDir::new().unwrap();
        let marker_paths = stable_paths(
            marker_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("fresh-marker-collision"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&marker_paths, &super::SandboxFaults::default())
                .unwrap();
        let inserted_identity = std::cell::Cell::new(None);
        let marker_result = super::create_fresh_sandbox_retained_with_hooks(
            &marker_paths,
            &handles,
            &super::SandboxFaults::default(),
            |root, marker_name| {
                let inserted = root.create_directory(marker_name).unwrap();
                inserted_identity.set(Some(inserted.identity()));
            },
            super::no_fresh_sandbox_after_marker_hook,
        );
        assert!(matches!(
            marker_result,
            Err(super::SandboxError::Io { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists
        ));
        let root = handles
            .sandboxes()
            .open_directory(&marker_paths.sandbox_name())
            .unwrap();
        assert_eq!(
            root.open_directory(&marker_paths.sandbox_marker_name())
                .unwrap()
                .identity(),
            inserted_identity.get().unwrap()
        );
        for name in marker_paths.sandbox_child_names() {
            assert!(root.open_directory(&name).is_ok());
        }
        assert!(
            super::prepare_fresh_sandbox_retained(
                &marker_paths,
                &handles,
                &super::SandboxFaults::default(),
            )
            .is_err()
        );
        assert_eq!(
            root.open_directory(&marker_paths.sandbox_marker_name())
                .unwrap()
                .identity(),
            inserted_identity.get().unwrap()
        );
        drop(namespace_marker);

        let child_parent = tempfile::TempDir::new().unwrap();
        let child_paths = stable_paths(
            child_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("fresh-child-replacement"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&child_paths, &super::SandboxFaults::default())
                .unwrap();
        let parked_child = child_paths.sandbox_root.join("parked-data");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let child_result = super::create_fresh_sandbox_retained_with_hooks(
            &child_paths,
            &handles,
            &super::SandboxFaults::default(),
            super::no_fresh_sandbox_before_marker_hook,
            |root| {
                let data_name = super::ChildName::literal("data");
                let original = root.open_directory(&data_name).unwrap();
                original_identity.set(Some(original.identity()));
                drop(original);
                fs::rename(root.path().join("data"), &parked_child).unwrap();
                let replacement = root.create_directory(&data_name).unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
        );
        assert!(matches!(
            child_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert_eq!(
            fs::read(child_paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            child_paths.sandbox_marker_bytes()
        );
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );
        assert!(parked_child.is_dir());
        assert!(child_paths.sandbox_root.join("data").is_dir());
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_original_and_cleanup_validation_refuse_child_replacements() {
        for cleanup in [false, true] {
            let parent = tempfile::TempDir::new().unwrap();
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                safe_profile(if cleanup {
                    "live-cleanup-child"
                } else {
                    "live-original-child"
                }),
            )
            .unwrap();
            let (handles, namespace_marker) =
                super::initialize_namespace_retained(&paths, &super::SandboxFaults::default())
                    .unwrap();
            let (name, root_path) = if cleanup {
                (paths.cleanup_name(), paths.cleanup_root.clone())
            } else {
                (paths.sandbox_name(), paths.sandbox_root.clone())
            };
            create_valid_sandbox_evidence(&root_path, &paths);
            let parked_child = root_path.join("parked-data");
            let original_identity = std::cell::Cell::new(None);
            let replacement_identity = std::cell::Cell::new(None);

            let result = if cleanup {
                let sandbox =
                    super::validate_cleanup_candidate(handles.sandboxes(), &name, &paths).unwrap();
                super::validate_same_cleanup_with_hook(
                    &sandbox,
                    handles.sandboxes(),
                    &name,
                    &paths,
                    |root, child_name, child| {
                        if child_name == &super::ChildName::literal("data") {
                            original_identity.set(Some(child.identity()));
                            fs::rename(child.path(), &parked_child).unwrap();
                            let replacement = root.create_directory(child_name).unwrap();
                            replacement_identity.set(Some(replacement.identity()));
                        }
                    },
                )
            } else {
                let sandbox =
                    super::validate_original_candidate(handles.sandboxes(), &name, &paths).unwrap();
                super::validate_same_original_with_hook(
                    &sandbox,
                    handles.sandboxes(),
                    &name,
                    &paths,
                    |root, child_name, child| {
                        if child_name == &super::ChildName::literal("data") {
                            original_identity.set(Some(child.identity()));
                            fs::rename(child.path(), &parked_child).unwrap();
                            let replacement = root.create_directory(child_name).unwrap();
                            replacement_identity.set(Some(replacement.identity()));
                        }
                    },
                )
            };

            assert!(matches!(
                result,
                Err(super::SandboxError::InvalidEvidence { .. })
            ));
            assert_ne!(
                original_identity.get().unwrap(),
                replacement_identity.get().unwrap()
            );
            assert!(parked_child.is_dir());
            assert!(root_path.join("data").is_dir());
            drop(namespace_marker);
        }
    }

    #[cfg(target_os = "linux")]
    fn retained_cleanup_fixture(
        profile: &str,
    ) -> (
        tempfile::TempDir,
        super::StableSandboxPaths,
        super::NamespaceHandles,
        super::PinnedFile,
        super::ValidatedCleanupSandbox,
    ) {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile(profile),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap();
        create_valid_sandbox_evidence(&paths.cleanup_root, &paths);
        let sandbox =
            super::validate_cleanup_candidate(handles.sandboxes(), &paths.cleanup_name(), &paths)
                .unwrap();
        (parent, paths, handles, namespace_marker, sandbox)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cleanup_plan_refuses_unexpected_and_replaced_entries_before_removal() {
        #[derive(Clone, Copy, Debug)]
        enum Race {
            Unexpected,
            FileReplacement,
            DirectoryReplacement,
        }

        for race in [
            Race::Unexpected,
            Race::FileReplacement,
            Race::DirectoryReplacement,
        ] {
            let (parent, paths, handles, namespace_marker, sandbox) =
                retained_cleanup_fixture(match race {
                    Race::Unexpected => "cleanup-unexpected",
                    Race::FileReplacement => "cleanup-file-replacement",
                    Race::DirectoryReplacement => "cleanup-directory-replacement",
                });
            let config_name = super::ChildName::literal("config");
            let original_identity = sandbox.children().get(&config_name).unwrap().identity();
            let parked_child = paths.cleanup_root.join("parked-config");
            let replacement_identity = std::cell::Cell::new(None);
            let unexpected_identity = std::cell::Cell::new(None);

            let result = super::remove_cleanup_sandbox_with_hooks(
                &paths,
                &handles,
                sandbox,
                &super::SandboxFaults::default(),
                super::CleanupHooks {
                    after_plan: |root: &super::PinnedDirectory| match race {
                        Race::Unexpected => {
                            let unexpected = root
                                .create_directory(&super::ChildName::literal("0-unexpected"))
                                .unwrap();
                            unexpected_identity.set(Some(unexpected.identity()));
                        }
                        Race::FileReplacement => {
                            fs::rename(root.path().join("config"), &parked_child).unwrap();
                            let replacement = root.create_file(&config_name).unwrap();
                            replacement
                                .publish_bytes(root, &config_name, b"replacement file")
                                .unwrap();
                            replacement_identity.set(Some(replacement.identity()));
                        }
                        Race::DirectoryReplacement => {
                            fs::rename(root.path().join("config"), &parked_child).unwrap();
                            let replacement = root.create_directory(&config_name).unwrap();
                            replacement_identity.set(Some(replacement.identity()));
                        }
                    },
                    after_children: super::no_cleanup_marker_hook,
                    after_marker_read: super::no_cleanup_marker_hook,
                    before_root_ticket: super::no_cleanup_root_hook,
                },
            );

            assert!(matches!(
                result,
                Err(super::SandboxError::InvalidEvidence { .. })
            ));
            let root = handles
                .sandboxes()
                .open_directory(&paths.cleanup_name())
                .unwrap();
            assert_eq!(
                fs::read(root.path().join(SANDBOX_MARKER_FILE)).unwrap(),
                paths.sandbox_marker_bytes()
            );
            match race {
                Race::Unexpected => {
                    assert_eq!(
                        root.open_directory(&super::ChildName::literal("0-unexpected"))
                            .unwrap()
                            .identity(),
                        unexpected_identity.get().unwrap()
                    );
                    for name in paths.sandbox_child_names() {
                        assert!(root.open_directory(&name).is_ok());
                    }
                }
                Race::FileReplacement => {
                    assert_eq!(
                        root.open_directory(&super::ChildName::literal("parked-config"))
                            .unwrap()
                            .identity(),
                        original_identity
                    );
                    let replacement = root.open_file(&config_name, false).unwrap();
                    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                    assert_eq!(fs::read(replacement.path()).unwrap(), b"replacement file");
                }
                Race::DirectoryReplacement => {
                    assert_eq!(
                        root.open_directory(&super::ChildName::literal("parked-config"))
                            .unwrap()
                            .identity(),
                        original_identity
                    );
                    let replacement = root.open_directory(&config_name).unwrap();
                    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                }
            }
            assert!(paths.cleanup_root.exists());
            drop(namespace_marker);
            drop(parent);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cleanup_marker_rechecks_preserve_each_marker_last_failure_boundary() {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum Race {
            Corrupt,
            Remove,
            Replace,
        }

        for race in [Race::Corrupt, Race::Remove, Race::Replace] {
            let (parent, paths, handles, namespace_marker, sandbox) =
                retained_cleanup_fixture(match race {
                    Race::Corrupt => "cleanup-corrupt-marker",
                    Race::Remove => "cleanup-removed-marker",
                    Race::Replace => "cleanup-replaced-marker",
                });
            let original_identity = sandbox.marker().identity();
            let parked_marker = paths.cleanup_root.join("parked-marker");
            let replacement_identity = std::cell::Cell::new(None);
            let corrupt_bytes = b"corrupt cleanup marker";

            let result = super::remove_cleanup_sandbox_with_hooks(
                &paths,
                &handles,
                sandbox,
                &super::SandboxFaults::default(),
                super::CleanupHooks {
                    after_plan: super::no_cleanup_plan_hook,
                    after_children: |root: &super::PinnedDirectory, marker: &super::PinnedFile| {
                        if race == Race::Corrupt {
                            use std::io::{Seek as _, Write as _};

                            let writable =
                                root.open_file(&paths.sandbox_marker_name(), true).unwrap();
                            assert_eq!(writable.identity(), marker.identity());
                            let mut file = writable.file().try_clone().unwrap();
                            file.set_len(0).unwrap();
                            file.seek(io::SeekFrom::Start(0)).unwrap();
                            file.write_all(corrupt_bytes).unwrap();
                            file.sync_all().unwrap();
                        }
                    },
                    after_marker_read:
                        |root: &super::PinnedDirectory, marker: &super::PinnedFile| {
                            if race == Race::Remove {
                                fs::remove_file(marker.path()).unwrap();
                            } else {
                                assert_eq!(race, Race::Replace);
                                fs::rename(marker.path(), &parked_marker).unwrap();
                                let replacement =
                                    root.create_file(&paths.sandbox_marker_name()).unwrap();
                                replacement
                                    .publish_bytes(
                                        root,
                                        &paths.sandbox_marker_name(),
                                        paths.sandbox_marker_bytes(),
                                    )
                                    .unwrap();
                                replacement_identity.set(Some(replacement.identity()));
                            }
                        },
                    before_root_ticket: super::no_cleanup_root_hook,
                },
            );

            assert!(matches!(
                result,
                Err(super::SandboxError::InvalidEvidence { .. })
            ));
            let root = handles
                .sandboxes()
                .open_directory(&paths.cleanup_name())
                .unwrap();
            for name in paths.sandbox_child_names() {
                assert!(root.open_directory(&name).is_err());
            }
            match race {
                Race::Corrupt => {
                    let marker = root.open_file(&paths.sandbox_marker_name(), false).unwrap();
                    assert_eq!(marker.identity(), original_identity);
                    assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
                }
                Race::Remove => {
                    assert!(root.open_file(&paths.sandbox_marker_name(), false).is_err());
                    assert!(root.tickets().unwrap().is_empty());
                }
                Race::Replace => {
                    let parked = root
                        .open_file(&super::ChildName::literal("parked-marker"), false)
                        .unwrap();
                    let replacement = root.open_file(&paths.sandbox_marker_name(), false).unwrap();
                    assert_eq!(parked.identity(), original_identity);
                    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                    assert_ne!(parked.identity(), replacement.identity());
                    assert_eq!(
                        fs::read(parked.path()).unwrap(),
                        paths.sandbox_marker_bytes()
                    );
                    assert_eq!(
                        fs::read(replacement.path()).unwrap(),
                        paths.sandbox_marker_bytes()
                    );
                }
            }
            assert!(paths.cleanup_root.is_dir());
            drop(namespace_marker);
            drop(parent);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cleanup_root_recheck_preserves_unmarked_original_and_replacement() {
        let (parent, paths, handles, namespace_marker, sandbox) =
            retained_cleanup_fixture("cleanup-root-replacement");
        let original_identity = sandbox.root().identity();
        let parked_name = super::ChildName::literal("parked-cleanup-root");
        let parked_root = paths.sandboxes_root.join(parked_name.as_os_str());
        let replacement_identity = std::cell::Cell::new(None);

        let result = super::remove_cleanup_sandbox_with_hooks(
            &paths,
            &handles,
            sandbox,
            &super::SandboxFaults::default(),
            super::CleanupHooks {
                after_plan: super::no_cleanup_plan_hook,
                after_children: super::no_cleanup_marker_hook,
                after_marker_read: super::no_cleanup_marker_hook,
                before_root_ticket: |parent: &super::PinnedDirectory,
                                     root: &super::PinnedDirectory,
                                     name: &super::ChildName| {
                    fs::rename(root.path(), &parked_root).unwrap();
                    let replacement = parent.create_directory(name).unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                },
            },
        );

        assert!(matches!(
            result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let parked = handles.sandboxes().open_directory(&parked_name).unwrap();
        let replacement = handles
            .sandboxes()
            .open_directory(&paths.cleanup_name())
            .unwrap();
        assert_eq!(parked.identity(), original_identity);
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        assert!(parked.tickets().unwrap().is_empty());
        assert!(replacement.tickets().unwrap().is_empty());
        assert!(!parked_root.join(SANDBOX_MARKER_FILE).exists());
        assert!(!paths.cleanup_root.join(SANDBOX_MARKER_FILE).exists());
        drop(namespace_marker);
        drop(parent);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn quarantine_recheck_refuses_a_valid_replacement_with_the_wrong_identity() {
        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("quarantine-destination-swap"),
        )
        .unwrap();
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap();
        create_valid_sandbox_evidence(&paths.sandbox_root, &paths);
        let sandbox =
            super::validate_original_candidate(handles.sandboxes(), &paths.sandbox_name(), &paths)
                .unwrap();
        let original_identity = sandbox.root().identity();
        let parked_name = super::ChildName::literal("parked-moved-original");
        let parked_root = paths.sandboxes_root.join(parked_name.as_os_str());
        let replacement_identity = std::cell::Cell::new(None);

        let result = super::quarantine_original_sandbox_with_hook(
            &paths,
            &handles,
            paths.sandbox_name(),
            sandbox,
            &super::SandboxFaults::default(),
            |parent, cleanup_name| {
                fs::rename(&paths.cleanup_root, &parked_root).unwrap();
                let replacement = parent.create_directory(cleanup_name).unwrap();
                let marker = replacement
                    .create_file(&paths.sandbox_marker_name())
                    .unwrap();
                marker
                    .publish_bytes(
                        &replacement,
                        &paths.sandbox_marker_name(),
                        paths.sandbox_marker_bytes(),
                    )
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
        );

        assert!(matches!(
            result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert!(!paths.sandbox_root.exists());
        let parked = handles.sandboxes().open_directory(&parked_name).unwrap();
        let replacement = handles
            .sandboxes()
            .open_directory(&paths.cleanup_name())
            .unwrap();
        assert_eq!(parked.identity(), original_identity);
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        for name in paths.sandbox_child_names() {
            assert!(parked.open_directory(&name).is_ok());
            assert!(replacement.open_directory(&name).is_err());
        }
        assert_eq!(
            fs::read(parked_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        assert_eq!(
            fs::read(paths.cleanup_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            paths.sandbox_marker_bytes()
        );
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recovery_revalidates_retained_cleanup_children_and_marker() {
        #[derive(Clone, Copy, Debug)]
        enum Race {
            Child,
            Marker,
        }

        for race in [Race::Child, Race::Marker] {
            let (parent, paths, handles, namespace_marker, sandbox) =
                retained_cleanup_fixture(match race {
                    Race::Child => "recover-child-swap",
                    Race::Marker => "recover-marker-corrupt",
                });
            let parked_child = paths.cleanup_root.join("parked-data");
            let original_identity = std::cell::Cell::new(None);
            let replacement_identity = std::cell::Cell::new(None);
            let corrupt_bytes = b"corrupt recovery marker";

            let result = super::recover_cleanup_sandbox_with_hook(
                &paths,
                &handles,
                sandbox,
                &super::SandboxFaults::default(),
                |sandbox| match race {
                    Race::Child => {
                        let data_name = super::ChildName::literal("data");
                        let child = sandbox.children().get(&data_name).unwrap();
                        original_identity.set(Some(child.identity()));
                        fs::rename(child.path(), &parked_child).unwrap();
                        let replacement = sandbox.root().create_directory(&data_name).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                    Race::Marker => {
                        use std::io::{Seek as _, Write as _};

                        original_identity.set(Some(sandbox.marker().identity()));
                        let writable = sandbox
                            .root()
                            .open_file(&paths.sandbox_marker_name(), true)
                            .unwrap();
                        assert_eq!(writable.identity(), sandbox.marker().identity());
                        let mut file = writable.file().try_clone().unwrap();
                        file.set_len(0).unwrap();
                        file.seek(io::SeekFrom::Start(0)).unwrap();
                        file.write_all(corrupt_bytes).unwrap();
                        file.sync_all().unwrap();
                    }
                },
            );

            assert!(matches!(
                result,
                Err(super::SandboxError::InvalidEvidence { .. })
            ));
            let root = handles
                .sandboxes()
                .open_directory(&paths.cleanup_name())
                .unwrap();
            match race {
                Race::Child => {
                    let parked = root
                        .open_directory(&super::ChildName::literal("parked-data"))
                        .unwrap();
                    let replacement = root
                        .open_directory(&super::ChildName::literal("data"))
                        .unwrap();
                    assert_eq!(parked.identity(), original_identity.get().unwrap());
                    assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
                    assert_ne!(parked.identity(), replacement.identity());
                    assert_eq!(
                        fs::read(root.path().join(SANDBOX_MARKER_FILE)).unwrap(),
                        paths.sandbox_marker_bytes()
                    );
                }
                Race::Marker => {
                    let marker = root.open_file(&paths.sandbox_marker_name(), false).unwrap();
                    assert_eq!(marker.identity(), original_identity.get().unwrap());
                    assert_eq!(fs::read(marker.path()).unwrap(), corrupt_bytes);
                    for name in paths.sandbox_child_names() {
                        assert!(root.open_directory(&name).is_ok());
                    }
                }
            }
            drop(namespace_marker);
            drop(parent);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_propagates_original_and_cleanup_classification_io() {
        use std::os::unix::fs::PermissionsExt as _;

        for original in [true, false] {
            let parent = tempfile::TempDir::new().unwrap();
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                safe_profile(if original {
                    "prepare-original-io"
                } else {
                    "prepare-cleanup-io"
                }),
            )
            .unwrap();
            let (handles, namespace_marker) =
                super::initialize_namespace_retained(&paths, &super::SandboxFaults::default())
                    .unwrap();
            let evidence_root = if original {
                &paths.sandbox_root
            } else {
                &paths.cleanup_root
            };
            create_valid_sandbox_evidence(evidence_root, &paths);
            fs::set_permissions(evidence_root, fs::Permissions::from_mode(0o000)).unwrap();

            let result = super::prepare_fresh_sandbox_retained(
                &paths,
                &handles,
                &super::SandboxFaults::default(),
            );

            fs::set_permissions(evidence_root, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(matches!(
                result,
                Err(super::SandboxError::Io { source, .. })
                    if source.kind() == io::ErrorKind::PermissionDenied
            ));
            assert_eq!(
                fs::read(evidence_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                paths.sandbox_marker_bytes()
            );
            for name in paths.sandbox_child_names() {
                assert!(evidence_root.join(name.as_os_str()).is_dir());
            }
            drop(namespace_marker);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn prepare_faults_keep_cleanup_original_and_recovery_states_recoverable() {
        for original in [true, false] {
            let parent = tempfile::TempDir::new().unwrap();
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                safe_profile(if original {
                    "cleanup-original-propagation"
                } else {
                    "recover-cleanup-propagation"
                }),
            )
            .unwrap();
            let (handles, namespace_marker) =
                super::initialize_namespace_retained(&paths, &super::SandboxFaults::default())
                    .unwrap();
            let evidence_root = if original {
                &paths.sandbox_root
            } else {
                &paths.cleanup_root
            };
            create_valid_sandbox_evidence(evidence_root, &paths);

            let result = super::prepare_fresh_sandbox_retained(
                &paths,
                &handles,
                &super::SandboxFaults::at(super::SandboxFaultPoint::RemoveChild),
            );

            assert!(matches!(
                result,
                Err(super::SandboxError::InjectedFault {
                    point: super::SandboxFaultPoint::RemoveChild,
                })
            ));
            assert!(!paths.sandbox_root.exists());
            assert_eq!(
                fs::read(paths.cleanup_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                paths.sandbox_marker_bytes()
            );
            for name in paths.sandbox_child_names() {
                assert!(paths.cleanup_root.join(name.as_os_str()).is_dir());
            }

            let fresh = super::prepare_fresh_sandbox_retained(
                &paths,
                &handles,
                &super::SandboxFaults::default(),
            )
            .unwrap();
            assert_eq!(fresh.root().path(), paths.sandbox_root);
            assert!(!paths.cleanup_root.exists());
            assert_eq!(
                fs::read(paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                paths.sandbox_marker_bytes()
            );
            drop(fresh);
            drop(namespace_marker);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_close_propagates_cleanup_io_and_refuses_valid_cleanup() {
        use std::os::unix::fs::PermissionsExt as _;

        for permission_error in [true, false] {
            let parent = tempfile::TempDir::new().unwrap();
            let profile = safe_profile(if permission_error {
                "close-cleanup-io"
            } else {
                "close-valid-cleanup"
            });
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                profile.clone(),
            )
            .unwrap();
            let sandbox = super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
                profile,
                super::SandboxFaults::default(),
            )
            .unwrap();
            if permission_error {
                create_valid_sandbox_evidence(&paths.cleanup_root, &paths);
                fs::set_permissions(&paths.cleanup_root, fs::Permissions::from_mode(0o000))
                    .unwrap();
            } else {
                create_private_test_directory(&paths.cleanup_root);
                write_private_test_file(
                    &paths.cleanup_root.join(SANDBOX_MARKER_FILE),
                    paths.sandbox_marker_bytes(),
                );
            }

            let error = sandbox.close().unwrap_err();

            if permission_error {
                fs::set_permissions(&paths.cleanup_root, fs::Permissions::from_mode(0o700))
                    .unwrap();
                assert!(matches!(
                    error,
                    super::SandboxError::Io { source, .. }
                        if source.kind() == io::ErrorKind::PermissionDenied
                ));
                for name in paths.sandbox_child_names() {
                    assert!(paths.cleanup_root.join(name.as_os_str()).is_dir());
                }
            } else {
                assert!(matches!(error, super::SandboxError::InvalidEvidence { .. }));
                assert_eq!(
                    fs::read(paths.cleanup_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                    paths.sandbox_marker_bytes()
                );
                assert_eq!(fs::read_dir(&paths.cleanup_root).unwrap().count(), 1);
            }
            assert_eq!(
                fs::read(paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                paths.sandbox_marker_bytes()
            );
            for name in paths.sandbox_child_names() {
                assert!(paths.sandbox_root.join(name.as_os_str()).is_dir());
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn live_namespace_validation_refuses_marker_content_and_identity_changes() {
        let replacement_parent = tempfile::TempDir::new().unwrap();
        let replacement_profile = safe_profile("live-namespace-marker-swap");
        let replacement_paths = stable_paths(
            replacement_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            replacement_profile.clone(),
        )
        .unwrap();
        let replacement_sandbox = super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(replacement_paths.namespace_root.clone())
                .unwrap(),
            replacement_profile,
            super::SandboxFaults::default(),
        )
        .unwrap();
        let parked_marker = replacement_paths.namespace_root.join("parked-marker");
        let original_identity = replacement_sandbox
            .namespace_marker
            .as_ref()
            .unwrap()
            .identity();
        let replacement_identity = std::cell::Cell::new(None);

        let replacement_result =
            replacement_sandbox.validate_namespace_with_hook(|namespace, marker, marker_name| {
                fs::rename(marker.path(), &parked_marker).unwrap();
                let replacement = namespace.create_file(marker_name).unwrap();
                replacement
                    .publish_bytes(
                        namespace,
                        marker_name,
                        replacement_paths.namespace_marker_bytes(),
                    )
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            });

        assert!(matches!(
            replacement_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let namespace = replacement_sandbox.handles().namespace();
        let parked = namespace
            .open_file(&super::ChildName::literal("parked-marker"), false)
            .unwrap();
        let replacement = namespace
            .open_file(&replacement_paths.namespace_marker_name(), false)
            .unwrap();
        assert_eq!(parked.identity(), original_identity);
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(
            fs::read(parked.path()).unwrap(),
            replacement_paths.namespace_marker_bytes()
        );
        assert_eq!(
            fs::read(replacement.path()).unwrap(),
            replacement_paths.namespace_marker_bytes()
        );
        drop(replacement_sandbox);

        let corrupt_parent = tempfile::TempDir::new().unwrap();
        let corrupt_profile = safe_profile("live-namespace-marker-corrupt");
        let corrupt_paths = stable_paths(
            corrupt_parent.path().join(STABLE_SANDBOX_NAMESPACE),
            corrupt_profile.clone(),
        )
        .unwrap();
        let corrupt_sandbox = super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(corrupt_paths.namespace_root.clone()).unwrap(),
            corrupt_profile,
            super::SandboxFaults::default(),
        )
        .unwrap();
        let marker = corrupt_sandbox.namespace_marker.as_ref().unwrap();
        let original_identity = marker.identity();
        let writable = corrupt_sandbox
            .handles()
            .namespace()
            .open_file(&corrupt_paths.namespace_marker_name(), true)
            .unwrap();
        assert_eq!(writable.identity(), original_identity);
        let corrupt_bytes = b"corrupt live namespace marker";
        {
            use std::io::{Seek as _, Write as _};

            let mut file = writable.file().try_clone().unwrap();
            file.set_len(0).unwrap();
            file.seek(io::SeekFrom::Start(0)).unwrap();
            file.write_all(corrupt_bytes).unwrap();
            file.sync_all().unwrap();
        }

        assert!(matches!(
            corrupt_sandbox.validate_namespace(),
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        let retained = corrupt_sandbox
            .handles()
            .namespace()
            .open_file(&corrupt_paths.namespace_marker_name(), false)
            .unwrap();
        assert_eq!(retained.identity(), original_identity);
        assert_eq!(fs::read(retained.path()).unwrap(), corrupt_bytes);
        drop(corrupt_sandbox);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn acquisition_prepare_failure_releases_lease_and_retains_partial_evidence() {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile("abort-partial-prepare");
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let paths = stable_paths(namespace_root.clone(), profile.clone()).unwrap();

        let error = super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            profile,
            super::SandboxFaults::at(super::SandboxFaultPoint::CreateSandboxChildIo),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            super::SandboxError::Io {
                operation: "create sandbox child",
                ..
            }
        ));
        assert!(paths.sandbox_root.is_dir());
        assert!(fs::read_dir(&paths.sandbox_root).unwrap().next().is_none());
        assert!(!paths.sandbox_root.join(SANDBOX_MARKER_FILE).exists());
        let (handles, namespace_marker) =
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap();
        let lease = super::acquire_profile_lease_retained(
            &paths,
            &handles,
            &super::SandboxFaults::default(),
        )
        .unwrap();
        lease.release(handles.leases()).unwrap();
        assert!(paths.sandbox_root.is_dir());
        drop(namespace_marker);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn acquisition_prepare_and_release_failures_keep_both_typed_causes() {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile("abort-prepare-release");
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let release_calls = std::cell::Cell::new(0);

        let error = super::StableSandbox::acquire_with_hooks(
            super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            profile.clone(),
            super::SandboxFaults::at(super::SandboxFaultPoint::CreateSandboxRootIo),
            super::no_acquisition_after_prepare_hook,
            |lease, leases| {
                release_calls.set(release_calls.get() + 1);
                lease.release_with_ops(leases, |_| {
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected acquisition unlock failure",
                    ))
                })
            },
        )
        .unwrap_err();

        assert_eq!(release_calls.get(), 1);
        assert!(matches!(
            error,
            super::SandboxError::Dual { primary, release }
                if matches!(*primary, super::SandboxError::Io {
                    operation: "create sandbox",
                    ..
                }) && matches!(*release, super::SandboxError::Io {
                    operation: "unlock profile lease",
                    ..
                })
        ));
        super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            profile,
            super::SandboxFaults::default(),
        )
        .unwrap()
        .close()
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn acquisition_final_namespace_failure_contains_sandbox_and_releases_once() {
        let parent = tempfile::TempDir::new().unwrap();
        let profile = safe_profile("abort-final-namespace");
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let paths = stable_paths(namespace_root.clone(), profile.clone()).unwrap();
        let parked_marker = namespace_root.join("parked-namespace-marker");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let release_calls = std::cell::Cell::new(0);

        let error = super::StableSandbox::acquire_with_hooks(
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            profile,
            super::SandboxFaults::default(),
            |sandbox| {
                let namespace = sandbox.handles().namespace();
                let marker = sandbox.namespace_marker.as_ref().unwrap();
                original_identity.set(Some(marker.identity()));
                fs::rename(marker.path(), &parked_marker).unwrap();
                let replacement = namespace
                    .create_file(&sandbox.paths.namespace_marker_name())
                    .unwrap();
                replacement
                    .publish_bytes(
                        namespace,
                        &sandbox.paths.namespace_marker_name(),
                        sandbox.paths.namespace_marker_bytes(),
                    )
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
            |lease, leases| {
                release_calls.set(release_calls.get() + 1);
                lease.release(leases)
            },
        )
        .unwrap_err();

        assert_eq!(release_calls.get(), 1);
        assert!(matches!(error, super::SandboxError::InvalidEvidence { .. }));
        assert!(!paths.sandbox_root.exists());
        assert!(!paths.cleanup_root.exists());
        let parent_handle = super::PinnedDirectory::open_ambient_parent(parent.path()).unwrap();
        let namespace = parent_handle
            .open_directory(&paths.namespace_name())
            .unwrap();
        let parked = namespace
            .open_file(&super::ChildName::literal("parked-namespace-marker"), false)
            .unwrap();
        let replacement = namespace
            .open_file(&paths.namespace_marker_name(), false)
            .unwrap();
        assert_eq!(parked.identity(), original_identity.get().unwrap());
        assert_eq!(replacement.identity(), replacement_identity.get().unwrap());
        assert_ne!(parked.identity(), replacement.identity());
        assert_eq!(
            fs::read(parked.path()).unwrap(),
            paths.namespace_marker_bytes()
        );
        assert_eq!(
            fs::read(replacement.path()).unwrap(),
            paths.namespace_marker_bytes()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn acquisition_final_lease_and_live_failures_retain_every_replacement() {
        #[derive(Clone, Copy, Debug)]
        enum Race {
            Lease,
            LiveChild,
        }

        for race in [Race::Lease, Race::LiveChild] {
            let parent = tempfile::TempDir::new().unwrap();
            let profile = safe_profile(match race {
                Race::Lease => "abort-final-lease",
                Race::LiveChild => "abort-final-live",
            });
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let paths = stable_paths(namespace_root.clone(), profile.clone()).unwrap();
            let parked = match race {
                Race::Lease => paths.leases_root.join("parked-lease"),
                Race::LiveChild => paths.sandbox_root.join("parked-data"),
            };
            let original_identity = std::cell::Cell::new(None);
            let replacement_identity = std::cell::Cell::new(None);
            let release_calls = std::cell::Cell::new(0);

            let error = super::StableSandbox::acquire_with_hooks(
                super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
                profile,
                super::SandboxFaults::default(),
                |sandbox| match race {
                    Race::Lease => {
                        let leases = sandbox.handles().leases();
                        let lease = sandbox.lease.as_ref().unwrap();
                        original_identity.set(Some(lease.file().identity()));
                        fs::rename(lease.file().path(), &parked).unwrap();
                        let replacement = leases.create_file(&sandbox.paths.lease_name()).unwrap();
                        replacement
                            .publish_bytes(
                                leases,
                                &sandbox.paths.lease_name(),
                                sandbox.paths.lease_marker_bytes(),
                            )
                            .unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                    Race::LiveChild => {
                        let root = sandbox.sandbox.as_ref().unwrap().root();
                        let data_name = super::ChildName::literal("data");
                        let child = root.open_directory(&data_name).unwrap();
                        original_identity.set(Some(child.identity()));
                        drop(child);
                        fs::rename(root.path().join("data"), &parked).unwrap();
                        let replacement = root.create_directory(&data_name).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                },
                |lease, leases| {
                    release_calls.set(release_calls.get() + 1);
                    match race {
                        Race::Lease => lease.release(leases),
                        Race::LiveChild => lease.release_with_ops(leases, |_| {
                            Err(io::Error::new(
                                io::ErrorKind::PermissionDenied,
                                "injected final-validation unlock failure",
                            ))
                        }),
                    }
                },
            )
            .unwrap_err();

            assert_eq!(release_calls.get(), 1);
            assert_ne!(
                original_identity.get().unwrap(),
                replacement_identity.get().unwrap()
            );
            match race {
                Race::Lease => {
                    assert!(matches!(
                        error,
                        super::SandboxError::Dual { primary, release }
                            if matches!(*primary, super::SandboxError::InvalidEvidence { .. })
                                && matches!(*release, super::SandboxError::InvalidEvidence { .. })
                    ));
                    assert!(!paths.sandbox_root.exists());
                    assert!(!paths.cleanup_root.exists());
                    assert_eq!(fs::read(&parked).unwrap(), paths.lease_marker_bytes());
                    assert_eq!(
                        fs::read(&paths.lease_path).unwrap(),
                        paths.lease_marker_bytes()
                    );
                }
                Race::LiveChild => {
                    assert!(matches!(
                        error,
                        super::SandboxError::Dual { primary, release }
                            if matches!(primary.as_ref(), super::SandboxError::Dual {
                                primary: nested_primary,
                                release: nested_cleanup,
                            } if matches!(nested_primary.as_ref(), super::SandboxError::InvalidEvidence { .. })
                                && matches!(nested_cleanup.as_ref(), super::SandboxError::InvalidEvidence { .. }))
                                && matches!(*release, super::SandboxError::Io {
                                    operation: "unlock profile lease",
                                    ..
                                })
                    ));
                    assert!(paths.sandbox_root.is_dir());
                    assert!(parked.is_dir());
                    assert!(paths.sandbox_root.join("data").is_dir());
                    assert_eq!(
                        fs::read(paths.sandbox_root.join(SANDBOX_MARKER_FILE)).unwrap(),
                        paths.sandbox_marker_bytes()
                    );
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn namespace_validation_refuses_replaced_children_and_unexpected_names() {
        let lease_parent = tempfile::TempDir::new().unwrap();
        let lease_paths = initialized_namespace_paths(&lease_parent, &safe_profile("lease-swap"));
        let (parent, namespace, leases, sandboxes) = opened_namespace_parts(&lease_paths);
        let parked_lease = namespace.path().join("parked-leases");
        let replacement_identity = std::cell::Cell::new(None);
        let lease_result = super::validate_namespace_opened_with_hook(
            &lease_paths,
            &parent,
            &namespace,
            &leases,
            &sandboxes,
            |_, namespace, leases, _| {
                fs::rename(leases.path(), &parked_lease).unwrap();
                let replacement = namespace
                    .create_directory(&lease_paths.leases_name())
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
        );
        assert!(matches!(
            lease_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert!(parked_lease.is_dir());
        assert_ne!(replacement_identity.get().unwrap(), leases.identity());

        let sandbox_parent = tempfile::TempDir::new().unwrap();
        let sandbox_paths =
            initialized_namespace_paths(&sandbox_parent, &safe_profile("sandbox-swap"));
        let (parent, namespace, leases, sandboxes) = opened_namespace_parts(&sandbox_paths);
        let parked_sandboxes = namespace.path().join("parked-sandboxes");
        let replacement_identity = std::cell::Cell::new(None);
        let sandbox_result = super::validate_namespace_opened_with_hook(
            &sandbox_paths,
            &parent,
            &namespace,
            &leases,
            &sandboxes,
            |_, namespace, _, sandboxes| {
                fs::rename(sandboxes.path(), &parked_sandboxes).unwrap();
                let replacement = namespace
                    .create_directory(&sandbox_paths.sandboxes_name())
                    .unwrap();
                replacement_identity.set(Some(replacement.identity()));
            },
        );
        assert!(matches!(
            sandbox_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert!(parked_sandboxes.is_dir());
        assert_ne!(replacement_identity.get().unwrap(), sandboxes.identity());

        let extra_parent = tempfile::TempDir::new().unwrap();
        let extra_paths =
            initialized_namespace_paths(&extra_parent, &safe_profile("unexpected-child"));
        let (parent, namespace, leases, sandboxes) = opened_namespace_parts(&extra_paths);
        let extra_result = super::validate_namespace_opened_with_hook(
            &extra_paths,
            &parent,
            &namespace,
            &leases,
            &sandboxes,
            |_, namespace, _, _| {
                drop(
                    namespace
                        .create_directory(&super::ChildName::literal("unexpected"))
                        .unwrap(),
                );
            },
        );
        assert!(matches!(
            extra_result,
            Err(super::SandboxError::InvalidEvidence { .. })
        ));
        assert!(extra_paths.namespace_root.join("unexpected").is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn initialization_release_refuses_each_replaced_namespace_identity() {
        #[derive(Clone, Copy, Debug)]
        enum Target {
            Namespace,
            Leases,
            Sandboxes,
            Marker,
        }

        for target in [
            Target::Namespace,
            Target::Leases,
            Target::Sandboxes,
            Target::Marker,
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let paths = stable_paths(
                parent.path().join(STABLE_SANDBOX_NAMESPACE),
                safe_profile("release-swap"),
            )
            .unwrap();
            let parked = match target {
                Target::Namespace => parent.path().join("parked-namespace"),
                Target::Leases => paths.namespace_root.join("parked-leases"),
                Target::Sandboxes => paths.namespace_root.join("parked-sandboxes"),
                Target::Marker => paths.namespace_root.join("parked-marker"),
            };
            let original_identity = std::cell::Cell::new(None);
            let replacement_identity = std::cell::Cell::new(None);

            let result = super::initialize_namespace_retained_with_hook(
                &paths,
                &super::SandboxFaults::default(),
                |guard, namespace, leases, sandboxes, marker| match target {
                    Target::Namespace => {
                        original_identity.set(Some(namespace.identity()));
                        fs::rename(namespace.path(), &parked).unwrap();
                        let replacement = guard
                            .parent()
                            .create_directory(&paths.namespace_name())
                            .unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                    Target::Leases => {
                        original_identity.set(Some(leases.identity()));
                        fs::rename(leases.path(), &parked).unwrap();
                        let replacement = namespace.create_directory(&paths.leases_name()).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                    Target::Sandboxes => {
                        original_identity.set(Some(sandboxes.identity()));
                        fs::rename(sandboxes.path(), &parked).unwrap();
                        let replacement =
                            namespace.create_directory(&paths.sandboxes_name()).unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                    Target::Marker => {
                        original_identity.set(Some(marker.identity()));
                        fs::rename(marker.path(), &parked).unwrap();
                        let replacement = namespace
                            .create_file(&paths.namespace_marker_name())
                            .unwrap();
                        replacement
                            .publish_bytes(
                                namespace,
                                &paths.namespace_marker_name(),
                                paths.namespace_marker_bytes(),
                            )
                            .unwrap();
                        replacement_identity.set(Some(replacement.identity()));
                    }
                },
            );

            assert!(result.is_err(), "replacement {target:?} was accepted");
            assert!(parked.exists(), "parked {target:?} was removed");
            assert_ne!(
                original_identity.get().unwrap(),
                replacement_identity.get().unwrap()
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn namespace_parent_lock_blocks_a_second_initializer_until_release() {
        use std::sync::mpsc;
        use std::time::Duration;

        let parent = tempfile::TempDir::new().unwrap();
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            safe_profile("init-barrier"),
        )
        .unwrap();
        let parent_path = paths.namespace_root.parent().unwrap().to_path_buf();
        let init_name = paths.init_lock_name();
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let mut worker = None;

        let initialized = super::initialize_namespace_retained_with_hook(
            &paths,
            &super::SandboxFaults::default(),
            |_, _, _, _, _| {
                worker = Some(std::thread::spawn(move || {
                    started_tx.send(()).unwrap();
                    let guard =
                        super::InitializationGuard::acquire(&parent_path, init_name).unwrap();
                    acquired_tx.send(()).unwrap();
                    guard.release(|_| Ok(())).unwrap();
                }));
                started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
                assert!(matches!(
                    acquired_rx.recv_timeout(Duration::from_millis(100)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ));
            },
        )
        .unwrap();

        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        worker.take().unwrap().join().unwrap();
        drop(initialized);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stable_namespace_default_is_the_literal_linux_root() {
        assert_eq!(
            super::default_stable_namespace_root().unwrap(),
            PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE)
        );
    }

    #[test]
    fn stable_namespace_platform_policy_is_pure_and_complete() {
        let windows_temp = Path::new(r"C:\walker-temp");
        assert_eq!(
            super::stable_namespace_root_for(SandboxPlatform::Linux, windows_temp).unwrap(),
            PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE)
        );
        assert_eq!(
            super::stable_namespace_root_for(SandboxPlatform::Windows, windows_temp).unwrap(),
            windows_temp.join(STABLE_SANDBOX_NAMESPACE)
        );
        assert!(matches!(
            super::stable_namespace_root_for(SandboxPlatform::Macos, windows_temp),
            Err(super::SandboxError::UnsupportedPlatform { platform: "macos" })
        ));
        #[cfg(target_os = "linux")]
        assert_eq!(
            super::StableSandboxNamespace::system().unwrap(),
            super::StableSandboxNamespace::explicit(
                PathBuf::from("/tmp").join(STABLE_SANDBOX_NAMESPACE),
            )
            .unwrap()
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn stable_namespace_default_uses_the_windows_temp_root() {
        assert_eq!(
            super::default_stable_namespace_root().unwrap(),
            std::env::temp_dir().join(STABLE_SANDBOX_NAMESPACE)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_reports_random_metadata_but_refuses_stable_acquisition() {
        let review_profile = safe_profile("macos-random");
        let host = RealWalkerHost::spawn(profile()).unwrap();
        assert_eq!(
            host.sandbox_metadata(&review_profile).unwrap().platform(),
            SandboxPlatform::Macos
        );
        host.close().unwrap();
        assert!(matches!(
            super::StableSandboxNamespace::system(),
            Err(super::SandboxError::UnsupportedPlatform { platform: "macos" })
        ));
    }

    #[test]
    fn random_owner_reports_truthful_single_profile_metadata() {
        let review_profile = safe_profile("engine-smoke-60x24");
        let host = RealWalkerHost::spawn(profile()).unwrap();
        let root = host.sandbox_root().to_path_buf();
        assert!(host._sandbox.stable().is_none());

        let metadata = host.sandbox_metadata(&review_profile).unwrap();

        assert_eq!(metadata.mode(), SandboxMode::Random);
        assert_eq!(
            metadata.platform(),
            super::current_sandbox_platform().unwrap()
        );
        assert_eq!(metadata.root(), root.to_str().unwrap());
        assert_eq!(
            metadata.profiles().get(&review_profile).map(String::as_str),
            Some(root.to_str().unwrap())
        );
        host.close().unwrap();
        assert!(!root.exists());

        let creation = super::SandboxOwner::random_with(|| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "random sandbox denied",
            ))
        })
        .unwrap_err();
        assert!(matches!(
            creation,
            super::SandboxError::Io {
                operation: "create random sandbox",
                ..
            }
        ));
    }

    #[test]
    fn explicitly_owned_transient_outside_the_profile_uses_the_transient_root() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let path = outside.path().join(".injected-outside.js");
        let paths = &mut host.path_map;

        paths.register_owned_transient_path(&path, "injected-source", ".injected-");

        assert_eq!(
            paths.normalize_path(&path),
            "<transient>/<injected-source:0>.js"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_owner_publishes_exact_layout_and_closes_only_the_profile() {
        let parent = tempfile::TempDir::new().unwrap();
        fs::write(
            parent.path().join("unrelated-parent-file"),
            b"retain parent",
        )
        .unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile("engine-smoke-60x24");
        let host = RealWalkerHost::spawn_stable_in(
            profile(),
            review_profile.clone(),
            super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        )
        .unwrap();
        let profile_root = namespace_root.join(profile_sandbox_path(&review_profile));
        let cleanup_root = namespace_root.join(profile_cleanup_path(&review_profile));
        let lease_path = namespace_root.join(profile_lease_path(&review_profile));
        let init_lock = parent
            .path()
            .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
        let platform = super::current_sandbox_platform().unwrap();
        let roots = SandboxRoots::new(platform, profile_root.display().to_string()).unwrap();

        assert_eq!(host.sandbox_root(), profile_root);
        assert_eq!(host.adapters.system_temp, profile_root.join("system-temp"));
        let metadata = host.sandbox_metadata(&review_profile).unwrap();
        assert_eq!(metadata.mode(), SandboxMode::Stable);
        assert_eq!(metadata.platform(), platform);
        assert_eq!(metadata.root(), namespace_root.to_str().unwrap());
        assert_eq!(
            metadata.profiles().get(&review_profile).map(String::as_str),
            Some(profile_root.to_str().unwrap())
        );
        assert!(
            host.sandbox_metadata(&safe_profile("another-profile"))
                .is_err()
        );
        assert_eq!(
            fs::read(&init_lock).unwrap(),
            ParentInitLockMarker::new(platform, namespace_root.display().to_string())
                .unwrap()
                .canonical_bytes()
                .unwrap()
        );
        assert_eq!(
            fs::read(namespace_root.join(NAMESPACE_MARKER_FILE)).unwrap(),
            NamespaceMarker::new(platform, namespace_root.display().to_string(),)
                .unwrap()
                .canonical_bytes()
                .unwrap()
        );
        let stable = host._sandbox.stable().unwrap();
        assert_eq!(
            stable
                .lease
                .as_ref()
                .unwrap()
                .file()
                .read_all(stable.handles().leases(), &stable.paths.lease_name())
                .unwrap(),
            ProfileLeaseMarker::new(
                platform,
                review_profile.clone(),
                namespace_root.display().to_string(),
            )
            .unwrap()
            .canonical_bytes()
            .unwrap()
        );
        assert_eq!(
            fs::read(profile_root.join(SANDBOX_MARKER_FILE)).unwrap(),
            SandboxMarker::new(
                platform,
                review_profile.clone(),
                namespace_root.display().to_string(),
            )
            .unwrap()
            .canonical_bytes()
            .unwrap()
        );
        assert_eq!(
            SandboxMarker::new(
                platform,
                review_profile.clone(),
                namespace_root.display().to_string(),
            )
            .unwrap()
            .roots(),
            &roots
        );
        for child in [
            "data",
            "state",
            "config",
            "home",
            "cwd",
            "external",
            "system-temp",
        ] {
            assert!(profile_root.join(child).is_dir());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            for path in [
                namespace_root.clone(),
                namespace_root.join("leases"),
                namespace_root.join("sandboxes"),
                profile_root.clone(),
                profile_root.join("data"),
                profile_root.join("state"),
                profile_root.join("config"),
                profile_root.join("home"),
                profile_root.join("cwd"),
                profile_root.join("external"),
                profile_root.join("system-temp"),
            ] {
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
            for path in [
                init_lock.clone(),
                namespace_root.join(NAMESPACE_MARKER_FILE),
                lease_path.clone(),
                profile_root.join(SANDBOX_MARKER_FILE),
            ] {
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }

        host.close().unwrap();

        assert!(!profile_root.exists());
        assert!(!cleanup_root.exists());
        assert!(init_lock.is_file());
        assert!(lease_path.is_file());
        assert!(namespace_root.join(NAMESPACE_MARKER_FILE).is_file());
        assert_eq!(
            fs::read(parent.path().join("unrelated-parent-file")).unwrap(),
            b"retain parent"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn initialized_sandbox_paths(
        parent: &tempfile::TempDir,
        profile: &SafeProfileId,
    ) -> super::StableSandboxPaths {
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            profile.clone(),
        )
        .unwrap();
        super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
            profile.clone(),
            super::SandboxFaults::default(),
        )
        .unwrap()
        .close()
        .unwrap();
        paths
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn opened_namespace_parts(
        paths: &super::StableSandboxPaths,
    ) -> (
        super::PinnedDirectory,
        super::PinnedDirectory,
        super::PinnedDirectory,
        super::PinnedDirectory,
    ) {
        let parent = super::PinnedDirectory::open_ambient_parent(
            paths
                .namespace_root
                .parent()
                .expect("the namespace has a parent"),
        )
        .unwrap();
        let namespace = parent.open_directory(&paths.namespace_name()).unwrap();
        let leases = namespace.open_directory(&paths.leases_name()).unwrap();
        let sandboxes = namespace.open_directory(&paths.sandboxes_name()).unwrap();
        (parent, namespace, leases, sandboxes)
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn initialized_namespace_paths(
        parent: &tempfile::TempDir,
        profile: &SafeProfileId,
    ) -> super::StableSandboxPaths {
        let paths = stable_paths(
            parent.path().join(STABLE_SANDBOX_NAMESPACE),
            profile.clone(),
        )
        .unwrap();
        drop(
            super::initialize_namespace_retained(&paths, &super::SandboxFaults::default()).unwrap(),
        );
        paths
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn create_private_test_directory(path: &Path) {
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        builder.create(path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn write_private_test_file(path: &Path, bytes: &[u8]) {
        fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn create_valid_sandbox_evidence(path: &Path, paths: &super::StableSandboxPaths) {
        create_private_test_directory(path);
        for child in [
            "data",
            "state",
            "config",
            "home",
            "cwd",
            "external",
            "system-temp",
        ] {
            create_private_test_directory(&path.join(child));
        }
        write_private_test_file(
            &path.join(SANDBOX_MARKER_FILE),
            paths.sandbox_marker_bytes(),
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn create_invalid_sandbox_evidence(path: &Path) {
        create_private_test_directory(path);
        fs::write(path.join("unowned-evidence"), b"retain invalid bytes").unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn evidence_snapshot(path: &Path) -> Option<Vec<OutsideRecord>> {
        fs::symlink_metadata(path)
            .ok()
            .map(|_| outside_snapshot_portable(path))
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_acquisition_executes_all_nine_evidence_states() {
        use SandboxEvidenceState::{Absent, Invalid, Valid};

        for (original_state, cleanup_state, succeeds) in [
            (Valid, Absent, true),
            (Absent, Valid, true),
            (Absent, Absent, true),
            (Valid, Valid, false),
            (Valid, Invalid, false),
            (Invalid, Valid, false),
            (Invalid, Absent, false),
            (Absent, Invalid, false),
            (Invalid, Invalid, false),
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let profile = safe_profile("nine-state");
            let paths = initialized_sandbox_paths(&parent, &profile);
            for (path, state) in [
                (&paths.sandbox_root, original_state),
                (&paths.cleanup_root, cleanup_state),
            ] {
                match state {
                    Absent => {}
                    Valid => create_valid_sandbox_evidence(path, &paths),
                    Invalid => create_invalid_sandbox_evidence(path),
                }
            }
            let original_before = evidence_snapshot(&paths.sandbox_root);
            let cleanup_before = evidence_snapshot(&paths.cleanup_root);

            let result = super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
                profile,
                super::SandboxFaults::default(),
            );

            assert_eq!(
                result.is_ok(),
                succeeds,
                "original {original_state:?}, cleanup {cleanup_state:?}"
            );
            if let Ok(sandbox) = result {
                sandbox.close().unwrap();
            } else {
                assert_eq!(evidence_snapshot(&paths.sandbox_root), original_before);
                assert_eq!(evidence_snapshot(&paths.cleanup_root), cleanup_before);
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn exact_original_and_partial_cleanup_layouts_have_distinct_recovery_rules() {
        let original_parent = tempfile::TempDir::new().unwrap();
        let original_profile = safe_profile("extra-original");
        let original_paths = initialized_sandbox_paths(&original_parent, &original_profile);
        create_valid_sandbox_evidence(&original_paths.sandbox_root, &original_paths);
        fs::write(
            original_paths.sandbox_root.join("unexpected"),
            b"retain unexpected bytes",
        )
        .unwrap();
        let before = evidence_snapshot(&original_paths.sandbox_root);

        assert!(
            super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(original_paths.namespace_root.clone())
                    .unwrap(),
                original_profile,
                super::SandboxFaults::default(),
            )
            .is_err()
        );
        assert_eq!(evidence_snapshot(&original_paths.sandbox_root), before);

        let cleanup_parent = tempfile::TempDir::new().unwrap();
        let cleanup_profile = safe_profile("partial-cleanup");
        let cleanup_paths = initialized_sandbox_paths(&cleanup_parent, &cleanup_profile);
        create_valid_sandbox_evidence(&cleanup_paths.cleanup_root, &cleanup_paths);
        fs::remove_dir(cleanup_paths.cleanup_root.join("data")).unwrap();
        fs::remove_dir(cleanup_paths.cleanup_root.join("state")).unwrap();

        super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(cleanup_paths.namespace_root.clone()).unwrap(),
            cleanup_profile,
            super::SandboxFaults::default(),
        )
        .unwrap()
        .close()
        .unwrap();

        assert!(!cleanup_paths.cleanup_root.exists());

        let invalid_cleanup_parent = tempfile::TempDir::new().unwrap();
        let invalid_cleanup_profile = safe_profile("extra-cleanup");
        let invalid_cleanup_paths =
            initialized_sandbox_paths(&invalid_cleanup_parent, &invalid_cleanup_profile);
        create_valid_sandbox_evidence(&invalid_cleanup_paths.cleanup_root, &invalid_cleanup_paths);
        fs::write(
            invalid_cleanup_paths.cleanup_root.join("unexpected"),
            b"retain cleanup bytes",
        )
        .unwrap();
        let before = evidence_snapshot(&invalid_cleanup_paths.cleanup_root);

        assert!(
            super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(
                    invalid_cleanup_paths.namespace_root.clone()
                )
                .unwrap(),
                invalid_cleanup_profile,
                super::SandboxFaults::default(),
            )
            .is_err()
        );
        assert_eq!(
            evidence_snapshot(&invalid_cleanup_paths.cleanup_root),
            before
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_profile_leases_distinguish_contention_and_release() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("lease-profile");
        let first = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
            .unwrap();

        let busy = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
            .unwrap_err();

        assert!(matches!(
            busy,
            super::StableHostError::Sandbox(super::SandboxError::Busy {
                profile,
                path,
            }) if profile == review_profile
                && path == namespace_root.join(profile_lease_path(&review_profile))
        ));
        first.close().unwrap();
        RealWalkerHost::spawn_stable_in(profile(), review_profile, namespace())
            .unwrap()
            .close()
            .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn init_and_profile_lock_io_failures_are_not_reported_as_contention() {
        for point in [
            super::SandboxFaultPoint::ParentInitLockIo,
            super::SandboxFaultPoint::ProfileLeaseLockIo,
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);

            let error = RealWalkerHost::spawn_stable_in_with_faults(
                profile(),
                safe_profile("lock-io"),
                super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
                super::SandboxFaults::at(point),
            )
            .unwrap_err();

            assert!(matches!(
                error,
                super::StableHostError::Sandbox(super::SandboxError::Io {
                    operation: "lock",
                    ..
                })
            ));
        }

        let parent = tempfile::TempDir::new().unwrap();
        let paths = initialized_sandbox_paths(&parent, &safe_profile("mapped-lock-io"));
        let mapped = super::map_profile_lock_result(
            Err(fs::TryLockError::Error(io::Error::other(
                "injected native lease failure",
            ))),
            &paths,
        )
        .unwrap_err();
        assert!(matches!(
            mapped,
            super::SandboxError::Io {
                operation: "lock",
                ..
            }
        ));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn empty_persistent_lock_evidence_is_retried_then_retained_and_refused() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let init_lock = parent
            .path()
            .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
        write_private_test_file(&init_lock, b"");

        let init_error = super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            safe_profile("empty-init"),
            super::SandboxFaults::default(),
        )
        .unwrap_err();

        assert!(matches!(
            init_error,
            super::SandboxError::InvalidEvidence { path, .. } if path == init_lock
        ));
        assert_eq!(fs::read(&init_lock).unwrap(), b"");
        fs::write(&init_lock, b"wrong parent marker").unwrap();
        assert!(matches!(
            super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
                safe_profile("wrong-init"),
                super::SandboxFaults::default(),
            ),
            Err(super::SandboxError::InvalidEvidence { path, .. }) if path == init_lock
        ));
        assert_eq!(fs::read(&init_lock).unwrap(), b"wrong parent marker");

        let lease_parent = tempfile::TempDir::new().unwrap();
        let review_profile = safe_profile("empty-lease");
        let paths = initialized_sandbox_paths(&lease_parent, &review_profile);
        fs::write(&paths.lease_path, b"").unwrap();

        let lease_error = super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(paths.namespace_root.clone()).unwrap(),
            review_profile,
            super::SandboxFaults::default(),
        )
        .unwrap_err();

        assert!(matches!(
            lease_error,
            super::SandboxError::InvalidEvidence { path, .. } if path == paths.lease_path
        ));
        assert_eq!(fs::read(paths.lease_path).unwrap(), b"");
    }

    // The stable sandbox does not support macOS, so this test skips there. Keep it compiled on
    // macOS. A cfg gate would make the complete stable owner dead code.
    #[cfg(unix)]
    #[cfg_attr(
        target_os = "macos",
        ignore = "the stable sandbox does not support macOS"
    )]
    #[test]
    fn persistent_init_and_profile_lock_inodes_are_reused() {
        use std::os::unix::fs::MetadataExt as _;

        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("persistent-inodes");
        let init_lock = parent
            .path()
            .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
        let lease = namespace_root.join(profile_lease_path(&review_profile));
        RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
            .unwrap()
            .close()
            .unwrap();
        let first_init = fs::metadata(&init_lock).unwrap().ino();
        let first_lease = fs::metadata(&lease).unwrap().ino();

        RealWalkerHost::spawn_stable_in(profile(), review_profile, namespace())
            .unwrap()
            .close()
            .unwrap();

        assert_eq!(fs::metadata(init_lock).unwrap().ino(), first_init);
        assert_eq!(fs::metadata(lease).unwrap().ino(), first_lease);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn kernel_releases_the_profile_lease_after_process_exit() {
        const CHILD: &str = "SKIT_STABLE_SANDBOX_LEASE_EXIT_CHILD";
        const ROOT: &str = "SKIT_STABLE_SANDBOX_LEASE_EXIT_ROOT";
        const TEST: &str = concat!(
            "cli::tui_real_host::tests::",
            "kernel_releases_the_profile_lease_after_process_exit"
        );
        let review_profile = safe_profile("kernel-exit");
        if std::env::var_os(CHILD).is_some() {
            let namespace_root = PathBuf::from(std::env::var_os(ROOT).unwrap());
            let host = RealWalkerHost::spawn_stable_in(
                profile(),
                review_profile,
                super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            )
            .unwrap();
            assert!(host.sandbox_root().is_dir());
            std::process::exit(73);
        }

        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let coverage_profile = restrictive_umask_child_profile();
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .arg("--exact")
            .arg(TEST)
            .arg("--nocapture")
            .env(CHILD, "1")
            .env(ROOT, &namespace_root);
        if let Some(profile) = coverage_profile.as_ref() {
            command.env("LLVM_PROFILE_FILE", profile);
        }

        let status = command.status().unwrap();

        assert_eq!(status.code(), Some(73));
        #[cfg(windows)]
        let lease_identity = windows_file_identity(
            &namespace_root.join(profile_lease_path(&safe_profile("kernel-exit"))),
        );
        RealWalkerHost::spawn_stable_in(
            profile(),
            safe_profile("kernel-exit"),
            super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
        )
        .unwrap()
        .close()
        .unwrap();
        #[cfg(windows)]
        assert_eq!(
            windows_file_identity(
                &namespace_root.join(profile_lease_path(&safe_profile("kernel-exit"))),
            ),
            lease_identity
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn different_profiles_can_initialize_and_run_in_parallel() {
        use std::sync::{Arc, Barrier};

        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let barrier = Arc::new(Barrier::new(2));
        let results = std::thread::scope(|scope| {
            ["parallel-a", "parallel-b"]
                .into_iter()
                .map(|profile_id| {
                    let namespace_root = namespace_root.clone();
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        let review_profile = safe_profile(profile_id);
                        let host = RealWalkerHost::spawn_stable_in(
                            WalkerSeedSpec {
                                profile: "parallel-seed".to_owned(),
                                ..WalkerSeedSpec::default()
                            },
                            review_profile,
                            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
                        )
                        .map_err(|error| error.to_string())?;
                        host.close().map_err(|error| error.to_string())
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });

        assert_eq!(results, vec![Ok(()), Ok(())]);
        for profile_id in ["parallel-a", "parallel-b"] {
            assert!(
                namespace_root
                    .join(profile_lease_path(&safe_profile(profile_id)))
                    .is_file()
            );
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn marker_publication_faults_leave_only_the_specified_recovery_state() {
        use super::SandboxFaultPoint::{
            AfterNamespaceMarker, AfterSandboxMarker, BeforeNamespaceMarker, BeforeSandboxMarker,
        };

        for (point, next_acquire_succeeds) in [
            (BeforeNamespaceMarker, false),
            (AfterNamespaceMarker, true),
            (BeforeSandboxMarker, false),
            (AfterSandboxMarker, true),
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let namespace =
                || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
            let review_profile = safe_profile("marker-fault");

            let error = RealWalkerHost::spawn_stable_in_with_faults(
                profile(),
                review_profile.clone(),
                namespace(),
                super::SandboxFaults::at(point),
            )
            .unwrap_err();

            assert!(matches!(
                error,
                super::StableHostError::Sandbox(super::SandboxError::InjectedFault {
                    point: actual,
                }) if actual == point
            ));
            let next =
                RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace());
            assert_eq!(next.is_ok(), next_acquire_succeeds, "fault point {point:?}");
            if let Ok(host) = next {
                host.close().unwrap();
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn primary_directory_creation_faults_preserve_the_exact_recovery_boundary() {
        use super::SandboxFaultPoint::{
            CreateLeaseDirectoryIo, CreateNamespaceIo, CreateSandboxChildIo,
            CreateSandboxDirectoryIo, CreateSandboxRootIo, InspectNamespaceIo,
        };

        for (point, next_acquire_succeeds) in [
            (CreateNamespaceIo, true),
            (InspectNamespaceIo, true),
            (CreateLeaseDirectoryIo, false),
            (CreateSandboxDirectoryIo, false),
            (CreateSandboxRootIo, true),
            (CreateSandboxChildIo, false),
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let namespace =
                || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
            let review_profile = safe_profile("create-fault");

            let error = RealWalkerHost::spawn_stable_in_with_faults(
                profile(),
                review_profile.clone(),
                namespace(),
                super::SandboxFaults::at(point),
            )
            .unwrap_err();

            assert!(matches!(
                error,
                super::StableHostError::Sandbox(super::SandboxError::Io { .. })
            ));
            let next =
                RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace());
            assert_eq!(
                next.is_ok(),
                next_acquire_succeeds,
                "directory creation fault {point:?}"
            );
            if let Ok(host) = next {
                host.close().unwrap();
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn cleanup_faults_recover_only_while_the_marker_remains() {
        use super::SandboxFaultPoint::{
            AfterChildRemoval, AfterRenameIdentity, RemoveChild, RemoveDirectory, RemoveMarker,
            RenameOriginal,
        };

        for (point, next_acquire_succeeds) in [
            (RenameOriginal, true),
            (AfterRenameIdentity, true),
            (RemoveChild, true),
            (AfterChildRemoval, true),
            (RemoveMarker, true),
            (RemoveDirectory, false),
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let namespace =
                || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
            let review_profile = safe_profile("cleanup-fault");
            let host = RealWalkerHost::spawn_stable_in_with_faults(
                profile(),
                review_profile.clone(),
                namespace(),
                super::SandboxFaults::at(point),
            )
            .unwrap();

            let error = host.close().unwrap_err();

            assert!(matches!(
                error,
                super::SandboxError::InjectedFault { point: actual } if actual == point
            ));
            let sandbox_root = namespace_root.join(profile_sandbox_path(&review_profile));
            let cleanup_root = namespace_root.join(profile_cleanup_path(&review_profile));
            assert!(sandbox_root.exists() || cleanup_root.exists());
            if point == RemoveDirectory {
                assert!(cleanup_root.exists());
                assert!(!cleanup_root.join(SANDBOX_MARKER_FILE).exists());
            }
            let next =
                RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace());
            assert_eq!(
                next.is_ok(),
                next_acquire_succeeds,
                "cleanup fault point {point:?}"
            );
            if let Ok(host) = next {
                host.close().unwrap();
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_host_drop_performs_best_effort_containment() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile("drop-containment");
        let profile_root = namespace_root.join(profile_sandbox_path(&review_profile));
        let host = RealWalkerHost::spawn_stable_in(
            profile(),
            review_profile,
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap();

        drop(host);

        assert!(!profile_root.exists());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn seed_and_cleanup_failures_remain_separate_typed_evidence() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile("primary-cleanup");
        let mut spec = profile();
        spec.directories = vec![directory_seed(WalkerDirectoryRoot::Home, ".codex")];
        let io = G2DirectorySeedIo::new(
            G2DirectorySeedFault::Create,
            parent.path().join("unused-symlink-target"),
        );

        let error = RealWalkerHost::spawn_stable_in_with_faults_and_directory_seed_io(
            spec,
            review_profile.clone(),
            super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            super::SandboxFaults::at(super::SandboxFaultPoint::RemoveChild),
            &io,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            super::StableHostError::Primary {
                primary: super::StableHostPrimaryError::Seed(_),
                cleanup: Some(super::SandboxError::InjectedFault {
                    point: super::SandboxFaultPoint::RemoveChild,
                }),
            }
        ));
        let cleanup_root = namespace_root.join(profile_cleanup_path(&review_profile));
        assert!(cleanup_root.join(SANDBOX_MARKER_FILE).is_file());

        let clean_parent = tempfile::TempDir::new().unwrap();
        let clean_namespace = clean_parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let mut clean_failure_spec = profile();
        clean_failure_spec.directories = vec![directory_seed(WalkerDirectoryRoot::Cwd, ".claude")];
        let clean_io = G2DirectorySeedIo::new(
            G2DirectorySeedFault::Create,
            clean_parent.path().join("unused-symlink-target"),
        );
        let clean_error = RealWalkerHost::spawn_stable_in_with_directory_seed_io(
            clean_failure_spec.clone(),
            safe_profile("primary-only"),
            super::StableSandboxNamespace::explicit(clean_namespace.clone()).unwrap(),
            &clean_io,
        )
        .unwrap_err();
        assert!(matches!(
            clean_error,
            super::StableHostError::Primary {
                primary: super::StableHostPrimaryError::Seed(_),
                cleanup: None,
            }
        ));
        assert!(
            fs::read_dir(clean_namespace.join("sandboxes"))
                .unwrap()
                .next()
                .is_none()
        );
        let random_parent = tempfile::TempDir::new().unwrap();
        let random_io = G2DirectorySeedIo::new(
            G2DirectorySeedFault::Create,
            random_parent.path().join("unused-symlink-target"),
        );
        assert!(
            RealWalkerHost::spawn_with_directory_seed_io(clean_failure_spec, &random_io).is_err()
        );

        let invalid_parent = tempfile::TempDir::new().unwrap();
        let invalid_namespace = invalid_parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let invalid = WalkerSeedSpec {
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("../escape"),
                bytes: b"never written".to_vec(),
                readonly: false,
                unix_mode: 0o600,
            }],
            ..WalkerSeedSpec::default()
        };
        let invalid_error = RealWalkerHost::spawn_stable_in(
            invalid,
            safe_profile("invalid-seed"),
            super::StableSandboxNamespace::explicit(invalid_namespace.clone()).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(
            invalid_error,
            super::StableHostError::Primary {
                primary: super::StableHostPrimaryError::Seed(_),
                cleanup: None,
            }
        ));
        assert!(!invalid_namespace.exists());
    }

    #[cfg(unix)]
    #[test]
    fn random_primary_and_cleanup_failures_are_retained_together() {
        use std::os::unix::fs::PermissionsExt as _;

        let sandbox = tempfile::TempDir::new().unwrap();
        let path = sandbox.path().to_path_buf();
        fs::write(path.join("retained"), b"cleanup evidence").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o500)).unwrap();

        let error = super::SeededTuiHost::failed_random_spawn(
            super::SandboxOwner::Random {
                directory: sandbox,
                root: path.clone(),
            },
            "random primary failure".to_owned(),
        )
        .unwrap_err();

        assert!(error.contains("random primary failure; random sandbox cleanup also failed"));
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    // The stable sandbox does not support macOS, so this test skips there. Keep it compiled on
    // macOS. A cfg gate would make the complete stable owner dead code.
    #[cfg(unix)]
    #[cfg_attr(
        target_os = "macos",
        ignore = "the stable sandbox does not support macOS"
    )]
    #[test]
    fn stable_cleanup_removes_links_and_preserves_the_external_target() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile("link-cleanup");
        let outside = parent.path().join("outside-target");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("retained.txt"), b"outside bytes").unwrap();
        let host = RealWalkerHost::spawn_stable_in(
            profile(),
            review_profile,
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap();
        std::os::unix::fs::symlink(&outside, host.external_root.join("outside-link")).unwrap();

        host.close().unwrap();

        assert_eq!(
            fs::read(outside.join("retained.txt")).unwrap(),
            b"outside bytes"
        );
    }

    #[cfg(windows)]
    #[test]
    fn stable_cleanup_removes_reparse_directories_and_preserves_the_external_target() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile("reparse-cleanup");
        let outside = parent.path().join("outside-target");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("retained.txt"), b"outside bytes").unwrap();
        let mut host = RealWalkerHost::spawn_stable_in(
            profile(),
            review_profile,
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap();
        std::os::windows::fs::symlink_dir(&outside, host.external_root.join("outside-link"))
            .unwrap();

        host.close().unwrap();

        assert_eq!(
            fs::read(outside.join("retained.txt")).unwrap(),
            b"outside bytes"
        );
    }

    // The stable sandbox supports Linux and Windows. This test uses Unix links. Thus it runs on
    // Linux only.
    #[cfg(target_os = "linux")]
    #[test]
    fn ownership_links_are_refused_without_touching_their_targets() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let init_lock = parent
            .path()
            .join(format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"));
        let outside = parent.path().join("outside-lock-target");
        fs::write(&outside, b"outside lock bytes").unwrap();
        std::os::unix::fs::symlink(&outside, &init_lock).unwrap();

        let error = RealWalkerHost::spawn_stable_in(
            profile(),
            safe_profile("linked-owner"),
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            super::StableHostError::Sandbox(super::SandboxError::InvalidEvidence { path, .. })
                if path == init_lock
        ));
        assert_eq!(fs::read(outside).unwrap(), b"outside lock bytes");

        let namespace_parent = tempfile::TempDir::new().unwrap();
        let linked_namespace = namespace_parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let outside_namespace = namespace_parent.path().join("outside-namespace");
        fs::create_dir(&outside_namespace).unwrap();
        fs::write(outside_namespace.join("retained"), b"namespace target").unwrap();
        std::os::unix::fs::symlink(&outside_namespace, &linked_namespace).unwrap();
        assert!(
            RealWalkerHost::spawn_stable_in(
                profile(),
                safe_profile("linked-namespace"),
                super::StableSandboxNamespace::explicit(linked_namespace).unwrap(),
            )
            .is_err()
        );
        assert_eq!(
            fs::read(outside_namespace.join("retained")).unwrap(),
            b"namespace target"
        );

        let lease_parent = tempfile::TempDir::new().unwrap();
        let lease_profile = safe_profile("linked-lease");
        let lease_paths = initialized_sandbox_paths(&lease_parent, &lease_profile);
        let outside_lease = lease_parent.path().join("outside-lease");
        fs::write(&outside_lease, b"lease target").unwrap();
        fs::remove_file(&lease_paths.lease_path).unwrap();
        std::os::unix::fs::symlink(&outside_lease, &lease_paths.lease_path).unwrap();
        assert!(
            super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(lease_paths.namespace_root.clone())
                    .unwrap(),
                lease_profile,
                super::SandboxFaults::default(),
            )
            .is_err()
        );
        assert_eq!(fs::read(outside_lease).unwrap(), b"lease target");

        let sandbox_parent = tempfile::TempDir::new().unwrap();
        let sandbox_profile = safe_profile("linked-sandbox");
        let sandbox_paths = initialized_sandbox_paths(&sandbox_parent, &sandbox_profile);
        let outside_sandbox = sandbox_parent.path().join("outside-sandbox");
        fs::create_dir(&outside_sandbox).unwrap();
        fs::write(outside_sandbox.join("retained"), b"sandbox target").unwrap();
        std::os::unix::fs::symlink(&outside_sandbox, &sandbox_paths.sandbox_root).unwrap();
        assert!(
            super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(sandbox_paths.namespace_root.clone())
                    .unwrap(),
                sandbox_profile,
                super::SandboxFaults::default(),
            )
            .is_err()
        );
        assert_eq!(
            fs::read(outside_sandbox.join("retained")).unwrap(),
            b"sandbox target"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn corrupt_persistent_markers_are_retained_and_refused() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("corrupt-markers");
        RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
            .unwrap()
            .close()
            .unwrap();
        let namespace_marker = namespace_root.join(NAMESPACE_MARKER_FILE);
        fs::write(&namespace_marker, b"{}").unwrap();

        assert!(
            RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace(),)
                .is_err()
        );
        assert_eq!(fs::read(&namespace_marker).unwrap(), b"{}");

        let second_parent = tempfile::TempDir::new().unwrap();
        let second_root = second_parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let second_namespace =
            || super::StableSandboxNamespace::explicit(second_root.clone()).unwrap();
        RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), second_namespace())
            .unwrap()
            .close()
            .unwrap();
        let lease = second_root.join(profile_lease_path(&review_profile));
        fs::write(&lease, b"{}").unwrap();

        assert!(
            RealWalkerHost::spawn_stable_in(profile(), review_profile, second_namespace()).is_err()
        );
        assert_eq!(fs::read(lease).unwrap(), b"{}");
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn live_metadata_and_lease_revalidation_refuse_replaced_evidence() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let namespace = || super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
        let review_profile = safe_profile("live-revalidation");
        let host = RealWalkerHost::spawn_stable_in(profile(), review_profile.clone(), namespace())
            .unwrap();
        let profile_root = namespace_root.join(profile_sandbox_path(&review_profile));
        let lease = namespace_root.join(profile_lease_path(&review_profile));
        let marker = profile_root.join(SANDBOX_MARKER_FILE);
        let stable = host._sandbox.stable().unwrap();
        let lease_file = stable.lease.as_ref().unwrap().file();
        let read_lease = || {
            lease_file
                .read_all(stable.handles().leases(), &stable.paths.lease_name())
                .unwrap()
        };
        let write_lease = |bytes: &[u8]| {
            lease_file
                .publish_bytes(stable.handles().leases(), &stable.paths.lease_name(), bytes)
                .unwrap()
        };
        let expected_lease = read_lease();
        let expected_marker = fs::read(&marker).unwrap();
        write_lease(b"changed while held");

        assert!(matches!(
            host.sandbox_metadata(&review_profile),
            Err(super::SandboxError::InvalidEvidence { path, .. }) if path == lease
        ));
        assert_eq!(read_lease(), b"changed while held");
        assert!(profile_root.is_dir());
        write_lease(&expected_lease);
        fs::write(&marker, b"changed sandbox marker").unwrap();

        assert!(matches!(
            host.sandbox_metadata(&review_profile),
            Err(super::SandboxError::InvalidEvidence { path, .. }) if path == marker
        ));
        assert_eq!(fs::read(&marker).unwrap(), b"changed sandbox marker");
        assert!(profile_root.is_dir());
        fs::write(&marker, expected_marker).unwrap();

        let parked_root = namespace_root.join("sandboxes/.parked-live-revalidation");
        #[cfg(windows)]
        {
            assert!(fs::rename(&profile_root, &parked_root).is_err());
            host.sandbox_metadata(&review_profile).unwrap();
            host.close().unwrap();
        }
        #[cfg(target_os = "linux")]
        {
            fs::rename(&profile_root, &parked_root).unwrap();
            let stable = host
                ._sandbox
                .stable()
                .expect("the host must own a stable sandbox");
            create_valid_sandbox_evidence(&profile_root, &stable.paths);

            assert!(matches!(
                host.sandbox_metadata(&review_profile),
                Err(super::SandboxError::InvalidEvidence { path, .. }) if path == profile_root
            ));
            assert!(profile_root.is_dir());
            assert!(parked_root.is_dir());

            assert!(matches!(
                host.close(),
                Err(super::SandboxError::InvalidEvidence { path, .. }) if path == profile_root
            ));
            assert!(profile_root.is_dir());
            assert!(parked_root.is_dir());
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_seed_keeps_owned_children_or_refuses_missing_ones() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let review_profile = safe_profile("missing-stable-child");
        let sandbox = super::StableSandbox::acquire(
            super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            review_profile,
            super::SandboxFaults::default(),
        )
        .unwrap();
        let missing = sandbox.path().join("data");
        #[cfg(windows)]
        {
            assert!(fs::remove_dir(&missing).is_err());
            sandbox.close().unwrap();
            assert!(!missing.exists());
        }
        #[cfg(target_os = "linux")]
        {
            fs::remove_dir(&missing).unwrap();

            let result = super::SeededTuiHost::spawn_in_sandbox(
                profile(),
                super::SandboxOwner::Stable(Box::new(sandbox)),
                super::AllocationMaximums::default(),
                &super::SystemDirectorySeedIo,
            );

            let (sandbox, _error) =
                result.expect_err("stable seeding must refuse a missing retained child");
            assert!(!missing.exists());
            drop(sandbox);
        }
    }

    fn assert_system_temp_is_empty(host: &RealWalkerHost) {
        assert!(
            fs::read_dir(&host.adapters.system_temp)
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn deterministic_file_allocator_resets_and_isolates_each_purpose_counter() {
        let first_root = tempfile::TempDir::new().unwrap();
        let first = recording_allocator(&first_root.path().join("system-temp"));

        let draft_zero = first
            .temporary_file(
                TemporaryFilePurpose::AuthoredDraft,
                TempLocation::Directory(first_root.path()),
                ".prompt.md",
            )
            .unwrap();
        let injected_zero = first
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::Directory(first_root.path()),
                ".ts",
            )
            .unwrap();
        let quarantine_zero = first
            .private_directory(PrivateDirectoryPurpose::DraftQuarantine, first_root.path())
            .unwrap();
        let draft_one = first
            .temporary_file(
                TemporaryFilePurpose::AuthoredDraft,
                TempLocation::Directory(first_root.path()),
                ".py",
            )
            .unwrap();

        assert_eq!(
            draft_zero.path().file_name().unwrap(),
            "skit-new-000000.prompt.md"
        );
        assert_eq!(
            injected_zero.path().file_name().unwrap(),
            ".injected-000000.ts"
        );
        assert_eq!(
            quarantine_zero.file_name().unwrap(),
            ".skit-quarantine-000000"
        );
        assert_eq!(draft_one.path().file_name().unwrap(), "skit-new-000001.py");

        let second_root = tempfile::TempDir::new().unwrap();
        let second = recording_allocator(&second_root.path().join("system-temp"));
        let reset = second
            .temporary_file(
                TemporaryFilePurpose::AuthoredDraft,
                TempLocation::Directory(second_root.path()),
                ".py",
            )
            .unwrap();
        assert_eq!(reset.path().file_name().unwrap(), "skit-new-000000.py");
    }

    #[test]
    fn concurrent_hosts_isolate_the_walker_system_temp_location() {
        let first = RealWalkerHost::spawn(profile()).unwrap();
        let second = RealWalkerHost::spawn(profile()).unwrap();

        let first_file = first
            .adapters
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::System,
                ".sh",
            )
            .unwrap();
        let second_file = second
            .adapters
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::System,
                ".sh",
            )
            .unwrap();

        assert_eq!(
            first_file.path().file_name().unwrap(),
            ".injected-000000.sh"
        );
        assert_eq!(
            second_file.path().file_name().unwrap(),
            ".injected-000000.sh"
        );
        assert!(
            first_file
                .path()
                .starts_with(first._sandbox.path().join("system-temp"))
        );
        assert!(
            second_file
                .path()
                .starts_with(second._sandbox.path().join("system-temp"))
        );
        assert_ne!(first_file.path().parent(), second_file.path().parent());
    }

    #[test]
    fn injected_wrapper_preserves_both_location_fallback_orders() {
        let adjacent = RealWalkerHost::spawn(profile()).unwrap();
        let blocked_entry = adjacent.roots().state.join("blocked-entry");
        fs::write(&blocked_entry, b"not a directory").unwrap();

        let adjacent_file =
            new_injected_file_with_allocator(&blocked_entry, ".js", true, &adjacent.adapters)
                .unwrap();

        assert!(
            adjacent_file
                .path()
                .starts_with(&adjacent.adapters.system_temp)
        );
        assert!(matches!(
            adjacent.adapters.events.borrow().as_slice(),
            [
                PortEvent::Allocation {
                    location: super::AllocationLocation::Directory(first_path),
                    outcome: super::AllocationOutcome::Rejected(_),
                    ..
                },
                PortEvent::Allocation {
                    location: super::AllocationLocation::System(second_path),
                    outcome: super::AllocationOutcome::Accepted,
                    ..
                },
            ] if first_path == &blocked_entry
                && second_path == &adjacent.adapters.system_temp
        ));
        drop(adjacent_file);
        assert_system_temp_is_empty(&adjacent);

        let private = RealWalkerHost::spawn(profile()).unwrap();
        fs::remove_dir(&private.adapters.system_temp).unwrap();
        fs::write(&private.adapters.system_temp, b"not a directory").unwrap();
        let entry_dir = private.roots().state.join("entry");
        fs::create_dir(&entry_dir).unwrap();

        let private_file =
            new_injected_file_with_allocator(&entry_dir, ".sh", false, &private.adapters).unwrap();

        assert!(private_file.path().starts_with(&entry_dir));
        assert!(matches!(
            private.adapters.events.borrow().as_slice(),
            [
                PortEvent::Allocation {
                    location: super::AllocationLocation::System(first_path),
                    outcome: super::AllocationOutcome::Rejected(_),
                    ..
                },
                PortEvent::Allocation {
                    location: super::AllocationLocation::Directory(second_path),
                    outcome: super::AllocationOutcome::Accepted,
                    ..
                },
            ] if first_path == &private.adapters.system_temp && second_path == &entry_dir
        ));
        drop(private_file);
        fs::remove_file(&private.adapters.system_temp).unwrap();
        fs::create_dir(&private.adapters.system_temp).unwrap();
        assert_system_temp_is_empty(&private);
    }

    #[test]
    fn walker_system_temp_residual_is_normalized_and_refused() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let temporary = host
            .adapters
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::System,
                ".sh",
            )
            .unwrap();
        drop(temporary);
        fs::write(
            host.adapters.system_temp.join(".injected-residual.sh"),
            b"residual bytes",
        )
        .unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert!(error.contains(
            "walker system temp retained an artifact: <profile:fixture>/system-temp/.injected-residual.sh"
        ));
        assert!(!error.contains(&host._sandbox.path().display().to_string()));
        fs::remove_file(host.adapters.system_temp.join(".injected-residual.sh")).unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert_eq!(
            events_with_keys(&observation.transcript, &["allocation"]),
            vec![json!({ "allocation": {
                "purpose": "injected_source",
                "attempt": 0,
                "location": { "kind": "system" },
                "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                "outcome": { "accepted": true },
            }})]
        );
    }

    #[test]
    fn walker_system_temp_inspection_errors_are_stable_and_do_not_drain_events() {
        assert_eq!(
            super::first_system_temp_residual([Err(io::Error::other(
                "injected directory iteration failure",
            ))])
            .unwrap_err(),
            "could not inspect walker system temp: injected directory iteration failure"
        );

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let temporary = host
            .adapters
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::System,
                ".sh",
            )
            .unwrap();
        drop(temporary);
        fs::remove_dir(&host.adapters.system_temp).unwrap();
        let state = host.initial_state().unwrap();

        let error = host.observe(&state).unwrap_err();

        assert!(error.starts_with("could not inspect walker system temp:"));
        fs::create_dir(&host.adapters.system_temp).unwrap();
        let observation = host.observe(&state).unwrap();
        assert_eq!(
            events_with_keys(&observation.transcript, &["allocation"]).len(),
            1
        );
    }

    #[test]
    fn tree_failure_does_not_drain_pending_allocation_or_launch_events() {
        let mut spec = profile();
        spec.entries.push(injected_script(
            "shell",
            "Tree retry shell",
            "TOKEN=before\nprintf '%s' \"$TOKEN\"\n",
            "script.sh",
        ));
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        host.clear_transcript();
        assert!(matches!(
            host.dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Tree retry shell".to_owned()),
                values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
            })
            .unwrap(),
            Action::Complete { .. }
        ));
        let state = host.initial_state().unwrap();
        let parked = host._sandbox.path().join("external-parked");
        fs::rename(&host.external_root, &parked).unwrap();

        assert!(host.observe(&state).is_err());
        fs::rename(&parked, &host.external_root).unwrap();
        let observation = host.observe(&state).unwrap();
        let recorded = events_with_keys(&observation.transcript, &["allocation", "launch"]);

        assert_eq!(recorded.len(), 2);
        assert!(recorded[0].get("allocation").is_some());
        assert!(recorded[1].get("launch").is_some());
    }

    #[test]
    fn deterministic_file_allocator_retries_collisions_without_clobbering() {
        let root = tempfile::TempDir::new().unwrap();
        let occupied = root.path().join("skit-new-000000.py");
        fs::write(&occupied, b"keep me").unwrap();
        let allocator = recording_allocator(&root.path().join("system-temp"));

        let allocated = allocator
            .temporary_file(
                TemporaryFilePurpose::AuthoredDraft,
                TempLocation::Directory(root.path()),
                ".py",
            )
            .unwrap();

        assert_eq!(fs::read(occupied).unwrap(), b"keep me");
        assert_eq!(allocated.path().file_name().unwrap(), "skit-new-000001.py");
        assert!(matches!(
            allocator.events.borrow().as_slice(),
            [
                PortEvent::Allocation { attempt: 0, outcome, .. },
                PortEvent::Allocation {
                    attempt: 1,
                    outcome: super::AllocationOutcome::Accepted,
                    ..
                },
            ] if matches!(
                outcome,
                super::AllocationOutcome::Rejected(error)
                    if error.kind == io::ErrorKind::AlreadyExists
            )
        ));
    }

    #[test]
    fn deterministic_allocator_transcript_orders_collision_success_and_overflow() {
        let maximums = AllocationMaximums {
            authored_draft: 1,
            ..AllocationMaximums::default()
        };
        let mut host = RealWalkerHost::spawn_with_allocation_maximums(profile(), maximums).unwrap();
        let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
        let occupied = drafts.join("skit-new-000000.py");
        fs::write(&occupied, b"occupied").unwrap();

        let allocated = host
            .adapters
            .temporary_file(
                TemporaryFilePurpose::AuthoredDraft,
                TempLocation::Directory(&drafts),
                ".py",
            )
            .unwrap();
        let overflow = host.adapters.temporary_file(
            TemporaryFilePurpose::AuthoredDraft,
            TempLocation::Directory(&drafts),
            ".py",
        );
        let transcript = host.adapters.transcript(&mut host.path_map);

        assert_eq!(fs::read(occupied).unwrap(), b"occupied");
        assert_eq!(allocated.path().file_name().unwrap(), "skit-new-000001.py");
        assert_eq!(
            overflow.unwrap_err().to_string(),
            "authored_draft allocation counter overflow"
        );
        assert_eq!(
            events_with_keys(&transcript, &["allocation"]),
            vec![
                json!({ "allocation": {
                    "purpose": "authored_draft",
                    "attempt": 0,
                    "location": {
                        "kind": "directory",
                        "path": "<profile:fixture>/data/drafts",
                    },
                    "path": "<profile:fixture>/data/drafts/skit-new-000000.py",
                    "outcome": { "rejected": {
                        "kind": "AlreadyExists",
                        "reason": "the allocation candidate already exists",
                    }},
                }}),
                json!({ "allocation": {
                    "purpose": "authored_draft",
                    "attempt": 1,
                    "location": {
                        "kind": "directory",
                        "path": "<profile:fixture>/data/drafts",
                    },
                    "path": "<profile:fixture>/data/.drafts/<draft:0>.py",
                    "outcome": { "accepted": true },
                }}),
                json!({ "allocation": {
                    "purpose": "authored_draft",
                    "attempt": 2,
                    "location": {
                        "kind": "directory",
                        "path": "<profile:fixture>/data/drafts",
                    },
                    "path": null,
                    "outcome": { "rejected": {
                        "kind": "Other",
                        "reason": "authored_draft allocation counter overflow",
                    }},
                }}),
            ]
        );
    }

    #[test]
    fn private_mode_failures_roll_back_and_record_stable_noncollision_errors() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
        host.adapters
            .private_mode_failure
            .replace(Some(PrivateModeFailure {
                target: PrivateModeTarget::File,
                kind: io::ErrorKind::PermissionDenied,
                reason: "walker file chmod denied".to_owned(),
            }));

        let file_error = host
            .adapters
            .temporary_file(
                TemporaryFilePurpose::AuthoredDraft,
                TempLocation::Directory(&drafts),
                ".py",
            )
            .unwrap_err();
        host.adapters
            .private_mode_failure
            .replace(Some(PrivateModeFailure {
                target: PrivateModeTarget::Directory,
                kind: io::ErrorKind::PermissionDenied,
                reason: "walker directory chmod denied".to_owned(),
            }));
        let directory_error = host
            .adapters
            .private_directory(PrivateDirectoryPurpose::DraftQuarantine, &drafts)
            .unwrap_err();
        let transcript = host.adapters.transcript(&mut host.path_map);

        assert_eq!(file_error.to_string(), "walker file chmod denied");
        assert_eq!(directory_error.to_string(), "walker directory chmod denied");
        assert!(!drafts.join("skit-new-000000.py").exists());
        assert!(!drafts.join(".skit-quarantine-000000").exists());
        assert_eq!(
            events_with_keys(&transcript, &["allocation"]),
            vec![
                json!({ "allocation": {
                    "purpose": "authored_draft",
                    "attempt": 0,
                    "location": {
                        "kind": "directory",
                        "path": "<profile:fixture>/data/drafts",
                    },
                    "path": "<profile:fixture>/data/drafts/skit-new-000000.py",
                    "outcome": { "rejected": {
                        "kind": "PermissionDenied",
                        "reason": "walker file chmod denied",
                    }},
                }}),
                json!({ "allocation": {
                    "purpose": "draft_quarantine",
                    "attempt": 0,
                    "location": {
                        "kind": "directory",
                        "path": "<profile:fixture>/data/drafts",
                    },
                    "path": "<profile:fixture>/data/drafts/.skit-quarantine-000000",
                    "outcome": { "rejected": {
                        "kind": "PermissionDenied",
                        "reason": "walker directory chmod denied",
                    }},
                }}),
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn deterministic_file_allocator_uses_private_file_and_directory_modes() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::TempDir::new().unwrap();
        let allocator = recording_allocator(&root.path().join("system-temp"));
        let file = allocator
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::Directory(root.path()),
                ".js",
            )
            .unwrap();
        let directory = allocator
            .private_directory(PrivateDirectoryPurpose::DraftQuarantine, root.path())
            .unwrap();

        assert_eq!(
            fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    /// Every row is umask-independent except a symlink's own mode on macOS, where the OS applies
    /// the umask to the link. The macOS branch pins both link modes and compares every other
    /// field.
    #[cfg(unix)]
    #[test]
    fn tree_observation_is_umask_independent_and_keeps_raw_mode_evidence() {
        use std::os::unix::fs::PermissionsExt as _;

        const CHILD: &str = "SKIT_TREE_OBSERVATION_UMASK_CHILD";
        const RESULT: &str = "SKIT_TREE_OBSERVATION_UMASK_RESULT";
        const TEST: &str = concat!(
            "cli::tui_real_host::tests::",
            "tree_observation_is_umask_independent_and_keeps_raw_mode_evidence"
        );
        if std::env::var_os(CHILD).is_some() {
            let mut spec = profile();
            spec.external.extend([
                WalkerExternalSeed::File {
                    path: PathBuf::from("readonly.txt"),
                    bytes: b"readonly fixture\n".to_vec(),
                    readonly: true,
                    unix_mode: 0o440,
                },
                WalkerExternalSeed::Symlink {
                    path: PathBuf::from("outside-link.sh"),
                    target: PathBuf::from("outside.sh"),
                },
            ]);
            let mut host = RealWalkerHost::spawn(spec).unwrap();
            let config_path = host.roots().config.join("config.toml");
            let binary = host.service.show("Binary bytes").unwrap();
            let binary_path = host.service.repository().payload_path(&binary).unwrap();
            let external_path = host.external_root.join("outside.sh");
            let readonly_path = host.external_root.join("readonly.txt");
            let symlink_path = host.external_root.join("outside-link.sh");
            let raw_mode =
                |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777;
            let before = json!({
                "data": raw_mode(&host.roots().data),
                "config": raw_mode(&config_path),
                "binary": raw_mode(&binary_path),
                "external": raw_mode(&external_path),
                "readonly_mode": raw_mode(&readonly_path),
                "readonly": fs::symlink_metadata(&readonly_path).unwrap().permissions().readonly(),
                "symlink": raw_mode(&symlink_path),
                "symlink_target": fs::read_link(&symlink_path).unwrap(),
            });
            let observation = host.observe(&host.initial_state().unwrap()).unwrap();
            let after = json!({
                "data": raw_mode(&host.roots().data),
                "config": raw_mode(&config_path),
                "binary": raw_mode(&binary_path),
                "external": raw_mode(&external_path),
                "readonly_mode": raw_mode(&readonly_path),
                "readonly": fs::symlink_metadata(&readonly_path).unwrap().permissions().readonly(),
                "symlink": raw_mode(&symlink_path),
                "symlink_target": fs::read_link(&symlink_path).unwrap(),
            });
            let tree_row = |path: &str| {
                observation
                    .tree
                    .iter()
                    .find(|row| row.path == path)
                    .unwrap()
            };
            assert_eq!(
                tree_row("data/scripts/binary-bytes/script.py").mode,
                Some(raw_mode(&binary_path))
            );
            assert_eq!(raw_mode(&binary_path), 0o4751);
            assert_eq!(
                tree_row("external/outside.sh").mode,
                Some(raw_mode(&external_path))
            );
            assert_eq!(raw_mode(&external_path), 0o640);
            let readonly = tree_row("external/readonly.txt");
            assert!(readonly.readonly);
            assert_eq!(readonly.mode, Some(raw_mode(&readonly_path)));
            assert_eq!(raw_mode(&readonly_path), 0o440);
            let symlink = tree_row("external/outside-link.sh");
            assert_eq!(symlink.kind, "symlink");
            assert_eq!(symlink.mode, Some(raw_mode(&symlink_path)));
            assert_eq!(
                symlink.target.as_deref(),
                Some("<profile:fixture>/external/outside.sh")
            );
            fs::write(
                PathBuf::from(std::env::var_os(RESULT).unwrap()),
                serde_json::to_vec(&json!({
                    "observation": observation,
                    "raw_before": before,
                    "raw_after": after,
                }))
                .unwrap(),
            )
            .unwrap();
            return;
        }

        let results = tempfile::TempDir::new().unwrap();
        let run = |mask: &str, output: &Path| {
            let coverage_profile = restrictive_umask_child_profile();
            let mut command = std::process::Command::new("sh");
            command
                .arg("-c")
                .arg("umask \"$1\"; exec \"$2\" --exact \"$3\" --nocapture")
                .arg("sh")
                .arg(mask)
                .arg(std::env::current_exe().unwrap())
                .arg(TEST)
                .env(CHILD, "1")
                .env(RESULT, output);
            if let Some(profile) = coverage_profile.as_ref() {
                command.env("LLVM_PROFILE_FILE", profile);
            }
            assert!(command.status().unwrap().success());
            serde_json::from_slice::<Value>(&fs::read(output).unwrap()).unwrap()
        };
        let public = run("0022", &results.path().join("0022.json"));
        let private = run("0077", &results.path().join("0077.json"));

        assert_ne!(public["raw_before"]["data"], private["raw_before"]["data"]);
        assert_ne!(
            public["raw_before"]["config"],
            private["raw_before"]["config"]
        );
        assert_eq!(public["raw_before"], public["raw_after"]);
        assert_eq!(private["raw_before"], private["raw_after"]);
        // macOS applies the umask to a new symbolic link. Linux always makes the link mode 0o777.
        // The observation keeps the raw symlink mode. Thus on macOS the link mode is the only
        // field that the umask changes. Prove its exact value, then compare all other fields.
        #[cfg(target_os = "macos")]
        {
            const LINK: &str = "external/outside-link.sh";
            let link_mode = |observation: &Value| {
                observation["tree"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|row| row["path"] == LINK)
                    .unwrap()["mode"]
                    .clone()
            };
            let without_link_mode = |observation: &Value| {
                let mut observation = observation.clone();
                for row in observation["tree"].as_array_mut().unwrap() {
                    if row["path"] == LINK {
                        row["mode"] = Value::Null;
                    }
                }
                observation
            };
            assert_eq!(link_mode(&public["observation"]), json!(0o755));
            assert_eq!(link_mode(&private["observation"]), json!(0o700));
            assert_eq!(
                without_link_mode(&public["observation"]),
                without_link_mode(&private["observation"])
            );
        }
        #[cfg(not(target_os = "macos"))]
        assert_eq!(public["observation"], private["observation"]);

        let tree = public["observation"]["tree"].as_array().unwrap();
        for path in ["data", "config/config.toml"] {
            let row = tree.iter().find(|row| row["path"] == path).unwrap();
            assert_eq!(row["mode"], Value::Null);
        }
    }

    #[test]
    fn observation_mode_provenance_is_absolute_duplicate_and_kind_safe() {
        let root = tempfile::TempDir::new().unwrap();
        let file = root.path().join("exact-file");
        let directory = root.path().join("exact-directory");
        fs::write(&file, b"exact\n").unwrap();
        fs::create_dir(&directory).unwrap();
        let mut modes = super::ObservationModeProvenance::default();

        modes
            .register(&file, super::ObservationNodeKind::File)
            .unwrap();
        modes
            .register(&file, super::ObservationNodeKind::File)
            .unwrap();
        assert_eq!(
            modes
                .register(&file, super::ObservationNodeKind::Directory)
                .unwrap_err(),
            format!(
                "observation mode provenance conflicts at {}: file and directory",
                file.display()
            )
        );
        assert_eq!(
            modes
                .register(Path::new("relative"), super::ObservationNodeKind::File)
                .unwrap_err(),
            "an observation mode provenance path is not absolute: relative"
        );
        #[cfg(unix)]
        {
            assert_eq!(
                modes
                    .register(Path::new("/"), super::ObservationNodeKind::File)
                    .unwrap_err(),
                "an observation mode provenance path has no final component: /"
            );
            assert_eq!(
                modes
                    .register_if_parent_present(Path::new("/"), super::ObservationNodeKind::File,)
                    .unwrap_err(),
                "an observation mode provenance path has no final component: /"
            );
        }
        let missing_parent = root.path().join("missing-parent/leaf");
        let error = modes
            .register(&missing_parent, super::ObservationNodeKind::File)
            .unwrap_err();
        assert!(error.starts_with("could not resolve observation mode provenance parent "));
        assert!(error.contains(&missing_parent.parent().unwrap().display().to_string()));

        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        assert_eq!(
            modes.mode(&file, &fs::symlink_metadata(&file).unwrap()),
            Err(format!(
                "observation mode provenance expected file at {}, but found directory",
                file.display()
            ))
        );

        modes
            .register(&directory, super::ObservationNodeKind::Directory)
            .unwrap();
        assert_eq!(
            modes.mode(&directory, &fs::symlink_metadata(&directory).unwrap()),
            Ok(super::portable_mode(
                &fs::symlink_metadata(&directory).unwrap()
            ))
        );

        #[cfg(unix)]
        {
            let blocker = root.path().join("not-a-directory");
            let child = blocker.join("child");
            fs::write(&blocker, b"blocker\n").unwrap();
            let error = modes.register_existing(&child).unwrap_err();
            assert!(error.starts_with("could not inspect observation mode provenance at "));
            assert!(error.contains(&child.display().to_string()));
        }
    }

    #[cfg(unix)]
    #[test]
    fn g2c_host_spawn_refuses_conflicting_external_leaf_kinds_before_writing() {
        let error = RealWalkerHost::spawn(WalkerSeedSpec {
            profile: "mode-conflict".to_owned(),
            external: vec![
                WalkerExternalSeed::Symlink {
                    path: PathBuf::from("alias"),
                    target: PathBuf::from("target"),
                },
                WalkerExternalSeed::File {
                    path: PathBuf::from("alias"),
                    bytes: b"target\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
            ],
            ..WalkerSeedSpec::default()
        })
        .unwrap_err();

        assert_eq!(
            error,
            "walker external fixture leaf paths overlap: alias and alias"
        );
    }

    #[test]
    fn created_copy_fact_failures_are_deferred_and_noncopy_results_are_ignored() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        let commit = |slug: &str| {
            serde_json::from_value::<Action>(json!({
                "add": {"commit_finished": {
                    "request": 0,
                    "result": {"Ok": slug},
                }},
            }))
            .unwrap()
        };
        let mut failed = super::ObservationModeProvenance::default();
        failed.record_created_copy(&host.service, &commit("missing-one"));
        let first = failed.deferred_error.clone().unwrap();
        failed.record_created_copy(&host.service, &commit("missing-two"));

        assert_eq!(failed.deferred_error.as_deref(), Some(first.as_str()));
        assert_eq!(failed.refresh_live_sources(&host.service), Err(first));

        let mut noncopy = super::ObservationModeProvenance::default();
        noncopy
            .try_record_created_copy(&host.service, &commit("reference"))
            .unwrap();
        noncopy
            .try_record_created_copy(&host.service, &commit("command"))
            .unwrap();
        assert!(noncopy.exact.is_empty());
        assert!(noncopy.deferred_error.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn observation_modes_distinguish_equal_raw_bits_and_keep_special_bits() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::TempDir::new().unwrap();
        let exact = root.path().join("exact");
        let ordinary = root.path().join("ordinary");
        let special_file = root.path().join("special-file");
        let special_directory = root.path().join("special-directory");
        for path in [&exact, &ordinary, &special_file] {
            fs::write(path, b"mode\n").unwrap();
        }
        fs::create_dir(&special_directory).unwrap();
        fs::set_permissions(&exact, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&ordinary, fs::Permissions::from_mode(0o600)).unwrap();
        fs::set_permissions(&special_file, fs::Permissions::from_mode(0o4751)).unwrap();
        fs::set_permissions(&special_directory, fs::Permissions::from_mode(0o1770)).unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        modes
            .register(&exact, super::ObservationNodeKind::File)
            .unwrap();

        assert_eq!(
            modes.mode(&exact, &fs::symlink_metadata(&exact).unwrap()),
            Ok(Some(0o600))
        );
        assert_eq!(
            modes.mode(&ordinary, &fs::symlink_metadata(&ordinary).unwrap()),
            Ok(None)
        );
        assert_eq!(
            modes.mode(&special_file, &fs::symlink_metadata(&special_file).unwrap()),
            Ok(Some(0o4751))
        );
        assert_eq!(
            modes.mode(
                &special_directory,
                &fs::symlink_metadata(&special_directory).unwrap()
            ),
            Ok(Some(0o1770))
        );
        assert_eq!(
            super::portable_mode(&fs::symlink_metadata(&ordinary).unwrap()),
            Some(0o600)
        );
    }

    #[cfg(unix)]
    #[test]
    fn observation_mode_provenance_refuses_a_file_replaced_by_a_symlink() {
        let root = tempfile::TempDir::new().unwrap();
        let source = root.path().join("source");
        let replacement = root.path().join("replacement");
        fs::write(&source, b"source\n").unwrap();
        fs::write(&replacement, b"replacement\n").unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        modes
            .register(&source, super::ObservationNodeKind::File)
            .unwrap();
        fs::remove_file(&source).unwrap();
        std::os::unix::fs::symlink(&replacement, &source).unwrap();

        assert_eq!(
            modes.mode(&source, &fs::symlink_metadata(&source).unwrap()),
            Err(format!(
                "observation mode provenance expected file at {}, but found symlink",
                source.display()
            ))
        );
    }

    #[cfg(unix)]
    #[test]
    fn observation_mode_keys_resolve_a_registered_parent_alias_to_the_tree_path() {
        let root = tempfile::TempDir::new().unwrap();
        let physical = root.path().join("physical");
        let alias = root.path().join("alias");
        fs::create_dir(&physical).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let tree_path = physical.join("source.sh");
        let alias_path = alias.join("source.sh");
        fs::write(&tree_path, b"source\n").unwrap();
        super::set_unix_mode(&tree_path, 0o640).unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        modes
            .register(&alias_path, super::ObservationNodeKind::File)
            .unwrap();

        assert_eq!(
            modes.mode(&tree_path, &fs::symlink_metadata(&tree_path).unwrap()),
            Ok(Some(0o640))
        );
    }

    #[cfg(unix)]
    #[test]
    fn observation_mode_keys_resolve_a_tree_path_to_the_lookup_parent_alias() {
        let root = tempfile::TempDir::new().unwrap();
        let physical = root.path().join("physical");
        let alias = root.path().join("alias");
        fs::create_dir(&physical).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let tree_path = physical.join("source.sh");
        let alias_path = alias.join("source.sh");
        fs::write(&tree_path, b"source\n").unwrap();
        super::set_unix_mode(&tree_path, 0o640).unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        modes
            .register(&tree_path, super::ObservationNodeKind::File)
            .unwrap();

        assert_eq!(
            modes.mode(&alias_path, &fs::symlink_metadata(&alias_path).unwrap()),
            Ok(Some(0o640))
        );
    }

    #[cfg(unix)]
    #[test]
    fn observation_mode_keys_never_follow_the_final_symlink_leaf() {
        let root = tempfile::TempDir::new().unwrap();
        let physical = root.path().join("physical");
        let alias = root.path().join("alias");
        fs::create_dir(&physical).unwrap();
        std::os::unix::fs::symlink(&physical, &alias).unwrap();
        let tree_path = physical.join("source.sh");
        let alias_path = alias.join("source.sh");
        let target = physical.join("target.sh");
        fs::write(&tree_path, b"source\n").unwrap();
        fs::write(&target, b"target\n").unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        modes
            .register(&alias_path, super::ObservationNodeKind::File)
            .unwrap();
        fs::remove_file(&tree_path).unwrap();
        std::os::unix::fs::symlink(&target, &tree_path).unwrap();

        assert_eq!(
            modes.mode(&tree_path, &fs::symlink_metadata(&tree_path).unwrap()),
            Err(format!(
                "observation mode provenance expected file at {}, but found symlink",
                tree_path.display()
            ))
        );
        let mut distinct = super::ObservationModeProvenance::default();
        distinct
            .register(&alias_path, super::ObservationNodeKind::Symlink)
            .unwrap();
        distinct
            .register(&target, super::ObservationNodeKind::File)
            .unwrap();
    }

    #[test]
    fn stale_observation_mode_provenance_has_no_tree_row() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let stale = host.roots().state.join("removed-source");
        host.observation_modes
            .get_mut()
            .register(&stale, super::ObservationNodeKind::File)
            .unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert!(
            !observation
                .tree
                .iter()
                .any(|row| row.path == "state/removed-source")
        );
    }

    #[test]
    fn pending_events_register_every_private_mode_kind_and_the_managed_executable() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        let authored = host.roots().state.join("authored");
        let injected = host.roots().state.join("injected");
        let quarantine = host.roots().state.join("quarantine");
        let rejected = host.roots().state.join("rejected");
        for (purpose, path) in [
            (
                super::AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft),
                authored.clone(),
            ),
            (
                super::AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource),
                injected.clone(),
            ),
            (
                super::AllocationPurpose::PrivateDirectory(
                    PrivateDirectoryPurpose::DraftQuarantine,
                ),
                quarantine.clone(),
            ),
        ] {
            modes
                .register_event(
                    &PortEvent::Allocation {
                        purpose,
                        attempt: 0,
                        location: super::AllocationLocation::Directory(host.roots().state.clone()),
                        path: Some(path),
                        outcome: super::AllocationOutcome::Accepted,
                    },
                    host.roots(),
                )
                .unwrap();
        }
        modes
            .register_event(
                &PortEvent::Allocation {
                    purpose: super::AllocationPurpose::TemporaryFile(
                        TemporaryFilePurpose::InjectedSource,
                    ),
                    attempt: 1,
                    location: super::AllocationLocation::Directory(host.roots().state.clone()),
                    path: Some(rejected.clone()),
                    outcome: super::AllocationOutcome::Rejected(super::AllocationFailure {
                        kind: io::ErrorKind::AlreadyExists,
                        reason: "occupied".to_owned(),
                    }),
                },
                host.roots(),
            )
            .unwrap();
        let managed = skit_runtime::managed_uv_path(&host.roots().data);
        fs::create_dir_all(managed.parent().unwrap()).unwrap();
        modes
            .register_event(
                &PortEvent::Probe {
                    operation: "is_file".to_owned(),
                    path: managed.clone(),
                    result: super::ProbeResult::Bool(false),
                },
                host.roots(),
            )
            .unwrap();
        assert_eq!(modes.expected_kind(&managed).unwrap(), None);
        modes
            .register_event(
                &PortEvent::Probe {
                    operation: "find_program".to_owned(),
                    path: PathBuf::from("uv"),
                    result: super::ProbeResult::Path(Some(managed.clone())),
                },
                host.roots(),
            )
            .unwrap();
        assert_eq!(
            modes.expected_kind(&managed).unwrap(),
            Some(super::ObservationNodeKind::File)
        );
        modes
            .register_event(
                &PortEvent::Probe {
                    operation: "is_file".to_owned(),
                    path: managed.clone(),
                    result: super::ProbeResult::Bool(true),
                },
                host.roots(),
            )
            .unwrap();

        for path in [&authored, &injected, &managed] {
            assert_eq!(
                modes.expected_kind(path).unwrap(),
                Some(super::ObservationNodeKind::File)
            );
        }
        assert_eq!(
            modes.expected_kind(&quarantine).unwrap(),
            Some(super::ObservationNodeKind::Directory)
        );
        assert_eq!(modes.expected_kind(&rejected).unwrap(), None);
    }

    #[test]
    fn live_private_allocator_nodes_keep_modes_while_ordinary_siblings_use_none() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let private_file = host
            .adapters
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::Directory(&host.roots().state),
                ".txt",
            )
            .unwrap();
        fs::write(private_file.path(), b"private allocator file\n").unwrap();
        let private_directory = host
            .adapters
            .private_directory(
                PrivateDirectoryPurpose::DraftQuarantine,
                &host.roots().state,
            )
            .unwrap();
        let ordinary_file = host.roots().state.join("ordinary-file");
        let ordinary_directory = host.roots().state.join("ordinary-directory");
        fs::write(&ordinary_file, b"ordinary\n").unwrap();
        fs::create_dir(&ordinary_directory).unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let private_directory_path = host.path_map.normalize_path(&private_directory);
        let private_directory_path = private_directory_path
            .strip_prefix(&format!("{}/", host.path_map.profile_label))
            .unwrap()
            .to_owned();
        let private_file = observation
            .tree
            .iter()
            .find(|row| {
                row.content == Some(super::ByteView::Utf8("private allocator file\n".to_owned()))
            })
            .unwrap();
        let private_directory = observation
            .tree
            .iter()
            .find(|row| row.path == private_directory_path)
            .unwrap();
        let ordinary_file = observation
            .tree
            .iter()
            .find(|row| row.path == "state/ordinary-file")
            .unwrap();
        let ordinary_directory = observation
            .tree
            .iter()
            .find(|row| row.path == "state/ordinary-directory")
            .unwrap();

        #[cfg(unix)]
        {
            assert_eq!(private_file.mode, Some(0o600));
            assert_eq!(private_directory.mode, Some(0o700));
        }
        #[cfg(not(unix))]
        {
            assert_eq!(private_file.mode, None);
            assert_eq!(private_directory.mode, None);
        }
        assert_eq!(ordinary_file.mode, None);
        assert_eq!(ordinary_directory.mode, None);
    }

    #[cfg(not(unix))]
    #[test]
    fn non_unix_observation_modes_are_none_and_readonly_stays_physical() {
        let root = tempfile::TempDir::new().unwrap();
        let path = root.path().join("readonly");
        fs::write(&path, b"readonly\n").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        let mut modes = super::ObservationModeProvenance::default();
        modes
            .register(&path, super::ObservationNodeKind::File)
            .unwrap();

        assert_eq!(modes.mode(&path, &metadata), Ok(None));
        assert!(metadata.permissions().readonly());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stable_directory_creation_refuses_a_restrictive_umask_in_a_child_process() {
        use std::os::unix::fs::PermissionsExt as _;

        const CHILD: &str = "SKIT_STABLE_DIRECTORY_UMASK_CHILD";
        const ROOT: &str = "SKIT_STABLE_DIRECTORY_UMASK_ROOT";
        const TEST: &str = concat!(
            "cli::tui_real_host::tests::",
            "stable_directory_creation_refuses_a_restrictive_umask_in_a_child_process"
        );
        if std::env::var_os(CHILD).is_some() {
            let namespace_root = PathBuf::from(std::env::var_os(ROOT).unwrap());
            let error = super::StableSandbox::acquire(
                super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
                safe_profile("restrictive-umask"),
                super::SandboxFaults::default(),
            )
            .unwrap_err();
            assert!(matches!(
                error,
                super::SandboxError::Io { source, .. }
                    if source.kind() == io::ErrorKind::PermissionDenied
            ));
            assert_eq!(
                fs::metadata(namespace_root).unwrap().permissions().mode() & 0o7777,
                0
            );
            return;
        }

        let parent = tempfile::TempDir::new().unwrap();
        let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
        let coverage_profile = restrictive_umask_child_profile();
        let mut command = std::process::Command::new("sh");
        command
            .arg("-c")
            .arg("umask 0777; exec \"$1\" --exact \"$2\" --nocapture")
            .arg("sh")
            .arg(std::env::current_exe().unwrap())
            .arg(TEST)
            .env(CHILD, "1")
            .env(ROOT, &namespace_root);
        if let Some(profile) = coverage_profile.as_ref() {
            command.env("LLVM_PROFILE_FILE", profile);
        }

        let status = command.status().unwrap();

        assert!(status.success());
        assert_eq!(
            fs::metadata(&namespace_root).unwrap().permissions().mode() & 0o7777,
            0
        );
        fs::set_permissions(&namespace_root, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(fs::read_dir(&namespace_root).unwrap().next().is_none());
        assert!(!namespace_root.join(NAMESPACE_MARKER_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn walker_private_modes_ignore_restrictive_umask_in_a_child_process() {
        use std::os::unix::fs::PermissionsExt as _;

        const CHILD: &str = "SKIT_WALKER_ALLOCATOR_UMASK_CHILD";
        const TEST: &str = concat!(
            "cli::tui_real_host::tests::",
            "walker_private_modes_ignore_restrictive_umask_in_a_child_process"
        );
        if std::env::var_os(CHILD).is_some() {
            let root = tempfile::TempDir::new().unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let allocator = recording_allocator(root.path());
            let file = allocator
                .temporary_file(
                    TemporaryFilePurpose::InjectedSource,
                    TempLocation::Directory(root.path()),
                    ".js",
                )
                .unwrap();
            let directory = allocator
                .private_directory(PrivateDirectoryPurpose::DraftQuarantine, root.path())
                .unwrap();
            let system_file = SYSTEM_FILE_ALLOCATOR
                .temporary_file(
                    TemporaryFilePurpose::InjectedSource,
                    TempLocation::Directory(root.path()),
                    ".js",
                )
                .unwrap();
            let system_directory = SYSTEM_FILE_ALLOCATOR
                .private_directory(PrivateDirectoryPurpose::DraftQuarantine, root.path())
                .unwrap();
            assert_eq!(
                fs::metadata(file.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(system_file.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(system_directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            return;
        }

        let coverage_profile = restrictive_umask_child_profile();
        let mut command = std::process::Command::new("sh");
        command
            .arg("-c")
            .arg("umask 0777; exec \"$1\" --exact \"$2\" --nocapture")
            .arg("sh")
            .arg(std::env::current_exe().unwrap())
            .arg(TEST)
            .env(CHILD, "1");
        if let Some(profile) = coverage_profile.as_ref() {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let status = command.status().unwrap();

        assert!(status.success());
    }

    fn command(name: &str) -> CreateEntry {
        let mut parameter = ParamDecl::new("value");
        parameter.default = Some(ParameterValue::String("default".to_owned()));
        CreateEntry {
            name: name.to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: "walker command".to_owned(),
            payload: None,
            settings: EntrySettings {
                template: "printf {value}".to_owned(),
                params: vec!["value".to_owned()],
                parameters: vec![parameter],
                ..EntrySettings::default()
            },
        }
    }

    fn injected_script(kind: &str, name: &str, source: &str, stored_name: &str) -> CreateEntry {
        let mut declaration = ParamDecl::new("TOKEN");
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.default = Some(ParameterValue::String("before".to_owned()));
        let source =
            write_managed_params(kind, source, std::slice::from_ref(&declaration)).unwrap();
        CreateEntry {
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
                params: vec!["TOKEN".to_owned()],
                parameters: vec![declaration],
                ..EntrySettings::default()
            },
        }
    }

    fn profile() -> WalkerSeedSpec {
        WalkerSeedSpec {
            profile: "fixture".to_owned(),
            entries: vec![
                command("Command"),
                CreateEntry {
                    name: "Binary bytes".to_owned(),
                    kind: EntryKind::parse("python").unwrap(),
                    mode: StorageMode::Copy,
                    source: String::new(),
                    workdir: "store".to_owned(),
                    description: "non-UTF-8 source bytes".to_owned(),
                    payload: Some(EntryPayload {
                        bytes: b"print('\xff')\n\xff".to_vec(),
                        stored_name: Some("script.py".to_owned()),
                        permissions: SourcePermissions {
                            readonly: false,
                            unix_mode: Some(0o4751),
                        },
                    }),
                    settings: EntrySettings::default(),
                },
                CreateEntry {
                    name: "JavaScript".to_owned(),
                    kind: EntryKind::parse("js").unwrap(),
                    mode: StorageMode::Copy,
                    source: String::new(),
                    workdir: "store".to_owned(),
                    description: "dependency preflight".to_owned(),
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
                },
            ],
            settings: BTreeMap::from([
                ("after_run".to_owned(), "stay".to_owned()),
                ("lang".to_owned(), "en".to_owned()),
            ]),
            runners: vec![PromptRunner {
                name: "walker".to_owned(),
                argv: vec!["agent".to_owned(), "{{prompt}}".to_owned()],
            }],
            forms: vec![WalkerFormSeed {
                selector: "Command".to_owned(),
                values: BTreeMap::from([("value".to_owned(), "remembered".to_owned())]),
                extra_args: vec!["tail".to_owned()],
                extra_args_raw: false,
                preset: Some("favorite".to_owned()),
                last_run: Some(WalkerLastRunSeed {
                    exit: 7,
                    at: "2026-08-28T11:00:00+00:00".to_owned(),
                    values: Some(BTreeMap::from([(
                        "value".to_owned(),
                        "remembered".to_owned(),
                    )])),
                }),
            }],
            prompt_runner: "walker".to_owned(),
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("outside.sh"),
                bytes: b"printf outside\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            }],
            external_references: vec![WalkerExternalReferenceSeed {
                request: CreateEntry {
                    name: "Reference".to_owned(),
                    kind: EntryKind::parse("shell").unwrap(),
                    mode: StorageMode::Reference,
                    source: String::new(),
                    workdir: "origin".to_owned(),
                    description: "outside reference".to_owned(),
                    payload: Some(EntryPayload {
                        bytes: b"printf outside\n".to_vec(),
                        stored_name: Some("outside.sh".to_owned()),
                        permissions: SourcePermissions {
                            readonly: false,
                            unix_mode: Some(0o640),
                        },
                    }),
                    settings: EntrySettings::default(),
                },
                source: PathBuf::from("outside.sh"),
            }],
            directories: Vec::new(),
            editor_writes: Vec::new(),
        }
    }

    fn picker_provenance_profile() -> WalkerSeedSpec {
        let mut spec = profile();
        spec.external.extend([
            WalkerExternalSeed::File {
                path: PathBuf::from("nested/picked.py"),
                bytes: b"print('picked')\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("linked.py"),
                target: PathBuf::from("nested/picked.py"),
            },
        ]);
        spec
    }

    #[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct OutsideRecord {
        path: String,
        kind: String,
        content: Option<Vec<u8>>,
        mode: Option<u32>,
        readonly: bool,
        target: Option<String>,
    }

    fn outside_snapshot_portable(root: &std::path::Path) -> Vec<OutsideRecord> {
        fn visit(root: &std::path::Path, path: &std::path::Path, output: &mut Vec<OutsideRecord>) {
            let metadata = fs::symlink_metadata(path).unwrap();
            let relative = path.strip_prefix(root).unwrap().display().to_string();
            let mode = super::portable_mode(&metadata);
            let readonly = metadata.permissions().readonly();
            if metadata.file_type().is_symlink() {
                output.push(OutsideRecord {
                    path: relative,
                    kind: "symlink".to_owned(),
                    content: None,
                    mode,
                    readonly,
                    target: Some(fs::read_link(path).unwrap().display().to_string()),
                });
            } else if metadata.is_file() {
                output.push(OutsideRecord {
                    path: relative,
                    kind: "file".to_owned(),
                    content: Some(fs::read(path).unwrap()),
                    mode,
                    readonly,
                    target: None,
                });
            } else {
                output.push(OutsideRecord {
                    path: relative,
                    kind: "directory".to_owned(),
                    content: None,
                    mode,
                    readonly,
                    target: None,
                });
                let mut children = fs::read_dir(path)
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                children.sort_by_key(fs::DirEntry::file_name);
                for child in children {
                    visit(root, &child.path(), output);
                }
            }
        }

        let mut output = Vec::new();
        visit(root, root, &mut output);
        output.sort();
        output
    }

    fn add_external_source(host: &RealWalkerHost, name: &str) {
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetSourcePath(
            host.external_root.join(name).display().to_string(),
        ));
        let inspected = host
            .dispatch(Effect::Add(workflow.reduce(AddAction::Continue)))
            .unwrap();
        let inspected = serde_json::to_value(inspected).unwrap();
        let inspected: AddAction = serde_json::from_value(inspected["add"].clone()).unwrap();
        let _ = workflow.reduce(inspected);
        let committed = host
            .dispatch(Effect::Add(workflow.reduce(AddAction::Save)))
            .unwrap();
        assert!(matches!(
            committed,
            Action::Add(AddAction::CommitFinished { result: Ok(_), .. })
        ));
    }

    fn add_external_copy_payload(host: &RealWalkerHost, name: &str) -> PathBuf {
        let before = host
            .service
            .repository()
            .scan_entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.slug)
            .collect::<BTreeSet<_>>();
        add_external_source(host, name);
        let entry = host
            .service
            .repository()
            .scan_entries()
            .unwrap()
            .into_iter()
            .find(|entry| !before.contains(&entry.slug))
            .unwrap();
        host.service.repository().payload_path(&entry).unwrap()
    }

    fn events_with_parsed_launch_displays(transcript: &[Value], keys: &[&str]) -> Vec<Value> {
        let mut events = events_with_keys(transcript, keys);
        for event in &mut events {
            if let Some(launch) = event.get_mut("launch") {
                let display = launch["display"].as_str().unwrap();
                launch["display"] = json!(shlex::split(display).unwrap());
            }
        }
        events
    }

    fn events_with_keys(transcript: &[Value], keys: &[&str]) -> Vec<Value> {
        transcript
            .iter()
            .filter(|event| keys.iter().any(|key| event.get(*key).is_some()))
            .cloned()
            .collect()
    }

    #[cfg(unix)]
    fn assert_identity_sentinel(value: &Value, identity: u64, change: u64) {
        assert_eq!(
            value,
            &json!({
                "platform": "unix",
                "device": 0,
                "inode": identity,
                "change_time_seconds": 0,
                "change_time_nanoseconds": change,
            })
        );
    }

    #[cfg(windows)]
    fn assert_identity_sentinel(value: &Value, identity: u64, change: u64) {
        assert_eq!(
            value,
            &json!({
                "platform": "windows",
                "volume_serial_number": 0,
                "file_index": identity.to_string(),
                "creation_time": change,
            })
        );
    }

    #[cfg(not(any(unix, windows)))]
    fn assert_identity_sentinel(value: &Value, _identity: u64, _change: u64) {
        assert_eq!(value, &Value::Null);
    }

    #[test]
    fn seeded_profiles_reopen_every_store_and_normalize_volatile_facts() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        let first_external_before = outside_snapshot_portable(&first.external_root);
        let second_external_before = outside_snapshot_portable(&second.external_root);

        assert_ne!(first.roots().data, second.roots().data);
        let first_state = first.initial_state().unwrap();
        let second_state = second.initial_state().unwrap();
        let first_observation = first.observe(&first_state).unwrap();
        let second_observation = second.observe(&second_state).unwrap();
        assert_eq!(first_observation, second_observation);
        let encoded = serde_json::to_string_pretty(&first_observation).unwrap();
        assert!(!encoded.contains(&first.roots().data.display().to_string()));
        assert!(!encoded.contains(&second.roots().data.display().to_string()));
        assert!(encoded.contains("<entry-id:command>"));
        assert!(encoded.contains("<added-at:command>"));
        assert!(encoded.contains(&encode_hex(b"print('\xff')\n\xff")));
        let binary = first_observation
            .tree
            .iter()
            .find(|row| row.path == "data/scripts/binary-bytes/script.py")
            .unwrap();
        assert!(!binary.readonly);
        assert!(matches!(&binary.content, Some(super::ByteView::Hex(_))));
        #[cfg(unix)]
        assert_eq!(binary.mode, Some(0o4751));
        #[cfg(not(unix))]
        assert_eq!(binary.mode, None);
        assert_eq!(
            outside_snapshot_portable(&first.external_root),
            first_external_before
        );
        assert_eq!(
            outside_snapshot_portable(&second.external_root),
            second_external_before
        );
    }

    #[test]
    fn newly_added_and_replaced_copy_payload_modes_stay_raw() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let original = host
            .service
            .repository()
            .scan_entries()
            .unwrap()
            .into_iter()
            .map(|entry| entry.slug)
            .collect::<BTreeSet<_>>();
        add_external_source(&host, "outside.sh");
        let entry = host
            .service
            .repository()
            .scan_entries()
            .unwrap()
            .into_iter()
            .find(|entry| !original.contains(&entry.slug))
            .unwrap();
        let payload = host.service.repository().payload_path(&entry).unwrap();
        let tree_path = host.path_map.normalize_path(&payload);
        let tree_path = tree_path
            .strip_prefix(&format!("{}/", host.path_map.profile_label))
            .unwrap()
            .to_owned();

        let first = host.observe(&host.initial_state().unwrap()).unwrap();
        let first = first.tree.iter().find(|row| row.path == tree_path).unwrap();
        #[cfg(unix)]
        assert_eq!(first.mode, Some(0o640));
        #[cfg(not(unix))]
        assert_eq!(first.mode, None);

        fs::remove_file(&payload).unwrap();
        fs::write(&payload, b"replacement source\n").unwrap();
        super::set_unix_mode(&payload, 0o750).unwrap();
        let second = host.observe(&host.initial_state().unwrap()).unwrap();
        let second = second
            .tree
            .iter()
            .find(|row| row.path == tree_path)
            .unwrap();
        #[cfg(unix)]
        assert_eq!(second.mode, Some(0o750));
        #[cfg(not(unix))]
        assert_eq!(second.mode, None);
    }

    #[test]
    fn live_payload_mode_fact_refuses_a_later_directory() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let entry = host.service.show("Binary bytes").unwrap();
        let payload = host.service.repository().payload_path(&entry).unwrap();
        let state = host.initial_state().unwrap();
        host.observe(&state).unwrap();
        fs::remove_file(&payload).unwrap();
        fs::create_dir(&payload).unwrap();

        let error = host.observe(&state).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found directory",
                payload.display()
            )
        );
    }

    #[test]
    fn seed_copy_mode_fact_refuses_a_directory_before_first_observe() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let entry = host.service.show("Binary bytes").unwrap();
        let payload = host.service.repository().payload_path(&entry).unwrap();
        fs::remove_file(&payload).unwrap();
        fs::create_dir(&payload).unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found directory",
                payload.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn seed_copy_mode_fact_refuses_a_symlink_before_first_observe() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let entry = host.service.show("Binary bytes").unwrap();
        let payload = host.service.repository().payload_path(&entry).unwrap();
        fs::remove_file(&payload).unwrap();
        std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found symlink",
                payload.display()
            )
        );
    }

    fn external_reference_lane_copy_host() -> (RealWalkerHost, PathBuf) {
        let mut spec = profile();
        spec.external_references.push(WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "External lane copy".to_owned(),
                kind: EntryKind::parse("shell").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"printf external lane copy\n".to_vec(),
                    stored_name: Some("custom.bin".to_owned()),
                    permissions: SourcePermissions {
                        readonly: false,
                        unix_mode: Some(0o640),
                    },
                }),
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("outside.sh"),
        });
        let host = RealWalkerHost::spawn(spec).unwrap();
        let entry = host.service.show("External lane copy").unwrap();
        let payload = host.service.repository().payload_path(&entry).unwrap();
        assert_eq!(
            payload.file_name().and_then(|name| name.to_str()),
            Some("custom.bin")
        );
        (host, payload)
    }

    #[test]
    fn external_reference_lane_copy_fact_refuses_a_directory_before_first_observe() {
        let (mut host, payload) = external_reference_lane_copy_host();
        fs::remove_file(&payload).unwrap();
        fs::create_dir(&payload).unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found directory",
                payload.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_reference_lane_copy_fact_refuses_a_symlink_before_first_observe() {
        let (mut host, payload) = external_reference_lane_copy_host();
        fs::remove_file(&payload).unwrap();
        std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found symlink",
                payload.display()
            )
        );
    }

    #[test]
    fn new_copy_mode_fact_refuses_a_directory_before_first_observe() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let payload = add_external_copy_payload(&host, "outside.sh");
        fs::remove_file(&payload).unwrap();
        fs::create_dir(&payload).unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found directory",
                payload.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn new_copy_mode_fact_refuses_a_symlink_before_first_observe() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let payload = add_external_copy_payload(&host, "outside.sh");
        fs::remove_file(&payload).unwrap();
        std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

        let error = host.observe(&host.initial_state().unwrap()).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found symlink",
                payload.display()
            )
        );
    }

    #[cfg(unix)]
    #[test]
    fn live_payload_mode_fact_refuses_a_later_symlink() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let entry = host.service.show("Binary bytes").unwrap();
        let payload = host.service.repository().payload_path(&entry).unwrap();
        let state = host.initial_state().unwrap();
        host.observe(&state).unwrap();
        fs::remove_file(&payload).unwrap();
        std::os::unix::fs::symlink(host.external_root.join("outside.sh"), &payload).unwrap();

        let error = host.observe(&state).unwrap_err();

        assert_eq!(
            error,
            format!(
                "observation mode provenance expected file at {}, but found symlink",
                payload.display()
            )
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stable_sandbox_child_roots_keep_their_validated_private_modes() {
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            super::StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let mut host =
            RealWalkerHost::spawn_stable_in(profile(), safe_profile("mode-provenance"), namespace)
                .unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        for path in ["data", "state", "config", "home", "cwd", "external"] {
            let row = observation
                .tree
                .iter()
                .find(|row| row.path == path)
                .unwrap();
            #[cfg(unix)]
            assert_eq!(row.mode, Some(0o700));
            #[cfg(not(unix))]
            assert_eq!(row.mode, None);
        }
        host.close().unwrap();
    }

    #[test]
    fn repeated_profile_spawns_keep_exact_surface_order_and_bytes() {
        let mut expected = None;
        for _ in 0..12 {
            let mut spec = profile();
            spec.external.extend([
                WalkerExternalSeed::File {
                    path: PathBuf::from("alpha.py"),
                    bytes: b"print('alpha')\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
                WalkerExternalSeed::File {
                    path: PathBuf::from("beta.py"),
                    bytes: b"print('beta')\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
            ]);
            let mut host = RealWalkerHost::spawn(spec).unwrap();
            add_external_source(&host, "alpha.py");
            add_external_source(&host, "beta.py");
            let entries = host.service.repository().scan_entries().unwrap();
            let activity = entries
                .iter()
                .filter(|entry| matches!(entry.slug.as_str(), "alpha" | "beta"))
                .map(|entry| (entry.slug.as_str(), entry.meta.added_at.as_str()))
                .collect::<BTreeMap<_, _>>();
            assert_eq!(activity["alpha"], "2026-08-28T12:35:00+00:00");
            assert_eq!(activity["beta"], "2026-08-28T12:35:01+00:00");
            let observation = host.observe(&host.initial_state().unwrap()).unwrap();
            let bytes = serde_json::to_vec(&observation).unwrap();
            if let Some(expected) = &expected {
                assert_eq!(&bytes, expected);
            } else {
                expected = Some(bytes);
            }
        }
    }

    #[test]
    fn persistent_tokens_expose_same_slug_and_same_path_replacements() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        // Both drafts need distinct modified times. The sorted scan refuses equal times.
        let draft = write_draft_at(&host, "skit-new-same.py", b"same bytes\n", 10);
        write_draft_at(&host, "skit-new-z.py", b"zz bytes\n", 20);
        let first = host.observe(&host.initial_state().unwrap()).unwrap();
        let first_meta = first
            .tree
            .iter()
            .find(|row| row.path == "data/scripts/command/meta.toml")
            .and_then(|row| row.content.as_ref())
            .unwrap();
        let first_meta = serde_json::to_value(first_meta).unwrap();
        assert_eq!(first_meta["encoding"], "utf8");
        let first_meta = first_meta["data"].as_str().unwrap();
        let first_meta: toml::Value = toml::from_str(first_meta).unwrap();
        assert_eq!(first_meta["id"].as_str(), Some("<entry-id:command>"));
        assert_eq!(first_meta["added_at"].as_str(), Some("<added-at:command>"));
        assert_identity_sentinel(&first.drafts[0]["identity"], 0, 0);
        {
            let paths = &mut host.path_map;
            let next_draft = paths.next_draft;
            paths.register_draft_path(&draft);
            assert_eq!(paths.next_draft, next_draft);
        }

        let old_draft = OpenOptions::new().read(true).open(&draft).unwrap();
        fs::remove_file(&draft).unwrap();
        fs::write(&draft, b"same bytes\n").unwrap();
        let command_entry = host.service.show("Command").unwrap();
        host.service.remove(&command_entry).unwrap();
        host.service.add(command("Command")).unwrap();

        let second = host.observe(&host.initial_state().unwrap()).unwrap();
        drop(old_draft);
        let second_meta = second
            .tree
            .iter()
            .find(|row| row.path == "data/scripts/command/meta.toml")
            .and_then(|row| row.content.as_ref())
            .unwrap();
        let second_meta = serde_json::to_value(second_meta).unwrap();
        assert_eq!(second_meta["encoding"], "utf8");
        let second_meta = second_meta["data"].as_str().unwrap();
        let second_meta: toml::Value = toml::from_str(second_meta).unwrap();

        assert_eq!(second_meta["id"].as_str(), Some("<entry-id:command:2>"));
        assert_eq!(
            second_meta["added_at"].as_str(),
            Some("<added-at:command:2>")
        );
        assert_identity_sentinel(&second.drafts[0]["identity"], 2, 0);
    }

    #[test]
    fn absent_config_document_never_creates_a_dangling_tree_reference() {
        let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
            profile: "empty".to_owned(),
            ..WalkerSeedSpec::default()
        })
        .unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert_eq!(observation.config["document_ref"], Value::Null);
        assert!(
            !observation
                .tree
                .iter()
                .any(|row| row.path == "config/config.toml")
        );
    }

    #[test]
    fn an_empty_profile_name_uses_the_stable_default_artifact_label() {
        let mut host = RealWalkerHost::spawn(WalkerSeedSpec::default()).unwrap();

        assert_eq!(host.profile, "default");
        assert_eq!(host.path_map.profile_label, "<profile:default>");
        assert!(host.observe(&host.initial_state().unwrap()).is_ok());
    }

    #[test]
    fn preferences_drive_locale_file_discovery_install_and_raw_checkpoint_ports() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.adapters.variables.remove("LANG");
        host.adapters.system_locale.set(Locale::ZhCn);
        let home = host.roots().home.as_ref().unwrap();
        fs::write(home.join("bash"), b"#!/bin/sh\n").unwrap();
        fs::create_dir_all(home.join(".codex/skills")).unwrap();
        fs::create_dir_all(host.roots().cwd.join(".agents/skills")).unwrap();
        host.clear_transcript();

        assert!(matches!(
            host.dispatch(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([
                        ("shell.bash_path".to_owned(), "~/bash".to_owned(),)
                    ]),
                },
            )))
            .unwrap(),
            Action::PreferencesSaved { .. }
        ));
        let saved = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(
                &saved.transcript,
                &[
                    "environment_variable",
                    "system_locale",
                    "platform",
                    "preference"
                ]
            ),
            vec![
                json!({ "environment_variable": "SKIT_LANG", "result": null }),
                json!({ "environment_variable": "LC_ALL", "result": null }),
                json!({ "environment_variable": "LC_MESSAGES", "result": null }),
                json!({ "environment_variable": "LANG", "result": null }),
                json!({ "system_locale": "ZhCn" }),
                json!({ "platform": "Other" }),
                json!({
                    "preference": "is_file",
                    "path": "<profile:fixture>/home/bash",
                    "outcome": true,
                }),
            ]
        );
        let config_before_refusal = fs::read(host.roots().config.join("config.toml")).unwrap();
        assert!(matches!(
            host.dispatch(Effect::Preferences(PreferencesEffect::Save(
                PreferencesChangeSet {
                    settings: BTreeMap::from([(
                        "shell.bash_path".to_owned(),
                        "~/.codex".to_owned(),
                    )]),
                },
            )))
            .unwrap(),
            Action::Preferences(_)
        ));
        assert_eq!(
            fs::read(host.roots().config.join("config.toml")).unwrap(),
            config_before_refusal
        );
        let refused = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(
                &refused.transcript,
                &[
                    "environment_variable",
                    "system_locale",
                    "platform",
                    "preference"
                ]
            ),
            vec![
                json!({ "environment_variable": "SKIT_LANG", "result": null }),
                json!({ "environment_variable": "LC_ALL", "result": null }),
                json!({ "environment_variable": "LC_MESSAGES", "result": null }),
                json!({ "environment_variable": "LANG", "result": null }),
                json!({ "system_locale": "ZhCn" }),
                json!({ "platform": "Other" }),
                json!({
                    "preference": "is_file",
                    "path": "<profile:fixture>/home/.codex",
                    "outcome": false,
                }),
            ]
        );
        assert!(matches!(
            host.dispatch(Effect::Preferences(
                PreferencesEffect::DiscoverAgentSkillTargets,
            ))
            .unwrap(),
            Action::Preferences(skit_ui::PreferencesAction::PresentAgentSkillTargets(_))
        ));
        let discovered = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(
                &discovered.transcript,
                &[
                    "environment_variable",
                    "system_locale",
                    "platform",
                    "preference"
                ]
            ),
            vec![
                json!({ "environment_variable": "SKIT_LANG", "result": null }),
                json!({ "environment_variable": "LC_ALL", "result": null }),
                json!({ "environment_variable": "LC_MESSAGES", "result": null }),
                json!({ "environment_variable": "LANG", "result": null }),
                json!({ "system_locale": "ZhCn" }),
                json!({ "platform": "Other" }),
                json!({ "preference": "is_dir", "path": "<profile:fixture>/home/.claude", "outcome": false }),
                json!({ "preference": "is_dir", "path": "<profile:fixture>/home/.codex", "outcome": true }),
                json!({ "preference": "is_dir", "path": "<profile:fixture>/cwd/.claude", "outcome": false }),
                json!({ "preference": "is_dir", "path": "<profile:fixture>/cwd/.codex", "outcome": false }),
                json!({ "preference": "is_dir", "path": "<profile:fixture>/cwd/.agents", "outcome": true }),
            ]
        );
        let install = host.roots().cwd.join(".agents/skills");
        assert!(matches!(
            host.dispatch(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: install.clone(),
            },))
                .unwrap(),
            Action::Preferences(skit_ui::PreferencesAction::AgentSkillInstalled { .. })
        ));
        assert_eq!(
            fs::read(install.join("skit/SKILL.md")).unwrap(),
            include_bytes!("../../../../skills/skit/SKILL.md")
        );
        let installed = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(
                &installed.transcript,
                &[
                    "environment_variable",
                    "system_locale",
                    "platform",
                    "preference"
                ]
            ),
            vec![
                json!({ "environment_variable": "SKIT_LANG", "result": null }),
                json!({ "environment_variable": "LC_ALL", "result": null }),
                json!({ "environment_variable": "LC_MESSAGES", "result": null }),
                json!({ "environment_variable": "LANG", "result": null }),
                json!({ "system_locale": "ZhCn" }),
                json!({ "platform": "Other" }),
                json!({ "preference": "agent_skill::Inspect", "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md", "outcome": { "ok": true } }),
                json!({ "preference": "agent_skill::BeforeCreateDirectory", "path": "<profile:fixture>/cwd/.agents/skills/skit", "outcome": { "ok": true } }),
                json!({ "preference": "agent_skill::BeforeReplace", "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md", "outcome": { "ok": true } }),
            ]
        );
        let before = fs::read(install.join("skit/SKILL.md")).unwrap();
        host.adapters
            .preference_failure
            .replace(Some(super::PreferenceFailure {
                point: AgentSkillInstallPoint::BeforeReplace,
                kind: io::ErrorKind::PermissionDenied,
                reason: "walker replace denied".to_owned(),
            }));
        let failed = host
            .dispatch(Effect::Preferences(PreferencesEffect::InstallAgentSkill {
                skills_dir: install.clone(),
            }))
            .unwrap();
        assert!(
            matches!(failed, Action::SetStatus(ref status) if status.contains("walker replace denied"))
        );
        assert_eq!(fs::read(install.join("skit/SKILL.md")).unwrap(), before);

        let failed = host.observe(&host.initial_state().unwrap()).unwrap();
        let failure = json!({
            "preference": "agent_skill::BeforeReplace",
            "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md",
            "outcome": { "error": {
                "kind": "PermissionDenied",
                "reason": "walker replace denied",
            }},
        });
        let mut expected = vec![
            json!({ "environment_variable": "SKIT_LANG", "result": null }),
            json!({ "environment_variable": "LC_ALL", "result": null }),
            json!({ "environment_variable": "LC_MESSAGES", "result": null }),
            json!({ "environment_variable": "LANG", "result": null }),
            json!({ "system_locale": "ZhCn" }),
            json!({ "platform": "Other" }),
            json!({ "preference": "agent_skill::Inspect", "path": "<profile:fixture>/cwd/.agents/skills/skit/SKILL.md", "outcome": { "ok": true } }),
        ];
        expected.extend(std::iter::repeat_n(failure, 8));
        assert_eq!(
            events_with_keys(
                &failed.transcript,
                &[
                    "environment_variable",
                    "system_locale",
                    "platform",
                    "preference"
                ]
            ),
            expected
        );
    }

    #[test]
    fn launch_transcript_reads_the_program_bytes_at_the_call_boundary() {
        let spec = WalkerSeedSpec {
            profile: "executable".to_owned(),
            settings: BTreeMap::from([
                ("after_run".to_owned(), "stay".to_owned()),
                ("lang".to_owned(), "en".to_owned()),
            ]),
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("tool"),
                bytes: b"executable fixture\n".to_vec(),
                readonly: false,
                unix_mode: 0o751,
            }],
            external_references: vec![WalkerExternalReferenceSeed {
                request: CreateEntry {
                    name: "Tool".to_owned(),
                    kind: EntryKind::parse("exe").unwrap(),
                    mode: StorageMode::Reference,
                    source: String::new(),
                    workdir: "origin".to_owned(),
                    description: String::new(),
                    payload: Some(EntryPayload {
                        bytes: b"executable fixture\n".to_vec(),
                        stored_name: None,
                        permissions: SourcePermissions {
                            readonly: false,
                            unix_mode: Some(0o751),
                        },
                    }),
                    settings: EntrySettings::default(),
                },
                source: PathBuf::from("tool"),
            }],
            ..WalkerSeedSpec::default()
        };
        let mut first = RealWalkerHost::spawn(spec.clone()).unwrap();
        let mut second = RealWalkerHost::spawn(spec).unwrap();
        let run = |host: &mut RealWalkerHost| {
            let mut state = host.initial_state().unwrap();
            host.clear_transcript();
            let action = host
                .dispatch(Effect::Submit {
                    purpose: FormPurpose::Run,
                    selector: Some("Tool".to_owned()),
                    values: BTreeMap::new(),
                })
                .unwrap();
            assert!(matches!(action, Action::Complete { .. }), "{action:?}");
            let _ = state.update(action);
            host.observe(&state).unwrap()
        };
        let first_observation = run(&mut first);
        let second_observation = run(&mut second);
        assert_eq!(first_observation, second_observation);
        let encoded = serde_json::to_string(&first_observation).unwrap();
        assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
        assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
        let launch = first_observation
            .transcript
            .iter()
            .find_map(|event| event.get("launch"))
            .unwrap();

        assert_eq!(
            launch["readable_files"],
            json!([{
                "path": "<profile:executable>/external/tool",
                "content": {
                    "encoding": "utf8",
                    "data": "executable fixture\n",
                },
            }])
        );
    }

    #[test]
    fn trusted_dot_prefixed_external_files_never_become_owned_transient_tokens() {
        let mut spec = WalkerSeedSpec {
            profile: "trusted-dot-files".to_owned(),
            settings: BTreeMap::from([
                ("after_run".to_owned(), "stay".to_owned()),
                ("lang".to_owned(), "en".to_owned()),
            ]),
            ..WalkerSeedSpec::default()
        };
        for (name, path) in [
            ("Trusted run", ".run-report.sh"),
            ("Trusted injected", ".injected-notes"),
        ] {
            spec.external.push(WalkerExternalSeed::File {
                path: PathBuf::from(path),
                bytes: format!("{name}\n").into_bytes(),
                readonly: false,
                unix_mode: 0o751,
            });
            spec.external_references.push(WalkerExternalReferenceSeed {
                request: CreateEntry {
                    name: name.to_owned(),
                    kind: EntryKind::parse("exe").unwrap(),
                    mode: StorageMode::Reference,
                    source: String::new(),
                    workdir: "origin".to_owned(),
                    description: String::new(),
                    payload: None,
                    settings: EntrySettings::default(),
                },
                source: PathBuf::from(path),
            });
        }
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        host.clear_transcript();
        for selector in ["Trusted run", "Trusted injected"] {
            assert!(matches!(
                host.dispatch(Effect::Submit {
                    purpose: FormPurpose::Run,
                    selector: Some(selector.to_owned()),
                    values: BTreeMap::new(),
                })
                .unwrap(),
                Action::Complete { .. }
            ));
        }

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let encoded = serde_json::to_string(&observation.transcript).unwrap();
        assert!(encoded.contains("external/.run-report.sh"));
        assert!(encoded.contains("external/.injected-notes"));
        assert!(!encoded.contains("<run-snapshot:"));
        assert!(!encoded.contains("<injected-source:"));
    }

    #[cfg(unix)]
    #[test]
    fn external_executable_probe_matches_system_execute_permission_semantics() {
        let mut spec = WalkerSeedSpec {
            profile: "execute-bits".to_owned(),
            settings: BTreeMap::from([
                ("after_run".to_owned(), "stay".to_owned()),
                ("lang".to_owned(), "en".to_owned()),
            ]),
            ..WalkerSeedSpec::default()
        };
        for (name, path, mode) in [
            ("Not executable", "plain", 0o640),
            ("Executable", "ready", 0o751),
        ] {
            spec.external.push(WalkerExternalSeed::File {
                path: PathBuf::from(path),
                bytes: format!("{name}\n").into_bytes(),
                readonly: false,
                unix_mode: mode,
            });
            spec.external_references.push(WalkerExternalReferenceSeed {
                request: CreateEntry {
                    name: name.to_owned(),
                    kind: EntryKind::parse("exe").unwrap(),
                    mode: StorageMode::Reference,
                    source: String::new(),
                    workdir: "origin".to_owned(),
                    description: String::new(),
                    payload: None,
                    settings: EntrySettings::default(),
                },
                source: PathBuf::from(path),
            });
        }
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        host.clear_transcript();
        assert!(
            host.dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Not executable".to_owned()),
                values: BTreeMap::new(),
            })
            .is_err()
        );
        assert!(matches!(
            host.dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Executable".to_owned()),
                values: BTreeMap::new(),
            })
            .unwrap(),
            Action::Complete { .. }
        ));
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let executable_probes = observation
            .transcript
            .iter()
            .filter(|event| event["probe"] == "is_executable")
            .map(|event| (event["path"].clone(), event["result"].clone()))
            .collect::<Vec<_>>();
        assert_eq!(
            executable_probes,
            vec![
                (json!("<profile:execute-bits>/external/plain"), json!(false)),
                (json!("<profile:execute-bits>/external/ready"), json!(true)),
                (json!("<profile:execute-bits>/external/ready"), json!(true)),
            ]
        );
        assert_eq!(
            observation
                .transcript
                .iter()
                .filter(|event| event.get("launch").is_some())
                .count(),
            1
        );
    }

    #[test]
    fn observations_keep_full_ui_state_unknown_config_and_port_transcripts() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.adapters.programs.remove("npm");
        let config_path = host.roots().config.join("config.toml");
        let mut config = fs::read_to_string(&config_path).unwrap();
        config.push_str("\n[future]\nanswer = 42\n");
        fs::write(&config_path, config).unwrap();

        let mut state = host.initial_state().unwrap();
        let dependency_effect = Effect::Open {
            request: HostRequest::Run,
            selector: Some("JavaScript".to_owned()),
        };
        assert!(matches!(
            host.dispatch(dependency_effect).unwrap(),
            Action::SetStatus(ref message)
                if message.starts_with("Error:") && message.contains("dependencies")
        ));
        let action = host
            .dispatch(Effect::Open {
                request: HostRequest::Run,
                selector: Some("Command".to_owned()),
            })
            .unwrap();
        let _ = state.update(action);
        let completed = host
            .dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Command".to_owned()),
                values: BTreeMap::from([("value".to_owned(), FieldValue::text("walked"))]),
            })
            .unwrap();
        let _ = state.update(completed);
        assert!(matches!(
            host.dispatch(Effect::None).unwrap(),
            Action::ClearStatus
        ));
        assert!(matches!(
            host.dispatch(Effect::Quit).unwrap(),
            Action::ClearStatus
        ));
        let reload = host.dispatch(Effect::Reload).unwrap();
        let _ = state.update(reload);
        host.dispatch(Effect::Preferences(PreferencesEffect::Save(
            skit_application::preferences::PreferencesChangeSet {
                settings: BTreeMap::from([("after_run".to_owned(), "exit".to_owned())]),
            },
        )))
        .unwrap();
        assert!(
            fs::read_to_string(&config_path)
                .unwrap()
                .contains("answer = 42")
        );

        let observation = host.observe(&state).unwrap();
        let expected_state = super::PathMap::new(
            &host.profile,
            host._sandbox.path(),
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap()
        .normalize_library_json(serde_json::to_value(&state).unwrap())
        .unwrap();
        assert_eq!(observation.state, expected_state);
        assert!(
            observation
                .transcript
                .iter()
                .any(|event| event.get("launch").is_some())
        );
        assert!(
            observation
                .transcript
                .iter()
                .any(|event| event.get("environment_snapshot").is_some())
        );
        assert!(observation.transcript.iter().any(|event| {
            event
                == &json!({
                    "probe": "find_program",
                    "path": "npm",
                    "result": null,
                })
        }));
        assert_eq!(observation.prompt_runner, "walker");
        assert!(observation.tree.iter().any(|row| {
            row.path == "config/config.toml"
                && matches!(
                    &row.content,
                    Some(super::ByteView::Utf8(text)) if text.contains("answer = 42")
                )
        }));
        assert_eq!(
            FileConfigStore::new(&host.roots().config)
                .get("after_run")
                .unwrap(),
            "exit"
        );
    }

    #[test]
    fn run_and_settings_reducer_checkpoints_replay_without_profile_paths() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        let mut first_state = first.initial_state().unwrap();
        let mut second_state = second.initial_state().unwrap();

        for (request, selector) in [
            (HostRequest::Run, "Command"),
            (HostRequest::Settings, "Reference"),
        ] {
            let first_action = first
                .dispatch(Effect::Open {
                    request,
                    selector: Some(selector.to_owned()),
                })
                .unwrap();
            let second_action = second
                .dispatch(Effect::Open {
                    request,
                    selector: Some(selector.to_owned()),
                })
                .unwrap();
            let _ = first_state.update(first_action);
            let _ = second_state.update(second_action);

            let first_observation = first.observe(&first_state).unwrap();
            let second_observation = second.observe(&second_state).unwrap();
            assert_eq!(first_observation.state, second_observation.state);
            let encoded = serde_json::to_string(&first_observation.state).unwrap();
            assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
            assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
        }
    }

    #[test]
    fn run_modal_reducer_checkpoints_replay_without_profile_paths() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        let mut first_state = first.initial_state().unwrap();
        let mut second_state = second.initial_state().unwrap();

        for (host, state) in [(&first, &mut first_state), (&second, &mut second_state)] {
            let action = host
                .dispatch(Effect::Open {
                    request: HostRequest::Run,
                    selector: Some("Command".to_owned()),
                })
                .unwrap();
            assert_eq!(state.update(action), Effect::None);
            assert_eq!(state.update(Action::OpenRunTokenMenuFor(1)), Effect::None);
        }

        let first_tokens = first.observe(&first_state).unwrap();
        let second_tokens = second.observe(&second_state).unwrap();
        assert_eq!(first_tokens, second_tokens);
        assert_eq!(
            first_tokens
                .state
                .pointer("/modal/run_token_menu/options/1/fixed_directory/path")
                .and_then(Value::as_str),
            Some("<profile:fixture>/cwd")
        );
        let tokens = serde_json::to_string(&first_tokens).unwrap();
        assert!(!tokens.contains(&first._sandbox.path().display().to_string()));
        assert!(!tokens.contains(&second._sandbox.path().display().to_string()));

        assert_eq!(
            first_state.update(Action::OpenRunFilePicker(1)),
            Effect::None
        );
        assert_eq!(
            second_state.update(Action::OpenRunFilePicker(1)),
            Effect::None
        );
        let first_picker = first.observe(&first_state).unwrap();
        let second_picker = second.observe(&second_state).unwrap();
        assert_eq!(first_picker, second_picker);
        assert_eq!(
            first_picker
                .state
                .pointer("/modal/run_file_picker/context/workdir")
                .and_then(Value::as_str),
            Some("<profile:fixture>/cwd")
        );
        assert_eq!(
            first_picker
                .state
                .pointer("/modal/run_file_picker/context/invoke_cwd")
                .and_then(Value::as_str),
            Some("<profile:fixture>/cwd")
        );
        let picker = serde_json::to_string(&first_picker).unwrap();
        assert!(!picker.contains(&first._sandbox.path().display().to_string()));
        assert!(!picker.contains(&second._sandbox.path().display().to_string()));
    }

    #[test]
    fn preferences_target_and_install_reducer_checkpoints_replay_without_profile_paths() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        let mut first_state = first.initial_state().unwrap();
        let mut second_state = second.initial_state().unwrap();

        for host in [&first, &second] {
            fs::create_dir_all(host.roots().home.as_ref().unwrap().join(".codex/skills")).unwrap();
        }
        for (host, state) in [(&first, &mut first_state), (&second, &mut second_state)] {
            let open = host
                .dispatch(Effect::Open {
                    request: HostRequest::Preferences,
                    selector: None,
                })
                .unwrap();
            assert_eq!(state.update(open), Effect::None);
            let present = host
                .dispatch(Effect::Preferences(
                    PreferencesEffect::DiscoverAgentSkillTargets,
                ))
                .unwrap();
            assert_eq!(state.update(present), Effect::None);
        }

        let first_targets = first.observe(&first_state).unwrap();
        let second_targets = second.observe(&second_state).unwrap();
        assert_eq!(first_targets, second_targets);
        assert_eq!(
            first_targets
                .state
                .pointer("/workflow/active/preferences/agent_skill_install/targets/0/base",)
                .and_then(Value::as_str),
            Some("<profile:fixture>/home/.codex")
        );
        let targets = serde_json::to_string(&first_targets).unwrap();
        assert!(!targets.contains(&first._sandbox.path().display().to_string()));
        assert!(!targets.contains(&second._sandbox.path().display().to_string()));

        let mut installed_observations = Vec::new();
        for (host, state) in [
            (&mut first, &mut first_state),
            (&mut second, &mut second_state),
        ] {
            let mut request = state.update(Action::Preferences(
                skit_ui::PreferencesAction::ConfirmAgentSkillTarget,
            ));
            assert!(matches!(
                request,
                Effect::Preferences(PreferencesEffect::InstallAgentSkill { .. })
            ));
            let mut response = host.dispatch(request.clone()).unwrap();
            let mut emitted = state.update(response.clone());
            let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
            installed_observations.push(
                host.capture_checkpoint_parts(
                    state,
                    &mut session,
                    CheckpointCauseProjection::Host {
                        request: &mut request,
                        response: &mut response,
                        emitted: &mut emitted,
                    },
                )
                .unwrap(),
            );
        }

        let first_installed = &installed_observations[0];
        let second_installed = &installed_observations[1];
        assert_eq!(first_installed, second_installed);
        assert_eq!(
            first_installed.state.get("status").and_then(Value::as_str),
            Some(
                "Installed the skit Agent Skill: <profile:fixture>/home/.codex/skills/skit/SKILL.md"
            )
        );
        let installed = serde_json::to_string(&first_installed).unwrap();
        assert!(!installed.contains(&first._sandbox.path().display().to_string()));
        assert!(!installed.contains(&second._sandbox.path().display().to_string()));
    }

    #[test]
    fn missing_reference_target_replays_without_profile_paths() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        fs::remove_file(first.external_root.join("outside.sh")).unwrap();
        fs::remove_file(second.external_root.join("outside.sh")).unwrap();

        let first_observation = first.observe(&first.initial_state().unwrap()).unwrap();
        let second_observation = second.observe(&second.initial_state().unwrap()).unwrap();
        assert_eq!(first_observation, second_observation);
        assert_eq!(
            first_observation
                .state
                .pointer("/details/reference/missing_target")
                .and_then(Value::as_str),
            Some("<profile:fixture>/external/outside.sh")
        );
        let encoded = serde_json::to_string(&first_observation).unwrap();
        assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
        assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
    }

    #[test]
    fn modal_and_install_status_lookalikes_remain_user_text() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let raw = format!("{}-user", host._sandbox.path().display());
        let status = format!("Installed the skit Agent Skill: {raw}");
        let value = json!({
            "modal": {
                "run_token_menu": {
                    "field": 0,
                    "options": [{ "fixed_directory": { "path": raw } }],
                },
            },
            "status": status,
        });
        let path_map = &mut host.path_map;
        assert_eq!(
            path_map.normalize_library_json(value.clone()).unwrap(),
            value
        );
    }

    #[test]
    fn command_submit_records_the_exact_low_level_request_and_outcome_sequence() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        host.clear_transcript();

        assert!(matches!(
            host.dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Command".to_owned()),
                values: BTreeMap::from([("value".to_owned(), FieldValue::text("walked"))]),
            })
            .unwrap(),
            Action::Complete { .. }
        ));
        let mut paths = super::PathMap::new(
            &host.profile,
            host._sandbox.path(),
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap();
        let transcript = host.adapters.transcript(&mut paths);

        assert_eq!(
            transcript,
            vec![
                json!({ "clock": "2026-08-28T12:34:56+00:00" }),
                json!({ "local_offset_seconds": 0 }),
                json!({ "environment_snapshot": {
                    "HOME": "/explicit/home",
                    "LANG": "en_US.UTF-8",
                    "SENTINEL": "walker",
                }}),
                json!({ "environment_snapshot": {
                    "HOME": "/explicit/home",
                    "LANG": "en_US.UTF-8",
                    "SENTINEL": "walker",
                }}),
                json!({ "platform": "Other" }),
                json!({ "environment_variable": "COMSPEC", "result": null }),
                json!({ "environment_variable": "SystemRoot", "result": null }),
                json!({ "clock": "2026-08-28T12:34:56+00:00" }),
                json!({ "platform": "Other" }),
                json!({ "output": "diagnostic", "text": "Reusing your last arguments: tail" }),
                json!({ "probe": "is_dir", "path": "<profile:fixture>/cwd", "result": true }),
                json!({ "probe": "find_program", "path": "sh", "result": "/virtual/bin/sh" }),
                json!({ "clock": "2026-08-28T12:34:56+00:00" }),
                json!({ "probe": "is_dir", "path": "<profile:fixture>/cwd", "result": true }),
                json!({ "probe": "find_program", "path": "sh", "result": "/virtual/bin/sh" }),
                json!({ "output": "stdout", "text": "→ /virtual/bin/sh -c 'printf remembered tail'" }),
                json!({ "launch": {
                    "program": "/virtual/bin/sh",
                    "args": ["-c", "printf remembered tail"],
                    "environment": {},
                    "cwd": "<profile:fixture>/cwd",
                    "display": "/virtual/bin/sh -c 'printf remembered tail'",
                    "warnings": [],
                    "readable_files": [],
                    "outcome": { "ok": { "exit_code": 0, "signal": null } },
                }}),
                json!({ "clock": "2026-08-28T12:34:56+00:00" }),
                json!({ "clock": "2026-08-28T12:34:56+00:00" }),
            ]
        );
    }

    #[test]
    fn javascript_run_drives_dependency_and_syntax_success_refusal_and_raw_failure() {
        let submit = |selector: &str| Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some(selector.to_owned()),
            values: if selector == "Injected JavaScript" {
                BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))])
            } else {
                BTreeMap::new()
            },
        };
        let mut success = RealWalkerHost::spawn(profile()).unwrap();
        success
            .adapters
            .dependency_script
            .replace(super::DependencyScript::Completed(
                DependencyCommandOutput {
                    success: true,
                    exit_code: Some(0),
                    stderr: Vec::new(),
                },
            ));
        success.clear_transcript();
        assert!(matches!(
            success.dispatch(submit("JavaScript")).unwrap(),
            Action::Complete { .. }
        ));
        let successful = success.observe(&success.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_parsed_launch_displays(&successful.transcript, &["dependency", "launch"]),
            vec![
                json!({ "dependency": {
                    "program": "/virtual/bin/npm",
                    "args": ["install", "--no-audit", "--no-fund", "--ignore-scripts"],
                    "cwd": "<profile:fixture>/data/scripts/javascript",
                    "environment": {},
                    "outcome": { "ok": {
                        "success": true,
                        "exit_code": 0,
                        "stderr": { "encoding": "utf8", "data": "" },
                    }},
                }}),
                json!({ "launch": {
                    "program": "/virtual/bin/node",
                    "args": ["<profile:fixture>/data/scripts/javascript/script.js"],
                    "environment": {},
                    "cwd": "<profile:fixture>/data/scripts/javascript",
                    "display": ["/virtual/bin/node", "<profile:fixture>/data/scripts/javascript/script.js"],
                    "warnings": [],
                    "readable_files": [{
                        "path": "<profile:fixture>/data/scripts/javascript/script.js",
                        "content": { "encoding": "utf8", "data": "console.log('ok');\n" },
                    }],
                    "outcome": { "ok": { "exit_code": 0, "signal": null }},
                }}),
            ]
        );

        let default_success = RealWalkerHost::spawn(profile()).unwrap();
        assert!(matches!(
            default_success.dispatch(submit("JavaScript")).unwrap(),
            Action::Complete { .. }
        ));
        let dependency_event = |outcome: Value| {
            json!({ "dependency": {
                "program": "/virtual/bin/npm",
                "args": ["install", "--no-audit", "--no-fund", "--ignore-scripts"],
                "cwd": "<profile:fixture>/data/scripts/javascript",
                "environment": {},
                "outcome": outcome,
            }})
        };

        let mut rejected_dependency = RealWalkerHost::spawn(profile()).unwrap();
        rejected_dependency
            .adapters
            .dependency_script
            .replace(super::DependencyScript::Completed(
                DependencyCommandOutput {
                    success: false,
                    exit_code: Some(9),
                    stderr: b"offline \xff".to_vec(),
                },
            ));
        rejected_dependency.clear_transcript();
        assert!(rejected_dependency.dispatch(submit("JavaScript")).is_err());
        let rejected = rejected_dependency
            .observe(&rejected_dependency.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_keys(&rejected.transcript, &["dependency", "launch"]),
            vec![dependency_event(json!({ "ok": {
                "success": false,
                "exit_code": 9,
                "stderr": { "encoding": "hex", "data": "6f66666c696e6520ff" },
            }}))]
        );

        let mut raw_failure = RealWalkerHost::spawn(profile()).unwrap();
        raw_failure
            .adapters
            .dependency_script
            .replace(super::DependencyScript::Failure {
                kind: io::ErrorKind::NotFound,
                reason: "walker npm missing".to_owned(),
            });
        raw_failure.clear_transcript();
        assert!(raw_failure.dispatch(submit("JavaScript")).is_err());
        let raw = raw_failure
            .observe(&raw_failure.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_keys(&raw.transcript, &["dependency", "launch"]),
            vec![dependency_event(json!({ "error": {
                "kind": "NotFound",
                "reason": "walker npm missing",
            }}))]
        );

        let mut gate_spec = profile();
        gate_spec.entries.push(injected_script(
            "js",
            "Injected JavaScript",
            "const TOKEN = 'before';\nconsole.log(TOKEN);\n",
            "script.js",
        ));

        let mut successful_gate = RealWalkerHost::spawn(gate_spec.clone()).unwrap();
        successful_gate.clear_transcript();
        assert!(matches!(
            successful_gate
                .dispatch(submit("Injected JavaScript"))
                .unwrap(),
            Action::Complete { .. }
        ));
        let successful_gate = successful_gate
            .observe(&successful_gate.initial_state().unwrap())
            .unwrap();
        let expected_source = "// /// script\n// [tool.skit]\n// schema = 1\n//\n// [[tool.skit.params]]\n// default = \"before\"\n// kind = \"const\"\n// name = \"TOKEN\"\n// type = \"str\"\n// ///\nconst TOKEN = \"after\";\nconsole.log(TOKEN);\n";
        assert_eq!(
            events_with_parsed_launch_displays(
                &successful_gate.transcript,
                &["javascript_gate", "launch"]
            ),
            vec![
                json!({ "javascript_gate": {
                    "program": "/virtual/bin/node",
                    "source": "<profile:fixture>/system-temp/<injected-source:0>.js",
                    "timeout_ms": 30_000,
                    "content": { "encoding": "utf8", "data": expected_source },
                    "outcome": { "ok": {
                        "success": true,
                        "stderr": { "encoding": "utf8", "data": "" },
                    }},
                }}),
                json!({ "launch": {
                    "program": "/virtual/bin/node",
                    "args": ["<profile:fixture>/system-temp/<injected-source:0>.js"],
                    "environment": {},
                    "cwd": "<profile:fixture>/cwd",
                    "display": ["/virtual/bin/node", "<profile:fixture>/system-temp/<injected-source:0>.js"],
                    "warnings": [],
                    "readable_files": [{
                        "path": "<profile:fixture>/system-temp/<injected-source:0>.js",
                        "content": { "encoding": "utf8", "data": expected_source },
                    }],
                    "outcome": { "ok": { "exit_code": 0, "signal": null }},
                }}),
            ]
        );

        let mut rejected_gate = RealWalkerHost::spawn(gate_spec.clone()).unwrap();
        rejected_gate.adapters.javascript_gate_script.replace(
            super::JavaScriptGateScript::Completed(JavaScriptSyntaxGateOutput {
                success: false,
                stderr: b"syntax rejected".to_vec(),
            }),
        );
        rejected_gate.clear_transcript();
        assert!(
            rejected_gate
                .dispatch(submit("Injected JavaScript"))
                .is_err()
        );
        let gate = rejected_gate
            .observe(&rejected_gate.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_keys(&gate.transcript, &["javascript_gate", "launch"]),
            vec![json!({ "javascript_gate": {
                "program": "/virtual/bin/node",
                "source": "<profile:fixture>/system-temp/<injected-source:0>.js",
                "timeout_ms": 30_000,
                "content": { "encoding": "utf8", "data": expected_source },
                "outcome": { "ok": {
                    "success": false,
                    "stderr": { "encoding": "utf8", "data": "syntax rejected" },
                }},
            }})]
        );

        let mut unavailable_gate = RealWalkerHost::spawn(gate_spec).unwrap();
        unavailable_gate.adapters.javascript_gate_script.replace(
            super::JavaScriptGateScript::Unavailable(JavaScriptSyntaxGateUnavailable::Spawn {
                reason: "walker node gate unavailable".to_owned(),
            }),
        );
        unavailable_gate.clear_transcript();
        assert!(matches!(
            unavailable_gate
                .dispatch(submit("Injected JavaScript"))
                .unwrap(),
            Action::Complete { .. }
        ));
        let unavailable = unavailable_gate
            .observe(&unavailable_gate.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_parsed_launch_displays(
                &unavailable.transcript,
                &["javascript_gate", "launch"]
            ),
            vec![
                json!({ "javascript_gate": {
                "program": "/virtual/bin/node",
                "source": "<profile:fixture>/system-temp/<injected-source:0>.js",
                "timeout_ms": 30_000,
                "content": { "encoding": "utf8", "data": expected_source },
                "outcome": {
                    "unavailable": "could not run node syntax check: walker node gate unavailable"
                },
                }}),
                json!({ "launch": {
                    "program": "/virtual/bin/node",
                    "args": ["<profile:fixture>/system-temp/<injected-source:0>.js"],
                    "environment": {},
                    "cwd": "<profile:fixture>/cwd",
                    "display": ["/virtual/bin/node", "<profile:fixture>/system-temp/<injected-source:0>.js"],
                    "warnings": [],
                    "readable_files": [{
                        "path": "<profile:fixture>/system-temp/<injected-source:0>.js",
                        "content": { "encoding": "utf8", "data": expected_source },
                    }],
                    "outcome": { "ok": { "exit_code": 0, "signal": null }},
                }}),
            ]
        );
    }

    #[test]
    fn shell_injection_drives_success_rejection_and_unavailable_gate_outcomes() {
        let mut spec = profile();
        spec.entries.push(injected_script(
            "shell",
            "Injected shell",
            "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
            "script.sh",
        ));
        let submit = || Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Injected shell".to_owned()),
            values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
        };

        let mut success = RealWalkerHost::spawn(spec.clone()).unwrap();
        success.clear_transcript();
        assert!(matches!(
            success.dispatch(submit()).unwrap(),
            Action::Complete { .. }
        ));
        let successful = success.observe(&success.initial_state().unwrap()).unwrap();
        let expected_source = "# /// script\n# [tool.skit]\n# schema = 1\n#\n# [[tool.skit.params]]\n# default = \"before\"\n# kind = \"const\"\n# name = \"TOKEN\"\n# type = \"str\"\n# ///\nTOKEN='after'\nprintf '%s\\n' \"$TOKEN\"\n";
        assert_eq!(
            events_with_parsed_launch_displays(&successful.transcript, &["injected", "launch"]),
            vec![
                json!({ "injected": {
                    "program": "/virtual/bin/bash",
                    "args": ["-n", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                    "timeout_ms": 30_000,
                    "readable_files": [{
                        "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                        "content": { "encoding": "utf8", "data": expected_source },
                    }],
                    "outcome": { "ok": {
                        "success": true,
                        "stderr": { "encoding": "utf8", "data": "" },
                    }},
                }}),
                json!({ "launch": {
                    "program": "/virtual/bin/bash",
                    "args": ["<profile:fixture>/system-temp/<injected-source:0>.sh"],
                    "environment": {},
                    "cwd": "<profile:fixture>/cwd",
                    "display": ["/virtual/bin/bash", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                    "warnings": [],
                    "readable_files": [{
                        "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                        "content": { "encoding": "utf8", "data": expected_source },
                    }],
                    "outcome": { "ok": { "exit_code": 0, "signal": null }},
                }}),
            ]
        );
        let encoded = serde_json::to_string(&successful).unwrap();
        assert!(!encoded.contains("/tmp/.injected-"), "{encoded}");
        assert_system_temp_is_empty(&success);

        let mut rejected = RealWalkerHost::spawn(spec.clone()).unwrap();
        rejected
            .adapters
            .injected_script
            .replace(super::InjectedScript::Completed(InjectedCommandOutput {
                success: false,
                stderr: b"shell rejected".to_vec(),
            }));
        rejected.clear_transcript();
        assert!(rejected.dispatch(submit()).is_err());
        let rejected_observation = rejected
            .observe(&rejected.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_keys(&rejected_observation.transcript, &["injected", "launch"],),
            vec![json!({ "injected": {
                "program": "/virtual/bin/bash",
                "args": ["-n", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "timeout_ms": 30_000,
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": { "ok": {
                    "success": false,
                    "stderr": { "encoding": "utf8", "data": "shell rejected" },
                }},
            }})]
        );
        assert_system_temp_is_empty(&rejected);

        let mut unavailable = RealWalkerHost::spawn(spec).unwrap();
        unavailable
            .adapters
            .injected_script
            .replace(super::InjectedScript::Unavailable(
                InjectedCommandUnavailable::Spawn {
                    reason: "walker shell gate unavailable".to_owned(),
                },
            ));
        unavailable.clear_transcript();
        assert!(matches!(
            unavailable.dispatch(submit()).unwrap(),
            Action::Complete { .. }
        ));
        let unavailable_observation = unavailable
            .observe(&unavailable.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_parsed_launch_displays(
                &unavailable_observation.transcript,
                &["injected", "launch"]
            ),
            vec![
                json!({ "injected": {
                "program": "/virtual/bin/bash",
                "args": ["-n", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                "timeout_ms": 30_000,
                "readable_files": [{
                    "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                    "content": { "encoding": "utf8", "data": expected_source },
                }],
                "outcome": {
                    "unavailable": "could not run the injected-source check: walker shell gate unavailable"
                },
                }}),
                json!({ "launch": {
                    "program": "/virtual/bin/bash",
                    "args": ["<profile:fixture>/system-temp/<injected-source:0>.sh"],
                    "environment": {},
                    "cwd": "<profile:fixture>/cwd",
                    "display": ["/virtual/bin/bash", "<profile:fixture>/system-temp/<injected-source:0>.sh"],
                    "warnings": [],
                    "readable_files": [{
                        "path": "<profile:fixture>/system-temp/<injected-source:0>.sh",
                        "content": { "encoding": "utf8", "data": expected_source },
                    }],
                    "outcome": { "ok": { "exit_code": 0, "signal": null }},
                }}),
            ]
        );
        assert_system_temp_is_empty(&unavailable);
    }

    #[test]
    fn injected_stage_write_failure_removes_the_partial_system_temp_file() {
        let mut spec = profile();
        spec.entries.push(injected_script(
            "shell",
            "Injected shell",
            "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
            "script.sh",
        ));
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        let fault = StageWriteFaultGuard::for_current_thread();
        host.clear_transcript();

        let result = host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Injected shell".to_owned()),
            values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
        });
        drop(fault);

        assert!(result.is_err());
        let _ = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_system_temp_is_empty(&host);
    }

    #[test]
    fn injected_runner_failure_removes_the_system_temp_file() {
        let mut spec = profile();
        spec.entries.push(injected_script(
            "shell",
            "Injected shell",
            "TOKEN=before\nprintf '%s\\n' \"$TOKEN\"\n",
            "script.sh",
        ));
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        host.fail_next_launch(io::ErrorKind::PermissionDenied, "walker launch denied");
        host.clear_transcript();

        let result = host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Injected shell".to_owned()),
            values: BTreeMap::from([("value:TOKEN".to_owned(), FieldValue::text("after"))]),
        });

        assert!(result.is_err());
        let _ = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_system_temp_is_empty(&host);
    }

    #[test]
    fn uv_bootstrap_drives_consent_transport_and_invalid_archive_outcomes() {
        let mut spec = profile();
        spec.entries.push(CreateEntry {
            name: "Python uv".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"print('uv')\n".to_vec(),
                stored_name: Some("script.py".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        });
        let submit = || Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Python uv".to_owned()),
            values: BTreeMap::new(),
        };
        let asset =
            skit_runtime::uv_asset(&skit_runtime::UvTarget::current().unwrap(), None).unwrap();

        let mut declined = RealWalkerHost::spawn(spec.clone()).unwrap();
        declined.adapters.programs.remove("uv");
        declined.clear_transcript();
        assert!(declined.dispatch(submit()).is_err());
        let declined = declined
            .observe(&declined.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_keys(&declined.transcript, &["uv_consent", "uv_fetch", "launch"]),
            vec![json!({ "uv_consent": {
                "version": skit_runtime::UV_VERSION,
                "destination": "<profile:fixture>/data/bin",
                "result": false,
            }})]
        );

        let mut transport = RealWalkerHost::spawn(spec.clone()).unwrap();
        transport.adapters.programs.remove("uv");
        transport.adapters.uv_consent.set(true);
        transport
            .adapters
            .uv_fetch_script
            .replace(super::UvFetchScript::Failure(
                "walker network offline".to_owned(),
            ));
        transport.clear_transcript();
        assert!(transport.dispatch(submit()).is_err());
        let transport = transport
            .observe(&transport.initial_state().unwrap())
            .unwrap();
        assert_eq!(
            events_with_keys(&transport.transcript, &["uv_consent", "uv_fetch", "launch"]),
            vec![
                json!({ "uv_consent": {
                    "version": skit_runtime::UV_VERSION,
                    "destination": "<profile:fixture>/data/bin",
                    "result": true,
                }}),
                json!({ "uv_fetch": {
                    "url": asset.url.clone(),
                    "limit": 104_857_600_u64,
                    "outcome": { "error": "walker network offline" },
                }}),
            ]
        );

        let mut invalid = RealWalkerHost::spawn(spec.clone()).unwrap();
        invalid.adapters.programs.remove("uv");
        invalid.adapters.uv_consent.set(true);
        invalid
            .adapters
            .uv_fetch_script
            .replace(super::UvFetchScript::Bytes(Vec::new()));
        invalid.clear_transcript();
        assert!(invalid.dispatch(submit()).is_err());
        let invalid = invalid.observe(&invalid.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(&invalid.transcript, &["uv_consent", "uv_fetch", "launch"]),
            vec![
                json!({ "uv_consent": {
                    "version": skit_runtime::UV_VERSION,
                    "destination": "<profile:fixture>/data/bin",
                    "result": true,
                }}),
                json!({ "uv_fetch": {
                    "url": asset.url.clone(),
                    "limit": 104_857_600_u64,
                    "outcome": { "ok": {
                        "length": 0,
                        "sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
                    }},
                }}),
            ]
        );

        let fetch_outcome = |bytes: Vec<u8>| {
            let mut host = RealWalkerHost::spawn(spec.clone()).unwrap();
            host.adapters.programs.remove("uv");
            host.adapters.uv_consent.set(true);
            host.adapters
                .uv_fetch_script
                .replace(super::UvFetchScript::Bytes(bytes));
            host.clear_transcript();
            assert!(host.dispatch(submit()).is_err());
            host.observe(&host.initial_state().unwrap())
                .unwrap()
                .transcript
                .into_iter()
                .find_map(|event| event.get("uv_fetch").cloned())
                .unwrap()["outcome"]
                .clone()
        };
        let first_outcome = fetch_outcome(vec![1, 2, 3, 4]);
        let second_outcome = fetch_outcome(vec![4, 3, 2, 1]);
        assert_eq!(first_outcome["ok"]["length"], 4);
        assert_eq!(second_outcome["ok"]["length"], 4);
        assert_ne!(
            first_outcome["ok"]["sha256"],
            second_outcome["ok"]["sha256"]
        );

        let mut managed = RealWalkerHost::spawn(spec).unwrap();
        managed.adapters.programs.remove("uv");
        managed.adapters.uv_consent.set(true);
        let managed_uv = skit_runtime::managed_uv_path(&managed.roots().data);
        let managed_relative = format!(
            "data/bin/{}",
            managed_uv.file_name().unwrap().to_str().unwrap()
        );
        let projected_uv = format!("<profile:fixture>/{managed_relative}");
        fs::create_dir_all(managed_uv.parent().unwrap()).unwrap();
        fs::write(&managed_uv, b"managed uv fixture\n").unwrap();
        super::set_unix_mode(&managed_uv, 0o751).unwrap();
        managed.clear_transcript();
        assert!(matches!(
            managed.dispatch(submit()).unwrap(),
            Action::Complete { .. }
        ));
        let managed = managed.observe(&managed.initial_state().unwrap()).unwrap();
        let managed_executable = managed
            .tree
            .iter()
            .find(|row| row.path == managed_relative)
            .unwrap();
        #[cfg(unix)]
        assert_eq!(managed_executable.mode, Some(0o751));
        #[cfg(not(unix))]
        assert_eq!(managed_executable.mode, None);
        assert!(
            !managed
                .transcript
                .iter()
                .any(|event| event.get("uv_consent").is_some() || event.get("uv_fetch").is_some())
        );
        assert_eq!(
            events_with_parsed_launch_displays(&managed.transcript, &["launch"]),
            vec![json!({ "launch": {
                "program": projected_uv,
                "args": [
                    "run",
                    "--no-project",
                    "--script",
                    "<profile:fixture>/data/scripts/python-uv/script.py",
                ],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": [projected_uv, "run", "--no-project", "--script", "<profile:fixture>/data/scripts/python-uv/script.py"],
                "warnings": [],
                "readable_files": [
                    {
                        "path": projected_uv,
                        "content": { "encoding": "utf8", "data": "managed uv fixture\n" },
                    },
                    {
                        "path": "<profile:fixture>/data/scripts/python-uv/script.py",
                        "content": { "encoding": "utf8", "data": "print('uv')\n" },
                    },
                ],
                "outcome": { "ok": { "exit_code": 0, "signal": null }},
            }})]
        );
    }

    #[test]
    fn launch_io_failure_is_raw_in_the_transcript_and_does_not_complete_state() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        host.clear_transcript();
        host.fail_next_launch(io::ErrorKind::PermissionDenied, "walker launch denied");

        let error = host
            .dispatch(Effect::Submit {
                purpose: FormPurpose::Run,
                selector: Some("Command".to_owned()),
                values: BTreeMap::from([("value".to_owned(), FieldValue::text("walked"))]),
            })
            .unwrap_err();
        assert!(error.to_string().contains("walker launch denied"));
        let mut paths = super::PathMap::new(
            &host.profile,
            host._sandbox.path(),
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap();
        let transcript = host.adapters.transcript(&mut paths);
        assert_eq!(
            transcript.last(),
            Some(&json!({ "launch": {
                "program": "/virtual/bin/sh",
                "args": ["-c", "printf remembered tail"],
                "environment": {},
                "cwd": "<profile:fixture>/cwd",
                "display": "/virtual/bin/sh -c 'printf remembered tail'",
                "warnings": [],
                "readable_files": [],
                "outcome": {
                    "error": {
                        "kind": "PermissionDenied",
                        "reason": "walker launch denied",
                    }
                },
            }}))
        );
        let state = FormStateService::new(FileFormStateStore::new(&host.roots().state))
            .load(&Slug::parse("command").unwrap());
        assert_eq!(state.last_run.exit, Some(7));
        assert_eq!(
            state.last_run.at.as_deref(),
            Some("2026-08-28T11:00:00+00:00")
        );
    }

    #[test]
    fn rejected_preflight_is_localized_and_never_enters_the_effect_server() {
        let process_cwd = std::env::current_dir().unwrap();
        let ambient_executable = tempfile::Builder::new()
            .prefix("skit-walker-cwd-")
            .suffix("-program")
            .tempfile_in(process_cwd)
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(ambient_executable.path(), fs::Permissions::from_mode(0o751))
                .unwrap();
        }
        let bare_program = ambient_executable
            .path()
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("the generated program name is UTF-8")
            .to_owned();
        let mut spec = profile();
        spec.settings.insert("lang".to_owned(), "zh-TW".to_owned());
        let javascript = spec
            .entries
            .iter_mut()
            .find(|entry| entry.name == "JavaScript")
            .expect("the fixture has one JavaScript entry");
        javascript.settings.interpreter = bare_program.clone();
        let host = RealWalkerHost::spawn(spec).unwrap();
        let effect = Effect::Open {
            request: HostRequest::Run,
            selector: Some("JavaScript".to_owned()),
        };
        host.clear_transcript();
        let error = host.preflight(&effect).unwrap_err();
        let expected_transcript = {
            let mut paths = super::PathMap::new(
                &host.profile,
                host._sandbox.path(),
                &host.service,
                &host.file_picker_tree.files,
            )
            .unwrap();
            host.adapters.transcript(&mut paths)
        };
        assert!(expected_transcript.iter().any(|event| {
            event
                == &json!({
                    "probe": "find_program",
                    "path": bare_program,
                    "result": null,
                })
        }));
        let expected_action = Action::SetStatus(
            Message::new("Error: {}")
                .nested(error.message())
                .localize(Locale::ZhTw),
        );

        host.clear_transcript();
        assert_eq!(host.dispatch(effect).unwrap(), expected_action);
        let mut paths = super::PathMap::new(
            &host.profile,
            host._sandbox.path(),
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap();
        assert_eq!(host.adapters.transcript(&mut paths), expected_transcript);
    }

    #[test]
    fn profiles_are_isolated_and_one_host_cannot_mutate_another() {
        let first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        let second_before = second.observe(&second.initial_state().unwrap()).unwrap();

        fs::write(
            first.external_root.join("outside.sh"),
            b"changed only in first\n",
        )
        .unwrap();

        first
            .dispatch(Effect::Remove {
                selector: "Command".to_owned(),
            })
            .unwrap();

        assert!(
            first.service.list().unwrap().entries.len()
                < second.service.list().unwrap().entries.len()
        );
        assert_eq!(
            second.observe(&second.initial_state().unwrap()).unwrap(),
            second_before
        );
        assert_eq!(
            fs::read(second.external_root.join("outside.sh")).unwrap(),
            b"printf outside\n"
        );
    }

    #[test]
    fn external_seed_paths_refuse_every_lexical_escape_before_writing() {
        let outside = tempfile::TempDir::new().unwrap();
        let sentinel = outside.path().join("sentinel");
        fs::write(&sentinel, b"outside stays exact\n").unwrap();
        let cases = [
            WalkerSeedSpec {
                external: vec![WalkerExternalSeed::File {
                    path: sentinel.clone(),
                    bytes: b"clobber\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                }],
                ..WalkerSeedSpec::default()
            },
            WalkerSeedSpec {
                external: vec![WalkerExternalSeed::File {
                    path: PathBuf::from("../escape"),
                    bytes: b"escape\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                }],
                ..WalkerSeedSpec::default()
            },
            WalkerSeedSpec {
                external: vec![WalkerExternalSeed::Symlink {
                    path: PathBuf::from("safe-link"),
                    target: PathBuf::from("../escape"),
                }],
                ..WalkerSeedSpec::default()
            },
            WalkerSeedSpec {
                external: vec![WalkerExternalSeed::Symlink {
                    path: PathBuf::from("../escape-link"),
                    target: PathBuf::from("safe-target"),
                }],
                ..WalkerSeedSpec::default()
            },
            WalkerSeedSpec {
                external_references: vec![WalkerExternalReferenceSeed {
                    request: command("Escape reference"),
                    source: PathBuf::from("../escape"),
                }],
                ..WalkerSeedSpec::default()
            },
        ];

        for spec in cases {
            assert!(
                RealWalkerHost::spawn(spec)
                    .unwrap_err()
                    .contains("relative descendants")
            );
            assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays exact\n");
        }
    }

    #[test]
    fn every_known_and_future_entry_kind_reopens_through_the_production_store() {
        let mut spec = WalkerSeedSpec {
            profile: "kinds".to_owned(),
            settings: BTreeMap::from([("lang".to_owned(), "en".to_owned())]),
            ..WalkerSeedSpec::default()
        };
        for (kind, stored_name, bytes) in [
            ("python", "script.py", b"print(1)\n".as_slice()),
            ("shell", "script.sh", b"printf ok\n".as_slice()),
            ("fish", "script.fish", b"echo ok\n".as_slice()),
            ("js", "script.js", b"console.log('ok');\n".as_slice()),
            ("ts", "script.ts", b"console.log('ok');\n".as_slice()),
            ("powershell", "script.ps1", b"Write-Output ok\n".as_slice()),
            ("ruby", "script.rb", b"puts 'ok'\n".as_slice()),
            ("perl", "script.pl", b"print qq(ok);\n".as_slice()),
            ("lua", "script.lua", b"print('ok')\n".as_slice()),
            ("r", "script.r", b"print('ok')\n".as_slice()),
            ("prompt", "prompt.md", b"Review this\n".as_slice()),
            ("future-kind", "payload", b"future bytes\n".as_slice()),
        ] {
            spec.entries.push(CreateEntry {
                name: format!("Kind {kind}"),
                kind: EntryKind::parse(kind).unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "store".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: bytes.to_vec(),
                    stored_name: Some(stored_name.to_owned()),
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            });
        }
        spec.entries.push(command("Kind command"));
        spec.external.push(WalkerExternalSeed::File {
            path: PathBuf::from("tool"),
            bytes: b"executable bytes\n".to_vec(),
            readonly: false,
            unix_mode: 0o751,
        });
        spec.external_references.push(WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Kind exe".to_owned(),
                kind: EntryKind::parse("exe").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"executable bytes\n".to_vec(),
                    stored_name: None,
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("tool"),
        });

        let host = RealWalkerHost::spawn(spec).unwrap();
        let kinds = host
            .service
            .list()
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| entry.kind.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            kinds,
            [
                "command",
                "exe",
                "fish",
                "future-kind",
                "js",
                "lua",
                "perl",
                "powershell",
                "prompt",
                "python",
                "r",
                "ruby",
                "shell",
                "ts",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect()
        );
    }

    #[test]
    fn normalization_keeps_user_text_that_only_looks_like_an_id_or_path() {
        let mut spec = profile();
        spec.entries.push(CreateEntry {
            name: "Normalizer fields".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "printf {identity} {id} {path}".to_owned(),
                params: vec!["identity".to_owned(), "id".to_owned(), "path".to_owned()],
                parameters: ["identity", "id", "path"]
                    .into_iter()
                    .map(ParamDecl::new)
                    .collect(),
                ..EntrySettings::default()
            },
        });
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        let command = host.service.show("Command").unwrap();
        let id = command.meta.id.as_ref().unwrap().as_str();
        let root_suffix = format!("{}-user-suffix", host._sandbox.path().display());
        let description = format!("literal user text: {id} and {root_suffix}",);
        host.service.describe(&command, &description).unwrap();
        let normalizer = host.service.show("Normalizer fields").unwrap();
        let declarations = super::super::entry_parameters(host.service.repository(), &normalizer);
        FormStateService::new(FileFormStateStore::new(&host.roots().state))
            .save_last(
                &normalizer.slug,
                &declarations,
                Some(&BTreeMap::from([
                    ("identity".to_owned(), "literal identity".to_owned()),
                    ("id".to_owned(), id.to_owned()),
                    ("path".to_owned(), root_suffix.clone()),
                ])),
                None,
                false,
            )
            .unwrap();
        let unknown_meta = host.roots().home.as_ref().unwrap().join("meta.toml");
        fs::write(
            &unknown_meta,
            format!("identity = \"literal identity\"\nid = \"{id}\"\npath = \"{root_suffix}\"\n"),
        )
        .unwrap();
        host.dispatch(Effect::Submit {
            purpose: FormPurpose::Run,
            selector: Some("Command".to_owned()),
            values: BTreeMap::from([("value:value".to_owned(), FieldValue::text(&root_suffix))]),
        })
        .unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let encoded = serde_json::to_string_pretty(&observation).unwrap();
        let normalizer_values = &observation.form_state["normalizer-fields"]["values"];

        let encoded_description = serde_json::to_string(&description).unwrap();
        assert!(encoded.contains(&encoded_description[1..encoded_description.len() - 1]));
        assert!(encoded.contains("<entry-id:command>"));
        assert_eq!(
            normalizer_values,
            &json!({
                "identity": "literal identity",
                "id": id,
                "path": root_suffix,
            })
        );
        assert!(observation.tree.iter().any(|row| {
            row.path == "home/meta.toml"
                && matches!(&row.content, Some(super::ByteView::Utf8(text))
                    if text.contains("identity = \"literal identity\"")
                        && text.contains(id)
                        && text.contains(&root_suffix))
        }));
        let encoded_suffix = serde_json::to_string(&root_suffix).unwrap();
        assert!(
            serde_json::to_string(&observation.transcript)
                .expect("the transcript is serializable")
                .contains(&encoded_suffix[1..encoded_suffix.len() - 1])
        );
        let mut path_map = super::PathMap::new(
            &host.profile,
            host._sandbox.path(),
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap();
        let user_screen = json!({
            "workflow": {
                "active": {
                    "form": {
                        "path": host.roots().data.join("user-value"),
                        "identity": "literal identity",
                    }
                },
                "history": [],
            }
        });
        assert_eq!(
            path_map
                .normalize_library_json(user_screen.clone())
                .unwrap(),
            user_screen
        );
        path_map.modified_ranks.assign_sorted(&[99]).unwrap();
        let add_source_path = host.roots().data.join("literal-user-input");
        let add_workflow = json!({
            "workflow": {
                "active": {
                    "add": {
                        "source": {
                            "path": add_source_path,
                            "drafts": [{
                                "path": host.roots().data.join("typed-draft"),
                                "identity": {
                                    "platform": "unix",
                                    "device": 7,
                                    "inode": 8,
                                    "change_time_seconds": 9,
                                    "change_time_nanoseconds": 10,
                                },
                                "modified": 99,
                            }],
                        },
                        "pending_source": {
                            "path": host.roots().data.join("typed-artifact"),
                            "identity": {
                                "platform": "unix",
                                "device": 1,
                                "inode": 2,
                                "change_time_seconds": 3,
                                "change_time_nanoseconds": 4,
                            },
                        },
                        "review": { "source": {
                            "path": host.roots().data.join("review-artifact"),
                            "identity": {
                                "platform": "unix",
                                "device": 2,
                                "inode": 3,
                                "change_time_seconds": 4,
                                "change_time_nanoseconds": 5,
                            },
                        }},
                        "pending_delete": ["draft", {
                            "path": host.roots().data.join("delete-artifact"),
                            "identity": {
                                "platform": "unix",
                                "device": 3,
                                "inode": 4,
                                "change_time_seconds": 5,
                                "change_time_nanoseconds": 6,
                            },
                        }],
                        "delete_candidate": {
                            "path": host.roots().data.join("delete-candidate"),
                            "identity": {
                                "platform": "unix",
                                "device": 4,
                                "inode": 5,
                                "change_time_seconds": 6,
                                "change_time_nanoseconds": 7,
                            },
                        },
                    }
                },
                "history": [{ "add": {
                    "pending_source": {
                        "path": host.roots().data.join("history-artifact"),
                        "identity": {
                            "platform": "unix",
                            "device": 5,
                            "inode": 6,
                            "change_time_seconds": 7,
                            "change_time_nanoseconds": 8,
                        },
                    }
                }}],
            }
        });
        let normalized_add = path_map
            .normalize_library_json(add_workflow.clone())
            .unwrap();
        assert_eq!(
            path_map.normalize_library_json(add_workflow).unwrap(),
            normalized_add
        );
        assert_eq!(
            normalized_add["workflow"]["active"]["add"]["source"]["path"],
            json!(add_source_path)
        );
        assert_eq!(
            normalized_add["workflow"]["active"]["add"]["source"]["drafts"][0],
            json!({
                "path": "<profile:fixture>/data/typed-draft",
                "identity": {
                    "platform": "unix",
                    "device": 0,
                    "inode": 0,
                    "change_time_seconds": 0,
                    "change_time_nanoseconds": 0,
                },
                "modified": 0,
            })
        );
        assert_eq!(
            normalized_add["workflow"]["active"]["add"]["pending_source"],
            json!({
                "path": "<profile:fixture>/data/typed-artifact",
                "identity": {
                    "platform": "unix",
                    "device": 0,
                    "inode": 1,
                    "change_time_seconds": 0,
                    "change_time_nanoseconds": 0,
                },
            })
        );
        assert_eq!(
            normalized_add["workflow"]["active"]["add"]["pending_delete"][1]["path"],
            "<profile:fixture>/data/delete-artifact"
        );
        assert_eq!(
            normalized_add["workflow"]["history"][0]["add"]["pending_source"]["path"],
            "<profile:fixture>/data/history-artifact"
        );
        let mut missing_identity = json!({
            "path": host.roots().data.join("missing"),
            "identity": null,
        });
        path_map
            .normalize_artifact_object(&mut missing_identity)
            .unwrap();
        assert_eq!(missing_identity["identity"], Value::Null);
        let mut scalar = json!("not an artifact");
        path_map.normalize_artifact_object(&mut scalar).unwrap();
        assert_eq!(scalar, "not an artifact");
        let unregistered_identity = json!({
            "platform": "unix",
            "device": 9,
            "inode": 9,
            "change_time_seconds": 9,
            "change_time_nanoseconds": 9,
        });
        let mut unregistered = json!({
            "path": "/outside/user-value",
            "identity": unregistered_identity.clone(),
        });
        path_map
            .normalize_artifact_object(&mut unregistered)
            .unwrap();
        assert_eq!(unregistered["identity"], unregistered_identity);
        let mut unknown_identity = json!({
            "path": host.roots().data.join("unknown-identity"),
            "identity": { "platform": "plan9" },
        });
        assert_eq!(
            path_map
                .normalize_artifact_object(&mut unknown_identity)
                .unwrap_err(),
            "source identity platform is unknown: plan9"
        );
        let mut unscanned_modified = json!({
            "path": host.roots().data.join("unscanned-draft"),
            "modified": 100,
        });
        assert_eq!(
            path_map
                .normalize_artifact_object(&mut unscanned_modified)
                .unwrap_err(),
            "the sorted scan did not assign a rank to this draft modified value: 100"
        );
        let unknown = || {
            json!({
                "path": host.roots().data.join("unknown-chain"),
                "identity": { "platform": "plan9" },
            })
        };
        assert_eq!(
            path_map
                .normalize_action_json(json!({
                    "present": { "add": { "pending_source": unknown() } },
                }))
                .unwrap_err(),
            "a serialized Action is invalid: unknown variant `plan9`, expected `unix` or `windows`"
        );
        assert_eq!(
            path_map
                .normalize_draft_list(json!([unknown()]))
                .unwrap_err(),
            "source identity platform is unknown: plan9"
        );
        assert_eq!(
            path_map
                .normalize_library_json(json!({
                    "workflow": {
                        "active": { "add": { "pending_source": unknown() } },
                    },
                }))
                .unwrap_err(),
            "source identity platform is unknown: plan9"
        );
        assert_eq!(
            path_map
                .normalize_library_json(json!({
                    "workflow": {
                        "history": [{ "add": { "pending_source": unknown() } }],
                    },
                }))
                .unwrap_err(),
            "source identity platform is unknown: plan9"
        );
        assert_eq!(
            path_map.normalize_library_json(json!({ "workflow": {} })),
            Ok(json!({ "workflow": {} }))
        );
        assert!(
            path_map
                .normalize_action_json(json!("not an action"))
                .is_err()
        );
        assert_eq!(
            path_map.normalize_draft_list(json!("not a draft list")),
            Ok(json!("not a draft list"))
        );
        assert!(
            path_map
                .normalize_action_json(json!({
                    "present": {
                        "settings": {
                            "sections": [{
                                "items": [{
                                    "item": "note",
                                    "text": "Keep a copy — your original file is never modified. Source: {}",
                                }],
                            }],
                        },
                    },
                }))
                .is_err()
        );
        assert_eq!(
            path_map.normalize_path(host._sandbox.path()),
            "<profile:fixture>"
        );
        path_map.register_owned_transient_path(Path::new("/"), "run-snapshot", ".run-");
        assert!(!path_map.transient_paths.contains_key(Path::new("/")));
        path_map.register_owned_transient_path(Path::new("ordinary.txt"), "run-snapshot", ".run-");
        assert!(
            !path_map
                .transient_paths
                .contains_key(Path::new("ordinary.txt"))
        );
        path_map
            .text_artifacts
            .insert("root".to_owned(), "stable".to_owned());
        assert_eq!(path_map.normalize_host_text("unchanged"), "unchanged");
        assert_eq!(path_map.normalize_host_text("root-suffix"), "root-suffix");
        assert_eq!(
            path_map.normalize_host_text("'root' root"),
            "'stable' stable"
        );
    }

    #[test]
    fn managed_documents_keep_unknown_bytes_and_invalid_cache_proofs_visible() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let meta_path = host.roots().data.join("scripts/command/meta.toml");
        let mut raw_meta = fs::read_to_string(&meta_path).unwrap();
        raw_meta.push_str(
            "\n# future comment stays byte-visible\n[future]\nwhen = 1979-05-27T07:32:00Z\nspaced    =    \"literal\"\n",
        );
        fs::write(&meta_path, &raw_meta).unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let viewed_meta = observation
            .tree
            .iter()
            .find(|row| row.path == "data/scripts/command/meta.toml")
            .and_then(|row| row.content.as_ref())
            .unwrap();
        let viewed_meta = serde_json::to_value(viewed_meta).unwrap();
        assert_eq!(viewed_meta["encoding"], "utf8");
        let viewed_meta = viewed_meta["data"].as_str().unwrap();
        assert!(viewed_meta.contains("# future comment stays byte-visible"));
        assert!(viewed_meta.contains("when = 1979-05-27T07:32:00Z"));
        assert!(viewed_meta.contains("spaced    =    \"literal\""));
        assert_eq!(fs::read_to_string(&meta_path).unwrap(), raw_meta);

        let invalid_registry = r#"[entries.command]
mtime_ns = "not-an-integer"

[entries.command.skit_cache]
schema = 1
platform = "unix"
file_id = ""
file_size = "01"
modified_ns = "bad"
changed_ns = "bad"
metadata_hash = "not-a-sha"
projection_hash = "not-a-sha"
"#;
        let path_map = &host.path_map;
        assert_eq!(
            path_map.byte_view(
                &host.roots().data.join("registry.toml"),
                invalid_registry.as_bytes(),
            ),
            super::ByteView::Utf8(invalid_registry.to_owned())
        );
        assert_eq!(
            path_map.byte_view(&host.roots().data.join("registry.toml"), b"not = [toml"),
            super::ByteView::Utf8("not = [toml".to_owned())
        );
        for registry in ["future = true\n", "[entries]\ncommand = \"not a row\"\n"] {
            assert_eq!(
                path_map.byte_view(
                    &host.roots().data.join("registry.toml"),
                    registry.as_bytes(),
                ),
                super::ByteView::Utf8(registry.to_owned())
            );
        }
        assert_eq!(
            path_map.byte_view(
                &host.roots().data.join("registry.toml"),
                b"[entries.command]\nmtime_ns = 7\n",
            ),
            super::ByteView::Utf8("[entries.command]\nmtime_ns = \"<mtime-ns>\"\n".to_owned())
        );
        #[cfg(unix)]
        let (current_platform, foreign_platform) = ("unix", "windows");
        #[cfg(windows)]
        let (current_platform, foreign_platform) = ("windows", "unix");
        #[cfg(not(any(unix, windows)))]
        let (current_platform, foreign_platform) = ("unsupported", "unix");
        for (platform, file_size) in [(foreign_platform, "1"), (current_platform, "-1")] {
            let invalid_proof = format!(
                "[entries.command.skit_cache]\n\
                 schema = 1\n\
                 platform = \"{platform}\"\n\
                 file_id = \"fixture\"\n\
                 file_size = \"{file_size}\"\n\
                 modified_ns = \"1\"\n\
                 changed_ns = \"1\"\n\
                 metadata_hash = \"sha256:{}\"\n\
                 projection_hash = \"sha256:{}\"\n",
                "0".repeat(64),
                "1".repeat(64),
            );
            assert_eq!(
                path_map.byte_view(
                    &host.roots().data.join("registry.toml"),
                    invalid_proof.as_bytes(),
                ),
                super::ByteView::Utf8(invalid_proof)
            );
        }

        let mut missing_id = raw_meta.parse::<toml_edit::DocumentMut>().unwrap();
        missing_id.remove("id");
        fs::write(&meta_path, missing_id.to_string()).unwrap();
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        assert!(observation.tree.iter().any(|row| {
            row.path == "data/scripts/command/meta.toml"
                && matches!(&row.content, Some(super::ByteView::Utf8(text))
                    if !text.lines().any(|line| line.starts_with("id =")))
        }));
    }

    #[cfg(unix)]
    #[test]
    fn corrupt_siblings_and_symlink_topology_stay_visible_without_following_outside() {
        use std::os::unix::fs::PermissionsExt as _;

        let mut spec = profile();
        spec.external.extend([
            WalkerExternalSeed::File {
                path: PathBuf::from("directory/must-not-be-followed"),
                bytes: b"outside\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            },
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("reference-link.sh"),
                target: PathBuf::from("outside.sh"),
            },
        ]);
        spec.external_references[0].source = PathBuf::from("reference-link.sh");
        spec.entries.push(command("Corrupt"));
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        let outside_before = outside_snapshot_portable(&host.external_root);
        std::os::unix::fs::symlink(
            host.external_root.join("directory"),
            host.roots().home.as_ref().unwrap().join("outside-link"),
        )
        .unwrap();
        let corrupt = host.roots().data.join("scripts/corrupt/meta.toml");
        fs::write(&corrupt, b"invalid = [toml\n").unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert!(observation.surface.to_string().contains("corrupt"));
        assert!(observation.surface.to_string().contains("command"));
        assert!(observation.tree.iter().any(|row| {
            row.path == "home/outside-link" && row.kind == "symlink" && row.content.is_none()
        }));
        let reference_link = host.external_root.join("reference-link.sh");
        let reference_row = observation
            .tree
            .iter()
            .find(|row| row.path == "external/reference-link.sh")
            .unwrap();
        assert_eq!(reference_row.kind, "symlink");
        assert_eq!(
            reference_row.mode,
            super::portable_mode(&fs::symlink_metadata(&reference_link).unwrap())
        );
        assert_eq!(
            host.observation_modes
                .borrow()
                .expected_kind(&reference_link)
                .unwrap(),
            Some(super::ObservationNodeKind::Symlink)
        );
        assert!(
            !observation
                .tree
                .iter()
                .any(|row| row.path.starts_with("home/outside-link/"))
        );
        assert_eq!(
            outside_snapshot_portable(&host.external_root),
            outside_before
        );
        assert!(
            fs::symlink_metadata(host.external_root.join("reference-link.sh"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(
            fs::read(host.external_root.join("outside.sh")).unwrap(),
            b"printf outside\n"
        );
        assert_eq!(
            fs::metadata(host.external_root.join("outside.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }

    #[cfg(unix)]
    #[test]
    fn reference_and_executable_external_sources_keep_symlink_provenance() {
        let mut spec = profile();
        spec.external.extend([
            WalkerExternalSeed::File {
                path: PathBuf::from("tool"),
                bytes: b"tool\n".to_vec(),
                readonly: false,
                unix_mode: 0o751,
            },
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("reference-link.sh"),
                target: PathBuf::from("outside.sh"),
            },
            WalkerExternalSeed::Symlink {
                path: PathBuf::from("tool-link"),
                target: PathBuf::from("tool"),
            },
        ]);
        spec.external_references[0].source = PathBuf::from("reference-link.sh");
        spec.external_references.push(WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Linked executable".to_owned(),
                kind: EntryKind::parse("exe").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            },
            source: PathBuf::from("tool-link"),
        });
        let mut host = RealWalkerHost::spawn(spec).unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        for path in ["reference-link.sh", "tool-link"] {
            let physical = host.external_root.join(path);
            let row = observation
                .tree
                .iter()
                .find(|row| row.path == format!("external/{path}"))
                .unwrap();
            assert_eq!(row.kind, "symlink");
            assert_eq!(
                row.mode,
                super::portable_mode(&fs::symlink_metadata(&physical).unwrap())
            );
            assert_eq!(
                host.observation_modes
                    .borrow()
                    .expected_kind(&physical)
                    .unwrap(),
                Some(super::ObservationNodeKind::Symlink)
            );
        }
    }

    #[test]
    fn production_form_seeding_never_persists_secret_values() {
        let mut secret = ParamDecl::new("token");
        secret.secret = true;
        let mut spec = profile();
        spec.entries.push(CreateEntry {
            name: "Secret".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "printf {token}".to_owned(),
                parameters: vec![secret],
                ..EntrySettings::default()
            },
        });
        spec.forms.push(WalkerFormSeed {
            selector: "Secret".to_owned(),
            values: BTreeMap::from([("token".to_owned(), "must-not-persist".to_owned())]),
            ..WalkerFormSeed::default()
        });

        let mut host = RealWalkerHost::spawn(spec).unwrap();
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert_eq!(observation.form_state["secret"]["values"], json!({}));
        assert!(
            !serde_json::to_string(&observation)
                .unwrap()
                .contains("must-not-persist")
        );
    }

    #[test]
    fn form_seeding_uses_legacy_params_and_source_comment_declarations() {
        let legacy = CreateEntry {
            name: "Legacy params".to_owned(),
            kind: EntryKind::parse("command").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: None,
            settings: EntrySettings {
                template: "printf {legacy}".to_owned(),
                params: vec!["legacy".to_owned()],
                parameters: Vec::new(),
                ..EntrySettings::default()
            },
        };
        let mut source_declaration = ParamDecl::new("SOURCE_ONLY");
        source_declaration.binding = ParameterBinding::Const;
        source_declaration.delivery = ParameterDelivery::Inject;
        source_declaration.default = Some(ParameterValue::String("before".to_owned()));
        let source = write_managed_params(
            "shell",
            "SOURCE_ONLY=before\nprintf '%s\\n' \"$SOURCE_ONLY\"\n",
            std::slice::from_ref(&source_declaration),
        )
        .unwrap();
        let source_only = CreateEntry {
            name: "Source comment".to_owned(),
            kind: EntryKind::parse("shell").unwrap(),
            mode: StorageMode::Copy,
            source: String::new(),
            workdir: "store".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: source.into_bytes(),
                stored_name: Some("script.sh".to_owned()),
                permissions: SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        };
        let form = |selector: &str, name: &str| WalkerFormSeed {
            selector: selector.to_owned(),
            values: BTreeMap::from([(name.to_owned(), "saved".to_owned())]),
            preset: Some("fixture".to_owned()),
            last_run: Some(WalkerLastRunSeed {
                exit: 3,
                at: "2026-08-28T10:00:00+00:00".to_owned(),
                values: Some(BTreeMap::from([(name.to_owned(), "launched".to_owned())])),
            }),
            ..WalkerFormSeed::default()
        };
        let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
            profile: "declarations".to_owned(),
            entries: vec![legacy, source_only],
            settings: BTreeMap::from([("lang".to_owned(), "en".to_owned())]),
            forms: vec![
                form("Legacy params", "legacy"),
                form("Source comment", "SOURCE_ONLY"),
            ],
            ..WalkerSeedSpec::default()
        })
        .unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        for (slug, name) in [
            ("legacy-params", "legacy"),
            ("source-comment", "SOURCE_ONLY"),
        ] {
            assert_eq!(observation.form_state[slug]["values"][name], "saved");
            assert_eq!(
                observation.form_state[slug]["presets"]["fixture"][name],
                "saved"
            );
            assert_eq!(
                observation.form_state[slug]["last_run"]["values"][name],
                "launched"
            );
            assert_eq!(observation.form_state[slug]["last_run"]["exit"], 3);
        }
    }

    fn write_draft_at(
        host: &RealWalkerHost,
        name: &str,
        bytes: &[u8],
        modified_seconds: u64,
    ) -> PathBuf {
        let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
        let path = drafts.join(name);
        fs::write(&path, bytes).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH
                    + std::time::Duration::from_secs(modified_seconds),
            ))
            .unwrap();
        path
    }

    // APFS refuses a non-UTF-8 file name, so only Linux can write this fixture.
    #[cfg(target_os = "linux")]
    fn write_non_utf_draft_at(host: &RealWalkerHost) -> PathBuf {
        use std::os::unix::ffi::OsStringExt as _;

        let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
        let path = drafts.join(std::ffi::OsString::from_vec(
            b"skit-non-utf-\xff.unknown".to_vec(),
        ));
        fs::write(&path, b"opaque draft\n").unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10),
            ))
            .unwrap();
        path
    }

    fn write_unicode_draft_at(host: &RealWalkerHost, name: &str, bytes: &[u8]) -> PathBuf {
        write_draft_at(host, name, bytes, 10)
    }

    fn render_real_session_snapshot(state: &LibraryState, session: &mut TuiSession) -> Value {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| {
                let _ = render_with_session(frame, state, Locale::En, session);
            })
            .unwrap();
        serde_json::to_value(session.agent_review_snapshot().unwrap()).unwrap()
    }

    fn capture_real_reducer_checkpoint(
        host: &mut RealWalkerHost,
        state: &LibraryState,
        session: &mut TuiSession,
        action: Action,
        emitted: Effect,
    ) -> (HostObservation, Action, Effect, Value) {
        let mut captured_state = state.clone();
        let mut captured_session = render_real_session_snapshot(state, session);
        let mut recorded_action = action;
        let mut recorded_emitted = emitted;
        let observation = host
            .capture_checkpoint_parts(
                &mut captured_state,
                &mut captured_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_action,
                    emitted: &mut recorded_emitted,
                },
            )
            .unwrap();
        (
            observation,
            recorded_action,
            recorded_emitted,
            captured_session,
        )
    }

    fn picker_add_state(host: &RealWalkerHost) -> LibraryState {
        let mut state = host.initial_state().unwrap();
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        state
    }

    fn picker_tui_session(host: &RealWalkerHost) -> TuiSession {
        let tree = host.file_picker_tree();
        TuiSession::with_file_picker_tree(tree.root, tree.directories, tree.files)
    }

    fn source_input_value(session: &Value) -> &str {
        session
            .pointer("/add/fields/inputs")
            .and_then(Value::as_array)
            .and_then(|inputs| inputs.iter().find(|input| input["id"] == "SourcePath"))
            .and_then(|input| input.pointer("/state/value"))
            .and_then(Value::as_str)
            .expect("the source stage owns one source-path input")
    }

    fn capture_real_host_checkpoint(
        host: &mut RealWalkerHost,
        state: &LibraryState,
        session: &mut TuiSession,
        request: Effect,
        response: Action,
        emitted: Effect,
    ) -> (HostObservation, Action, Effect, Value) {
        let mut captured_state = state.clone();
        let mut captured_session = render_real_session_snapshot(state, session);
        let mut recorded_request = request;
        let mut recorded_response = response;
        let mut recorded_emitted = emitted;
        let observation = host
            .capture_checkpoint_parts(
                &mut captured_state,
                &mut captured_session,
                CheckpointCauseProjection::Host {
                    request: &mut recorded_request,
                    response: &mut recorded_response,
                    emitted: &mut recorded_emitted,
                },
            )
            .unwrap();
        (
            observation,
            recorded_response,
            recorded_emitted,
            captured_session,
        )
    }

    fn assert_real_draft_transition_projects_and_replays(
        name: &str,
        bytes: &[u8],
        expected_stage: &str,
        derived_pointer: &str,
        raw_derived: &str,
        stable_path: &str,
        stable_derived: &str,
    ) {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_unicode_draft_at(&host, name, bytes);
        host.path_map.refresh(&host.service).unwrap();
        let mut state = host.initial_state().unwrap();
        assert_eq!(
            state.update(Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(super::sorted_tui_drafts(&host.roots().data)),
            )))),
            Effect::None
        );
        let mut session = TuiSession::default();

        let select = Action::Add(AddAction::SelectDraft(0));
        let select_emitted = state.update(select.clone());
        let _ = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            select,
            select_emitted,
        );
        let continue_action = Action::Add(AddAction::Continue);
        let inspect = state.update(continue_action.clone());
        let (inspecting, _, _, _) = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            continue_action,
            inspect.clone(),
        );
        let response = host.dispatch(inspect.clone()).unwrap();
        let emitted = state.update(response.clone());
        let raw = serde_json::to_value(&state).unwrap();
        assert_eq!(
            raw.pointer("/workflow/active/add/stage"),
            Some(&json!(expected_stage))
        );
        assert_eq!(raw.pointer(derived_pointer), Some(&json!(raw_derived)));

        let (projected, recorded_response, recorded_emitted, _) = capture_real_host_checkpoint(
            &mut host,
            &state,
            &mut session,
            inspect,
            response,
            emitted,
        );
        assert_eq!(
            projected
                .state
                .pointer("/workflow/active/add/pending_source/path")
                .filter(|value| !value.is_null())
                .or_else(|| {
                    projected
                        .state
                        .pointer("/workflow/active/add/review/source/path")
                }),
            Some(&json!(stable_path))
        );
        assert_eq!(
            projected.state.pointer(derived_pointer),
            Some(&json!(stable_derived))
        );
        super::super::tui_real_walker::validate_reducer_action(
            &inspecting.state,
            &projected.state,
            &serde_json::to_value(recorded_response).unwrap(),
            &serde_json::to_value(recorded_emitted).unwrap(),
        )
        .unwrap();
    }

    fn source_edited_response(request: &Effect, source: Value) -> Action {
        let request =
            serde_json::to_value(request).unwrap()["add"][0]["edit_source"]["request"].clone();
        super::deserialize_canonical(
            json!({"add": {"source_edited": {
                "request": request,
                "result": {"Ok": source},
            }}}),
            "C2 real SourceEdited response",
        )
        .unwrap()
    }

    fn set_review_name_projection(
        host: &mut RealWalkerHost,
        path: &Path,
        raw_name: &str,
        stable_name: &str,
    ) {
        host.path_map.add_provenance.review_name = Some(super::ReviewNameProjection {
            source_path: path.to_path_buf(),
            kind: "python".to_owned(),
            raw_name: raw_name.to_owned(),
            stable_name: stable_name.to_owned(),
        });
    }

    fn real_file_picker_session(host: &RealWalkerHost, draft: &Path) -> Value {
        let drafts = super::sorted_tui_drafts(&host.roots().data);
        let mut state = host.initial_state().unwrap();
        assert_eq!(
            state.update(Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(drafts),
            )))),
            Effect::None
        );
        let picker_root = draft.parent().unwrap().to_path_buf();
        let mut session = TuiSession::with_file_picker_tree(
            picker_root.clone(),
            BTreeSet::from([picker_root]),
            BTreeSet::from([draft.to_path_buf()]),
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();
        let binding = session
            .local_action_inventory()
            .actions
            .iter()
            .find(|action| action.target == LocalActionTarget::Add(AddControlId::BrowseSource))
            .unwrap()
            .keys[0];
        assert_eq!(
            session.handle_event(Event::Key(binding.event()), &state, &geometry),
            EventHandling::Consumed
        );
        serde_json::to_value(session.agent_review_snapshot().unwrap()).unwrap()
    }

    fn c2_memory_picker(root: &Path, draft: Option<&Path>) -> Value {
        let mut files = vec![json!(root.join("plain.txt"))];
        let mut entries = vec![json!({
            "name": "plain.txt",
            "path": root.join("plain.txt"),
            "entry_type": {"file": {"extension": "txt", "size": 0}},
        })];
        if let Some(draft) = draft {
            files.push(json!(draft));
            entries.push(json!({
                "name": draft.file_name().unwrap().to_string_lossy(),
                "path": draft,
                "entry_type": {"file": {"extension": "py", "size": 12}},
            }));
        }
        entries.push(json!({
            "name": "linked.txt",
            "path": root.join("linked.txt"),
            "entry_type": {"symlink": {"target": root.join("target.txt")}},
        }));
        json!({
            "kind": "file_picker",
            "fields": {
                "contract": {
                    "purpose": "source",
                    "start_dir": root,
                    "selection": "file",
                    "allow_multiple": true,
                    "show_hidden": false,
                    "query": format!("{}-query", root.display()),
                    "output_policy": {"relative_to": root},
                },
                "explorer": {
                    "current_dir": root,
                    "entries": entries,
                    "cursor_index": 0,
                    "scroll": 0,
                    "selected_files": files,
                    "show_hidden": false,
                    "mode": "browse",
                    "search_query": format!("{}-search", root.display()),
                    "filtered_indices": null,
                },
                "query": {
                    "value": format!("{}-widget", root.display()),
                    "cursor": root.display().to_string().chars().count() + 7,
                    "yank": "",
                    "last_was_cut": false,
                },
                "current_directory_focused": false,
                "visible_height": 1,
                "io_error": format!("Could not read {}.", root.display()),
                "footer_scroll": {"content_length": 0, "scroll_offset": 0},
                "footer_viewport": {"x": 0, "y": 0, "width": 0, "height": 0},
                "footer_visible_height": 0,
                "click": null,
                "query_editable": null,
                "memory_source": {
                    "kind": "memory_file_picker_source",
                    "fields": {
                        "root": root,
                        "directories": [root, root.join("directory")],
                        "files": files,
                    },
                },
            },
        })
    }

    fn c2_session(root: &Path, draft: Option<&Path>, inputs: Vec<Value>) -> Value {
        let picker = c2_memory_picker(root, draft);
        let stage = if inputs.iter().any(|input| input["id"] == "ReviewName") {
            "review"
        } else {
            "source"
        };
        json!({
            "schema_version": 1,
            "preferences": {
                "kind": "preferences",
                "fields": {
                    "agent_signature": [{"name": "codex", "scope": "user", "base": root}],
                },
            },
            "add": {
                "kind": "add",
                "fields": {
                    "signature": {
                        "stage": stage,
                        "kind": null,
                        "storage": null,
                        "dependency_surface": null,
                        "interpolate": null,
                        "drafts": 0,
                        "candidates": [],
                        "prompt_candidates": [],
                        "runners": [],
                    },
                    "picker_root": root,
                    "advertised": [{
                        "event": {"open_path_picker": {
                            "purpose": "source",
                            "start_dir": root,
                            "selection": "file",
                            "allow_multiple": false,
                            "show_hidden": false,
                            "query": format!("{}-advertised", root.display()),
                            "output_policy": {"relative_to": root},
                        }},
                    }],
                    "inputs": inputs,
                },
            },
            "path_suggestions": {
                "kind": "path_suggestions",
                "fields": {
                    "expected": {"request": {
                        "value": format!("{}-request", root.display()),
                        "context": {
                            "workdir": root,
                            "tokens": {
                                "cwd": root,
                                "home": root,
                                "env": {"ROOT": root},
                            },
                        },
                    }},
                    "visible": {
                        "value": format!("{}-visible-value", root.display()),
                        "suggestion": root.join("suggestion"),
                    },
                },
            },
            "run_modal": {
                "kind": "run_modal",
                "fields": {
                    "signature": {"file": {
                        "context": {"workdir": root, "invoke_cwd": root},
                    }},
                    "file": picker.clone(),
                    "file_picker_source": picker["fields"]["memory_source"].clone(),
                },
            },
            "add_overlay": {
                "kind": "add_file_overlay",
                "fields": {"session": picker.clone(), "geometry": null},
            },
            "file_picker_source": picker["fields"]["memory_source"].clone(),
        })
    }

    #[test]
    fn c2_host_text_boundary_grammar_and_root_registration_contract() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        let root = host._sandbox.path().display().to_string();
        let stable = "<profile:fixture>";

        assert_eq!(host.path_map.normalize_host_text(&root), stable);
        for (left, right) in [
            (" ", " "),
            ("'", "'"),
            ("\"", "\""),
            ("(", ")"),
            ("[", "]"),
            ("{", "}"),
            (":", ","),
            ("=", "/"),
            ("/", "\\"),
            ("，", "。"),
            ("；", "："),
            ("！", "？"),
            ("、", "（"),
            ("）", "【"),
            ("】", "「"),
            ("」", "『"),
            ("』", "《"),
            ("《", "》"),
        ] {
            assert_eq!(
                host.path_map
                    .normalize_host_text(&format!("{left}{root}{right}")),
                format!("{left}{stable}{right}")
            );
        }
        assert_eq!(
            host.path_map.normalize_host_text(&format!("{root}.")),
            format!("{stable}.")
        );
        assert_eq!(
            host.path_map.normalize_host_text(&format!("{root}~~⟧")),
            format!("{stable}~~⟧")
        );
        for lookalike in [
            format!("x{root}"),
            format!("{root}x"),
            format!("_{root}"),
            format!("{root}_"),
            format!("-{root}"),
            format!("{root}-suffix"),
            format!("{root}.py"),
            format!("{root}. trailing"),
            format!(".{root}"),
            format!("{root}~~⟧ trailing"),
        ] {
            assert_eq!(host.path_map.normalize_host_text(&lookalike), lookalike);
        }
        let unregistered = host
            ._sandbox
            .path()
            .parent()
            .unwrap()
            .join("unregistered-profile")
            .display()
            .to_string();
        assert_eq!(
            host.path_map.normalize_host_text(&unregistered),
            unregistered
        );
    }

    #[test]
    fn c2_exact_localized_agent_install_templates_contract() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let root = host._sandbox.path().display().to_string();
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            let raw = format_text(locale, "Installed the skit Agent Skill: {}", &[&root]);
            let stable = format_text(
                locale,
                "Installed the skit Agent Skill: {}",
                &[&"<profile:fixture>"],
            );
            let mut action = Action::Preferences(PreferencesAction::AgentSkillInstalled {
                message: raw.clone(),
            });
            host.path_map.project_typed_action(&mut action).unwrap();
            assert_eq!(
                serde_json::to_value(action).unwrap()["preferences"]["agent_skill_installed"]["message"],
                stable
            );

            let state = host
                .path_map
                .normalize_library_json(json!({"status": raw.clone()}))
                .unwrap();
            assert_eq!(state["status"], raw);
            let unregistered = format_text(
                locale,
                "Installed the skit Agent Skill: {}",
                &[&"/unregistered/skills/skit/SKILL.md"],
            );
            let action = host
                .path_map
                .normalize_action_json(json!({
                    "preferences": {
                        "agent_skill_installed": {"message": unregistered.clone()},
                    },
                }))
                .unwrap();
            assert_eq!(
                action["preferences"]["agent_skill_installed"]["message"],
                unregistered
            );
            for partial in [format!("prefix {raw}"), format!("{raw} suffix")] {
                assert!(
                    host.path_map
                        .normalize_action_json(json!({
                            "preferences": {"agent_skill_installed": {"message": partial.clone()}},
                        }))
                        .is_err()
                );
                assert_eq!(
                    host.path_map
                        .normalize_library_json(json!({"status": partial}))
                        .unwrap()["status"],
                    partial
                );
            }
        }
    }

    #[test]
    fn c2_session_owner_and_file_picker_pointer_contract() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(&host, "skit-new-session.py", b"print('session')\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        let root = host._sandbox.path().to_path_buf();
        let root_text = root.display().to_string();
        let draft_text = serde_json::to_string(&draft).unwrap();
        let query = format!("{root_text}-query");
        let mut session = c2_session(&root, Some(&draft), Vec::new());

        host.path_map.project_session_value(&mut session).unwrap();

        let encoded = serde_json::to_string(&session).unwrap();
        assert!(!encoded.contains(&draft_text));
        assert!(encoded.contains("<profile:fixture>"));
        assert!(encoded.contains("<draft:0>.py"));

        assert_eq!(
            session["preferences"]["fields"]["agent_signature"][0]["base"],
            "<profile:fixture>"
        );
        assert_eq!(session["add"]["fields"]["picker_root"], "<profile:fixture>");
        assert_eq!(
            session["run_modal"]["fields"]["file"]["fields"]["contract"]["query"],
            query
        );
        assert_eq!(
            session["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"][1]["name"],
            "<draft:0>.py"
        );
        assert_eq!(
            session["run_modal"]["fields"]["file"]["fields"]["io_error"],
            "Could not read <profile:fixture>."
        );
        assert_eq!(
            session["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"][2]["entry_type"]
                ["symlink"]["target"],
            "<profile:fixture>/target.txt"
        );

        let mut token_session = c2_session(&root, None, Vec::new());
        token_session["run_modal"]["fields"]["signature"] = json!({
            "token": {
                "field": 0,
                "options": ["environment", {"fixed_directory": {"path": root}}],
            },
        });
        host.path_map
            .project_session_value(&mut token_session)
            .unwrap();
        assert_eq!(
            token_session["run_modal"]["fields"]["signature"]["token"]["options"][1]["fixed_directory"]
                ["path"],
            "<profile:fixture>"
        );
    }

    #[test]
    fn c2_select_draft_provenance_projects_state_and_untouched_input_mirror() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(
            &host,
            "skit-new-provenance.py",
            b"print('provenance')\n",
            10,
        );
        let mut state = host.initial_state().unwrap();
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        let action = Action::Add(AddAction::SelectDraft(0));
        let emitted = state.update(action.clone());
        let raw = draft.display().to_string();
        let mut projected_state = state.clone();
        let mut session = c2_session(
            host._sandbox.path(),
            Some(&draft),
            vec![json!({
                "id": "SourcePath",
                "state": {
                    "value": raw,
                    "cursor": draft.display().to_string().chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        let mut recorded_action = action;
        let mut recorded_emitted = emitted;

        host.capture_checkpoint_parts(
            &mut projected_state,
            &mut session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_action,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();

        let projected = serde_json::to_value(projected_state).unwrap();
        let expected = "<profile:fixture>/data/.drafts/<draft:0>.py";
        assert_eq!(
            projected.pointer("/workflow/active/add/source/path"),
            Some(&json!(expected))
        );
        assert_eq!(
            session["add"]["fields"]["inputs"][0]["state"]["value"],
            expected
        );
        assert_eq!(
            session["add"]["fields"]["inputs"][0]["state"]["cursor"],
            expected.chars().count()
        );

        let user_action = Action::Add(AddAction::SetSourcePath(draft.display().to_string()));
        let user_emitted = state.update(user_action.clone());
        let mut user_state = state.clone();
        let mut user_session = c2_session(
            host._sandbox.path(),
            Some(&draft),
            vec![json!({
                "id": "SourcePath",
                "state": {
                    "value": draft.display().to_string(),
                    "cursor": draft.display().to_string().chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        let mut recorded_user_action = user_action;
        let mut recorded_user_emitted = user_emitted;
        host.capture_checkpoint_parts(
            &mut user_state,
            &mut user_session,
            CheckpointCauseProjection::Reducer {
                action: &mut recorded_user_action,
                emitted: &mut recorded_user_emitted,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(user_state).unwrap()["workflow"]["active"]["add"]["source"]["path"],
            draft.display().to_string()
        );
        assert_eq!(
            user_session["add"]["fields"]["inputs"][0]["state"]["value"],
            draft.display().to_string()
        );
    }

    #[test]
    fn g2c_picked_source_projects_action_state_session_and_replayed_inspection() {
        let exercise = |host: &mut RealWalkerHost| {
            let raw = host
                .external_root
                .join("nested/picked.py")
                .display()
                .to_string();
            let stable = "<profile:fixture>/external/nested/picked.py";
            let mut state = picker_add_state(host);
            let mut session = picker_tui_session(host);

            let picked_action = Action::Add(AddAction::PickedSourcePath(raw.clone()));
            let picked_effect = state.update(picked_action.clone());
            assert_eq!(picked_effect, Effect::None);
            let (picked, recorded_pick, recorded_pick_effect, projected_session) =
                capture_real_reducer_checkpoint(
                    host,
                    &state,
                    &mut session,
                    picked_action,
                    picked_effect,
                );
            let recorded_pick = serde_json::to_value(recorded_pick).unwrap();
            assert_eq!(recorded_pick["add"]["picked_source_path"], stable);
            assert_eq!(
                picked.state.pointer("/workflow/active/add/source/path"),
                Some(&json!(stable))
            );
            assert_eq!(source_input_value(&projected_session), stable);
            assert!(
                !serde_json::to_string(&recorded_pick)
                    .unwrap()
                    .contains(&raw)
            );
            assert!(!serde_json::to_string(&picked.state).unwrap().contains(&raw));
            assert!(
                !serde_json::to_string(&projected_session)
                    .unwrap()
                    .contains(&raw)
            );
            assert_eq!(recorded_pick_effect, Effect::None);

            let continue_action = Action::Add(AddAction::Continue);
            let inspect = state.update(continue_action.clone());
            let (inspecting, recorded_continue, recorded_inspect, _) =
                capture_real_reducer_checkpoint(
                    host,
                    &state,
                    &mut session,
                    continue_action,
                    inspect,
                );
            let recorded_inspect_value = serde_json::to_value(&recorded_inspect).unwrap();
            assert_eq!(
                recorded_inspect_value["add"][0]["inspect_source"]["path"],
                stable
            );
            super::super::tui_real_walker::validate_reducer_action(
                &picked.state,
                &inspecting.state,
                &serde_json::to_value(&recorded_continue).unwrap(),
                &recorded_inspect_value,
            )
            .unwrap();

            json!({
                "pick": recorded_pick,
                "state": picked.state,
                "session": projected_session,
                "continue": recorded_continue,
                "inspect": recorded_inspect,
            })
        };

        let spec = picker_provenance_profile();
        let mut main = RealWalkerHost::spawn(spec.clone()).unwrap();
        let mut replay = RealWalkerHost::spawn(spec).unwrap();
        assert_eq!(exercise(&mut main), exercise(&mut replay));
    }

    #[test]
    fn g2c_picked_source_refuses_every_non_file_and_state_action_mismatch() {
        let spec = picker_provenance_profile();
        let mut host = RealWalkerHost::spawn(spec.clone()).unwrap();
        let other = RealWalkerHost::spawn(spec).unwrap();
        let invalid = [
            host.external_root.join("unseeded.py"),
            host.external_root.join("nested"),
            host.external_root.join("linked.py"),
            host._sandbox.path().join("outside-root.py"),
            other.external_root.join("nested/picked.py"),
            PathBuf::from(format!(
                "{}/nested//picked.py",
                host.external_root.display()
            )),
            host.external_root.join("nested/./picked.py"),
        ];
        for path in invalid {
            let mut action = Action::Add(AddAction::PickedSourcePath(path.display().to_string()));
            assert!(
                host.path_map.project_typed_action(&mut action).is_err(),
                "accepted non-file picker provenance"
            );
        }

        let raw = host
            .external_root
            .join("nested/picked.py")
            .display()
            .to_string();
        let mut mismatched_value =
            json!({"picked_source_path": host.external_root.join("outside.sh")});
        assert!(
            host.path_map
                .project_typed_add_action(
                    &AddAction::PickedSourcePath(raw.clone()),
                    &mut mismatched_value,
                )
                .unwrap_err()
                .contains("does not match its typed action")
        );
        let mismatch = host.external_root.join("outside.sh").display().to_string();
        let mut state = picker_add_state(&host);
        let picked = Action::Add(AddAction::PickedSourcePath(raw));
        assert_eq!(state.update(picked.clone()), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SetSourcePath(mismatch))),
            Effect::None
        );
        let mut session = picker_tui_session(&host);
        let mut projected_state = state.clone();
        let mut projected_session = render_real_session_snapshot(&state, &mut session);
        let mut recorded_pick = picked;
        let mut recorded_effect = Effect::None;
        let error = host
            .capture_checkpoint_parts(
                &mut projected_state,
                &mut projected_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_pick,
                    emitted: &mut recorded_effect,
                },
            )
            .unwrap_err();
        assert!(error.contains("does not match the raw state"), "{error}");
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn g2c_picked_source_refuses_without_an_active_add_and_cleans_the_checkpoint() {
        for (profile_id, request) in [
            ("picked-on-library", None),
            ("picked-on-preferences", Some(HostRequest::Preferences)),
        ] {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let mut host = RealWalkerHost::spawn_stable_in(
                picker_provenance_profile(),
                safe_profile(profile_id),
                super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            )
            .unwrap();
            let sandbox_root = host.sandbox_root().to_path_buf();
            let mut state = host.initial_state().unwrap();
            if let Some(request) = request {
                let presented = host
                    .dispatch(Effect::Open {
                        request,
                        selector: None,
                    })
                    .unwrap();
                assert_eq!(state.update(presented), Effect::None);
            }
            let before_state = state.clone();
            let mut session = json!({"unpublished": profile_id});
            let before_session = session.clone();
            let raw = host
                .external_root
                .join("nested/picked.py")
                .display()
                .to_string();
            let mut action = Action::Add(AddAction::PickedSourcePath(raw));
            let mut emitted = Effect::None;

            let error = host
                .capture_checkpoint_parts(
                    &mut state,
                    &mut session,
                    CheckpointCauseProjection::Reducer {
                        action: &mut action,
                        emitted: &mut emitted,
                    },
                )
                .unwrap_err();

            assert!(error.contains("has no active Add state"), "{error}");
            assert_eq!(state, before_state);
            assert_eq!(session, before_session);
            assert_eq!(emitted, Effect::None);
            assert_eq!(
                host.path_map.add_provenance,
                super::AddProjectionProvenance::default()
            );
            assert!(!host.adapters.pending_events().is_empty());
            let _ = host.observe(&state).unwrap();
            assert!(host.adapters.pending_events().is_empty());
            host.close().unwrap();
            assert!(!sandbox_root.exists());
        }
    }

    #[test]
    fn g2c_manual_draft_new_add_and_exit_keep_picker_provenance_disjoint() {
        let mut host = RealWalkerHost::spawn(picker_provenance_profile()).unwrap();
        let draft = write_draft_at(&host, "skit-new-picker.py", b"print('draft')\n", 10);
        let raw = host
            .external_root
            .join("nested/picked.py")
            .display()
            .to_string();
        let mut state = picker_add_state(&host);
        let mut session = picker_tui_session(&host);

        let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
        let picked_effect = state.update(picked.clone());
        let _ =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
        assert_eq!(
            host.path_map.add_provenance.picked_source_path,
            Some(raw.clone())
        );
        assert!(host.path_map.add_provenance.selected_source_path.is_none());

        let manual = Action::Add(AddAction::SetSourcePath(raw.clone()));
        let manual_effect = state.update(manual.clone());
        let (manual_checkpoint, recorded_manual, _, manual_session) =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, manual, manual_effect);
        assert_eq!(
            serde_json::to_value(recorded_manual).unwrap()["add"]["set_source_path"],
            raw
        );
        assert_eq!(
            manual_checkpoint
                .state
                .pointer("/workflow/active/add/source/path"),
            Some(&json!(raw))
        );
        assert_eq!(source_input_value(&manual_session), raw);
        assert!(host.path_map.add_provenance.picked_source_path.is_none());

        let lookalike = format!("{raw}-user-text");
        let manual = Action::Add(AddAction::SetSourcePath(lookalike.clone()));
        let manual_effect = state.update(manual.clone());
        let (lookalike_checkpoint, recorded_manual, _, lookalike_session) =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, manual, manual_effect);
        assert_eq!(
            serde_json::to_value(recorded_manual).unwrap()["add"]["set_source_path"],
            lookalike
        );
        assert_eq!(
            lookalike_checkpoint
                .state
                .pointer("/workflow/active/add/source/path"),
            Some(&json!(lookalike))
        );
        assert_eq!(source_input_value(&lookalike_session), lookalike);

        let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
        let picked_effect = state.update(picked.clone());
        let _ =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
        let stale = format!("{raw}-stale-without-cause");
        assert_eq!(
            state.update(Action::Add(AddAction::SetSourcePath(stale.clone()))),
            Effect::None
        );
        let stale_observation = host.observe(&state).unwrap();
        assert_eq!(
            stale_observation
                .state
                .pointer("/workflow/active/add/source/path"),
            Some(&json!(stale))
        );
        assert!(host.path_map.add_provenance.picked_source_path.is_none());

        let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
        let picked_effect = state.update(picked.clone());
        let _ =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
        let select = Action::Add(AddAction::SelectDraft(0));
        let select_effect = state.update(select.clone());
        let (selected, _, _, _) =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, select, select_effect);
        assert!(host.path_map.add_provenance.picked_source_path.is_none());
        assert_eq!(
            host.path_map.add_provenance.selected_source_path,
            Some(draft.clone())
        );
        assert_eq!(
            selected.state.pointer("/workflow/active/add/source/path"),
            Some(&json!("<profile:fixture>/data/.drafts/<draft:0>.py"))
        );

        let picked = Action::Add(AddAction::PickedSourcePath(raw.clone()));
        let picked_effect = state.update(picked.clone());
        let _ =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
        let new_add = Action::Present(skit_ui::Screen::Add(Box::new(AddWorkflowState::new(
            Vec::new(),
        ))));
        let new_add_effect = state.update(new_add.clone());
        let _ = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            new_add,
            new_add_effect,
        );
        assert_eq!(
            host.path_map.add_provenance,
            super::AddProjectionProvenance::default()
        );

        let picked = Action::Add(AddAction::PickedSourcePath(raw));
        let picked_effect = state.update(picked.clone());
        let _ =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, picked, picked_effect);
        let leave = Action::Present(skit_ui::Screen::Library);
        let leave_effect = state.update(leave.clone());
        let _ =
            capture_real_reducer_checkpoint(&mut host, &state, &mut session, leave, leave_effect);
        assert_eq!(
            host.path_map.add_provenance,
            super::AddProjectionProvenance::default()
        );
    }

    #[test]
    fn c2_real_add_session_requires_only_the_mirror_owned_by_its_stage() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(
            &host,
            "skit-new-stage-mirror.py",
            b"print('stage mirror')\n",
            10,
        );
        host.path_map.refresh(&host.service).unwrap();
        let mut state = host.initial_state().unwrap();
        assert_eq!(
            state.update(Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(super::sorted_tui_drafts(&host.roots().data)),
            )))),
            Effect::None
        );
        let mut tui_session = TuiSession::default();

        let select = Action::Add(AddAction::SelectDraft(0));
        let select_emitted = state.update(select.clone());
        let mut selected_state = state.clone();
        let mut selected_session = render_real_session_snapshot(&state, &mut tui_session);
        let mut recorded_select = select;
        let mut recorded_select_emitted = select_emitted;
        let selected = host
            .capture_checkpoint_parts(
                &mut selected_state,
                &mut selected_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_select,
                    emitted: &mut recorded_select_emitted,
                },
            )
            .unwrap();
        assert_eq!(
            selected_session.pointer("/add/fields/signature/stage"),
            Some(&json!("source"))
        );
        assert_eq!(
            selected_session.pointer("/add/fields/inputs/0/id"),
            Some(&json!("SourcePath"))
        );

        let continue_action = Action::Add(AddAction::Continue);
        let inspect = state.update(continue_action.clone());
        let mut inspecting_state = state.clone();
        let mut inspecting_session = render_real_session_snapshot(&state, &mut tui_session);
        let mut recorded_continue = continue_action;
        let mut recorded_inspect = inspect.clone();
        let inspecting = host
            .capture_checkpoint_parts(
                &mut inspecting_state,
                &mut inspecting_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_continue,
                    emitted: &mut recorded_inspect,
                },
            )
            .unwrap();
        super::super::tui_real_walker::validate_reducer_action(
            &selected.state,
            &inspecting.state,
            &serde_json::to_value(recorded_continue).unwrap(),
            &serde_json::to_value(recorded_inspect).unwrap(),
        )
        .unwrap();

        let response = host.dispatch(inspect.clone()).unwrap();
        let response_emitted = state.update(response.clone());
        let mut review_state = state.clone();
        let mut review_session = render_real_session_snapshot(&state, &mut tui_session);
        let review_inputs = review_session
            .pointer("/add/fields/inputs")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            review_session.pointer("/add/fields/signature/stage"),
            Some(&json!("review"))
        );
        assert!(
            review_inputs
                .iter()
                .any(|input| input["id"] == "ReviewName")
        );
        assert!(
            review_inputs
                .iter()
                .all(|input| input["id"] != "SourcePath")
        );
        let mut recorded_request = inspect;
        let mut recorded_response = response;
        let mut recorded_response_emitted = response_emitted;
        let review = host
            .capture_checkpoint_parts(
                &mut review_state,
                &mut review_session,
                CheckpointCauseProjection::Host {
                    request: &mut recorded_request,
                    response: &mut recorded_response,
                    emitted: &mut recorded_response_emitted,
                },
            )
            .unwrap();
        let stable = "<profile:fixture>/data/.drafts/<draft:0>.py";
        assert_eq!(
            review.state.pointer("/workflow/active/add/source/path"),
            Some(&json!(stable))
        );
        assert_eq!(
            review.state.pointer("/workflow/active/add/review/name"),
            Some(&json!("<draft:0>"))
        );
        let review_name = review_session
            .pointer("/add/fields/inputs")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|input| input["id"] == "ReviewName")
            .unwrap();
        assert_eq!(review_name["state"]["value"], "<draft:0>");
        super::super::tui_real_walker::validate_reducer_action(
            &inspecting.state,
            &review.state,
            &serde_json::to_value(recorded_response).unwrap(),
            &serde_json::to_value(recorded_response_emitted).unwrap(),
        )
        .unwrap();

        let save = Action::Add(AddAction::Save);
        let commit = state.update(save.clone());
        let mut saving_state = state.clone();
        let mut saving_session = render_real_session_snapshot(&state, &mut tui_session);
        let mut recorded_save = save;
        let mut recorded_commit = commit.clone();
        let saving = host
            .capture_checkpoint_parts(
                &mut saving_state,
                &mut saving_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_save,
                    emitted: &mut recorded_commit,
                },
            )
            .unwrap();
        let committed = host.dispatch(commit.clone()).unwrap();
        let completion_effects = state.update(committed.clone());
        let mut complete_state = state.clone();
        let mut complete_session = render_real_session_snapshot(&state, &mut tui_session);
        assert_eq!(
            complete_session.pointer("/add/fields/signature/stage"),
            Some(&json!("complete"))
        );
        assert_eq!(
            complete_session.pointer("/add/fields/inputs"),
            Some(&json!([]))
        );
        let mut recorded_commit_request = commit;
        let mut recorded_committed = committed;
        let mut recorded_completion_effects = completion_effects;
        let complete = host
            .capture_checkpoint_parts(
                &mut complete_state,
                &mut complete_session,
                CheckpointCauseProjection::Host {
                    request: &mut recorded_commit_request,
                    response: &mut recorded_committed,
                    emitted: &mut recorded_completion_effects,
                },
            )
            .unwrap();
        super::super::tui_real_walker::validate_reducer_action(
            &saving.state,
            &complete.state,
            &serde_json::to_value(recorded_committed).unwrap(),
            &serde_json::to_value(recorded_completion_effects).unwrap(),
        )
        .unwrap();
        assert_eq!(draft.file_name().unwrap(), "skit-new-stage-mirror.py");
    }

    #[test]
    fn c2_real_unicode_kind_and_review_names_project_and_replay() {
        assert_real_draft_transition_projects_and_replays(
            "skit-new-界.unknown",
            b"ambiguous unicode source\n",
            "kind",
            "/workflow/active/add/kind_picker/filename",
            "skit-new-界.unknown",
            "<profile:fixture>/data/.drafts/<draft:0>.unknown",
            "<draft:0>.unknown",
        );
        assert_real_draft_transition_projects_and_replays(
            "skit-new-界.py",
            b"print('unicode review')\n",
            "review",
            "/workflow/active/add/review/name",
            "skit-new-界",
            "<profile:fixture>/data/.drafts/<draft:0>.py",
            "<draft:0>",
        );
        assert_real_draft_transition_projects_and_replays(
            "skit-x.prompt.py",
            b"print('python with prompt in its stem')\n",
            "review",
            "/workflow/active/add/review/name",
            "skit-x.prompt",
            "<profile:fixture>/data/.drafts/<draft:0>.py",
            "<draft:0>",
        );
    }

    #[test]
    fn c2_real_prompt_suffixes_preserve_kind_and_reducer_replay() {
        for (name, stable_path) in [
            (
                "skit-new-multisuffix.prompt.md",
                "<profile:fixture>/data/.drafts/<draft:0>.prompt.md",
            ),
            (
                "skit-new-single.prompt",
                "<profile:fixture>/data/.drafts/<draft:0>.prompt",
            ),
        ] {
            let raw_name = name
                .strip_suffix(".prompt.md")
                .or_else(|| name.strip_suffix(".prompt"))
                .unwrap();
            assert_real_draft_transition_projects_and_replays(
                name,
                b"Hello {{name}}\n",
                "review",
                "/workflow/active/add/review/name",
                raw_name,
                stable_path,
                "<draft:0>",
            );
        }
    }

    #[test]
    fn c2_highlight_draft_keeps_the_derived_source_projection_and_replays() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_draft_at(&host, "skit-new-first.py", b"a first\n", 10);
        write_draft_at(&host, "skit-new-second.py", b"z second\n", 20);
        host.path_map.refresh(&host.service).unwrap();
        let mut state = host.initial_state().unwrap();
        assert_eq!(
            state.update(Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(super::sorted_tui_drafts(&host.roots().data)),
            )))),
            Effect::None
        );
        let mut session = TuiSession::default();
        let select = Action::Add(AddAction::SelectDraft(0));
        let select_emitted = state.update(select.clone());
        let (selected, _, _, _) = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            select,
            select_emitted,
        );
        let raw_path = serde_json::to_value(&state)
            .unwrap()
            .pointer("/workflow/active/add/source/path")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();
        let stable_path = selected
            .state
            .pointer("/workflow/active/add/source/path")
            .and_then(Value::as_str)
            .unwrap()
            .to_owned();
        assert_ne!(stable_path, raw_path);

        let highlight = Action::Add(AddAction::HighlightDraft(1));
        let highlight_emitted = state.update(highlight.clone());
        let raw = serde_json::to_value(&state).unwrap();
        assert_eq!(
            raw.pointer("/workflow/active/add/source/selected_draft"),
            Some(&json!(1))
        );
        assert_eq!(
            raw.pointer("/workflow/active/add/source/path"),
            Some(&json!(raw_path))
        );
        let (highlighted, recorded_highlight, recorded_emitted, highlighted_session) =
            capture_real_reducer_checkpoint(
                &mut host,
                &state,
                &mut session,
                highlight,
                highlight_emitted,
            );

        assert_eq!(
            highlighted
                .state
                .pointer("/workflow/active/add/source/path"),
            Some(&json!(stable_path))
        );
        let source_input = highlighted_session
            .pointer("/add/fields/inputs")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|input| input["id"] == "SourcePath")
            .unwrap();
        assert_eq!(source_input["state"]["value"], stable_path);
        super::super::tui_real_walker::validate_reducer_action(
            &selected.state,
            &highlighted.state,
            &serde_json::to_value(recorded_highlight).unwrap(),
            &serde_json::to_value(recorded_emitted).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn c2_source_edited_keeps_its_derived_review_name_across_source_changes() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let first = write_draft_at(&host, "skit-new-first.py", b"a first\n", 10);
        let second = write_draft_at(&host, "skit-new-second.py", b"z second\n", 20);
        host.path_map.refresh(&host.service).unwrap();
        let mut state = host.initial_state().unwrap();
        assert_eq!(
            state.update(Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(super::sorted_tui_drafts(&host.roots().data)),
            )))),
            Effect::None
        );
        let mut session = TuiSession::default();
        let first_stable_path = host.path_map.draft_paths[&first].stable_path.clone();
        let first_stable_name = super::review_default_name(Path::new(&first_stable_path), "python");
        let second_stable_path = host.path_map.draft_paths[&second].stable_path.clone();
        let select = Action::Add(AddAction::SelectDraft(1));
        let selected_effect = state.update(select.clone());
        let _ = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            select,
            selected_effect,
        );
        let continue_action = Action::Add(AddAction::Continue);
        let inspect = state.update(continue_action.clone());
        let _ = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            continue_action,
            inspect.clone(),
        );
        let inspected = host.dispatch(inspect.clone()).unwrap();
        let inspected_effect = state.update(inspected.clone());
        let (review, _, _, _) = capture_real_host_checkpoint(
            &mut host,
            &state,
            &mut session,
            inspect,
            inspected,
            inspected_effect,
        );
        assert_eq!(
            review.state.pointer("/workflow/active/add/review/name"),
            Some(&json!(first_stable_name))
        );

        let edit = Action::Add(AddAction::EditSource);
        let edit_request = state.update(edit.clone());
        let (editing, _, _, _) = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            edit,
            edit_request.clone(),
        );
        let first_source = serde_json::to_value(&state)
            .unwrap()
            .pointer("/workflow/active/add/review/source")
            .unwrap()
            .clone();
        let same_response = source_edited_response(&edit_request, first_source.clone());
        let same_emitted = state.update(same_response.clone());
        let (same, recorded_same, recorded_same_emitted, same_session) =
            capture_real_host_checkpoint(
                &mut host,
                &state,
                &mut session,
                edit_request,
                same_response,
                same_emitted,
            );
        assert_eq!(
            same.state.pointer("/workflow/active/add/review/name"),
            Some(&json!(first_stable_name))
        );
        assert!(
            same_session
                .pointer("/add/fields/inputs")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .any(|input| input["id"] == "ReviewName"
                    && input["state"]["value"] == first_stable_name)
        );
        super::super::tui_real_walker::validate_reducer_action(
            &editing.state,
            &same.state,
            &serde_json::to_value(recorded_same).unwrap(),
            &serde_json::to_value(recorded_same_emitted).unwrap(),
        )
        .unwrap();

        let edit = Action::Add(AddAction::EditSource);
        let edit_request = state.update(edit.clone());
        let (editing, _, _, _) = capture_real_reducer_checkpoint(
            &mut host,
            &state,
            &mut session,
            edit,
            edit_request.clone(),
        );
        let mut changed_source = first_source;
        changed_source["path"] = json!(second);
        changed_source["source_record"] = json!(second);
        changed_source["bytes"] = json!(b"z second\n");
        let changed_response = source_edited_response(&edit_request, changed_source);
        let changed_emitted = state.update(changed_response.clone());
        let raw = serde_json::to_value(&state).unwrap();
        assert_eq!(
            raw.pointer("/workflow/active/add/review/source/path"),
            Some(&json!(second))
        );
        assert_eq!(
            raw.pointer("/workflow/active/add/review/name"),
            Some(&json!("skit-new-first"))
        );
        let (changed, recorded_changed, recorded_changed_emitted, changed_session) =
            capture_real_host_checkpoint(
                &mut host,
                &state,
                &mut session,
                edit_request,
                changed_response,
                changed_emitted,
            );
        assert_eq!(
            changed
                .state
                .pointer("/workflow/active/add/review/source/path"),
            Some(&json!(second_stable_path))
        );
        assert_eq!(
            changed.state.pointer("/workflow/active/add/review/name"),
            Some(&json!(first_stable_name))
        );
        assert!(
            changed_session
                .pointer("/add/fields/inputs")
                .and_then(Value::as_array)
                .unwrap()
                .iter()
                .any(|input| input["id"] == "ReviewName"
                    && input["state"]["value"] == first_stable_name)
        );
        super::super::tui_real_walker::validate_reducer_action(
            &editing.state,
            &changed.state,
            &serde_json::to_value(recorded_changed).unwrap(),
            &serde_json::to_value(recorded_changed_emitted).unwrap(),
        )
        .unwrap();
        assert_eq!(first.file_name().unwrap(), "skit-new-first.py");
    }

    #[test]
    fn c2_library_state_and_every_screen_pointer_contract() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let root = host._sandbox.path().display().to_string();
        let stable = "<profile:fixture>";
        let message = format!("Host failure ({root}).");
        let projected_message = format!("Host failure ({stable}).");
        host.path_map.added_at.insert(
            "volatile-added-at".to_owned(),
            "<added-at:fixture>".to_owned(),
        );
        let mut value = json!({
            "entries": [{"slug": "entry", "kind": "python", "target": root}],
            "diagnostics": [{"message": message}],
            "details": {"entry": {
                "added_at": "volatile-added-at",
                "missing_target": root,
                "last_run": {"at": root},
                "parameters": [{"value": root}],
            }},
            "status": format!("prefix Installed the skit Agent Skill: {root}"),
            "workflow": {
                "active": {"run": {
                    "context": {
                        "path": {"workdir": root, "invoke_cwd": root},
                        "tokens": {"cwd": root, "home": root, "env": {"ROOT": root}},
                    },
                    "fields": [{
                        "control": {"text": {"value": root}},
                        "default": root,
                        "feedback": {"expanded": message},
                    }],
                    "initial_values": {"path": root},
                    "hidden_values": {"path": root},
                    "presets": {"saved": {"path": root}},
                }},
                "history": [
                    {"health": {
                        "snapshot": {
                            "library_path": root,
                            "uv": {"found": root},
                            "library_size": root,
                            "issues": [{"kind": {"launch_blocked": {"reason": message}}}],
                            "diagnostics": [message],
                        },
                        "rebuilt": {"problems": [message]},
                    }},
                    {"preferences": {
                        "agent_skill_install": {"targets": [{"base": root}]},
                    }},
                    {"runners": {
                        "status": message,
                        "overlay": {"editor": {"host_error": message}},
                    }},
                ],
            },
            "modal": {
                "run_file_picker": {"context": {"workdir": root, "invoke_cwd": root}},
                "run_token_menu": {"options": [{"fixed_directory": {"path": root}}]},
                "runner_editor": {"view": {"host_error": message}},
            },
        });

        value = host.path_map.normalize_library_json(value).unwrap();

        assert_eq!(value["entries"][0]["target"], stable);
        assert_eq!(value["diagnostics"][0]["message"], projected_message);
        assert_eq!(value["details"]["entry"]["added_at"], "<added-at:fixture>");
        assert_eq!(value["details"]["entry"]["missing_target"], stable);
        assert_eq!(value["details"]["entry"]["last_run"]["at"], root);
        assert_eq!(value["details"]["entry"]["parameters"][0]["value"], root);
        assert_eq!(
            value["status"],
            format!("prefix Installed the skit Agent Skill: {root}")
        );
        let run = &value["workflow"]["active"]["run"];
        for pointer in [
            "/context/path/workdir",
            "/context/path/invoke_cwd",
            "/context/tokens/cwd",
            "/context/tokens/home",
            "/context/tokens/env/ROOT",
        ] {
            assert_eq!(run.pointer(pointer), Some(&json!(stable)));
        }
        assert_eq!(run["fields"][0]["feedback"]["expanded"], projected_message);
        assert_eq!(run["fields"][0]["control"]["text"]["value"], root);
        assert_eq!(run["fields"][0]["default"], root);
        assert_eq!(run["initial_values"]["path"], root);
        assert_eq!(run["hidden_values"]["path"], root);
        assert_eq!(run["presets"]["saved"]["path"], root);
        let health = &value["workflow"]["history"][0]["health"];
        assert_eq!(health["snapshot"]["library_path"], stable);
        assert_eq!(health["snapshot"]["uv"]["found"], stable);
        assert_eq!(health["snapshot"]["library_size"], root);
        assert_eq!(
            health["snapshot"]["issues"][0]["kind"]["launch_blocked"]["reason"],
            projected_message
        );
        assert_eq!(health["snapshot"]["diagnostics"][0], projected_message);
        assert_eq!(health["rebuilt"]["problems"][0], projected_message);
        assert_eq!(
            value["workflow"]["history"][1]["preferences"]["agent_skill_install"]["targets"][0]["base"],
            stable
        );
        assert_eq!(
            value["workflow"]["history"][2]["runners"]["status"],
            message
        );
        assert_eq!(
            value["workflow"]["history"][2]["runners"]["overlay"]["editor"]["host_error"],
            projected_message
        );
        assert_eq!(
            value["modal"]["run_file_picker"]["context"]["workdir"],
            stable
        );
        assert_eq!(
            value["modal"]["run_file_picker"]["context"]["invoke_cwd"],
            stable
        );
        assert_eq!(
            value["modal"]["run_token_menu"]["options"][0]["fixed_directory"]["path"],
            stable
        );
        assert_eq!(
            value["modal"]["runner_editor"]["view"]["host_error"],
            projected_message
        );
    }

    #[test]
    fn c2_add_action_result_and_effect_pointer_contract() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft_path = write_draft_at(&host, "skit-new-add-matrix.py", b"print('matrix')\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        let draft = super::sorted_tui_drafts(&host.roots().data)
            .into_iter()
            .next()
            .unwrap();
        let source = SourceSnapshot {
            path: draft_path.clone(),
            source_record: draft_path.display().to_string(),
            bytes: b"print('matrix')\n".to_vec(),
            permissions: SourcePermissions {
                readonly: true,
                unix_mode: Some(0o640),
            },
            executable: Some(false),
            is_regular: true,
            is_directory: false,
            is_draft: true,
            identity: draft.identity.clone(),
        };
        let source_value = serde_json::to_value(&source).unwrap();
        let draft_value = serde_json::to_value(&draft).unwrap();
        let stable_path = "<profile:fixture>/data/.drafts/<draft:0>.py";
        let root = host._sandbox.path().display().to_string();
        let error = format!("Host error: {root}.");
        let stable_error = "Host error: <profile:fixture>.";

        for (tag, ok) in [
            ("source_inspected", json!({"Ok": source_value.clone()})),
            ("draft_edited", json!({"Ok": source_value.clone()})),
            ("source_edited", json!({"Ok": source_value.clone()})),
        ] {
            let action = host
                .path_map
                .normalize_action_json(json!({"add": {tag: {
                    "request": 0,
                    "result": ok,
                }}}))
                .unwrap();
            let source = &action["add"][tag]["result"]["Ok"];
            assert_eq!(source["path"], stable_path);
            assert_eq!(source["source_record"], stable_path);
            assert_eq!(source["permissions"]["unix_mode"], 0o640);
        }
        let changed = host
            .path_map
            .normalize_action_json(json!({"add": {"draft_deleted": {
                "request": 0,
                "result": {"Ok": {"changed": draft_value.clone()}},
            }}}))
            .unwrap();
        assert_eq!(
            changed["add"]["draft_deleted"]["result"]["Ok"]["changed"]["path"],
            stable_path
        );
        assert_eq!(
            changed["add"]["draft_deleted"]["result"]["Ok"]["changed"]["modified"],
            0
        );
        for stable_result in [
            json!({"draft_edited": {"request": 0, "result": {"Ok": null}}}),
            json!({"draft_deleted": {"request": 0, "result": {"Ok": "removed"}}}),
            json!({"draft_deleted": {"request": 0, "result": {"Ok": "already_missing"}}}),
            json!({"commit_finished": {"request": 0, "result": {"Ok": "created"}}}),
        ] {
            let expected = stable_result.clone();
            assert_eq!(
                host.path_map
                    .normalize_action_json(json!({"add": stable_result}))
                    .unwrap()["add"],
                expected
            );
        }

        for tag in [
            "source_inspected",
            "draft_edited",
            "draft_deleted",
            "source_edited",
            "commit_finished",
        ] {
            let action = host
                .path_map
                .normalize_action_json(json!({"add": {tag: {
                    "request": 0,
                    "result": {"Err": error},
                }}}))
                .unwrap();
            assert_eq!(action["add"][tag]["result"]["Err"], stable_error);
            let lookalike = format!("{root}-user");
            let action = host
                .path_map
                .normalize_action_json(json!({"add": {tag: {
                    "request": 0,
                    "result": {"Err": lookalike},
                }}}))
                .unwrap();
            assert_eq!(action["add"][tag]["result"]["Err"], lookalike);
        }

        let mut entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
        entry["source"] = json!(draft_path);
        let mut effect: Effect = super::deserialize_canonical(
            json!({"add": [
                {"inspect_source": {"request": 0, "path": draft_path}},
                {"delete_draft": {"request": 0, "draft": draft_value}},
                {"edit_source": {"request": 0, "path": draft_path}},
                {"commit": {"request": 0, "entry": entry, "source": source_value}},
                {"consume_draft": source},
                {"draft_kept": draft_path},
            ]}),
            "C2 Add Effect fixture",
        )
        .unwrap();
        host.path_map.project_typed_effect(&mut effect).unwrap();
        let effect = serde_json::to_value(effect).unwrap();
        assert_eq!(effect["add"][0]["inspect_source"]["path"], stable_path);
        assert_eq!(
            effect["add"][1]["delete_draft"]["draft"]["path"],
            stable_path
        );
        assert_eq!(effect["add"][2]["edit_source"]["path"], stable_path);
        assert_eq!(
            effect["add"][3]["commit"]["source"]["source_record"],
            stable_path
        );
        assert_eq!(effect["add"][3]["commit"]["entry"]["source"], stable_path);
        assert_eq!(
            effect["add"][3]["commit"]["source"]["permissions"]["unix_mode"],
            0o640
        );
        assert_eq!(effect["add"][4]["consume_draft"]["path"], stable_path);
        assert_eq!(effect["add"][5]["draft_kept"], stable_path);

        let mismatch = format!("{}-user", draft_path.display());
        let mut entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
        entry["source"] = json!(mismatch);
        let mut effect: Effect = super::deserialize_canonical(
            json!({"add": [{"commit": {
                "request": 0,
                "entry": entry,
                "source": serde_json::to_value(&source).unwrap(),
            }}]}),
            "C2 mismatched Commit fixture",
        )
        .unwrap();
        host.path_map.project_typed_effect(&mut effect).unwrap();
        assert_eq!(
            serde_json::to_value(effect).unwrap()["add"][0]["commit"]["entry"]["source"],
            mismatch
        );

        for raw in [
            json!({"count_run_glob": {
                "selector": "entry",
                "field": 0,
                "value": "*",
                "request": {"cwd": root, "pieces": ["*"]},
            }}),
            json!({"preferences": {"install_agent_skill": {"skills_dir": root}}}),
        ] {
            let mut effect: Effect =
                super::deserialize_canonical(raw, "C2 Effect fixture").unwrap();
            host.path_map.project_typed_effect(&mut effect).unwrap();
            let encoded = serde_json::to_string(&effect).unwrap();
            assert!(!encoded.contains(&root));
            assert!(encoded.contains("<profile:fixture>"));
        }
    }

    #[test]
    fn c2_agent_skill_status_provenance_replays_and_user_status_stays_byte_exact() {
        for locale in [Locale::En, Locale::ZhCn, Locale::ZhTw, Locale::Pseudo] {
            let mut host = RealWalkerHost::spawn(profile()).unwrap();
            let mut state = host.initial_state().unwrap();
            let open = host
                .dispatch(Effect::Open {
                    request: HostRequest::Preferences,
                    selector: None,
                })
                .unwrap();
            assert_eq!(state.update(open), Effect::None);
            let previous = host.observe(&state).unwrap().state;
            let raw = format_text(
                locale,
                "Installed the skit Agent Skill: {}",
                &[&host._sandbox.path().display()],
            );
            let stable = format_text(
                locale,
                "Installed the skit Agent Skill: {}",
                &[&"<profile:fixture>"],
            );
            let action =
                Action::Preferences(PreferencesAction::AgentSkillInstalled { message: raw });
            let emitted = state.update(action.clone());
            let mut projected_state = state.clone();
            let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
            let mut recorded_action = action;
            let mut recorded_emitted = emitted;
            let observation = host
                .capture_checkpoint_parts(
                    &mut projected_state,
                    &mut session,
                    CheckpointCauseProjection::Reducer {
                        action: &mut recorded_action,
                        emitted: &mut recorded_emitted,
                    },
                )
                .unwrap();
            assert_eq!(observation.state["status"], stable);
            assert_eq!(
                serde_json::to_value(&recorded_action).unwrap()["preferences"]["agent_skill_installed"]
                    ["message"],
                stable
            );
            super::super::tui_real_walker::validate_reducer_action(
                &previous,
                &observation.state,
                &serde_json::to_value(recorded_action).unwrap(),
                &serde_json::to_value(recorded_emitted).unwrap(),
            )
            .unwrap();
        }

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let mut state = host.initial_state().unwrap();
        let root = host._sandbox.path().display().to_string();
        let user_message = format!("User entry ({root}).");
        let action = Action::SetStatus(user_message.clone());
        let emitted = state.update(action.clone());
        let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let mut recorded_action = action;
        let mut recorded_emitted = emitted;
        let observation = host
            .capture_checkpoint_parts(
                &mut state,
                &mut session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_action,
                    emitted: &mut recorded_emitted,
                },
            )
            .unwrap();
        assert_eq!(observation.state["status"], user_message);
        assert_eq!(
            serde_json::to_value(recorded_action).unwrap()["set_status"],
            user_message
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn c2_session_native_paths_project_only_canonical_registered_objects() {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let registered = host
            ._sandbox
            .path()
            .join(std::ffi::OsString::from_vec(b"native-\xff".to_vec()));
        fs::write(&registered, b"native").unwrap();
        let registered_value = json!({"unix_bytes": registered.as_os_str().as_bytes()});
        let outside = PathBuf::from(std::ffi::OsString::from_vec(
            b"/outside/native-\xff".to_vec(),
        ));
        let outside_value = json!({"unix_bytes": outside.as_os_str().as_bytes()});
        let mut session = c2_session(host._sandbox.path(), None, Vec::new());
        session["preferences"]["fields"]["agent_signature"][0]["base"] = registered_value;
        session["file_picker_source"]["fields"]["files"][0] = outside_value.clone();

        host.path_map.project_session_value(&mut session).unwrap();

        assert_eq!(
            session["preferences"]["fields"]["agent_signature"][0]["base"],
            "<profile:fixture>/native-\\xff"
        );
        assert_eq!(
            session["file_picker_source"]["fields"]["files"][0],
            outside_value
        );
    }

    #[cfg(unix)]
    #[test]
    fn c2_session_native_unix_path_values_refuse_malformed_shapes() {
        let host = RealWalkerHost::spawn(profile()).unwrap();

        for malformed in [
            json!({"windows_wide": [65]}),
            json!({"other": []}),
            json!({"unix_bytes": "not-an-array"}),
            json!({"unix_bytes": [65], "extra": true}),
            json!({"unix_bytes": [256]}),
            json!({"unix_bytes": ["x"]}),
            json!({"unix_bytes": []}),
            json!({"unix_bytes": [97]}),
            json!(7),
        ] {
            let mut malformed = malformed;
            assert!(
                host.path_map
                    .normalize_session_path_value(&mut malformed)
                    .is_err()
            );
        }
    }

    #[test]
    fn c2_wide_unit_escape_is_injective() {
        assert_eq!(super::escaped_wide_units(&[0xd800]), "\\ud800");
        assert_eq!(super::escaped_wide_units(&[0xd801]), "\\ud801");
        assert_ne!(
            super::escaped_wide_units(&[0xd800]),
            super::escaped_wide_units(&[0xd801])
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn c2_real_non_utf_file_picker_entry_projects_its_lossy_name_and_native_path() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_non_utf_draft_at(&host);
        host.path_map.refresh(&host.service).unwrap();
        let mut session = real_file_picker_session(&host, &draft);
        let lossy = draft.file_name().unwrap().to_string_lossy();
        let raw_entry = session
            .pointer("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == lossy.as_ref())
            .unwrap();
        assert!(raw_entry["path"].get("unix_bytes").is_some());

        host.path_map.project_session_value(&mut session).unwrap();

        let projected_entry = session
            .pointer("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "<draft:0>.unknown")
            .unwrap();
        assert_eq!(
            projected_entry["path"],
            "<profile:fixture>/data/.drafts/<draft:0>.unknown"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn c2_real_non_utf_file_picker_entry_refuses_a_lossy_name_lookalike() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_non_utf_draft_at(&host);
        host.path_map.refresh(&host.service).unwrap();
        let mut session = real_file_picker_session(&host, &draft);
        let lossy = draft.file_name().unwrap().to_string_lossy();
        let entry = session
            .pointer_mut("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array_mut)
            .unwrap()
            .iter_mut()
            .find(|entry| entry["name"] == lossy.as_ref())
            .unwrap();
        entry["name"] = json!(format!("{lossy}-lookalike"));

        assert!(host.path_map.project_session_value(&mut session).is_err());
    }

    #[test]
    fn c2_real_unicode_file_picker_entry_projects_and_refuses_a_name_mismatch() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_unicode_draft_at(&host, "skit-new-界.py", b"print('unicode picker')\n");
        host.path_map.refresh(&host.service).unwrap();
        let mut session = real_file_picker_session(&host, &draft);
        let raw_entry = session
            .pointer("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "skit-new-界.py")
            .unwrap();
        assert_eq!(raw_entry["path"], draft.display().to_string());

        host.path_map.project_session_value(&mut session).unwrap();

        let projected_entry = session
            .pointer("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "<draft:0>.py")
            .unwrap();
        assert_eq!(
            projected_entry["path"],
            "<profile:fixture>/data/.drafts/<draft:0>.py"
        );

        let mut mismatch = real_file_picker_session(&host, &draft);
        let entry = mismatch
            .pointer_mut("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array_mut)
            .unwrap()
            .iter_mut()
            .find(|entry| entry["name"] == "skit-new-界.py")
            .unwrap();
        entry["name"] = json!("skit-new-\\xe7\\x95\\x8c.py");
        assert!(host.path_map.project_session_value(&mut mismatch).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn c2_non_utf_add_kind_and_review_stop_at_the_typed_serde_boundary() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_non_utf_draft_at(&host);
        host.path_map.refresh(&host.service).unwrap();
        let mut workflow = AddWorkflowState::new(Vec::new());
        assert!(
            workflow
                .reduce(AddAction::SetSourcePath("placeholder".to_owned()))
                .is_empty()
        );
        let mut effects = workflow.reduce(AddAction::Continue);
        assert!(matches!(
            effects.as_slice(),
            [skit_ui::AddEffect::InspectSource { .. }]
        ));
        if let skit_ui::AddEffect::InspectSource { path, .. } = &mut effects[0] {
            *path = draft;
        }
        let response = host.dispatch(Effect::Add(effects)).unwrap();
        assert!(matches!(
            &response,
            Action::Add(AddAction::SourceInspected { result: Ok(_), .. })
        ));
        let action = expect_add_action(response.clone());
        assert!(workflow.reduce(action).is_empty());
        assert_eq!(workflow.kind_picker().unwrap().filename(), "");

        let mut projected = response;
        let error = host
            .path_map
            .project_typed_action(&mut projected)
            .unwrap_err();
        assert!(error.contains("path contains invalid UTF-8 characters"));

        assert!(
            workflow
                .reduce(AddAction::PickKind(Some(KnownEntryKind::Python)))
                .is_empty()
        );
        assert_eq!(workflow.review().unwrap().name(), "entry");
        assert!(serde_json::to_value(workflow).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn c2_windows_session_native_paths_project_only_canonical_registered_objects() {
        use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let registered = host
            ._sandbox
            .path()
            .join(std::ffi::OsString::from_wide(&[0xd800]));
        let registered_value = json!({
            "windows_wide": registered.as_os_str().encode_wide().collect::<Vec<_>>(),
        });
        let outside = PathBuf::from(std::ffi::OsString::from_wide(&[0xd801]));
        let outside_value = json!({
            "windows_wide": outside.as_os_str().encode_wide().collect::<Vec<_>>(),
        });
        let mut session = c2_session(host._sandbox.path(), None, Vec::new());
        session["preferences"]["fields"]["agent_signature"][0]["base"] = registered_value;
        session["file_picker_source"]["fields"]["files"][0] = outside_value.clone();

        host.path_map.project_session_value(&mut session).unwrap();

        assert_eq!(
            session["preferences"]["fields"]["agent_signature"][0]["base"],
            format!(
                "<profile:fixture>/{}",
                super::escaped_os(registered.file_name().unwrap())
            )
        );
        assert_eq!(
            session["file_picker_source"]["fields"]["files"][0],
            outside_value
        );
        let sibling = host
            ._sandbox
            .path()
            .join(std::ffi::OsString::from_wide(&[0xd801]));
        let mut first = json!({
            "windows_wide": registered.as_os_str().encode_wide().collect::<Vec<_>>(),
        });
        let mut second = json!({
            "windows_wide": sibling.as_os_str().encode_wide().collect::<Vec<_>>(),
        });
        host.path_map
            .normalize_session_path_value(&mut first)
            .unwrap();
        host.path_map
            .normalize_session_path_value(&mut second)
            .unwrap();
        assert_ne!(first, second);
        for malformed in [
            json!({"unix_bytes": [65]}),
            json!({"other": []}),
            json!({"windows_wide": "not-an-array"}),
            json!({"windows_wide": [65], "extra": true}),
            json!({"windows_wide": [65536]}),
            json!({"windows_wide": ["x"]}),
            json!({"windows_wide": []}),
            json!({"windows_wide": [65]}),
            json!(7),
        ] {
            let mut malformed = malformed;
            assert!(
                host.path_map
                    .normalize_session_path_value(&mut malformed)
                    .is_err()
            );
        }
    }

    #[test]
    fn c2_session_schema_node_and_nested_picker_corruption_refusals() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let root = Path::new("/unregistered/session");
        let valid = c2_session(root, None, Vec::new());

        for mut invalid in [
            Value::Null,
            json!({}),
            json!({"schema_version": 2}),
            json!({"schema_version": "1"}),
        ] {
            assert!(host.path_map.project_session_value(&mut invalid).is_err());
        }
        for owner in [
            "preferences",
            "add",
            "path_suggestions",
            "run_modal",
            "add_overlay",
            "file_picker_source",
        ] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(owner);
            assert!(host.path_map.project_session_value(&mut invalid).is_err());
        }
        for (pointer, replacement) in [
            ("/preferences/kind", Value::Null),
            ("/preferences/kind", json!("wrong")),
            ("/preferences/fields", Value::Null),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(host.path_map.project_session_value(&mut invalid).is_err());
        }
        let mut missing_kind = valid.clone();
        missing_kind["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("kind");
        assert!(
            host.path_map
                .project_session_value(&mut missing_kind)
                .is_err()
        );
        let mut missing_fields = valid.clone();
        missing_fields["preferences"]
            .as_object_mut()
            .unwrap()
            .remove("fields");
        assert!(
            host.path_map
                .project_session_value(&mut missing_fields)
                .is_err()
        );
        let mut extra = valid.clone();
        extra["preferences"]["extra"] = json!(true);
        assert!(host.path_map.project_session_value(&mut extra).is_err());

        let mut bad_overlay = valid.clone();
        bad_overlay["add_overlay"]["kind"] = json!("unknown_overlay");
        assert!(
            host.path_map
                .project_session_value(&mut bad_overlay)
                .is_err()
        );
        let mut scalar_overlay = valid.clone();
        scalar_overlay["add_overlay"] = json!(7);
        assert!(
            host.path_map
                .project_session_value(&mut scalar_overlay)
                .is_err()
        );
        let mut missing_overlay_session = valid.clone();
        missing_overlay_session["add_overlay"]["fields"] = json!({});
        assert!(
            host.path_map
                .project_session_value(&mut missing_overlay_session)
                .is_err()
        );
        let mut wrong_picker = valid.clone();
        wrong_picker["run_modal"]["fields"]["file"]["kind"] = json!("ordinary");
        assert!(
            host.path_map
                .project_session_value(&mut wrong_picker)
                .is_err()
        );
        let mut missing_entries = valid.clone();
        missing_entries["run_modal"]["fields"]["file"]["fields"]["explorer"] = json!({});
        assert!(
            host.path_map
                .project_session_value(&mut missing_entries)
                .is_err()
        );
        let mut missing_memory = valid.clone();
        missing_memory["run_modal"]["fields"]["file"]["fields"]
            .as_object_mut()
            .unwrap()
            .remove("memory_source");
        assert!(
            host.path_map
                .project_session_value(&mut missing_memory)
                .is_err()
        );
        let mut wrong_memory = valid.clone();
        wrong_memory["file_picker_source"]["kind"] = json!("file_picker");
        assert!(
            host.path_map
                .project_session_value(&mut wrong_memory)
                .is_err()
        );

        let mut null_memory = valid.clone();
        null_memory["run_modal"]["fields"]["file"]["fields"]["memory_source"] = Value::Null;
        assert!(
            host.path_map
                .project_session_value(&mut null_memory)
                .is_err()
        );
        let mut null_owners = valid.clone();
        null_owners["add"]["fields"]["picker_root"] = Value::Null;
        null_owners["run_modal"]["fields"]["file"] = Value::Null;
        null_owners["run_modal"]["fields"]["file_picker_source"] = Value::Null;
        null_owners["add_overlay"] = Value::Null;
        null_owners["file_picker_source"] = Value::Null;
        host.path_map
            .project_session_value(&mut null_owners)
            .unwrap();

        let mut prompt_overlay = valid.clone();
        prompt_overlay["add_overlay"] = json!({
            "kind": "add_prompt_overlay",
            "fields": {
                "session": {"kind": "prompt_candidate_picker", "fields": {}},
                "geometry": null,
            },
        });
        host.path_map
            .project_session_value(&mut prompt_overlay)
            .unwrap();

        for event in [
            json!("open_prompt_candidates"),
            json!("open_runner_editor"),
            json!("changed"),
            json!({"action": "continue"}),
        ] {
            let mut session = valid.clone();
            session["add"]["fields"]["advertised"][0]["event"] = event;
            host.path_map.project_session_value(&mut session).unwrap();
        }
        let mut absolute = valid.clone();
        absolute["add"]["fields"]["advertised"][0]["event"]["open_path_picker"]["output_policy"] =
            json!("absolute");
        absolute["run_modal"]["fields"]["file"]["fields"]["contract"]["output_policy"] =
            json!("absolute");
        host.path_map.project_session_value(&mut absolute).unwrap();
        for signature in [
            Value::Null,
            json!("preset"),
            json!("other"),
            json!({"environment": {"field": 0, "names": []}}),
        ] {
            let mut session = valid.clone();
            session["run_modal"]["fields"]["signature"] = signature;
            host.path_map.project_session_value(&mut session).unwrap();
        }
        let mut nullable = valid.clone();
        nullable["path_suggestions"]["fields"]["expected"]["request"]["context"]["tokens"]["home"] =
            Value::Null;
        nullable["run_modal"]["fields"]["file"]["fields"]["io_error"] = Value::Null;
        let symlink = nullable["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry.pointer("/entry_type/symlink").is_some())
            .unwrap();
        symlink["entry_type"]["symlink"]["target"] = Value::Null;
        host.path_map.project_session_value(&mut nullable).unwrap();

        for (pointer, replacement) in [
            ("/preferences/fields/agent_signature", json!(7)),
            ("/preferences/fields/agent_signature/0/base", Value::Null),
            ("/add/fields/picker_root", json!(7)),
            ("/add/fields/advertised", json!({})),
            ("/add/fields/advertised/0/event", json!(7)),
            ("/add/fields/advertised/0/event", json!("unknown")),
            ("/add/fields/advertised/0/event", json!({"unknown": {}})),
            (
                "/add/fields/advertised/0/event",
                json!({"action": "continue", "open_path_picker": {}}),
            ),
            (
                "/add/fields/advertised/0/event/open_path_picker/start_dir",
                Value::Null,
            ),
            (
                "/add/fields/advertised/0/event/open_path_picker/output_policy",
                json!("unknown"),
            ),
            (
                "/add/fields/advertised/0/event/open_path_picker/output_policy",
                json!({"unknown": "/path"}),
            ),
            ("/path_suggestions/fields/expected", json!([])),
            (
                "/path_suggestions/fields/expected/request/context/workdir",
                Value::Null,
            ),
            (
                "/path_suggestions/fields/expected/request/context/tokens",
                json!([]),
            ),
            (
                "/path_suggestions/fields/expected/request/context/tokens/env",
                json!([]),
            ),
            ("/path_suggestions/fields/visible/suggestion", Value::Null),
            ("/run_modal/fields/signature", json!(7)),
            ("/run_modal/fields/signature", json!("unknown")),
            ("/run_modal/fields/signature", json!({"unknown": {}})),
            (
                "/run_modal/fields/signature",
                json!({"file": {}, "token": {}}),
            ),
            (
                "/run_modal/fields/signature/file/context/workdir",
                Value::Null,
            ),
            (
                "/run_modal/fields/file/fields/contract/start_dir",
                Value::Null,
            ),
            (
                "/run_modal/fields/file/fields/contract/output_policy",
                json!([]),
            ),
            (
                "/run_modal/fields/file/fields/contract/output_policy",
                json!({"relative_to": "/path", "extra": true}),
            ),
            (
                "/run_modal/fields/file/fields/explorer/current_dir",
                Value::Null,
            ),
            ("/run_modal/fields/file/fields/explorer/entries", json!({})),
            (
                "/run_modal/fields/file/fields/explorer/entries/0/path",
                Value::Null,
            ),
            (
                "/run_modal/fields/file/fields/explorer/selected_files",
                json!({}),
            ),
            ("/run_modal/fields/file/fields/io_error", json!(7)),
            (
                "/run_modal/fields/file/fields/memory_source/fields/root",
                Value::Null,
            ),
            (
                "/run_modal/fields/file/fields/memory_source/fields/directories",
                json!({}),
            ),
            (
                "/run_modal/fields/file/fields/memory_source/fields/files",
                json!({}),
            ),
        ] {
            let mut session = valid.clone();
            *session.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                host.path_map.project_session_value(&mut session).is_err(),
                "accepted corrupt session pointer {pointer}"
            );
        }

        let mut ordinary = valid;
        ordinary["add"]["fields"]["ordinary"] = json!({
            "kind": "file_picker",
            "fields": {"path": host._sandbox.path()},
        });
        let expected = ordinary["add"]["fields"]["ordinary"].clone();
        host.path_map.project_session_value(&mut ordinary).unwrap();
        assert_eq!(ordinary["add"]["fields"]["ordinary"], expected);

        let draft = write_draft_at(&host, "skit-new-picker-name.py", b"print(1)\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        let mut mismatch = c2_session(host._sandbox.path(), Some(&draft), Vec::new());
        let entry = mismatch["run_modal"]["fields"]["file"]["fields"]["explorer"]["entries"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["path"] == json!(draft))
            .unwrap();
        entry["name"] = json!("wrong.py");
        assert!(host.path_map.project_session_value(&mut mismatch).is_err());
    }

    #[test]
    fn c2_action_surface_run_health_preferences_and_runner_owner_contract() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let root = host._sandbox.path().display().to_string();
        let stable = "<profile:fixture>";
        let diagnostic = format!("Diagnostic ({root}).");
        let stable_diagnostic = format!("Diagnostic ({stable}).");
        let mut reload = serde_json::to_value(host.dispatch(Effect::Reload).unwrap()).unwrap();
        let surface = &mut reload["replace_surface"]["surface"];
        surface["scan"]["entries"][0]["target"] = json!(root);
        surface["scan"]["diagnostics"] = json!([{
            "code": "io",
            "message": diagnostic,
        }]);
        let detail_key = surface["details"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        surface["details"][&detail_key]["missing_target"] = json!(root);
        surface["details"][&detail_key]["parameters"] = json!([{
            "key": "user",
            "value": root,
            "secret": false,
        }]);
        let surface = surface.clone();
        let scan = surface["scan"].clone();
        let user_message = format!("Created user entry ({root}).");
        let fixtures = [
            (
                json!({"replace": {"scan": scan, "rerunnable": []}}),
                false,
                false,
            ),
            (
                json!({"replace_surface": {"surface": surface, "rerunnable": []}}),
                false,
                true,
            ),
            (
                json!({"complete": {
                    "surface": surface,
                    "rerunnable": [],
                    "message": user_message,
                }}),
                true,
                true,
            ),
            (
                json!({"add_completed": {
                    "surface": surface,
                    "rerunnable": [],
                    "slug": "command",
                    "message": user_message,
                }}),
                true,
                true,
            ),
        ];
        for (fixture, has_user_message, has_details) in fixtures {
            let action = host.path_map.normalize_action_json(fixture).unwrap();
            let encoded = serde_json::to_string(&action).unwrap();
            assert!(encoded.contains(stable));
            assert!(encoded.contains(&stable_diagnostic));
            let encoded_message = serde_json::to_string(&user_message).unwrap();
            assert_eq!(
                encoded.contains(&encoded_message[1..encoded_message.len() - 1]),
                has_user_message
            );
            assert_eq!(
                encoded.contains(&format!(
                    "\"value\":{}",
                    serde_json::to_string(&root).unwrap()
                )),
                has_details
            );
        }

        let mut run = serde_json::to_value(
            host.dispatch(Effect::Open {
                request: HostRequest::Run,
                selector: Some("Command".to_owned()),
            })
            .unwrap(),
        )
        .unwrap()["present"]["run"]
            .clone();
        for pointer in [
            "/context/path/workdir",
            "/context/path/invoke_cwd",
            "/context/tokens/cwd",
            "/context/tokens/home",
        ] {
            *run.pointer_mut(pointer).unwrap() = json!(root);
        }
        run["context"]["tokens"]["env"] = json!({"ROOT": root});
        let text_index = run["fields"]
            .as_array()
            .unwrap()
            .iter()
            .position(|field| field.pointer("/control/text").is_some())
            .unwrap();
        run["fields"][text_index]["feedback"]["expanded"] = json!(diagnostic);
        run["fields"][text_index]["control"]["text"]["value"] = json!(root);
        run["fields"][text_index]["default"] = json!(root);
        run["initial_values"] = json!({"user": root});
        run["hidden_values"] = json!({"user": root});
        run["presets"] = json!({"saved": {"user": root}});
        let action = host
            .path_map
            .normalize_action_json(json!({"prompt_runner_required": {
                "form": run,
                "cancel_status": user_message,
            }}))
            .unwrap();
        let form = &action["prompt_runner_required"]["form"];
        assert_eq!(form["context"]["path"]["workdir"], stable);
        assert_eq!(form["context"]["tokens"]["env"]["ROOT"], stable);
        assert_eq!(
            form["fields"][text_index]["feedback"]["expanded"],
            stable_diagnostic
        );
        assert_eq!(form["fields"][text_index]["control"]["text"]["value"], root);
        assert_eq!(form["fields"][text_index]["default"], root);
        assert_eq!(form["initial_values"]["user"], root);
        assert_eq!(form["hidden_values"]["user"], root);
        assert_eq!(form["presets"]["saved"]["user"], root);
        assert_eq!(
            action["prompt_runner_required"]["cancel_status"],
            user_message
        );

        let snapshot = json!({
            "uv": {"found": root},
            "entry_count": 1,
            "issues": [{
                "slug": "command",
                "name": "Command",
                "kind": {"launch_blocked": {"reason": diagnostic}},
            }],
            "invalid_runner_rows": [],
            "mirror": "off",
            "library_path": root,
            "library_size": root,
            "diagnostics": [diagnostic],
        });
        let action = host
            .path_map
            .normalize_action_json(json!({"health": {"rebuilt": {
                "snapshot": snapshot,
                "outcome": {"entry_count": 1, "problems": [diagnostic]},
            }}}))
            .unwrap();
        let rebuilt = &action["health"]["rebuilt"];
        assert_eq!(rebuilt["snapshot"]["library_path"], stable);
        assert_eq!(rebuilt["snapshot"]["uv"]["found"], stable);
        assert_eq!(rebuilt["snapshot"]["library_size"], root);
        assert_eq!(
            rebuilt["snapshot"]["issues"][0]["kind"]["launch_blocked"]["reason"],
            stable_diagnostic
        );
        assert_eq!(rebuilt["snapshot"]["diagnostics"][0], stable_diagnostic);
        assert_eq!(rebuilt["outcome"]["problems"][0], stable_diagnostic);

        let targets = host
            .path_map
            .normalize_action_json(json!({"preferences": {
                "present_agent_skill_targets": [{
                    "name": "codex",
                    "scope": "user",
                    "base": root,
                }],
            }}))
            .unwrap();
        assert_eq!(
            targets["preferences"]["present_agent_skill_targets"][0]["base"],
            stable
        );
        let mutation = host
            .path_map
            .normalize_action_json(json!({"runners": {
                "mutation_failed": diagnostic,
            }}))
            .unwrap();
        assert_eq!(mutation["runners"]["mutation_failed"], stable_diagnostic);
        let editor = host
            .path_map
            .normalize_action_json(json!({"runner_editor_save_failed": {
                "owner": "add",
                "message": diagnostic,
            }}))
            .unwrap();
        assert_eq!(
            editor["runner_editor_save_failed"]["message"],
            stable_diagnostic
        );

        let mut preferences = serde_json::to_value(
            host.dispatch(Effect::Open {
                request: HostRequest::Preferences,
                selector: None,
            })
            .unwrap(),
        )
        .unwrap()["present"]["preferences"]
            .clone();
        preferences["agent_skill_install"] = json!({
            "targets": [{"name": "codex", "scope": "user", "base": root}],
            "selected": 0,
        });
        for fixture in [
            json!({"present": {"preferences": preferences}}),
            json!({"runner_manager_closed": {"preferences": preferences}}),
        ] {
            let action = host.path_map.normalize_action_json(fixture).unwrap();
            let encoded = serde_json::to_string(&action).unwrap();
            assert!(!encoded.contains(&root));
            assert!(encoded.contains(stable));
        }

        let mut runners = serde_json::to_value(
            host.dispatch(Effect::Open {
                request: HostRequest::Runners,
                selector: None,
            })
            .unwrap(),
        )
        .unwrap()["present"]["runners"]
            .clone();
        runners["status"] = json!(diagnostic);
        runners["overlay"] = json!({"editor": {
            "name": "",
            "command": root,
            "target": "new",
            "focused": "name",
            "error": null,
            "host_error": diagnostic,
        }});
        let action = host
            .path_map
            .normalize_action_json(json!({"present": {"runners": runners}}))
            .unwrap();
        assert_eq!(action["present"]["runners"]["status"], diagnostic);
        assert_eq!(
            action["present"]["runners"]["overlay"]["editor"]["host_error"],
            stable_diagnostic
        );
        assert_eq!(
            action["present"]["runners"]["overlay"]["editor"]["command"],
            root
        );
    }

    #[test]
    fn c2_review_name_and_kind_filename_require_draft_provenance() {
        assert_eq!(
            serde_json::to_value(skit_tui::AddTextField::SourcePath).unwrap(),
            "SourcePath"
        );
        assert_eq!(
            serde_json::to_value(skit_tui::AddTextField::ReviewName).unwrap(),
            "ReviewName"
        );

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft_path = write_draft_at(&host, "skit-new-review-name.py", b"print('review')\n", 10);
        let raw_name = "skit-new-review-name";
        let mut state = host.initial_state().unwrap();
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::SelectDraft(0))),
            Effect::None
        );
        let request = state.update(Action::Add(AddAction::Continue));
        let response = host.dispatch(request.clone()).unwrap();
        let response_value = serde_json::to_value(&response).unwrap();
        let source: SourceSnapshot = serde_json::from_value(
            response_value["add"]["source_inspected"]["result"]["Ok"].clone(),
        )
        .unwrap();
        let emitted = state.update(response.clone());
        let mut projected_state = state.clone();
        let mut session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "ReviewName",
                "state": {
                    "value": raw_name,
                    "cursor": raw_name.chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        let mut recorded_request = request;
        let mut recorded_response = response;
        let mut recorded_emitted = emitted;
        host.capture_checkpoint_parts(
            &mut projected_state,
            &mut session,
            CheckpointCauseProjection::Host {
                request: &mut recorded_request,
                response: &mut recorded_response,
                emitted: &mut recorded_emitted,
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&projected_state).unwrap()["workflow"]["active"]["add"]["review"]
                ["name"],
            "<draft:0>"
        );
        assert_eq!(
            session["add"]["fields"]["inputs"][0]["state"]["value"],
            "<draft:0>"
        );

        let projected_before_save = serde_json::to_value(&projected_state).unwrap();
        let mut save_state = state.clone();
        let mut save_action = Action::Add(AddAction::Save);
        let mut save_effect = save_state.update(save_action.clone());
        let mut save_session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "ReviewName",
                "state": {
                    "value": raw_name,
                    "cursor": raw_name.chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        let save_observation = host
            .capture_checkpoint_parts(
                &mut save_state,
                &mut save_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut save_action,
                    emitted: &mut save_effect,
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&save_effect).unwrap()["add"][0]["commit"]["entry"]["name"],
            "<draft:0>"
        );
        super::super::tui_real_walker::validate_reducer_action(
            &projected_before_save,
            &save_observation.state,
            &serde_json::to_value(save_action).unwrap(),
            &serde_json::to_value(save_effect).unwrap(),
        )
        .unwrap();

        let action = Action::Add(AddAction::SetReviewName(raw_name.to_owned()));
        let emitted = state.update(action.clone());
        let mut user_state = state.clone();
        let mut user_session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "ReviewName",
                "state": {
                    "value": raw_name,
                    "cursor": raw_name.chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        let mut recorded_action = action;
        let mut recorded_emitted = emitted;
        let user_observation = host
            .capture_checkpoint_parts(
                &mut user_state,
                &mut user_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_action,
                    emitted: &mut recorded_emitted,
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(user_state).unwrap()["workflow"]["active"]["add"]["review"]["name"],
            raw_name
        );
        assert_eq!(
            user_session["add"]["fields"]["inputs"][0]["state"]["value"],
            raw_name
        );

        let mut manual_save_state = state.clone();
        let mut manual_save_action = Action::Add(AddAction::Save);
        let mut manual_save_effect = manual_save_state.update(manual_save_action.clone());
        let mut manual_save_session = user_session.clone();
        let manual_save_observation = host
            .capture_checkpoint_parts(
                &mut manual_save_state,
                &mut manual_save_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut manual_save_action,
                    emitted: &mut manual_save_effect,
                },
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(&manual_save_effect).unwrap()["add"][0]["commit"]["entry"]["name"],
            raw_name
        );
        super::super::tui_real_walker::validate_reducer_action(
            &user_observation.state,
            &manual_save_observation.state,
            &serde_json::to_value(manual_save_action).unwrap(),
            &serde_json::to_value(manual_save_effect).unwrap(),
        )
        .unwrap();

        let mut default_host = RealWalkerHost::spawn(profile()).unwrap();
        let default_path = write_draft_at(
            &default_host,
            "skit-new-review-name.py",
            b"print('review default')\n",
            10,
        );
        let drafts = super::sorted_tui_drafts(&default_host.roots().data);
        let workflow = AddWorkflowState::new(drafts).with_review_defaults(ReviewDefaults {
            name: Some(raw_name.to_owned()),
            ..ReviewDefaults::default()
        });
        let mut default_state = default_host.initial_state().unwrap();
        assert_eq!(
            default_state.update(Action::Present(skit_ui::Screen::Add(Box::new(workflow)))),
            Effect::None
        );
        assert_eq!(
            default_state.update(Action::Add(AddAction::SelectDraft(0))),
            Effect::None
        );
        let mut default_request = default_state.update(Action::Add(AddAction::Continue));
        let mut default_response = default_host.dispatch(default_request.clone()).unwrap();
        let mut default_emitted = default_state.update(default_response.clone());
        let mut session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "ReviewName",
                "state": {
                    "value": raw_name,
                    "cursor": raw_name.chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        let observation = default_host
            .capture_checkpoint_parts(
                &mut default_state,
                &mut session,
                CheckpointCauseProjection::Host {
                    request: &mut default_request,
                    response: &mut default_response,
                    emitted: &mut default_emitted,
                },
            )
            .unwrap();
        assert_eq!(
            observation.state["workflow"]["active"]["add"]["review"]["name"],
            raw_name
        );
        assert_eq!(
            session["add"]["fields"]["inputs"][0]["state"]["value"],
            raw_name
        );
        assert_eq!(default_path.file_stem().unwrap(), raw_name);

        let source_value = serde_json::to_value(source).unwrap();
        let mut kind = json!({"add": {
            "pending_source": source_value,
            "kind_picker": {"filename": "skit-new-review-name.py"},
        }});
        host.path_map.normalize_add_screen(&mut kind).unwrap();
        assert_eq!(kind["add"]["kind_picker"]["filename"], "<draft:0>.py");
        let mut corrupt = json!({"add": {
            "pending_source": serde_json::to_value(SourceSnapshot {
                path: draft_path.clone(),
                source_record: draft_path.display().to_string(),
                bytes: Vec::new(),
                permissions: SourcePermissions::default(),
                executable: None,
                is_regular: true,
                is_directory: false,
                is_draft: true,
                identity: None,
            }).unwrap(),
            "kind_picker": {"filename": "wrong.py"},
        }});
        assert!(host.path_map.normalize_add_screen(&mut corrupt).is_err());
    }

    #[test]
    fn c2_commit_review_name_requires_exact_live_draft_provenance() {
        fn commit_value(path: &Path, name: &str) -> Value {
            let path = path.display().to_string();
            let source = json!({
                "path": path,
                "source_record": path,
                "bytes": [1, 2, 3],
                "permissions": {"readonly": false, "unix_mode": 384},
                "executable": false,
                "is_regular": true,
                "is_directory": false,
                "is_draft": true,
                "identity": null,
            });
            let mut entry = serde_json::to_value(profile().entries[1].clone()).unwrap();
            entry["name"] = json!(name);
            entry["source"] = json!(path);
            json!({"add": [{"commit": {
                "request": 0,
                "entry": entry,
                "source": source,
            }}]})
        }

        fn project(host: &mut RealWalkerHost, value: Value) -> Result<Value, String> {
            let mut effect: Effect =
                super::deserialize_canonical(value, "C2 derived Commit fixture")?;
            host.path_map.project_typed_effect(&mut effect)?;
            serde_json::to_value(effect).map_err(|error| error.to_string())
        }

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let first = write_draft_at(&host, "skit-new-first.py", b"first\n", 10);
        let second = write_draft_at(&host, "skit-new-second.py", b"second\n", 20);
        host.path_map.refresh(&host.service).unwrap();
        host.path_map.add_provenance.review_source_path = Some(first.clone());
        set_review_name_projection(&mut host, &first, "skit-new-first", "<draft:0>");

        let valid = commit_value(&first, "skit-new-first");
        assert_eq!(
            project(&mut host, valid.clone()).unwrap()["add"][0]["commit"]["entry"]["name"],
            "<draft:0>"
        );

        host.path_map.add_provenance.review_source_path = None;
        assert!(project(&mut host, valid.clone()).is_err());
        host.path_map.add_provenance.review_source_path = Some(first.clone());
        host.path_map
            .add_provenance
            .review_name
            .as_mut()
            .unwrap()
            .stable_name = "wrong".to_owned();
        assert!(project(&mut host, valid.clone()).is_err());
        host.path_map
            .add_provenance
            .review_name
            .as_mut()
            .unwrap()
            .stable_name = "<draft:0>".to_owned();
        host.path_map.add_provenance.review_source_path =
            Some(PathBuf::from("/unregistered/current.py"));
        assert!(project(&mut host, valid.clone()).is_err());
        host.path_map.add_provenance.review_source_path = Some(first.clone());

        for pointer in [
            "/add/0/commit/entry/name",
            "/add/0/commit/entry/kind",
            "/add/0/commit/entry/mode",
            "/add/0/commit/entry/source",
            "/add/0/commit/source/path",
            "/add/0/commit/source/source_record",
            "/add/0/commit/source/is_draft",
        ] {
            let mut mismatch = valid.clone();
            *mismatch.pointer_mut(pointer).unwrap() = match pointer {
                "/add/0/commit/entry/kind" => json!("shell"),
                "/add/0/commit/entry/mode" => json!("reference"),
                "/add/0/commit/source/is_draft" => json!(false),
                _ => json!("user-owned-lookalike"),
            };
            let mut typed: Effect =
                super::deserialize_canonical(mismatch, "C2 mismatched derived Commit fixture")
                    .unwrap();
            let before = typed.clone();
            assert!(host.path_map.project_typed_effect(&mut typed).is_err());
            assert_eq!(typed, before, "{pointer}");
        }
        let mut missing_source = valid.clone();
        missing_source["add"][0]["commit"]["source"] = Value::Null;
        assert!(project(&mut host, missing_source).is_err());

        host.path_map.add_provenance.review_source_path = Some(second.clone());
        let changed_source = commit_value(&second, "skit-new-first");
        assert_eq!(
            project(&mut host, changed_source).unwrap()["add"][0]["commit"]["entry"]["name"],
            "<draft:0>"
        );

        set_review_name_projection(
            &mut host,
            Path::new("/unregistered/skit-new-first.py"),
            "skit-new-first",
            "<draft:0>",
        );
        assert!(project(&mut host, valid).is_err());
    }

    #[test]
    fn c2_add_input_mirror_refuses_cursor_yank_cut_and_shape_drift() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(&host, "skit-new-界.py", b"print('unicode')\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        host.path_map.add_provenance.selected_source_path = Some(draft.clone());
        let raw = draft.display().to_string();
        let input = json!({
            "id": "SourcePath",
            "state": {
                "value": raw,
                "cursor": raw.chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        });
        let valid = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![input.clone()],
        );
        let mut projected = valid.clone();
        host.path_map.project_session_value(&mut projected).unwrap();
        let stable = "<profile:fixture>/data/.drafts/<draft:0>.py";
        assert_eq!(
            projected["add"]["fields"]["inputs"][0]["state"]["value"],
            stable
        );
        assert_eq!(
            projected["add"]["fields"]["inputs"][0]["state"]["cursor"],
            stable.chars().count()
        );

        let mut invalid = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        assert!(host.path_map.project_session_value(&mut invalid).is_err());
        let mut duplicate = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![input.clone(), input.clone()],
        );
        assert!(host.path_map.project_session_value(&mut duplicate).is_err());
        for (pointer, replacement) in [
            ("/add/fields/inputs/0/state/value", json!("wrong")),
            ("/add/fields/inputs/0/state/cursor", json!(0)),
            ("/add/fields/inputs/0/state/yank", json!("cut")),
            ("/add/fields/inputs/0/state/yank", json!(7)),
            ("/add/fields/inputs/0/state/last_was_cut", json!(true)),
            ("/add/fields/inputs/0/state", json!([])),
        ] {
            let mut invalid = valid.clone();
            *invalid.pointer_mut(pointer).unwrap() = replacement;
            assert!(host.path_map.project_session_value(&mut invalid).is_err());
        }
        let mut missing_state = valid.clone();
        missing_state["add"]["fields"]["inputs"][0]
            .as_object_mut()
            .unwrap()
            .remove("state");
        assert!(
            host.path_map
                .project_session_value(&mut missing_state)
                .is_err()
        );
        let mut missing_field = valid.clone();
        missing_field["add"]["fields"]["inputs"][0]["state"]
            .as_object_mut()
            .unwrap()
            .remove("yank");
        assert!(
            host.path_map
                .project_session_value(&mut missing_field)
                .is_err()
        );
        let mut extra_field = valid;
        extra_field["add"]["fields"]["inputs"][0]["state"]["extra"] = json!(true);
        assert!(
            host.path_map
                .project_session_value(&mut extra_field)
                .is_err()
        );
    }

    #[test]
    fn c2_add_review_name_yank_keeps_the_cut_derived_name_projected() {
        let review_input = |value: &str, yank: &str, cut: bool| {
            json!({
                "id": "ReviewName",
                "state": {
                    "value": value,
                    "cursor": value.chars().count(),
                    "yank": yank,
                    "last_was_cut": cut,
                },
            })
        };
        let project = |host: &mut RealWalkerHost, value: &str, yank: &str, cut: bool| {
            let mut session = c2_session(
                Path::new("/unregistered/session"),
                None,
                vec![review_input(value, yank, cut)],
            );
            host.path_map
                .project_session_value(&mut session)
                .map(|()| session["add"]["fields"]["inputs"][0]["state"].clone())
        };
        let edit = |name: &str| {
            super::deserialize_canonical::<Action>(
                json!({"add": {"set_review_name": name}}),
                "C2 SetReviewName fixture",
            )
            .unwrap()
        };
        let state = |name: &str| {
            json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {"name": name},
            }}}})
        };

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(&host, "skit-new-first.py", b"first\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        host.path_map.add_provenance.review_source_path = Some(draft.clone());
        set_review_name_projection(&mut host, &draft, "skit-new-first", "<draft:0>");

        host.path_map.record_add_projection_cause(&edit(""));
        host.path_map.reconcile_add_provenance(&state("")).unwrap();
        assert!(host.path_map.add_provenance.review_name.is_none());
        let cut = project(&mut host, "", "skit-new-first", true).unwrap();
        assert_eq!(cut["yank"], "<draft:0>");
        assert_eq!(cut["value"], "");
        assert_eq!(cut["cursor"], 0);
        assert_eq!(cut["last_was_cut"], true);

        host.path_map
            .record_add_projection_cause(&edit("Corpus Prompt"));
        host.path_map
            .reconcile_add_provenance(&state("Corpus Prompt"))
            .unwrap();
        let typed = project(&mut host, "Corpus Prompt", "skit-new-first", false).unwrap();
        assert_eq!(typed["yank"], "<draft:0>");
        assert_eq!(typed["value"], "Corpus Prompt");

        let replaced = project(&mut host, "Corpus Prompt", "Corpus", false).unwrap();
        assert_eq!(replaced["yank"], "Corpus");

        let mut manual = RealWalkerHost::spawn(profile()).unwrap();
        let manual_draft = write_draft_at(&manual, "skit-new-first.py", b"first\n", 10);
        manual.path_map.refresh(&manual.service).unwrap();
        manual.path_map.add_provenance.review_source_path = Some(manual_draft);
        manual.path_map.add_provenance.review_name_edited = true;
        let lookalike = project(&mut manual, "typed", "skit-new-first", true).unwrap();
        assert_eq!(lookalike["yank"], "skit-new-first");

        let mut drift = RealWalkerHost::spawn(profile()).unwrap();
        let drift_draft = write_draft_at(&drift, "skit-new-first.py", b"first\n", 10);
        drift.path_map.refresh(&drift.service).unwrap();
        drift.path_map.add_provenance.review_source_path = Some(drift_draft.clone());
        set_review_name_projection(&mut drift, &drift_draft, "skit-new-first", "<draft:0>");
        drift.path_map.add_cause = super::AddProjectionCause::None;
        drift
            .path_map
            .reconcile_add_provenance(&state("Corpus Prompt"))
            .unwrap();
        assert!(drift.path_map.add_provenance.review_name.is_none());
        let drifted = project(&mut drift, "Corpus Prompt", "skit-new-first", false).unwrap();
        assert_eq!(drifted["yank"], "<draft:0>");
    }

    #[test]
    fn c2_runner_failure_status_provenance_replays_and_success_values_stay_user_owned() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let mut state = host.initial_state().unwrap();
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Runners,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        let previous = host.observe(&state).unwrap().state;
        let root = host._sandbox.path().display().to_string();
        let failure = format!("Runner store failure ({root}).");
        let stable_failure = "Runner store failure (<profile:fixture>).";
        let action = Action::Runners(skit_ui::RunnerManagerAction::MutationFailed(failure));
        let emitted = state.update(action.clone());
        let mut projected_state = state.clone();
        let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let mut recorded_action = action;
        let mut recorded_emitted = emitted;
        let observation = host
            .capture_checkpoint_parts(
                &mut projected_state,
                &mut session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_action,
                    emitted: &mut recorded_emitted,
                },
            )
            .unwrap();
        assert_eq!(
            observation.state["workflow"]["active"]["runners"]["status"],
            stable_failure
        );
        assert_eq!(
            serde_json::to_value(&recorded_action).unwrap()["runners"]["mutation_failed"],
            stable_failure
        );
        super::super::tui_real_walker::validate_reducer_action(
            &previous,
            &observation.state,
            &serde_json::to_value(recorded_action).unwrap(),
            &serde_json::to_value(recorded_emitted).unwrap(),
        )
        .unwrap();

        let success: Action = super::deserialize_canonical(
            json!({"runners": {"mutation_succeeded": {
                "rows": [{
                    "identity": {"index": 0, "snapshot_token": root},
                    "name": root,
                    "argv": [root],
                    "reason": null,
                    "descriptor": root,
                    "key_identities": [],
                    "pinned_count": 0,
                }],
                "selected_name": root,
                "message": format!("Saved runner ({root})."),
            }}}),
            "C2 runner success fixture",
        )
        .unwrap();
        let emitted = state.update(success.clone());
        let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let mut recorded_action = success;
        let mut recorded_emitted = emitted;
        let observation = host
            .capture_checkpoint_parts(
                &mut state,
                &mut session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_action,
                    emitted: &mut recorded_emitted,
                },
            )
            .unwrap();
        let action = serde_json::to_value(recorded_action).unwrap();
        assert_eq!(
            observation.state["workflow"]["active"]["runners"]["status"],
            format!("Saved runner ({root}).")
        );
        let row = &action["runners"]["mutation_succeeded"]["rows"][0];
        assert_eq!(row["name"], root);
        assert_eq!(row["argv"][0], root);
        assert_eq!(row["descriptor"], root);
        assert_eq!(row["identity"]["snapshot_token"], root);
    }

    #[test]
    fn c2_external_skit_new_editor_path_never_gains_draft_provenance() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let path = host.external_root.join("skit-new-user.py");
        fs::write(&path, b"print('user')\n").unwrap();

        host.path_map
            .register_event_artifacts(&super::PortEvent::Editor {
                argv: vec!["editor".to_owned()],
                path: path.clone(),
                outcome: json!({"ok": true}),
            });

        assert!(!host.path_map.draft_paths.contains_key(&path));
        assert_eq!(
            host.path_map.normalize_path(&path),
            "<profile:fixture>/external/skit-new-user.py"
        );
        assert_eq!(
            host.path_map
                .text_artifacts
                .get(&path.display().to_string()),
            Some(&"<profile:fixture>/external/skit-new-user.py".to_owned())
        );
    }

    #[test]
    fn c2_session_projection_failure_retains_then_drains_the_event_prefix_once() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.clear_transcript();
        host.adapters.push(super::PortEvent::Output {
            stream: "diagnostic".to_owned(),
            text: "retained session event".to_owned(),
        });
        let state = host.initial_state().unwrap();
        let mut failed_state = state.clone();
        let mut invalid_session = json!({"schema_version": 2});

        assert!(
            host.capture_checkpoint_parts(
                &mut failed_state,
                &mut invalid_session,
                CheckpointCauseProjection::Observation,
            )
            .is_err()
        );
        assert!(host.adapters.pending_events().iter().any(|event| {
            matches!(event, super::PortEvent::Output { text, .. } if text == "retained session event")
        }));

        let mut valid_state = state.clone();
        let mut valid_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let observation = host
            .capture_checkpoint_parts(
                &mut valid_state,
                &mut valid_session,
                CheckpointCauseProjection::Observation,
            )
            .unwrap();
        assert_eq!(
            events_with_keys(&observation.transcript, &["output"]),
            vec![json!({
                "output": "diagnostic",
                "text": "retained session event",
            })]
        );
        assert!(host.adapters.pending_events().is_empty());

        let mut final_state = state;
        let mut final_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let final_observation = host
            .capture_checkpoint_parts(
                &mut final_state,
                &mut final_session,
                CheckpointCauseProjection::Observation,
            )
            .unwrap();
        assert!(events_with_keys(&final_observation.transcript, &["output"]).is_empty());
    }

    #[test]
    fn c2_every_add_effect_path_projects_in_all_checkpoint_cause_positions() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft_path = write_draft_at(
            &host,
            "skit-new-effect-cause.py",
            b"print('effect cause')\n",
            10,
        );
        host.path_map.refresh(&host.service).unwrap();
        let draft = serde_json::to_value(
            super::sorted_tui_drafts(&host.roots().data)
                .into_iter()
                .next()
                .unwrap(),
        )
        .unwrap();
        let source = json!({
            "path": draft_path,
            "source_record": draft_path,
            "bytes": [1, 2, 3],
            "permissions": {"readonly": false, "unix_mode": 384},
            "executable": false,
            "is_regular": true,
            "is_directory": false,
            "is_draft": true,
            "identity": null,
        });
        let mut entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
        entry["source"] = json!(draft_path);
        let raw: Effect = super::deserialize_canonical(
            json!({"add": [
                {"inspect_source": {"request": 0, "path": draft_path}},
                {"delete_draft": {"request": 0, "draft": draft}},
                {"edit_source": {"request": 0, "path": draft_path}},
                {"commit": {"request": 0, "entry": entry, "source": source}},
                {"consume_draft": source},
                {"draft_kept": draft_path},
            ]}),
            "C2 checkpoint Add Effect fixture",
        )
        .unwrap();
        let raw_text = serde_json::to_string(&draft_path).unwrap();
        let stable = "<profile:fixture>/data/.drafts/<draft:0>.py";
        let assert_projected = |effect: &Effect| {
            let value = serde_json::to_value(effect).unwrap();
            for pointer in [
                "/add/0/inspect_source/path",
                "/add/1/delete_draft/draft/path",
                "/add/2/edit_source/path",
                "/add/3/commit/entry/source",
                "/add/3/commit/source/path",
                "/add/3/commit/source/source_record",
                "/add/4/consume_draft/path",
                "/add/4/consume_draft/source_record",
                "/add/5/draft_kept",
            ] {
                assert_eq!(value.pointer(pointer), Some(&json!(stable)), "{pointer}");
            }
            assert!(!serde_json::to_string(&value).unwrap().contains(&raw_text));
        };

        let mut state = host.initial_state().unwrap();
        let mut action = Action::ClearStatus;
        let mut reducer_emitted = raw.clone();
        let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        host.capture_checkpoint_parts(
            &mut state,
            &mut session,
            CheckpointCauseProjection::Reducer {
                action: &mut action,
                emitted: &mut reducer_emitted,
            },
        )
        .unwrap();
        assert_projected(&reducer_emitted);

        let mut state = host.initial_state().unwrap();
        let mut request = raw.clone();
        let mut response = Action::ClearStatus;
        let mut host_emitted = raw.clone();
        let mut session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        host.capture_checkpoint_parts(
            &mut state,
            &mut session,
            CheckpointCauseProjection::Host {
                request: &mut request,
                response: &mut response,
                emitted: &mut host_emitted,
            },
        )
        .unwrap();
        for projected in [&request, &host_emitted] {
            assert_projected(projected);
        }
        assert!(serde_json::to_string(&raw).unwrap().contains(&raw_text));
    }

    #[test]
    fn c2_source_edited_keeps_derived_review_provenance_when_the_name_is_unchanged() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let first = write_draft_at(&host, "skit-new-first.py", b"first\n", 10);
        let second = write_draft_at(&host, "skit-new-second.py", b"second\n", 20);
        host.path_map.refresh(&host.service).unwrap();
        host.path_map.add_provenance.review_source_path = Some(first.clone());
        set_review_name_projection(&mut host, &first, "skit-new-first", "<draft:0>");
        let source = |path: &Path| {
            json!({
                "path": path,
                "source_record": path,
                "bytes": [],
                "permissions": {"readonly": false, "unix_mode": null},
                "executable": false,
                "is_regular": true,
                "is_directory": false,
                "is_draft": true,
                "identity": null,
            })
        };
        let action = |path: &Path| {
            super::deserialize_canonical::<Action>(
                json!({"add": {"source_edited": {
                    "request": 0,
                    "result": {"Ok": source(path)},
                }}}),
                "C2 SourceEdited fixture",
            )
            .unwrap()
        };
        let state = |path: &Path, name: &str| {
            json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {"name": name, "source": source(path)},
            }}}})
        };

        host.path_map.record_add_projection_cause(&action(&first));
        host.path_map
            .reconcile_add_provenance(&state(&first, "skit-new-first"))
            .unwrap();
        assert_eq!(
            host.path_map
                .add_provenance
                .review_name
                .as_ref()
                .map(|projection| &projection.source_path),
            Some(&first)
        );
        let mut same_screen = json!({"add": state(&first, "skit-new-first")
            ["workflow"]["active"]["add"].clone()});
        host.path_map
            .normalize_add_screen(&mut same_screen)
            .unwrap();
        assert_eq!(same_screen["add"]["review"]["name"], "<draft:0>");
        let mut same_session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "ReviewName",
                "state": {
                    "value": "skit-new-first",
                    "cursor": "skit-new-first".chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        host.path_map
            .project_session_value(&mut same_session)
            .unwrap();
        assert_eq!(
            same_session["add"]["fields"]["inputs"][0]["state"]["value"],
            "<draft:0>"
        );

        host.path_map.record_add_projection_cause(&action(&second));
        host.path_map
            .reconcile_add_provenance(&state(&second, "skit-new-first"))
            .unwrap();
        assert_eq!(
            host.path_map.add_provenance.review_source_path,
            Some(second.clone())
        );
        assert_eq!(
            host.path_map
                .add_provenance
                .review_name
                .as_ref()
                .map(|projection| &projection.source_path),
            Some(&first)
        );
        assert!(!host.path_map.add_provenance.review_name_edited);
        let mut changed_screen = json!({"add": state(&second, "skit-new-first")
            ["workflow"]["active"]["add"].clone()});
        host.path_map
            .normalize_add_screen(&mut changed_screen)
            .unwrap();
        assert_eq!(changed_screen["add"]["review"]["name"], "<draft:0>");
        let mut changed_session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "ReviewName",
                "state": {
                    "value": "skit-new-first",
                    "cursor": "skit-new-first".chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        host.path_map
            .project_session_value(&mut changed_session)
            .unwrap();
        assert_eq!(
            changed_session["add"]["fields"]["inputs"][0]["state"]["value"],
            "<draft:0>"
        );

        host.path_map.add_cause = super::AddProjectionCause::ReviewSource(Some(first.clone()));
        let pending = json!({"workflow": {"active": {"add": {
            "review_defaults": {"name": null},
            "pending_source": source(&first),
            "kind_picker": {"filename": "skit-new-first.py"},
        }}}});
        host.path_map.reconcile_add_provenance(&pending).unwrap();
        let mut pending_screen = json!({"add": pending["workflow"]["active"]["add"].clone()});
        host.path_map
            .normalize_add_screen(&mut pending_screen)
            .unwrap();
        assert_eq!(
            pending_screen["add"]["kind_picker"]["filename"],
            "<draft:0>.py"
        );
        host.path_map.add_cause = super::AddProjectionCause::None;
        let review = state(&first, "skit-new-first");
        host.path_map.reconcile_add_provenance(&review).unwrap();
        let mut review_screen = json!({"add": review["workflow"]["active"]["add"].clone()});
        host.path_map
            .normalize_add_screen(&mut review_screen)
            .unwrap();
        assert_eq!(review_screen["add"]["review"]["name"], "<draft:0>");

        host.path_map
            .record_add_projection_cause(&Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(Vec::new()),
            ))));
        let leaving = json!({"workflow": {"active": {"add": {
            "source": {"path": first, "selected_draft": null, "drafts": []},
        }}}});
        host.path_map.reconcile_add_provenance(&leaving).unwrap();
        let mut leaving_screen = json!({"add": leaving["workflow"]["active"]["add"].clone()});
        host.path_map
            .normalize_add_screen(&mut leaving_screen)
            .unwrap();
        assert_eq!(
            leaving_screen["add"]["source"]["path"],
            first.display().to_string()
        );
        let mut leaving_session = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "SourcePath",
                "state": {
                    "value": first.display().to_string(),
                    "cursor": first.display().to_string().chars().count(),
                    "yank": "",
                    "last_was_cut": false,
                },
            })],
        );
        host.path_map
            .project_session_value(&mut leaving_session)
            .unwrap();
        assert_eq!(
            leaving_session["add"]["fields"]["inputs"][0]["state"]["value"],
            first.display().to_string()
        );
    }

    #[test]
    fn c2_provenance_refuses_mismatched_state_and_covers_optional_session_shapes() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(&host, "skit-new-refusal.py", b"refusal\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        let draft_row = serde_json::to_value(
            super::sorted_tui_drafts(&host.roots().data)
                .into_iter()
                .next()
                .unwrap(),
        )
        .unwrap();
        let add_state = |path: &Path, selected: Value, source_path: &str| {
            json!({"workflow": {"active": {"add": {
                "source": {
                    "drafts": [{"path": path}],
                    "selected_draft": selected,
                    "path": source_path,
                },
            }}}})
        };

        host.path_map.add_cause = super::AddProjectionCause::SelectDraft(0);
        assert!(
            host.path_map
                .reconcile_add_provenance(&add_state(
                    Path::new("/outside/unregistered.py"),
                    json!(0),
                    "/outside/unregistered.py",
                ))
                .is_err()
        );
        host.path_map.add_cause = super::AddProjectionCause::SelectDraft(0);
        assert!(
            host.path_map
                .reconcile_add_provenance(&add_state(
                    &draft,
                    json!(1),
                    &draft.display().to_string()
                ))
                .is_err()
        );
        host.path_map.add_cause = super::AddProjectionCause::SelectDraft(0);
        assert!(
            host.path_map
                .reconcile_add_provenance(&add_state(&draft, json!(0), "different"))
                .is_err()
        );
        host.path_map.add_provenance.selected_source_path = Some(draft.clone());
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&add_state(
                &draft,
                Value::Null,
                &draft.display().to_string(),
            ))
            .unwrap();
        assert_eq!(
            host.path_map.add_provenance.selected_source_path,
            Some(draft.clone())
        );
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&add_state(&draft, Value::Null, "different"))
            .unwrap();
        assert!(host.path_map.add_provenance.selected_source_path.is_none());

        host.path_map.add_provenance.review_source_path = Some(draft.clone());
        set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {}}}}))
            .unwrap();
        assert!(host.path_map.add_provenance.review_source_path.is_none());

        host.path_map.add_provenance.review_source_path = Some(draft.clone());
        set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
        host.path_map.add_provenance.review_name_edited = false;
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {
                    "name": "user changed the derived name",
                    "source": {"path": draft},
                },
            }}}}))
            .unwrap();
        assert!(host.path_map.add_provenance.review_name.is_none());
        assert!(host.path_map.add_provenance.review_name_edited);

        host.path_map.add_provenance.review_source_path = None;
        host.path_map.add_provenance.review_name = None;
        host.path_map.add_provenance.review_name_edited = false;
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {"name": "user", "source": {"path": draft}},
            }}}}))
            .unwrap();
        assert!(host.path_map.add_provenance.review_name.is_none());
        assert!(!host.path_map.add_provenance.review_name_edited);

        host.path_map.add_provenance.review_source_path = Some(draft.clone());
        host.path_map.add_provenance.review_name = None;
        host.path_map.add_provenance.review_name_edited = false;
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {
                    "name": "skit-new-refusal",
                    "source": {"path": "/outside/different.py"},
                },
            }}}}))
            .unwrap();
        assert!(host.path_map.add_provenance.review_source_path.is_none());
        assert!(host.path_map.add_provenance.review_name_edited);

        let unregistered = host.roots().data.join("not-a-draft.py");
        host.path_map.add_provenance.review_source_path = Some(unregistered.clone());
        host.path_map.add_provenance.review_name = None;
        host.path_map.add_provenance.review_name_edited = false;
        host.path_map.add_cause = super::AddProjectionCause::None;
        let error = host
            .path_map
            .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {
                    "name": "not-a-draft",
                    "source": {"path": unregistered},
                },
            }}}}))
            .unwrap_err();
        assert_eq!(
            error,
            "an auto-derived Add review source path is not registered"
        );
        assert!(host.path_map.add_provenance.review_name.is_none());

        host.path_map.add_provenance.review_source_path = Some(draft.clone());
        host.path_map.add_provenance.review_name_edited = false;
        host.path_map.add_cause = super::AddProjectionCause::None;
        host.path_map
            .reconcile_add_provenance(&json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {"name": "wrong", "source": {"path": draft}},
            }}}}))
            .unwrap();
        assert!(host.path_map.add_provenance.review_name_edited);
        set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
        let mut wrong_review = json!({"add": {"review": {"name": "wrong"}}});
        assert!(
            host.path_map
                .normalize_add_screen(&mut wrong_review)
                .is_err()
        );
        set_review_name_projection(&mut host, &unregistered, "not-a-draft", "<not-a-draft>");
        let mut unregistered_review = json!({"add": {"review": {"name": "not-a-draft"}}});
        assert!(
            host.path_map
                .normalize_add_screen(&mut unregistered_review)
                .is_err()
        );

        host.path_map.status_cause =
            super::StatusProjectionCause::AgentSkillInstalled("expected".to_owned());
        assert!(
            host.path_map
                .reconcile_status_provenance(&json!({"status": "different"}))
                .is_err()
        );
        host.path_map.status_provenance = Some("expected".to_owned());
        assert!(
            host.path_map
                .normalize_library_json(json!({"status": "different"}))
                .is_err()
        );
        host.path_map.status_provenance = Some("not a template".to_owned());
        assert!(
            host.path_map
                .normalize_library_json(json!({"status": "not a template"}))
                .is_err()
        );
        host.path_map.status_provenance = None;
        let mut not_text = Value::Null;
        assert!(
            !host
                .path_map
                .normalize_agent_skill_install_text(&mut not_text)
                .unwrap()
        );

        let source = SourceSnapshot {
            path: draft.clone(),
            source_record: draft.display().to_string(),
            bytes: Vec::new(),
            permissions: SourcePermissions::default(),
            executable: None,
            is_regular: true,
            is_directory: false,
            is_draft: true,
            identity: None,
        };
        for action in [
            super::deserialize_canonical::<AddAction>(
                json!({"draft_edited": {"request": 0, "result": {"Ok": source}}}),
                "DraftEdited Some cause",
            )
            .unwrap(),
            super::deserialize_canonical::<AddAction>(
                json!({"draft_edited": {"request": 0, "result": {"Ok": null}}}),
                "DraftEdited None cause",
            )
            .unwrap(),
            super::deserialize_canonical::<AddAction>(
                json!({"draft_edited": {"request": 0, "result": {"Err": "failed"}}}),
                "DraftEdited Err cause",
            )
            .unwrap(),
        ] {
            let _ = super::add_projection_cause(&action);
        }

        host.path_map.add_provenance = super::AddProjectionProvenance::default();
        let mut optional = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![json!({
                "id": "Other",
                "state": {},
            })],
        );
        optional["preferences"]["fields"]["agent_signature"] = Value::Null;
        optional["path_suggestions"]["fields"]["expected"] = Value::Null;
        optional["path_suggestions"]["fields"]["visible"] = Value::Null;
        optional["add"]["fields"]["signature"] = Value::Null;
        host.path_map.project_session_value(&mut optional).unwrap();

        set_review_name_projection(&mut host, &draft, "skit-new-refusal", "<draft:0>");
        let mut missing = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        missing["add"]["fields"]["signature"]["stage"] = json!("review");
        assert!(host.path_map.project_session_value(&mut missing).is_err());
        let review = json!({
            "id": "ReviewName",
            "state": {
                "value": "skit-new-refusal",
                "cursor": "skit-new-refusal".chars().count(),
                "yank": "",
                "last_was_cut": false,
            },
        });
        let mut duplicate = c2_session(
            Path::new("/unregistered/session"),
            None,
            vec![review.clone(), review],
        );
        assert!(host.path_map.project_session_value(&mut duplicate).is_err());
        let mut scalar_inputs = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        scalar_inputs["add"]["fields"]["inputs"] = json!({});
        assert!(
            host.path_map
                .project_session_value(&mut scalar_inputs)
                .is_err()
        );

        let mut unrelated = json!({});
        host.path_map
            .normalize_preferences_screen(&mut unrelated)
            .unwrap();
        assert_eq!(draft_row["path"], draft.display().to_string());
    }

    #[test]
    fn c2_add_screen_artifact_problem_notice_and_user_field_contract() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft_path = write_draft_at(&host, "skit-new-screen.py", b"print('screen')\n", 10);
        host.path_map.refresh(&host.service).unwrap();
        let draft = serde_json::to_value(
            super::sorted_tui_drafts(&host.roots().data)
                .into_iter()
                .next()
                .unwrap(),
        )
        .unwrap();
        let source = json!({
            "path": draft_path,
            "source_record": draft_path,
            "bytes": [1, 2, 3],
            "permissions": {"readonly": true, "unix_mode": 416},
            "executable": false,
            "is_regular": true,
            "is_directory": false,
            "is_draft": true,
            "identity": null,
        });
        let root = host._sandbox.path().display().to_string();
        let message = format!("Add failure ({root}).");
        let stable_message = "Add failure (<profile:fixture>).";
        let mut screen = json!({"add": {
            "source": {"path": root, "drafts": [draft]},
            "pending_source": source,
            "review": {"name": root, "source": source},
            "pending_delete": [0, draft],
            "delete_candidate": draft,
            "problem": {"source_unavailable": {"path": root, "reason": message}},
            "notice": {"draft_kept": draft_path},
        }});
        host.path_map.normalize_add_screen(&mut screen).unwrap();
        let add = &screen["add"];
        assert_eq!(add["source"]["path"], root);
        assert_eq!(add["review"]["name"], root);
        for pointer in [
            "/source/drafts/0/path",
            "/pending_source/path",
            "/pending_source/source_record",
            "/review/source/path",
            "/review/source/source_record",
            "/pending_delete/1/path",
            "/delete_candidate/path",
            "/notice/draft_kept",
        ] {
            assert_eq!(
                add.pointer(pointer),
                Some(&json!("<profile:fixture>/data/.drafts/<draft:0>.py"))
            );
        }
        assert_eq!(
            add["problem"]["source_unavailable"]["path"],
            "<profile:fixture>"
        );
        assert_eq!(
            add["problem"]["source_unavailable"]["reason"],
            stable_message
        );
        assert_eq!(add["pending_source"]["permissions"]["unix_mode"], 416);

        for (problem, pointer) in [
            (
                json!({"draft_changed": {"path": draft_path}}),
                "/draft_changed/path",
            ),
            (
                json!({"source_edit": {"reason": message}}),
                "/source_edit/reason",
            ),
            (
                json!({"commit_failed": {"reason": message}}),
                "/commit_failed/reason",
            ),
            (
                json!({"edit_failed": {"reason": message}}),
                "/edit_failed/reason",
            ),
            (
                json!({"draft_delete_failed": {"reason": message}}),
                "/draft_delete_failed/reason",
            ),
        ] {
            let mut screen = json!({"add": {"problem": problem}});
            host.path_map.normalize_add_screen(&mut screen).unwrap();
            let expected = if pointer.ends_with("/path") {
                "<profile:fixture>/data/.drafts/<draft:0>.py"
            } else {
                stable_message
            };
            assert_eq!(
                screen["add"]["problem"].pointer(pointer),
                Some(&json!(expected))
            );
        }
        for notice in [
            json!({"draft_kept": draft_path}),
            json!({"draft_deleted": draft_path}),
        ] {
            let mut screen = json!({"add": {"notice": notice}});
            host.path_map.normalize_add_screen(&mut screen).unwrap();
            assert!(
                serde_json::to_string(&screen)
                    .unwrap()
                    .contains("<profile:fixture>/data/.drafts/<draft:0>.py")
            );
        }
        for problem in [
            json!({"invalid_dependency": {"value": root}}),
            json!({"invalid_python_constraint": {"value": root}}),
        ] {
            let mut screen = json!({"add": {"problem": problem}});
            let before = screen.clone();
            host.path_map.normalize_add_screen(&mut screen).unwrap();
            assert_eq!(screen, before);
        }
    }

    fn assert_clone_debug_eq<T: Clone + std::fmt::Debug + Eq>(value: &T) {
        assert_eq!(value, &value.clone(), "{value:?}");
    }

    #[test]
    fn ambient_path_facts_keep_lexical_and_resolved_spellings() {
        let parent = tempfile::TempDir::new().unwrap();
        let lexical = parent.path().join(".");
        let resolved = fs::canonicalize(&lexical).unwrap();
        let absent = parent.path().join("absent");
        let paths = super::ambient_path_spellings([
            lexical.clone(),
            lexical.clone(),
            absent.clone(),
            PathBuf::new(),
        ]);

        let expected = BTreeSet::from([
            lexical.display().to_string(),
            resolved.display().to_string(),
            super::ordinary_path(&resolved).display().to_string(),
            absent.display().to_string(),
        ]);
        assert_eq!(paths, expected);
    }

    #[test]
    fn ambient_path_facts_capture_the_process_without_entering_observation_json() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let facts = host.leak_oracle_facts();
        let mut roots = vec![
            super::super::tui_walker_bundle::checkout_root().unwrap(),
            std::env::current_dir().unwrap(),
            std::env::temp_dir(),
        ];
        roots.extend(["HOME", "USERPROFILE"].into_iter().filter_map(|name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        }));
        for root in roots {
            assert!(facts.ambient_paths.contains(&root.display().to_string()));
        }

        let state = host.initial_state().unwrap();
        let observation = serde_json::to_value(host.observe(&state).unwrap()).unwrap();
        assert!(observation.get("ambient_paths").is_none());
        assert_eq!(host.leak_oracle_facts().ambient_paths, facts.ambient_paths);
        host.close().unwrap();
    }

    #[test]
    fn c3_artifact_facts_capture_only_exact_projection_pairs() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft_path = write_draft_at(
            &host,
            "skit-new-c3-artifact.py",
            b"print('artifact facts')\n",
            10,
        );
        host.path_map.refresh(&host.service).unwrap();
        let mut draft = serde_json::to_value(
            super::sorted_tui_drafts(&host.roots().data)
                .into_iter()
                .find(|draft| draft.path == draft_path)
                .unwrap(),
        )
        .unwrap();
        let raw_path = draft["path"].as_str().unwrap().to_owned();
        let raw_identity =
            skit_tui_walker_support::canonical_json_bytes(&draft["identity"]).unwrap();
        let raw_modified = draft["modified"].as_u64().unwrap();
        let mut path_only = draft["path"].clone();
        host.path_map.normalize_path_value(&mut path_only);

        let before = host.leak_oracle_facts();
        assert_clone_debug_eq(&before);
        assert_eq!(
            before
                .artifacts
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["<profile:fixture>"]
        );

        host.path_map.normalize_draft_summary(&mut draft).unwrap();

        let projected_path = draft["path"].as_str().unwrap();
        let projected_identity =
            skit_tui_walker_support::canonical_json_bytes(&draft["identity"]).unwrap();
        let projected_modified = draft["modified"].as_u64().unwrap();
        let after = host.leak_oracle_facts();
        let artifact = &after.artifacts[projected_path];
        assert_eq!(artifact.raw_path_spellings.len(), 1);
        assert!(artifact.raw_path_spellings.contains(&raw_path));
        assert_eq!(artifact.source_identities.len(), 1);
        let identity = artifact.source_identities.iter().next().unwrap();
        assert_eq!(identity.raw, raw_identity);
        assert_eq!(identity.projected, projected_identity);
        assert_eq!(artifact.modified_values.len(), 1);
        let modified = artifact.modified_values.iter().next().unwrap();
        assert_eq!(modified.raw, raw_modified);
        assert_eq!(modified.projected, projected_modified);

        host.path_map.normalize_draft_summary(&mut draft).unwrap();
        assert_eq!(host.leak_oracle_facts(), after);
    }

    #[test]
    fn c3_renderer_draft_facts_keep_all_producer_names_and_observed_review_pairs() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_draft_at(
            &host,
            "skit-x.prompt.py",
            b"print('kind-sensitive name')\n",
            10,
        );
        host.path_map.refresh(&host.service).unwrap();
        let stable_path = host.path_map.normalize_path(&draft);
        let before = host.leak_oracle_facts();
        let renderer = &before.renderer_drafts[&stable_path];
        assert_eq!(renderer.raw_path, draft.display().to_string());
        assert_eq!(renderer.projected_path, stable_path);
        assert_eq!(renderer.raw_kind_picker_basename, "skit-x.prompt.py");
        assert_eq!(renderer.raw_lossy_basename, "skit-x.prompt.py");
        assert_eq!(renderer.projected_basename, "<draft:0>.py");
        assert!(renderer.review_names.is_empty());

        for (kind, raw_name) in [("python", "skit-x.prompt"), ("prompt", "skit-x")] {
            host.path_map.add_cause = super::AddProjectionCause::ReviewSource(Some(draft.clone()));
            let state = json!({"workflow": {"active": {"add": {
                "review_defaults": {"name": null},
                "review": {
                    "kind": kind,
                    "name": raw_name,
                    "source": {"path": draft},
                },
            }}}});
            host.path_map.reconcile_add_provenance(&state).unwrap();
            let mut screen = json!({"add": state["workflow"]["active"]["add"].clone()});
            host.path_map.normalize_add_screen(&mut screen).unwrap();
            assert_eq!(screen["add"]["review"]["name"], "<draft:0>");
        }

        let after = host.leak_oracle_facts();
        assert!(before.renderer_drafts[&stable_path].review_names.is_empty());
        let review_names = &after.renderer_drafts[&stable_path].review_names;
        assert_eq!(review_names.len(), 2);
        for (kind, raw_name) in [("python", "skit-x.prompt"), ("prompt", "skit-x")] {
            assert!(review_names.iter().any(|fact| {
                fact.kind == kind
                    && fact.raw_source_path == draft.display().to_string()
                    && fact.projected_source_path == stable_path
                    && fact.raw_name == raw_name
                    && fact.projected_name == "<draft:0>"
            }));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn c3_renderer_draft_facts_keep_distinct_non_utf_producer_basenames() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = write_non_utf_draft_at(&host);
        host.path_map.refresh(&host.service).unwrap();
        let stable_path = host.path_map.normalize_path(&draft);

        let facts = host.leak_oracle_facts();
        let renderer = &facts.renderer_drafts[&stable_path];
        assert_eq!(renderer.raw_kind_picker_basename, "");
        assert_eq!(
            renderer.raw_lossy_basename,
            draft.file_name().unwrap().to_string_lossy()
        );
        assert_eq!(renderer.projected_basename, "<draft:0>.unknown");
    }

    #[test]
    fn real_host_add_screen_with_a_draft_round_trips_through_parse_library_state() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_draft_at(&host, "skit-new-round-trip.py", b"print('draft')\n", 10);
        let mut state = host.initial_state().unwrap();
        let action = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert!(matches!(&action, Action::Present(skit_ui::Screen::Add(_))));
        assert_eq!(state.update(action), Effect::None);

        let value = host.observe(&state).unwrap().state;
        let parsed = super::super::tui_real_walker::parse_library_state(&value).unwrap();

        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }

    #[test]
    fn real_host_add_screen_with_a_draft_round_trips_through_validate_reducer_action() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_draft_at(&host, "skit-new-action.py", b"print('draft')\n", 10);
        let mut state = host.initial_state().unwrap();
        let previous = host.observe(&state).unwrap().state;
        let action = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert!(matches!(&action, Action::Present(skit_ui::Screen::Add(_))));
        let canonical_action = host.canonical_action(&action).unwrap();
        let emitted = serde_json::to_value(state.update(action)).unwrap();
        let current = host.observe(&state).unwrap().state;

        super::super::tui_real_walker::validate_reducer_action(
            &previous,
            &current,
            &canonical_action,
            &emitted,
        )
        .unwrap();
    }

    #[test]
    fn checkpoint_capture_projects_the_real_add_delete_chain_in_every_effect_position() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft_path = write_draft_at(
            &host,
            "skit-new-checkpoint-delete.py",
            b"print('draft')\n",
            10,
        );
        let mut state = host.initial_state().unwrap();
        let open = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(open), Effect::None);
        assert_eq!(
            state.update(Action::Add(AddAction::HighlightDraft(0))),
            Effect::None
        );
        assert_eq!(
            state.update(Action::Add(AddAction::DeleteSelectedDraft)),
            Effect::None
        );

        let previous = host.observe(&state).unwrap().state;
        let raw_action = Action::Add(AddAction::ConfirmDraftDelete(true));
        let raw_effect = state.update(raw_action.clone());
        assert!(
            matches!(&raw_effect, Effect::Add(effects) if matches!(effects.as_slice(), [skit_ui::AddEffect::DeleteDraft { .. }]))
        );
        let raw_draft =
            serde_json::to_value(&raw_effect).unwrap()["add"][0]["delete_draft"]["draft"].clone();
        let mut recorded_action = raw_action;
        let mut reducer_emitted = raw_effect.clone();
        let mut reducer_state = state.clone();
        let expected_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let mut reducer_session = expected_session.clone();
        let reducer = host
            .capture_checkpoint_parts(
                &mut reducer_state,
                &mut reducer_session,
                CheckpointCauseProjection::Reducer {
                    action: &mut recorded_action,
                    emitted: &mut reducer_emitted,
                },
            )
            .unwrap();
        assert_eq!(reducer_session, expected_session);
        super::super::tui_real_walker::validate_reducer_action(
            &previous,
            &reducer.state,
            &serde_json::to_value(&recorded_action).unwrap(),
            &serde_json::to_value(&reducer_emitted).unwrap(),
        )
        .unwrap();

        let mut direct_request = Effect::None;
        let mut direct_response = Action::ClearStatus;
        let mut direct_emitted = raw_effect.clone();
        let mut direct_state = state.clone();
        let mut direct_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        host.capture_checkpoint_parts(
            &mut direct_state,
            &mut direct_session,
            CheckpointCauseProjection::Host {
                request: &mut direct_request,
                response: &mut direct_response,
                emitted: &mut direct_emitted,
            },
        )
        .unwrap();
        assert_eq!(direct_emitted, reducer_emitted);

        let raw_response = host.dispatch(raw_effect.clone()).unwrap();
        let raw_host_emitted = state.update(raw_response.clone());
        let mut host_request = raw_effect;
        let mut host_response = raw_response;
        let mut host_emitted = raw_host_emitted;
        let mut host_state = state.clone();
        let mut host_session = c2_session(Path::new("/unregistered/session"), None, Vec::new());
        let host_checkpoint = host
            .capture_checkpoint_parts(
                &mut host_state,
                &mut host_session,
                CheckpointCauseProjection::Host {
                    request: &mut host_request,
                    response: &mut host_response,
                    emitted: &mut host_emitted,
                },
            )
            .unwrap();
        assert_eq!(reducer_emitted, host_request);
        for projected in [&reducer_emitted, &host_request, &direct_emitted] {
            let projected_draft =
                &serde_json::to_value(projected).unwrap()["add"][0]["delete_draft"]["draft"];
            assert_ne!(projected_draft["path"], raw_draft["path"]);
            assert_ne!(projected_draft["modified"], raw_draft["modified"]);
            assert_ne!(projected_draft["identity"], raw_draft["identity"]);
            assert!(
                !projected_draft
                    .to_string()
                    .contains(&draft_path.display().to_string())
            );
        }
        super::super::tui_real_walker::validate_reducer_action(
            &reducer.state,
            &host_checkpoint.state,
            &serde_json::to_value(&host_response).unwrap(),
            &serde_json::to_value(&host_emitted).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn checkpoint_projection_failure_preserves_transcript_until_one_successful_drain() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.clear_transcript();
        host.adapters.push(PortEvent::Output {
            stream: "plain".to_owned(),
            text: "pending checkpoint output".to_owned(),
        });
        let draft = skit_ui::DraftSummary {
            path: host.roots().data.join("unscanned-checkpoint-draft.py"),
            modified: 100,
            identity: None,
            permissions: SourcePermissions::default(),
            content_hash: None,
        };
        let mut workflow = AddWorkflowState::new(vec![draft]);
        assert!(workflow.reduce(AddAction::HighlightDraft(0)).is_empty());
        assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
        let mut emitted = Effect::Add(workflow.reduce(AddAction::ConfirmDraftDelete(true)));
        let mut action = Action::Add(AddAction::ConfirmDraftDelete(true));
        let mut state = host.initial_state().unwrap();
        let mut session = json!({"keep": "byte-exact"});

        assert_eq!(
            host.capture_checkpoint_parts(
                &mut state,
                &mut session,
                CheckpointCauseProjection::Reducer {
                    action: &mut action,
                    emitted: &mut emitted,
                },
            )
            .unwrap_err(),
            "the sorted scan did not assign a rank to this draft modified value: 100"
        );
        assert_eq!(session, json!({"keep": "byte-exact"}));

        let first = host.observe(&host.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(&first.transcript, &["output"]),
            vec![json!({
                "output": "plain",
                "text": "pending checkpoint output",
            })]
        );
        let second = host.observe(&host.initial_state().unwrap()).unwrap();
        assert!(events_with_keys(&second.transcript, &["output"]).is_empty());
    }

    #[test]
    fn checkpoint_registers_pending_event_artifacts_before_projecting_the_cause() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.clear_transcript();
        let allocated = host
            .adapters
            .temporary_file(
                TemporaryFilePurpose::InjectedSource,
                TempLocation::System,
                ".py",
            )
            .unwrap();
        let path = allocated.path().to_path_buf();
        drop(allocated);
        assert!(!host.path_map.transient_paths.contains_key(&path));
        let draft = skit_ui::DraftSummary {
            path: host.roots().data.join("unscanned-order-draft.py"),
            modified: 100,
            identity: None,
            permissions: SourcePermissions::default(),
            content_hash: None,
        };
        let mut workflow = AddWorkflowState::new(vec![draft]);
        assert!(workflow.reduce(AddAction::HighlightDraft(0)).is_empty());
        assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
        let mut action = Action::Add(AddAction::ConfirmDraftDelete(true));
        let mut emitted = Effect::Add(workflow.reduce(AddAction::ConfirmDraftDelete(true)));
        let mut state = host.initial_state().unwrap();
        let mut session = Value::Null;

        assert_eq!(
            host.capture_checkpoint_parts(
                &mut state,
                &mut session,
                CheckpointCauseProjection::Reducer {
                    action: &mut action,
                    emitted: &mut emitted,
                },
            )
            .unwrap_err(),
            "the sorted scan did not assign a rank to this draft modified value: 100"
        );
        assert_eq!(
            host.path_map.transient_paths.get(&path).map(String::as_str),
            Some("<profile:fixture>/system-temp/<injected-source:0>.py")
        );
        let events = host.adapters.events.borrow();
        assert!(matches!(events.first(), Some(PortEvent::Allocation { .. })));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, PortEvent::Clock(_)))
        );
    }

    #[test]
    fn strict_projection_refuses_noncanonical_outer_nested_and_screen_tags() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        for malformed in [
            json!({}),
            json!({"quit": null}),
            json!({"quit": null, "reload": null}),
            json!({"future_action": null}),
            json!({"set_status": "status", "extra": true}),
            json!({"add": "future_add_action"}),
            json!({"settings": {"action": "future_settings_action"}}),
        ] {
            assert!(host.path_map.normalize_action_json(malformed).is_err());
        }
        assert_eq!(
            host.path_map
                .normalize_action_json(json!({
                    "toggle_detail": {"currently_visible": true, "extra": false},
                }))
                .unwrap_err(),
            "a serialized Action is not canonical"
        );
        assert_eq!(
            host.path_map
                .normalize_library_json(json!({
                    "workflow": {"active": {"future_screen": null}, "history": []},
                }))
                .unwrap_err(),
            "unknown Screen tag: future_screen"
        );
        let raw_path = host.roots().data.join("draft-lookalike.py");
        let raw_path = raw_path.display().to_string();
        let normalized = host
            .path_map
            .normalize_library_json(json!({
                "workflow": {
                    "active": {"add": {
                        "notice": {"draft_deleted": raw_path.clone()},
                        "user_text": raw_path,
                    }},
                    "history": [],
                },
            }))
            .unwrap();
        assert_eq!(
            normalized["workflow"]["active"]["add"]["notice"]["draft_deleted"],
            "<profile:fixture>/data/draft-lookalike.py"
        );
        assert_eq!(
            normalized["workflow"]["active"]["add"]["user_text"],
            raw_path
        );

        for (mut screen, expected) in [
            (json!({}), "a serialized external tag object is empty"),
            (
                json!({"library": null, "run": null}),
                "a serialized external tag object has more than one member",
            ),
            (
                json!({"library": null}),
                "serialized tag library must not have a payload",
            ),
            (json!("run"), "serialized tag run must have one payload"),
            (json!(7), "a serialized external tag has a malformed shape"),
            (
                json!({"future_screen": null}),
                "unknown Screen tag: future_screen",
            ),
        ] {
            assert_eq!(
                host.path_map
                    .project_serialized_screen(&mut screen)
                    .unwrap_err(),
                expected
            );
        }
        assert_eq!(
            super::require_internal_tag(&json!({"action": "future"}), "action", "focus_next",)
                .unwrap_err(),
            "expected serialized action tag focus_next, but found future"
        );
        assert_eq!(
            super::require_internal_tag(&json!({"action": 7}), "action", "focus_next").unwrap_err(),
            "serialized action tag focus_next is not text"
        );
        assert_eq!(
            super::require_internal_tag(&json!({}), "action", "focus_next").unwrap_err(),
            "serialized action tag focus_next is missing"
        );
        assert_eq!(
            super::require_external_unit(&json!("cancel"), "continue").unwrap_err(),
            "expected serialized tag continue, but found cancel"
        );
        assert_eq!(
            super::require_external_unit(&json!({"continue": null, "cancel": null}), "continue",)
                .unwrap_err(),
            "serialized tag continue must have exactly one object member"
        );
        assert_eq!(
            super::require_external_unit(&json!(7), "continue").unwrap_err(),
            "serialized tag continue has a malformed shape"
        );
        let mut nested_multi = json!({"continue": null, "cancel": null});
        assert_eq!(
            super::external_payload_mut(&mut nested_multi, "continue").unwrap_err(),
            "serialized tag continue must have exactly one object member"
        );
        assert_eq!(
            host.adapters.drain_event_prefix(usize::MAX).unwrap_err(),
            "the walker port transcript changed during checkpoint capture"
        );
        let mut unrelated = json!("library");
        host.path_map.normalize_run_screen(&mut unrelated).unwrap();
        host.path_map.normalize_settings_screen(&mut unrelated);
        host.path_map.normalize_add_screen(&mut unrelated).unwrap();
        let raw_note_path = host.roots().data.join("note-source.py");
        let mut settings_note = json!({"settings": {"sections": [{"items": [{
            "item": "note",
            "text": "Linked to the original: {}",
            "arguments": [raw_note_path],
        }]}]}});
        host.path_map.normalize_settings_screen(&mut settings_note);
        assert_eq!(
            settings_note["settings"]["sections"][0]["items"][0]["arguments"][0],
            "<profile:fixture>/data/note-source.py"
        );
        let mut note_without_arguments = json!({"settings": {"sections": [{"items": [{
            "item": "note",
            "text": "Linked to the original: {}",
        }]}]}});
        host.path_map
            .normalize_settings_screen(&mut note_without_arguments);

        let nested_failures = [
            (Action::Add(AddAction::Continue), json!({"add": "cancel"})),
            (
                Action::Health(skit_ui::HealthAction::Previous),
                json!({"health": "next"}),
            ),
            (
                Action::Runners(skit_ui::RunnerManagerAction::Previous),
                json!({"runners": "next"}),
            ),
            (
                Action::RunnerEditor(skit_ui::RunnerEditorAction::FocusNext),
                json!({"runner_editor": "cancel"}),
            ),
            (
                Action::Preferences(skit_ui::PreferencesAction::Previous),
                json!({"preferences": "next"}),
            ),
            (
                Action::Settings(skit_ui::SettingsAction::FocusNext),
                json!({"settings": {"action": "close"}}),
            ),
        ];
        for (action, mut raw) in nested_failures {
            assert!(
                host.path_map
                    .project_typed_action_value(&action, &mut raw)
                    .is_err()
            );
        }
        let mut wrong_screen = json!("run");
        assert_eq!(
            host.path_map
                .project_typed_screen(&skit_ui::Screen::Library, &mut wrong_screen)
                .unwrap_err(),
            "expected serialized Screen tag library, but found run"
        );
        let mut wrong_preferences = json!({"preferences": "close"});
        assert!(
            host.path_map
                .project_typed_effect_value(
                    &Effect::Preferences(PreferencesEffect::None),
                    &mut wrong_preferences,
                )
                .is_err()
        );
        let mut wrong_add_length = json!({"add": ["cancel"]});
        assert_eq!(
            host.path_map
                .project_typed_effect_value(&Effect::Add(Vec::new()), &mut wrong_add_length)
                .unwrap_err(),
            "an Add Effect payload changed its length"
        );
    }

    #[test]
    fn serialized_screen_dispatcher_covers_the_typed_nine_variant_vocabulary() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let present = |host: &RealWalkerHost, request, selector: Option<&str>| {
            let action = host
                .dispatch(Effect::Open {
                    request,
                    selector: selector.map(str::to_owned),
                })
                .unwrap();
            let value = serde_json::to_value(action).unwrap();
            serde_json::from_value::<skit_ui::Screen>(value["present"].clone()).unwrap()
        };
        let fixtures = [
            skit_ui::Screen::Library,
            present(&host, HostRequest::Run, Some("Command")),
            present(&host, HostRequest::Preferences, None),
            present(&host, HostRequest::Add, None),
            present(&host, HostRequest::Health, None),
            present(&host, HostRequest::Runners, None),
            present(&host, HostRequest::Settings, Some("Reference")),
            present(&host, HostRequest::Rename, Some("Reference")),
            skit_ui::Screen::Report(skit_ui::ReportView {
                title: "Fixture report".to_owned(),
                items: Vec::new(),
            }),
        ];
        let expected = [
            "library",
            "run",
            "preferences",
            "add",
            "health",
            "runners",
            "settings",
            "form",
            "report",
        ];
        for (screen, expected) in fixtures.iter().zip(expected) {
            assert_eq!(super::screen_tag(screen), expected);
            let mut fixture = serde_json::to_value(screen).unwrap();
            assert_eq!(super::external_tag(&fixture).unwrap(), expected);
            host.path_map
                .project_typed_screen(screen, &mut fixture)
                .unwrap();
            let round_trip: skit_ui::Screen =
                super::deserialize_canonical(fixture, "Screen fixture").unwrap();
            assert_eq!(super::screen_tag(&round_trip), expected);
        }
    }

    #[test]
    fn typed_action_projection_executes_all_seventy_five_outer_variants() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let reload = serde_json::to_value(host.dispatch(Effect::Reload).unwrap()).unwrap();
        let surface = reload["replace_surface"]["surface"].clone();
        let scan = surface["scan"].clone();
        let run = serde_json::to_value(
            host.dispatch(Effect::Open {
                request: HostRequest::Run,
                selector: Some("Command".to_owned()),
            })
            .unwrap(),
        )
        .unwrap();
        let form = run["present"]["run"].clone();
        let preferences = serde_json::to_value(
            host.dispatch(Effect::Open {
                request: HostRequest::Preferences,
                selector: None,
            })
            .unwrap(),
        )
        .unwrap()["present"]["preferences"]
            .clone();
        let fixtures = vec![
            json!("previous"),
            json!("next"),
            json!("page_previous"),
            json!("page_next"),
            json!("home"),
            json!("end"),
            json!({"select_visible": 0}),
            json!("begin_search"),
            json!("finish_search"),
            json!({"input": "x"}),
            json!("backspace"),
            json!({"paste": "paste"}),
            json!({"set_search_query": "query"}),
            json!("clear_search"),
            json!({"replace": {"scan": scan, "rerunnable": []}}),
            reload,
            json!({"replace_rerunnable": []}),
            json!("reload"),
            json!("rerun"),
            json!("open_run"),
            json!("open_add"),
            json!("open_settings"),
            json!("open_preferences"),
            json!("open_health"),
            json!("open_runners"),
            json!("open_presets"),
            json!("open_rename"),
            json!("edit"),
            json!("ask_remove"),
            json!("open_help"),
            json!({"toggle_detail": {"currently_visible": true}}),
            json!("focus_next"),
            json!("focus_previous"),
            json!({"focus_field": 0}),
            json!({"set_field_value": {"field": 0, "value": "value"}}),
            json!({"set_run_glob_count": {"field": 0, "value": "*", "count": 1}}),
            json!("open_run_preset_save"),
            json!("open_run_token_menu"),
            json!({"open_run_token_menu_for": 0}),
            json!({"open_run_environment_picker": 0}),
            json!({"set_run_environment_query": "PATH"}),
            json!({"open_run_file_picker": 0}),
            json!("open_focused_run_file_picker"),
            json!({"set_run_field_value_and_close_modal": {"field": 0, "value": "value"}}),
            json!({"set_run_picked_path_and_close_modal": {"field": 0, "path": "path"}}),
            json!("reset_focused_run_field"),
            json!("open_run_runner_editor"),
            json!({"add": "continue"}),
            json!("open_add_runner_editor"),
            json!({"health": "previous"}),
            json!({"runners": "previous"}),
            json!({"runner_editor": "focus_next"}),
            json!({"runner_editor_saved": {"owner": "add", "name": "runner", "message": "saved"}}),
            json!({"runner_editor_save_failed": {"owner": "add", "message": "failed"}}),
            json!({"runner_manager_closed": {"preferences": preferences}}),
            json!({"preferences": "previous"}),
            json!({"settings": {"action": "focus_next"}}),
            json!({"preferences_saved": {"locale": "en", "message": "saved"}}),
            json!("keep_editing"),
            json!("discard_changes"),
            json!({"set_modal_input": "value"}),
            json!({"run_preset_saved": {"name": "preset", "presets": {}, "message": "saved"}}),
            json!({"toggle_field": 0}),
            json!({"select_field_option": {"field": 0, "value": "choice"}}),
            json!({"reset_run_field": 0}),
            json!("submit"),
            json!("back"),
            json!({"present": "library"}),
            json!({"prompt_runner_required": {"form": form, "cancel_status": "cancelled"}}),
            json!({"add_completed": {"surface": surface, "rerunnable": [], "slug": "created", "message": "added"}}),
            json!("add_cancelled"),
            json!({"complete": {"surface": null, "rerunnable": null, "message": "complete"}}),
            json!({"set_status": "status"}),
            json!("clear_status"),
            json!("quit"),
        ];
        assert_eq!(fixtures.len(), 75);
        for fixture in fixtures {
            let projected = host.path_map.normalize_action_json(fixture).unwrap();
            let _: Action = super::deserialize_canonical(projected, "Action fixture").unwrap();
        }
    }

    #[test]
    fn typed_nested_action_projection_executes_every_recorded_variant() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let mut add = vec![
            json!({"set_source_path": "source"}),
            json!({"picked_source_path": host.external_root.join("outside.sh").display().to_string()}),
            json!({"set_command_template": "echo {value}"}),
            json!({"set_command_name": "command"}),
            json!({"set_command_description": "description"}),
            json!({"select_draft": 0}),
            json!({"highlight_draft": 0}),
            json!("continue"),
            json!({"source_inspected": {"request": 0, "result": {"Err": "failed"}}}),
            json!({"pick_kind": null}),
            json!({"new_draft": "script"}),
            json!({"draft_edited": {"request": 0, "result": {"Err": "failed"}}}),
            json!("delete_selected_draft"),
            json!({"confirm_draft_delete": true}),
            json!({"draft_deleted": {"request": 0, "result": {"Err": "failed"}}}),
            json!({"set_review_name": "name"}),
            json!({"set_review_description": "description"}),
            json!({"set_review_storage": "copy"}),
            json!({"set_review_dependencies": "dep"}),
            json!({"set_review_python": ">=3.12"}),
            json!({"set_review_candidate": {"name": "candidate", "selected": true}}),
            json!({"set_prompt_interpolation": true}),
            json!({"set_prompt_candidate": {"name": "value", "selected": true}}),
            json!({"set_prompt_candidates": ["value"]}),
            json!({"set_prompt_runner": {"name": "runner", "picked": true}}),
            json!({"prompt_runner_added": "runner"}),
            json!("edit_source"),
            json!({"source_edited": {"request": 0, "result": {"Err": "failed"}}}),
            json!("save"),
            json!({"commit_finished": {"request": 0, "result": {"Err": "failed"}}}),
            json!("cancel"),
        ];
        assert_eq!(add.len(), 31);
        let health_rebuilt = serde_json::to_value(host.dispatch(Effect::HealthRebuild).unwrap())
            .unwrap()["health"]
            .clone();
        let health = vec![
            json!("previous"),
            json!("next"),
            json!({"page_previous": 2}),
            json!({"page_next": 2}),
            json!("home"),
            json!("end"),
            json!({"select_issue": 0}),
            json!("jump"),
            json!({"activate_issue": 0}),
            json!("rebuild"),
            health_rebuilt,
            json!("back"),
        ];
        assert_eq!(health.len(), 12);
        let preferences = vec![
            json!({"set_language": "en"}),
            json!({"set_editor": "editor"}),
            json!({"set_interactive_form": "tui"}),
            json!({"set_after_run": "stay"}),
            json!({"set_javascript": "automatic"}),
            json!({"set_bash_path": "bash"}),
            json!({"set_mirror_master": true}),
            json!({"choose_mirror": {"field": "pypi_mirror", "choice": "off"}}),
            json!({"set_mirror_url": {"field": "pypi_mirror", "value": "https://example.invalid"}}),
            json!({"focus": "language"}),
            json!("previous"),
            json!("next"),
            json!("save"),
            json!("close"),
            json!("manage_agents"),
            json!("install_agent_skill"),
            json!({"present_agent_skill_targets": [{"name": "codex", "scope": "user", "base": "/user/base"}]}),
            json!({"select_agent_skill_target": 0}),
            json!({"activate_agent_skill_target": 0}),
            json!("confirm_agent_skill_target"),
            json!("close_agent_skill_targets"),
            json!({"agent_skill_installed": {"message": "installed"}}),
            json!({"validation_failed": {"bash_path_missing": {"path": "missing"}}}),
        ];
        assert_eq!(preferences.len(), 23);
        let runner_editor = vec![
            json!({"set_name": "name"}),
            json!({"set_command": "runner --flag"}),
            json!({"focus": "name"}),
            json!("focus_next"),
            json!("focus_previous"),
            json!("submit"),
            json!("cancel"),
            json!({"mutation_failed": "failed"}),
        ];
        assert_eq!(runner_editor.len(), 8);
        let runners = vec![
            json!("previous"),
            json!("next"),
            json!({"page_previous": 2}),
            json!({"page_next": 2}),
            json!("home"),
            json!("end"),
            json!({"select": 0}),
            json!("activate_selected"),
            json!({"activate_row": 0}),
            json!("new"),
            json!("edit_selected"),
            json!("remove_selected"),
            json!("close_actions"),
            json!({"editor": "focus_next"}),
            json!("cancel_editor"),
            json!("confirm_remove"),
            json!("cancel_remove"),
            json!({"mutation_succeeded": {"rows": [], "selected_name": null, "message": "saved"}}),
            json!({"mutation_failed": "failed"}),
            json!("back"),
        ];
        assert_eq!(runners.len(), 20);
        let settings = vec![
            json!({"action": "set_field", "key": "name", "value": {"state": "inherit"}}),
            json!({"action": "focus", "key": "name"}),
            json!({"action": "focus_next"}),
            json!({"action": "focus_previous"}),
            json!({"action": "resync"}),
            json!({"action": "set_prompt_candidates", "selected": ["value"]}),
            json!({"action": "save"}),
            json!({"action": "close"}),
            json!({"action": "new_runner"}),
        ];
        assert_eq!(settings.len(), 9);

        for nested in add.drain(..) {
            host.path_map
                .normalize_action_json(json!({"add": nested}))
                .unwrap();
        }
        for nested in health {
            host.path_map
                .normalize_action_json(json!({"health": nested}))
                .unwrap();
        }
        for nested in preferences {
            host.path_map
                .normalize_action_json(json!({"preferences": nested}))
                .unwrap();
        }
        for nested in runner_editor {
            host.path_map
                .normalize_action_json(json!({"runner_editor": nested}))
                .unwrap();
        }
        for nested in runners {
            host.path_map
                .normalize_action_json(json!({"runners": nested}))
                .unwrap();
        }
        for nested in settings {
            host.path_map
                .normalize_action_json(json!({"settings": nested}))
                .unwrap();
        }
        let source = json!({
            "path": "outside-source.py",
            "source_record": "outside-source.py",
            "bytes": [],
            "permissions": serde_json::to_value(SourcePermissions::default()).unwrap(),
            "executable": null,
            "is_regular": true,
            "is_directory": false,
            "is_draft": false,
            "identity": null,
        });
        let draft = json!({
            "path": "outside-draft.py",
            "modified": 1,
            "identity": null,
            "permissions": serde_json::to_value(SourcePermissions::default()).unwrap(),
            "content_hash": null,
        });
        for nested in [
            json!({"source_inspected": {"request": 0, "result": {"Ok": source.clone()}}}),
            json!({"draft_edited": {"request": 0, "result": {"Ok": source.clone()}}}),
            json!({"draft_deleted": {"request": 0, "result": {"Ok": {"changed": draft}}}}),
            json!({"source_edited": {"request": 0, "result": {"Ok": source.clone()}}}),
        ] {
            host.path_map
                .normalize_action_json(json!({"add": nested}))
                .unwrap();
        }
    }

    #[test]
    fn typed_effect_projection_executes_all_outer_and_recorded_nested_variants() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let outer = vec![
            json!("none"),
            json!("reload"),
            json!("quit"),
            json!({"rerun": {"selector": "entry"}}),
            json!({"open": {"request": "run", "selector": "entry"}}),
            json!({"submit": {"purpose": "run", "selector": "entry", "values": {}}}),
            json!({"count_run_glob": {"selector": "entry", "field": 0, "value": "*", "request": {"cwd": "/cwd", "pieces": ["*"]}}}),
            json!({"save_run_preset": {"selector": "entry", "name": "preset", "values": {}, "secret_names": []}}),
            json!({"add": ["cancel"]}),
            json!("health_rebuild"),
            json!({"save_runner": {"request": {"name": "runner", "argv": ["runner"], "target": "new"}, "owner": "manager"}}),
            json!({"remove_runner": {"named": {"name": "runner", "expected": [], "expected_pinned_count": 0}}}),
            json!("refresh_preferences_after_runners"),
            json!({"preferences": "none"}),
            json!({"edit": {"selector": "entry"}}),
            json!({"remove": {"selector": "entry"}}),
        ];
        assert_eq!(outer.len(), 16);
        for raw in outer {
            let mut effect: Effect = super::deserialize_canonical(raw, "Effect fixture").unwrap();
            host.path_map.project_typed_effect(&mut effect).unwrap();
            let projected = serde_json::to_value(&effect).unwrap();
            let _: Effect = super::deserialize_canonical(projected, "Effect fixture").unwrap();
        }

        for nested in [
            json!("none"),
            json!({"save": {"settings": {}}}),
            json!("close"),
            json!("confirm_discard"),
            json!("manage_agents"),
            json!("discover_agent_skill_targets"),
            json!({"install_agent_skill": {"skills_dir": "/skills"}}),
        ] {
            let raw = json!({"preferences": nested});
            let mut effect: Effect = super::deserialize_canonical(raw, "Effect fixture").unwrap();
            host.path_map.project_typed_effect(&mut effect).unwrap();
        }

        let draft_path = write_draft_at(
            &host,
            "skit-new-effect-fixture.py",
            b"print('effect')\n",
            10,
        );
        let state = host.initial_state().unwrap();
        host.observe(&state).unwrap();
        let draft = super::sorted_tui_drafts(&host.roots().data)
            .into_iter()
            .next()
            .unwrap();
        let mut workflow = AddWorkflowState::new(vec![draft]);
        assert!(workflow.reduce(AddAction::HighlightDraft(0)).is_empty());
        assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
        let delete = workflow
            .reduce(AddAction::ConfirmDraftDelete(true))
            .pop()
            .unwrap();
        let source = SourceSnapshot {
            path: PathBuf::from("source.py"),
            source_record: "source.py".to_owned(),
            bytes: b"print('source')\n".to_vec(),
            permissions: SourcePermissions::default(),
            executable: Some(false),
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        };
        let entry = serde_json::to_value(profile().entries[0].clone()).unwrap();
        let source_json = serde_json::to_value(&source).unwrap();
        let mut nested = vec![
            super::deserialize_canonical::<skit_ui::AddEffect>(
                json!({"inspect_source": {"request": 0, "path": "source.py"}}),
                "AddEffect fixture",
            )
            .unwrap(),
            super::deserialize_canonical::<skit_ui::AddEffect>(
                json!({"author_draft": {"request": 0, "kind": "script"}}),
                "AddEffect fixture",
            )
            .unwrap(),
            delete,
            super::deserialize_canonical::<skit_ui::AddEffect>(
                json!({"edit_source": {"request": 0, "path": "source.py"}}),
                "AddEffect fixture",
            )
            .unwrap(),
            super::deserialize_canonical::<skit_ui::AddEffect>(
                json!({"commit": {"request": 0, "entry": entry, "source": null}}),
                "AddEffect fixture",
            )
            .unwrap(),
            skit_ui::AddEffect::ConsumeDraft(source),
            skit_ui::AddEffect::DraftKept(PathBuf::from("draft.py")),
            skit_ui::AddEffect::RememberRunner("runner".to_owned()),
            skit_ui::AddEffect::Complete("created".to_owned()),
            skit_ui::AddEffect::Cancel,
        ];
        assert_eq!(nested.len(), 10);
        let mut effect = Effect::Add(std::mem::take(&mut nested));
        host.path_map.project_typed_effect(&mut effect).unwrap();
        let projected = serde_json::to_value(&effect).unwrap();
        assert_ne!(
            projected["add"][2]["delete_draft"]["draft"]["path"],
            draft_path.display().to_string()
        );
        assert_eq!(
            serde_json::to_value(&effect).unwrap()["add"][5]["consume_draft"],
            source_json
        );
    }

    #[test]
    fn real_host_refuses_an_unscanned_draft_in_state_and_action() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = skit_ui::DraftSummary {
            path: host.roots().data.join("unscanned-draft.py"),
            modified: 100,
            identity: None,
            permissions: SourcePermissions::default(),
            content_hash: None,
        };
        let action = Action::Present(skit_ui::Screen::Add(Box::new(AddWorkflowState::new(vec![
            draft,
        ]))));
        assert_eq!(
            host.canonical_action(&action).unwrap_err(),
            "the sorted scan did not assign a rank to this draft modified value: 100"
        );

        let mut state = host.initial_state().unwrap();
        assert_eq!(state.update(action), Effect::None);
        assert_eq!(
            host.observe(&state).unwrap_err(),
            "the sorted scan did not assign a rank to this draft modified value: 100"
        );
    }

    #[test]
    fn real_host_refuses_drafts_with_equal_modified_times() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_draft_at(&host, "skit-new-a.py", b"a\n", 10);
        write_draft_at(&host, "skit-new-b.py", b"b\n", 10);

        assert_eq!(
            host.observe(&host.initial_state().unwrap()).map(|_| ()),
            Err("rank scan contains duplicate value 10000000000".to_owned())
        );
    }

    #[test]
    fn real_host_refuses_a_new_draft_with_an_older_modified_time() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_draft_at(&host, "skit-new-later.py", b"later\n", 20);
        let state = host.initial_state().unwrap();
        host.observe(&state).unwrap();
        write_draft_at(&host, "skit-new-earlier.py", b"earlier\n", 10);

        assert_eq!(
            host.observe(&state).map(|_| ()),
            Err(
                "rank value 10000000000 does not exceed the current maximum 20000000000".to_owned()
            )
        );
    }

    #[test]
    fn real_host_two_drafts_use_sorted_paths_and_ascending_modified_ranks() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        write_draft_at(&host, "skit-new-z.py", b"z\n", 20);
        write_draft_at(&host, "skit-new-a.py", b"a\n", 10);
        let mut state = host.initial_state().unwrap();

        let observation = host.observe(&state).unwrap();
        assert_eq!(
            observation
                .drafts
                .as_array()
                .unwrap()
                .iter()
                .map(|draft| (&draft["path"], &draft["modified"]))
                .collect::<Vec<_>>(),
            vec![
                (
                    &json!("<profile:fixture>/data/.drafts/<draft:0>.py"),
                    &json!(0)
                ),
                (
                    &json!("<profile:fixture>/data/.drafts/<draft:1>.py"),
                    &json!(1)
                ),
            ]
        );

        let action = host
            .dispatch(Effect::Open {
                request: HostRequest::Add,
                selector: None,
            })
            .unwrap();
        assert_eq!(state.update(action), Effect::None);
        let state = host.observe(&state).unwrap().state;
        let drafts = state
            .pointer("/workflow/active/add/source/drafts")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            drafts
                .iter()
                .map(|draft| (&draft["path"], &draft["modified"]))
                .collect::<Vec<_>>(),
            vec![
                (
                    &json!("<profile:fixture>/data/.drafts/<draft:1>.py"),
                    &json!(1)
                ),
                (
                    &json!("<profile:fixture>/data/.drafts/<draft:0>.py"),
                    &json!(0)
                ),
            ]
        );
    }

    #[test]
    fn real_host_copy_mode_settings_normalizes_the_source_path() {
        let mut spec = profile();
        let mut request = spec.external_references[0].request.clone();
        request.name = "Copied source".to_owned();
        request.mode = StorageMode::Copy;
        spec.external_references.push(WalkerExternalReferenceSeed {
            request,
            source: PathBuf::from("outside.sh"),
        });
        let mut host = RealWalkerHost::spawn(spec).unwrap();
        let mut state = host.initial_state().unwrap();
        let action = host
            .dispatch(Effect::Open {
                request: HostRequest::Settings,
                selector: Some("Copied source".to_owned()),
            })
            .unwrap();
        assert!(matches!(
            &action,
            Action::Present(skit_ui::Screen::Settings(_))
        ));
        assert_eq!(state.update(action), Effect::None);

        let value = host.observe(&state).unwrap().state;
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(!encoded.contains(&host.external_root.display().to_string()));
        assert!(encoded.contains("<profile:fixture>/external/outside.sh"));
        let parsed = super::super::tui_real_walker::parse_library_state(&value).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }

    #[test]
    fn draft_paths_timestamps_and_source_identities_are_artifact_only_values() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        for (host, name) in [
            (&first, "skit-new-random-a.py"),
            (&second, "skit-new-random-b.py"),
        ] {
            let drafts = super::super::create_owned_drafts_dir(&host.roots().data).unwrap();
            fs::write(drafts.join(name), b"print('draft')\n").unwrap();
        }

        let first_observation = first.observe(&first.initial_state().unwrap()).unwrap();
        let second_observation = second.observe(&second.initial_state().unwrap()).unwrap();

        assert_eq!(first_observation.drafts, second_observation.drafts);
        assert_eq!(
            first_observation
                .tree
                .iter()
                .filter(|row| row.path.contains("<draft:"))
                .collect::<Vec<_>>(),
            second_observation
                .tree
                .iter()
                .filter(|row| row.path.contains("<draft:"))
                .collect::<Vec<_>>()
        );
        assert_identity_sentinel(&first_observation.drafts[0]["identity"], 0, 0);
        assert_eq!(first_observation.drafts[0]["modified"], 0);
        let encoded = first_observation.drafts.to_string();
        assert!(encoded.contains("<draft:0>.py"));
        assert!(!encoded.contains("skit-new-random-a"));
        let raw_draft = super::sorted_tui_drafts(first.service.repository().data_dir())
            .into_iter()
            .next()
            .unwrap();
        let tree_draft = first_observation
            .tree
            .iter()
            .find(|row| row.path == "data/.drafts/<draft:0>.py")
            .unwrap();
        assert_eq!(
            tree_draft.mode,
            super::portable_mode(&fs::symlink_metadata(raw_draft.path).unwrap())
        );
    }

    #[test]
    fn removed_authored_draft_keeps_a_stable_editor_transcript_path() {
        let mut first = RealWalkerHost::spawn(profile()).unwrap();
        let mut second = RealWalkerHost::spawn(profile()).unwrap();
        let author = |host: &mut RealWalkerHost| {
            let mut workflow = AddWorkflowState::new(Vec::new());
            host.clear_transcript();
            let action = host
                .dispatch(Effect::Add(
                    workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
                ))
                .unwrap();
            assert!(matches!(
                action,
                Action::Add(AddAction::DraftEdited {
                    result: Ok(None),
                    ..
                })
            ));
            host.observe(&host.initial_state().unwrap()).unwrap()
        };

        let first_observation = author(&mut first);
        let second_observation = author(&mut second);
        assert_eq!(first_observation, second_observation);
        let encoded = serde_json::to_string(&first_observation.transcript).unwrap();
        assert!(encoded.contains("<draft:0>.py"));
        assert!(!encoded.contains("skit-new-"));
        assert!(!encoded.contains(&first._sandbox.path().display().to_string()));
        assert!(!encoded.contains(&second._sandbox.path().display().to_string()));
    }

    #[test]
    fn queued_editor_write_records_its_filesystem_error() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        host.adapters
            .editor_write_queue
            .borrow_mut()
            .push_back(b"edited".to_vec());
        host.clear_transcript();
        let result = super::EditorLauncher::launch(
            &host.adapters,
            &["editor".to_owned()],
            host._sandbox.path(),
        );
        let error = result.unwrap_err();
        let events = host.adapters.events.borrow();
        let expected = serde_json::json!({
            "error": {"kind": format!("{:?}", error.kind()), "reason": error.to_string()}
        });
        assert!(matches!(
            events.last(),
            Some(super::PortEvent::Editor { outcome, .. }) if outcome == &expected
        ));
    }

    #[test]
    fn authored_draft_editor_write_keeps_the_deterministic_private_file() {
        #[cfg(unix)]
        use std::os::unix::fs::PermissionsExt as _;

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.adapters
            .editor_script
            .replace(super::EditorScript::Write(b"print('kept')\n".to_vec()));
        let mut workflow = AddWorkflowState::new(Vec::new());
        host.clear_transcript();

        let action = host
            .dispatch(Effect::Add(
                workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
            ))
            .unwrap();
        let source = expect_authored_draft_source(action);

        assert_eq!(source.path.file_name().unwrap(), "skit-new-000000.py");
        assert_eq!(fs::read(&source.path).unwrap(), b"print('kept')\n");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&source.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        let draft = observation
            .tree
            .iter()
            .find(|row| row.path == "data/.drafts/<draft:0>.py")
            .unwrap();
        #[cfg(unix)]
        assert_eq!(draft.mode, Some(0o600));
        #[cfg(not(unix))]
        assert_eq!(draft.mode, None);
        let mut ordered = Vec::new();
        for event in &observation.transcript {
            if event.get("allocation").is_some() || event.get("editor").is_some() {
                ordered.push(event);
            }
        }
        assert_eq!(ordered.len(), 2);
        assert_eq!(
            ordered[0]["allocation"]["path"],
            "<profile:fixture>/data/.drafts/<draft:0>.py"
        );
        assert_eq!(
            ordered[1]["path"],
            "<profile:fixture>/data/.drafts/<draft:0>.py"
        );
    }

    #[test]
    fn changed_draft_restore_removes_its_deterministic_quarantine() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.adapters
            .editor_script
            .replace(super::EditorScript::Write(b"print('before')\n".to_vec()));
        let mut workflow = AddWorkflowState::new(Vec::new());
        let action = host
            .dispatch(Effect::Add(
                workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
            ))
            .unwrap();
        let source = expect_authored_draft_source(action);
        fs::write(&source.path, b"print('changed')\n").unwrap();
        host.clear_transcript();

        let outcome = super::super::consume_owned_draft_with(
            &host.roots().data,
            &source,
            &host.adapters,
            |_, _| {},
        )
        .unwrap();

        assert_eq!(outcome, super::super::DraftConsumeOutcome::Changed);
        assert_eq!(fs::read(&source.path).unwrap(), b"print('changed')\n");
        let drafts = source.path.parent().unwrap();
        assert!(fs::read_dir(drafts).unwrap().all(|item| {
            !item
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".skit-quarantine-")
        }));
        let transcript = host.adapters.transcript(&mut host.path_map);
        assert_eq!(
            events_with_keys(&transcript, &["allocation"]),
            serde_json::from_str::<Vec<Value>>(
                r#"[{
                    "allocation": {
                        "purpose": "draft_quarantine",
                        "attempt": 0,
                        "location": {
                            "kind": "directory",
                            "path": "<profile:fixture>/data/drafts"
                        },
                        "path": "<profile:fixture>/data/drafts/<quarantine:0>",
                        "outcome": {"accepted": true}
                    }
                }]"#,
            )
            .unwrap()
        );
    }

    #[test]
    fn failed_changed_draft_restore_retains_the_deterministic_quarantine() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        host.adapters
            .editor_script
            .replace(super::EditorScript::Write(b"print('before')\n".to_vec()));
        let mut workflow = AddWorkflowState::new(Vec::new());
        let action = host
            .dispatch(Effect::Add(
                workflow.reduce(AddAction::NewDraft(DraftKind::Script)),
            ))
            .unwrap();
        let source = expect_authored_draft_source(action);
        fs::write(&source.path, b"print('changed')\n").unwrap();
        host.clear_transcript();

        let error = super::super::consume_owned_draft_with(
            &host.roots().data,
            &source,
            &host.adapters,
            |point, _| {
                if point == super::super::DraftConsumeTestPoint::BeforeRestore {
                    fs::write(&source.path, b"replacement").unwrap();
                }
            },
        )
        .unwrap_err();
        let quarantine = source
            .path
            .parent()
            .unwrap()
            .join(".skit-quarantine-000000/draft");

        assert!(
            error
                .to_string()
                .contains("could not restore quarantined draft")
        );
        assert_eq!(fs::read(&source.path).unwrap(), b"replacement");
        assert_eq!(fs::read(quarantine).unwrap(), b"print('changed')\n");
        let transcript = host.adapters.transcript(&mut host.path_map);
        assert_eq!(
            events_with_keys(&transcript, &["allocation"]),
            vec![json!({ "allocation": {
                "purpose": "draft_quarantine",
                "attempt": 0,
                "location": {
                    "kind": "directory",
                    "path": "<profile:fixture>/data/drafts",
                },
                "path": "<profile:fixture>/data/drafts/<quarantine:0>",
                "outcome": { "accepted": true },
            }})]
        );
    }

    /// Each accepted allocator boundary enters the path map with one typed sentinel.
    ///
    /// The three boundaries are the authored draft, the draft quarantine directory, and the
    /// injected source. The leak oracle refuses every raw allocator
    /// name, so a boundary without a sentinel cannot reach an installed corpus.
    #[test]
    fn every_accepted_allocation_purpose_projects_one_typed_sentinel() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let data = host.roots().data.clone();
        let boundaries = [
            (
                super::AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft),
                data.join("drafts/skit-new-0000ab.py"),
                "<profile:fixture>/data/.drafts/<draft:0>.py",
            ),
            (
                super::AllocationPurpose::PrivateDirectory(
                    PrivateDirectoryPurpose::DraftQuarantine,
                ),
                data.join("drafts/.skit-quarantine-0000cd"),
                "<profile:fixture>/data/drafts/<quarantine:0>",
            ),
            (
                super::AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource),
                host.roots().state.join(".injected-0000ef.sh"),
                "<profile:fixture>/state/<injected-source:1>.sh",
            ),
        ];

        for (purpose, path, expected) in &boundaries {
            host.path_map
                .register_event_artifacts(&PortEvent::Allocation {
                    purpose: *purpose,
                    attempt: 0,
                    location: super::AllocationLocation::Directory(
                        path.parent().unwrap().to_path_buf(),
                    ),
                    path: Some(path.clone()),
                    outcome: super::AllocationOutcome::Accepted,
                });
            assert_eq!(host.path_map.normalize_path(path), *expected);
            assert_eq!(
                host.path_map
                    .normalize_host_text(&path.display().to_string()),
                *expected
            );
        }
    }

    #[test]
    fn prompt_edit_drives_editor_terminal_output_and_raw_editor_failure() {
        let mut spec = profile();
        spec.settings
            .insert("editor".to_owned(), "walker-editor --wait".to_owned());
        spec.external.push(WalkerExternalSeed::File {
            path: PathBuf::from("prompt.md"),
            bytes: b"Review {{fresh}}\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        });
        spec.external_references.push(WalkerExternalReferenceSeed {
            request: CreateEntry {
                name: "Editable prompt".to_owned(),
                kind: EntryKind::parse("prompt").unwrap(),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"Review {{fresh}}\n".to_vec(),
                    stored_name: None,
                    permissions: SourcePermissions::default(),
                }),
                settings: EntrySettings {
                    interpolate: true,
                    ..EntrySettings::default()
                },
            },
            source: PathBuf::from("prompt.md"),
        });
        let mut success = RealWalkerHost::spawn(spec.clone()).unwrap();
        success.adapters.stdin_terminal.set(true);
        success.adapters.stdout_terminal.set(false);
        success.clear_transcript();

        assert!(matches!(
            success
                .dispatch(Effect::Edit {
                    selector: "Editable prompt".to_owned(),
                })
                .unwrap(),
            Action::Complete { .. }
        ));
        let observation = success.observe(&success.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(
                &observation.transcript,
                &["environment_variable", "output", "editor", "terminal"]
            ),
            vec![
                json!({ "environment_variable": "VISUAL", "result": null }),
                json!({ "environment_variable": "EDITOR", "result": null }),
                json!({
                    "output": "detail",
                    "text": "Editing the original file (reference mode): <profile:fixture>/external/prompt.md",
                }),
                json!({
                    "editor": ["walker-editor", "--wait"],
                    "path": "<profile:fixture>/external/prompt.md",
                    "outcome": { "ok": true },
                }),
                json!({ "output": "success", "text": "Saved Editable prompt." }),
                json!({ "terminal": "stdin", "result": true }),
                json!({ "terminal": "stdout", "result": false }),
                json!({
                    "output": "plain",
                    "text": "Detected but not yet managed: fresh (use --add to manage them)",
                }),
            ]
        );

        let mut failure = RealWalkerHost::spawn(spec).unwrap();
        let source = failure.external_root.join("prompt.md");
        let before = fs::read(&source).unwrap();
        failure
            .adapters
            .editor_script
            .replace(super::EditorScript::Failure {
                kind: io::ErrorKind::PermissionDenied,
                reason: "walker editor denied".to_owned(),
            });
        failure.clear_transcript();
        assert!(
            failure
                .dispatch(Effect::Edit {
                    selector: "Editable prompt".to_owned(),
                })
                .unwrap_err()
                .to_string()
                .contains("walker editor denied")
        );
        assert_eq!(fs::read(&source).unwrap(), before);
        let failed = failure.observe(&failure.initial_state().unwrap()).unwrap();
        assert_eq!(
            events_with_keys(
                &failed.transcript,
                &["environment_variable", "output", "editor", "terminal"]
            ),
            vec![
                json!({ "environment_variable": "VISUAL", "result": null }),
                json!({ "environment_variable": "EDITOR", "result": null }),
                json!({
                    "output": "detail",
                    "text": "Editing the original file (reference mode): <profile:fixture>/external/prompt.md",
                }),
                json!({
                    "editor": ["walker-editor", "--wait"],
                    "path": "<profile:fixture>/external/prompt.md",
                    "outcome": { "error": {
                        "kind": "PermissionDenied",
                        "reason": "walker editor denied",
                    }},
                }),
            ]
        );
    }

    fn directory_seed(root: WalkerDirectoryRoot, path: &str) -> WalkerDirectorySeed {
        WalkerDirectorySeed {
            root,
            path: PathBuf::from(path),
        }
    }

    #[test]
    fn g2_profile_directory_seed_duplicates_and_overlaps_are_idempotent_private_and_discoverable() {
        let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
            profile: "g2-directories".to_owned(),
            directories: vec![
                directory_seed(WalkerDirectoryRoot::Home, ".codex"),
                directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
                directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
                directory_seed(WalkerDirectoryRoot::Cwd, ".claude/skills"),
            ],
            ..WalkerSeedSpec::default()
        })
        .unwrap();

        let home = host.roots().home.as_ref().unwrap().join(".codex");
        let cwd = host.roots().cwd.join(".claude");
        assert!(home.join("skills").is_dir());
        assert!(cwd.join("skills").is_dir());
        let action = host
            .dispatch(Effect::Preferences(
                PreferencesEffect::DiscoverAgentSkillTargets,
            ))
            .unwrap();
        assert!(matches!(
            &action,
            Action::Preferences(PreferencesAction::PresentAgentSkillTargets(_))
        ));
        let action = serde_json::to_value(action).unwrap();
        let targets = action["preferences"]["present_agent_skill_targets"]
            .as_array()
            .unwrap();
        assert!(targets.iter().any(|target| {
            target["name"] == "codex" && target["scope"] == "user" && target["base"] == json!(home)
        }));
        assert!(targets.iter().any(|target| {
            target["name"] == "claude"
                && target["scope"] == "project"
                && target["base"] == json!(cwd)
        }));

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();
        for relative in [
            "home/.codex",
            "home/.codex/skills",
            "cwd/.claude",
            "cwd/.claude/skills",
        ] {
            let row = observation
                .tree
                .iter()
                .find(|row| row.path == relative)
                .unwrap();
            assert_eq!(row.kind, "directory");
            #[cfg(unix)]
            assert_eq!(row.mode, Some(0o700));
            #[cfg(not(unix))]
            assert_eq!(row.mode, None);
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum G2DirectorySeedFault {
        FileConflict,
        #[cfg(unix)]
        SymlinkConflict,
        Create,
        Inspect,
        PrivateMode,
    }

    #[derive(Debug)]
    struct G2DirectorySeedIo {
        fault: Cell<Option<G2DirectorySeedFault>>,
        attempts: RefCell<Vec<PathBuf>>,
        symlink_target: PathBuf,
    }

    impl G2DirectorySeedIo {
        fn new(fault: G2DirectorySeedFault, symlink_target: PathBuf) -> Self {
            Self {
                fault: Cell::new(Some(fault)),
                attempts: RefCell::new(Vec::new()),
                symlink_target,
            }
        }

        fn take(&self, fault: G2DirectorySeedFault) -> bool {
            if self.fault.get() != Some(fault) {
                return false;
            }
            self.fault.set(None);
            true
        }
    }

    impl super::DirectorySeedIo for G2DirectorySeedIo {
        fn create_dir(&self, path: &Path) -> io::Result<()> {
            self.attempts.borrow_mut().push(path.to_path_buf());
            if self.take(G2DirectorySeedFault::FileConflict) {
                fs::write(path, b"seed conflict")?;
            }
            #[cfg(unix)]
            if self.take(G2DirectorySeedFault::SymlinkConflict) {
                std::os::unix::fs::symlink(&self.symlink_target, path)?;
            }
            if self.take(G2DirectorySeedFault::Create) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected directory seed create failure",
                ));
            }
            if self.fault.get() == Some(G2DirectorySeedFault::Inspect) {
                return Err(io::Error::from(io::ErrorKind::AlreadyExists));
            }
            fs::create_dir(path)
        }

        fn symlink_metadata(&self, path: &Path) -> io::Result<fs::Metadata> {
            if self.take(G2DirectorySeedFault::Inspect) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "injected directory seed inspect failure",
                ));
            }
            fs::symlink_metadata(path)
        }

        fn set_private_directory_mode(&self, path: &Path) -> io::Result<()> {
            assert!(self.take(G2DirectorySeedFault::PrivateMode));
            let _ = path;
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected directory seed private-mode failure",
            ))
        }
    }

    fn g2_directory_seed_fault_cases() -> Vec<(G2DirectorySeedFault, &'static str)> {
        let mut cases = vec![
            (G2DirectorySeedFault::FileConflict, "is not a directory"),
            (G2DirectorySeedFault::Create, "could not create"),
            (G2DirectorySeedFault::Inspect, "could not inspect"),
            (
                G2DirectorySeedFault::PrivateMode,
                "could not set private mode",
            ),
        ];
        #[cfg(unix)]
        cases.insert(
            1,
            (G2DirectorySeedFault::SymlinkConflict, "is not a directory"),
        );
        cases
    }

    #[test]
    fn g2_random_directory_seed_io_failures_roll_back_without_touching_outside() {
        for (fault, message) in g2_directory_seed_fault_cases() {
            let outside = tempfile::TempDir::new().unwrap();
            let sentinel = outside.path().join("sentinel");
            fs::write(&sentinel, b"outside stays").unwrap();
            let io = G2DirectorySeedIo::new(fault, sentinel.clone());
            let error = RealWalkerHost::spawn_with_directory_seed_io(
                WalkerSeedSpec {
                    directories: vec![directory_seed(WalkerDirectoryRoot::Home, ".codex")],
                    ..WalkerSeedSpec::default()
                },
                &io,
            )
            .unwrap_err();

            assert!(error.contains(message), "{fault:?}: {error}");
            let attempted = io.attempts.borrow();
            let sandbox = attempted[0].parent().unwrap().parent().unwrap();
            assert!(!sandbox.exists(), "{fault:?}: {}", sandbox.display());
            assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays");
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn g2_stable_directory_seed_io_failures_remove_profile_and_release_lease() {
        for (index, (fault, message)) in g2_directory_seed_fault_cases().into_iter().enumerate() {
            let parent = tempfile::TempDir::new().unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let namespace =
                super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap();
            let profile = safe_profile(&format!("g2-seed-fault-{index}"));
            let outside = parent.path().join("outside-sentinel");
            fs::write(&outside, b"outside stays").unwrap();
            let io = G2DirectorySeedIo::new(fault, outside.clone());
            let error = RealWalkerHost::spawn_stable_in_with_directory_seed_io(
                WalkerSeedSpec {
                    directories: vec![directory_seed(WalkerDirectoryRoot::Cwd, ".claude")],
                    ..WalkerSeedSpec::default()
                },
                profile.clone(),
                namespace,
                &io,
            )
            .unwrap_err();

            assert!(error.to_string().contains(message), "{fault:?}: {error}");
            assert!(matches!(
                error,
                super::StableHostError::Primary {
                    primary: super::StableHostPrimaryError::Seed(_),
                    cleanup: None,
                }
            ));
            assert!(!namespace_root.join(profile_sandbox_path(&profile)).exists());
            assert!(!namespace_root.join(profile_cleanup_path(&profile)).exists());
            assert_eq!(fs::read(&outside).unwrap(), b"outside stays");
            RealWalkerHost::spawn_stable_in(
                WalkerSeedSpec::default(),
                profile,
                super::StableSandboxNamespace::explicit(namespace_root).unwrap(),
            )
            .unwrap()
            .close()
            .unwrap();
        }
    }

    #[test]
    fn g2_directory_seed_shapes_refuse_before_any_seed_write() {
        let paths = vec![
            PathBuf::new(),
            PathBuf::from("."),
            PathBuf::from("./child"),
            PathBuf::from("nested/./child"),
            PathBuf::from("../escape"),
            PathBuf::from("nested/../escape"),
            PathBuf::from(std::path::MAIN_SEPARATOR.to_string()),
            std::env::temp_dir().join("absolute-seed"),
        ];
        #[cfg(windows)]
        let paths = {
            let mut paths = paths;
            paths.push(PathBuf::from(r"C:prefix"));
            paths
        };

        for (index, path) in paths.into_iter().enumerate() {
            let spec = WalkerSeedSpec {
                directories: vec![WalkerDirectorySeed {
                    root: WalkerDirectoryRoot::Home,
                    path: path.clone(),
                }],
                external: vec![WalkerExternalSeed::File {
                    path: PathBuf::from("would-write"),
                    bytes: b"must not exist".to_vec(),
                    readonly: false,
                    unix_mode: 0o600,
                }],
                ..WalkerSeedSpec::default()
            };
            let error = super::validate_external_spec(&spec).unwrap_err();
            assert!(error.contains("relative descendants"), "{path:?}: {error}");

            #[cfg(any(target_os = "linux", target_os = "windows"))]
            {
                let parent = tempfile::TempDir::new().unwrap();
                let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
                let result = RealWalkerHost::spawn_stable_in(
                    spec,
                    safe_profile(&format!("invalid-directory-{index}")),
                    super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
                );
                assert!(matches!(
                    result,
                    Err(super::StableHostError::Primary {
                        primary: super::StableHostPrimaryError::Seed(_),
                        cleanup: None,
                    })
                ));
                assert!(!namespace_root.exists(), "{path:?}");
            }
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            let _ = (index, spec);
        }
    }

    #[test]
    fn g2_seeded_directory_file_replacement_refuses_before_first_observe() {
        let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
            directories: vec![directory_seed(WalkerDirectoryRoot::Home, ".codex")],
            ..WalkerSeedSpec::default()
        })
        .unwrap();
        let state = host.initial_state().unwrap();
        let path = host.roots().home.as_ref().unwrap().join(".codex");
        fs::remove_dir(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();

        let error = host.observe(&state).unwrap_err();
        assert!(error.contains("expected directory"), "{error}");
        assert!(error.contains("found file"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn g2_seeded_directory_symlink_replacement_refuses_before_first_observe() {
        let mut host = RealWalkerHost::spawn(WalkerSeedSpec {
            directories: vec![directory_seed(WalkerDirectoryRoot::Cwd, ".claude")],
            ..WalkerSeedSpec::default()
        })
        .unwrap();
        let state = host.initial_state().unwrap();
        let path = host.roots().cwd.join(".claude");
        fs::remove_dir(&path).unwrap();
        std::os::unix::fs::symlink(host.roots().home.as_ref().unwrap(), &path).unwrap();

        let error = host.observe(&state).unwrap_err();
        assert!(error.contains("expected directory"), "{error}");
        assert!(error.contains("found symlink"), "{error}");
    }

    // The stable sandbox does not support macOS, so this test skips there. Keep it compiled on
    // macOS. A cfg gate would make the complete stable owner dead code.
    #[cfg_attr(
        target_os = "macos",
        ignore = "the stable sandbox does not support macOS"
    )]
    #[test]
    fn g2_editor_write_queue_drives_real_add_edit_fifo_and_stable_replay() {
        let spec = WalkerSeedSpec {
            profile: "g2-editor-queue".to_owned(),
            editor_writes: vec![b"first\n".to_vec(), b"second\n".to_vec()],
            ..WalkerSeedSpec::default()
        };
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            super::StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let profile = safe_profile("g2-editor-queue");
        let run = |spec, namespace| {
            let mut host =
                RealWalkerHost::spawn_stable_in(spec, profile.clone(), namespace).unwrap();
            let mut state = host.initial_state().unwrap();
            let opened = host
                .dispatch(Effect::Open {
                    request: HostRequest::Add,
                    selector: None,
                })
                .unwrap();
            assert_eq!(state.update(opened), Effect::None);
            host.clear_transcript();

            let author = state.update(Action::Add(AddAction::NewDraft(DraftKind::Script)));
            let authored = host.dispatch(author).unwrap();
            let source = expect_authored_draft_source(authored.clone());
            assert_eq!(fs::read(&source.path).unwrap(), b"first\n");
            assert_eq!(source.bytes, b"first\n");
            assert_eq!(state.update(authored), Effect::None);

            let mut bytes = vec![b"first\n".to_vec()];
            for expected in [b"second\n".as_slice(), b"second\n".as_slice()] {
                let edit = state.update(Action::Add(AddAction::EditSource));
                let edited = host.dispatch(edit).unwrap();
                assert!(matches!(
                    &edited,
                    Action::Add(AddAction::SourceEdited { result: Ok(_), .. })
                ));
                let edited_value = serde_json::to_value(&edited).unwrap();
                let edited_source = &edited_value["add"]["source_edited"]["result"]["Ok"];
                assert_eq!(edited_source["path"], json!(source.path));
                assert_eq!(edited_source["bytes"], json!(expected));
                assert_eq!(fs::read(&source.path).unwrap(), expected);
                bytes.push(expected.to_vec());
                assert_eq!(state.update(edited), Effect::None);
            }

            let raw_editor_events = host
                .adapters
                .pending_events()
                .into_iter()
                .filter_map(|event| match event {
                    PortEvent::Editor {
                        argv,
                        path,
                        outcome,
                    } => Some((argv, path, outcome)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let observation = host.observe(&state).unwrap();
            let projected_editor_events = events_with_keys(&observation.transcript, &["editor"]);
            host.close().unwrap();
            (
                raw_editor_events,
                projected_editor_events,
                bytes,
                observation,
            )
        };

        let first = run(spec.clone(), namespace.clone());
        let replay = run(spec, namespace);
        assert_eq!(first, replay);
        assert_eq!(first.0.len(), 3);
        assert_eq!(first.1.len(), 3);
        assert_eq!(
            first.2,
            vec![
                b"first\n".to_vec(),
                b"second\n".to_vec(),
                b"second\n".to_vec(),
            ]
        );
        assert_eq!(
            first
                .3
                .state
                .pointer("/workflow/active/add/review/source/bytes"),
            Some(&json!(b"second\n"))
        );
        let draft = first
            .3
            .tree
            .iter()
            .find(|row| row.path == "data/.drafts/<draft:0>.py")
            .unwrap();
        assert_eq!(
            draft.content,
            Some(super::ByteView::Utf8("second\n".to_owned()))
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn g2_stable_directory_seed_replay_has_equal_paths_modes_and_observations() {
        let spec = WalkerSeedSpec {
            profile: "g2-stable".to_owned(),
            directories: vec![
                directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
                directory_seed(WalkerDirectoryRoot::Cwd, ".claude/skills"),
            ],
            ..WalkerSeedSpec::default()
        };
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            super::StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let review_profile = safe_profile("g2-stable");
        let capture = |spec, namespace| {
            let mut host =
                RealWalkerHost::spawn_stable_in(spec, review_profile.clone(), namespace).unwrap();
            let seeded = (
                host.roots().home.as_ref().unwrap().join(".codex/skills"),
                host.roots().cwd.join(".claude/skills"),
            );
            let observation = host.observe(&host.initial_state().unwrap()).unwrap();
            host.close().unwrap();
            (seeded, observation)
        };

        let first = capture(spec.clone(), namespace.clone());
        let second = capture(spec, namespace);
        assert_eq!(first, second);
        for relative in [
            "home/.codex",
            "home/.codex/skills",
            "cwd/.claude",
            "cwd/.claude/skills",
        ] {
            let row = first
                .1
                .tree
                .iter()
                .find(|row| row.path == relative)
                .unwrap();
            assert_eq!(row.kind, "directory");
            #[cfg(unix)]
            assert_eq!(row.mode, Some(0o700));
            #[cfg(not(unix))]
            assert_eq!(row.mode, None);
        }
    }

    #[test]
    fn g2c_file_picker_tree_is_exact_idempotent_and_external_only() {
        super::validate_external_spec(&profile()).unwrap();
        let spec = WalkerSeedSpec {
            profile: "g2c-picker-tree".to_owned(),
            external: vec![
                WalkerExternalSeed::File {
                    path: PathBuf::from("nested/deep/alpha.txt"),
                    bytes: b"alpha\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
                WalkerExternalSeed::File {
                    path: PathBuf::from("nested/beta.txt"),
                    bytes: b"beta\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
                WalkerExternalSeed::File {
                    path: PathBuf::from("nested/deep/alpha.txt"),
                    bytes: b"alpha\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
            ],
            directories: vec![
                directory_seed(WalkerDirectoryRoot::Home, ".codex/skills"),
                directory_seed(WalkerDirectoryRoot::Cwd, ".claude/skills"),
            ],
            ..WalkerSeedSpec::default()
        };
        let host = RealWalkerHost::spawn(spec).unwrap();
        let root = host.external_root.clone();

        let tree = host.file_picker_tree();

        assert_eq!(tree.root, root);
        assert_eq!(
            tree.directories,
            BTreeSet::from([root.clone(), root.join("nested"), root.join("nested/deep"),])
        );
        assert_eq!(
            tree.files,
            BTreeSet::from([
                root.join("nested/beta.txt"),
                root.join("nested/deep/alpha.txt"),
            ])
        );
        assert!(
            tree.directories
                .iter()
                .chain(&tree.files)
                .all(|path| path.starts_with(&root))
        );
        assert!(
            !tree
                .directories
                .contains(&host.roots().home.as_ref().unwrap().join(".codex/skills"))
        );
        assert!(
            !tree
                .directories
                .contains(&host.roots().cwd.join(".claude/skills"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn g2c_file_picker_tree_keeps_symlink_parents_but_omits_the_leaf() {
        let host = RealWalkerHost::spawn(WalkerSeedSpec {
            profile: "g2c-picker-symlink".to_owned(),
            external: vec![
                WalkerExternalSeed::File {
                    path: PathBuf::from("targets/real.txt"),
                    bytes: b"target\n".to_vec(),
                    readonly: false,
                    unix_mode: 0o640,
                },
                WalkerExternalSeed::Symlink {
                    path: PathBuf::from("links/deep/alias.txt"),
                    target: PathBuf::from("targets/real.txt"),
                },
            ],
            ..WalkerSeedSpec::default()
        })
        .unwrap();
        let root = host.external_root.clone();

        let tree = host.file_picker_tree();

        assert_eq!(
            tree.directories,
            BTreeSet::from([
                root.clone(),
                root.join("links"),
                root.join("links/deep"),
                root.join("targets"),
            ])
        );
        assert_eq!(tree.files, BTreeSet::from([root.join("targets/real.txt")]));
        let symlink = root.join("links/deep/alias.txt");
        assert!(
            fs::symlink_metadata(&symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(!tree.directories.contains(&symlink));
        assert!(!tree.files.contains(&symlink));
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn g2c_file_picker_tree_is_equal_across_fresh_stable_hosts() {
        let spec = WalkerSeedSpec {
            profile: "g2c-picker-replay".to_owned(),
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("nested/replay.txt"),
                bytes: b"replay\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            }],
            ..WalkerSeedSpec::default()
        };
        let parent = tempfile::TempDir::new().unwrap();
        let namespace =
            super::StableSandboxNamespace::explicit(parent.path().join(STABLE_SANDBOX_NAMESPACE))
                .unwrap();
        let review_profile = safe_profile("g2c-picker-replay");
        let capture = |spec, namespace| {
            let host =
                RealWalkerHost::spawn_stable_in(spec, review_profile.clone(), namespace).unwrap();
            let tree = host.file_picker_tree();
            host.close().unwrap();
            tree
        };

        let first = capture(spec.clone(), namespace.clone());
        let replay = capture(spec, namespace);

        assert_eq!(first, replay);
    }

    #[test]
    fn g2c_file_picker_tree_drives_the_real_add_picker_with_relative_identity() {
        let host = RealWalkerHost::spawn(WalkerSeedSpec {
            profile: "g2c-picker-session".to_owned(),
            external: vec![WalkerExternalSeed::File {
                path: PathBuf::from("visible.sh"),
                bytes: b"printf visible\n".to_vec(),
                readonly: false,
                unix_mode: 0o640,
            }],
            ..WalkerSeedSpec::default()
        })
        .unwrap();
        let tree = host.file_picker_tree();
        let mut state = host.initial_state().unwrap();
        assert_eq!(
            state.update(Action::Present(skit_ui::Screen::Add(Box::new(
                AddWorkflowState::new(Vec::new()),
            )))),
            Effect::None
        );
        let mut session = TuiSession::with_file_picker_tree(
            tree.root.clone(),
            tree.directories.clone(),
            tree.files.clone(),
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut geometry = ViewGeometry::default();
        terminal
            .draw(|frame| {
                geometry = render_with_session(frame, &state, Locale::En, &mut session);
            })
            .unwrap();
        let binding = session
            .local_action_inventory()
            .actions
            .iter()
            .find(|action| action.target == LocalActionTarget::Add(AddControlId::BrowseSource))
            .unwrap()
            .keys[0];
        assert_eq!(
            session.handle_event(Event::Key(binding.event()), &state, &geometry),
            EventHandling::Consumed
        );

        let snapshot = serde_json::to_value(session.agent_review_snapshot().unwrap()).unwrap();
        let entry = snapshot
            .pointer("/add_overlay/fields/session/fields/explorer/entries")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "visible.sh")
            .unwrap();
        let absolute = PathBuf::from(entry["path"].as_str().unwrap());
        assert_eq!(absolute, tree.root.join("visible.sh"));
        assert_eq!(
            absolute.strip_prefix(&tree.root).unwrap(),
            Path::new("visible.sh")
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn g2c_file_symlink_overlaps_refuse_before_acquisition_in_every_order() {
        let file = |path: &str| WalkerExternalSeed::File {
            path: PathBuf::from(path),
            bytes: b"file\n".to_vec(),
            readonly: false,
            unix_mode: 0o640,
        };
        let symlink = |path: &str| WalkerExternalSeed::Symlink {
            path: PathBuf::from(path),
            target: PathBuf::from("target"),
        };
        let cases = [
            (
                vec![symlink("collision"), file("collision")],
                "walker external fixture leaf paths overlap: collision and collision",
            ),
            (
                vec![file("collision"), symlink("collision")],
                "walker external fixture leaf paths overlap: collision and collision",
            ),
            (
                vec![symlink("alias"), file("alias/child.txt")],
                "walker external fixture leaf paths overlap: alias and alias/child.txt",
            ),
            (
                vec![file("alias/child.txt"), symlink("alias")],
                "walker external fixture leaf paths overlap: alias and alias/child.txt",
            ),
            (
                vec![file("leaf"), symlink("leaf/descendant")],
                "walker external fixture leaf paths overlap: leaf and leaf/descendant",
            ),
        ];

        for (index, (external, expected)) in cases.into_iter().enumerate() {
            let parent = tempfile::TempDir::new().unwrap();
            let sentinel = parent.path().join("outside-sentinel");
            fs::write(&sentinel, b"outside stays").unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let review_profile = safe_profile(&format!("g2c-picker-overlap-{index}"));
            let spec = WalkerSeedSpec {
                external,
                ..WalkerSeedSpec::default()
            };

            let validation = super::validate_external_spec(&spec).unwrap_err();
            assert_eq!(validation, expected);
            let error = RealWalkerHost::spawn_stable_in(
                spec,
                review_profile,
                super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            )
            .unwrap_err();

            assert!(matches!(
                error,
                super::StableHostError::Primary {
                    primary: super::StableHostPrimaryError::Seed(_),
                    cleanup: None,
                }
            ));
            assert!(!namespace_root.exists());
            assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays");
        }
    }

    #[cfg(unix)]
    #[test]
    fn g2c_identical_external_seed_duplicates_are_written_once() {
        use std::os::unix::fs::PermissionsExt as _;

        let readonly = WalkerExternalSeed::File {
            path: PathBuf::from("nested/target.txt"),
            bytes: b"target\n".to_vec(),
            readonly: true,
            unix_mode: 0o440,
        };
        let symlink = WalkerExternalSeed::Symlink {
            path: PathBuf::from("links/alias.txt"),
            target: PathBuf::from("nested/target.txt"),
        };
        let host = RealWalkerHost::spawn(WalkerSeedSpec {
            external: vec![readonly.clone(), readonly, symlink.clone(), symlink],
            ..WalkerSeedSpec::default()
        })
        .unwrap();
        let root = host.external_root.clone();

        assert_eq!(
            fs::read(root.join("nested/target.txt")).unwrap(),
            b"target\n"
        );
        assert_eq!(
            fs::metadata(root.join("nested/target.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o440
        );
        assert!(
            fs::symlink_metadata(root.join("links/alias.txt"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            host.file_picker_tree().files,
            BTreeSet::from([root.join("nested/target.txt")])
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn g2c_same_kind_seed_payload_drift_refuses_before_acquisition() {
        let file = |bytes: &[u8], readonly, unix_mode| WalkerExternalSeed::File {
            path: PathBuf::from("collision"),
            bytes: bytes.to_vec(),
            readonly,
            unix_mode,
        };
        let symlink = |target: &str| WalkerExternalSeed::Symlink {
            path: PathBuf::from("collision"),
            target: PathBuf::from(target),
        };
        let cases = [
            vec![
                file(b"first\n", false, 0o640),
                file(b"second\n", false, 0o640),
            ],
            vec![file(b"same\n", false, 0o640), file(b"same\n", true, 0o640)],
            vec![file(b"same\n", false, 0o640), file(b"same\n", false, 0o600)],
            vec![symlink("first-target"), symlink("second-target")],
        ];

        for (index, external) in cases.into_iter().enumerate() {
            let parent = tempfile::TempDir::new().unwrap();
            let sentinel = parent.path().join("outside-sentinel");
            fs::write(&sentinel, b"outside stays").unwrap();
            let namespace_root = parent.path().join(STABLE_SANDBOX_NAMESPACE);
            let spec = WalkerSeedSpec {
                external,
                ..WalkerSeedSpec::default()
            };

            assert_eq!(
                super::validate_external_spec(&spec).unwrap_err(),
                "walker external fixture seed payloads conflict at collision"
            );
            let error = RealWalkerHost::spawn_stable_in(
                spec,
                safe_profile(&format!("g2c-payload-conflict-{index}")),
                super::StableSandboxNamespace::explicit(namespace_root.clone()).unwrap(),
            )
            .unwrap_err();

            assert!(matches!(
                error,
                super::StableHostError::Primary {
                    primary: super::StableHostPrimaryError::Seed(_),
                    cleanup: None,
                }
            ));
            assert!(!namespace_root.exists());
            assert_eq!(fs::read(&sentinel).unwrap(), b"outside stays");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tree_observation_escapes_non_utf8_paths_without_dropping_bytes() {
        use std::os::unix::ffi::OsStringExt as _;

        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let path = host
            .roots()
            .state
            .join(std::ffi::OsString::from_vec(b"opaque-\xff".to_vec()));
        fs::write(&path, b"\x00\xff\x10").unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert!(observation.tree.iter().any(|row| {
            row.path == "state/opaque-\\xff"
                && matches!(&row.content, Some(super::ByteView::Hex(bytes)) if bytes == "00ff10")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn tree_observation_keeps_a_leaked_temporary_file() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        fs::write(host.roots().data.join("leaked.tmp-1234"), b"visible").unwrap();

        let observation = host.observe(&host.initial_state().unwrap()).unwrap();

        assert!(
            observation
                .tree
                .iter()
                .any(|row| row.path == "data/leaked.tmp-1234")
        );
    }
    /// Every case of the Windows verbatim-prefix rule, as text.
    ///
    /// The rule decides which spelling the fixture looks up and compares. No Windows host runs
    /// these tests, so the cases are literal text and they run on every platform.
    #[test]
    fn the_verbatim_prefix_rule_keeps_the_ordinary_spelling_of_one_location() {
        for (recorded, ordinary) in [
            (r"\\?\C:\a\b", r"C:\a\b"),
            (r"\\?\c:\", r"c:\"),
            (r"\\?\UNC\srv\share\x", r"\\srv\share\x"),
            (r"C:\a\b", r"C:\a\b"),
            (r"\\srv\share\x", r"\\srv\share\x"),
            (r"\\?\Volume{0}\x", r"\\?\Volume{0}\x"),
            (r"\\?\C:", r"\\?\C:"),
            (r"\\?\1:\a", r"\\?\1:\a"),
            (r"\\?\", r"\\?\"),
            ("/tmp/a/b", "/tmp/a/b"),
            ("", ""),
        ] {
            assert_eq!(
                super::ordinary_windows_path(recorded),
                ordinary,
                "{recorded}"
            );
            assert_eq!(
                *super::ordinary_path(Path::new(recorded)),
                *Path::new(ordinary),
                "{recorded}"
            );
            assert!(
                super::names_the_same_path(recorded, Path::new(ordinary)),
                "{recorded}"
            );
            assert!(
                !super::names_the_same_path(recorded, Path::new("/other")),
                "{recorded}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn one_path_that_is_not_unicode_keeps_its_exact_spelling() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = PathBuf::from(std::ffi::OsString::from_vec(b"/tmp/opaque-\xff".to_vec()));

        assert_eq!(*super::ordinary_path(&path), *path);
    }

    #[cfg(windows)]
    #[test]
    fn one_path_that_is_not_unicode_keeps_its_exact_spelling() {
        use std::os::windows::ffi::OsStringExt as _;

        let path = PathBuf::from(std::ffi::OsString::from_wide(&[0xd800]));

        assert_eq!(*super::ordinary_path(&path), *path);
    }

    #[cfg(unix)]
    #[test]
    fn resolved_profile_paths_keep_the_declared_fixture_spelling() {
        let host = RealWalkerHost::spawn(profile()).unwrap();
        let parent = tempfile::TempDir::new().unwrap();
        let alias = parent.path().join("profile-alias");
        std::os::unix::fs::symlink(host.sandbox_root(), &alias).unwrap();
        let mut paths = super::PathMap::new(
            &host.profile,
            &alias,
            &host.service,
            &host.file_picker_tree.files,
        )
        .unwrap();
        let draft = alias.join("data/.drafts/skit-new-first.py");
        paths.register_draft_path(&draft);
        let resolved = host.sandbox_root().join("data/.drafts/skit-new-first.py");
        let projected = paths.normalize_path(&draft);
        assert!(paths.is_registered_path(&resolved));
        assert_eq!(paths.normalize_path(&resolved), projected);
        assert_eq!(
            paths.normalize_host_text(resolved.to_str().unwrap()),
            projected
        );
        let facts = paths.leak_oracle_facts();
        let root = &facts.artifacts["<profile:fixture>"].raw_path_spellings;
        assert!(root.contains(alias.to_str().unwrap()));
        assert!(root.contains(host.sandbox_root().to_str().unwrap()));
    }

    /// The fixture owns one path in the verbatim spelling of it.
    ///
    /// Windows `fs::canonicalize` returns the verbatim form, so a path that the product recorded
    /// arrives in that form. The lookup keys stay in the ordinary spelling.
    #[test]
    fn a_registered_path_accepts_the_windows_verbatim_spelling_of_itself() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = Path::new(r"C:\sandbox\data\.drafts\skit-new-first.py");
        host.path_map.register_draft_path(draft);
        host.path_map.register_owned_transient_path(
            Path::new(r"C:\sandbox\state\run.tmp"),
            "run",
            "",
        );
        let stable = host.path_map.normalize_path(draft);

        for (recorded, expected) in [
            (r"\\?\C:\sandbox\data\.drafts\skit-new-first.py", &stable),
            (r"C:\sandbox\data\.drafts\skit-new-first.py", &stable),
        ] {
            assert!(host.path_map.is_registered_path(Path::new(recorded)));
            assert_eq!(host.path_map.normalize_path(Path::new(recorded)), *expected);
        }
        assert!(
            host.path_map
                .is_registered_path(Path::new(r"\\?\C:\sandbox\state\run.tmp"))
        );
        let stranger = Path::new(r"\\?\C:\sandbox\data\.drafts\stranger.py");
        let outside = host.path_map.normalize_path(stranger);
        assert!(!host.path_map.is_registered_path(stranger));
        assert_ne!(outside, stable);
        assert!(outside.contains("stranger.py"), "{outside}");
    }

    /// One Add Commit that the product recorded in the Windows verbatim spelling.
    ///
    /// The product resolves `source_record` and `CreateEntry::source`, so both arrive with the
    /// `\\?\` prefix while the draft path keeps skit's own spelling. The oracle must accept the
    /// pair, and no verbatim text may reach the projected artifact.
    #[test]
    fn a_commit_accepts_the_windows_verbatim_spelling_of_its_recorded_source() {
        let mut host = RealWalkerHost::spawn(profile()).unwrap();
        let draft = Path::new(r"C:\sandbox\data\.drafts\skit-new-first.py");
        let recorded = r"\\?\C:\sandbox\data\.drafts\skit-new-first.py";
        host.path_map.register_draft_path(draft);
        let stable_path = host.path_map.normalize_path(draft);
        let raw_name = super::review_default_name(draft, "python");
        let stable_name = super::review_default_name(Path::new(&stable_path), "python");
        host.path_map.add_provenance.review_source_path = Some(draft.to_path_buf());
        set_review_name_projection(&mut host, draft, &raw_name, &stable_name);

        let commit_value = |record: &str| {
            let mut entry = serde_json::to_value(profile().entries[1].clone()).unwrap();
            entry["name"] = json!(raw_name);
            entry["kind"] = json!("python");
            entry["mode"] = json!("copy");
            entry["source"] = json!(record);
            json!({"add": [{"commit": {
                "request": 0,
                "entry": entry,
                "source": {
                    "path": draft.display().to_string(),
                    "source_record": record,
                    "bytes": [1, 2, 3],
                    "permissions": {"readonly": false, "unix_mode": 384},
                    "executable": false,
                    "is_regular": true,
                    "is_directory": false,
                    "is_draft": true,
                    "identity": null,
                },
            }}]})
        };
        let mut effect: Effect =
            super::deserialize_canonical(commit_value(recorded), "verbatim Commit fixture")
                .unwrap();
        host.path_map.project_typed_effect(&mut effect).unwrap();
        let projected = serde_json::to_value(effect).unwrap();

        let commit = &projected["add"][0]["commit"];
        assert_eq!(commit["entry"]["name"], json!(stable_name));
        assert_eq!(commit["entry"]["source"], json!(stable_path));
        assert_eq!(commit["source"]["path"], json!(stable_path));
        assert_eq!(commit["source"]["source_record"], json!(stable_path));

        let mut stranger: Effect = super::deserialize_canonical(
            commit_value(r"\\?\C:\sandbox\data\.drafts\stranger.py"),
            "verbatim Commit stranger fixture",
        )
        .unwrap();
        assert!(host.path_map.project_typed_effect(&mut stranger).is_err());
    }
}
