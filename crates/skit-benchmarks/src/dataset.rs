//! Deterministic benchmark libraries generated through product persistence APIs.

use std::{
    collections::BTreeMap,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use skit_application::form_state::FormStateService;
use skit_application::{CreateEntry, EntryPayload, LibraryService, SourcePermissions};
use skit_domain::parameters::{ParamDecl, ParameterType};
use skit_domain::{Entry, EntryKind, EntrySettings, Slug, StorageMode};
use skit_store::{FileFormStateStore, FileStore, stored_filename};
use thiserror::Error;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::python_random::PythonRandom;

/// Dataset schema and input discontinuity marker.
pub const GENERATOR_VERSION: u32 = 1;
/// Default reproducibility seed.
pub const DEFAULT_SEED: u64 = 20_260_720;
/// Fraction of entries with remembered state.
pub const DEFAULT_STATE_FRACTION: f64 = 0.5;
/// Search probe character inherited from latest Python main.
pub const SEARCH_PROBE_CHAR: &str = "o";
/// Every tenth reference entry has a deliberately missing target.
pub const MISSING_TARGET_STRIDE: usize = 10;
/// Exact latest-main Python run-overhead subject.
pub const RUNOVER_PYTHON: &str = include_str!("../../../benchmarks/fixtures/noop.py");
/// Exact latest-main shell run-overhead subject.
pub const RUNOVER_SHELL: &str = include_str!("../../../benchmarks/fixtures/noop.sh");
/// Exact latest-main JavaScript run-overhead subject.
pub const RUNOVER_JAVASCRIPT: &str = include_str!("../../../benchmarks/fixtures/noop.js");

const KIND_MIX: &[(&str, usize)] = &[
    ("python", 30),
    ("shell", 20),
    ("js", 10),
    ("ts", 5),
    ("command", 10),
    ("prompt", 10),
    ("fish", 5),
    ("exe", 6),
    ("ruby", 1),
    ("perl", 1),
    ("lua", 1),
    ("r", 1),
];

const WORDS: &[&str] = &[
    "alpha", "bravo", "delta", "gamma", "kilo", "lima", "omega", "sigma",
];

/// Absolute product directories for one benchmark dataset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatasetDirs {
    /// Store root.
    pub data: PathBuf,
    /// Form-state root.
    pub state: PathBuf,
    /// Configuration root.
    pub config: PathBuf,
}

/// Reuse stamp and diagnostic description of a generated library.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DatasetManifest {
    /// Root is injected while loading and is not duplicated in JSON.
    #[serde(skip)]
    pub root: PathBuf,
    /// Entry count.
    pub n: usize,
    /// Generator seed.
    pub seed: u64,
    /// Remembered-state fraction.
    pub state_fraction: f64,
    /// Dataset format version.
    pub generator_version: u32,
    /// Product version that wrote the store.
    pub skit_version: String,
    /// Search probe input.
    pub probe_char: String,
    /// Entries in add order.
    pub slugs: Vec<Slug>,
    /// Diagnostic kind per slug.
    pub kinds: BTreeMap<String, String>,
}

impl DatasetManifest {
    /// Middle entry used by scale-show cases.
    pub fn middle_slug(&self) -> Result<&Slug, DatasetError> {
        self.slugs
            .get(self.slugs.len() / 2)
            .ok_or(DatasetError::EmptyMiddle)
    }

    /// Load a complete manifest and name the delete-and-rerun remedy on corruption.
    pub fn load(root: &Path) -> Result<Self, DatasetError> {
        let path = root.join("manifest.json");
        let text = fs::read_to_string(&path).map_err(|source| DatasetError::Io {
            operation: "read",
            path: path.clone(),
            source,
        })?;
        let mut manifest: Self =
            serde_json::from_str(&text).map_err(|source| DatasetError::UnreadableManifest {
                path: path.clone(),
                reason: source.to_string(),
            })?;
        manifest.root = absolute(root)?;
        validate_manifest(&manifest)?;
        Ok(manifest)
    }

    /// Stable JSON with a final newline.
    pub fn to_json(&self) -> Result<String, DatasetError> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

/// Dataset generation or reuse failed.
#[derive(Debug, Error)]
pub enum DatasetError {
    /// Invalid generation input.
    #[error("{0}")]
    Invalid(String),
    /// Existing user-owned content must not be overwritten.
    #[error("refusing to generate into non-empty {0}")]
    NonEmpty(PathBuf),
    /// A manifest is incomplete or corrupt.
    #[error("{path} is unreadable ({reason}) - delete {path:?}'s dataset directory and rerun")]
    UnreadableManifest {
        /// Manifest path.
        path: PathBuf,
        /// Parse or validation reason.
        reason: String,
    },
    /// A stored dataset does not match current inputs.
    #[error("dataset {root} was generated with different inputs - delete it and rerun")]
    ReuseMismatch {
        /// Dataset root.
        root: PathBuf,
    },
    /// An empty dataset has no show target.
    #[error("empty dataset has no middle entry")]
    EmptyMiddle,
    /// Product persistence rejected a generated entry.
    #[error("product store rejected generated data: {0}")]
    Product(String),
    /// Filesystem operation failed.
    #[error("could not {operation} {path}: {source}")]
    Io {
        /// Operation.
        operation: &'static str,
        /// Target.
        path: PathBuf,
        /// OS error.
        #[source]
        source: std::io::Error,
    },
    /// Manifest JSON failed.
    #[error("manifest JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    /// Timestamp formatting failed.
    #[error("timestamp formatting failed: {0}")]
    Time(#[from] time::error::Format),
}

/// Return absolute store/state/config paths for a dataset root.
pub fn dataset_dirs(root: &Path) -> Result<DatasetDirs, DatasetError> {
    let root = absolute(root)?;
    Ok(DatasetDirs {
        data: root.join("data"),
        state: root.join("state"),
        config: root.join("config"),
    })
}

/// Return one deterministically shuffled 100-slot kind grid.
#[must_use]
pub fn kind_slots(seed: u64, n: usize) -> Vec<String> {
    let mut rng = PythonRandom::seeded(&format!(
        "{seed}:{n}:{}",
        python_float(DEFAULT_STATE_FRACTION)
    ));
    shuffled_kind_slots(&mut rng)
}

fn shuffled_kind_slots(rng: &mut PythonRandom) -> Vec<String> {
    let mut slots = KIND_MIX
        .iter()
        .flat_map(|(kind, share)| std::iter::repeat_n((*kind).to_owned(), *share))
        .collect::<Vec<_>>();
    rng.shuffle(&mut slots);
    slots
}

/// Refuse to reuse a dataset when any load-bearing input changed.
pub fn check_reusable(manifest: &DatasetManifest, n: usize) -> Result<(), DatasetError> {
    let matches = manifest.n == n
        && manifest.seed == DEFAULT_SEED
        && manifest.state_fraction == DEFAULT_STATE_FRACTION
        && manifest.generator_version == GENERATOR_VERSION
        && manifest.skit_version == env!("CARGO_PKG_VERSION")
        && manifest.probe_char == SEARCH_PROBE_CHAR;
    if matches {
        Ok(())
    } else {
        Err(DatasetError::ReuseMismatch {
            root: manifest.root.clone(),
        })
    }
}

/// Generate a full deterministic product home through public application ports.
pub fn generate(
    root: &Path,
    n: isize,
    seed: u64,
    state_fraction: f64,
) -> Result<DatasetManifest, DatasetError> {
    if n < 0 {
        return Err(DatasetError::Invalid("n must be >= 0".to_owned()));
    }
    if !(0.0..=1.0).contains(&state_fraction) {
        return Err(DatasetError::Invalid(
            "state_fraction must be within [0, 1]".to_owned(),
        ));
    }
    require_empty(root)?;
    create_directory_tree(root)?;
    let root = absolute(root)?;
    let sources = root.join("srcfiles");
    create_directory(&sources)?;
    let n = usize::try_from(n).expect("non-negative isize fits usize");
    let dirs = dataset_dirs(&root)?;
    let library = LibraryService::new(FileStore::new(&dirs.data));
    let state = FormStateService::new(FileFormStateStore::new(&dirs.state));
    let mut rng = PythonRandom::seeded(&format!("{seed}:{n}:{}", python_float(state_fraction)));
    let slots = shuffled_kind_slots(&mut rng);
    let mut slugs = Vec::with_capacity(n);
    let mut kinds = BTreeMap::new();
    let mut references = Vec::new();
    let mut entries = Vec::with_capacity(n);
    for index in 0..n {
        let kind = &slots[index % slots.len()];
        let name = entry_name(index, &mut rng);
        let description = entry_description(index, &mut rng);
        let entry = add_entry(&library, &sources, index, kind, name, description)?;
        if kind == "exe" {
            references.push(PathBuf::from(&entry.meta.source));
        }
        kinds.insert(entry.slug.as_str().to_owned(), kind.clone());
        slugs.push(entry.slug.clone());
        entries.push(entry);
    }
    for (index, path) in references.iter().enumerate() {
        if index % MISSING_TARGET_STRIDE == 0 {
            remove_file(path)?;
        }
    }

    let base = OffsetDateTime::from_unix_timestamp(1_767_225_600)
        .expect("2026-01-01 UTC is representable");
    for (index, entry) in entries.iter().enumerate() {
        if rng.random() >= state_fraction {
            continue;
        }
        let declarations = &EntrySettings::from_meta(&entry.meta).parameters;
        let values = BTreeMap::from([("count".to_owned(), (index % 7).to_string())]);
        state
            .save_last(&entry.slug, declarations, Some(&values), None, false)
            .map_err(|error| DatasetError::Product(error.to_string()))?;
        let at = (base + Duration::hours(index as i64)).format(&Rfc3339)?;
        state
            .record_run(&entry.slug, 0, &at, declarations, Some(&values))
            .map_err(|error| DatasetError::Product(error.to_string()))?;
    }
    let found = library
        .list()
        .map_err(|error| DatasetError::Product(error.to_string()))?
        .entries
        .len();
    validate_generated_count(found, n)?;
    finalize(root, n, seed, state_fraction, slugs, kinds)
}

/// Generate the dedicated Python, shell, and JavaScript run-overhead library.
pub fn generate_runover(root: PathBuf) -> Result<DatasetManifest, DatasetError> {
    require_empty(&root)?;
    create_directory_tree(&root)?;
    let root = absolute(&root)?;
    let sources = root.join("srcfiles");
    create_directory(&sources)?;
    let dirs = dataset_dirs(&root)?;
    let library = LibraryService::new(FileStore::new(dirs.data));
    let lanes = [
        ("python", "noop-py", RUNOVER_PYTHON),
        ("shell", "noop-sh", RUNOVER_SHELL),
        ("js", "noop-js", RUNOVER_JAVASCRIPT),
    ];
    let mut slugs = Vec::new();
    let mut kinds = BTreeMap::new();
    for (index, (kind, name, body)) in lanes.into_iter().enumerate() {
        let entry = add_entry(
            &library,
            &sources,
            index,
            kind,
            name.to_owned(),
            String::new(),
        )?;
        let entry = library
            .commit_copy_edit(&entry, body.as_bytes(), &entry.meta.source_hash)
            .map_err(|error| DatasetError::Product(error.to_string()))?;
        kinds.insert(entry.slug.as_str().to_owned(), kind.to_owned());
        slugs.push(entry.slug);
    }
    finalize(root, 3, 0, 0.0, slugs, kinds)
}

/// Generate the command-only index-worst-case library used by Criterion and CodSpeed.
pub fn generate_command_only(root: PathBuf, n: usize) -> Result<DatasetManifest, DatasetError> {
    require_empty(&root)?;
    create_directory_tree(&root)?;
    let root = absolute(&root)?;
    let dirs = dataset_dirs(&root)?;
    let library = LibraryService::new(FileStore::new(dirs.data));
    let mut slugs = Vec::with_capacity(n);
    let mut kinds = BTreeMap::new();
    for index in 0..n {
        let entry = library
            .add(CreateEntry {
                name: format!("cmd-{index:04}"),
                kind: EntryKind::parse("command".to_owned())
                    .expect("the command kind is always valid"),
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description: String::new(),
                payload: None,
                settings: EntrySettings {
                    template: format!("echo {index} {{arg}}"),
                    ..EntrySettings::default()
                },
            })
            .map_err(|error| DatasetError::Product(error.to_string()))?;
        kinds.insert(entry.slug.as_str().to_owned(), "command".to_owned());
        slugs.push(entry.slug);
    }
    finalize(root, n, 0, 0.0, slugs, kinds)
}

fn add_entry(
    library: &LibraryService<FileStore>,
    sources: &Path,
    index: usize,
    kind: &str,
    name: String,
    description: String,
) -> Result<Entry, DatasetError> {
    let entry_kind = EntryKind::parse(kind.to_owned())
        .map_err(|error| DatasetError::Invalid(error.to_string()))?;
    if kind == "command" {
        let settings = EntrySettings {
            template: format!("echo {{msg}} entry-{index}"),
            ..EntrySettings::default()
        };
        return library
            .add(CreateEntry {
                name,
                kind: entry_kind,
                mode: StorageMode::Reference,
                source: String::new(),
                workdir: "invoke".to_owned(),
                description,
                payload: None,
                settings,
            })
            .map_err(|error| DatasetError::Product(error.to_string()));
    }
    let extension = extension(kind)?;
    let source = sources.join(format!("src_{index}.{extension}"));
    let body = if kind == "prompt" {
        format!(
            "Review the file {{{{path}}}} and summarize finding {index} in one paragraph.\nFocus on {{{{topic}}}}.\n"
        )
    } else {
        source_text(kind, index)?
    };
    write_file(&source, body.as_bytes())?;
    if kind == "exe" {
        make_executable(&source)?;
    }
    let parameters =
        if matches!(kind, "python" | "shell" | "js" | "ts" | "fish") && index.is_multiple_of(3) {
            benchmark_parameters()
        } else if kind == "prompt" {
            vec![ParamDecl::new("path"), ParamDecl::new("topic")]
        } else {
            Vec::new()
        };
    let settings = EntrySettings {
        parameters,
        ..EntrySettings::default()
    };
    let mode = if kind == "exe" {
        StorageMode::Reference
    } else {
        StorageMode::Copy
    };
    let stored_name =
        (mode == StorageMode::Copy).then(|| stored_filename(kind).unwrap_or("payload").to_owned());
    let bytes = read_file(&source)?;
    library
        .add(CreateEntry {
            name,
            kind: entry_kind,
            mode,
            source: source.display().to_string(),
            workdir: "origin".to_owned(),
            description,
            payload: Some(EntryPayload {
                bytes,
                stored_name,
                permissions: source_permissions(&source)?,
            }),
            settings,
        })
        .map_err(|error| DatasetError::Product(error.to_string()))
}

fn benchmark_parameters() -> Vec<ParamDecl> {
    let mut count = ParamDecl::new("count");
    count.flag = "--count".to_owned();
    let mut verbose = ParamDecl::new("verbose");
    verbose.parameter_type = ParameterType::Bool;
    verbose.flag = "--verbose".to_owned();
    verbose.action = "store_true".to_owned();
    vec![count, verbose]
}

fn entry_name(index: usize, rng: &mut PythonRandom) -> String {
    if index == 0 {
        return "alpha-seed-0".to_owned();
    }
    if index % 7 == 3 {
        return format!("測試腳本-{index}");
    }
    if index % 7 == 5 {
        return format!("🚀-tool-{index}");
    }
    if index.is_multiple_of(2) {
        return format!(
            "{}-{}-{}-{index}",
            random_word(rng),
            random_word(rng),
            random_word(rng)
        );
    }
    format!("{}-{index}", random_word(rng))
}

fn entry_description(index: usize, rng: &mut PythonRandom) -> String {
    if index == 0 || index.is_multiple_of(3) {
        return String::new();
    }
    if index % 3 == 1 {
        return format!("runs the {} task", random_word(rng));
    }
    "a long description that tells what this entry does, why it was added, and when to reach for it during daily work, in enough words to wrap a line".to_owned()
}

fn random_word(rng: &mut PythonRandom) -> &'static str {
    rng.choice(WORDS)
}

fn extension(kind: &str) -> Result<&'static str, DatasetError> {
    match kind {
        "python" => Ok("py"),
        "shell" | "exe" => Ok("sh"),
        "js" => Ok("js"),
        "ts" => Ok("ts"),
        "fish" => Ok("fish"),
        "ruby" => Ok("rb"),
        "perl" => Ok("pl"),
        "lua" => Ok("lua"),
        "r" => Ok("r"),
        "prompt" => Ok("md"),
        _ => Err(DatasetError::Invalid(format!(
            "no source extension for kind {kind:?}"
        ))),
    }
}

fn source_text(kind: &str, index: usize) -> Result<String, DatasetError> {
    let body = match kind {
        "python" => format!(
            "# a small generated benchmark entry\nimport sys\n\nprint('entry {index}', len(sys.argv))\n"
        ),
        "shell" | "exe" => format!(
            "#!/usr/bin/env bash\nGREETING=\"${{GREETING:-hello-{index}}}\"\necho \"$GREETING\"\n"
        ),
        "js" => format!("console.log('entry {index}', process.argv.length);\n"),
        "ts" => format!("const n: number = {index};\nconsole.log('entry', n);\n"),
        "fish" => format!("echo entry-{index}\n"),
        "ruby" => format!("puts 'entry {index}'\n"),
        "perl" => format!("print \"entry {index}\\n\";\n"),
        "lua" => format!("print('entry {index}')\n"),
        "r" => format!("cat('entry {index}\\n')\n"),
        _ => {
            return Err(DatasetError::Invalid(format!(
                "no source template for kind {kind:?}"
            )));
        }
    };
    Ok(body)
}

fn finalize(
    root: PathBuf,
    n: usize,
    seed: u64,
    state_fraction: f64,
    slugs: Vec<Slug>,
    kinds: BTreeMap<String, String>,
) -> Result<DatasetManifest, DatasetError> {
    let manifest = DatasetManifest {
        root: root.clone(),
        n,
        seed,
        state_fraction,
        generator_version: GENERATOR_VERSION,
        skit_version: env!("CARGO_PKG_VERSION").to_owned(),
        probe_char: SEARCH_PROBE_CHAR.to_owned(),
        slugs,
        kinds,
    };
    let mut staged = create_staged_manifest(&root)?;
    let staged_path = staged.path().to_path_buf();
    write_manifest(&mut staged, &staged_path, &manifest.to_json()?)?;
    persist_manifest(staged, &root.join("manifest.json"))?;
    Ok(manifest)
}

fn validate_manifest(manifest: &DatasetManifest) -> Result<(), DatasetError> {
    if manifest.slugs.len() != manifest.n || manifest.kinds.len() != manifest.n {
        return Err(DatasetError::UnreadableManifest {
            path: manifest.root.join("manifest.json"),
            reason: "entry counts do not match n".to_owned(),
        });
    }
    if manifest.generator_version == 0
        || manifest.skit_version.is_empty()
        || manifest.probe_char.is_empty()
        || !(0.0..=1.0).contains(&manifest.state_fraction)
    {
        return Err(DatasetError::UnreadableManifest {
            path: manifest.root.join("manifest.json"),
            reason: "required generation inputs are invalid".to_owned(),
        });
    }
    Ok(())
}

fn require_empty(root: &Path) -> Result<(), DatasetError> {
    if !root.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(root).map_err(|source| DatasetError::Io {
        operation: "scan",
        path: root.to_path_buf(),
        source,
    })?;
    if entries.next().is_some() {
        Err(DatasetError::NonEmpty(root.to_path_buf()))
    } else {
        Ok(())
    }
}

fn create_directory_tree(path: &Path) -> Result<(), DatasetError> {
    fs::create_dir_all(path).map_err(|source| io("create", path, source))
}

fn create_directory(path: &Path) -> Result<(), DatasetError> {
    fs::create_dir(path).map_err(|source| io("create", path, source))
}

fn remove_file(path: &Path) -> Result<(), DatasetError> {
    fs::remove_file(path).map_err(|source| io("remove", path, source))
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), DatasetError> {
    fs::write(path, bytes).map_err(|source| io("write", path, source))
}

fn read_file(path: &Path) -> Result<Vec<u8>, DatasetError> {
    fs::read(path).map_err(|source| io("read", path, source))
}

fn validate_generated_count(found: usize, expected: usize) -> Result<(), DatasetError> {
    if found == expected {
        Ok(())
    } else {
        Err(DatasetError::Invalid(format!(
            "generated {found} entries, expected {expected}"
        )))
    }
}

fn create_staged_manifest(root: &Path) -> Result<tempfile::NamedTempFile, DatasetError> {
    tempfile::NamedTempFile::new_in(root).map_err(|source| io("create", root, source))
}

fn write_manifest(
    writer: &mut impl std::io::Write,
    path: &Path,
    text: &str,
) -> Result<(), DatasetError> {
    writer
        .write_all(text.as_bytes())
        .map_err(|source| io("write", path, source))
}

fn persist_manifest(
    staged: tempfile::NamedTempFile,
    target: &Path,
) -> Result<(), DatasetError> {
    staged
        .persist(target)
        .map(|_| ())
        .map_err(|error| io("commit", target, error.error))
}

fn io(operation: &'static str, path: &Path, source: std::io::Error) -> DatasetError {
    DatasetError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

fn absolute(path: &Path) -> Result<PathBuf, DatasetError> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|source| DatasetError::Io {
            operation: "resolve",
            path: path.to_path_buf(),
            source,
        })
}

fn python_float(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.1}")
    } else {
        value.to_string()
    }
}

fn source_permissions(path: &Path) -> Result<SourcePermissions, DatasetError> {
    let metadata = fs::metadata(path).map_err(|source| DatasetError::Io {
        operation: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    let unix_mode = {
        use std::os::unix::fs::PermissionsExt as _;
        Some(metadata.permissions().mode())
    };
    #[cfg(not(unix))]
    let unix_mode = None;
    Ok(SourcePermissions {
        readonly: metadata.permissions().readonly(),
        unix_mode,
    })
}

fn make_executable(path: &Path) -> Result<(), DatasetError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|source| {
            DatasetError::Io {
                operation: "chmod",
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, io, path::Path};

    use tempfile::TempDir;

    #[test]
    fn private_source_contract_rejects_unsupported_kinds() {
        assert!(super::extension("unknown").is_err());
        assert!(super::source_text("unknown", 0).is_err());
    }

    #[test]
    fn python_float_keeps_cpython_seed_spelling() {
        assert_eq!(super::python_float(0.0), "0.0");
        assert_eq!(super::python_float(0.5), "0.5");
    }

    #[test]
    fn filesystem_adapter_names_each_failed_operation() {
        let root = TempDir::new().unwrap();
        let missing_parent = root.path().join("missing/child");
        let missing = root.path().join("missing-file");
        let ordinary = root.path().join("ordinary");
        fs::write(&ordinary, "content").unwrap();

        assert!(super::create_directory_tree(&ordinary.join("child")).is_err());
        assert!(super::create_directory(&ordinary).is_err());
        assert!(super::remove_file(&missing).is_err());
        assert!(super::write_file(&missing_parent, b"content").is_err());
        assert!(super::read_file(&missing).is_err());
        assert!(super::require_empty(&ordinary).is_err());
        assert!(super::source_permissions(&missing).is_err());
        assert!(super::absolute(Path::new("relative")).unwrap().is_absolute());
        assert!(super::validate_generated_count(2, 2).is_ok());
        assert!(super::validate_generated_count(1, 2).is_err());
        assert!(super::create_staged_manifest(&ordinary).is_err());

        struct FailedWriter;
        impl io::Write for FailedWriter {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("test write failure"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        assert!(super::write_manifest(&mut FailedWriter, &missing, "manifest").is_err());

        let staged = super::create_staged_manifest(root.path()).unwrap();
        let target = root.path().join("manifest.json");
        fs::create_dir(&target).unwrap();
        assert!(super::persist_manifest(staged, &target).is_err());

        #[cfg(unix)]
        assert!(super::make_executable(&missing).is_err());
    }
}
