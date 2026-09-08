mod agent_skill;
mod atomic;
mod hash;
pub(crate) mod registry;
mod runner_management;

use std::{
    collections::BTreeMap,
    fmt, fs,
    fs::{File, OpenOptions},
    io::{self, Seek as _, Write as _},
    path::{Path, PathBuf},
};

use crate::fs_ops::sync_directory;
pub use agent_skill::{AgentSkillInstallPoint, FileAgentSkillStore};
use atomic::{
    FileLock, StagedDirectory, acquire_lock, acquire_shared_lock, atomic_write_bytes,
    create_dir_all, invalid, io_error, write_new_file, write_new_metadata,
};
pub use hash::content_hash;
use registry::Registry;
pub use runner_management::{
    FileRunnerManagementStore, RunnerManagementStoreError, RunnerRemovalCas,
};
use skit_application::{
    CreateEntry, EntryMutationRepository, EntryPayload, ExternalCopyEdit,
    FinalizeExternalCopyEditError, FinalizedExternalCopyEdit, RepositoryError, UpdateEntry,
};
use skit_domain::{Entry, EntryId, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_i18n::{Localize as _, Message};

use super::{
    FileStore,
    paths::{LAUNCH_SNAPSHOT_PREFIX, is_support_file, stored_filenames},
};

/// One directory-specific problem found during an explicit registry rebuild.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryRebuildProblem {
    /// The entry directory has no metadata file.
    MissingMetadata {
        /// Directory name.
        slug: String,
    },
    /// The metadata file cannot produce a valid entry.
    CorruptMetadata {
        /// Directory name.
        slug: String,
        /// Parser, validation, or filesystem detail.
        reason: String,
    },
    /// A reference entry was indexed, but its source path is gone.
    MissingReferenceSource {
        /// Entry slug.
        slug: String,
        /// Missing owned source path.
        path: String,
    },
}

impl RegistryRebuildProblem {
    /// Convert the typed problem into localizable presentation text.
    #[must_use]
    pub fn message(&self) -> Message {
        match self {
            Self::MissingMetadata { slug } => {
                Message::new("{}: meta.toml is missing; skipped").with(slug)
            }
            Self::CorruptMetadata { slug, reason } => {
                Message::new("{}: meta.toml is corrupt ({}); skipped")
                    .with(slug)
                    .with(reason)
            }
            Self::MissingReferenceSource { slug, path } => {
                Message::new("{}: the referenced source file is gone: {}")
                    .with(slug)
                    .with(path)
            }
        }
    }
}

/// Complete result of an explicit registry rebuild.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RegistryRebuildReport {
    /// Valid entries written to the new registry.
    pub entry_count: usize,
    /// Deterministic per-directory problems isolated during the scan.
    pub problems: Vec<RegistryRebuildProblem>,
}

/// One identity-checked launch whose copy-mode payload cannot change underneath the child.
///
/// The launch lease remains held until this value is dropped. Source edits may continue while a
/// child runs, but removing and reusing the entry directory waits until the child and its
/// post-run state transaction are complete.
#[derive(Debug)]
pub struct PreparedLaunch {
    entry: Entry,
    payload: Option<PathBuf>,
    temporary_payload: Option<TemporaryLaunchPayload>,
    _lease: FileLock,
}

#[derive(Debug)]
struct TemporaryLaunchPayload {
    path: PathBuf,
    identity: Option<LaunchSnapshotIdentity>,
    file: Option<File>,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LaunchSnapshotIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug)]
struct LaunchSnapshotIdentity(same_file::Handle);

struct LaunchSnapshotWrite<'a> {
    path: PathBuf,
    bytes: &'a [u8],
    permissions: fs::Permissions,
    attempt: u64,
    stem: LaunchSnapshotStem,
}

/// One opaque 128-bit token for a launch snapshot name.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LaunchSnapshotStem(u128);

impl LaunchSnapshotStem {
    /// Construct a stem from one allocator-owned value.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// Return the allocator-owned value.
    #[must_use]
    pub const fn value(self) -> u128 {
        self.0
    }
}

impl fmt::Display for LaunchSnapshotStem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

/// The validated result of one store-owned launch snapshot allocation attempt.
#[derive(Debug)]
pub enum LaunchSnapshotAttemptOutcome<'a> {
    /// The store created and validated the candidate.
    Accepted,
    /// The store could not create or validate the candidate.
    Rejected(&'a io::Error),
}

/// Evidence from one store-owned launch snapshot allocation attempt.
#[derive(Debug)]
pub struct LaunchSnapshotAttempt<'a> {
    /// Zero-based attempt index for this snapshot preparation.
    pub attempt: u64,
    /// Opaque stem supplied for this attempt.
    pub stem: LaunchSnapshotStem,
    /// Exact direct-child path constructed by the store.
    pub path: &'a Path,
    /// Validated allocation result.
    pub outcome: LaunchSnapshotAttemptOutcome<'a>,
}

/// Supply opaque launch snapshot stems and observe store-owned allocation results.
#[derive(Clone, Copy, Debug)]
pub struct LaunchSnapshotRequest<'a> {
    /// Zero-based attempt index for this snapshot preparation.
    pub attempt: u64,
    /// Entry directory that will own the store-built direct child.
    pub directory: &'a Path,
    /// Store-derived source suffix, including its leading dot when present.
    pub suffix: &'a str,
}

/// Supply opaque launch snapshot stems and observe store-owned allocation results.
pub trait LaunchSnapshotAllocator: fmt::Debug {
    /// Supply the next opaque stem.
    fn next_stem(&self, request: LaunchSnapshotRequest<'_>) -> io::Result<LaunchSnapshotStem>;

    /// Record one validated store-owned allocation result.
    fn record_attempt(&self, _evidence: LaunchSnapshotAttempt<'_>) {}

    /// Report whether an `AlreadyExists` collision can consume another stem.
    fn retry_collisions(&self) -> bool {
        false
    }
}

/// Production launch snapshot stem allocation.
#[derive(Clone, Copy, Debug)]
pub struct SystemLaunchSnapshotAllocator;

impl LaunchSnapshotAllocator for SystemLaunchSnapshotAllocator {
    fn next_stem(&self, _request: LaunchSnapshotRequest<'_>) -> io::Result<LaunchSnapshotStem> {
        let id = EntryId::generate();
        let value = u128::from_str_radix(id.as_str(), 16)
            .expect("an EntryId always contains 32 hexadecimal digits");
        Ok(LaunchSnapshotStem::new(value))
    }
}

impl FileStore {
    /// Remove one stale direct-child file from this entry's launch snapshot namespace.
    pub fn remove_stale_launch_snapshot(&self, entry: &Entry, path: &Path) -> io::Result<()> {
        let entry_dir = self.entry_dir(&entry.slug);
        let reserved = path.parent() == Some(entry_dir.as_path())
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(super::paths::is_launch_snapshot_name);
        if !reserved {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the path is outside this entry's launch snapshot namespace",
            ));
        }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the stale launch snapshot is not a regular file",
            ));
        }
        let (identity, file) = open_launch_snapshot_cleanup_handle(path)?;
        #[cfg(test)]
        run_stale_before_remove_hook(path);
        remove_launch_snapshot_if_same_file(path, identity, file)
    }
}

#[cfg(unix)]
fn open_launch_snapshot_cleanup_handle(path: &Path) -> io::Result<(LaunchSnapshotIdentity, File)> {
    let file = File::open(path)?;
    let identity = launch_snapshot_identity(&file)?;
    Ok((identity, file))
}

#[cfg(windows)]
fn open_launch_snapshot_cleanup_handle(path: &Path) -> io::Result<(LaunchSnapshotIdentity, File)> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_WRITE_ATTRIBUTES,
    };

    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    let identity = launch_snapshot_identity(&file)?;
    Ok((identity, file))
}

#[cfg(not(any(unix, windows)))]
fn open_launch_snapshot_cleanup_handle(path: &Path) -> io::Result<(LaunchSnapshotIdentity, File)> {
    let file = File::open(path)?;
    let identity = launch_snapshot_identity(&file)?;
    Ok((identity, file))
}

/// One FileStore-owned claim for an in-place external copy edit.
#[derive(Clone, Debug)]
pub struct PreparedExternalCopyEdit {
    entry: Entry,
    path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RemoveLockPoint {
    Dependency,
    Entry,
    Namespace,
}

impl ExternalCopyEdit for PreparedExternalCopyEdit {
    fn entry(&self) -> &Entry {
        &self.entry
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl PreparedLaunch {
    /// Return the freshly claimed entry incarnation.
    #[must_use]
    pub const fn entry(&self) -> &Entry {
        &self.entry
    }

    /// Return the path whose bytes were checked while the entry lock was held.
    #[must_use]
    pub fn payload_path(&self) -> Option<&Path> {
        self.payload.as_deref()
    }
}

impl Drop for TemporaryLaunchPayload {
    fn drop(&mut self) {
        if let (Some(identity), Some(file)) = (self.identity.take(), self.file.take()) {
            let _ = remove_launch_snapshot_if_same_file(&self.path, identity, file);
        }
    }
}

impl Drop for PreparedLaunch {
    fn drop(&mut self) {
        drop(self.temporary_payload.take());
    }
}

impl EntryMutationRepository for FileStore {
    type ExternalEdit = PreparedExternalCopyEdit;

    fn create(&self, request: CreateEntry) -> Result<Entry, RepositoryError> {
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        self.create_locked(request, &mut registry)
    }

    fn claim_identity(&self, entry: &Entry) -> Result<Entry, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let fresh = self.verify_claim_locked(entry)?;
        if fresh.meta.id.is_some() {
            return Ok(fresh);
        }
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        self.stamp_identity_locked(fresh, &mut registry)
    }

    fn preflight_update_entry(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        let name = validated_name(name)?;
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        self.ensure_name_available(&name, Some(&fresh.slug), &registry)?;
        Ok(fresh)
    }

    fn describe(&self, entry: &Entry, description: &str) -> Result<Entry, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        let before = fresh.clone();
        let mut after = fresh;
        after.meta.description = description.to_owned();
        self.commit_meta_projection(&before, &after, &mut registry)?;
        Ok(after)
    }

    fn update_settings(
        &self,
        entry: &Entry,
        settings: &EntrySettings,
        workdir: &str,
    ) -> Result<Entry, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        let before = fresh.clone();
        let mut after = fresh;
        after.meta.workdir = workdir.to_owned();
        settings.write_to_meta(&mut after.meta);
        self.commit_meta_projection(&before, &after, &mut registry)?;
        Ok(after)
    }

    fn update_entry(&self, entry: &Entry, update: UpdateEntry) -> Result<Entry, RepositoryError> {
        let name = validated_name(&update.name)?;
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        self.ensure_name_available(&name, Some(&fresh.slug), &registry)?;
        let before = fresh.clone();
        let mut after = fresh;
        after.meta.name = name;
        after.meta.description = update.description;
        after.meta.workdir = update.workdir;
        update.settings.write_to_meta(&mut after.meta);

        let Some(bytes) = update.source else {
            self.commit_meta_projection(&before, &after, &mut registry)?;
            return Ok(after);
        };
        if after.meta.mode != StorageMode::Copy {
            return Err(invalid(Message::new(
                "reference entries are edited at their original path",
            )));
        }
        let target = self.stored_path(&after)?;
        let original = fs::read(&target).map_err(|error| io_error("read", &target, error))?;
        let actual = content_hash(&original);
        if actual != update.expected_source_hash {
            return Err(RepositoryError::SourceChanged {
                slug: after.slug.as_str().to_owned(),
                expected: update.expected_source_hash,
                actual,
            });
        }
        after.meta.source_hash = content_hash(&bytes);
        replace_source(&target, &bytes, &original, || self.write_meta(&after))?;
        let projection = self.refresh_existing_projection(&mut registry, &after);
        if let Err(error) = projection {
            return Err(rollback_source_projection(
                error,
                &target,
                &original,
                || self.write_meta(&before),
            ));
        }
        Ok(after)
    }

    fn rename(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        let name = validated_name(name)?;
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        self.ensure_name_available(&name, Some(&fresh.slug), &registry)?;
        let before = fresh.clone();
        let mut after = fresh;
        after.meta.name = name;
        self.commit_meta_projection(&before, &after, &mut registry)?;
        Ok(after)
    }

    fn remove(&self, entry: &Entry) -> Result<String, RepositoryError> {
        self.remove_with_lock_hook(entry, |_| {})
    }

    fn commit_copy_edit(
        &self,
        entry: &Entry,
        bytes: &[u8],
        expected_source_hash: &str,
    ) -> Result<Entry, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        if fresh.meta.mode != StorageMode::Copy {
            return Err(invalid(Message::new(
                "reference entries are edited at their original path",
            )));
        }

        let target = self.stored_path(&fresh)?;
        let original = fs::read(&target).map_err(|error| io_error("read", &target, error))?;
        let actual = content_hash(&original);
        if actual != expected_source_hash {
            return Err(RepositoryError::SourceChanged {
                slug: fresh.slug.as_str().to_owned(),
                expected: expected_source_hash.to_owned(),
                actual,
            });
        }

        let before = fresh.clone();
        let mut after = fresh;
        after.meta.source_hash = content_hash(bytes);
        replace_source(&target, bytes, &original, || self.write_meta(&after))?;
        let projection = self.refresh_existing_projection(&mut registry, &after);
        if let Err(error) = projection {
            return Err(rollback_source_projection(
                error,
                &target,
                &original,
                || self.write_meta(&before),
            ));
        }
        Ok(after)
    }

    fn prepare_external_copy_edit(
        &self,
        entry: &Entry,
    ) -> Result<Self::ExternalEdit, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let fresh = self.verify_claim_locked(entry)?;
        if fresh.meta.mode != StorageMode::Copy {
            return Err(invalid(Message::new(
                "reference entries are edited at their original path",
            )));
        }
        let path = self.stored_path(&fresh)?;
        let fresh = if fresh.meta.id.is_some() {
            fresh
        } else {
            let _namespace = self.namespace_lock()?;
            let mut registry = Registry::load(self.data_dir())?;
            self.stamp_identity_locked(fresh, &mut registry)?
        };
        Ok(PreparedExternalCopyEdit { entry: fresh, path })
    }

    fn finalize_external_copy_edit(
        &self,
        edit: &Self::ExternalEdit,
    ) -> Result<FinalizedExternalCopyEdit, FinalizeExternalCopyEditError> {
        let entry = edit.entry();
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        if fresh.meta != entry.meta {
            return Err(stale(&entry.slug).into());
        }
        if fresh.meta.mode != StorageMode::Copy {
            return Err(invalid(Message::new(
                "reference entries are edited at their original path",
            ))
            .into());
        }

        let target = edit.path();
        if target.parent() != Some(self.entry_dir(&fresh.slug).as_path())
            || target.file_name().is_none()
        {
            return Err(invalid(Message::new(
                "external edit source is outside its entry directory",
            ))
            .into());
        }
        let authoritative = match self.stored_path(&fresh) {
            Ok(path) => path,
            Err(RepositoryError::InvalidMutation { reason })
                if reason.template() == "copy entry has no stored payload"
                    && stored_filenames(fresh.meta.kind.as_str()).len() == 1 =>
            {
                self.entry_dir(&fresh.slug)
                    .join(stored_filenames(fresh.meta.kind.as_str())[0])
            }
            Err(error) => return Err(error.into()),
        };
        if target != authoritative {
            return Err(invalid(Message::new(
                "external edit source is not the entry's stored payload",
            ))
            .into());
        }
        let bytes = fs::read(target).map_err(|source| FinalizeExternalCopyEditError::Read {
            path: target.to_owned(),
            source,
        })?;
        let next_hash = content_hash(&bytes);
        if next_hash == fresh.meta.source_hash {
            return Ok(FinalizedExternalCopyEdit::new(fresh, bytes));
        }

        let before = fresh.clone();
        let mut after = fresh;
        after.meta.source_hash = next_hash;
        self.commit_meta_projection(&before, &after, &mut registry)?;
        Ok(FinalizedExternalCopyEdit::new(after, bytes))
    }
}

impl FileStore {
    fn remove_with_lock_hook(
        &self,
        entry: &Entry,
        mut before_lock: impl FnMut(RemoveLockPoint),
    ) -> Result<String, RepositoryError> {
        let _launch = self.launch_lock(&entry.slug)?;
        before_lock(RemoveLockPoint::Dependency);
        let _dependencies = self.dependency_lock(&entry.slug)?;
        before_lock(RemoveLockPoint::Entry);
        let _entry = self.entry_lock(&entry.slug)?;
        before_lock(RemoveLockPoint::Namespace);
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        let name = fresh.meta.name.clone();
        let source = self.entry_dir(&fresh.slug);
        let trash_root = self.data_dir().join(".trash");
        create_dir_all(&trash_root, "create")?;
        let trash = trash_root.join(format!("{}-{}", fresh.slug, EntryId::generate().as_str()));
        fs::rename(&source, &trash).map_err(|error| io_error("remove", &source, error))?;
        let _ = sync_directory(&self.scripts_dir());
        let _ = sync_directory(&trash_root);

        registry.remove(&fresh.slug);
        if let Err(error) = registry.save() {
            let rollback = fs::rename(&trash, &source)
                .map_err(|rollback| io_error("rollback remove", &trash, rollback));
            let _ = sync_directory(&self.scripts_dir());
            let _ = sync_directory(&trash_root);
            return Err(rollback_error(error, rollback, &source));
        }
        match fs::remove_dir_all(&trash) {
            Ok(()) => {
                let _ = sync_directory(&trash_root);
                Ok(name)
            }
            Err(_) => {
                let incomplete = RepositoryError::RemovalIncomplete {
                    name,
                    path: source.display().to_string(),
                };
                let restored = fs::rename(&trash, &source)
                    .map_err(|error| io_error("restore incomplete removal", &trash, error));
                let _ = sync_directory(&self.scripts_dir());
                let _ = sync_directory(&trash_root);
                Err(rollback_error(incomplete, restored, &source))
            }
        }
    }

    /// Recheck identity and source bytes, then pin a copy-mode payload for one launch.
    ///
    /// `expected_source_hash` is the hash observed while the launch form was assembled. Passing it
    /// closes the legacy-entry gap where a remove/re-add can preserve idless metadata while
    /// replacing the payload. Reference entries are rechecked but continue to launch their owned
    /// source path; copy entries launch a private byte-for-byte snapshot.
    pub fn prepare_launch(
        &self,
        held: &Entry,
        expected_source_hash: Option<&str>,
    ) -> Result<PreparedLaunch, RepositoryError> {
        self.prepare_launch_with_snapshot_allocator(
            held,
            expected_source_hash,
            &SystemLaunchSnapshotAllocator,
        )
    }

    /// Prepare one launch and use a raw allocator for a copy-mode snapshot stem.
    ///
    /// The store owns the path, file, source claim, bytes, mode, sync, rollback, lease, and cleanup.
    pub fn prepare_launch_with_snapshot_allocator(
        &self,
        held: &Entry,
        expected_source_hash: Option<&str>,
        allocator: &(impl LaunchSnapshotAllocator + ?Sized),
    ) -> Result<PreparedLaunch, RepositoryError> {
        let lease = self.launch_lease(&held.slug)?;
        let _entry = self.entry_lock(&held.slug)?;
        let fresh = self.verify_claim_locked(held)?;
        if held.meta.id.is_some() && fresh.meta != held.meta {
            return Err(stale(&held.slug));
        }
        let fresh = if fresh.meta.id.is_some() {
            fresh
        } else {
            let _namespace = self.namespace_lock()?;
            let mut registry = Registry::load(self.data_dir())?;
            self.stamp_identity_locked(fresh, &mut registry)?
        };

        if fresh.meta.kind.as_str() == "command" {
            return Ok(PreparedLaunch {
                entry: fresh,
                payload: None,
                temporary_payload: None,
                _lease: lease,
            });
        }

        let source = self.payload_path(&fresh)?;
        if fresh.meta.kind.as_str() == "exe" {
            return Ok(PreparedLaunch {
                entry: fresh,
                payload: Some(source),
                temporary_payload: None,
                _lease: lease,
            });
        }
        let bytes = read_launch_source_bytes(&source, &fresh.slug, expected_source_hash)?;
        if fresh.meta.mode == StorageMode::Reference {
            return Ok(PreparedLaunch {
                entry: fresh,
                payload: Some(source),
                temporary_payload: None,
                _lease: lease,
            });
        }

        #[cfg(test)]
        run_launch_before_permissions_hook(&source);
        let permissions = fs::metadata(&source)
            .map_err(|error| io_error("inspect", &source, error))?
            .permissions();
        let entry_dir = self.entry_dir(&fresh.slug);
        let suffix = launch_snapshot_suffix(&source);
        let temporary = write_launch_snapshot(&entry_dir, &suffix, &bytes, permissions, allocator)?;
        let snapshot = temporary.path.clone();
        Ok(PreparedLaunch {
            entry: fresh,
            payload: Some(snapshot.clone()),
            temporary_payload: Some(temporary),
            _lease: lease,
        })
    }

    /// Rebuild `registry.toml` from every valid authoritative metadata file.
    pub fn rebuild_registry(&self) -> Result<usize, RepositoryError> {
        self.rebuild_registry_report()
            .map(|report| report.entry_count)
    }

    /// Rebuild the registry and retain every isolated directory problem.
    pub fn rebuild_registry_report(&self) -> Result<RegistryRebuildReport, RepositoryError> {
        let _namespace = self.namespace_lock()?;
        let scripts = self.scripts_dir();
        let mut registry = Registry::fresh(self.data_dir());
        let mut report = RegistryRebuildReport::default();
        let reader = match fs::read_dir(&scripts) {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                registry.save()?;
                return Ok(report);
            }
            Err(error) => return Err(io_error("scan", &scripts, error)),
        };
        let mut items = reader
            .map(|item| item.map_err(|error| io_error("scan", &scripts, error)))
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_by_key(std::fs::DirEntry::file_name);
        for item in items {
            if !item
                .file_type()
                .map_err(|error| io_error("inspect", &item.path(), error))?
                .is_dir()
            {
                continue;
            }
            let directory_name = item.file_name().to_string_lossy().into_owned();
            let Ok(slug) = Slug::parse(directory_name.clone()) else {
                report
                    .problems
                    .push(RegistryRebuildProblem::CorruptMetadata {
                        slug: directory_name,
                        reason: "the entry directory name is not a valid slug".to_owned(),
                    });
                continue;
            };
            let metadata = item.path().join("meta.toml");
            let entry = match self.read_entry(slug.clone()) {
                Ok(entry) => entry,
                Err(error) => {
                    let problem = if !metadata.exists() {
                        RegistryRebuildProblem::MissingMetadata {
                            slug: slug.as_str().to_owned(),
                        }
                    } else {
                        RegistryRebuildProblem::CorruptMetadata {
                            slug: slug.as_str().to_owned(),
                            reason: rebuild_error_reason(error),
                        }
                    };
                    report.problems.push(problem);
                    continue;
                }
            };
            if entry.meta.mode == StorageMode::Reference
                && !entry.meta.source.is_empty()
                && !Path::new(&entry.meta.source).exists()
            {
                report
                    .problems
                    .push(RegistryRebuildProblem::MissingReferenceSource {
                        slug: entry.slug.as_str().to_owned(),
                        path: entry.meta.source.clone(),
                    });
            }
            // One entry skit cannot stamp must not cost the whole projection.
            #[cfg(test)]
            run_rebuild_before_project_hook(&metadata);
            if let Err(error) = registry.project(&entry, &item.path()) {
                report
                    .problems
                    .push(RegistryRebuildProblem::CorruptMetadata {
                        slug: entry.slug.as_str().to_owned(),
                        reason: rebuild_error_reason(error),
                    });
                continue;
            }
            report.entry_count += 1;
        }
        registry.save()?;
        Ok(report)
    }

    fn create_locked(
        &self,
        mut request: CreateEntry,
        registry: &mut Registry,
    ) -> Result<Entry, RepositoryError> {
        request.name = validated_name(&request.name)?;
        self.ensure_name_available(&request.name, None, registry)?;
        validate_payload(request.mode, request.payload.as_ref())?;
        let slug = self.allocate_slug(Slug::from_display_name(&request.name), registry)?;
        let id = EntryId::generate();
        let source_hash = request
            .payload
            .as_ref()
            .map_or_else(String::new, |payload| content_hash(&payload.bytes));
        let mut meta = EntryMeta {
            schema: 1,
            name: request.name,
            kind: request.kind,
            mode: request.mode,
            source: request.source,
            source_hash,
            added_at: self.create_stamp(),
            id: Some(id.clone()),
            workdir: request.workdir,
            description: request.description,
            extra: BTreeMap::new(),
        };
        request.settings.write_to_meta(&mut meta);

        let staging_root = self.data_dir().join(".staging");
        create_dir_all(&staging_root, "create")?;
        sweep_staging(&staging_root)?;
        let stage_path = staging_root.join(format!("{}-{}", slug, id.as_str()));
        fs::create_dir(&stage_path).map_err(|error| io_error("create", &stage_path, error))?;
        let mut stage = StagedDirectory::new(stage_path);

        if let (StorageMode::Copy, Some(payload)) = (request.mode, request.payload.as_ref()) {
            let stored_name = payload
                .stored_name
                .as_deref()
                .expect("validate_payload requires a stored filename for copy mode");
            write_new_file(&stage.path().join(stored_name), payload)?;
        }
        write_new_metadata(&stage.path().join("meta.toml"), &meta)?;

        let scripts = self.scripts_dir();
        create_dir_all(&scripts, "create")?;
        let destination = scripts.join(slug.as_str());
        self.remove_empty_destination(&destination)?;
        let _ = sync_directory(stage.path());
        fs::rename(stage.path(), &destination)
            .map_err(|error| io_error("commit", &destination, error))?;
        let _ = sync_directory(&scripts);
        let _ = sync_directory(&staging_root);
        let entry = Entry { slug, meta };
        let projection = registry
            .project(&entry, &destination)
            .and_then(|()| registry.save());
        if let Err(error) = projection {
            let rollback = fs::remove_dir_all(&destination)
                .map_err(|rollback| io_error("rollback create", &destination, rollback));
            let _ = sync_directory(&scripts);
            return Err(rollback_error(error, rollback, &destination));
        }
        stage.commit();
        Ok(entry)
    }

    fn verify_claim_locked(&self, held: &Entry) -> Result<Entry, RepositoryError> {
        let directory = self.entry_dir(&held.slug);
        if !directory.is_dir() {
            return Err(stale(&held.slug));
        }
        let fresh = self.read_entry(held.slug.clone())?;
        match held.meta.id.as_ref() {
            Some(expected) if fresh.meta.id.as_ref() == Some(expected) => Ok(fresh),
            Some(_) => Err(stale(&held.slug)),
            None if fresh.meta.id.is_none() && fresh.meta == held.meta => Ok(fresh),
            None => Err(stale(&held.slug)),
        }
    }

    fn claim_for_mutation(
        &self,
        held: &Entry,
        registry: &mut Registry,
    ) -> Result<Entry, RepositoryError> {
        let fresh = self.verify_claim_locked(held)?;
        if fresh.meta.id.is_some() {
            Ok(fresh)
        } else {
            self.stamp_identity_locked(fresh, registry)
        }
    }

    fn stamp_identity_locked(
        &self,
        before: Entry,
        registry: &mut Registry,
    ) -> Result<Entry, RepositoryError> {
        let mut after = before.clone();
        after.meta.id = Some(EntryId::generate());
        self.commit_meta_projection(&before, &after, registry)?;
        Ok(after)
    }

    fn commit_meta_projection(
        &self,
        before: &Entry,
        after: &Entry,
        registry: &mut Registry,
    ) -> Result<(), RepositoryError> {
        self.write_meta(after)?;
        let projection = self.refresh_existing_projection(registry, after);
        if let Err(error) = projection {
            return Err(rollback_error(
                error,
                self.write_meta(before),
                &self.entry_dir(&before.slug),
            ));
        }
        Ok(())
    }

    fn refresh_existing_projection(
        &self,
        registry: &mut Registry,
        entry: &Entry,
    ) -> Result<(), RepositoryError> {
        // Reload after the metadata commit. A person or an older skit can remove an index row
        // without taking this process's lock. That removal defines membership and must win.
        *registry = Registry::load(self.data_dir())?;
        if registry.project_existing(entry, &self.entry_dir(&entry.slug))? {
            registry.save()?;
        }
        Ok(())
    }

    fn ensure_name_available(
        &self,
        name: &str,
        excluded: Option<&Slug>,
        registry: &Registry,
    ) -> Result<(), RepositoryError> {
        if registry.name_owner(name, excluded).is_some() {
            return Err(name_conflict(name, excluded.is_some()));
        }
        for existing in self.scan_entries()? {
            if existing.meta.name == name && excluded != Some(&existing.slug) {
                return Err(name_conflict(name, excluded.is_some()));
            }
        }
        Ok(())
    }

    fn allocate_slug(&self, base: Slug, registry: &Registry) -> Result<Slug, RepositoryError> {
        if !registry.slug_is_taken(&base) && !self.slug_path_is_taken(&base)? {
            return Ok(base);
        }

        let mut suffix = 2_u64;
        loop {
            let candidate = Slug::parse(format!("{}-{suffix}", base.as_str()))
                .map_err(|error| invalid(error.message()))?;
            if !registry.slug_is_taken(&candidate) && !self.slug_path_is_taken(&candidate)? {
                return Ok(candidate);
            }
            suffix = suffix
                .checked_add(1)
                .ok_or_else(|| invalid(Message::new("entry slug suffix space is exhausted")))?;
        }
    }

    fn slug_path_is_taken(&self, slug: &Slug) -> Result<bool, RepositoryError> {
        let path = self.entry_dir(slug);
        if !path.exists() {
            return Ok(false);
        }
        if !path.is_dir() {
            return Ok(true);
        }
        let mut items = fs::read_dir(&path).map_err(|error| io_error("scan", &path, error))?;
        Ok(items
            .next()
            .transpose()
            .map_err(|error| io_error("scan", &path, error))?
            .is_some())
    }

    fn remove_empty_destination(&self, path: &Path) -> Result<(), RepositoryError> {
        if !path.is_dir() {
            return Ok(());
        }
        let mut items = fs::read_dir(path).map_err(|error| io_error("scan", path, error))?;
        if items
            .next()
            .transpose()
            .map_err(|error| io_error("scan", path, error))?
            .is_none()
        {
            fs::remove_dir(path).map_err(|error| io_error("reuse", path, error))?;
        }
        Ok(())
    }

    fn stored_path(&self, entry: &Entry) -> Result<PathBuf, RepositoryError> {
        let directory = self.entry_dir(&entry.slug);
        for candidate in stored_filenames(entry.meta.kind.as_str()) {
            let path = directory.join(candidate);
            if path.is_file() {
                return Ok(path);
            }
        }

        let mut files = Vec::new();
        let reader =
            fs::read_dir(&directory).map_err(|error| io_error("scan", &directory, error))?;
        for item in reader {
            let item = item.map_err(|error| io_error("scan", &directory, error))?;
            if is_support_file(&item.file_name().to_string_lossy()) {
                continue;
            }
            let file_type = item
                .file_type()
                .map_err(|error| io_error("inspect", &item.path(), error))?;
            if file_type.is_file() {
                files.push(item.path());
            }
        }
        match files.as_slice() {
            [path] => Ok(path.clone()),
            [] => Err(invalid(Message::new("copy entry has no stored payload"))),
            _ => Err(invalid(Message::new(
                "copy entry has more than one possible stored payload",
            ))),
        }
    }

    fn write_meta(&self, entry: &Entry) -> Result<(), RepositoryError> {
        let path = self.entry_dir(&entry.slug).join("meta.toml");
        let text = encode_metadata(&path, &entry.meta)?;
        atomic_write_bytes(&path, text.as_bytes())
    }

    fn entry_dir(&self, slug: &Slug) -> PathBuf {
        self.scripts_dir().join(slug.as_str())
    }

    fn namespace_lock(&self) -> Result<FileLock, RepositoryError> {
        acquire_lock(&self.data_dir().join("registry.native.lock"))
    }

    fn entry_lock(&self, slug: &Slug) -> Result<FileLock, RepositoryError> {
        acquire_lock(
            &self
                .data_dir()
                .join(".locks")
                .join(format!("{}.meta.lock", slug.as_str())),
        )
    }

    fn launch_lock(&self, slug: &Slug) -> Result<FileLock, RepositoryError> {
        acquire_lock(
            &self
                .data_dir()
                .join(".locks")
                .join(format!("{}.launch.lock", slug.as_str())),
        )
    }

    fn launch_lease(&self, slug: &Slug) -> Result<FileLock, RepositoryError> {
        acquire_shared_lock(
            &self
                .data_dir()
                .join(".locks")
                .join(format!("{}.launch.lock", slug.as_str())),
        )
    }

    fn dependency_lock(&self, slug: &Slug) -> Result<FileLock, RepositoryError> {
        acquire_lock(
            &self
                .data_dir()
                .join(".locks")
                .join(format!("{}.skit-deps.lock", slug.as_str())),
        )
    }
}

fn launch_snapshot_suffix(source: &Path) -> String {
    source
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(String::new, |extension| format!(".{extension}"))
}

fn read_launch_source_bytes(
    source: &Path,
    slug: &Slug,
    expected_source_hash: Option<&str>,
) -> Result<Vec<u8>, RepositoryError> {
    let bytes = fs::read(source).map_err(|error| io_error("read", source, error))?;
    if let Some(expected) = expected_source_hash {
        let actual = content_hash(&bytes);
        if actual != expected {
            return Err(RepositoryError::SourceChanged {
                slug: slug.as_str().to_owned(),
                expected: expected.to_owned(),
                actual,
            });
        }
    }
    Ok(bytes)
}

fn rebuild_error_reason(error: RepositoryError) -> String {
    match error {
        RepositoryError::Corrupt { reason, .. } | RepositoryError::Io { reason, .. } => reason,
        other => other.to_string(),
    }
}

#[cfg(test)]
type RebuildBeforeProjectHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
thread_local! {
    static REBUILD_BEFORE_PROJECT_HOOK: std::cell::RefCell<Option<RebuildBeforeProjectHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
fn run_rebuild_before_project_hook(path: &Path) {
    REBUILD_BEFORE_PROJECT_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(path);
        }
    });
}

#[cfg(test)]
type LaunchBeforePermissionsHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
type LaunchAfterOpenHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
type LaunchChmodHook = Box<dyn FnOnce(&File, &Path, &fs::Permissions) -> io::Result<()>>;
#[cfg(test)]
type LaunchInitialModeHook = Box<dyn FnOnce(&File, &Path) -> io::Result<()>>;
#[cfg(test)]
type StaleBeforeRemoveHook = Box<dyn FnOnce(&Path)>;
#[cfg(test)]
type LaunchAfterClearReadonlyHook = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchIdentityFault {
    Clone,
    Handle,
}

#[cfg(test)]
thread_local! {
    static LAUNCH_BEFORE_PERMISSIONS_HOOK: std::cell::RefCell<Option<LaunchBeforePermissionsHook>> =
        std::cell::RefCell::new(None);
    static LAUNCH_AFTER_OPEN_HOOK: std::cell::RefCell<Option<LaunchAfterOpenHook>> =
        std::cell::RefCell::new(None);
    static LAUNCH_CHMOD_HOOK: std::cell::RefCell<Option<LaunchChmodHook>> =
        std::cell::RefCell::new(None);
    static LAUNCH_INITIAL_MODE_HOOK: std::cell::RefCell<Option<LaunchInitialModeHook>> =
        std::cell::RefCell::new(None);
    static STALE_BEFORE_REMOVE_HOOK: std::cell::RefCell<Option<StaleBeforeRemoveHook>> =
        std::cell::RefCell::new(None);
    static LAUNCH_AFTER_CLEAR_READONLY_HOOK: std::cell::RefCell<Option<LaunchAfterClearReadonlyHook>> =
        std::cell::RefCell::new(None);
    static LAUNCH_IDENTITY_FAULT: std::cell::Cell<Option<LaunchIdentityFault>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn run_launch_before_permissions_hook(path: &Path) {
    LAUNCH_BEFORE_PERMISSIONS_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(path);
        }
    });
}

#[cfg(test)]
fn run_launch_after_open_hook(path: &Path) {
    LAUNCH_AFTER_OPEN_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(path);
        }
    });
}

#[cfg(test)]
fn run_stale_before_remove_hook(path: &Path) {
    STALE_BEFORE_REMOVE_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(path);
        }
    });
}

#[cfg(test)]
fn run_launch_after_clear_readonly_hook(path: &Path) {
    LAUNCH_AFTER_CLEAR_READONLY_HOOK.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook(path);
        }
    });
}

fn set_launch_snapshot_permissions(
    file: &File,
    _path: &Path,
    permissions: fs::Permissions,
) -> io::Result<()> {
    #[cfg(test)]
    if let Some(hook) = LAUNCH_CHMOD_HOOK.with(|hook| hook.borrow_mut().take()) {
        return hook(file, _path, &permissions);
    }
    file.set_permissions(permissions)
}

fn set_initial_launch_snapshot_mode(file: &File, _path: &Path) -> io::Result<()> {
    #[cfg(test)]
    if let Some(hook) = LAUNCH_INITIAL_MODE_HOOK.with(|hook| hook.borrow_mut().take()) {
        return hook(file, _path);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(0o600))
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Ok(())
    }
}

#[cfg(test)]
fn take_launch_identity_fault(point: LaunchIdentityFault) -> bool {
    LAUNCH_IDENTITY_FAULT.with(|fault| {
        if fault.get() == Some(point) {
            fault.set(None);
            true
        } else {
            false
        }
    })
}

fn write_launch_snapshot(
    entry_dir: &Path,
    suffix: &str,
    bytes: &[u8],
    permissions: fs::Permissions,
    allocator: &(impl LaunchSnapshotAllocator + ?Sized),
) -> Result<TemporaryLaunchPayload, RepositoryError> {
    write_launch_snapshot_with_finalize(
        entry_dir,
        suffix,
        bytes,
        permissions,
        allocator,
        File::sync_all,
    )
}

fn write_launch_snapshot_with_finalize(
    entry_dir: &Path,
    suffix: &str,
    bytes: &[u8],
    permissions: fs::Permissions,
    allocator: &(impl LaunchSnapshotAllocator + ?Sized),
    finalize: impl FnOnce(&File) -> io::Result<()>,
) -> Result<TemporaryLaunchPayload, RepositoryError> {
    let mut attempt = 0_u64;
    loop {
        let request = LaunchSnapshotRequest {
            attempt,
            directory: entry_dir,
            suffix,
        };
        let stem = allocator
            .next_stem(request)
            .map_err(|error| io_error("allocate launch snapshot", entry_dir, error))?;
        let snapshot = entry_dir.join(format!("{LAUNCH_SNAPSHOT_PREFIX}{stem}{suffix}"));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = match options.open(&snapshot) {
            Ok(file) => file,
            Err(error) => {
                allocator.record_attempt(LaunchSnapshotAttempt {
                    attempt,
                    stem,
                    path: &snapshot,
                    outcome: LaunchSnapshotAttemptOutcome::Rejected(&error),
                });
                if error.kind() == io::ErrorKind::AlreadyExists && allocator.retry_collisions() {
                    attempt = next_launch_snapshot_attempt(attempt)
                        .map_err(|error| io_error("allocate launch snapshot", &snapshot, error))?;
                    continue;
                }
                return Err(io_error("create", &snapshot, error));
            }
        };
        if let Err(error) = set_initial_launch_snapshot_mode(&file, &snapshot) {
            allocator.record_attempt(LaunchSnapshotAttempt {
                attempt,
                stem,
                path: &snapshot,
                outcome: LaunchSnapshotAttemptOutcome::Rejected(&error),
            });
            let repository_error = io_error("chmod", &snapshot, error);
            match launch_snapshot_identity(&file) {
                Ok(identity) => {
                    let _ = remove_launch_snapshot_if_same_file(&snapshot, identity, file);
                }
                Err(_) => drop(file),
            }
            return Err(repository_error);
        }
        #[cfg(test)]
        run_launch_after_open_hook(&snapshot);
        return finish_launch_snapshot(
            LaunchSnapshotWrite {
                path: snapshot,
                bytes,
                permissions,
                attempt,
                stem,
            },
            file,
            allocator,
            finalize,
        );
    }
}

fn next_launch_snapshot_attempt(attempt: u64) -> io::Result<u64> {
    attempt
        .checked_add(1)
        .ok_or_else(|| io::Error::other("launch snapshot attempt counter overflow"))
}

fn finish_launch_snapshot(
    request: LaunchSnapshotWrite<'_>,
    mut file: File,
    allocator: &(impl LaunchSnapshotAllocator + ?Sized),
    finalize: impl FnOnce(&File) -> io::Result<()>,
) -> Result<TemporaryLaunchPayload, RepositoryError> {
    let LaunchSnapshotWrite {
        path: snapshot,
        bytes,
        permissions,
        attempt,
        stem,
    } = request;
    let identity = match launch_snapshot_identity(&file) {
        Ok(identity) => identity,
        Err(error) => {
            allocator.record_attempt(LaunchSnapshotAttempt {
                attempt,
                stem,
                path: &snapshot,
                outcome: LaunchSnapshotAttemptOutcome::Rejected(&error),
            });
            let repository_error = io_error("inspect", &snapshot, error);
            drop(file);
            return Err(repository_error);
        }
    };
    if let Err(error) = validate_new_launch_snapshot(&mut file, &snapshot, &identity) {
        allocator.record_attempt(LaunchSnapshotAttempt {
            attempt,
            stem,
            path: &snapshot,
            outcome: LaunchSnapshotAttemptOutcome::Rejected(&error),
        });
        let repository_error = io_error("inspect", &snapshot, error);
        let _ = remove_launch_snapshot_if_same_file(&snapshot, identity, file);
        return Err(repository_error);
    }
    allocator.record_attempt(LaunchSnapshotAttempt {
        attempt,
        stem,
        path: &snapshot,
        outcome: LaunchSnapshotAttemptOutcome::Accepted,
    });
    let result = file
        .write_all(bytes)
        .map_err(|error| io_error("write", &snapshot, error))
        .and_then(|()| {
            set_launch_snapshot_permissions(&file, &snapshot, permissions)
                .map_err(|error| io_error("chmod", &snapshot, error))
        })
        .and_then(|()| finalize(&file).map_err(|error| io_error("sync", &snapshot, error)))
        .and_then(|()| {
            validate_launch_snapshot_path(&snapshot, &identity)
                .map_err(|error| io_error("inspect", &snapshot, error))
        });
    if let Err(error) = result {
        let _ = remove_launch_snapshot_if_same_file(&snapshot, identity, file);
        return Err(error);
    }
    // Retain attribute access for cleanup. A child can deny shared write-data access.
    #[cfg(windows)]
    let (identity, file) = {
        let retained = open_launch_snapshot_cleanup_handle(&snapshot).and_then(|retained| {
            if retained.0.0 != identity.0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the launch snapshot path does not match its open handle",
                ));
            }
            Ok(retained)
        });
        match retained {
            Ok(retained) => {
                drop(identity);
                drop(file);
                retained
            }
            Err(error) => {
                let repository_error = io_error("inspect", &snapshot, error);
                let _ = remove_launch_snapshot_if_same_file(&snapshot, identity, file);
                return Err(repository_error);
            }
        }
    };
    Ok(TemporaryLaunchPayload {
        path: snapshot,
        identity: Some(identity),
        file: Some(file),
    })
}

fn launch_snapshot_identity(file: &File) -> io::Result<LaunchSnapshotIdentity> {
    LaunchSnapshotIdentity::from_file(file)
}

#[cfg(unix)]
impl LaunchSnapshotIdentity {
    fn from_file(file: &File) -> io::Result<Self> {
        #[cfg(test)]
        if take_launch_identity_fault(LaunchIdentityFault::Clone) {
            return Err(io::Error::other("injected launch snapshot clone failure"));
        }
        let metadata = file.metadata()?;
        #[cfg(test)]
        if take_launch_identity_fault(LaunchIdentityFault::Handle) {
            return Err(io::Error::other("injected launch snapshot handle failure"));
        }
        Ok(Self::from_metadata(&metadata))
    }

    fn from_metadata(metadata: &fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;

        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[cfg(not(unix))]
impl LaunchSnapshotIdentity {
    fn from_file(file: &File) -> io::Result<Self> {
        #[cfg(test)]
        if take_launch_identity_fault(LaunchIdentityFault::Clone) {
            return Err(io::Error::other("injected launch snapshot clone failure"));
        }
        let cloned = file.try_clone()?;
        #[cfg(test)]
        if take_launch_identity_fault(LaunchIdentityFault::Handle) {
            drop(cloned);
            return Err(io::Error::other("injected launch snapshot handle failure"));
        }
        same_file::Handle::from_file(cloned).map(Self)
    }
}

impl LaunchSnapshotIdentity {
    fn matches_path(&self, path: &Path) -> io::Result<bool> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Ok(false);
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;

            Ok(metadata.dev() == self.device && metadata.ino() == self.inode)
        }
        #[cfg(not(unix))]
        {
            launch_snapshot_identity_matches_regular_path_non_unix(self, path)
        }
    }
}

#[cfg(not(unix))]
fn launch_snapshot_identity_matches_regular_path_non_unix(
    identity: &LaunchSnapshotIdentity,
    path: &Path,
) -> io::Result<bool> {
    same_file::Handle::from_path(path).map(|current| current.eq(&identity.0))
}

fn validate_new_launch_snapshot(
    file: &mut File,
    snapshot: &Path,
    identity: &LaunchSnapshotIdentity,
) -> io::Result<()> {
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot handle is not a regular file",
        ));
    }
    if metadata.len() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot handle is not empty",
        ));
    }
    if file.stream_position()? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot handle is not at position zero",
        ));
    }
    validate_launch_snapshot_path(snapshot, identity)
}

fn validate_launch_snapshot_path(
    snapshot: &Path,
    identity: &LaunchSnapshotIdentity,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(snapshot)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot path is not a regular file",
        ));
    }
    if !identity.matches_path(snapshot)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot path does not match its open handle",
        ));
    }
    Ok(())
}

fn remove_launch_snapshot_if_same_file(
    snapshot: &Path,
    identity: LaunchSnapshotIdentity,
    file: File,
) -> io::Result<()> {
    let result = validate_launch_snapshot_removal(snapshot, &identity, &file);
    release_launch_snapshot_identity(identity);
    drop(file);
    result?;
    fs::remove_file(snapshot)
}

fn validate_launch_snapshot_removal(
    snapshot: &Path,
    identity: &LaunchSnapshotIdentity,
    file: &File,
) -> io::Result<()> {
    if !identity.matches_path(snapshot)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot path no longer matches its owned file",
        ));
    }
    #[cfg(windows)]
    if launch_snapshot_has_multiple_links(file)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot has more than one hard link",
        ));
    }
    clear_launch_snapshot_readonly(file)?;
    #[cfg(test)]
    run_launch_after_clear_readonly_hook(snapshot);
    if !identity.matches_path(snapshot)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the launch snapshot path changed while readonly was cleared",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn launch_snapshot_has_multiple_links(file: &File) -> io::Result<bool> {
    Ok(winapi_util::file::information(file)?.number_of_links() > 1)
}

#[cfg(windows)]
fn clear_launch_snapshot_readonly(file: &File) -> io::Result<()> {
    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(false);
    file.set_permissions(permissions)
}

#[cfg(not(windows))]
fn clear_launch_snapshot_readonly(_file: &File) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn release_launch_snapshot_identity(_identity: LaunchSnapshotIdentity) {}

#[cfg(not(unix))]
fn release_launch_snapshot_identity(identity: LaunchSnapshotIdentity) {
    drop(identity);
}

#[cfg(test)]
fn write_launch_snapshot_with(
    source: &Path,
    snapshot: &Path,
    bytes: &[u8],
    finalize: impl FnOnce(&File) -> io::Result<()>,
) -> Result<(), RepositoryError> {
    let permissions = fs::metadata(source)
        .map_err(|error| io_error("inspect", source, error))?
        .permissions();
    let entry_dir = snapshot
        .parent()
        .expect("a launch snapshot test path has a parent");
    let suffix = snapshot
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(String::new, |extension| format!(".{extension}"));
    let stem_text = snapshot
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_prefix(".run-"))
        .expect("a launch snapshot test path has a .run stem");
    let stem = LaunchSnapshotStem::new(
        u128::from_str_radix(stem_text, 16).expect("a launch snapshot test stem is hexadecimal"),
    );
    #[derive(Debug)]
    struct FixedAllocator(LaunchSnapshotStem);
    impl LaunchSnapshotAllocator for FixedAllocator {
        fn next_stem(&self, _request: LaunchSnapshotRequest<'_>) -> io::Result<LaunchSnapshotStem> {
            Ok(self.0)
        }
    }
    write_launch_snapshot_with_finalize(
        entry_dir,
        &suffix,
        bytes,
        permissions,
        &FixedAllocator(stem),
        finalize,
    )
    .map(drop)
}

const CORE_METADATA_KEYS: &[&str] = &[
    "schema",
    "name",
    "kind",
    "mode",
    "source",
    "source_hash",
    "added_at",
    "id",
    "workdir",
    "description",
];

const MANAGED_METADATA_KEYS: &[&str] = &[
    "template",
    "dependencies",
    "requires_python",
    "params",
    "interpreter",
    "runner",
    "interpolate",
    "needs",
    "parameters",
];

fn encode_metadata(path: &Path, meta: &EntryMeta) -> Result<String, RepositoryError> {
    let mut managed = meta.clone();
    managed
        .extra
        .retain(|key, _| MANAGED_METADATA_KEYS.contains(&key.as_str()));
    let encoded = toml::to_string_pretty(&managed)
        .map_err(|error| invalid(Message::new("could not encode metadata: {}").with(error)))?;
    let updates = encoded
        .parse::<toml::Table>()
        .map_err(|error| invalid(Message::new("could not encode metadata: {}").with(error)))?;
    let original = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error("read", path, error)),
    };
    let mut document = original
        .parse::<toml::Table>()
        .map_err(|error| invalid(Message::new("could not encode metadata: {}").with(error)))?;
    let before = document.clone();

    for key in CORE_METADATA_KEYS {
        document.remove(*key);
        if let Some(value) = updates.get(*key) {
            document.insert((*key).to_owned(), value.clone());
        }
    }
    for key in MANAGED_METADATA_KEYS {
        document.remove(*key);
        if let Some(value) = updates.get(*key) {
            document.insert((*key).to_owned(), value.clone());
        }
    }
    let desired = toml::to_string_pretty(&document)
        .map_err(|error| invalid(Message::new("could not encode metadata: {}").with(error)))?;
    crate::toml_document::merge_update(&original, &desired, &before, &document)
        .map_err(|error| invalid(Message::new("could not encode metadata: {}").with(error)))
}

fn validated_name(name: &str) -> Result<String, RepositoryError> {
    let name = name.trim();
    if name.is_empty() {
        Err(invalid(Message::new("entry name cannot be blank")))
    } else {
        Ok(name.to_owned())
    }
}

fn validate_payload(
    mode: StorageMode,
    payload: Option<&EntryPayload>,
) -> Result<(), RepositoryError> {
    let Some(payload) = payload else {
        return Ok(());
    };
    if mode == StorageMode::Copy && payload.stored_name.is_none() {
        return Err(invalid(Message::new(
            "copy-mode payloads require a stored filename",
        )));
    }
    if let Some(name) = payload.stored_name.as_deref() {
        validate_stored_name(name)?;
    }
    Ok(())
}

fn validate_stored_name(name: &str) -> Result<(), RepositoryError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
        || Path::new(name).is_absolute()
    {
        return Err(invalid(Message::new(
            "stored filename must be one safe path component",
        )));
    }
    Ok(())
}

fn sweep_staging(staging_root: &Path) -> Result<(), RepositoryError> {
    let items =
        fs::read_dir(staging_root).map_err(|error| io_error("scan", staging_root, error))?;
    for item in items {
        let item = item.map_err(|error| io_error("scan", staging_root, error))?;
        let path = item.path();
        let file_type = fs::symlink_metadata(&path)
            .map_err(|error| io_error("inspect", &path, error))?
            .file_type();
        if file_type.is_dir() {
            fs::remove_dir_all(&path).map_err(|error| io_error("remove", &path, error))?;
        } else {
            fs::remove_file(&path).map_err(|error| io_error("remove", &path, error))?;
        }
    }
    let _ = sync_directory(staging_root);
    Ok(())
}

/// The add voice advises picking another name; the rename voice states the fact.
/// Version 0.4 raises NameConflictError (store.py:812-816) for a create and
/// StoreError (store.py:1449-1452) for a rename, with these exact texts.
fn name_conflict(name: &str, renaming: bool) -> RepositoryError {
    if renaming {
        RepositoryError::RenameConflict {
            name: name.to_owned(),
        }
    } else {
        RepositoryError::Conflict {
            name: name.to_owned(),
        }
    }
}

fn stale(slug: &Slug) -> RepositoryError {
    RepositoryError::StaleEntry {
        slug: slug.as_str().to_owned(),
    }
}

fn replace_source(
    target: &Path,
    bytes: &[u8],
    original: &[u8],
    write_meta: impl FnOnce() -> Result<(), RepositoryError>,
) -> Result<(), RepositoryError> {
    atomic_write_bytes(target, bytes)?;
    write_meta()
        .map_err(|error| rollback_error(error, atomic_write_bytes(target, original), target))
}

fn rollback_source_projection(
    primary: RepositoryError,
    target: &Path,
    original: &[u8],
    restore_meta: impl FnOnce() -> Result<(), RepositoryError>,
) -> RepositoryError {
    match atomic_write_bytes(target, original) {
        Ok(()) => rollback_error(primary, restore_meta(), target),
        Err(error) => rollback_error(primary, Err(error), target),
    }
}

fn rollback_error(
    primary: RepositoryError,
    rollback: Result<(), RepositoryError>,
    path: &Path,
) -> RepositoryError {
    match rollback {
        Ok(()) => primary,
        Err(rollback) => RepositoryError::Rollback {
            path: path.display().to_string(),
            primary: Box::new(primary),
            rollback: Box::new(rollback),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        rc::Rc,
        sync::{Arc, Barrier, mpsc},
        thread,
    };

    use skit_domain::EntryKind;
    use tempfile::TempDir;

    use super::*;

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
            "{}umask-store-",
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

    fn replace_launch_snapshot_after_readonly_clear(path: &Path) {
        fs::rename(path, path.with_extension("parked")).unwrap();
        fs::write(path, b"replacement").unwrap();
    }

    fn fail_launch_snapshot_sync(_file: &File) -> io::Result<()> {
        Err(io::Error::other("injected sync failure"))
    }

    fn copy_request(name: &str, kind: &str, stored_name: &str) -> CreateEntry {
        CreateEntry {
            name: name.to_owned(),
            kind: EntryKind::parse(kind).unwrap(),
            mode: StorageMode::Copy,
            source: format!("/original/{stored_name}"),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: b"stored\n".to_vec(),
                stored_name: Some(stored_name.to_owned()),
                permissions: skit_application::SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum RecordedSnapshotAttemptOutcome {
        Accepted,
        Rejected(io::ErrorKind, String),
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct RecordedSnapshotAttempt {
        attempt: u64,
        stem: LaunchSnapshotStem,
        path: PathBuf,
        outcome: RecordedSnapshotAttemptOutcome,
    }

    #[derive(Debug)]
    struct SequenceSnapshotAllocator {
        stems: RefCell<VecDeque<io::Result<LaunchSnapshotStem>>>,
        attempts: RefCell<Vec<RecordedSnapshotAttempt>>,
        retry_collisions: bool,
    }

    impl SequenceSnapshotAllocator {
        fn new(stems: impl IntoIterator<Item = u128>, retry_collisions: bool) -> Self {
            Self {
                stems: RefCell::new(
                    stems
                        .into_iter()
                        .map(|stem| Ok(LaunchSnapshotStem::new(stem)))
                        .collect(),
                ),
                attempts: RefCell::new(Vec::new()),
                retry_collisions,
            }
        }
    }

    impl LaunchSnapshotAllocator for SequenceSnapshotAllocator {
        fn next_stem(&self, _request: LaunchSnapshotRequest<'_>) -> io::Result<LaunchSnapshotStem> {
            self.stems
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err(io::Error::other("snapshot stem sequence exhausted")))
        }

        fn record_attempt(&self, evidence: LaunchSnapshotAttempt<'_>) {
            let outcome = match evidence.outcome {
                LaunchSnapshotAttemptOutcome::Accepted => RecordedSnapshotAttemptOutcome::Accepted,
                LaunchSnapshotAttemptOutcome::Rejected(error) => {
                    RecordedSnapshotAttemptOutcome::Rejected(error.kind(), error.to_string())
                }
            };
            self.attempts.borrow_mut().push(RecordedSnapshotAttempt {
                attempt: evidence.attempt,
                stem: evidence.stem,
                path: evidence.path.to_path_buf(),
                outcome,
            });
        }

        fn retry_collisions(&self) -> bool {
            self.retry_collisions
        }
    }

    #[test]
    fn external_edit_prepare_rejects_references_and_stamps_legacy_copy_identity() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let reference = store
            .create(CreateEntry {
                name: "Reference".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/reference.py".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        assert!(matches!(
            store.prepare_external_copy_edit(&reference),
            Err(RepositoryError::InvalidMutation { .. })
        ));

        let copy = store
            .create(copy_request("Legacy identity", "python", "script.py"))
            .unwrap();
        let meta_path = store.entry_dir(&copy.slug).join("meta.toml");
        let mut document =
            toml::from_str::<toml::Table>(&fs::read_to_string(&meta_path).unwrap()).unwrap();
        document.remove("id");
        fs::write(&meta_path, toml::to_string_pretty(&document).unwrap()).unwrap();
        let legacy = store.read_entry(copy.slug.clone()).unwrap();
        assert!(legacy.meta.id.is_none());

        let edit = store.prepare_external_copy_edit(&legacy).unwrap();

        assert!(edit.entry().meta.id.is_some());
        assert_eq!(edit.path(), store.entry_dir(&copy.slug).join("script.py"));
    }

    #[test]
    fn external_finalize_rejects_reference_outside_and_missing_multi_name_payloads() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let reference = store
            .create(CreateEntry {
                name: "Reference finalize".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Reference,
                source: "/original/reference.py".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        let reference_edit = PreparedExternalCopyEdit {
            path: store.entry_dir(&reference.slug).join("script.py"),
            entry: reference,
        };
        assert!(matches!(
            store.finalize_external_copy_edit(&reference_edit),
            Err(FinalizeExternalCopyEditError::Repository(
                RepositoryError::InvalidMutation { .. }
            ))
        ));

        let copy = store
            .create(copy_request("Outside", "python", "script.py"))
            .unwrap();
        let mut outside = store.prepare_external_copy_edit(&copy).unwrap();
        outside.path = root.path().join("outside.py");
        assert!(matches!(
            store.finalize_external_copy_edit(&outside),
            Err(FinalizeExternalCopyEditError::Repository(
                RepositoryError::InvalidMutation { .. }
            ))
        ));

        let javascript = store
            .create(copy_request("Missing module", "js", "script.js"))
            .unwrap();
        let missing = store.prepare_external_copy_edit(&javascript).unwrap();
        fs::remove_file(missing.path()).unwrap();
        assert!(matches!(
            store.finalize_external_copy_edit(&missing),
            Err(FinalizeExternalCopyEditError::Repository(
                RepositoryError::InvalidMutation { .. }
            ))
        ));
    }

    #[test]
    fn test_add_python_injected_write_failure_rolls_back_entire_entry() {
        let data = TempDir::new().unwrap();
        let origin = TempDir::new().unwrap();
        let original = origin.path().join("boom.py");
        let original_bytes = b"print('original')\n";
        fs::write(&original, original_bytes).unwrap();
        let request = || CreateEntry {
            name: "boom".to_owned(),
            kind: EntryKind::parse("python").unwrap(),
            mode: StorageMode::Copy,
            source: original.display().to_string(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            payload: Some(EntryPayload {
                bytes: original_bytes.to_vec(),
                stored_name: Some("script.py".to_owned()),
                permissions: skit_application::SourcePermissions::default(),
            }),
            settings: EntrySettings::default(),
        };
        let store = FileStore::new(data.path());
        atomic::fail_next_new_file_write();

        let error = store.create(request()).unwrap_err();

        assert!(matches!(
            error,
            RepositoryError::Io {
                operation: "write",
                ref path,
                ref reason,
            } if path.ends_with("script.py") && reason.contains("injected payload write failure")
        ));
        assert_eq!(fs::read(&original).unwrap(), original_bytes);
        assert!(!data.path().join("registry.toml").exists());
        assert!(!data.path().join("scripts").exists());
        let staging = data.path().join(".staging");
        assert!(staging.is_dir());
        assert!(fs::read_dir(&staging).unwrap().next().is_none());
        let private_names = fs::read_dir(data.path())
            .unwrap()
            .map(|item| item.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            private_names,
            [".staging", "registry.native.lock"]
                .into_iter()
                .map(std::ffi::OsString::from)
                .collect()
        );

        let retried = store.create(request()).unwrap();
        assert_eq!(retried.slug.as_str(), "boom");
        assert_eq!(
            fs::read(data.path().join("scripts/boom/script.py")).unwrap(),
            original_bytes
        );
        assert!(data.path().join("registry.toml").is_file());
        assert!(fs::read_dir(staging).unwrap().next().is_none());
        assert_eq!(fs::read(original).unwrap(), original_bytes);
    }

    #[test]
    fn remove_attempts_the_dependency_lock_before_the_entry_lock() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(CreateEntry {
                name: "Removal lock order".to_owned(),
                kind: EntryKind::parse("js").unwrap(),
                mode: StorageMode::Copy,
                source: "/original.js".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"console.log('keep');\n".to_vec(),
                    stored_name: Some("script.js".to_owned()),
                    permissions: skit_application::SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
        let dependency = store.dependency_lock(&entry.slug).unwrap();
        let release = Arc::new(Barrier::new(2));
        let worker_release = Arc::clone(&release);
        let worker_store = store.clone();
        let worker_entry = entry.clone();
        let (point_tx, point_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut first = true;
            worker_store.remove_with_lock_hook(&worker_entry, |point| {
                if first {
                    first = false;
                    point_tx.send(point).unwrap();
                    worker_release.wait();
                }
            })
        });
        let first = point_rx.recv().unwrap();

        if first == RemoveLockPoint::Dependency {
            let entry_lock = store.entry_lock(&entry.slug).unwrap();
            drop(entry_lock);
        }
        release.wait();
        drop(dependency);

        assert_eq!(first, RemoveLockPoint::Dependency);
        assert_eq!(worker.join().unwrap().unwrap(), "Removal lock order");
    }

    #[test]
    fn external_finalize_rejects_a_different_existing_file_in_the_same_entry_directory() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(CreateEntry {
                name: "Exact external source".to_owned(),
                kind: EntryKind::parse("python").unwrap(),
                mode: StorageMode::Copy,
                source: "/original.py".to_owned(),
                workdir: "origin".to_owned(),
                description: String::new(),
                payload: Some(EntryPayload {
                    bytes: b"base".to_vec(),
                    stored_name: Some("script.py".to_owned()),
                    permissions: skit_application::SourcePermissions::default(),
                }),
                settings: EntrySettings::default(),
            })
            .unwrap();
        let mut edit = store.prepare_external_copy_edit(&entry).unwrap();
        let source = edit.path.clone();
        let source_before = fs::read(&source).unwrap();
        let meta = store.entry_dir(&entry.slug).join("meta.toml");
        let meta_before = fs::read(&meta).unwrap();
        edit.path.clone_from(&meta);

        let error = store.finalize_external_copy_edit(&edit).unwrap_err();

        assert!(matches!(
            error,
            FinalizeExternalCopyEditError::Repository(RepositoryError::InvalidMutation { .. })
        ));
        assert_eq!(fs::read(source).unwrap(), source_before);
        assert_eq!(fs::read(meta).unwrap(), meta_before);
    }

    #[test]
    fn external_finalize_returns_the_locked_replacement_or_revert_bytes() {
        for (name, final_bytes) in [
            ("Locked replacement", b"replacement".as_slice()),
            ("Locked revert", b"base".as_slice()),
        ] {
            let root = TempDir::new().unwrap();
            let store = FileStore::new(root.path());
            let entry = store
                .create(CreateEntry {
                    name: name.to_owned(),
                    kind: EntryKind::parse("python").unwrap(),
                    mode: StorageMode::Copy,
                    source: "/original.py".to_owned(),
                    workdir: "origin".to_owned(),
                    description: String::new(),
                    payload: Some(EntryPayload {
                        bytes: b"base".to_vec(),
                        stored_name: Some("script.py".to_owned()),
                        permissions: skit_application::SourcePermissions::default(),
                    }),
                    settings: EntrySettings::default(),
                })
                .unwrap();
            let edit = store.prepare_external_copy_edit(&entry).unwrap();
            fs::write(edit.path(), b"first editor snapshot").unwrap();
            let held = store.entry_lock(&entry.slug).unwrap();
            let barrier = Arc::new(Barrier::new(2));
            let worker_barrier = Arc::clone(&barrier);
            let worker_store = store.clone();
            let worker_edit = edit.clone();
            let worker = thread::spawn(move || {
                worker_barrier.wait();
                worker_store.finalize_external_copy_edit(&worker_edit)
            });
            barrier.wait();

            // The finalize read cannot pass the held entry lock until this replacement completes.
            fs::write(edit.path(), final_bytes).unwrap();
            drop(held);
            let finalized = worker.join().unwrap().unwrap();

            assert_eq!(finalized.bytes(), final_bytes);
            assert_eq!(fs::read(edit.path()).unwrap(), final_bytes);
            assert_eq!(
                finalized.entry().meta.source_hash,
                content_hash(final_bytes)
            );
        }
    }

    #[test]
    fn rebuild_isolates_a_metadata_file_removed_after_its_authoritative_read() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        store
            .create(CreateEntry {
                name: "Raced".to_owned(),
                kind: EntryKind::parse("command").unwrap(),
                mode: StorageMode::Copy,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings::default(),
            })
            .unwrap();
        REBUILD_BEFORE_PROJECT_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|path| fs::remove_file(path).unwrap()));
        });

        let report = store.rebuild_registry_report().unwrap();

        assert_eq!(report.entry_count, 0);
        assert_eq!(report.problems.len(), 1);
        assert_eq!(
            rebuild_error_reason(RepositoryError::NotFound {
                query: "missing".to_owned(),
            }),
            "entry not found: missing"
        );
    }

    #[test]
    fn metadata_encoding_reports_its_boundary_failures() {
        let root = TempDir::new().unwrap();
        let meta = EntryMeta::minimal("Demo", skit_domain::EntryKind::parse("shell").unwrap());
        let missing = root.path().join("missing.toml");
        assert!(
            encode_metadata(&missing, &meta)
                .unwrap()
                .contains("name = \"Demo\"")
        );

        let unreadable = root.path().join("directory.toml");
        fs::create_dir(&unreadable).unwrap();
        assert!(matches!(
            encode_metadata(&unreadable, &meta),
            Err(RepositoryError::Io {
                operation: "read",
                ..
            })
        ));
    }

    #[test]
    fn a_failed_rollback_reports_both_failures_and_the_affected_path() {
        let primary = invalid(Message::new("primary failed"));
        let rollback = Err(invalid(Message::new("restore failed")));

        let error = rollback_error(primary, rollback, Path::new("affected"));

        assert!(matches!(
            error,
            RepositoryError::Rollback {
                ref path,
                ref primary,
                ref rollback,
            } if path == "affected"
                && primary.to_string().contains("primary failed")
                && rollback.to_string().contains("restore failed")
        ));
    }

    #[test]
    fn source_replacement_restores_bytes_when_metadata_fails() {
        let root = TempDir::new().unwrap();
        let source = root.path().join("source");
        fs::write(&source, b"before").unwrap();

        assert!(
            replace_source(&source, b"after", b"before", || Err(invalid(Message::new(
                "metadata failed"
            ))))
            .is_err()
        );
        assert_eq!(fs::read(source).unwrap(), b"before");
    }

    #[test]
    fn source_rollback_failure_preserves_both_error_causes() {
        let root = TempDir::new().unwrap();
        let source = root.path().join("source");
        fs::write(&source, b"before").unwrap();
        let directory = root.path().to_owned();

        let error = replace_source(&source, b"after", b"before", || {
            fs::remove_dir_all(&directory).unwrap();
            fs::write(&directory, "block parent recreation").unwrap();
            Err(invalid(Message::new("metadata failed")))
        })
        .unwrap_err();

        assert!(matches!(error, RepositoryError::Rollback { .. }));

        let error = rollback_source_projection(
            invalid(Message::new("projection failed")),
            &source,
            b"before",
            restore_succeeds,
        );
        assert!(matches!(error, RepositoryError::Rollback { .. }));
    }

    fn restore_succeeds() -> Result<(), RepositoryError> {
        Ok(())
    }

    #[test]
    fn a_successful_rollback_keeps_the_first_cause() {
        let root = TempDir::new().unwrap();
        let source = root.path().join("source");
        fs::write(&source, b"after").unwrap();

        let error = rollback_source_projection(
            invalid(Message::new("projection failed")),
            &source,
            b"before",
            restore_succeeds,
        );

        assert!(matches!(error, RepositoryError::InvalidMutation { .. }));
        assert_eq!(fs::read(&source).unwrap(), b"before");
    }

    #[test]
    fn launch_snapshot_stems_and_attempt_overflow_are_exact() {
        assert_eq!(
            LaunchSnapshotStem::new(0).to_string(),
            "00000000000000000000000000000000"
        );
        assert_eq!(
            LaunchSnapshotStem::new(u128::MAX).to_string(),
            "ffffffffffffffffffffffffffffffff"
        );
        assert_eq!(LaunchSnapshotStem::new(42).value(), 42);
        assert_eq!(next_launch_snapshot_attempt(41).unwrap(), 42);
        assert_eq!(
            next_launch_snapshot_attempt(u64::MAX)
                .unwrap_err()
                .to_string(),
            "launch snapshot attempt counter overflow"
        );
        let root = TempDir::new().unwrap();
        let generated = SystemLaunchSnapshotAllocator
            .next_stem(LaunchSnapshotRequest {
                attempt: 0,
                directory: root.path(),
                suffix: ".sh",
            })
            .unwrap();
        assert!(!SystemLaunchSnapshotAllocator.retry_collisions());
        assert_eq!(generated.to_string().len(), 32);
        assert!(
            generated
                .to_string()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        );
    }

    #[test]
    fn stale_launch_snapshot_removal_is_limited_to_reserved_regular_files() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Stale", "shell", "script.sh"))
            .unwrap();
        let entry_dir = store.entry_dir(&entry.slug);
        let outside = root.path().join(".run-outside.sh");
        fs::write(&outside, b"outside").unwrap();
        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &outside)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(outside.exists());

        let ordinary = entry_dir.join("ordinary.sh");
        fs::write(&ordinary, b"ordinary").unwrap();
        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &ordinary)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert!(ordinary.exists());

        let directory = entry_dir.join(".run-directory");
        fs::create_dir(&directory).unwrap();
        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &directory)
                .unwrap_err()
                .to_string(),
            "the stale launch snapshot is not a regular file"
        );
        assert!(directory.is_dir());

        let snapshot = entry_dir.join(".run-stale.sh");
        fs::write(&snapshot, b"stale").unwrap();
        store
            .remove_stale_launch_snapshot(&entry, &snapshot)
            .unwrap();
        assert!(!snapshot.exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            for (name, mode) in [(".run-mode-zero", 0o000), (".run-write-only", 0o200)] {
                let unreadable = entry_dir.join(name);
                fs::write(&unreadable, b"unreadable").unwrap();
                fs::set_permissions(&unreadable, fs::Permissions::from_mode(mode)).unwrap();
                assert_eq!(
                    store
                        .remove_stale_launch_snapshot(&entry, &unreadable)
                        .unwrap_err()
                        .kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert!(unreadable.exists());
                fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn stale_launch_snapshot_removal_never_follows_a_reserved_symlink() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Symlink", "shell", "script.sh"))
            .unwrap();
        let target = root.path().join("target");
        let link = store.entry_dir(&entry.slug).join(".run-link");
        fs::write(&target, b"target").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        fs::write(&link, b"stale").unwrap();
        let target_mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        STALE_BEFORE_REMOVE_HOOK.with(|hook| {
            let target = target.clone();
            *hook.borrow_mut() = Some(Box::new(move |path| {
                fs::remove_file(path).unwrap();
                symlink(target, path).unwrap();
            }));
        });

        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &link)
                .unwrap_err()
                .to_string(),
            "the launch snapshot path no longer matches its owned file"
        );
        assert_eq!(fs::read(target).unwrap(), b"target");
        assert_eq!(
            fs::metadata(root.path().join("target"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            target_mode
        );
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &link)
                .unwrap_err()
                .to_string(),
            "the stale launch snapshot is not a regular file"
        );

        let reused = store.entry_dir(&entry.slug).join(".run-reused");
        fs::write(&reused, b"stale").unwrap();
        STALE_BEFORE_REMOVE_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|path| {
                fs::remove_file(path).unwrap();
                fs::write(path, b"replacement").unwrap();
            }));
        });
        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &reused)
                .unwrap_err()
                .to_string(),
            "the launch snapshot path no longer matches its owned file"
        );
        assert_eq!(fs::read(reused).unwrap(), b"replacement");
    }

    #[cfg(windows)]
    #[test]
    fn stale_launch_snapshot_removal_clears_readonly_before_delete() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Readonly", "shell", "script.sh"))
            .unwrap();
        let snapshot = store.entry_dir(&entry.slug).join(".run-readonly.sh");
        fs::write(&snapshot, b"readonly").unwrap();
        let mut permissions = fs::metadata(&snapshot).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&snapshot, permissions).unwrap();

        store
            .remove_stale_launch_snapshot(&entry, &snapshot)
            .unwrap();

        assert!(!snapshot.exists());

        let linked = store.entry_dir(&entry.slug).join(".run-hard-linked.sh");
        let sibling = store.entry_dir(&entry.slug).join("hard-link-sibling");
        fs::write(&linked, b"linked readonly").unwrap();
        let mut permissions = fs::metadata(&linked).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&linked, permissions).unwrap();
        fs::hard_link(&linked, &sibling).unwrap();

        assert_eq!(
            store
                .remove_stale_launch_snapshot(&entry, &linked)
                .unwrap_err()
                .to_string(),
            "the launch snapshot has more than one hard link"
        );
        assert!(linked.exists());
        assert!(sibling.exists());
        assert!(fs::metadata(&linked).unwrap().permissions().readonly());
        let mut cleanup_permissions = fs::metadata(&linked).unwrap().permissions();
        cleanup_permissions.set_readonly(false);
        fs::set_permissions(&linked, cleanup_permissions).unwrap();
        fs::remove_file(linked).unwrap();
        fs::remove_file(sibling).unwrap();
    }

    #[test]
    fn launch_snapshot_validation_guards_reject_nonempty_and_nonzero_handles() {
        use std::io::{Seek as _, SeekFrom};

        let root = TempDir::new().unwrap();
        let nonempty_path = root.path().join("nonempty");
        let mut nonempty = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&nonempty_path)
            .unwrap();
        nonempty.write_all(b"x").unwrap();
        let nonempty_identity = launch_snapshot_identity(&nonempty).unwrap();
        assert_eq!(
            validate_new_launch_snapshot(&mut nonempty, &nonempty_path, &nonempty_identity)
                .unwrap_err()
                .to_string(),
            "the launch snapshot handle is not empty"
        );

        let positioned_path = root.path().join("positioned");
        let mut positioned = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&positioned_path)
            .unwrap();
        positioned.seek(SeekFrom::Start(1)).unwrap();
        let positioned_identity = launch_snapshot_identity(&positioned).unwrap();
        assert_eq!(
            validate_new_launch_snapshot(&mut positioned, &positioned_path, &positioned_identity)
                .unwrap_err()
                .to_string(),
            "the launch snapshot handle is not at position zero"
        );
    }

    #[test]
    fn launch_snapshot_identity_validation_accepts_current_and_rejects_replacement() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("snapshot");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let identity = launch_snapshot_identity(&file).unwrap();

        assert!(identity.matches_path(&path).unwrap());
        validate_launch_snapshot_path(&path, &identity).unwrap();

        fs::rename(&path, root.path().join("parked")).unwrap();
        fs::write(&path, b"replacement").unwrap();

        assert!(!identity.matches_path(&path).unwrap());
        assert_eq!(
            validate_launch_snapshot_path(&path, &identity)
                .unwrap_err()
                .to_string(),
            "the launch snapshot path does not match its open handle"
        );
    }

    #[cfg(unix)]
    #[test]
    fn launch_snapshot_validation_guards_reject_nonregular_handles_and_paths() {
        use std::os::unix::fs::symlink;

        let root = TempDir::new().unwrap();
        let mut directory = File::open(root.path()).unwrap();
        let directory_identity = launch_snapshot_identity(&directory).unwrap();
        assert!(!directory_identity.matches_path(root.path()).unwrap());
        assert_eq!(
            validate_new_launch_snapshot(&mut directory, root.path(), &directory_identity)
                .unwrap_err()
                .to_string(),
            "the launch snapshot handle is not a regular file"
        );

        let file_path = root.path().join("file");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&file_path)
            .unwrap();
        let identity = launch_snapshot_identity(&file).unwrap();
        assert_eq!(
            validate_launch_snapshot_path(root.path(), &identity)
                .unwrap_err()
                .to_string(),
            "the launch snapshot path is not a regular file"
        );
        let symlink_path = root.path().join("file-link");
        symlink(&file_path, &symlink_path).unwrap();
        assert!(!identity.matches_path(&symlink_path).unwrap());
        assert_eq!(
            validate_launch_snapshot_path(&symlink_path, &identity)
                .unwrap_err()
                .to_string(),
            "the launch snapshot path is not a regular file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn final_identity_validation_accepts_mode_zero_and_write_only_snapshots() {
        use std::os::unix::fs::PermissionsExt as _;

        for (stem, mode) in [(21, 0o000), (22, 0o200)] {
            let root = TempDir::new().unwrap();
            let allocator = SequenceSnapshotAllocator::new([stem], false);
            let temporary = write_launch_snapshot_with_finalize(
                root.path(),
                ".sh",
                b"printf checked",
                fs::Permissions::from_mode(mode),
                &allocator,
                File::sync_all,
            )
            .unwrap();
            let path = temporary.path.clone();
            let metadata = fs::symlink_metadata(&path).unwrap();

            assert_eq!(metadata.permissions().mode() & 0o777, mode);
            assert_eq!(metadata.len(), b"printf checked".len() as u64);
            drop(temporary);
            assert!(!path.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn launch_snapshot_starts_private_then_uses_the_source_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        const CHILD: &str = "SKIT_STORE_SNAPSHOT_UMASK_CHILD";
        const TEST: &str =
            "mutations::tests::launch_snapshot_starts_private_then_uses_the_source_mode";
        if std::env::var_os(CHILD).is_some() {
            let root = TempDir::new().unwrap();
            fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
            let initial_mode = Rc::new(Cell::new(u32::MAX));
            let observed_mode = Rc::clone(&initial_mode);
            LAUNCH_AFTER_OPEN_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |path| {
                    observed_mode
                        .set(fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777);
                }));
            });
            let allocator = SequenceSnapshotAllocator::new([31], false);
            let temporary = write_launch_snapshot_with_finalize(
                root.path(),
                ".sh",
                b"printf checked",
                fs::Permissions::from_mode(0o640),
                &allocator,
                File::sync_all,
            )
            .unwrap();

            assert_eq!(initial_mode.get(), 0o600);
            assert_eq!(
                fs::symlink_metadata(&temporary.path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o640
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

    #[test]
    fn launch_snapshot_collision_retry_is_an_explicit_allocator_policy() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Retry", "shell", "script.sh"))
            .unwrap();
        let entry_dir = store.entry_dir(&entry.slug);
        let occupied = entry_dir.join(".run-00000000000000000000000000000000.sh");
        fs::write(&occupied, b"occupied").unwrap();
        let retrying = SequenceSnapshotAllocator::new([0, 1], true);

        let prepared = store
            .prepare_launch_with_snapshot_allocator(
                &entry,
                Some(&entry.meta.source_hash),
                &retrying,
            )
            .unwrap();

        assert_eq!(fs::read(&occupied).unwrap(), b"occupied");
        assert_eq!(
            prepared.payload_path().unwrap().file_name().unwrap(),
            ".run-00000000000000000000000000000001.sh"
        );
        assert_eq!(retrying.attempts.borrow().len(), 2);
        assert!(matches!(
            &retrying.attempts.borrow()[0],
            RecordedSnapshotAttempt {
                attempt: 0,
                stem,
                outcome: RecordedSnapshotAttemptOutcome::Rejected(
                    io::ErrorKind::AlreadyExists,
                    _
                ),
                ..
            } if *stem == LaunchSnapshotStem::new(0)
        ));
        assert!(matches!(
            &retrying.attempts.borrow()[1],
            RecordedSnapshotAttempt {
                attempt: 1,
                stem,
                outcome: RecordedSnapshotAttemptOutcome::Accepted,
                ..
            } if *stem == LaunchSnapshotStem::new(1)
        ));

        drop(prepared);
        let refusing = SequenceSnapshotAllocator::new([0, 2], false);
        let error = store
            .prepare_launch_with_snapshot_allocator(
                &entry,
                Some(&entry.meta.source_hash),
                &refusing,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RepositoryError::Io {
                operation: "create",
                ..
            }
        ));
        assert_eq!(refusing.attempts.borrow().len(), 1);
        assert_eq!(refusing.stems.borrow().len(), 1);
        assert_eq!(fs::read(occupied).unwrap(), b"occupied");
    }

    #[test]
    fn validation_failure_leaves_a_replacement_at_the_store_owned_path() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Validation race", "shell", "script.sh"))
            .unwrap();
        let replacement = store
            .entry_dir(&entry.slug)
            .join(".run-00000000000000000000000000000000.sh");
        LAUNCH_AFTER_OPEN_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|path| {
                fs::rename(path, path.with_extension("parked")).unwrap();
                fs::write(path, b"replacement").unwrap();
            }));
        });
        let allocator = SequenceSnapshotAllocator::new([0], false);

        let error = store
            .prepare_launch_with_snapshot_allocator(
                &entry,
                Some(&entry.meta.source_hash),
                &allocator,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            RepositoryError::Io {
                operation: "inspect",
                ..
            }
        ));
        assert_eq!(fs::read(replacement).unwrap(), b"replacement");
        assert!(matches!(
            allocator.attempts.borrow().as_slice(),
            [RecordedSnapshotAttempt {
                outcome: RecordedSnapshotAttemptOutcome::Rejected(
                    io::ErrorKind::InvalidData,
                    reason
                ),
                ..
            }] if reason == "the launch snapshot path does not match its open handle"
        ));
    }

    #[test]
    fn chmod_failures_remove_only_the_same_file_candidate() {
        for (stem, replace, identity_fault) in [
            (38, false, Some(LaunchIdentityFault::Clone)),
            (39, false, None),
            (40, true, None),
        ] {
            let root = TempDir::new().unwrap();
            let store = FileStore::new(root.path());
            let entry = store
                .create(copy_request("Initial chmod failure", "shell", "script.sh"))
                .unwrap();
            LAUNCH_INITIAL_MODE_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |_, path| {
                    if replace {
                        fs::rename(path, path.with_extension("parked")).unwrap();
                        fs::write(path, b"replacement").unwrap();
                    }
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected initial launch snapshot chmod failure",
                    ))
                }));
            });
            if let Some(fault) = identity_fault {
                LAUNCH_IDENTITY_FAULT.with(|injected| injected.set(Some(fault)));
            }
            let allocator = SequenceSnapshotAllocator::new([stem], false);
            let candidate = store.entry_dir(&entry.slug).join(format!(
                ".run-{}{}",
                LaunchSnapshotStem::new(stem),
                ".sh"
            ));

            let error = store
                .prepare_launch_with_snapshot_allocator(
                    &entry,
                    Some(&entry.meta.source_hash),
                    &allocator,
                )
                .unwrap_err();

            assert!(matches!(
                error,
                RepositoryError::Io {
                    operation: "chmod",
                    ..
                }
            ));
            match (replace, identity_fault) {
                (true, _) => assert_eq!(fs::read(candidate).unwrap(), b"replacement"),
                (false, Some(_)) => assert_eq!(fs::metadata(candidate).unwrap().len(), 0),
                (false, None) => assert!(!candidate.exists()),
            }
            assert!(matches!(
                allocator.attempts.borrow().as_slice(),
                [RecordedSnapshotAttempt {
                    outcome: RecordedSnapshotAttemptOutcome::Rejected(
                        io::ErrorKind::PermissionDenied,
                        _
                    ),
                    ..
                }]
            ));
        }

        for (stem, replace) in [(41, false), (42, true)] {
            let root = TempDir::new().unwrap();
            let store = FileStore::new(root.path());
            let entry = store
                .create(copy_request("Chmod failure", "shell", "script.sh"))
                .unwrap();
            LAUNCH_CHMOD_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move |_, path, _| {
                    if replace {
                        fs::rename(path, path.with_extension("parked")).unwrap();
                        fs::write(path, b"replacement").unwrap();
                    }
                    Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "injected launch snapshot chmod failure",
                    ))
                }));
            });
            let allocator = SequenceSnapshotAllocator::new([stem], false);
            let candidate = store.entry_dir(&entry.slug).join(format!(
                ".run-{}{}",
                LaunchSnapshotStem::new(stem),
                ".sh"
            ));

            let error = store
                .prepare_launch_with_snapshot_allocator(
                    &entry,
                    Some(&entry.meta.source_hash),
                    &allocator,
                )
                .unwrap_err();

            assert!(matches!(
                error,
                RepositoryError::Io {
                    operation: "chmod",
                    ..
                }
            ));
            if replace {
                assert_eq!(fs::read(candidate).unwrap(), b"replacement");
            } else {
                assert!(!candidate.exists());
            }
            assert!(matches!(
                allocator.attempts.borrow().as_slice(),
                [RecordedSnapshotAttempt {
                    outcome: RecordedSnapshotAttemptOutcome::Accepted,
                    ..
                }]
            ));
        }
    }

    #[test]
    fn identity_setup_failures_never_unlink_an_unverified_replacement() {
        for (fault, stem, reason) in [
            (
                LaunchIdentityFault::Clone,
                11,
                "injected launch snapshot clone failure",
            ),
            (
                LaunchIdentityFault::Handle,
                12,
                "injected launch snapshot handle failure",
            ),
        ] {
            let root = TempDir::new().unwrap();
            let store = FileStore::new(root.path());
            let entry = store
                .create(copy_request("Identity failure", "shell", "script.sh"))
                .unwrap();
            let allocator = SequenceSnapshotAllocator::new([stem], false);
            LAUNCH_AFTER_OPEN_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(|path| {
                    fs::rename(path, path.with_extension("parked")).unwrap();
                    fs::write(path, b"replacement").unwrap();
                }));
            });
            LAUNCH_IDENTITY_FAULT.with(|injected| injected.set(Some(fault)));

            let error = store
                .prepare_launch_with_snapshot_allocator(
                    &entry,
                    Some(&entry.meta.source_hash),
                    &allocator,
                )
                .unwrap_err();
            let candidate = store.entry_dir(&entry.slug).join(format!(
                ".run-{}{}",
                LaunchSnapshotStem::new(stem),
                ".sh"
            ));

            assert!(matches!(
                error,
                RepositoryError::Io {
                    operation: "inspect",
                    ..
                }
            ));
            assert_eq!(fs::read(candidate).unwrap(), b"replacement");
            assert!(matches!(
                allocator.attempts.borrow().as_slice(),
                [RecordedSnapshotAttempt {
                    outcome: RecordedSnapshotAttemptOutcome::Rejected(
                        io::ErrorKind::Other,
                        recorded
                    ),
                    ..
                }] if recorded == reason
            ));
        }
    }

    #[test]
    fn prepared_launch_drop_leaves_a_replacement_at_its_snapshot_path() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Drop race", "shell", "script.sh"))
            .unwrap();
        let allocator = SequenceSnapshotAllocator::new([7], false);
        let prepared = store
            .prepare_launch_with_snapshot_allocator(
                &entry,
                Some(&entry.meta.source_hash),
                &allocator,
            )
            .unwrap();
        let snapshot = prepared.payload_path().unwrap().to_path_buf();
        fs::rename(&snapshot, snapshot.with_extension("parked")).unwrap();
        fs::write(&snapshot, b"replacement").unwrap();

        drop(prepared);

        assert_eq!(fs::read(snapshot).unwrap(), b"replacement");
    }

    #[cfg(unix)]
    #[test]
    fn retained_file_handle_prevents_unlinked_inode_reuse_from_deleting_replacement() {
        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Unlink race", "shell", "script.sh"))
            .unwrap();
        let allocator = SequenceSnapshotAllocator::new([8], false);
        let prepared = store
            .prepare_launch_with_snapshot_allocator(
                &entry,
                Some(&entry.meta.source_hash),
                &allocator,
            )
            .unwrap();
        let snapshot = prepared.payload_path().unwrap().to_path_buf();
        fs::remove_file(&snapshot).unwrap();
        fs::write(&snapshot, b"replacement").unwrap();

        drop(prepared);

        assert_eq!(fs::read(snapshot).unwrap(), b"replacement");
    }

    #[test]
    fn a_failed_launch_snapshot_finalize_removes_the_private_file() {
        for (stem, replace_after_clear) in [(0, false), (1, true)] {
            let root = TempDir::new().unwrap();
            let source = root.path().join("source.sh");
            let snapshot = root.path().join(format!(".run-{stem:032x}.sh"));
            fs::write(&source, b"printf checked").unwrap();
            if replace_after_clear {
                LAUNCH_AFTER_CLEAR_READONLY_HOOK.with(|hook| {
                    *hook.borrow_mut() =
                        Some(Box::new(replace_launch_snapshot_after_readonly_clear));
                });
            }

            let error = write_launch_snapshot_with(
                &source,
                &snapshot,
                b"printf checked",
                fail_launch_snapshot_sync,
            )
            .unwrap_err();

            assert!(matches!(
                error,
                RepositoryError::Io {
                    operation: "sync",
                    ..
                }
            ));
            if replace_after_clear {
                assert_eq!(fs::read(snapshot).unwrap(), b"replacement");
            } else {
                assert!(!snapshot.exists());
            }
        }
    }

    #[test]
    fn source_permission_inspection_precedes_snapshot_stem_allocation() {
        #[derive(Debug, Default)]
        struct CountingAllocator {
            calls: Cell<u64>,
        }

        impl LaunchSnapshotAllocator for CountingAllocator {
            fn next_stem(
                &self,
                _request: LaunchSnapshotRequest<'_>,
            ) -> io::Result<LaunchSnapshotStem> {
                self.calls.set(self.calls.get() + 1);
                Ok(LaunchSnapshotStem::new(91))
            }
        }

        let root = TempDir::new().unwrap();
        let store = FileStore::new(root.path());
        let entry = store
            .create(copy_request("Demo", "shell", "script.sh"))
            .unwrap();
        LAUNCH_BEFORE_PERMISSIONS_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(|path| fs::remove_file(path).unwrap()));
        });
        let allocator = CountingAllocator::default();

        let error = store
            .prepare_launch_with_snapshot_allocator(
                &entry,
                Some(&entry.meta.source_hash),
                &allocator,
            )
            .unwrap_err();

        assert!(matches!(
            error,
            RepositoryError::Io {
                operation: "inspect",
                ..
            }
        ));
        assert!(
            fs::read_dir(store.entry_dir(&entry.slug))
                .unwrap()
                .all(|item| !item
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".run-"))
        );
        assert_eq!(allocator.calls.get(), 0);
        assert_eq!(
            allocator
                .next_stem(LaunchSnapshotRequest {
                    attempt: 0,
                    directory: root.path(),
                    suffix: ".sh",
                })
                .unwrap(),
            LaunchSnapshotStem::new(91)
        );
        assert_eq!(allocator.calls.get(), 1);
    }
}
