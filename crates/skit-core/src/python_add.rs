use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::{
    AddMode, AddUseCaseError, Entry, EntryDraft, ScriptMeta, Store, has_pep723, inject_pep723,
    sha256_source_hash,
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
    if !request.source.is_file() {
        return Err(AddUseCaseError::SourceNotFile(request.source));
    }
    let source = request
        .source
        .canonicalize()
        .map_err(|source_error| AddUseCaseError::Io {
            path: request.source.clone(),
            source: source_error,
        })?;
    let mut stream = fs::File::open(&source).map_err(|source_error| AddUseCaseError::Io {
        path: source.clone(),
        source: source_error,
    })?;
    let mut source_bytes = Vec::new();
    stream
        .read_to_end(&mut source_bytes)
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
    let readonly = permissions.readonly();
    let unix_mode = unix_mode(&permissions);
    let source_hash = sha256_source_hash(&source_bytes);

    let dependencies = request
        .dependencies
        .into_iter()
        .map(|dependency| dependency.trim().to_owned())
        .filter(|dependency| !dependency.is_empty())
        .collect::<Vec<_>>();
    let wants_uv_metadata = !dependencies.is_empty() || !request.requires_python.is_empty();
    let strict_text = std::str::from_utf8(&source_bytes).ok();
    let inject_metadata = request.mode == AddMode::Copy
        && wants_uv_metadata
        && strict_text.is_some_and(|text| !has_pep723(text, "#"));

    let payload = match request.mode {
        AddMode::Reference => None,
        AddMode::Copy if inject_metadata => {
            let Some(text) = strict_text else {
                unreachable!("inject_metadata requires strict UTF-8");
            };
            Some(inject_pep723(text, &dependencies, &request.requires_python, "#").into_bytes())
        }
        AddMode::Copy => Some(source_bytes),
    };
    let name = request.name.unwrap_or_else(|| default_name(&source));
    let workdir = match request.mode {
        AddMode::Reference => "origin".to_owned(),
        AddMode::Copy => request.workdir.unwrap_or_else(|| "invoke".to_owned()),
    };
    let meta = ScriptMeta {
        schema: 1,
        name,
        kind: "python".to_owned(),
        mode: mode_text(request.mode).to_owned(),
        source: source.to_string_lossy().into_owned(),
        source_hash,
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
        draft.with_payload_permissions(readonly, unix_mode)
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
