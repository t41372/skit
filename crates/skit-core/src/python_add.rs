use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{
    AddMode, AddUseCaseError, Entry, EntryDraft, ParamDecl, PythonMetadataValidationError,
    ScriptMeta, Store, analyze_python_managed, has_pep723, inject_pep723,
    normalize_python_dependency, normalize_requires_python, parse_pep723, python_version_pin,
    read_python_params, sha256_source_hash, shebang_program_from_line, suggest_python_dependencies,
    write_python_params,
};

/// Fully resolved inputs for the Python storage lane. Static analysis/onboarding is a
/// separate layer; this use case owns byte fidelity and metadata placement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonAddRequest {
    pub source: PathBuf,
    pub name: Option<String>,
    pub mode: AddMode,
    pub description: String,
    pub workdir: Option<String>,
    pub dependencies: Vec<String>,
    pub requires_python: String,
    pub added_at: String,
}

/// Frontend policy for Python intake before an interactive review surface exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonAutoAddRequest {
    pub source: PathBuf,
    pub name: Option<String>,
    pub mode: AddMode,
    pub description: Option<String>,
    pub workdir: Option<String>,
    pub added_at: String,
    pub interactive: bool,
    pub no_input: bool,
    /// `None` means infer dependencies. `Some` means the caller explicitly chose the
    /// list, even if validation/empty-dropping leaves it empty.
    pub dependencies: Option<Vec<String>>,
    /// `None` means use a versioned shebang pin when present. `Some` is explicit;
    /// empty/`-`/`none` normalize to automatic/no constraint.
    pub requires_python: Option<String>,
}

/// What automatic Python intake accepted or discovered.
#[derive(Debug)]
pub struct PythonAutoAddOutcome {
    pub entry: Entry,
    pub dependencies: Vec<String>,
    pub requires_python: String,
    /// New source candidates only. Automatic/no-input intake deliberately does not
    /// manage them; a review frontend may later offer this list for selection.
    pub parameter_candidates: Vec<String>,
}

/// Python-specific add failures above the ordinary file/store boundary.
#[derive(Debug)]
pub enum PythonAddError {
    Add(AddUseCaseError),
    ManagedParametersRequireCopy,
    ManagedParametersRequireUtf8,
}

impl fmt::Display for PythonAddError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add(source) => source.fmt(formatter),
            Self::ManagedParametersRequireCopy => formatter.write_str(
                "new managed Python parameters require copy mode; reference mode never rewrites the original",
            ),
            Self::ManagedParametersRequireUtf8 => formatter.write_str(
                "new managed Python parameters require strict UTF-8 source so the stored copy can be rewritten byte-safely",
            ),
        }
    }
}

impl StdError for PythonAddError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Add(source) => Some(source),
            Self::ManagedParametersRequireCopy | Self::ManagedParametersRequireUtf8 => None,
        }
    }
}

impl From<AddUseCaseError> for PythonAddError {
    fn from(value: AddUseCaseError) -> Self {
        Self::Add(value)
    }
}

/// Automatic Python intake cannot continue without a review or because explicit
/// metadata cannot be honored safely.
#[derive(Debug)]
pub enum PythonAutoAddError {
    Add(AddUseCaseError),
    Validation(PythonMetadataValidationError),
    SourceMetadataConflict,
    ReviewRequired {
        dependencies: Vec<String>,
        parameters: Vec<String>,
    },
}

impl fmt::Display for PythonAutoAddError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Add(source) => source.fmt(formatter),
            Self::Validation(source) => source.fmt(formatter),
            Self::SourceMetadataConflict => formatter.write_str(
                "the Python source already declares PEP 723 metadata; drop --dep/--python or edit the source block instead",
            ),
            Self::ReviewRequired {
                dependencies,
                parameters,
            } => {
                formatter.write_str(
                    "Python intake needs review before writing; the Rust interactive review surface is not enabled yet. Rerun with --no-input to accept dependency suggestions and skip new managed parameters.",
                )?;
                if !dependencies.is_empty() {
                    write!(formatter, " Dependencies: {}.", dependencies.join(", "))?;
                }
                if !parameters.is_empty() {
                    write!(formatter, " Parameter candidates: {}.", parameters.join(", "))?;
                }
                Ok(())
            }
        }
    }
}

impl StdError for PythonAutoAddError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Add(source) => Some(source),
            Self::Validation(source) => Some(source),
            Self::SourceMetadataConflict | Self::ReviewRequired { .. } => None,
        }
    }
}

impl From<AddUseCaseError> for PythonAutoAddError {
    fn from(value: AddUseCaseError) -> Self {
        Self::Add(value)
    }
}

impl From<PythonMetadataValidationError> for PythonAutoAddError {
    fn from(value: PythonMetadataValidationError) -> Self {
        Self::Validation(value)
    }
}

#[derive(Debug)]
struct SourceSnapshot {
    source: PathBuf,
    bytes: Vec<u8>,
    readonly: bool,
    unix_mode: Option<u32>,
    source_hash: String,
}

/// Add one Python source file without performing parser-backed parameter onboarding.
///
/// Copy mode embeds explicitly supplied UV metadata into a new PEP 723 block only when
/// the original is strict UTF-8 and has no existing block. Otherwise the copied bytes
/// stay exact and those axes live in meta. Reference mode never modifies the original.
///
/// # Errors
///
/// Returns an error if the source is not a regular file, cannot be read, or the shared
/// store transaction fails.
pub fn add_python_file(store: &Store, request: PythonAddRequest) -> Result<Entry, AddUseCaseError> {
    let snapshot = read_snapshot(&request.source)?;
    add_snapshot(store, request, snapshot, &[])
}

/// Add one Python copy with an already-reviewed frozen managed-parameter schema.
///
/// The source is read exactly once. Its original bytes are the hash/permission truth;
/// PEP 723 axes and `[tool.skit]` are composed in memory into one final payload before
/// `Store::insert_entry` begins. A failure can therefore never leave metadata committed
/// while the stored script missed its managed schema.
///
/// Reference mode is refused for a newly supplied managed schema because it may never
/// rewrite the original. Non-UTF-8 source is also refused for new managed parameters;
/// re-encoding replacement characters would violate byte fidelity.
///
/// # Errors
///
/// Returns Python-specific policy errors or the ordinary source/store failures.
pub fn add_python_file_with_params(
    store: &Store,
    request: PythonAddRequest,
    managed_params: &[ParamDecl],
) -> Result<Entry, PythonAddError> {
    if !managed_params.is_empty() && request.mode == AddMode::Reference {
        return Err(PythonAddError::ManagedParametersRequireCopy);
    }
    let snapshot = read_snapshot(&request.source)?;
    if !managed_params.is_empty() && std::str::from_utf8(&snapshot.bytes).is_err() {
        return Err(PythonAddError::ManagedParametersRequireUtf8);
    }
    add_snapshot(store, request, snapshot, managed_params).map_err(PythonAddError::from)
}

/// Analyze and add Python from one immutable source snapshot.
///
/// Existing PEP 723 metadata is authoritative. Explicit dependency/Python overrides
/// are validated before storage and are refused when a source block already exists so
/// no caller-supplied flag can be silently ignored. Without a block, explicit values
/// override automatic import suggestions and the versioned-shebang pin respectively.
///
/// New managed-parameter candidates are never selected automatically, matching the
/// Python-era noninteractive contract. Interactive review is required only for
/// auto-suggested dependencies and new parameter candidates; explicit dependencies are
/// already a caller decision and are not asked about again.
///
/// # Errors
///
/// Returns validation/source/store failures, a source-metadata conflict, or an explicit
/// pre-write review refusal.
pub fn add_python_auto(
    store: &Store,
    request: PythonAutoAddRequest,
) -> Result<PythonAutoAddOutcome, PythonAutoAddError> {
    let explicit_dependencies = normalize_dependencies(request.dependencies.as_deref())?;
    let explicit_requires_python = request
        .requires_python
        .as_deref()
        .map(normalize_requires_python)
        .transpose()?;
    let snapshot = read_snapshot(&request.source)?;
    let mut dependencies = explicit_dependencies.clone().unwrap_or_default();
    let mut requires_python = explicit_requires_python.clone().unwrap_or_default();
    let mut effective_dependencies = Vec::new();
    let mut effective_requires_python = String::new();
    let mut parameter_candidates = Vec::new();
    let mut automatic_dependencies = Vec::new();

    if let Ok(text) = std::str::from_utf8(&snapshot.bytes) {
        if has_pep723(text, "#") {
            if explicit_dependencies.is_some() || explicit_requires_python.is_some() {
                return Err(PythonAutoAddError::SourceMetadataConflict);
            }
            if let Some(metadata) = parse_pep723(text, "#") {
                effective_dependencies = metadata.dependencies;
                effective_requires_python = metadata.requires_python;
            }
        } else {
            if explicit_dependencies.is_none() {
                automatic_dependencies =
                    suggest_python_dependencies(text, snapshot.source.parent());
                dependencies.clone_from(&automatic_dependencies);
            }
            if explicit_requires_python.is_none() {
                let first_line = text.split_once('\n').map_or(text, |(line, _)| line);
                requires_python = python_version_pin(shebang_program_from_line(first_line));
            }
            effective_dependencies.clone_from(&dependencies);
            effective_requires_python.clone_from(&requires_python);
        }
        if read_python_params(text).is_empty() {
            parameter_candidates = analyze_python_managed(text)
                .candidates
                .into_iter()
                .map(|candidate| candidate.decl.name)
                .collect();
        }
    } else {
        effective_dependencies.clone_from(&dependencies);
        effective_requires_python.clone_from(&requires_python);
    }

    if request.interactive
        && !request.no_input
        && (!automatic_dependencies.is_empty() || !parameter_candidates.is_empty())
    {
        return Err(PythonAutoAddError::ReviewRequired {
            dependencies: automatic_dependencies,
            parameters: parameter_candidates,
        });
    }

    let storage = PythonAddRequest {
        source: request.source,
        name: request.name,
        mode: request.mode,
        description: request.description.unwrap_or_default(),
        workdir: request.workdir,
        dependencies,
        requires_python,
        added_at: request.added_at,
    };
    let entry = add_snapshot(store, storage, snapshot, &[])?;
    Ok(PythonAutoAddOutcome {
        entry,
        dependencies: effective_dependencies,
        requires_python: effective_requires_python,
        parameter_candidates,
    })
}

fn normalize_dependencies(
    values: Option<&[String]>,
) -> Result<Option<Vec<String>>, PythonMetadataValidationError> {
    values
        .map(|values| {
            values
                .iter()
                .filter_map(|value| normalize_python_dependency(value).transpose())
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
}

fn read_snapshot(source: &Path) -> Result<SourceSnapshot, AddUseCaseError> {
    if !source.is_file() {
        return Err(AddUseCaseError::SourceNotFile(source.to_owned()));
    }
    let source = source
        .canonicalize()
        .map_err(|source_error| AddUseCaseError::Io {
            path: source.to_owned(),
            source: source_error,
        })?;
    let mut stream = fs::File::open(&source).map_err(|source_error| AddUseCaseError::Io {
        path: source.clone(),
        source: source_error,
    })?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|source_error| AddUseCaseError::Io {
            path: source.clone(),
            source: source_error,
        })?;
    let permissions = stream
        .metadata()
        .map_err(|source_error| AddUseCaseError::Io {
            path: source.clone(),
            source: source_error,
        })?
        .permissions();
    Ok(SourceSnapshot {
        source,
        source_hash: sha256_source_hash(&bytes),
        bytes,
        readonly: permissions.readonly(),
        unix_mode: unix_mode(&permissions),
    })
}

fn add_snapshot(
    store: &Store,
    request: PythonAddRequest,
    snapshot: SourceSnapshot,
    managed_params: &[ParamDecl],
) -> Result<Entry, AddUseCaseError> {
    let dependencies = request
        .dependencies
        .into_iter()
        .map(|dependency| dependency.trim().to_owned())
        .filter(|dependency| !dependency.is_empty())
        .collect::<Vec<_>>();
    let wants_uv_metadata = !dependencies.is_empty() || !request.requires_python.is_empty();
    let strict_utf8 = std::str::from_utf8(&snapshot.bytes).is_ok();
    let has_existing_block =
        std::str::from_utf8(&snapshot.bytes).is_ok_and(|text| has_pep723(text, "#"));
    let inject_metadata =
        request.mode == AddMode::Copy && wants_uv_metadata && strict_utf8 && !has_existing_block;

    let payload = match request.mode {
        AddMode::Reference => None,
        AddMode::Copy => {
            if let Ok(text) = std::str::from_utf8(&snapshot.bytes)
                && (inject_metadata || !managed_params.is_empty())
            {
                let with_uv = if inject_metadata {
                    inject_pep723(text, &dependencies, &request.requires_python, "#")
                } else {
                    text.to_owned()
                };
                let final_text = if managed_params.is_empty() {
                    with_uv
                } else {
                    write_python_params(&with_uv, managed_params)
                };
                Some(final_text.into_bytes())
            } else {
                Some(snapshot.bytes)
            }
        }
    };
    let name = request
        .name
        .unwrap_or_else(|| default_name(&snapshot.source));
    let workdir = match request.mode {
        AddMode::Reference => "origin".to_owned(),
        AddMode::Copy => request.workdir.unwrap_or_else(|| "invoke".to_owned()),
    };
    let meta = ScriptMeta {
        schema: 1,
        name,
        kind: "python".to_owned(),
        mode: mode_text(request.mode).to_owned(),
        source: snapshot.source.to_string_lossy().into_owned(),
        source_hash: snapshot.source_hash,
        added_at: request.added_at,
        workdir,
        description: request.description,
        template: String::new(),
        dependencies: if inject_metadata || dependencies.is_empty() {
            None
        } else {
            Some(dependencies)
        },
        requires_python: if inject_metadata {
            String::new()
        } else {
            request.requires_python
        },
        params: None,
        interpreter: String::new(),
        runner: String::new(),
        interpolate: true,
        needs: None,
        parameters: None,
        extra: BTreeMap::new(),
    };
    let draft = EntryDraft::new(meta, payload);
    let draft = if request.mode == AddMode::Copy {
        draft.with_payload_permissions(snapshot.readonly, snapshot.unix_mode)
    } else {
        draft
    };
    store.insert_entry(draft).map_err(AddUseCaseError::from)
}

const fn mode_text(mode: AddMode) -> &'static str {
    match mode {
        AddMode::Copy => "copy",
        AddMode::Reference => "reference",
    }
}

fn default_name(source: &Path) -> String {
    source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "script".to_owned())
}

#[cfg(unix)]
fn unix_mode(permissions: &fs::Permissions) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;

    Some(permissions.mode())
}

#[cfg(not(unix))]
fn unix_mode(_permissions: &fs::Permissions) -> Option<u32> {
    None
}
