mod atomic;
mod hash;
pub(crate) mod registry;

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use atomic::{
    FileLock, StagedDirectory, acquire_lock, atomic_write_bytes, create_dir_all, invalid, io_error,
    write_new_file, write_new_metadata,
};
pub use hash::content_hash;
use registry::Registry;
use skit_application::{CreateEntry, EntryMutationRepository, EntryPayload, RepositoryError};
use skit_domain::{Entry, EntryId, EntryMeta, EntrySettings, Slug, StorageMode};

use super::FileStore;

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

    fn rename(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        let name = validated_name(name)?;
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        self.ensure_name_available(&name, Some(&fresh.slug), &registry)?;
        let new_slug =
            self.allocate_slug(Slug::from_display_name(&name), Some(&fresh.slug), &registry)?;

        let before = fresh.clone();
        let mut after = fresh;
        after.meta.name = name;
        if before.slug == new_slug {
            self.commit_meta_projection(&before, &after, &mut registry)?;
            return Ok(after);
        }

        let old_dir = self.entry_dir(&before.slug);
        let new_dir = self.entry_dir(&new_slug);
        self.write_meta(&after)?;
        if let Err(error) = fs::rename(&old_dir, &new_dir) {
            let primary = io_error("rename", &old_dir, error);
            return Err(rollback_error(primary, self.write_meta(&before), &old_dir));
        }
        after.slug = new_slug;
        registry.remove(&before.slug);
        let projection = registry
            .project(&after, &new_dir)
            .and_then(|()| registry.save());
        if let Err(error) = projection {
            let move_back = fs::rename(&new_dir, &old_dir)
                .map_err(|rollback| io_error("rollback rename", &new_dir, rollback));
            if let Err(rollback) = move_back {
                return Err(rollback_error(error, Err(rollback), &old_dir));
            }
            return Err(rollback_error(error, self.write_meta(&before), &old_dir));
        }
        Ok(after)
    }

    fn remove(&self, entry: &Entry) -> Result<String, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut registry = Registry::load(self.data_dir())?;
        let fresh = self.claim_for_mutation(entry, &mut registry)?;
        let name = fresh.meta.name.clone();
        let source = self.entry_dir(&fresh.slug);
        let trash_root = self.data_dir().join(".trash");
        create_dir_all(&trash_root, "create")?;
        let trash = trash_root.join(format!("{}-{}", fresh.slug, EntryId::generate().as_str()));
        fs::rename(&source, &trash).map_err(|error| io_error("remove", &source, error))?;

        registry.remove(&fresh.slug);
        if let Err(error) = registry.save() {
            let rollback = fs::rename(&trash, &source)
                .map_err(|rollback| io_error("rollback remove", &trash, rollback));
            return Err(rollback_error(error, rollback, &source));
        }
        fs::remove_dir_all(&trash).map_err(|error| io_error("clean", &trash, error))?;
        Ok(name)
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
            return Err(invalid(
                "reference entries are edited at their original path",
            ));
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
        atomic_write_bytes(&target, bytes)?;
        after.meta.source_hash = content_hash(bytes);
        if let Err(error) = self.write_meta(&after) {
            return Err(rollback_error(
                error,
                atomic_write_bytes(&target, &original),
                &target,
            ));
        }
        let projection = registry
            .project(&after, &self.entry_dir(&after.slug))
            .and_then(|()| registry.save());
        if let Err(error) = projection {
            let source_rollback = atomic_write_bytes(&target, &original);
            if let Err(rollback) = source_rollback {
                return Err(rollback_error(error, Err(rollback), &target));
            }
            return Err(rollback_error(
                error,
                self.write_meta(&before),
                &self.entry_dir(&before.slug),
            ));
        }
        Ok(after)
    }
}

impl FileStore {
    /// Rebuild `registry.toml` from every valid authoritative metadata file.
    pub fn rebuild_registry(&self) -> Result<usize, RepositoryError> {
        let _namespace = self.namespace_lock()?;
        let scripts = self.scripts_dir();
        let mut registry = Registry::fresh(self.data_dir());
        let mut count = 0;
        let reader = match fs::read_dir(&scripts) {
            Ok(reader) => reader,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                registry.save()?;
                return Ok(0);
            }
            Err(error) => return Err(io_error("scan", &scripts, error)),
        };
        for item in reader {
            let item = item.map_err(|error| io_error("scan", &scripts, error))?;
            if !item
                .file_type()
                .map_err(|error| io_error("inspect", &item.path(), error))?
                .is_dir()
            {
                continue;
            }
            let Some(slug) = item
                .file_name()
                .to_str()
                .and_then(|name| Slug::parse(name.to_owned()).ok())
            else {
                continue;
            };
            let Ok(entry) = self.read_entry(slug) else {
                continue;
            };
            registry.project(&entry, &item.path())?;
            count += 1;
        }
        registry.save()?;
        Ok(count)
    }

    fn create_locked(
        &self,
        mut request: CreateEntry,
        registry: &mut Registry,
    ) -> Result<Entry, RepositoryError> {
        request.name = validated_name(&request.name)?;
        self.ensure_name_available(&request.name, None, registry)?;
        validate_payload(request.mode, request.payload.as_ref())?;
        let slug = self.allocate_slug(Slug::from_display_name(&request.name), None, registry)?;
        let id = EntryId::generate();
        let source_hash = request
            .payload
            .as_ref()
            .map_or_else(String::new, |payload| content_hash(&payload.bytes));
        let meta = EntryMeta {
            schema: 1,
            name: request.name,
            kind: request.kind,
            mode: request.mode,
            source: request.source,
            source_hash,
            added_at: String::new(),
            id: Some(id.clone()),
            workdir: request.workdir,
            description: request.description,
            extra: BTreeMap::new(),
        };

        let staging_root = self.data_dir().join(".staging");
        create_dir_all(&staging_root, "create")?;
        let stage_path = staging_root.join(format!("{}-{}", slug, id.as_str()));
        fs::create_dir(&stage_path).map_err(|error| io_error("create", &stage_path, error))?;
        let mut stage = StagedDirectory::new(stage_path);

        if let (StorageMode::Copy, Some(payload)) = (request.mode, request.payload.as_ref()) {
            let stored_name = payload
                .stored_name
                .as_deref()
                .ok_or_else(|| invalid("copy-mode payloads require a stored filename"))?;
            write_new_file(&stage.path().join(stored_name), payload)?;
        }
        write_new_metadata(&stage.path().join("meta.toml"), &meta)?;

        let scripts = self.scripts_dir();
        create_dir_all(&scripts, "create")?;
        let destination = scripts.join(slug.as_str());
        self.remove_empty_destination(&destination)?;
        fs::rename(stage.path(), &destination)
            .map_err(|error| io_error("commit", &destination, error))?;
        let entry = Entry { slug, meta };
        let projection = registry
            .project(&entry, &destination)
            .and_then(|()| registry.save());
        if let Err(error) = projection {
            let rollback = fs::remove_dir_all(&destination)
                .map_err(|rollback| io_error("rollback create", &destination, rollback));
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
        let entry_dir = self.entry_dir(&after.slug);
        let projection = registry
            .project(after, &entry_dir)
            .and_then(|()| registry.save());
        if let Err(error) = projection {
            return Err(rollback_error(
                error,
                self.write_meta(before),
                &self.entry_dir(&before.slug),
            ));
        }
        Ok(())
    }

    fn ensure_name_available(
        &self,
        name: &str,
        excluded: Option<&Slug>,
        registry: &Registry,
    ) -> Result<(), RepositoryError> {
        if let Some(slug) = registry.name_owner(name, excluded) {
            return Err(RepositoryError::Conflict {
                name: name.to_owned(),
                slug,
            });
        }
        for existing in self.scan_entries()? {
            if existing.meta.name == name && excluded != Some(&existing.slug) {
                return Err(conflict(name, &existing.slug));
            }
        }
        Ok(())
    }

    fn allocate_slug(
        &self,
        base: Slug,
        excluded: Option<&Slug>,
        registry: &Registry,
    ) -> Result<Slug, RepositoryError> {
        if !registry.slug_is_taken(&base, excluded) && !self.slug_path_is_taken(&base, excluded)? {
            return Ok(base);
        }

        let mut suffix = 2_u64;
        loop {
            let candidate = Slug::parse(format!("{}-{suffix}", base.as_str()))
                .map_err(|error| invalid(error.to_string()))?;
            if !registry.slug_is_taken(&candidate, excluded)
                && !self.slug_path_is_taken(&candidate, excluded)?
            {
                return Ok(candidate);
            }
            suffix = suffix
                .checked_add(1)
                .ok_or_else(|| invalid("entry slug suffix space is exhausted"))?;
        }
    }

    fn slug_path_is_taken(
        &self,
        slug: &Slug,
        excluded: Option<&Slug>,
    ) -> Result<bool, RepositoryError> {
        if excluded == Some(slug) {
            return Ok(false);
        }
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
        if let Some(candidate) = conventional_stored_name(entry.meta.kind.as_str()) {
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
            if item.file_name().to_string_lossy() == "meta.toml" {
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
            [] => Err(invalid("copy entry has no stored payload")),
            _ => Err(invalid(
                "copy entry has more than one possible stored payload",
            )),
        }
    }

    fn write_meta(&self, entry: &Entry) -> Result<(), RepositoryError> {
        let path = self.entry_dir(&entry.slug).join("meta.toml");
        let text = toml::to_string_pretty(&entry.meta)
            .map_err(|error| invalid(format!("could not encode metadata: {error}")))?;
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
}

fn validated_name(name: &str) -> Result<String, RepositoryError> {
    let name = name.trim();
    if name.is_empty() {
        Err(invalid("entry name cannot be blank"))
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
        return Err(invalid("copy-mode payloads require a stored filename"));
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
        return Err(invalid("stored filename must be one safe path component"));
    }
    Ok(())
}

fn conventional_stored_name(kind: &str) -> Option<&'static str> {
    match kind {
        "python" => Some("script.py"),
        "shell" => Some("script.sh"),
        "js" => Some("script.js"),
        "ts" => Some("script.ts"),
        "fish" => Some("script.fish"),
        "powershell" => Some("script.ps1"),
        "ruby" => Some("script.rb"),
        "perl" => Some("script.pl"),
        "lua" => Some("script.lua"),
        "r" => Some("script.R"),
        _ => None,
    }
}

fn conflict(name: &str, slug: &Slug) -> RepositoryError {
    RepositoryError::Conflict {
        name: name.to_owned(),
        slug: slug.as_str().to_owned(),
    }
}

fn stale(slug: &Slug) -> RepositoryError {
    RepositoryError::StaleEntry {
        slug: slug.as_str().to_owned(),
    }
}

fn rollback_error(
    primary: RepositoryError,
    rollback: Result<(), RepositoryError>,
    path: &Path,
) -> RepositoryError {
    match rollback {
        Ok(()) => primary,
        Err(rollback) => RepositoryError::Io {
            operation: "rollback",
            path: path.display().to_string(),
            reason: format!("{primary}; rollback also failed: {rollback}"),
        },
    }
}
