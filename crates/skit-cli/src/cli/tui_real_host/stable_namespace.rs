//! Sandbox errors, the stable namespace, its markers, and its leases.

use std::{
    cell::Cell,
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use skit_tui_walker_support::sandbox::SandboxRoots;
use skit_tui_walker_support::{
    ArtifactError,
    sandbox::{
        LEASE_DIRECTORY, NAMESPACE_MARKER_FILE, NamespaceMarker, ParentInitLockMarker,
        ProfileLeaseMarker, SANDBOX_DIRECTORY, SANDBOX_MARKER_FILE, STABLE_SANDBOX_NAMESPACE,
        SafeProfileId, SandboxMarker, SandboxPlatform, profile_cleanup_path, profile_lease_path,
        profile_sandbox_path,
    },
    validate_stable_namespace_root,
};

use crate::cli::tui_real_sandbox_fs::{
    ChildName, EntryTicket, InitializationGuard, NamespaceHandles, NodeKind as SandboxNodeKind,
    PinnedDirectory, PinnedFile, ProfileLease, SandboxFsError, combine_release,
};

const EMPTY_LOCK_RETRIES: usize = 256;

#[derive(Debug)]
pub(crate) enum SandboxError {
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
    pub(super) fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }

    pub(super) fn invalid(path: &Path, reason: impl Into<String>) -> Self {
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
pub(crate) enum StableHostPrimaryError {
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
pub(crate) enum StableHostError {
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
pub(crate) enum SandboxFaultPoint {
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
pub(super) struct SandboxFaults(Cell<Option<SandboxFaultPoint>>);

impl SandboxFaults {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(super) const fn at(point: SandboxFaultPoint) -> Self {
        Self(Cell::new(Some(point)))
    }

    pub(super) fn check(&self, point: SandboxFaultPoint) -> Result<(), SandboxError> {
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

    pub(super) fn take(&self, point: SandboxFaultPoint) -> bool {
        if self.0.get() != Some(point) {
            return false;
        }
        self.0.set(None);
        true
    }
}

pub(super) fn current_sandbox_platform() -> Result<SandboxPlatform, SandboxError> {
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

pub(super) fn default_stable_namespace_root() -> Result<PathBuf, SandboxError> {
    stable_namespace_root_for(current_sandbox_platform()?, &std::env::temp_dir())
}

pub(super) fn stable_namespace_root_for(
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
pub(crate) struct StableSandboxNamespace {
    platform: SandboxPlatform,
    path: PathBuf,
    literal: String,
}

impl StableSandboxNamespace {
    pub(crate) fn system() -> Result<Self, SandboxError> {
        Self::explicit(default_stable_namespace_root()?)
    }

    pub(crate) fn explicit(path: PathBuf) -> Result<Self, SandboxError> {
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

pub(super) fn literal_path(path: &Path) -> Result<String, SandboxError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| SandboxError::invalid(path, "the literal path is not valid UTF-8"))
}

pub(super) fn map_contract_result<T, E: std::fmt::Display>(
    path: &Path,
    result: Result<T, E>,
) -> Result<T, SandboxError> {
    result.map_err(|error| SandboxError::invalid(path, error.to_string()))
}

#[derive(Debug)]
pub(super) struct StableSandboxPaths {
    pub(super) platform: SandboxPlatform,
    pub(super) profile: SafeProfileId,
    pub(super) namespace_root: PathBuf,
    pub(super) init_lock: PathBuf,
    pub(super) leases_root: PathBuf,
    pub(super) sandboxes_root: PathBuf,
    pub(super) lease_path: PathBuf,
    pub(super) sandbox_root: PathBuf,
    pub(super) cleanup_root: PathBuf,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(super) roots: SandboxRoots,
    parent_lock_bytes: Vec<u8>,
    namespace_marker_bytes: Vec<u8>,
    lease_marker_bytes: Vec<u8>,
    sandbox_marker_bytes: Vec<u8>,
}

pub(super) struct StableContractData {
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
    pub(super) fn new(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
    ) -> Result<Self, SandboxError> {
        Self::new_with_contract_builder(namespace, profile, derive_stable_contract_data)
    }

    pub(super) fn new_with_contract_builder<E: std::fmt::Display>(
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

    pub(super) fn parent_lock_bytes(&self) -> &[u8] {
        &self.parent_lock_bytes
    }

    pub(super) fn namespace_marker_bytes(&self) -> &[u8] {
        &self.namespace_marker_bytes
    }

    pub(super) fn lease_marker_bytes(&self) -> &[u8] {
        &self.lease_marker_bytes
    }

    pub(super) fn sandbox_marker_bytes(&self) -> &[u8] {
        &self.sandbox_marker_bytes
    }

    pub(super) fn namespace_name(&self) -> ChildName {
        ChildName::literal(STABLE_SANDBOX_NAMESPACE)
    }

    pub(super) fn init_lock_name(&self) -> ChildName {
        ChildName::new(
            self.init_lock
                .file_name()
                .expect("the typed init lock has a filename"),
        )
        .expect("the typed init lock filename is one component")
    }

    pub(super) fn leases_name(&self) -> ChildName {
        ChildName::literal(LEASE_DIRECTORY)
    }

    pub(super) fn sandboxes_name(&self) -> ChildName {
        ChildName::literal(SANDBOX_DIRECTORY)
    }

    pub(super) fn lease_name(&self) -> ChildName {
        ChildName::new(
            self.lease_path
                .file_name()
                .expect("the typed lease has a filename"),
        )
        .expect("the typed lease filename is one component")
    }

    pub(super) fn sandbox_name(&self) -> ChildName {
        ChildName::new(self.profile.as_str()).expect("a safe profile is one filename")
    }

    pub(super) fn cleanup_name(&self) -> ChildName {
        ChildName::new(
            self.cleanup_root
                .file_name()
                .expect("the typed cleanup root has a filename"),
        )
        .expect("the typed cleanup filename is one component")
    }

    pub(super) fn namespace_marker_name(&self) -> ChildName {
        ChildName::literal(NAMESPACE_MARKER_FILE)
    }

    pub(super) fn sandbox_marker_name(&self) -> ChildName {
        ChildName::literal(SANDBOX_MARKER_FILE)
    }

    pub(super) fn sandbox_child_names(&self) -> [ChildName; 7] {
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

pub(super) fn ticket_named(
    directory: &PinnedDirectory,
    name: &ChildName,
) -> Result<Option<EntryTicket>, SandboxError> {
    Ok(directory
        .tickets()
        .map_err(SandboxError::from)?
        .into_iter()
        .find(|ticket| ticket.name() == name))
}

pub(super) fn require_ticket(
    directory: &PinnedDirectory,
    name: &ChildName,
    expected_kind: SandboxNodeKind,
    expected_identity: crate::cli::tui_real_sandbox_fs::NodeIdentity,
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

pub(super) fn publish_relative_marker(
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

pub(super) fn validate_relative_marker(
    directory: &PinnedDirectory,
    name: &ChildName,
    expected: &[u8],
) -> Result<PinnedFile, SandboxError> {
    validate_relative_marker_with_hook(directory, name, expected, |_, _, _| {})
}

pub(super) fn validate_relative_marker_with_hook(
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

pub(super) fn require_exact_names(
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

pub(super) fn create_relative_directory_with_fault(
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

pub(super) fn combine_sandbox_release(
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

pub(super) fn release_initialization_guard(
    guard: InitializationGuard,
) -> Result<PinnedDirectory, SandboxFsError> {
    guard.release(|_| Ok(()))
}

pub(super) fn acquire_initialization_guard(
    paths: &StableSandboxPaths,
    faults: &SandboxFaults,
) -> Result<InitializationGuard, SandboxError> {
    acquire_initialization_guard_with_hooks(paths, faults, |_| {}, release_initialization_guard)
}

pub(super) fn acquire_initialization_guard_with_hooks(
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

pub(super) fn validate_namespace_opened_with_hook(
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

pub(super) fn initialize_namespace_retained(
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

pub(super) fn no_namespace_marker_publish_hook(_: &PinnedDirectory, _: &ChildName) {}

pub(super) fn no_before_namespace_release_hook(
    _: &InitializationGuard,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedFile,
) {
}

pub(super) fn no_after_namespace_release_hook(
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedDirectory,
    _: &PinnedFile,
) {
}

#[cfg(target_os = "linux")]
pub(super) fn initialize_namespace_retained_with_hook(
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

pub(super) fn initialize_namespace_retained_with_hooks(
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

pub(super) fn acquire_profile_lease_retained(
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

pub(super) fn no_profile_lease_hook(_: &ProfileLease, _: &PinnedDirectory) {}

pub(super) fn publish_profile_lease_marker(
    file: &PinnedFile,
    parent: &PinnedDirectory,
    name: &ChildName,
    bytes: &[u8],
) -> Result<(), SandboxFsError> {
    file.publish_bytes(parent, name, bytes)
}

pub(super) fn release_profile_lease(
    lease: ProfileLease,
    leases: &PinnedDirectory,
) -> Result<(), SandboxFsError> {
    lease.release(leases)
}

pub(super) fn acquire_profile_lease_retained_with_hooks(
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

pub(super) fn map_profile_lock_result(
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
