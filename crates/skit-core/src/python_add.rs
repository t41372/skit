use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{
    AddMode, AddUseCaseError, Entry, EntryDraft, ParamDecl, ScriptMeta, Store, has_pep723,
    inject_pep723, sha256_source_hash, write_python_params,
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
