use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;
use skit_application::RepositoryError;
use skit_domain::{Entry, EntryKind, EntrySummary, Slug, StorageMode};
use skit_i18n::Message;
use toml::{Table, Value};

use super::{
    atomic::{atomic_write_bytes, io_error},
    hash::content_hash,
};

const CACHE_SCHEMA: i64 = 1;
const MANAGED_ROW_KEYS: &[&str] = &[
    "name",
    "kind",
    "mode",
    "description",
    "mtime_ns",
    "target",
    "skit_cache",
];

/// The Python-compatible, rebuildable `registry.toml` projection.
#[derive(Clone, Debug)]
pub(crate) struct Registry {
    path: PathBuf,
    document: Table,
}

impl Registry {
    /// Start an empty projection for an explicit doctor rebuild.
    pub(super) fn fresh(data_dir: &Path) -> Self {
        let mut document = Table::new();
        normalize_entries(&mut document);
        Self {
            path: data_dir.join("registry.toml"),
            document,
        }
    }

    /// Read the current projection, moving unparseable bytes aside before degrading to empty.
    ///
    /// The chokepoint both `scan` and `resolve` load through, mirroring the oracle's single
    /// `_load_registry`. `registry.toml` is a rebuildable index, so a corrupt or unreadable one
    /// degrades the same way a missing file does: an empty registry that `doctor --rebuild` can
    /// reconstruct from the untouched `scripts/<slug>` metas. The bad bytes are preserved, not
    /// discarded -- renamed to `registry.toml.corrupt` so a corrupt file cannot keep re-triggering
    /// this branch (and spawning a fresh backup) on every later read before the next write. This is
    /// the read-path translation of `skit.store._load_registry`'s `TOMLDecodeError`/`OSError` branch
    /// (store.py ~139-147); the writer's `load` keeps its own stricter backup contract.
    pub(crate) fn read(data_dir: &Path) -> Option<Self> {
        let path = data_dir.join("registry.toml");
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            // A missing index reads as no registry, exactly as the oracle's `if not path.exists()`
            // early return: doctor rebuilds it, and there is nothing to move aside.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            // Any other read failure degrades like a corrupt parse: preserve the bytes, read empty.
            Err(_) => {
                back_up_corrupt_index_best_effort(&path);
                return None;
            }
        };
        match toml::from_str::<Table>(&text) {
            Ok(mut document) => {
                normalize_entries(&mut document);
                Some(Self { path, document })
            }
            Err(_) => {
                back_up_corrupt_index_best_effort(&path);
                None
            }
        }
    }

    /// Load the current projection for a writer, backing up corrupt bytes before starting fresh.
    pub(super) fn load(data_dir: &Path) -> Result<Self, RepositoryError> {
        let path = data_dir.join("registry.toml");
        let mut document = match fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<Table>(&text) {
                Ok(document) => document,
                Err(_) => {
                    backup_corrupt(&path)?;
                    Table::new()
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Table::new(),
            Err(_) => {
                backup_corrupt(&path)?;
                Table::new()
            }
        };
        normalize_entries(&mut document);
        Ok(Self { path, document })
    }

    /// Return a listing projection only when it is bound to the current metadata incarnation.
    ///
    /// Rust rows use an incarnation proof. Python rows use the exact `mtime_ns` contract from
    /// latest v0.4 main. This keeps upgraded libraries on the one-index-read fast path.
    pub(crate) fn summary(&self, slug: &Slug, meta_path: &Path) -> Option<EntrySummary> {
        let row = self.entries().get(slug.as_str())?.as_table()?;
        let projection = CachedProjection::parse(slug, row)?;
        projection.verify(meta_path).then_some(projection.summary)
    }

    /// Check whether one row represents an already-read authoritative metadata snapshot.
    pub(crate) fn matches_entry_snapshot(
        &self,
        entry: &Entry,
        meta_path: &Path,
        metadata_bytes: &[u8],
    ) -> bool {
        self.entries()
            .get(entry.slug.as_str())
            .and_then(Value::as_table)
            .and_then(|row| CachedProjection::parse(&entry.slug, row))
            .is_some_and(|projection| {
                projection.matches_entry_snapshot(entry, meta_path, metadata_bytes)
            })
    }

    /// Return the registry row keys that define library membership.
    pub(crate) fn row_keys(&self) -> Vec<String> {
        let mut keys = self.entries().keys().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    /// Return whether the index currently claims one canonical slug.
    pub(crate) fn contains(&self, slug: &Slug) -> bool {
        self.entries().contains_key(slug.as_str())
    }

    /// Return canonical slugs whose rows claim this exact display name.
    pub(crate) fn name_claimants(&self, name: &str) -> Vec<Slug> {
        let mut claimants = self
            .entries()
            .iter()
            .filter_map(|(slug, value)| {
                let claimed_name = value
                    .as_table()
                    .and_then(|row| row.get("name"))
                    .and_then(Value::as_str);
                if claimed_name != Some(name) {
                    return None;
                }
                Slug::parse(slug.clone()).ok()
            })
            .collect::<Vec<_>>();
        claimants.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        claimants
    }

    /// Return the slug of a row that already claims `name`, excluding one held entry.
    pub(super) fn name_owner(&self, name: &str, excluded: Option<&Slug>) -> Option<String> {
        if self.entries().contains_key(name)
            && excluded.is_none_or(|excluded| excluded.as_str() != name)
        {
            return Some(name.to_owned());
        }
        self.entries().iter().find_map(|(slug, value)| {
            let belongs_to_other_entry = excluded.is_none_or(|excluded| excluded.as_str() != slug);
            let same_name = value
                .as_table()
                .and_then(|row| row.get("name"))
                .and_then(Value::as_str)
                == Some(name);
            (belongs_to_other_entry && same_name).then(|| slug.clone())
        })
    }

    /// Whether an index row already claims this slug.
    pub(super) fn slug_is_taken(&self, slug: &Slug) -> bool {
        self.entries().contains_key(slug.as_str())
    }

    /// Replace one row with a projection stamped from the current metadata file.
    pub(super) fn project(
        &mut self,
        entry: &Entry,
        entry_dir: &Path,
    ) -> Result<(), RepositoryError> {
        let meta_path = entry_dir.join("meta.toml");
        let mtime_ns = metadata_mtime_ns(&meta_path)?;
        let proof = CacheProof::project(entry, &meta_path, mtime_ns);
        self.project_with_proof(entry, mtime_ns, proof.as_ref());
        Ok(())
    }

    /// Refresh one existing row without inventing library membership.
    ///
    /// Normal metadata writers use this path. Only create and doctor can add a row.
    pub(super) fn project_existing(
        &mut self,
        entry: &Entry,
        entry_dir: &Path,
    ) -> Result<bool, RepositoryError> {
        if !self.contains(&entry.slug) {
            return Ok(false);
        }
        self.project(entry, entry_dir)?;
        Ok(true)
    }

    /// Delete one row.
    pub(super) fn remove(&mut self, slug: &Slug) {
        self.entries_mut().remove(slug.as_str());
    }

    /// Re-derive one stale row from its meta as it is NOW, replacing it only if it changed.
    ///
    /// The per-slug body of the read-path self-heal (`FileStore::repair_rows`), translating the loop
    /// in `skit.store._repair_rows` (store.py ~1043-1059): re-read the meta under the lock, project a
    /// fresh row, and report whether it differs from the stored one so the caller saves only on a
    /// real change ("nothing is saved unless something actually changed"). Re-deriving from the meta
    /// -- never from the listing's snapshot -- is what makes the newest committed state win no matter
    /// who wrote it. Returns `Err` only when the meta could not be stat'd for its stamp (it vanished
    /// between the caller's read and here), and leaves the row untouched in that case; the caller
    /// skips such a slug.
    pub(crate) fn reproject_if_changed(
        &mut self,
        entry: &Entry,
        entry_dir: &Path,
    ) -> Result<bool, RepositoryError> {
        let before = self.entries().get(entry.slug.as_str()).cloned();
        self.project(entry, entry_dir)?;
        Ok(before.as_ref() != self.entries().get(entry.slug.as_str()))
    }

    /// Persist the whole projection through the same atomic replacement discipline as metadata.
    pub(crate) fn save(&self) -> Result<(), RepositoryError> {
        let text = toml::to_string_pretty(&self.document)
            .expect("a normalized TOML value tree must serialize");
        atomic_write_bytes(&self.path, text.as_bytes())
    }

    fn project_with_proof(&mut self, entry: &Entry, mtime_ns: i64, proof: Option<&CacheProof>) {
        let existing = self
            .entries()
            .get(entry.slug.as_str())
            .and_then(Value::as_table)
            .cloned()
            .unwrap_or_default();
        let row = merge_row(existing, row_for(entry, mtime_ns, proof));
        self.entries_mut()
            .insert(entry.slug.as_str().to_owned(), Value::Table(row));
    }

    fn entries(&self) -> &Table {
        self.document
            .get("entries")
            .and_then(Value::as_table)
            .expect("Registry constructors normalize entries to a table")
    }

    fn entries_mut(&mut self) -> &mut Table {
        self.document
            .get_mut("entries")
            .and_then(Value::as_table_mut)
            .expect("Registry constructors normalize entries to a table")
    }
}

fn normalize_entries(document: &mut Table) {
    if !matches!(document.get("entries"), Some(Value::Table(_))) {
        document.insert("entries".to_owned(), Value::Table(Table::new()));
    }
}

fn row_for(entry: &Entry, mtime_ns: i64, proof: Option<&CacheProof>) -> Table {
    let mut row = Table::new();
    row.insert("name".to_owned(), Value::String(entry.meta.name.clone()));
    row.insert(
        "kind".to_owned(),
        Value::String(entry.meta.kind.as_str().to_owned()),
    );
    row.insert(
        "mode".to_owned(),
        Value::String(
            match entry.meta.mode {
                StorageMode::Copy => "copy",
                StorageMode::Reference => "reference",
            }
            .to_owned(),
        ),
    );
    row.insert(
        "description".to_owned(),
        Value::String(entry.meta.description.clone()),
    );
    row.insert("mtime_ns".to_owned(), Value::Integer(mtime_ns));
    if entry.meta.mode == StorageMode::Reference {
        row.insert(
            "target".to_owned(),
            Value::String(entry.meta.source.clone()),
        );
    }
    if let Some(proof) = proof {
        row.insert("skit_cache".to_owned(), Value::Table(proof.to_table()));
    }
    row
}

fn merge_row(mut existing: Table, replacement: Table) -> Table {
    for key in MANAGED_ROW_KEYS {
        existing.remove(*key);
    }
    existing.extend(replacement);
    existing
}

/// Identify one metadata file incarnation with values available from one file-system query.
///
/// The file ID rejects replacements. The change time rejects in-place edits that restore the
/// modification time and file size. The registry modification time keeps the Python row stamp
/// coherent. Unix uses change time. Windows verifies the stored content hash because stable Rust
/// does not expose the change counter. Other targets do not use the shortcut.
#[derive(Clone, Debug, Eq, PartialEq)]
struct MetadataFingerprint {
    platform: &'static str,
    file_id: String,
    file_size: u64,
    registry_mtime_ns: i64,
    modified_ns: i128,
    changed_ns: i128,
}

impl MetadataFingerprint {
    #[cfg(unix)]
    fn read(path: &Path) -> Option<Self> {
        use std::os::unix::fs::MetadataExt as _;

        let metadata = fs::metadata(path).ok()?;
        if !metadata.is_file() {
            return None;
        }
        Some(Self {
            platform: "unix",
            file_id: format!("{}:{}", metadata.dev(), metadata.ino()),
            file_size: metadata.len(),
            registry_mtime_ns: timestamp_ns(metadata.modified().ok()?).ok()?,
            modified_ns: seconds_and_nanos(metadata.mtime(), metadata.mtime_nsec()),
            changed_ns: seconds_and_nanos(metadata.ctime(), metadata.ctime_nsec()),
        })
    }

    #[cfg(windows)]
    fn read(path: &Path) -> Option<Self> {
        use std::os::windows::fs::MetadataExt as _;

        let file = fs::File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        if !metadata.is_file() {
            return None;
        }
        let id = fs_id::FileID::new(&file).ok()?;
        Some(Self {
            platform: "windows",
            file_id: format!("{}:{}", id.storage_id(), id.internal_file_id()),
            file_size: metadata.len(),
            registry_mtime_ns: timestamp_ns(metadata.modified().ok()?).ok()?,
            // Windows file times count 100-nanosecond intervals.
            modified_ns: i128::from(metadata.last_write_time()) * 100,
            // Stable Rust does not expose the Windows change counter. Cache verification also
            // compares the authoritative content hash on Windows, so creation time is sufficient
            // here to reject file-ID reuse without weakening edit detection.
            changed_ns: i128::from(metadata.creation_time()) * 100,
        })
    }

    // The portable metadata API has no file identity or change counter. Unsupported targets take
    // the authoritative read path because size and restored modification time are not proof.
    #[cfg(not(any(unix, windows)))]
    fn read(_path: &Path) -> Option<Self> {
        None
    }

    #[cfg(any(unix, windows))]
    fn from_table(table: &Table, registry_mtime_ns: i64) -> Option<Self> {
        let platform = table.get("platform")?.as_str()?;
        (platform == current_cache_platform()).then_some(())?;
        let file_id = table.get("file_id")?.as_str()?.to_owned();
        (!file_id.is_empty()).then_some(())?;
        let file_size = parse_canonical(table.get("file_size")?.as_str()?)?;
        let modified_ns = parse_canonical(table.get("modified_ns")?.as_str()?)?;
        let changed_ns = parse_canonical(table.get("changed_ns")?.as_str()?)?;
        Some(Self {
            platform: current_cache_platform(),
            file_id,
            file_size,
            registry_mtime_ns,
            modified_ns,
            changed_ns,
        })
    }

    #[cfg(not(any(unix, windows)))]
    fn from_table(_table: &Table, _registry_mtime_ns: i64) -> Option<Self> {
        None
    }

    fn write_to(&self, table: &mut Table) {
        table.insert(
            "platform".to_owned(),
            Value::String(self.platform.to_owned()),
        );
        table.insert("file_id".to_owned(), Value::String(self.file_id.clone()));
        table.insert(
            "file_size".to_owned(),
            Value::String(self.file_size.to_string()),
        );
        table.insert(
            "modified_ns".to_owned(),
            Value::String(self.modified_ns.to_string()),
        );
        table.insert(
            "changed_ns".to_owned(),
            Value::String(self.changed_ns.to_string()),
        );
    }
}

#[cfg(unix)]
const fn current_cache_platform() -> &'static str {
    "unix"
}

#[cfg(windows)]
const fn current_cache_platform() -> &'static str {
    "windows"
}

fn parse_canonical<T>(value: &str) -> Option<T>
where
    T: std::str::FromStr + ToString,
{
    let parsed = value.parse::<T>().ok()?;
    (parsed.to_string() == value).then_some(parsed)
}

fn seconds_and_nanos(seconds: i64, nanos: i64) -> i128 {
    i128::from(seconds) * 1_000_000_000 + i128::from(nanos)
}

/// Bind one listing projection to the exact metadata snapshot that produced it.
///
/// The metadata hash names the byte snapshot. The projection hash covers the slug, every listing
/// field, the compatibility stamp, the metadata hash, and the file fingerprint. This is an
/// integrity check for a rebuildable local cache. It is not a security boundary.
#[derive(Clone, Debug)]
struct CacheProof {
    fingerprint: MetadataFingerprint,
    metadata_hash: String,
    projection_hash: String,
}

impl CacheProof {
    fn project(entry: &Entry, meta_path: &Path, mtime_ns: i64) -> Option<Self> {
        let before = MetadataFingerprint::read(meta_path)?;
        let bytes = fs::read(meta_path).ok()?;
        let after = MetadataFingerprint::read(meta_path)?;
        (before == after
            && before.registry_mtime_ns == mtime_ns
            && ProjectedMetadata::matches_entry(&bytes, entry))
        .then_some(())?;
        let metadata_hash = content_hash(&bytes);
        let summary = summary_from_entry(entry);
        let projection_hash =
            projection_hash(&entry.slug, &summary, mtime_ns, &before, &metadata_hash);
        Some(Self {
            fingerprint: before,
            metadata_hash,
            projection_hash,
        })
    }

    fn parse(table: &Table, mtime_ns: i64) -> Option<Self> {
        (table.get("schema")?.as_integer()? == CACHE_SCHEMA).then_some(())?;
        let fingerprint = MetadataFingerprint::from_table(table, mtime_ns)?;
        let metadata_hash = table.get("metadata_hash")?.as_str()?.to_owned();
        let projection_hash = table.get("projection_hash")?.as_str()?.to_owned();
        (is_sha256(&metadata_hash) && is_sha256(&projection_hash)).then_some(())?;
        Some(Self {
            fingerprint,
            metadata_hash,
            projection_hash,
        })
    }

    fn to_table(&self) -> Table {
        let mut table = Table::new();
        table.insert("schema".to_owned(), Value::Integer(CACHE_SCHEMA));
        self.fingerprint.write_to(&mut table);
        table.insert(
            "metadata_hash".to_owned(),
            Value::String(self.metadata_hash.clone()),
        );
        table.insert(
            "projection_hash".to_owned(),
            Value::String(self.projection_hash.clone()),
        );
        table
    }
}

/// Hold one fully parsed cache row before its live file identity is checked.
#[derive(Debug)]
struct CachedProjection {
    slug: Slug,
    summary: EntrySummary,
    mtime_ns: i64,
    proof: Option<CacheProof>,
}

impl CachedProjection {
    fn parse(slug: &Slug, row: &Table) -> Option<Self> {
        let name = row.get("name")?.as_str()?.to_owned();
        let kind = EntryKind::parse(row.get("kind")?.as_str()?.to_owned()).ok()?;
        let description = row.get("description")?.as_str()?.to_owned();
        let mode = match row.get("mode")?.as_str()? {
            "copy" => StorageMode::Copy,
            "reference" => StorageMode::Reference,
            _ => return None,
        };
        let target = match mode {
            StorageMode::Copy => {
                (!row.get("target").is_some_and(|value| !value.is_str())).then_some(())?;
                None
            }
            StorageMode::Reference => {
                let target = row.get("target")?.as_str()?.to_owned();
                if target.is_empty() && kind.as_str() != "command" {
                    return None;
                }
                Some(target)
            }
        };
        let mtime_ns = row.get("mtime_ns")?.as_integer()?;
        let proof = match row.get("skit_cache") {
            Some(value) => Some(CacheProof::parse(value.as_table()?, mtime_ns)?),
            None => None,
        };
        Some(Self {
            slug: slug.clone(),
            summary: EntrySummary {
                slug: slug.clone(),
                name,
                kind,
                mode,
                description,
                target,
            },
            mtime_ns,
            proof,
        })
    }

    fn verify(&self, meta_path: &Path) -> bool {
        let Some(proof) = &self.proof else {
            return legacy_metadata_mtime_ns(meta_path) == Some(self.mtime_ns);
        };
        let expected = projection_hash(
            &self.slug,
            &self.summary,
            self.mtime_ns,
            &proof.fingerprint,
            &proof.metadata_hash,
        );
        expected == proof.projection_hash
            && MetadataFingerprint::read(meta_path).as_ref() == Some(&proof.fingerprint)
            && metadata_hash_matches(meta_path, &proof.metadata_hash)
    }

    fn matches_entry_snapshot(
        &self,
        entry: &Entry,
        meta_path: &Path,
        metadata_bytes: &[u8],
    ) -> bool {
        if self.summary != summary_from_entry(entry) {
            return false;
        }
        let Some(proof) = &self.proof else {
            return legacy_metadata_mtime_ns(meta_path) == Some(self.mtime_ns);
        };
        let expected = projection_hash(
            &self.slug,
            &self.summary,
            self.mtime_ns,
            &proof.fingerprint,
            &proof.metadata_hash,
        );
        expected == proof.projection_hash
            && MetadataFingerprint::read(meta_path).as_ref() == Some(&proof.fingerprint)
            && content_hash(metadata_bytes) == proof.metadata_hash
    }
}

#[cfg(unix)]
fn metadata_hash_matches(_path: &Path, _expected: &str) -> bool {
    true
}

#[cfg(windows)]
fn metadata_hash_matches(path: &Path, expected: &str) -> bool {
    fs::read(path).is_ok_and(|bytes| content_hash(&bytes) == expected)
}

#[cfg(not(any(unix, windows)))]
fn metadata_hash_matches(_path: &Path, _expected: &str) -> bool {
    false
}

fn legacy_metadata_mtime_ns(path: &Path) -> Option<i64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    timestamp_ns(modified).ok()
}

#[derive(Debug, Deserialize)]
struct ProjectedMetadata {
    name: String,
    kind: String,
    #[serde(default)]
    mode: StorageMode,
    #[serde(default)]
    source: String,
    #[serde(default)]
    description: String,
}

impl ProjectedMetadata {
    fn matches_entry(bytes: &[u8], entry: &Entry) -> bool {
        std::str::from_utf8(bytes)
            .ok()
            .and_then(|text| toml::from_str::<Self>(text).ok())
            .is_some_and(|meta| {
                meta.name == entry.meta.name
                    && meta.kind == entry.meta.kind.as_str()
                    && meta.mode == entry.meta.mode
                    && meta.description == entry.meta.description
                    && (meta.mode == StorageMode::Copy || meta.source == entry.meta.source)
            })
    }
}

fn summary_from_entry(entry: &Entry) -> EntrySummary {
    EntrySummary {
        slug: entry.slug.clone(),
        name: entry.meta.name.clone(),
        kind: entry.meta.kind.clone(),
        mode: entry.meta.mode,
        description: entry.meta.description.clone(),
        target: (entry.meta.mode == StorageMode::Reference).then(|| entry.meta.source.clone()),
    }
}

fn projection_hash(
    slug: &Slug,
    summary: &EntrySummary,
    mtime_ns: i64,
    fingerprint: &MetadataFingerprint,
    metadata_hash: &str,
) -> String {
    let mut bytes = Vec::new();
    push_field(&mut bytes, "skit-registry-cache-v1");
    push_field(&mut bytes, slug.as_str());
    push_field(&mut bytes, &summary.name);
    push_field(&mut bytes, summary.kind.as_str());
    push_field(
        &mut bytes,
        match summary.mode {
            StorageMode::Copy => "copy",
            StorageMode::Reference => "reference",
        },
    );
    push_field(&mut bytes, &summary.description);
    match summary.target.as_deref() {
        Some(target) => {
            bytes.push(1);
            push_field(&mut bytes, target);
        }
        None => bytes.push(0),
    }
    push_field(&mut bytes, &mtime_ns.to_string());
    push_field(&mut bytes, fingerprint.platform);
    push_field(&mut bytes, &fingerprint.file_id);
    push_field(&mut bytes, &fingerprint.file_size.to_string());
    push_field(&mut bytes, &fingerprint.modified_ns.to_string());
    push_field(&mut bytes, &fingerprint.changed_ns.to_string());
    push_field(&mut bytes, metadata_hash);
    content_hash(&bytes)
}

fn push_field(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn is_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(crate) fn metadata_mtime_ns(path: &Path) -> Result<i64, RepositoryError> {
    let modified = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map_err(|error| io_error("inspect", path, error))?;
    // TOML integers cannot represent every timestamp that a restored file system can expose.
    // Keep the membership row with a stale sentinel in that rare case. Readers then use the
    // authoritative metadata path, and doctor never hides the entry merely because it cannot
    // accelerate it.
    Ok(timestamp_ns(modified).unwrap_or(0))
}

fn timestamp_ns(modified: SystemTime) -> Result<i64, RepositoryError> {
    let nanos =
        modified
            .duration_since(UNIX_EPOCH)
            .map_err(|error| RepositoryError::InvalidMutation {
                reason: Message::new("metadata timestamp predates the Unix epoch: {}").with(error),
            })?;
    i64::try_from(nanos.as_nanos()).map_err(|error| RepositoryError::InvalidMutation {
        reason: Message::new("metadata timestamp does not fit registry.toml: {}").with(error),
    })
}

/// Move an unparseable index aside to `registry.toml.corrupt`, preserving the bad bytes.
///
/// Best-effort by contract: a READ must never fail because the backup could not be made, so every
/// error is swallowed (the oracle wraps the same rename in `contextlib.suppress(OSError)`). Rename,
/// not copy, so the corrupt file cannot re-trigger a fresh backup on every later read. On Linux
/// `fs::rename` overwrites an existing `.corrupt` file just as the oracle's `os.replace` does; when
/// `.corrupt` is a directory the rename fails and the corrupt original simply stays in place.
fn back_up_corrupt_index_best_effort(path: &Path) {
    let backup = path.with_file_name(format!(
        "{}.corrupt",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("registry.toml")
    ));
    let _ = fs::rename(path, &backup);
}

fn backup_corrupt(path: &Path) -> Result<(), RepositoryError> {
    let backup = path.with_file_name(format!(
        "{}.corrupt",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("registry.toml")
    ));
    if backup.is_file() {
        fs::remove_file(&backup).map_err(|error| io_error("replace backup", &backup, error))?;
    } else if backup.exists() {
        return Err(RepositoryError::Io {
            operation: "backup",
            path: backup.display().to_string(),
            reason: "the backup path exists and is not a regular file".to_owned(),
        });
    }
    fs::rename(path, &backup).map_err(|error| io_error("backup", path, error))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    #[test]
    fn registry_timestamps_refuse_pre_epoch_and_oversized_values() {
        assert!(timestamp_ns(UNIX_EPOCH - Duration::from_nanos(1)).is_err());
        let oversized = SystemTime::UNIX_EPOCH
            + Duration::from_secs(u64::try_from(i64::MAX).unwrap() / 1_000_000_000 + 1);
        assert!(timestamp_ns(oversized).is_err());
        assert_eq!(
            timestamp_ns(UNIX_EPOCH + Duration::from_nanos(7)).unwrap(),
            7
        );
    }
}
