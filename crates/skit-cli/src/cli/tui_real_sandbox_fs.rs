//! Retained-handle filesystem operations for the real walker sandbox.
//!
//! This module accepts an ambient path only when it opens the namespace parent. Every owned
//! descendant is one validated component resolved relative to a retained directory handle.

use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
};

const PRIVATE_FILE_MODE: u32 = 0o600;
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const RENAME_NOREPLACE_OPERATION: &str = "rename sandbox without replacement";

#[derive(Debug)]
pub(super) enum SandboxFsError {
    UnsupportedPlatform,
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
    Invalid {
        path: PathBuf,
        reason: String,
    },
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    Dual {
        primary: Box<SandboxFsError>,
        release: Box<SandboxFsError>,
    },
}

impl SandboxFsError {
    fn invalid(path: &Path, reason: impl Into<String>) -> Self {
        Self::Invalid {
            path: path.to_path_buf(),
            reason: reason.into(),
        }
    }

    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_path_buf(),
            source,
        }
    }

    /// Tell if this error reports that the path is not found.
    #[cfg(target_os = "linux")]
    pub(super) fn is_not_found(&self) -> bool {
        matches!(self, Self::Io { source, .. } if source.kind() == io::ErrorKind::NotFound)
    }

    /// Tell if this error is a no-replace rename that refused because the destination name is in
    /// use. It does not tell the kind of the destination; the caller must open the destination to
    /// find out.
    pub(super) fn destination_already_exists(&self) -> bool {
        matches!(
            self,
            Self::Io {
                operation,
                source,
                ..
            } if *operation == RENAME_NOREPLACE_OPERATION
                && source.kind() == io::ErrorKind::AlreadyExists
        )
    }
}

impl std::fmt::Display for SandboxFsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedPlatform => {
                formatter.write_str("stable sandbox filesystem is unsupported")
            }
            Self::Unsupported {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {} because the filesystem operation is unsupported: {source}",
                path.display()
            ),
            Self::NativeStatus {
                operation,
                path,
                status,
            } => write!(
                formatter,
                "could not {operation} {}: native status 0x{status:08x}",
                path.display()
            ),
            Self::Invalid { path, reason } => {
                write!(
                    formatter,
                    "invalid sandbox path {}: {reason}",
                    path.display()
                )
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
            Self::Dual { primary, release } => {
                write!(formatter, "{primary}; release also failed: {release}")
            }
        }
    }
}

impl std::error::Error for SandboxFsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported { source, .. } => Some(source.as_ref()),
            Self::Io { source, .. } => Some(source),
            Self::Dual { primary, .. } => Some(primary),
            Self::UnsupportedPlatform | Self::NativeStatus { .. } | Self::Invalid { .. } => None,
        }
    }
}

pub(super) fn combine_release<E>(
    primary: Result<(), E>,
    release: Result<(), E>,
    dual: impl FnOnce(E, E) -> E,
) -> Result<(), E> {
    match (primary, release) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(primary), Ok(())) => Err(primary),
        (Ok(()), Err(release)) => Err(release),
        (Err(primary), Err(release)) => Err(dual(primary, release)),
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ChildName(OsString);

impl ChildName {
    pub(super) fn new(name: impl Into<OsString>) -> Result<Self, SandboxFsError> {
        Self::new_with_windows_rules(name, cfg!(windows))
    }

    fn new_with_windows_rules(
        name: impl Into<OsString>,
        windows_rules: bool,
    ) -> Result<Self, SandboxFsError> {
        let name = name.into();
        let path = Path::new(&name);
        if name.as_encoded_bytes().contains(&0) {
            return Err(SandboxFsError::invalid(
                path,
                "the child name contains a NUL",
            ));
        }
        if windows_rules && name.as_encoded_bytes().contains(&b':') {
            return Err(SandboxFsError::invalid(
                path,
                "the child name selects a Windows alternate data stream",
            ));
        }
        let mut components = path.components();
        let valid = matches!(components.next(), Some(Component::Normal(found)) if found == name)
            && components.next().is_none();
        if !valid {
            return Err(SandboxFsError::invalid(
                path,
                "the child name is not one normal component",
            ));
        }
        Ok(Self(name))
    }

    pub(super) fn literal(name: &'static str) -> Self {
        Self::new(name).expect("a source-owned child name is one normal component")
    }

    pub(super) fn as_os_str(&self) -> &OsStr {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NodeIdentity {
    Unix { device: u64, inode: u64 },
    Windows { volume: u64, file_id: u128 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NodeKind {
    RegularFile,
    Directory,
    LinkOrReparse,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpectedNode {
    RegularFile { private: bool },
    Directory { private: bool },
}

impl ExpectedNode {
    const fn kind(self) -> NodeKind {
        match self {
            Self::RegularFile { .. } => NodeKind::RegularFile,
            Self::Directory { .. } => NodeKind::Directory,
        }
    }

    const fn private_mode(self) -> Option<u32> {
        match self {
            Self::RegularFile { private: true } => Some(PRIVATE_FILE_MODE),
            Self::Directory { private: true } => Some(PRIVATE_DIRECTORY_MODE),
            Self::RegularFile { private: false } | Self::Directory { private: false } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EntryTicket {
    name: ChildName,
    identity: NodeIdentity,
    kind: NodeKind,
    reparse_tag: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RenameTicket {
    source: ChildName,
    destination: ChildName,
    identity: NodeIdentity,
}

impl RenameTicket {
    pub(super) fn new(source: ChildName, destination: ChildName, identity: NodeIdentity) -> Self {
        Self {
            source,
            destination,
            identity,
        }
    }
}

#[derive(Debug)]
pub(super) struct NamespaceHandles {
    parent: PinnedDirectory,
    namespace: PinnedDirectory,
    leases: PinnedDirectory,
    sandboxes: PinnedDirectory,
}

impl NamespaceHandles {
    pub(super) fn new(
        parent: PinnedDirectory,
        namespace: PinnedDirectory,
        leases: PinnedDirectory,
        sandboxes: PinnedDirectory,
    ) -> Self {
        Self {
            parent,
            namespace,
            leases,
            sandboxes,
        }
    }

    pub(super) fn namespace(&self) -> &PinnedDirectory {
        &self.namespace
    }

    pub(super) fn leases(&self) -> &PinnedDirectory {
        &self.leases
    }

    pub(super) fn sandboxes(&self) -> &PinnedDirectory {
        &self.sandboxes
    }

    pub(super) fn verify(
        &self,
        namespace_name: &ChildName,
        leases_name: &ChildName,
        sandboxes_name: &ChildName,
    ) -> Result<(), SandboxFsError> {
        Self::verify_opened(
            &self.parent,
            &self.namespace,
            &self.leases,
            &self.sandboxes,
            namespace_name,
            leases_name,
            sandboxes_name,
        )
    }

    pub(super) fn verify_opened(
        parent: &PinnedDirectory,
        namespace: &PinnedDirectory,
        leases: &PinnedDirectory,
        sandboxes: &PinnedDirectory,
        namespace_name: &ChildName,
        leases_name: &ChildName,
        sandboxes_name: &ChildName,
    ) -> Result<(), SandboxFsError> {
        parent.verify_handle()?;
        namespace.verify_handle()?;
        leases.verify_handle()?;
        sandboxes.verify_handle()?;
        namespace.verify_at(parent, namespace_name)?;
        leases.verify_at(namespace, leases_name)?;
        sandboxes.verify_at(namespace, sandboxes_name)
    }
}

#[derive(Debug)]
pub(super) struct InitializationGuard {
    parent: PinnedDirectory,
    init_name: ChildName,
    init_file: PinnedFile,
    created: bool,
}

#[derive(Debug)]
pub(super) struct ProfileLease {
    file: PinnedFile,
    name: ChildName,
    locked: bool,
}

#[derive(Debug)]
pub(super) struct ValidatedSandbox<C> {
    root: PinnedDirectory,
    marker: PinnedFile,
    children: C,
}

#[derive(Debug)]
pub(super) struct OriginalChildren([(ChildName, PinnedDirectory); 7]);

#[derive(Debug)]
pub(super) struct CleanupChildren(BTreeMap<ChildName, PinnedDirectory>);

pub(super) type ValidatedOriginalSandbox = ValidatedSandbox<OriginalChildren>;
pub(super) type ValidatedCleanupSandbox = ValidatedSandbox<CleanupChildren>;

#[derive(Debug)]
pub(super) struct CleanupPlan {
    root: PinnedDirectory,
    marker: PinnedFile,
    children: BTreeMap<ChildName, NodeIdentity>,
}

impl OriginalChildren {
    fn from_map(
        root: &Path,
        expected: [ChildName; 7],
        mut children: BTreeMap<ChildName, PinnedDirectory>,
    ) -> Result<Self, SandboxFsError> {
        let ordered = expected
            .into_iter()
            .map(|name| {
                let directory = children.remove(&name).ok_or_else(|| {
                    SandboxFsError::invalid(
                        &root.join(name.as_os_str()),
                        "the original sandbox child is missing",
                    )
                })?;
                Ok((name, directory))
            })
            .collect::<Result<Vec<_>, SandboxFsError>>()?;
        let ordered = ordered
            .try_into()
            .expect("seven expected sandbox children produce seven entries");
        Ok(Self(ordered))
    }
}

impl<C> ValidatedSandbox<C> {
    pub(super) fn root(&self) -> &PinnedDirectory {
        &self.root
    }

    pub(super) fn marker(&self) -> &PinnedFile {
        &self.marker
    }
}

impl ValidatedSandbox<OriginalChildren> {
    pub(super) fn new_original(
        root: PinnedDirectory,
        marker: PinnedFile,
        expected: [ChildName; 7],
        children: BTreeMap<ChildName, PinnedDirectory>,
    ) -> Result<Self, SandboxFsError> {
        let children = OriginalChildren::from_map(root.path(), expected, children)?;
        Ok(Self {
            root,
            marker,
            children,
        })
    }

    pub(super) fn children(&self) -> &[(ChildName, PinnedDirectory); 7] {
        &self.children.0
    }

    pub(super) fn into_rename_ticket(
        self,
        source: ChildName,
        destination: ChildName,
    ) -> RenameTicket {
        let Self {
            root,
            marker,
            children,
        } = self;
        let identity = root.identity();
        drop(children);
        drop(marker);
        drop(root);
        RenameTicket::new(source, destination, identity)
    }
}

impl ValidatedSandbox<CleanupChildren> {
    pub(super) fn new_cleanup(
        root: PinnedDirectory,
        marker: PinnedFile,
        children: BTreeMap<ChildName, PinnedDirectory>,
    ) -> Self {
        Self {
            root,
            marker,
            children: CleanupChildren(children),
        }
    }

    pub(super) fn children(&self) -> &BTreeMap<ChildName, PinnedDirectory> {
        &self.children.0
    }

    pub(super) fn into_cleanup_plan(self) -> CleanupPlan {
        CleanupPlan {
            root: self.root,
            marker: self.marker,
            children: self
                .children
                .0
                .into_iter()
                .map(|(name, directory)| (name, directory.identity()))
                .collect(),
        }
    }
}

impl CleanupPlan {
    pub(super) fn root(&self) -> &PinnedDirectory {
        &self.root
    }

    pub(super) fn marker(&self) -> &PinnedFile {
        &self.marker
    }

    pub(super) fn children(&self) -> &BTreeMap<ChildName, NodeIdentity> {
        &self.children
    }

    pub(super) fn into_root(self) -> PinnedDirectory {
        let Self {
            root,
            marker,
            children,
        } = self;
        drop(children);
        drop(marker);
        root
    }
}

impl EntryTicket {
    pub(super) fn name(&self) -> &ChildName {
        &self.name
    }

    pub(super) const fn identity(&self) -> NodeIdentity {
        self.identity
    }

    pub(super) const fn kind(&self) -> NodeKind {
        self.kind
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    pub(super) const fn reparse_tag(&self) -> Option<u32> {
        self.reparse_tag
    }
}

impl InitializationGuard {
    pub(super) fn acquire(
        parent_path: &Path,
        init_name: ChildName,
    ) -> Result<Self, SandboxFsError> {
        Self::acquire_with_ops(
            parent_path,
            init_name,
            |parent| {
                #[cfg(target_os = "linux")]
                return parent.lock();
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = parent;
                    Ok(())
                }
            },
            fs::File::lock,
        )
    }

    fn acquire_with_ops(
        parent_path: &Path,
        init_name: ChildName,
        lock_parent: impl FnOnce(&File) -> io::Result<()>,
        lock_init: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<Self, SandboxFsError> {
        let parent = PinnedDirectory::open_ambient_parent(parent_path)?;
        lock_parent(parent.file())
            .map_err(|error| SandboxFsError::io("lock namespace parent", parent_path, error))?;
        let (init_file, created) = parent.open_or_create_file(&init_name)?;
        lock_init(init_file.file()).map_err(|error| {
            SandboxFsError::io("lock initialization file", init_file.path(), error)
        })?;
        init_file.verify_at(&parent, &init_name)?;
        Ok(Self {
            parent,
            init_name,
            init_file,
            created,
        })
    }

    pub(super) fn parent(&self) -> &PinnedDirectory {
        &self.parent
    }

    pub(super) fn file(&self) -> &PinnedFile {
        &self.init_file
    }

    pub(super) fn name(&self) -> &ChildName {
        &self.init_name
    }

    pub(super) const fn created(&self) -> bool {
        self.created
    }

    pub(super) fn verify_core(&self) -> Result<(), SandboxFsError> {
        self.parent.verify_handle()?;
        self.init_file.verify_handle()?;
        self.init_file.verify_at(&self.parent, &self.init_name)
    }

    pub(super) fn release(
        self,
        validate: impl FnOnce(&InitializationGuard) -> Result<(), SandboxFsError>,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        self.release_with_ops(validate, fs::File::unlock, |parent| {
            #[cfg(target_os = "linux")]
            return parent.unlock();
            #[cfg(not(target_os = "linux"))]
            {
                let _ = parent;
                Ok(())
            }
        })
    }

    pub(super) fn release_with_ops(
        self,
        validate: impl FnOnce(&InitializationGuard) -> Result<(), SandboxFsError>,
        unlock_init: impl FnOnce(&File) -> io::Result<()>,
        unlock_parent: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        let validation = validate(&self).and_then(|()| self.verify_core());
        let init_unlock = unlock_init(self.init_file.file()).map_err(|error| {
            SandboxFsError::io("unlock initialization file", self.init_file.path(), error)
        });
        let parent_unlock = unlock_parent(self.parent.file()).map_err(|error| {
            SandboxFsError::io("unlock namespace parent", self.parent.path(), error)
        });
        let release = init_unlock.and(parent_unlock);
        let combined = combine_release(validation, release, |primary, release| {
            SandboxFsError::Dual {
                primary: Box::new(primary),
                release: Box::new(release),
            }
        });
        combined.map(|()| self.parent)
    }
}

impl ProfileLease {
    pub(super) fn open(
        leases: &PinnedDirectory,
        name: ChildName,
    ) -> Result<(Self, bool), SandboxFsError> {
        let (file, created) = leases.open_or_create_file(&name)?;
        Ok((
            Self {
                file,
                name,
                locked: false,
            },
            created,
        ))
    }

    pub(super) fn file(&self) -> &PinnedFile {
        &self.file
    }

    pub(super) fn try_lock(&mut self) -> Result<(), fs::TryLockError> {
        let result = self.file.file().try_lock();
        if result.is_ok() {
            self.locked = true;
        }
        result
    }

    pub(super) fn validate(&self, leases: &PinnedDirectory) -> Result<(), SandboxFsError> {
        self.file.verify_handle()?;
        self.file.verify_at(leases, &self.name)
    }

    pub(super) fn release(self, leases: &PinnedDirectory) -> Result<(), SandboxFsError> {
        self.release_with_ops(leases, fs::File::unlock)
    }

    pub(super) fn release_with_ops(
        self,
        leases: &PinnedDirectory,
        unlock: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<(), SandboxFsError> {
        let validation = self.validate(leases);
        let unlock = if self.locked {
            unlock(self.file.file()).map_err(|error| {
                SandboxFsError::io("unlock profile lease", self.file.path(), error)
            })
        } else {
            Ok(())
        };
        combine_release(validation, unlock, |primary, release| {
            SandboxFsError::Dual {
                primary: Box::new(primary),
                release: Box::new(release),
            }
        })
    }
}

#[derive(Debug)]
pub(super) struct PinnedFile {
    file: File,
    display_path: PathBuf,
    identity: NodeIdentity,
}

struct PublishOps<Truncate, Write, Sync> {
    truncate: Truncate,
    write: Write,
    sync: Sync,
}

fn no_publish_hook() {}

impl PinnedFile {
    fn from_file(file: File, display_path: PathBuf) -> Result<Self, SandboxFsError> {
        validate_opened_node(
            &file,
            &display_path,
            ExpectedNode::RegularFile { private: true },
        )?;
        let identity = file_identity(&file, &display_path)?;
        Ok(Self {
            file,
            display_path,
            identity,
        })
    }

    pub(super) fn file(&self) -> &File {
        &self.file
    }

    pub(super) fn path(&self) -> &Path {
        &self.display_path
    }

    pub(super) const fn identity(&self) -> NodeIdentity {
        self.identity
    }

    pub(super) fn verify_handle(&self) -> Result<(), SandboxFsError> {
        validate_opened_node(
            &self.file,
            &self.display_path,
            ExpectedNode::RegularFile { private: true },
        )
    }

    pub(super) fn verify_at(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
    ) -> Result<(), SandboxFsError> {
        let reopened = parent.open_file(name, false)?;
        if reopened.identity != self.identity {
            return Err(SandboxFsError::invalid(
                &self.display_path,
                "the child file identity changed",
            ));
        }
        Ok(())
    }

    pub(super) fn read_all(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
    ) -> Result<Vec<u8>, SandboxFsError> {
        use std::io::{Read as _, Seek as _};

        let mut file = self
            .file
            .try_clone()
            .map_err(|error| SandboxFsError::io("clone child file", &self.display_path, error))?;
        file.seek(io::SeekFrom::Start(0))
            .map_err(|error| SandboxFsError::io("seek child file", &self.display_path, error))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| SandboxFsError::io("read child file", &self.display_path, error))?;
        self.verify_at(parent, name)?;
        Ok(bytes)
    }

    pub(super) fn publish_bytes(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
        bytes: &[u8],
    ) -> Result<(), SandboxFsError> {
        self.publish_bytes_with_hook(parent, name, bytes, no_publish_hook)
    }

    fn publish_bytes_with_hook(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
        bytes: &[u8],
        after_sync: impl FnOnce(),
    ) -> Result<(), SandboxFsError> {
        self.publish_bytes_with_ops_and_hook(
            parent,
            name,
            bytes,
            PublishOps {
                truncate: |file: &File| file.set_len(0),
                write: std::io::Write::write_all,
                sync: File::sync_all,
            },
            after_sync,
        )
    }

    #[cfg(target_os = "linux")]
    pub(super) fn publish_bytes_with_ops(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
        bytes: &[u8],
        truncate: impl FnOnce(&File) -> io::Result<()>,
        write: impl FnOnce(&mut File, &[u8]) -> io::Result<()>,
        sync: impl FnOnce(&File) -> io::Result<()>,
    ) -> Result<(), SandboxFsError> {
        self.publish_bytes_with_ops_and_hook(
            parent,
            name,
            bytes,
            PublishOps {
                truncate,
                write,
                sync,
            },
            no_publish_hook,
        )
    }

    fn publish_bytes_with_ops_and_hook<Truncate, Write, Sync>(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
        bytes: &[u8],
        ops: PublishOps<Truncate, Write, Sync>,
        after_sync: impl FnOnce(),
    ) -> Result<(), SandboxFsError>
    where
        Truncate: FnOnce(&File) -> io::Result<()>,
        Write: FnOnce(&mut File, &[u8]) -> io::Result<()>,
        Sync: FnOnce(&File) -> io::Result<()>,
    {
        use std::io::Seek as _;

        let PublishOps {
            truncate,
            write,
            sync,
        } = ops;
        truncate(&self.file).map_err(|error| {
            SandboxFsError::io("truncate child file", &self.display_path, error)
        })?;
        let mut file = self
            .file
            .try_clone()
            .map_err(|error| SandboxFsError::io("clone child file", &self.display_path, error))?;
        file.seek(io::SeekFrom::Start(0))
            .map_err(|error| SandboxFsError::io("seek child file", &self.display_path, error))?;
        write(&mut file, bytes)
            .map_err(|error| SandboxFsError::io("write child file", &self.display_path, error))?;
        sync(&file)
            .map_err(|error| SandboxFsError::io("sync child file", &self.display_path, error))?;
        after_sync();
        self.verify_at(parent, name)?;
        let actual = self.read_all(parent, name)?;
        if actual != bytes {
            return Err(SandboxFsError::invalid(
                &self.display_path,
                "the published bytes changed after sync",
            ));
        }
        parent.sync()
    }
}

#[derive(Debug)]
pub(super) struct PinnedDirectory {
    file: File,
    display_path: PathBuf,
    identity: NodeIdentity,
    private: bool,
    #[cfg(windows)]
    capability: cap_std::fs::Dir,
    #[cfg(windows)]
    fence: fence_windows::DirectoryHandle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectorySyncFailureClass {
    Unsupported,
    Io,
}

const fn classify_directory_sync_failure(raw_os_error: Option<i32>) -> DirectorySyncFailureClass {
    match raw_os_error {
        Some(1 | 50 | 87 | 120) => DirectorySyncFailureClass::Unsupported,
        _ => DirectorySyncFailureClass::Io,
    }
}

fn directory_sync_error(path: &Path, error: io::Error) -> SandboxFsError {
    #[cfg(windows)]
    if classify_directory_sync_failure(error.raw_os_error())
        == DirectorySyncFailureClass::Unsupported
    {
        return SandboxFsError::Unsupported {
            operation: "sync directory",
            path: path.to_path_buf(),
            source: Box::new(error),
        };
    }
    SandboxFsError::io("sync directory", path, error)
}

impl PinnedDirectory {
    fn from_file(file: File, display_path: PathBuf, private: bool) -> Result<Self, SandboxFsError> {
        validate_opened_node(&file, &display_path, ExpectedNode::Directory { private })?;
        #[cfg(target_os = "linux")]
        let identity = file_identity(&file, &display_path)?;
        #[cfg(windows)]
        let (identity, capability, fence) = {
            let identity = file_identity(&file, &display_path)?;
            let capability_file = file
                .try_clone()
                .map_err(|error| SandboxFsError::io("clone directory", &display_path, error))?;
            let fence_file = file
                .try_clone()
                .map_err(|error| SandboxFsError::io("clone directory", &display_path, error))?;
            let capability = cap_std::fs::Dir::from_std_file(capability_file);
            let fence = fence_windows::DirectoryHandle::from_directory_file(fence_file)
                .map_err(|error| platform_error("pin directory", &display_path, error))?;
            if fence_identity(fence.metadata().identity) != identity {
                return Err(SandboxFsError::invalid(
                    &display_path,
                    "the adopted directory identity changed",
                ));
            }
            (identity, capability, fence)
        };
        #[cfg(not(any(target_os = "linux", windows)))]
        return Err(SandboxFsError::UnsupportedPlatform);
        #[cfg(any(target_os = "linux", windows))]
        Ok(Self {
            file,
            display_path,
            identity,
            private,
            #[cfg(windows)]
            capability,
            #[cfg(windows)]
            fence,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.display_path
    }

    pub(super) const fn identity(&self) -> NodeIdentity {
        self.identity
    }

    pub(super) fn verify_handle(&self) -> Result<(), SandboxFsError> {
        validate_opened_node(
            &self.file,
            &self.display_path,
            ExpectedNode::Directory {
                private: self.private,
            },
        )
    }

    fn file(&self) -> &File {
        &self.file
    }

    #[cfg(windows)]
    fn capability(&self) -> &cap_std::fs::Dir {
        &self.capability
    }

    #[cfg(windows)]
    fn fence(&self) -> &fence_windows::DirectoryHandle {
        &self.fence
    }

    pub(super) fn sync(&self) -> Result<(), SandboxFsError> {
        self.sync_with(File::sync_all)
    }

    fn sync_with(&self, sync: impl FnOnce(&File) -> io::Result<()>) -> Result<(), SandboxFsError> {
        sync(&self.file).map_err(|error| directory_sync_error(&self.display_path, error))
    }

    pub(super) fn verify_at(
        &self,
        parent: &PinnedDirectory,
        name: &ChildName,
    ) -> Result<(), SandboxFsError> {
        let reopened = parent.open_directory(name)?;
        if reopened.identity != self.identity {
            return Err(SandboxFsError::invalid(
                &self.display_path,
                "the child directory identity changed",
            ));
        }
        Ok(())
    }

    pub(super) fn tickets(&self) -> Result<Vec<EntryTicket>, SandboxFsError> {
        #[cfg(target_os = "linux")]
        let mut tickets = self.linux_tickets()?;
        #[cfg(windows)]
        let mut tickets = self.windows_tickets()?;
        #[cfg(not(any(target_os = "linux", windows)))]
        return Err(SandboxFsError::UnsupportedPlatform);
        #[cfg(any(target_os = "linux", windows))]
        {
            tickets.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(tickets)
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_tickets(&self) -> Result<Vec<EntryTicket>, SandboxFsError> {
        use std::os::unix::ffi::OsStrExt as _;

        let mut directory = rustix::fs::Dir::read_from(self.file())
            .map_err(io::Error::from)
            .map_err(|error| SandboxFsError::io("list directory", &self.display_path, error))?;
        let mut names = Vec::new();
        while let Some(entry) = directory.read() {
            let entry = entry
                .map_err(io::Error::from)
                .map_err(|error| SandboxFsError::io("list directory", &self.display_path, error))?;
            let raw = entry.file_name().to_bytes();
            if raw == b"." || raw == b".." {
                continue;
            }
            names.push(ChildName::new(OsStr::from_bytes(raw).to_os_string())?);
        }
        Self::present_tickets(names, |name| self.ticket_for_name(name))
    }

    /// Keep the ticket of every listed child that still exists.
    ///
    /// The scan reads the complete listing first and opens each name after that. Another process
    /// can remove a child after the listing and before its open. That child is absent from the
    /// directory now. A listing taken one moment later would omit it, so it gets no ticket. Every
    /// other open failure is an error.
    #[cfg(target_os = "linux")]
    fn present_tickets(
        names: Vec<ChildName>,
        mut open: impl FnMut(&ChildName) -> Result<EntryTicket, SandboxFsError>,
    ) -> Result<Vec<EntryTicket>, SandboxFsError> {
        let mut tickets = Vec::new();
        for name in &names {
            match open(name) {
                Ok(ticket) => tickets.push(ticket),
                Err(error) if error.is_not_found() => {}
                Err(error) => return Err(error),
            }
        }
        Ok(tickets)
    }

    #[cfg(windows)]
    fn windows_tickets(&self) -> Result<Vec<EntryTicket>, SandboxFsError> {
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;

        let volume = match self.identity {
            NodeIdentity::Windows { volume, .. } => volume,
            NodeIdentity::Unix { .. } => unreachable!("a Windows directory has a Windows identity"),
        };
        self.fence()
            .entries()
            .map_err(|error| platform_error("list directory", &self.display_path, error))?
            .into_iter()
            .map(|entry| {
                let kind = if entry.reparse_tag.is_some() {
                    NodeKind::LinkOrReparse
                } else if entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                    NodeKind::Directory
                } else {
                    NodeKind::RegularFile
                };
                Ok(EntryTicket {
                    name: ChildName::new(entry.name)?,
                    identity: NodeIdentity::Windows {
                        volume,
                        file_id: u128::from_ne_bytes(entry.file_id),
                    },
                    kind,
                    reparse_tag: entry.reparse_tag,
                })
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn ticket_for_name(&self, name: &ChildName) -> Result<EntryTicket, SandboxFsError> {
        let path = self.display_path.join(name.as_os_str());
        let file = fs_at::OpenOptions::default()
            .open_path_at(self.file(), name.as_os_str())
            .map_err(|error| SandboxFsError::io("open directory entry", &path, error))?;
        let metadata = file
            .metadata()
            .map_err(|error| SandboxFsError::io("inspect directory entry", &path, error))?;
        Ok(EntryTicket {
            name: name.clone(),
            identity: file_identity(&file, &path)?,
            kind: metadata_kind(&metadata),
            reparse_tag: None,
        })
    }

    pub(super) fn open_ticket_directory(
        &self,
        ticket: &EntryTicket,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        if ticket.kind != NodeKind::Directory {
            return Err(SandboxFsError::invalid(
                &self.display_path.join(ticket.name.as_os_str()),
                "the directory ticket does not name an ordinary directory",
            ));
        }
        let opened = self.open_directory_with_policy(&ticket.name, false)?;
        if opened.identity != ticket.identity {
            return Err(SandboxFsError::invalid(
                opened.path(),
                "the directory entry changed after enumeration",
            ));
        }
        Ok(opened)
    }

    pub(super) fn require_ticket_current(
        &self,
        ticket: &EntryTicket,
    ) -> Result<(), SandboxFsError> {
        let current = self
            .tickets()?
            .into_iter()
            .find(|current| current.name == ticket.name)
            .ok_or_else(|| {
                SandboxFsError::invalid(
                    &self.display_path.join(ticket.name.as_os_str()),
                    "the directory entry disappeared after enumeration",
                )
            })?;
        if current.identity != ticket.identity
            || current.kind != ticket.kind
            || current.reparse_tag != ticket.reparse_tag
        {
            return Err(SandboxFsError::invalid(
                &self.display_path.join(ticket.name.as_os_str()),
                "the directory entry changed after enumeration",
            ));
        }
        Ok(())
    }

    pub(super) fn remove_ticket(&self, ticket: EntryTicket) -> Result<(), SandboxFsError> {
        self.remove_ticket_with_hook(ticket, |_, _| Ok(()))
    }

    fn remove_ticket_with_hook(
        &self,
        ticket: EntryTicket,
        after_remove: impl FnOnce(&PinnedDirectory, &EntryTicket) -> Result<(), SandboxFsError>,
    ) -> Result<(), SandboxFsError> {
        self.require_ticket_current(&ticket)?;
        let path = self.display_path.join(ticket.name.as_os_str());
        self.remove_native_entry(&ticket, &path)?;
        after_remove(self, &ticket)?;
        if self
            .tickets()?
            .iter()
            .any(|current| current.name == ticket.name)
        {
            return Err(SandboxFsError::invalid(
                &path,
                "the removed directory entry is still present or was replaced",
            ));
        }
        self.sync()
    }

    /// Remove one directory entry with the platform operation.
    #[cfg(target_os = "linux")]
    fn remove_native_entry(&self, ticket: &EntryTicket, path: &Path) -> Result<(), SandboxFsError> {
        let options = fs_at::OpenOptions::default();
        let result = match ticket.kind {
            NodeKind::Directory => options.rmdir_at(self.file(), ticket.name.as_os_str()),
            NodeKind::RegularFile | NodeKind::LinkOrReparse | NodeKind::Other => {
                options.unlink_at(self.file(), ticket.name.as_os_str())
            }
        };
        result.map_err(|error| SandboxFsError::io("remove directory entry", path, error))
    }

    /// Remove one directory entry with the platform operation.
    #[cfg(windows)]
    fn remove_native_entry(&self, ticket: &EntryTicket, path: &Path) -> Result<(), SandboxFsError> {
        use fs_at::os::windows::{FileExt as _, OpenOptionsExt as _};
        use windows_sys::Win32::Storage::FileSystem::{
            DELETE, FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES,
        };

        let mut options = fs_at::OpenOptions::default();
        options.desired_access(DELETE | FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES);
        let opened = options
            .open_path_at(self.file(), ticket.name.as_os_str())
            .map_err(|error| SandboxFsError::io("open directory entry for removal", path, error))?;
        let identity = file_identity(&opened, path)?;
        let metadata = opened.metadata().map_err(|error| {
            SandboxFsError::io("inspect directory entry for removal", path, error)
        })?;
        if identity != ticket.identity || metadata_kind(&metadata) != ticket.kind {
            return Err(SandboxFsError::invalid(
                path,
                "the directory entry changed before removal",
            ));
        }
        opened
            .delete_by_handle()
            .map_err(|(_, error)| SandboxFsError::io("remove directory entry", path, error))
    }

    /// Refuse the removal. This platform has no sandbox filesystem support.
    #[cfg(not(any(target_os = "linux", windows)))]
    fn remove_native_entry(
        &self,
        _ticket: &EntryTicket,
        _path: &Path,
    ) -> Result<(), SandboxFsError> {
        Err(SandboxFsError::UnsupportedPlatform)
    }

    pub(super) fn remove_tree(&self, ticket: EntryTicket) -> Result<(), SandboxFsError> {
        self.remove_tree_with_hooks(ticket, &mut |_, _| Ok(()), &mut |_, _| Ok(()))
    }

    pub(super) fn rename_noreplace(
        &self,
        ticket: RenameTicket,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        self.rename_noreplace_with_hooks(ticket, |_, _| Ok(()), |_, _| Ok(()))
    }

    fn rename_noreplace_with_hooks(
        &self,
        ticket: RenameTicket,
        before_final_source_check: impl FnOnce(
            &PinnedDirectory,
            &RenameTicket,
        ) -> Result<(), SandboxFsError>,
        after_native_rename: impl FnOnce(&PinnedDirectory, &RenameTicket) -> Result<(), SandboxFsError>,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        let source_path = self.display_path.join(ticket.source.as_os_str());
        let destination_path = self.display_path.join(ticket.destination.as_os_str());
        let opened = self.open_directory(&ticket.source)?;
        if opened.identity != ticket.identity {
            return Err(SandboxFsError::invalid(
                &source_path,
                "the rename source identity changed",
            ));
        }
        let source_ticket = EntryTicket {
            name: ticket.source.clone(),
            identity: ticket.identity,
            kind: NodeKind::Directory,
            reparse_tag: None,
        };
        before_final_source_check(self, &ticket)?;
        self.require_ticket_current(&source_ticket)?;
        self.rename_native_noreplace(
            &ticket,
            opened,
            after_native_rename,
            &source_path,
            &destination_path,
        )
    }

    /// Rename the source with the platform operation and open the destination.
    #[cfg(target_os = "linux")]
    fn rename_native_noreplace(
        &self,
        ticket: &RenameTicket,
        source_handle: PinnedDirectory,
        after_native_rename: impl FnOnce(&PinnedDirectory, &RenameTicket) -> Result<(), SandboxFsError>,
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        rustix::fs::renameat_with(
            self.file(),
            ticket.source.as_os_str(),
            self.file(),
            ticket.destination.as_os_str(),
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(io::Error::from)
        .map_err(|error| SandboxFsError::io(RENAME_NOREPLACE_OPERATION, source_path, error))?;
        after_native_rename(self, ticket)?;
        self.sync()?;
        let moved = self.open_directory(&ticket.destination)?;
        if moved.identity != ticket.identity {
            return Err(SandboxFsError::invalid(
                destination_path,
                "the renamed sandbox identity changed",
            ));
        }
        drop(source_handle);
        Ok(moved)
    }

    /// Rename the source with the platform operation and open the destination.
    #[cfg(windows)]
    fn rename_native_noreplace(
        &self,
        ticket: &RenameTicket,
        source_handle: PinnedDirectory,
        after_native_rename: impl FnOnce(&PinnedDirectory, &RenameTicket) -> Result<(), SandboxFsError>,
        source_path: &Path,
        destination_path: &Path,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        drop(source_handle);
        self.fence()
            .rename_child_noreplace(ticket.source.as_os_str(), ticket.destination.as_os_str())
            .map_err(|error| platform_error(RENAME_NOREPLACE_OPERATION, source_path, error))?;
        after_native_rename(self, ticket)?;
        self.sync()?;
        if self
            .tickets()?
            .iter()
            .any(|current| current.name == ticket.source)
        {
            return Err(SandboxFsError::invalid(
                source_path,
                "the rename source is still present or was replaced",
            ));
        }
        let moved = self.open_directory(&ticket.destination)?;
        if moved.identity != ticket.identity {
            return Err(SandboxFsError::invalid(
                destination_path,
                "the renamed sandbox identity changed",
            ));
        }
        Ok(moved)
    }

    /// Refuse the rename. This platform has no sandbox filesystem support.
    #[cfg(not(any(target_os = "linux", windows)))]
    fn rename_native_noreplace(
        &self,
        _ticket: &RenameTicket,
        _source_handle: PinnedDirectory,
        _after_native_rename: impl FnOnce(&PinnedDirectory, &RenameTicket) -> Result<(), SandboxFsError>,
        _source_path: &Path,
        _destination_path: &Path,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        Err(SandboxFsError::UnsupportedPlatform)
    }

    /// Rename one private child directory to a sibling name without replacing any destination.
    pub(super) fn rename_child_directory_noreplace(
        &self,
        source: &ChildName,
        destination: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        self.rename_child_directory_noreplace_with_hook(source, destination, |_, _| Ok(()))
    }

    fn rename_child_directory_noreplace_with_hook(
        &self,
        source: &ChildName,
        destination: &ChildName,
        after_open: impl FnOnce(&PinnedDirectory, &ChildName) -> Result<(), SandboxFsError>,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        let opened = self.open_directory(source)?;
        let ticket = RenameTicket::new(source.clone(), destination.clone(), opened.identity());
        drop(opened);
        after_open(self, source)?;
        self.rename_noreplace(ticket)
    }

    #[cfg(target_os = "linux")]
    fn remove_tree_with_hook(
        &self,
        ticket: EntryTicket,
        hook: &mut impl FnMut(&PinnedDirectory, &EntryTicket) -> Result<(), SandboxFsError>,
    ) -> Result<(), SandboxFsError> {
        self.remove_tree_with_hooks(ticket, hook, &mut |_, _| Ok(()))
    }

    #[cfg(target_os = "linux")]
    fn remove_tree_with_post_children_hook(
        &self,
        ticket: EntryTicket,
        hook: &mut impl FnMut(&PinnedDirectory, &EntryTicket) -> Result<(), SandboxFsError>,
    ) -> Result<(), SandboxFsError> {
        self.remove_tree_with_hooks(ticket, &mut |_, _| Ok(()), hook)
    }

    fn remove_tree_with_hooks(
        &self,
        ticket: EntryTicket,
        before: &mut impl FnMut(&PinnedDirectory, &EntryTicket) -> Result<(), SandboxFsError>,
        after_children: &mut impl FnMut(&PinnedDirectory, &EntryTicket) -> Result<(), SandboxFsError>,
    ) -> Result<(), SandboxFsError> {
        before(self, &ticket)?;
        if ticket.kind == NodeKind::Directory {
            let child = self.open_ticket_directory(&ticket)?;
            for descendant in child.tickets()? {
                child.remove_tree_with_hooks(descendant, before, after_children)?;
            }
            after_children(&child, &ticket)?;
            if !child.tickets()?.is_empty() {
                return Err(SandboxFsError::invalid(
                    child.path(),
                    "the cleanup directory changed while it was being removed",
                ));
            }
        }
        self.remove_ticket(ticket)
    }

    pub(super) fn open_ambient_parent(path: &Path) -> Result<Self, SandboxFsError> {
        let file = open_ambient_parent_file(path)?;
        Self::from_file(file, path.to_path_buf(), false)
    }

    pub(super) fn open_file(
        &self,
        name: &ChildName,
        writable: bool,
    ) -> Result<PinnedFile, SandboxFsError> {
        let path = self.display_path.join(name.as_os_str());
        let file = self.open_native_file(name, writable, &path)?;
        PinnedFile::from_file(file, path)
    }

    /// Open one child file with the platform operation.
    #[cfg(target_os = "linux")]
    fn open_native_file(
        &self,
        name: &ChildName,
        writable: bool,
        path: &Path,
    ) -> Result<File, SandboxFsError> {
        use fs_at::{OpenOptions, OpenOptionsWriteMode};

        let mut options = OpenOptions::default();
        options.read(true).follow(false);
        if writable {
            options.write(OpenOptionsWriteMode::Write);
        }
        options
            .open_at(self.file(), name.as_os_str())
            .map_err(|error| SandboxFsError::io("open child file", path, error))
    }

    /// Open one child file with the platform operation.
    #[cfg(windows)]
    fn open_native_file(
        &self,
        name: &ChildName,
        writable: bool,
        path: &Path,
    ) -> Result<File, SandboxFsError> {
        self.open_windows_file(name, writable, false, path)
    }

    /// Do not open the child on this platform. It has no sandbox filesystem support.
    #[cfg(not(any(target_os = "linux", windows)))]
    fn open_native_file(
        &self,
        _name: &ChildName,
        _writable: bool,
        _path: &Path,
    ) -> Result<File, SandboxFsError> {
        Err(SandboxFsError::UnsupportedPlatform)
    }

    #[cfg(windows)]
    fn open_windows_file(
        &self,
        name: &ChildName,
        writable: bool,
        create_new: bool,
        path: &Path,
    ) -> Result<File, SandboxFsError> {
        use cap_std::fs::{OpenOptions, OpenOptionsExt as _};
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::{
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
                FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
            },
        };

        let can_write = writable || create_new;
        let mut access = GENERIC_READ | FILE_READ_ATTRIBUTES;
        if can_write {
            access |= GENERIC_WRITE | FILE_WRITE_ATTRIBUTES;
        }
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(can_write)
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
        if create_new {
            options.write(true).create_new(true);
        }
        let operation = if create_new {
            "create child file"
        } else {
            "open child file"
        };
        let file = self
            .capability()
            .open_with(name.as_os_str(), &options)
            .map(cap_std::fs::File::into_std)
            .map_err(|error| SandboxFsError::io(operation, path, error))?;
        validate_opened_node(&file, path, ExpectedNode::RegularFile { private: true })?;
        let _ = file_identity(&file, path)?;
        Ok(file)
    }

    #[cfg(windows)]
    fn open_windows_directory(
        &self,
        name: &ChildName,
        private: bool,
        path: &Path,
    ) -> Result<File, SandboxFsError> {
        use cap_std::fs::{OpenOptions, OpenOptionsExt as _};
        use windows_sys::Win32::{
            Foundation::GENERIC_WRITE,
            Storage::FileSystem::{
                FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD,
                FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
                FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
                FILE_WRITE_ATTRIBUTES,
            },
        };

        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .access_mode(
                GENERIC_WRITE
                    | FILE_LIST_DIRECTORY
                    | FILE_ADD_FILE
                    | FILE_ADD_SUBDIRECTORY
                    | FILE_DELETE_CHILD
                    | FILE_TRAVERSE
                    | FILE_READ_ATTRIBUTES
                    | FILE_WRITE_ATTRIBUTES,
            )
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
        let file = self
            .capability()
            .open_with(name.as_os_str(), &options)
            .map(cap_std::fs::File::into_std)
            .map_err(|error| SandboxFsError::io("open child directory", path, error))?;
        validate_opened_node(&file, path, ExpectedNode::Directory { private })?;
        let _ = file_identity(&file, path)?;
        Ok(file)
    }

    pub(super) fn create_file(&self, name: &ChildName) -> Result<PinnedFile, SandboxFsError> {
        self.create_file_with_hook(name, |_| {})
    }

    pub(super) fn open_or_create_file(
        &self,
        name: &ChildName,
    ) -> Result<(PinnedFile, bool), SandboxFsError> {
        match self.create_file(name) {
            Ok(file) => Ok((file, true)),
            Err(SandboxFsError::Io { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists =>
            {
                self.open_file(name, true).map(|file| (file, false))
            }
            Err(error) => Err(error),
        }
    }

    fn create_file_with_hook(
        &self,
        name: &ChildName,
        before_mode: impl FnOnce(&Path),
    ) -> Result<PinnedFile, SandboxFsError> {
        let path = self.display_path.join(name.as_os_str());
        let file = self.create_native_file(name, &path)?;
        before_mode(&path);
        set_private_mode(&file, &path, PRIVATE_FILE_MODE)?;
        PinnedFile::from_file(file, path)
    }

    /// Create one private child file with the platform operation.
    #[cfg(target_os = "linux")]
    fn create_native_file(&self, name: &ChildName, path: &Path) -> Result<File, SandboxFsError> {
        use fs_at::os::unix::OpenOptionsExt as _;
        use fs_at::{OpenOptions, OpenOptionsWriteMode};

        let mut options = OpenOptions::default();
        options
            .read(true)
            .write(OpenOptionsWriteMode::Write)
            .create_new(true)
            .follow(false);
        options.mode(PRIVATE_FILE_MODE);
        options
            .open_at(self.file(), name.as_os_str())
            .map_err(|error| SandboxFsError::io("create child file", path, error))
    }

    /// Create one private child file with the platform operation.
    #[cfg(windows)]
    fn create_native_file(&self, name: &ChildName, path: &Path) -> Result<File, SandboxFsError> {
        self.open_windows_file(name, true, true, path)
    }

    /// Do not create the child on this platform. It has no sandbox filesystem support.
    #[cfg(not(any(target_os = "linux", windows)))]
    fn create_native_file(&self, _name: &ChildName, _path: &Path) -> Result<File, SandboxFsError> {
        Err(SandboxFsError::UnsupportedPlatform)
    }

    pub(super) fn open_directory(
        &self,
        name: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        self.open_directory_with_policy(name, true)
    }

    /// Open one child directory of any mode. The open does not follow a link and it refuses every
    /// other node kind.
    pub(super) fn open_directory_shared(
        &self,
        name: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        self.open_directory_with_policy(name, false)
    }

    fn open_directory_with_policy(
        &self,
        name: &ChildName,
        private: bool,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        let path = self.display_path.join(name.as_os_str());
        let file = self.open_native_directory(name, private, &path)?;
        PinnedDirectory::from_file(file, path, private)
    }

    /// Open one child directory with the platform operation.
    #[cfg(target_os = "linux")]
    fn open_native_directory(
        &self,
        name: &ChildName,
        _private: bool,
        path: &Path,
    ) -> Result<File, SandboxFsError> {
        use fs_at::{OpenOptions, OpenOptionsWriteMode};

        let mut options = OpenOptions::default();
        options
            .read(true)
            .write(OpenOptionsWriteMode::Write)
            .follow(false);
        options
            .open_dir_at(self.file(), name.as_os_str())
            .map_err(|error| SandboxFsError::io("open child directory", path, error))
    }

    /// Open one child directory with the platform operation.
    #[cfg(windows)]
    fn open_native_directory(
        &self,
        name: &ChildName,
        private: bool,
        path: &Path,
    ) -> Result<File, SandboxFsError> {
        self.open_windows_directory(name, private, path)
    }

    /// Do not open the child on this platform. It has no sandbox filesystem support.
    #[cfg(not(any(target_os = "linux", windows)))]
    fn open_native_directory(
        &self,
        _name: &ChildName,
        _private: bool,
        _path: &Path,
    ) -> Result<File, SandboxFsError> {
        Err(SandboxFsError::UnsupportedPlatform)
    }

    /// Create one private child directory with the platform operation.
    #[cfg(target_os = "linux")]
    pub(super) fn create_directory(
        &self,
        name: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        self.create_linux_directory_with(
            name,
            linux_create_private_directory,
            no_directory_promotion_hook,
        )
    }

    /// Create one private child directory with the platform operation.
    #[cfg(windows)]
    pub(super) fn create_directory(
        &self,
        name: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        let path = self.display_path.join(name.as_os_str());
        let (file, created_identity) = {
            let mut options = fs_at::OpenOptions::default();
            options.create_new(true).follow(false);
            let provisional = options
                .mkdir_at(self.file(), name.as_os_str())
                .map_err(|error| SandboxFsError::io("create child directory", &path, error))?;
            // mkdir_at creates a directory but does not grant FILE_READ_ATTRIBUTES.
            // Keep its file ID. The promoted handle validates the metadata below.
            let identity = file_identity(&provisional, &path)?;
            drop(provisional);
            let file = self.open_windows_directory(name, true, &path)?;
            (file, identity)
        };
        let directory = PinnedDirectory::from_file(file, path.clone(), true)?;
        if directory.identity() != created_identity {
            return Err(SandboxFsError::invalid(
                &path,
                "the created directory identity changed before promotion",
            ));
        }
        Ok(directory)
    }

    /// Do not create the child on this platform. It has no sandbox filesystem support.
    #[cfg(not(any(target_os = "linux", windows)))]
    pub(super) fn create_directory(
        &self,
        _name: &ChildName,
    ) -> Result<PinnedDirectory, SandboxFsError> {
        Err(SandboxFsError::UnsupportedPlatform)
    }

    #[cfg(target_os = "linux")]
    fn create_linux_directory_with(
        &self,
        name: &ChildName,
        create: impl FnOnce(&File, &ChildName) -> io::Result<File>,
        after_identity: impl FnOnce(&PinnedDirectory),
    ) -> Result<PinnedDirectory, SandboxFsError> {
        let path = self.display_path.join(name.as_os_str());
        let file = create(self.file(), name)
            .map_err(|error| SandboxFsError::io("create child directory", &path, error))?;
        let directory = PinnedDirectory::from_file(file, path, true)?;
        if !directory.tickets()?.is_empty() {
            return Err(SandboxFsError::invalid(
                directory.path(),
                "the newly created directory is not empty",
            ));
        }
        after_identity(&directory);
        directory.verify_at(self, name)?;
        Ok(directory)
    }
}

/// Open the ambient namespace parent with the platform operation.
#[cfg(target_os = "linux")]
fn open_ambient_parent_file(path: &Path) -> Result<File, SandboxFsError> {
    rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map(File::from)
    .map_err(io::Error::from)
    .map_err(|error| SandboxFsError::io("open namespace parent", path, error))
}

/// Open the ambient namespace parent with the platform operation.
#[cfg(windows)]
fn open_ambient_parent_file(path: &Path) -> Result<File, SandboxFsError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::{
        Foundation::GENERIC_WRITE,
        Storage::FileSystem::{
            FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_ATTRIBUTES,
        },
    };

    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .access_mode(
            GENERIC_WRITE
                | FILE_LIST_DIRECTORY
                | FILE_ADD_FILE
                | FILE_ADD_SUBDIRECTORY
                | FILE_DELETE_CHILD
                | FILE_TRAVERSE
                | FILE_READ_ATTRIBUTES
                | FILE_WRITE_ATTRIBUTES,
        )
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| SandboxFsError::io("open namespace parent", path, error))
}

/// Do not open the child on this platform. It has no sandbox filesystem support.
#[cfg(not(any(target_os = "linux", windows)))]
fn open_ambient_parent_file(_path: &Path) -> Result<File, SandboxFsError> {
    Err(SandboxFsError::UnsupportedPlatform)
}

#[cfg(target_os = "linux")]
fn linux_create_private_directory(parent: &File, name: &ChildName) -> io::Result<File> {
    use fs_at::OpenOptions;
    use fs_at::os::unix::OpenOptionsExt as _;

    let mut options = OpenOptions::default();
    options.create_new(true).follow(false);
    options.mode(PRIVATE_DIRECTORY_MODE);
    options.mkdir_at(parent, name.as_os_str())
}

#[cfg(target_os = "linux")]
fn no_directory_promotion_hook(_: &PinnedDirectory) {}

#[cfg(target_os = "linux")]
fn file_identity(file: &File, path: &Path) -> Result<NodeIdentity, SandboxFsError> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file
        .metadata()
        .map_err(|error| SandboxFsError::io("inspect opened node", path, error))?;
    Ok(NodeIdentity::Unix {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(windows)]
fn file_identity(file: &File, path: &Path) -> Result<NodeIdentity, SandboxFsError> {
    let identity = fs_id::FileID::new(file)
        .map_err(|error| SandboxFsError::io("identify opened file", path, error))?;
    Ok(NodeIdentity::Windows {
        volume: identity.storage_id(),
        file_id: identity.internal_file_id(),
    })
}

#[cfg(not(any(target_os = "linux", windows)))]
fn file_identity(_file: &File, _path: &Path) -> Result<NodeIdentity, SandboxFsError> {
    Err(SandboxFsError::UnsupportedPlatform)
}

#[cfg(windows)]
fn fence_identity(identity: fence_windows::FileIdentity) -> NodeIdentity {
    NodeIdentity::Windows {
        volume: identity.volume_serial,
        file_id: u128::from_ne_bytes(identity.file_id),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsFailureKind {
    IdentityChanged,
    Win32(i32),
    NtStatus(i32),
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WindowsFailureClass {
    AlreadyExists,
    Invalid,
    Unsupported,
    Native,
}

fn classify_windows_failure(kind: WindowsFailureKind) -> WindowsFailureClass {
    const ERROR_INVALID_FUNCTION: i32 = 1;
    const ERROR_NOT_SUPPORTED: i32 = 50;
    const ERROR_FILE_EXISTS: i32 = 80;
    const ERROR_INVALID_PARAMETER: i32 = 87;
    const ERROR_CALL_NOT_IMPLEMENTED: i32 = 120;
    const ERROR_ALREADY_EXISTS: i32 = 183;
    const STATUS_NOT_IMPLEMENTED: i32 = 0xC000_0002_u32 as i32;
    const STATUS_INVALID_PARAMETER: i32 = 0xC000_000D_u32 as i32;
    const STATUS_INVALID_DEVICE_REQUEST: i32 = 0xC000_0010_u32 as i32;
    const STATUS_OBJECT_NAME_COLLISION: i32 = 0xC000_0035_u32 as i32;
    const STATUS_NOT_SUPPORTED: i32 = 0xC000_00BB_u32 as i32;

    match kind {
        WindowsFailureKind::IdentityChanged => WindowsFailureClass::Invalid,
        WindowsFailureKind::Win32(ERROR_FILE_EXISTS | ERROR_ALREADY_EXISTS)
        | WindowsFailureKind::NtStatus(STATUS_OBJECT_NAME_COLLISION) => {
            WindowsFailureClass::AlreadyExists
        }
        WindowsFailureKind::Win32(
            ERROR_INVALID_FUNCTION
            | ERROR_NOT_SUPPORTED
            | ERROR_INVALID_PARAMETER
            | ERROR_CALL_NOT_IMPLEMENTED,
        )
        | WindowsFailureKind::NtStatus(
            STATUS_NOT_IMPLEMENTED
            | STATUS_INVALID_PARAMETER
            | STATUS_INVALID_DEVICE_REQUEST
            | STATUS_NOT_SUPPORTED,
        ) => WindowsFailureClass::Unsupported,
        WindowsFailureKind::Win32(_)
        | WindowsFailureKind::NtStatus(_)
        | WindowsFailureKind::Other => WindowsFailureClass::Native,
    }
}

#[cfg(windows)]
fn platform_error(
    operation: &'static str,
    path: &Path,
    error: fence_windows::WindowsError,
) -> SandboxFsError {
    use fence_windows::WindowsError;

    match error {
        WindowsError::IdentityChanged => {
            SandboxFsError::invalid(path, "the Windows directory entry identity changed")
        }
        WindowsError::Io { source, .. } => {
            match source
                .raw_os_error()
                .map_or(WindowsFailureKind::Other, WindowsFailureKind::Win32)
            {
                kind if classify_windows_failure(kind) == WindowsFailureClass::AlreadyExists => {
                    SandboxFsError::io(
                        operation,
                        path,
                        io::Error::new(io::ErrorKind::AlreadyExists, source),
                    )
                }
                kind if classify_windows_failure(kind) == WindowsFailureClass::Unsupported => {
                    SandboxFsError::Unsupported {
                        operation,
                        path: path.to_path_buf(),
                        source: Box::new(source),
                    }
                }
                _ => SandboxFsError::io(operation, path, source),
            }
        }
        error @ (WindowsError::NativeStatus { status, .. }
        | WindowsError::NtStatus { status, .. }) => {
            match classify_windows_failure(WindowsFailureKind::NtStatus(status)) {
                WindowsFailureClass::AlreadyExists => SandboxFsError::io(
                    operation,
                    path,
                    io::Error::from(io::ErrorKind::AlreadyExists),
                ),
                WindowsFailureClass::Unsupported => SandboxFsError::Unsupported {
                    operation,
                    path: path.to_path_buf(),
                    source: Box::new(error),
                },
                WindowsFailureClass::Native | WindowsFailureClass::Invalid => {
                    SandboxFsError::NativeStatus {
                        operation,
                        path: path.to_path_buf(),
                        status,
                    }
                }
            }
        }
        WindowsError::Path(_)
        | WindowsError::Malformed(_)
        | WindowsError::TooLarge(_)
        | WindowsError::NotDirectory
        | WindowsError::PrivateDirectoryReparse => SandboxFsError::invalid(path, error.to_string()),
    }
}

fn validate_opened_node(
    file: &File,
    path: &Path,
    expected: ExpectedNode,
) -> Result<(), SandboxFsError> {
    let metadata = file
        .metadata()
        .map_err(|error| SandboxFsError::io("inspect opened node", path, error))?;
    let actual = metadata_kind(&metadata);
    if actual != expected.kind() {
        return Err(SandboxFsError::invalid(
            path,
            format!(
                "the opened node is {actual:?}, expected {:?}",
                expected.kind()
            ),
        ));
    }
    if let Some(mode) = expected.private_mode() {
        validate_private_mode(&metadata, path, mode)?;
    }
    Ok(())
}

fn metadata_kind(metadata: &fs::Metadata) -> NodeKind {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return NodeKind::LinkOrReparse;
        }
    }
    if metadata.file_type().is_symlink() {
        NodeKind::LinkOrReparse
    } else if metadata.is_file() {
        NodeKind::RegularFile
    } else if metadata.is_dir() {
        NodeKind::Directory
    } else {
        NodeKind::Other
    }
}

#[cfg(unix)]
fn validate_private_mode(
    metadata: &fs::Metadata,
    path: &Path,
    expected: u32,
) -> Result<(), SandboxFsError> {
    use std::os::unix::fs::PermissionsExt as _;

    let actual = metadata.permissions().mode() & 0o7777;
    if actual != expected {
        return Err(SandboxFsError::invalid(
            path,
            format!("the mode is {actual:04o}, expected {expected:04o}"),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_mode(
    _metadata: &fs::Metadata,
    _path: &Path,
    _expected: u32,
) -> Result<(), SandboxFsError> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn set_private_mode(file: &File, path: &Path, mode: u32) -> Result<(), SandboxFsError> {
    rustix::fs::fchmod(file, rustix::fs::Mode::from_raw_mode(mode))
        .map_err(io::Error::from)
        .map_err(|error| SandboxFsError::io("set private mode", path, error))
}

#[cfg(windows)]
fn set_private_mode(_file: &File, _path: &Path, _mode: u32) -> Result<(), SandboxFsError> {
    Ok(())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn set_private_mode(_file: &File, _path: &Path, _mode: u32) -> Result<(), SandboxFsError> {
    Err(SandboxFsError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    static PROFILE_UNLOCK_CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn child(name: &'static str) -> ChildName {
        ChildName::literal(name)
    }

    fn first_error<E>(primary: E, _release: E) -> E {
        primary
    }

    #[cfg(target_os = "linux")]
    fn ok_file_op(_file: &File) -> io::Result<()> {
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn denied_file_op(_file: &File) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected syscall failure",
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn counted_denied_profile_unlock(_file: &File) -> io::Result<()> {
        PROFILE_UNLOCK_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "injected profile unlock failure",
        ))
    }

    #[cfg(target_os = "linux")]
    fn ok_rename_hook(
        _parent: &PinnedDirectory,
        _ticket: &RenameTicket,
    ) -> Result<(), SandboxFsError> {
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn opened_temp_parent() -> (tempfile::TempDir, PinnedDirectory) {
        let temporary = tempfile::TempDir::new().unwrap();
        let parent = PinnedDirectory::open_ambient_parent(temporary.path()).unwrap();
        (temporary, parent)
    }

    #[test]
    fn child_names_are_exactly_one_normal_component() {
        assert_eq!(
            SandboxFsError::UnsupportedPlatform.to_string(),
            "stable sandbox filesystem is unsupported"
        );
        assert_ne!(
            NodeIdentity::Unix {
                device: 1,
                inode: 2,
            },
            NodeIdentity::Windows {
                volume: 1,
                file_id: 2,
            }
        );
        assert!(ChildName::new("profile").is_ok());
        assert!(ChildName::new_with_windows_rules("profile", true).is_ok());
        assert!(ChildName::new_with_windows_rules("stream:name", false).is_ok());
        for invalid in ["", ".", "..", "a/b", "nul\0suffix"] {
            assert!(ChildName::new(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(ChildName::new_with_windows_rules("stream:name", true).is_err());
    }

    #[test]
    fn windows_failure_codes_keep_collision_identity_and_unsupported_distinct() {
        for code in [80, 183] {
            assert_eq!(
                classify_windows_failure(WindowsFailureKind::Win32(code)),
                WindowsFailureClass::AlreadyExists
            );
        }
        assert_eq!(
            classify_windows_failure(WindowsFailureKind::NtStatus(0xC000_0035_u32 as i32,)),
            WindowsFailureClass::AlreadyExists
        );
        assert_eq!(
            classify_windows_failure(WindowsFailureKind::IdentityChanged),
            WindowsFailureClass::Invalid
        );
        for code in [1, 50, 87, 120] {
            assert_eq!(
                classify_windows_failure(WindowsFailureKind::Win32(code)),
                WindowsFailureClass::Unsupported
            );
        }
        for status in [
            0xC000_0002_u32 as i32,
            0xC000_000D_u32 as i32,
            0xC000_0010_u32 as i32,
            0xC000_00BB_u32 as i32,
        ] {
            assert_eq!(
                classify_windows_failure(WindowsFailureKind::NtStatus(status)),
                WindowsFailureClass::Unsupported
            );
        }
        assert_eq!(
            classify_windows_failure(WindowsFailureKind::Win32(5)),
            WindowsFailureClass::Native
        );
        assert_eq!(
            classify_windows_failure(WindowsFailureKind::NtStatus(-1)),
            WindowsFailureClass::Native
        );
        assert_eq!(
            classify_windows_failure(WindowsFailureKind::Other),
            WindowsFailureClass::Native
        );

        let path = PathBuf::from("unsupported");
        let unsupported = SandboxFsError::Unsupported {
            operation: "rename",
            path: path.clone(),
            source: Box::new(io::Error::new(io::ErrorKind::Unsupported, "not supported")),
        };
        assert!(unsupported.to_string().contains("not supported"));
        assert!(std::error::Error::source(&unsupported).is_some());
        let native = SandboxFsError::NativeStatus {
            operation: "rename",
            path,
            status: -1,
        };
        assert!(native.to_string().contains("0xffffffff"));
        assert!(std::error::Error::source(&native).is_none());
        let invalid = SandboxFsError::invalid(Path::new("invalid"), "bad evidence");
        assert!(invalid.to_string().contains("bad evidence"));
        let io = SandboxFsError::io(
            "open",
            Path::new("io"),
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        );
        assert!(io.to_string().contains("could not open io"));
        assert!(std::error::Error::source(&io).is_some());
        let dual = SandboxFsError::Dual {
            primary: Box::new(invalid),
            release: Box::new(io),
        };
        assert!(dual.to_string().contains("release also failed"));
        assert!(std::error::Error::source(&dual).is_some());
    }

    #[test]
    fn release_combiner_keeps_all_four_primary_and_release_results() {
        assert_eq!(combine_release::<&str>(Ok(()), Ok(()), first_error), Ok(()));
        assert_eq!(
            combine_release(Err("primary"), Ok(()), first_error),
            Err("primary")
        );
        assert_eq!(
            combine_release(Ok(()), Err("release"), first_error),
            Err("release")
        );
        assert_eq!(
            combine_release(Err("primary"), Err("release"), first_error),
            Err("primary")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn lock_unlock_and_truncate_syscall_seams_keep_typed_io_failures() {
        let temporary = tempfile::TempDir::new().unwrap();
        let parent_lock = InitializationGuard::acquire_with_ops(
            temporary.path(),
            child("parent-failure.lock"),
            denied_file_op,
            ok_file_op,
        )
        .unwrap_err();
        assert!(matches!(
            parent_lock,
            SandboxFsError::Io {
                operation: "lock namespace parent",
                ..
            }
        ));
        let init_lock = InitializationGuard::acquire_with_ops(
            temporary.path(),
            child("init-failure.lock"),
            ok_file_op,
            denied_file_op,
        )
        .unwrap_err();
        assert!(matches!(
            init_lock,
            SandboxFsError::Io {
                operation: "lock initialization file",
                ..
            }
        ));

        let guard =
            InitializationGuard::acquire(temporary.path(), child("init-unlock-failure.lock"))
                .unwrap();
        let release = guard
            .release_with_ops(|_| Ok(()), denied_file_op, ok_file_op)
            .unwrap_err();
        assert!(matches!(
            release,
            SandboxFsError::Io {
                operation: "unlock initialization file",
                ..
            }
        ));
        let guard =
            InitializationGuard::acquire(temporary.path(), child("parent-unlock-failure.lock"))
                .unwrap();
        let release = guard
            .release_with_ops(|_| Ok(()), ok_file_op, denied_file_op)
            .unwrap_err();
        assert!(matches!(
            release,
            SandboxFsError::Io {
                operation: "unlock namespace parent",
                ..
            }
        ));
        let guard =
            InitializationGuard::acquire(temporary.path(), child("dual-release.lock")).unwrap();
        let release = guard
            .release_with_ops(
                |_| Err(SandboxFsError::invalid(Path::new("primary"), "primary")),
                denied_file_op,
                ok_file_op,
            )
            .unwrap_err();
        assert!(matches!(release, SandboxFsError::Dual { .. }));

        let parent = PinnedDirectory::open_ambient_parent(temporary.path()).unwrap();
        let leases = parent
            .create_directory(&child("leases-for-unlock"))
            .unwrap();
        let (mut lease, _) = ProfileLease::open(&leases, child("lease.lock")).unwrap();
        lease.try_lock().unwrap();
        let release = lease.release_with_ops(&leases, denied_file_op).unwrap_err();
        assert!(matches!(
            release,
            SandboxFsError::Io {
                operation: "unlock profile lease",
                ..
            }
        ));
        let (mut lease, _) = ProfileLease::open(&leases, child("dual-lease.lock")).unwrap();
        lease.try_lock().unwrap();
        let parked_lease = leases.path().join("parked-dual-lease.lock");
        fs::rename(leases.path().join("dual-lease.lock"), &parked_lease).unwrap();
        let release = lease.release_with_ops(&leases, denied_file_op).unwrap_err();
        assert!(matches!(release, SandboxFsError::Dual { .. }));
        assert!(parked_lease.is_file());

        let file_name = child("truncate-failure");
        let file = parent.create_file(&file_name).unwrap();
        file.publish_bytes(&parent, &file_name, b"original")
            .unwrap();
        let error = file
            .publish_bytes_with_ops(
                &parent,
                &file_name,
                b"changed",
                denied_file_op,
                std::io::Write::write_all,
                File::sync_all,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            SandboxFsError::Io {
                operation: "truncate child file",
                ..
            }
        ));
        assert_eq!(fs::read(file.path()).unwrap(), b"original");
    }

    #[test]
    fn windows_backend_source_keeps_the_audited_capability_surface() {
        let source = include_str!("tui_real_sandbox_fs.rs");
        for forbidden in [
            [".open_", "child("].concat(),
            [".open_mutation_", "directory("].concat(),
            [".remove_", "child("].concat(),
            ["verify_", "path_identity"].concat(),
        ] {
            assert!(
                !source.contains(&forbidden),
                "the backend uses forbidden Fence API {forbidden}"
            );
        }
        assert!(source.contains("cap_std::fs::Dir::from_std_file"));
        assert!(source.contains(".share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)"));
        assert!(source.contains(".custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)"));
        assert!(
            source.contains(
                ".custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)"
            )
        );
        assert!(source.contains("options.write(true).create_new(true)"));
        assert!(source.contains(".rename_child_noreplace("));
        assert!(source.contains(".delete_by_handle()"));
        assert!(!source.contains(&["io::Error::", "other(error)"].concat()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn relative_creation_stays_bound_to_a_parked_parent() {
        let container = tempfile::TempDir::new().unwrap();
        let named_parent = container.path().join("owned-parent");
        let parked_parent = container.path().join("parked-parent");
        let outside = container.path().join("outside");
        fs::create_dir(&named_parent).unwrap();
        fs::create_dir(&outside).unwrap();
        let parent = PinnedDirectory::open_ambient_parent(&named_parent).unwrap();
        fs::rename(&named_parent, &parked_parent).unwrap();
        std::os::unix::fs::symlink(&outside, &named_parent).unwrap();

        let created = parent.create_file(&child("created")).unwrap();

        assert!(parked_parent.join("created").is_file());
        assert!(!outside.join("created").exists());
        assert_eq!(
            created.identity(),
            file_identity(created.file(), created.path()).unwrap()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn handle_chmod_does_not_change_a_path_replacement() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, parent) = opened_temp_parent();
        let parked = temporary.path().join("parked-file");
        let created = parent
            .create_file_with_hook(&child("mode-file"), |path| {
                fs::rename(path, &parked).unwrap();
                fs::write(path, b"replacement").unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
            })
            .unwrap();

        assert_eq!(
            fs::metadata(&parked).unwrap().permissions().mode() & 0o7777,
            0o600
        );
        assert_eq!(
            fs::metadata(temporary.path().join("mode-file"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o644
        );
        assert!(created.verify_at(&parent, &child("mode-file")).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_directory_creation_promotes_exact_mode_before_identity_recheck() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, parent) = opened_temp_parent();
        let ordinary = parent.create_directory(&child("ordinary-private")).unwrap();
        assert_eq!(
            fs::metadata(ordinary.path()).unwrap().permissions().mode() & 0o7777,
            PRIVATE_DIRECTORY_MODE
        );
        ordinary
            .verify_at(&parent, &child("ordinary-private"))
            .unwrap();

        let raced_name = child("mkdir-open-race");
        let raced_path = temporary.path().join(raced_name.as_os_str());
        let parked_path = temporary.path().join("parked-mkdir-open-race");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);
        let raced = parent.create_linux_directory_with(
            &raced_name,
            |parent_file, name| {
                let created = linux_create_private_directory(parent_file, name)?;
                fs::set_permissions(&raced_path, fs::Permissions::from_mode(0o700))?;
                original_identity.set(Some(file_identity(&created, &raced_path).unwrap()));
                fs::rename(&raced_path, &parked_path)?;
                fs::create_dir(&raced_path)?;
                fs::set_permissions(&raced_path, fs::Permissions::from_mode(0o755))?;
                let mut options = fs_at::OpenOptions::default();
                options.read(true).follow(false);
                let replacement = options.open_dir_at(parent_file, name.as_os_str())?;
                replacement_identity.set(Some(file_identity(&replacement, &raced_path).unwrap()));
                Ok(replacement)
            },
            no_directory_promotion_hook,
        );
        assert!(matches!(raced, Err(SandboxFsError::Invalid { .. })));
        assert_eq!(
            fs::metadata(&parked_path).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert_eq!(
            fs::metadata(&raced_path).unwrap().permissions().mode() & 0o7777,
            0o755
        );
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );

        let promoted_name = child("promotion-race");
        let promoted_path = temporary.path().join(promoted_name.as_os_str());
        let parked_promoted = temporary.path().join("parked-promotion-race");
        let promoted = parent.create_linux_directory_with(
            &promoted_name,
            linux_create_private_directory,
            |directory| {
                fs::rename(directory.path(), &parked_promoted).unwrap();
                fs::create_dir(&promoted_path).unwrap();
                fs::set_permissions(&promoted_path, fs::Permissions::from_mode(0o700)).unwrap();
            },
        );
        assert!(matches!(promoted, Err(SandboxFsError::Invalid { .. })));
        assert!(parked_promoted.is_dir());
        assert!(promoted_path.is_dir());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_directory_create_refuses_nonempty_exact_mode_replacement() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, parent) = opened_temp_parent();
        let name = child("nonempty-promotion-race");
        let path = temporary.path().join(name.as_os_str());
        let parked = temporary.path().join("parked-nonempty-promotion-race");
        let original_identity = std::cell::Cell::new(None);
        let replacement_identity = std::cell::Cell::new(None);

        let result = parent.create_linux_directory_with(
            &name,
            |parent_file, name| {
                let created = linux_create_private_directory(parent_file, name)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                original_identity.set(Some(file_identity(&created, &path).unwrap()));
                fs::rename(&path, &parked)?;
                fs::create_dir(&path)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
                fs::write(path.join("canary"), b"replacement canary")?;
                let mut options = fs_at::OpenOptions::default();
                options.read(true).follow(false);
                let replacement = options.open_dir_at(parent_file, name.as_os_str())?;
                replacement_identity.set(Some(file_identity(&replacement, &path).unwrap()));
                Ok(replacement)
            },
            no_directory_promotion_hook,
        );

        assert!(matches!(result, Err(SandboxFsError::Invalid { .. })));
        assert_eq!(
            fs::metadata(&parked).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert_ne!(
            original_identity.get().unwrap(),
            replacement_identity.get().unwrap()
        );
        assert_eq!(
            fs::read(path.join("canary")).unwrap(),
            b"replacement canary"
        );
        assert!(fs::read_dir(&parked).unwrap().next().is_none());
    }

    #[test]
    fn directory_sync_failure_policy_is_pure_and_linux_io_is_typed() {
        for code in [1, 50, 87, 120] {
            assert_eq!(
                classify_directory_sync_failure(Some(code)),
                DirectorySyncFailureClass::Unsupported
            );
        }
        for code in [None, Some(5), Some(80), Some(183)] {
            assert_eq!(
                classify_directory_sync_failure(code),
                DirectorySyncFailureClass::Io
            );
        }

        #[cfg(target_os = "linux")]
        {
            let (_temporary, parent) = opened_temp_parent();
            let identity = parent.identity();
            let calls = std::cell::Cell::new(0);
            let error = parent
                .sync_with(|_| {
                    calls.set(calls.get() + 1);
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected directory sync failure",
                    ))
                })
                .unwrap_err();
            assert_eq!(calls.get(), 1);
            assert!(matches!(
                error,
                SandboxFsError::Io {
                    operation: "sync directory",
                    ..
                }
            ));
            assert_eq!(parent.identity(), identity);
            parent.verify_handle().unwrap();
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_directory_sync_uses_the_retained_handle_and_preserves_unmarked_evidence() {
        let (_temporary, parent) = opened_temp_parent();
        let private_name = child("private");
        let private = parent.create_directory(&private_name).unwrap();
        let parent_identity = parent.identity();
        let private_identity = private.identity();

        for directory in [&parent, &private] {
            match directory.sync() {
                Ok(()) => {}
                Err(SandboxFsError::Unsupported {
                    operation: "sync directory",
                    source,
                    ..
                }) => assert!(source.downcast_ref::<io::Error>().is_some()),
                Err(error) => panic!("unexpected directory sync result: {error}"),
            }
        }
        assert_eq!(parent.identity(), parent_identity);
        assert_eq!(private.identity(), private_identity);
        private.verify_at(&parent, &private_name).unwrap();

        let namespace_name = child("namespace");
        let namespace = parent.create_directory(&namespace_name).unwrap();
        let leases_name = child("leases");
        let leases = namespace.create_directory(&leases_name).unwrap();
        let sandboxes_name = child("sandboxes");
        let sandboxes = namespace.create_directory(&sandboxes_name).unwrap();
        let namespace_identity = namespace.identity();
        let leases_identity = leases.identity();
        let sandboxes_identity = sandboxes.identity();

        for code in [1, 50, 87, 120] {
            let calls = std::cell::Cell::new(0);
            let error = namespace
                .sync_with(|_| {
                    calls.set(calls.get() + 1);
                    Err(io::Error::from_raw_os_error(code))
                })
                .unwrap_err();
            assert_eq!(calls.get(), 1);
            match error {
                SandboxFsError::Unsupported {
                    operation: "sync directory",
                    source,
                    ..
                } => assert_eq!(
                    source.downcast_ref::<io::Error>().unwrap().raw_os_error(),
                    Some(code)
                ),
                error => panic!("unexpected injected directory sync error: {error}"),
            }
        }
        let error = namespace
            .sync_with(|_| Err(io::Error::from_raw_os_error(5)))
            .unwrap_err();
        assert!(matches!(
            error,
            SandboxFsError::Io {
                operation: "sync directory",
                ..
            }
        ));
        assert_eq!(namespace.identity(), namespace_identity);
        assert_eq!(leases.identity(), leases_identity);
        assert_eq!(sandboxes.identity(), sandboxes_identity);
        leases.verify_at(&namespace, &leases_name).unwrap();
        sandboxes.verify_at(&namespace, &sandboxes_name).unwrap();
        assert!(
            namespace
                .tickets()
                .unwrap()
                .iter()
                .all(|ticket| ticket.name() != &child("ownership-marker"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn private_modes_reject_special_bits() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, parent) = opened_temp_parent();
        let directory = parent.create_directory(&child("special-mode")).unwrap();
        fs::set_permissions(
            temporary.path().join("special-mode"),
            fs::Permissions::from_mode(0o1700),
        )
        .unwrap();

        assert!(directory.verify_handle().is_err());
        assert!(parent.open_directory(&child("special-mode")).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn present_tickets_skips_an_absent_child_and_keeps_every_other_answer() {
        fn unix_ticket(name: ChildName, inode: u64) -> EntryTicket {
            EntryTicket {
                name,
                identity: NodeIdentity::Unix { device: 7, inode },
                kind: NodeKind::Directory,
                reparse_tag: None,
            }
        }
        fn open_failure(name: &ChildName, kind: io::ErrorKind) -> SandboxFsError {
            SandboxFsError::io(
                "open directory entry",
                &Path::new("sandboxes").join(name.as_os_str()),
                io::Error::from(kind),
            )
        }

        let names = || vec![child("a"), child("b"), child("c")];
        let kept = PinnedDirectory::present_tickets(names(), |name| {
            if name == &child("b") {
                Err(open_failure(name, io::ErrorKind::NotFound))
            } else if name == &child("a") {
                Ok(unix_ticket(name.clone(), 10))
            } else {
                Ok(unix_ticket(name.clone(), 12))
            }
        })
        .unwrap();
        assert_eq!(
            kept,
            vec![unix_ticket(child("a"), 10), unix_ticket(child("c"), 12)]
        );

        let error = PinnedDirectory::present_tickets(names(), |name| {
            if name == &child("b") {
                Err(open_failure(name, io::ErrorKind::PermissionDenied))
            } else {
                Ok(unix_ticket(name.clone(), 10))
            }
        })
        .unwrap_err();
        assert!(matches!(
            &error,
            SandboxFsError::Io { source, .. } if source.kind() == io::ErrorKind::PermissionDenied
        ));
        assert!(error.to_string().contains("sandboxes/b"));
        assert!(!error.is_not_found());

        let none = PinnedDirectory::present_tickets(names(), |name| {
            Err(open_failure(name, io::ErrorKind::NotFound))
        })
        .unwrap();
        assert!(none.is_empty());
        assert!(open_failure(&child("a"), io::ErrorKind::NotFound).is_not_found());
        assert!(
            !SandboxFsError::invalid(Path::new("sandboxes"), "the listing is not an error")
                .is_not_found()
        );
    }

    /// The real open of an absent name reports `NotFound`, and the scan omits that name.
    ///
    /// The injected test above pins the policy. This one pins the error mapping of the real open,
    /// so a change that reports an absent entry through another error variant fails here.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_real_scan_omits_a_name_that_is_absent_at_its_open() {
        let (_temporary, parent) = opened_temp_parent();
        let present = child("present");
        parent.create_file(&present).unwrap();
        let absent = child("absent");
        assert!(parent.ticket_for_name(&absent).unwrap_err().is_not_found());

        let tickets = PinnedDirectory::present_tickets(vec![present.clone(), absent], |name| {
            parent.ticket_for_name(name)
        })
        .unwrap();
        assert_eq!(
            tickets.iter().map(EntryTicket::name).collect::<Vec<_>>(),
            vec![&present]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tickets_kinds_and_retained_identities_refuse_real_replacements() {
        let (temporary, parent) = opened_temp_parent();
        let file_name = child("ticket-file");
        let file = parent.create_file(&file_name).unwrap();
        let file_ticket = parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &file_name)
            .unwrap();
        assert!(parent.open_ticket_directory(&file_ticket).is_err());
        assert!(parent.open_directory(&file_name).is_err());
        assert_eq!(
            file.identity(),
            file_identity(file.file(), file.path()).unwrap()
        );
        file.verify_handle().unwrap();
        drop(file);

        let parked_file = temporary.path().join("parked-ticket-file");
        fs::rename(temporary.path().join("ticket-file"), &parked_file).unwrap();
        assert!(parent.require_ticket_current(&file_ticket).is_err());
        fs::write(temporary.path().join("ticket-file"), b"replacement").unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            temporary.path().join("ticket-file"),
            fs::Permissions::from_mode(PRIVATE_FILE_MODE),
        )
        .unwrap();
        assert!(parent.require_ticket_current(&file_ticket).is_err());
        assert!(parked_file.is_file());

        let directory_name = child("ticket-directory");
        let directory = parent.create_directory(&directory_name).unwrap();
        let directory_identity = directory.identity();
        let directory_ticket = parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &directory_name)
            .unwrap();
        let wrong_ticket = EntryTicket {
            identity: file_ticket.identity(),
            ..directory_ticket.clone()
        };
        assert!(parent.open_ticket_directory(&wrong_ticket).is_err());
        assert!(parent.open_file(&directory_name, false).is_err());
        assert!(
            parent
                .rename_noreplace(RenameTicket::new(
                    directory_name.clone(),
                    child("destination"),
                    file_ticket.identity(),
                ))
                .is_err()
        );
        assert_eq!(directory.identity(), directory_identity);
        assert_eq!(
            directory.identity(),
            file_identity(directory.file(), directory.path()).unwrap()
        );
        directory.verify_handle().unwrap();
        drop(directory);
        let parked_directory = temporary.path().join("parked-ticket-directory");
        fs::rename(temporary.path().join("ticket-directory"), &parked_directory).unwrap();
        let replacement = parent.create_directory(&directory_name).unwrap();
        fs::write(replacement.path().join("replacement"), b"replacement").unwrap();
        let replacement_identity = replacement.identity();
        drop(replacement);
        assert!(parent.open_ticket_directory(&directory_ticket).is_err());
        assert_ne!(replacement_identity, directory_identity);
        assert!(parked_directory.is_dir());
        assert_eq!(
            fs::read(temporary.path().join("ticket-directory/replacement")).unwrap(),
            b"replacement"
        );

        let restricted = parent.create_directory(&child("restricted")).unwrap();
        fs::set_permissions(restricted.path(), fs::Permissions::from_mode(0o500)).unwrap();
        let denied = restricted.open_or_create_file(&child("denied"));
        fs::set_permissions(
            restricted.path(),
            fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE),
        )
        .unwrap();
        assert!(matches!(
            denied,
            Err(SandboxFsError::Io { source, .. })
                if source.kind() == io::ErrorKind::PermissionDenied
        ));

        let socket_path = temporary.path().join("socket");
        let socket = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let socket_ticket = parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &child("socket"))
            .unwrap();
        assert_eq!(socket_ticket.kind(), NodeKind::Other);
        drop(socket);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn original_children_refuse_a_real_missing_seventh_root() {
        let (_temporary, parent) = opened_temp_parent();
        let root = parent.create_directory(&child("original")).unwrap();
        let marker = root.create_file(&child("marker")).unwrap();
        let expected = [
            child("data"),
            child("state"),
            child("config"),
            child("home"),
            child("cwd"),
            child("external"),
            child("system-temp"),
        ];
        let mut children = BTreeMap::new();
        for name in &expected[..6] {
            children.insert(name.clone(), root.create_directory(name).unwrap());
        }

        let error =
            ValidatedOriginalSandbox::new_original(root, marker, expected, children).unwrap_err();

        assert!(matches!(
            error,
            SandboxFsError::Invalid { path, .. }
                if path.ends_with("original/system-temp")
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn publication_and_removal_postconditions_refuse_same_name_rewrites() {
        use std::io::Write as _;

        let (temporary, parent) = opened_temp_parent();
        let marker_name = child("marker");
        let marker = parent.create_file(&marker_name).unwrap();
        let marker_identity = marker.identity();
        let marker_path = marker.path().to_path_buf();
        let publication =
            marker.publish_bytes_with_hook(&parent, &marker_name, b"published bytes", || {
                let mut other = fs::OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&marker_path)
                    .unwrap();
                other.write_all(b"rewritten bytes").unwrap();
                other.sync_all().unwrap();
            });
        assert!(matches!(publication, Err(SandboxFsError::Invalid { .. })));
        assert_eq!(marker.identity(), marker_identity);
        assert_eq!(fs::read(&marker_path).unwrap(), b"rewritten bytes");

        let removed_name = child("removed");
        let removed = parent.create_file(&removed_name).unwrap();
        removed
            .publish_bytes(&parent, &removed_name, b"original bytes")
            .unwrap();
        let original_identity = removed.identity();
        let parked = temporary.path().join("parked-removed");
        fs::hard_link(removed.path(), &parked).unwrap();
        let ticket = parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &removed_name)
            .unwrap();
        drop(removed);
        let replacement_identity = std::cell::Cell::new(None);

        let removal = parent.remove_ticket_with_hook(ticket, |parent, ticket| {
            let replacement = parent.create_file(ticket.name())?;
            replacement.publish_bytes(parent, ticket.name(), b"replacement bytes")?;
            replacement_identity.set(Some(replacement.identity()));
            Ok(())
        });

        assert!(matches!(removal, Err(SandboxFsError::Invalid { .. })));
        let replacement_identity = replacement_identity.get().unwrap();
        assert_ne!(replacement_identity, original_identity);
        assert_eq!(fs::read(&parked).unwrap(), b"original bytes");
        assert_eq!(
            file_identity(&File::open(&parked).unwrap(), &parked).unwrap(),
            original_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("removed")).unwrap(),
            b"replacement bytes"
        );
        assert_eq!(
            parent.open_file(&removed_name, false).unwrap().identity(),
            replacement_identity
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rename_boundaries_refuse_source_and_destination_replacements() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("source-before");
        let destination_name = child("destination-before");
        let source = parent.create_directory(&source_name).unwrap();
        fs::write(source.path().join("original"), b"original before").unwrap();
        let source_identity = source.identity();
        drop(source);
        let parked_source = temporary.path().join("parked-source-before");
        let replacement_identity = std::cell::Cell::new(None);

        let before_result = parent.rename_noreplace_with_hooks(
            RenameTicket::new(
                source_name.clone(),
                destination_name.clone(),
                source_identity,
            ),
            |parent, ticket| {
                fs::rename(
                    parent.path().join(ticket.source.as_os_str()),
                    &parked_source,
                )
                .unwrap();
                let replacement = parent.create_directory(&ticket.source)?;
                fs::write(
                    replacement.path().join("replacement"),
                    b"replacement before",
                )
                .unwrap();
                replacement_identity.set(Some(replacement.identity()));
                Ok(())
            },
            ok_rename_hook,
        );

        assert!(matches!(before_result, Err(SandboxFsError::Invalid { .. })));
        assert_eq!(
            parent
                .open_directory(&child("parked-source-before"))
                .unwrap()
                .identity(),
            source_identity
        );
        assert_eq!(
            fs::read(parked_source.join("original")).unwrap(),
            b"original before"
        );
        assert_eq!(
            fs::read(temporary.path().join("source-before/replacement")).unwrap(),
            b"replacement before"
        );
        assert_ne!(replacement_identity.get().unwrap(), source_identity);
        assert!(!temporary.path().join("destination-before").exists());

        let source_name = child("source-after");
        let destination_name = child("destination-after");
        let source = parent.create_directory(&source_name).unwrap();
        fs::write(source.path().join("original"), b"original after").unwrap();
        let source_identity = source.identity();
        drop(source);
        let parked_destination = temporary.path().join("parked-destination-after");
        let replacement_identity = std::cell::Cell::new(None);

        let after_result = parent.rename_noreplace_with_hooks(
            RenameTicket::new(source_name, destination_name, source_identity),
            ok_rename_hook,
            |parent, ticket| {
                fs::rename(
                    parent.path().join(ticket.destination.as_os_str()),
                    &parked_destination,
                )
                .unwrap();
                let replacement = parent.create_directory(&ticket.destination)?;
                fs::write(replacement.path().join("replacement"), b"replacement after").unwrap();
                replacement_identity.set(Some(replacement.identity()));
                Ok(())
            },
        );

        assert!(matches!(after_result, Err(SandboxFsError::Invalid { .. })));
        assert!(!temporary.path().join("source-after").exists());
        assert_eq!(
            parent
                .open_directory(&child("parked-destination-after"))
                .unwrap()
                .identity(),
            source_identity
        );
        assert_eq!(
            fs::read(parked_destination.join("original")).unwrap(),
            b"original after"
        );
        assert_eq!(
            fs::read(temporary.path().join("destination-after/replacement")).unwrap(),
            b"replacement after"
        );
        assert_ne!(replacement_identity.get().unwrap(), source_identity);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn no_replace_rename_moves_the_exact_opened_directory() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("source");
        let destination_name = child("destination");
        let source = parent.create_directory(&source_name).unwrap();
        fs::write(temporary.path().join("source/owned"), b"source").unwrap();
        let destination = parent.create_directory(&destination_name).unwrap();
        let ticket = RenameTicket::new(
            source_name.clone(),
            destination_name.clone(),
            source.identity(),
        );

        assert!(parent.rename_noreplace(ticket).is_err());
        assert!(temporary.path().join("source/owned").is_file());
        assert_eq!(
            destination.identity(),
            parent.open_directory(&destination_name).unwrap().identity()
        );
        fs::remove_dir(temporary.path().join("destination")).unwrap();

        let moved = parent
            .rename_noreplace(RenameTicket::new(
                source_name,
                destination_name,
                source.identity(),
            ))
            .unwrap();
        assert_eq!(moved.identity(), source.identity());
        assert_eq!(
            fs::read(temporary.path().join("destination/owned")).unwrap(),
            b"source"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn created_source_directory(
        parent: &PinnedDirectory,
        name: &ChildName,
        bytes: &[u8],
    ) -> NodeIdentity {
        let source = parent.create_directory(name).unwrap();
        let identity = source.identity();
        fs::write(source.path().join("owned"), bytes).unwrap();
        identity
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn child_directory_rename_moves_the_exact_directory_to_an_absent_name() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("move-source");
        let destination_name = child("move-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"moved bytes");

        let moved = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap();

        assert_eq!(moved.identity(), source_identity);
        assert_eq!(
            fs::read(moved.path().join("owned")).unwrap(),
            b"moved bytes"
        );
        assert!(parent.open_directory(&source_name).is_err());
        let names: Vec<ChildName> = parent
            .tickets()
            .unwrap()
            .iter()
            .map(|ticket| ticket.name().clone())
            .collect();
        assert_eq!(names, vec![destination_name]);
        assert_eq!(
            fs::read(temporary.path().join("move-destination/owned")).unwrap(),
            b"moved bytes"
        );
        drop(moved);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn child_directory_rename_refuses_an_existing_directory_destination() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("kept-source");
        let destination_name = child("kept-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"source bytes");
        let destination = parent.create_directory(&destination_name).unwrap();
        let destination_identity = destination.identity();
        drop(destination);

        let error = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap_err();

        assert!(
            error.destination_already_exists(),
            "unexpected refusal: {error}"
        );
        let existing = parent.open_directory(&destination_name).unwrap();
        assert_eq!(existing.identity(), destination_identity);
        assert!(existing.tickets().unwrap().is_empty());
        assert_eq!(
            parent
                .open_directory_shared(&destination_name)
                .unwrap()
                .identity(),
            destination_identity
        );
        assert_eq!(
            parent.open_directory(&source_name).unwrap().identity(),
            source_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("kept-source/owned")).unwrap(),
            b"source bytes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_directory_rename_refuses_a_nonempty_directory_destination() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("busy-source");
        let destination_name = child("busy-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"source bytes");
        let destination = parent.create_directory(&destination_name).unwrap();
        let destination_identity = destination.identity();
        fs::write(destination.path().join("installed"), b"installed bytes").unwrap();
        drop(destination);

        let error = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap_err();

        assert!(
            error.destination_already_exists(),
            "unexpected refusal: {error}"
        );
        let existing = parent.open_directory_shared(&destination_name).unwrap();
        assert_eq!(existing.identity(), destination_identity);
        assert_eq!(
            existing
                .tickets()
                .unwrap()
                .iter()
                .map(|ticket| ticket.name().clone())
                .collect::<Vec<_>>(),
            vec![child("installed")]
        );
        assert_eq!(
            fs::read(temporary.path().join("busy-destination/installed")).unwrap(),
            b"installed bytes"
        );
        assert_eq!(
            parent.open_directory(&source_name).unwrap().identity(),
            source_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("busy-source/owned")).unwrap(),
            b"source bytes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_directory_rename_refuses_a_shared_mode_directory_destination() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, parent) = opened_temp_parent();
        let source_name = child("relaxed-source");
        let destination_name = child("relaxed-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"source bytes");
        let destination = parent.create_directory(&destination_name).unwrap();
        let destination_identity = destination.identity();
        fs::write(destination.path().join("installed"), b"installed bytes").unwrap();
        drop(destination);
        fs::set_permissions(
            temporary.path().join("relaxed-destination"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let error = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap_err();

        assert!(
            error.destination_already_exists(),
            "unexpected refusal: {error}"
        );
        assert_eq!(
            parent
                .open_directory_shared(&destination_name)
                .unwrap()
                .identity(),
            destination_identity
        );
        let private = parent.open_directory(&destination_name).unwrap_err();
        assert!(
            matches!(
                &private,
                SandboxFsError::Invalid { reason, .. }
                    if reason == "the mode is 0755, expected 0700"
            ),
            "unexpected private-mode refusal: {private:?}"
        );
        assert_eq!(
            fs::read(temporary.path().join("relaxed-destination/installed")).unwrap(),
            b"installed bytes"
        );
        assert_eq!(
            parent.open_directory(&source_name).unwrap().identity(),
            source_identity
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn child_directory_rename_refuses_a_file_destination() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("file-source");
        let destination_name = child("file-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"source bytes");
        let destination = parent.create_file(&destination_name).unwrap();
        destination
            .publish_bytes(&parent, &destination_name, b"destination bytes")
            .unwrap();
        let destination_identity = destination.identity();
        drop(destination);

        let error = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap_err();

        assert!(
            error.destination_already_exists(),
            "unexpected refusal: {error}"
        );
        let refusal = parent.open_directory_shared(&destination_name).unwrap_err();
        assert!(
            matches!(
                &refusal,
                SandboxFsError::Invalid { reason, .. }
                    if reason == "the opened node is RegularFile, expected Directory"
            ),
            "unexpected destination refusal: {refusal:?}"
        );
        assert_eq!(
            parent
                .open_file(&destination_name, false)
                .unwrap()
                .identity(),
            destination_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("file-destination")).unwrap(),
            b"destination bytes"
        );
        assert_eq!(
            parent.open_directory(&source_name).unwrap().identity(),
            source_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("file-source/owned")).unwrap(),
            b"source bytes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_directory_rename_refuses_a_symlink_destination() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("link-source");
        let destination_name = child("link-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"source bytes");
        std::os::unix::fs::symlink("elsewhere", temporary.path().join("link-destination")).unwrap();

        let error = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap_err();

        assert!(
            error.destination_already_exists(),
            "unexpected refusal: {error}"
        );
        let refusal = parent.open_directory_shared(&destination_name).unwrap_err();
        assert!(
            matches!(
                &refusal,
                SandboxFsError::Io {
                    operation: "open child directory",
                    source,
                    ..
                } if source.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error())
            ),
            "unexpected destination refusal: {refusal:?}"
        );
        assert_eq!(
            fs::read_link(temporary.path().join("link-destination")).unwrap(),
            Path::new("elsewhere")
        );
        assert_eq!(
            parent.open_directory(&source_name).unwrap().identity(),
            source_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("link-source/owned")).unwrap(),
            b"source bytes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_shared_mode_ambient_parent_publishes_a_private_child_directory() {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path().join("review-corpus");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        let parent = PinnedDirectory::open_ambient_parent(&root).unwrap();
        let source_name = child("staged");
        let destination_name = child("corpus-digest");
        let source_identity = created_source_directory(&parent, &source_name, b"staged bytes");

        let moved = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap();

        assert_eq!(moved.identity(), source_identity);
        assert_eq!(
            fs::metadata(root.join("corpus-digest"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            PRIVATE_DIRECTORY_MODE
        );
        assert_eq!(
            fs::metadata(&root).unwrap().permissions().mode() & 0o7777,
            0o755
        );

        let file_path = temporary.path().join("regular-file");
        fs::write(&file_path, b"not a directory").unwrap();
        let file_refusal = PinnedDirectory::open_ambient_parent(&file_path).unwrap_err();
        assert!(
            matches!(
                &file_refusal,
                SandboxFsError::Io {
                    operation: "open namespace parent",
                    source,
                    ..
                } if source.kind() == io::ErrorKind::NotADirectory
            ),
            "unexpected file refusal: {file_refusal:?}"
        );
        let link_path = temporary.path().join("link-to-review-corpus");
        std::os::unix::fs::symlink(&root, &link_path).unwrap();
        let link_refusal = PinnedDirectory::open_ambient_parent(&link_path).unwrap_err();
        assert!(
            matches!(
                &link_refusal,
                SandboxFsError::Io {
                    operation: "open namespace parent",
                    source,
                    ..
                } if source.kind() == io::ErrorKind::NotADirectory
            ),
            "unexpected link refusal: {link_refusal:?}"
        );
        assert_eq!(fs::read_link(&link_path).unwrap(), root);
        drop(moved);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_directory_rename_refuses_a_source_identity_swap() {
        let (temporary, parent) = opened_temp_parent();
        let source_name = child("swap-source");
        let destination_name = child("swap-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"original bytes");
        let parked = temporary.path().join("parked-swap-source");
        let replacement_identity = std::cell::Cell::new(None);

        let error = parent
            .rename_child_directory_noreplace_with_hook(
                &source_name,
                &destination_name,
                |parent, name| {
                    fs::rename(parent.path().join(name.as_os_str()), &parked).unwrap();
                    let replacement = parent.create_directory(name)?;
                    fs::write(replacement.path().join("replacement"), b"replacement bytes")
                        .unwrap();
                    replacement_identity.set(Some(replacement.identity()));
                    Ok(())
                },
            )
            .unwrap_err();

        assert!(
            matches!(
                &error,
                SandboxFsError::Invalid { reason, .. }
                    if reason == "the rename source identity changed"
            ),
            "unexpected refusal: {error:?}"
        );
        assert!(!error.destination_already_exists());
        assert_ne!(replacement_identity.get().unwrap(), source_identity);
        assert_eq!(
            parent
                .open_directory(&child("parked-swap-source"))
                .unwrap()
                .identity(),
            source_identity
        );
        assert_eq!(fs::read(parked.join("owned")).unwrap(), b"original bytes");
        assert_eq!(
            fs::read(temporary.path().join("swap-source/replacement")).unwrap(),
            b"replacement bytes"
        );
        assert!(
            parent
                .tickets()
                .unwrap()
                .iter()
                .all(|ticket| ticket.name() != &destination_name)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_directory_rename_refuses_a_shared_mode_source() {
        use std::os::unix::fs::PermissionsExt as _;

        let (temporary, parent) = opened_temp_parent();
        let source_name = child("shared-source");
        let destination_name = child("shared-destination");
        let source_identity = created_source_directory(&parent, &source_name, b"shared bytes");
        fs::set_permissions(
            temporary.path().join("shared-source"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        let error = parent
            .rename_child_directory_noreplace(&source_name, &destination_name)
            .unwrap_err();

        assert!(
            matches!(
                &error,
                SandboxFsError::Invalid { reason, .. }
                    if reason.contains("the mode is 0755") && reason.contains("expected 0700")
            ),
            "unexpected refusal: {error:?}"
        );
        assert!(!error.destination_already_exists());
        assert!(
            parent
                .tickets()
                .unwrap()
                .iter()
                .all(|ticket| ticket.name() != &destination_name)
        );
        assert_eq!(
            parent
                .open_directory_with_policy(&source_name, false)
                .unwrap()
                .identity(),
            source_identity
        );
        assert_eq!(
            fs::read(temporary.path().join("shared-source/owned")).unwrap(),
            b"shared bytes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn initialization_release_refuses_a_replaced_lock_entry() {
        let temporary = tempfile::TempDir::new().unwrap();
        let lock_name = child("init.lock");
        let guard = InitializationGuard::acquire(temporary.path(), lock_name.clone()).unwrap();
        guard
            .file()
            .publish_bytes(guard.parent(), guard.name(), b"owned lock")
            .unwrap();
        let parked = temporary.path().join("parked.lock");
        fs::rename(temporary.path().join("init.lock"), &parked).unwrap();
        fs::write(temporary.path().join("init.lock"), b"owned lock").unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(
            temporary.path().join("init.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();

        assert!(guard.release(|guard| guard.verify_core()).is_err());
        assert_eq!(fs::read(parked).unwrap(), b"owned lock");
        assert_eq!(
            fs::read(temporary.path().join("init.lock")).unwrap(),
            b"owned lock"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn profile_lease_contention_releases_without_replacing_the_inode() {
        let (_temporary, parent) = opened_temp_parent();
        let leases = parent.create_directory(&child("leases")).unwrap();
        let lease_name = child("review.lock");
        let (mut first, created) = ProfileLease::open(&leases, lease_name.clone()).unwrap();
        assert!(created);
        first.try_lock().unwrap();
        first
            .file()
            .publish_bytes(&leases, &lease_name, b"lease")
            .unwrap();
        let identity = first.file().identity();
        let (mut second, created) = ProfileLease::open(&leases, lease_name.clone()).unwrap();
        assert!(!created);
        assert!(matches!(
            second.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));
        PROFILE_UNLOCK_CALLS.store(0, std::sync::atomic::Ordering::Relaxed);
        second
            .release_with_ops(&leases, counted_denied_profile_unlock)
            .unwrap();
        assert_eq!(
            PROFILE_UNLOCK_CALLS.load(std::sync::atomic::Ordering::Relaxed),
            0
        );

        first.release(&leases).unwrap();
        let (mut acquired, created) = ProfileLease::open(&leases, lease_name.clone()).unwrap();
        assert!(!created);
        acquired.try_lock().unwrap();
        assert_eq!(acquired.file().identity(), identity);
        PROFILE_UNLOCK_CALLS.store(0, std::sync::atomic::Ordering::Relaxed);
        let error = acquired
            .release_with_ops(&leases, counted_denied_profile_unlock)
            .unwrap_err();
        assert_eq!(
            PROFILE_UNLOCK_CALLS.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert!(matches!(
            error,
            SandboxFsError::Io {
                operation: "unlock profile lease",
                ..
            }
        ));

        let (mut recovered, created) = ProfileLease::open(&leases, lease_name).unwrap();
        assert!(!created);
        recovered.try_lock().unwrap();
        assert_eq!(recovered.file().identity(), identity);
        recovered.release(&leases).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recursive_cleanup_unlinks_a_link_and_preserves_its_target() {
        let (temporary, parent) = opened_temp_parent();
        let tree_name = child("tree");
        let tree = parent.create_directory(&tree_name).unwrap();
        let outside = temporary.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("retained"), b"outside").unwrap();
        std::os::unix::fs::symlink(&outside, tree.path().join("link")).unwrap();
        fs::write(tree.path().join("file"), b"inside").unwrap();
        let ticket = parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &tree_name)
            .unwrap();
        assert_eq!(ticket.reparse_tag(), None);

        parent.remove_tree(ticket).unwrap();

        assert!(!temporary.path().join("tree").exists());
        assert_eq!(fs::read(outside.join("retained")).unwrap(), b"outside");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recursive_cleanup_refuses_a_directory_swapped_to_an_external_link() {
        let (temporary, parent) = opened_temp_parent();
        let tree = parent.create_directory(&child("tree")).unwrap();
        let victim = tree.create_directory(&child("victim")).unwrap();
        fs::write(victim.path().join("parked-bytes"), b"parked").unwrap();
        let outside = temporary.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("retained"), b"outside").unwrap();
        let ticket = tree
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &child("victim"))
            .unwrap();
        let parked = tree.path().join("parked-victim");
        let mut swapped = false;

        let result = tree.remove_tree_with_hook(ticket, &mut |_, ticket| {
            if !swapped && ticket.name() == &child("victim") {
                swapped = true;
                fs::rename(tree.path().join("victim"), &parked).unwrap();
                std::os::unix::fs::symlink(&outside, tree.path().join("victim")).unwrap();
            }
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(fs::read(parked.join("parked-bytes")).unwrap(), b"parked");
        assert!(
            fs::symlink_metadata(tree.path().join("victim"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read(outside.join("retained")).unwrap(), b"outside");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn recursive_cleanup_refuses_a_child_added_before_the_empty_recheck() {
        let (temporary, parent) = opened_temp_parent();
        let tree_name = child("late-tree");
        let tree = parent.create_directory(&tree_name).unwrap();
        let tree_identity = tree.identity();
        let nested = tree.create_directory(&child("nested")).unwrap();
        fs::write(nested.path().join("initial"), b"initial").unwrap();
        drop(nested);
        let ticket = parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == &tree_name)
            .unwrap();
        let mut added = false;

        let result =
            parent.remove_tree_with_post_children_hook(ticket, &mut |directory, ticket| {
                if ticket.name() == &tree_name {
                    let name = child("late");
                    let late = directory.create_file(&name)?;
                    late.publish_bytes(directory, &name, b"late bytes")?;
                    added = true;
                }
                Ok(())
            });

        assert!(matches!(result, Err(SandboxFsError::Invalid { .. })));
        assert!(added);
        assert_eq!(tree.identity(), tree_identity);
        assert_eq!(
            fs::read(temporary.path().join("late-tree/late")).unwrap(),
            b"late bytes"
        );
    }

    #[cfg(windows)]
    fn windows_ticket(parent: &PinnedDirectory, name: &ChildName) -> EntryTicket {
        parent
            .tickets()
            .unwrap()
            .into_iter()
            .find(|ticket| ticket.name() == name)
            .unwrap()
    }

    #[cfg(windows)]
    fn create_windows_junction(root: &Path) {
        let status = std::process::Command::new("cmd.exe")
            .args([
                "/D",
                "/V:OFF",
                "/S",
                "/C",
                "mklink /J owned\\junction outside",
            ])
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed with {status}");
    }

    #[cfg(windows)]
    fn windows_file_node_identity(path: &Path) -> NodeIdentity {
        let file = File::open(path).unwrap();
        file_identity(&file, path).unwrap()
    }

    #[cfg(windows)]
    fn open_windows_delete_sharing_parent(path: &Path) -> PinnedDirectory {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_ADD_FILE, FILE_ADD_SUBDIRECTORY, FILE_DELETE_CHILD, FILE_FLAG_BACKUP_SEMANTICS,
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES,
            FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TRAVERSE,
            FILE_WRITE_ATTRIBUTES,
        };

        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .access_mode(
                FILE_LIST_DIRECTORY
                    | FILE_ADD_FILE
                    | FILE_ADD_SUBDIRECTORY
                    | FILE_DELETE_CHILD
                    | FILE_TRAVERSE
                    | FILE_READ_ATTRIBUTES
                    | FILE_WRITE_ATTRIBUTES,
            )
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .unwrap();
        PinnedDirectory::from_file(file, path.to_path_buf(), false).unwrap()
    }

    #[cfg(windows)]
    fn assert_already_exists(error: SandboxFsError) {
        assert!(
            matches!(
                &error,
                SandboxFsError::Io { source, .. }
                    if source.kind() == io::ErrorKind::AlreadyExists
            ),
            "unexpected collision error: {error}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_relative_open_stays_with_a_parked_parent_across_a_junction_replacement() {
        let temporary = tempfile::TempDir::new().unwrap();
        let owned = temporary.path().join("owned");
        let parked = temporary.path().join("parked");
        let outside = temporary.path().join("outside");
        fs::create_dir_all(owned.join("junction")).unwrap();
        fs::write(owned.join("junction/original.bin"), b"original bytes").unwrap();
        fs::create_dir(&outside).unwrap();
        let outside_canary = outside.join("retained.bin");
        fs::write(&outside_canary, b"outside bytes").unwrap();
        let outside_canary_identity = windows_file_node_identity(&outside_canary);
        let parent = open_windows_delete_sharing_parent(&owned);
        let original = parent.open_directory(&child("junction")).unwrap();
        let original_identity = original.identity();
        drop(original);

        fs::rename(&owned, &parked).unwrap();
        fs::create_dir(&owned).unwrap();
        create_windows_junction(temporary.path());
        let replacement = PinnedDirectory::open_ambient_parent(&owned).unwrap();
        let replacement_identity = replacement.identity();
        let replacement_junction = windows_ticket(&replacement, &child("junction"));
        assert_eq!(replacement_junction.kind(), NodeKind::LinkOrReparse);
        assert_eq!(
            replacement_junction.reparse_tag(),
            Some(windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT)
        );

        let reopened = parent.open_directory(&child("junction")).unwrap();
        assert_eq!(reopened.identity(), original_identity);
        let created_name = child("created.bin");
        let created = reopened.create_file(&created_name).unwrap();
        created
            .publish_bytes(&reopened, &created_name, b"parked bytes")
            .unwrap();

        assert_eq!(
            fs::read(parked.join("junction/original.bin")).unwrap(),
            b"original bytes"
        );
        assert_eq!(
            fs::read(parked.join("junction/created.bin")).unwrap(),
            b"parked bytes"
        );
        assert!(!outside.join("created.bin").exists());
        assert_eq!(fs::read(&outside_canary).unwrap(), b"outside bytes");
        assert_eq!(
            windows_file_node_identity(&outside_canary),
            outside_canary_identity
        );
        parent.verify_handle().unwrap();
        replacement.verify_handle().unwrap();
        assert_eq!(replacement.identity(), replacement_identity);

        drop(created);
        drop(reopened);
        replacement.remove_ticket(replacement_junction).unwrap();
        assert_eq!(fs::read(&outside_canary).unwrap(), b"outside bytes");
        assert_eq!(
            windows_file_node_identity(&outside_canary),
            outside_canary_identity
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_created_directory_supports_metadata_and_child_io() {
        let (_temporary, parent) = opened_temp_parent();
        let name = child("metadata-directory");
        let directory = parent.create_directory(&name).unwrap();
        assert!(directory.file().metadata().unwrap().is_dir());
        assert_eq!(
            parent.open_directory(&name).unwrap().identity(),
            directory.identity()
        );
        let marker = child("marker");
        let file = directory.create_file(&marker).unwrap();
        file.publish_bytes(&directory, &marker, b"created directory")
            .unwrap();
        assert_eq!(
            file.read_all(&directory, &marker).unwrap(),
            b"created directory"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_primary_pins_refuse_replacement_until_the_pin_drops() {
        let container = tempfile::TempDir::new().unwrap();
        let owned = container.path().join("owned-parent");
        let parked_parent = container.path().join("parked-parent");
        fs::create_dir(&owned).unwrap();
        let parent_pin = PinnedDirectory::open_ambient_parent(&owned).unwrap();

        assert!(fs::rename(&owned, &parked_parent).is_err());
        parent_pin.verify_handle().unwrap();
        drop(parent_pin);
        fs::rename(&owned, &parked_parent).unwrap();

        let parent = PinnedDirectory::open_ambient_parent(&parked_parent).unwrap();
        let file_name = child("pinned-file");
        let file = parent.create_file(&file_name).unwrap();
        file.publish_bytes(&parent, &file_name, b"pinned file")
            .unwrap();
        let parked_file = parked_parent.join("parked-file");
        assert!(fs::rename(file.path(), &parked_file).is_err());
        file.verify_at(&parent, &file_name).unwrap();
        drop(file);
        fs::rename(parked_parent.join("pinned-file"), &parked_file).unwrap();
        assert_eq!(fs::read(&parked_file).unwrap(), b"pinned file");

        let directory_name = child("pinned-directory");
        let directory = parent.create_directory(&directory_name).unwrap();
        fs::write(directory.path().join("retained"), b"pinned directory").unwrap();
        let parked_directory = parked_parent.join("parked-directory");
        assert!(fs::rename(directory.path(), &parked_directory).is_err());
        directory.verify_at(&parent, &directory_name).unwrap();
        drop(directory);
        fs::rename(parked_parent.join("pinned-directory"), &parked_directory).unwrap();
        assert_eq!(
            fs::read(parked_directory.join("retained")).unwrap(),
            b"pinned directory"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_no_replace_preserves_every_destination_kind_and_moves_exact_identity() {
        let temporary = tempfile::TempDir::new().unwrap();
        let owned = temporary.path().join("owned");
        let outside = temporary.path().join("outside");
        fs::create_dir(&owned).unwrap();
        fs::create_dir(&outside).unwrap();
        let outside_canary = outside.join("retained.bin");
        fs::write(&outside_canary, b"outside bytes").unwrap();
        let outside_canary_identity = windows_file_node_identity(&outside_canary);
        let outside_pin = PinnedDirectory::open_ambient_parent(&outside).unwrap();
        let parent = PinnedDirectory::open_ambient_parent(&owned).unwrap();

        let empty_name = child("empty-destination");
        let empty = parent.create_directory(&empty_name).unwrap();
        let empty_identity = empty.identity();
        drop(empty);
        let source_empty_name = child("source-empty");
        let source_empty = parent.create_directory(&source_empty_name).unwrap();
        let source_empty_identity = source_empty.identity();
        fs::write(source_empty.path().join("owned"), b"source empty").unwrap();
        drop(source_empty);
        assert_already_exists(
            parent
                .rename_noreplace(RenameTicket::new(
                    source_empty_name.clone(),
                    empty_name.clone(),
                    source_empty_identity,
                ))
                .unwrap_err(),
        );
        assert_eq!(
            parent.open_directory(&empty_name).unwrap().identity(),
            empty_identity
        );
        assert_eq!(
            fs::read(owned.join("source-empty/owned")).unwrap(),
            b"source empty"
        );

        let nonempty_name = child("nonempty-destination");
        let nonempty = parent.create_directory(&nonempty_name).unwrap();
        fs::write(nonempty.path().join("retained"), b"destination directory").unwrap();
        let nonempty_identity = nonempty.identity();
        drop(nonempty);
        let source_nonempty_name = child("source-nonempty");
        let source_nonempty = parent.create_directory(&source_nonempty_name).unwrap();
        let source_nonempty_identity = source_nonempty.identity();
        drop(source_nonempty);
        assert_already_exists(
            parent
                .rename_noreplace(RenameTicket::new(
                    source_nonempty_name.clone(),
                    nonempty_name.clone(),
                    source_nonempty_identity,
                ))
                .unwrap_err(),
        );
        assert_eq!(
            parent.open_directory(&nonempty_name).unwrap().identity(),
            nonempty_identity
        );
        assert_eq!(
            fs::read(owned.join("nonempty-destination/retained")).unwrap(),
            b"destination directory"
        );

        let file_name = child("file-destination");
        let file = parent.create_file(&file_name).unwrap();
        file.publish_bytes(&parent, &file_name, b"destination file")
            .unwrap();
        let file_identity = file.identity();
        drop(file);
        let source_file_name = child("source-file");
        let source_file = parent.create_directory(&source_file_name).unwrap();
        let source_file_identity = source_file.identity();
        drop(source_file);
        assert_already_exists(
            parent
                .rename_noreplace(RenameTicket::new(
                    source_file_name.clone(),
                    file_name.clone(),
                    source_file_identity,
                ))
                .unwrap_err(),
        );
        assert_eq!(
            parent.open_file(&file_name, false).unwrap().identity(),
            file_identity
        );
        assert_eq!(
            fs::read(owned.join("file-destination")).unwrap(),
            b"destination file"
        );

        create_windows_junction(temporary.path());
        let junction_name = child("junction");
        let junction_before = windows_ticket(&parent, &junction_name);
        assert_eq!(junction_before.kind(), NodeKind::LinkOrReparse);
        assert_eq!(
            junction_before.reparse_tag(),
            Some(windows_sys::Win32::System::SystemServices::IO_REPARSE_TAG_MOUNT_POINT)
        );
        let source_junction_name = child("source-junction");
        let source_junction = parent.create_directory(&source_junction_name).unwrap();
        let source_junction_identity = source_junction.identity();
        drop(source_junction);
        assert_already_exists(
            parent
                .rename_noreplace(RenameTicket::new(
                    source_junction_name.clone(),
                    junction_name.clone(),
                    source_junction_identity,
                ))
                .unwrap_err(),
        );
        let junction_after = windows_ticket(&parent, &junction_name);
        assert_eq!(junction_after.identity(), junction_before.identity());
        assert_eq!(junction_after.kind(), NodeKind::LinkOrReparse);
        assert_eq!(junction_after.reparse_tag(), junction_before.reparse_tag());
        assert_eq!(fs::read(&outside_canary).unwrap(), b"outside bytes");
        assert_eq!(
            windows_file_node_identity(&outside_canary),
            outside_canary_identity
        );
        outside_pin.verify_handle().unwrap();

        let moved_source_name = child("successful-source");
        let moved_destination_name = child("successful-destination");
        let moved_source = parent.create_directory(&moved_source_name).unwrap();
        fs::write(moved_source.path().join("owned"), b"moved bytes").unwrap();
        let moved_identity = moved_source.identity();
        drop(moved_source);
        let moved = parent
            .rename_noreplace(RenameTicket::new(
                moved_source_name,
                moved_destination_name,
                moved_identity,
            ))
            .unwrap();
        assert_eq!(moved.identity(), moved_identity);
        assert_eq!(
            fs::read(moved.path().join("owned")).unwrap(),
            b"moved bytes"
        );
        drop(moved);

        parent.remove_ticket(junction_after).unwrap();
        assert_eq!(fs::read(&outside_canary).unwrap(), b"outside bytes");
        assert_eq!(
            windows_file_node_identity(&outside_canary),
            outside_canary_identity
        );
        outside_pin.verify_handle().unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_child_directory_rename_refuses_a_junction_destination() {
        let temporary = tempfile::TempDir::new().unwrap();
        let owned = temporary.path().join("owned");
        let outside = temporary.path().join("outside");
        fs::create_dir(&owned).unwrap();
        fs::create_dir(&outside).unwrap();
        let outside_canary = outside.join("retained.bin");
        fs::write(&outside_canary, b"outside bytes").unwrap();
        let outside_canary_identity = windows_file_node_identity(&outside_canary);
        let parent = PinnedDirectory::open_ambient_parent(&owned).unwrap();
        let source_name = child("junction-source");
        let source_identity = created_source_directory(&parent, &source_name, b"source bytes");
        create_windows_junction(temporary.path());
        let junction_name = child("junction");
        let junction_before = windows_ticket(&parent, &junction_name);
        assert_eq!(junction_before.kind(), NodeKind::LinkOrReparse);

        let error = parent
            .rename_child_directory_noreplace(&source_name, &junction_name)
            .unwrap_err();

        assert!(
            error.destination_already_exists(),
            "unexpected refusal: {error}"
        );
        assert!(parent.open_directory_shared(&junction_name).is_err());
        let junction_after = windows_ticket(&parent, &junction_name);
        assert_eq!(junction_after.identity(), junction_before.identity());
        assert_eq!(junction_after.kind(), NodeKind::LinkOrReparse);
        assert_eq!(junction_after.reparse_tag(), junction_before.reparse_tag());
        assert_eq!(
            parent.open_directory(&source_name).unwrap().identity(),
            source_identity
        );
        assert_eq!(
            fs::read(owned.join("junction-source/owned")).unwrap(),
            b"source bytes"
        );
        assert_eq!(fs::read(&outside_canary).unwrap(), b"outside bytes");
        assert_eq!(
            windows_file_node_identity(&outside_canary),
            outside_canary_identity
        );
        parent.remove_ticket(junction_after).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_exact_delete_handles_cover_file_empty_directory_and_nonempty_refusal() {
        let (temporary, parent) = opened_temp_parent();

        let file_name = child("delete-file");
        let file = parent.create_file(&file_name).unwrap();
        file.publish_bytes(&parent, &file_name, b"delete me")
            .unwrap();
        drop(file);
        parent
            .remove_ticket(windows_ticket(&parent, &file_name))
            .unwrap();
        assert!(!temporary.path().join("delete-file").exists());

        let empty_name = child("delete-empty");
        drop(parent.create_directory(&empty_name).unwrap());
        parent
            .remove_ticket(windows_ticket(&parent, &empty_name))
            .unwrap();
        assert!(!temporary.path().join("delete-empty").exists());

        let nonempty_name = child("delete-nonempty");
        let nonempty = parent.create_directory(&nonempty_name).unwrap();
        fs::write(nonempty.path().join("retained"), b"retained").unwrap();
        drop(nonempty);
        let ticket = windows_ticket(&parent, &nonempty_name);
        assert!(parent.remove_ticket(ticket.clone()).is_err());
        assert_eq!(
            fs::read(temporary.path().join("delete-nonempty/retained")).unwrap(),
            b"retained"
        );
        parent.remove_tree(ticket).unwrap();
        assert!(!temporary.path().join("delete-nonempty").exists());
    }
}
