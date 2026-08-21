mod agent_skill;
mod atomic;
mod hash;
pub(crate) mod registry;
mod runner_management;

use std::{
    collections::BTreeMap,
    fs,
    fs::{File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::fs_ops::sync_directory;
pub use agent_skill::FileAgentSkillStore;
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
    CreateEntry, EntryMutationRepository, EntryPayload, RepositoryError, UpdateEntry,
};
use skit_domain::{Entry, EntryId, EntryMeta, EntrySettings, Slug, StorageMode};
use skit_i18n::{Localize as _, Message};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::{
    FileStore,
    paths::{is_support_file, stored_filenames},
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
    temporary_payload: Option<PathBuf>,
    _lease: FileLock,
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

impl Drop for PreparedLaunch {
    fn drop(&mut self) {
        if let Some(path) = self.temporary_payload.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
}

impl EntryMutationRepository for FileStore {
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
        let _launch = self.launch_lock(&entry.slug)?;
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let _dependencies = self.dependency_lock(&entry.slug)?;
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
}

impl FileStore {
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
        let bytes = fs::read(&source).map_err(|error| io_error("read", &source, error))?;
        if let Some(expected) = expected_source_hash {
            let actual = content_hash(&bytes);
            if actual != expected {
                return Err(RepositoryError::SourceChanged {
                    slug: fresh.slug.as_str().to_owned(),
                    expected: expected.to_owned(),
                    actual,
                });
            }
        }

        if fresh.meta.mode == StorageMode::Reference {
            return Ok(PreparedLaunch {
                entry: fresh,
                payload: Some(source),
                temporary_payload: None,
                _lease: lease,
            });
        }

        let snapshot = launch_snapshot_path(&source, &self.entry_dir(&fresh.slug));
        write_launch_snapshot(&source, &snapshot, &bytes)?;
        Ok(PreparedLaunch {
            entry: fresh,
            payload: Some(snapshot.clone()),
            temporary_payload: Some(snapshot),
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
            added_at: format_added_at(OffsetDateTime::now_utc())?,
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

fn launch_snapshot_path(source: &Path, entry_dir: &Path) -> PathBuf {
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map_or_else(String::new, |extension| format!(".{extension}"));
    entry_dir.join(format!(
        ".run-{}{}",
        EntryId::generate().as_str(),
        extension
    ))
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

fn write_launch_snapshot(
    source: &Path,
    snapshot: &Path,
    bytes: &[u8],
) -> Result<(), RepositoryError> {
    write_launch_snapshot_with(source, snapshot, bytes, File::sync_all)
}

fn write_launch_snapshot_with(
    source: &Path,
    snapshot: &Path,
    bytes: &[u8],
    finalize: impl FnOnce(&File) -> io::Result<()>,
) -> Result<(), RepositoryError> {
    let permissions = fs::metadata(source)
        .map_err(|error| io_error("inspect", source, error))?
        .permissions();
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options
        .open(snapshot)
        .map_err(|error| io_error("create", snapshot, error))?;
    let result = file
        .write_all(bytes)
        .map_err(|error| io_error("write", snapshot, error))
        .and_then(|()| {
            file.set_permissions(permissions)
                .map_err(|error| io_error("chmod", snapshot, error))
        })
        .and_then(|()| finalize(&file).map_err(|error| io_error("sync", snapshot, error)));
    if result.is_err() {
        drop(file);
        let _ = fs::remove_file(snapshot);
    }
    result
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

fn format_added_at(timestamp: OffsetDateTime) -> Result<String, RepositoryError> {
    timestamp
        .format(&Rfc3339)
        .map_err(|error| invalid(Message::new("could not format add timestamp: {}").with(error)))
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
    use skit_domain::EntryKind;
    use tempfile::TempDir;
    use time::{Date, Month};

    use super::*;

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
    fn timestamp_and_metadata_encoding_report_their_boundary_failures() {
        let ancient = Date::from_calendar_date(-1, Month::January, 1)
            .unwrap()
            .midnight()
            .assume_utc();
        assert!(matches!(
            format_added_at(ancient),
            Err(RepositoryError::InvalidMutation { .. })
        ));

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
    fn a_failed_launch_snapshot_finalize_removes_the_private_file() {
        let root = TempDir::new().unwrap();
        let source = root.path().join("source.sh");
        let snapshot = root.path().join(".run-test.sh");
        fs::write(&source, b"printf checked").unwrap();

        let error = write_launch_snapshot_with(&source, &snapshot, b"printf checked", |_| {
            Err(io::Error::other("injected sync failure"))
        })
        .unwrap_err();

        assert!(matches!(
            error,
            RepositoryError::Io {
                operation: "sync",
                ..
            }
        ));
        assert!(!snapshot.exists());
    }
}
