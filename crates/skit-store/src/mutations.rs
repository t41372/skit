mod atomic;
mod hash;

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
use skit_application::{CreateEntry, EntryMutationRepository, EntryPayload, RepositoryError};
use skit_domain::{Entry, EntryId, EntryMeta, Slug, StorageMode};

use super::FileStore;

impl EntryMutationRepository for FileStore {
    fn create(&self, request: CreateEntry) -> Result<Entry, RepositoryError> {
        let _namespace = self.namespace_lock()?;
        self.create_locked(request)
    }

    fn claim_identity(&self, entry: &Entry) -> Result<Entry, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        self.claim_locked(entry)
    }

    fn describe(&self, entry: &Entry, description: &str) -> Result<Entry, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let mut fresh = self.claim_locked(entry)?;
        fresh.meta.description = description.to_owned();
        self.write_meta(&fresh)?;
        Ok(fresh)
    }

    fn rename(&self, entry: &Entry, name: &str) -> Result<Entry, RepositoryError> {
        let name = validated_name(name)?;
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let mut fresh = self.claim_locked(entry)?;
        self.ensure_name_available(&name, Some(&fresh.slug))?;
        let new_slug = self.allocate_slug(Slug::from_display_name(&name), Some(&fresh.slug))?;

        let old_slug = fresh.slug.clone();
        let old_meta = fresh.meta.clone();
        fresh.meta.name = name;
        if old_slug == new_slug {
            self.write_meta(&fresh)?;
            return Ok(fresh);
        }

        let old_dir = self.entry_dir(&old_slug);
        let new_dir = self.entry_dir(&new_slug);
        self.write_meta(&fresh)?;
        if let Err(error) = fs::rename(&old_dir, &new_dir) {
            let rollback = Entry {
                slug: old_slug,
                meta: old_meta,
            };
            let _ = self.write_meta(&rollback);
            return Err(io_error("rename", &old_dir, error));
        }
        fresh.slug = new_slug;
        Ok(fresh)
    }

    fn remove(&self, entry: &Entry) -> Result<String, RepositoryError> {
        let _entry = self.entry_lock(&entry.slug)?;
        let _namespace = self.namespace_lock()?;
        let fresh = self.claim_locked(entry)?;
        let name = fresh.meta.name.clone();
        let source = self.entry_dir(&fresh.slug);
        let trash_root = self.data_dir().join(".trash");
        create_dir_all(&trash_root, "create")?;
        let trash = trash_root.join(format!(
            "{}-{}",
            fresh.slug,
            EntryId::generate().as_str()
        ));
        fs::rename(&source, &trash).map_err(|error| io_error("remove", &source, error))?;
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
        let mut fresh = self.claim_locked(entry)?;
        if fresh.meta.mode != StorageMode::Copy {
            return Err(invalid("reference entries are edited at their original path"));
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

        atomic_write_bytes(&target, bytes)?;
        fresh.meta.source_hash = content_hash(bytes);
        if let Err(error) = self.write_meta(&fresh) {
            let _ = atomic_write_bytes(&target, &original);
            return Err(error);
        }
        Ok(fresh)
    }
}

impl FileStore {
    fn create_locked(&self, mut request: CreateEntry) -> Result<Entry, RepositoryError> {
        request.name = validated_name(&request.name)?;
        self.ensure_name_available(&request.name, None)?;
        validate_payload(request.mode, request.payload.as_ref())?;
        let slug = self.allocate_slug(Slug::from_display_name(&request.name), None)?;
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
        fs::rename(stage.path(), &destination)
            .map_err(|error| io_error("commit", &destination, error))?;
        stage.commit();
        Ok(Entry { slug, meta })
    }

    fn claim_locked(&self, held: &Entry) -> Result<Entry, RepositoryError> {
        let directory = self.entry_dir(&held.slug);
        if !directory.is_dir() {
            return Err(stale(&held.slug));
        }
        let mut fresh = self.read_entry(held.slug.clone())?;
        match held.meta.id.as_ref() {
            Some(expected) if fresh.meta.id.as_ref() == Some(expected) => Ok(fresh),
            Some(_) => Err(stale(&held.slug)),
            None if fresh.meta.id.is_none() && fresh.meta == held.meta => {
                fresh.meta.id = Some(EntryId::generate());
                self.write_meta(&fresh)?;
                Ok(fresh)
            }
            None => Err(stale(&held.slug)),
        }
    }

    fn ensure_name_available(
        &self,
        name: &str,
        excluded: Option<&Slug>,
    ) -> Result<(), RepositoryError> {
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
    ) -> Result<Slug, RepositoryError> {
        if excluded == Some(&base) || !self.entry_dir(&base).exists() {
            return Ok(base);
        }

        let mut suffix = 2_u64;
        loop {
            let candidate = Slug::parse(format!("{}-{suffix}", base.as_str()))
                .map_err(|error| invalid(error.to_string()))?;
            if excluded == Some(&candidate) || !self.entry_dir(&candidate).exists() {
                return Ok(candidate);
            }
            suffix = suffix
                .checked_add(1)
                .ok_or_else(|| invalid("entry slug suffix space is exhausted"))?;
        }
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
        let reader = fs::read_dir(&directory)
            .map_err(|error| io_error("scan", &directory, error))?;
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
