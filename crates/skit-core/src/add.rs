use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::{
    Entry, EntryDraft, Error as StoreError, ScriptMeta, Store, infer_kind, sha256_source_hash,
    shebang_program_from_line, spec_for,
};

/// How a file entry keeps its source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMode {
    Copy,
    Reference,
}

impl AddMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Reference => "reference",
        }
    }
}

/// Deterministic metadata supplied by the caller around one add operation.
///
/// Wall-clock access stays at the frontend boundary. The source hash is computed in
/// this use case from the same byte snapshot that is analyzed and copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddPreparation {
    pub added_at: String,
}

/// Input for the ordinary file add lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddFileRequest {
    pub source: PathBuf,
    pub name: Option<String>,
    pub kind: Option<String>,
    pub mode: AddMode,
    pub description: Option<String>,
    pub workdir: Option<String>,
    pub interpreter: Option<String>,
    pub preparation: AddPreparation,
}

/// Errors from the ordinary file add use case.
#[derive(Debug)]
pub enum AddUseCaseError {
    SourceNotFile(PathBuf),
    UnknownKind,
    UnsupportedKind(String),
    Io { path: PathBuf, source: io::Error },
    Store(StoreError),
}

impl fmt::Display for AddUseCaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotFile(path) => write!(formatter, "file not found: {}", path.display()),
            Self::UnknownKind => write!(formatter, "cannot determine the entry kind"),
            Self::UnsupportedKind(kind) => {
                write!(
                    formatter,
                    "entry kind is not valid for the file lane: {kind}"
                )
            }
            Self::Io { path, source } => {
                write!(formatter, "cannot access {}: {source}", path.display())
            }
            Self::Store(source) => source.fmt(formatter),
        }
    }
}

impl StdError for AddUseCaseError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Store(source) => Some(source),
            Self::SourceNotFile(_) | Self::UnknownKind | Self::UnsupportedKind(_) => None,
        }
    }
}

impl From<StoreError> for AddUseCaseError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

/// Add one ordinary file entry.
///
/// This use case accepts interpreted source files and executable files. Command and
/// prompt entries have separate lanes because their metadata and payload rules differ.
///
/// # Errors
///
/// Returns an error if the source is not a file, the kind is unknown or belongs to a
/// different lane, the source cannot be read, or the store transaction fails.
pub fn add_file(store: &Store, request: AddFileRequest) -> Result<Entry, AddUseCaseError> {
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
    let bytes = fs::read(&source).map_err(|source_error| AddUseCaseError::Io {
        path: source.clone(),
        source: source_error,
    })?;
    let source_hash = sha256_source_hash(&bytes);

    let kind = resolve_kind(&source, request.kind.as_deref())?;
    let text = String::from_utf8_lossy(&bytes);
    let name = request.name.unwrap_or_else(|| default_name(&source));
    let description = request
        .description
        .unwrap_or_else(|| infer_comment_description(&text, &kind));
    let interpreter = request
        .interpreter
        .unwrap_or_else(|| infer_interpreter(&source, &text, &kind));

    let mode = if kind == "exe" {
        AddMode::Reference
    } else {
        request.mode
    };
    let workdir = request.workdir.unwrap_or_else(|| match mode {
        AddMode::Copy => "invoke".to_owned(),
        AddMode::Reference => "origin".to_owned(),
    });
    let payload = (mode == AddMode::Copy).then_some(bytes);
    let meta = ScriptMeta {
        schema: 1,
        name,
        kind,
        mode: mode.as_str().to_owned(),
        source: source.to_string_lossy().into_owned(),
        source_hash,
        added_at: request.preparation.added_at,
        workdir,
        description,
        template: String::new(),
        dependencies: None,
        requires_python: String::new(),
        params: None,
        interpreter,
        runner: String::new(),
        interpolate: true,
        needs: None,
        parameters: None,
        extra: BTreeMap::new(),
    };
    store
        .insert_entry(EntryDraft::new(meta, payload))
        .map_err(AddUseCaseError::from)
}

fn resolve_kind(source: &Path, explicit: Option<&str>) -> Result<String, AddUseCaseError> {
    if let Some(kind) = explicit {
        if kind == "command" || kind == "prompt" || spec_for(kind).is_none() {
            return Err(AddUseCaseError::UnsupportedKind(kind.to_owned()));
        }
        return Ok(kind.to_owned());
    }
    let kind = infer_kind(source, false);
    if kind == "unknown" {
        return Err(AddUseCaseError::UnknownKind);
    }
    Ok(kind.to_owned())
}

fn default_name(source: &Path) -> String {
    source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "script".to_owned())
}

fn infer_comment_description(text: &str, kind: &str) -> String {
    let Some(prefix) = comment_prefix(kind) else {
        return String::new();
    };
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if index == 0 && trimmed.starts_with("#!") {
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        return trimmed
            .strip_prefix(prefix)
            .map_or_else(String::new, |value| value.trim().to_owned());
    }
    String::new()
}

const fn comment_prefix(kind: &str) -> Option<&'static str> {
    match kind.as_bytes() {
        b"shell" | b"fish" | b"powershell" | b"ruby" | b"perl" | b"r" => Some("#"),
        b"js" | b"ts" => Some("//"),
        b"lua" => Some("--"),
        _ => None,
    }
}

fn infer_interpreter(source: &Path, text: &str, kind: &str) -> String {
    if kind == "python" || kind == "exe" {
        return String::new();
    }
    let first_line = text.split_once('\n').map_or(text, |(line, _)| line);
    if let Some(program) = shebang_program_from_line(first_line)
        && spec_for(kind).is_some_and(|spec| spec.shebangs.contains(&program))
    {
        return program.to_owned();
    }
    if kind == "shell"
        && let Some(extension) = source.extension().and_then(|value| value.to_str())
    {
        return match extension.to_ascii_lowercase().as_str() {
            "bash" => "bash".to_owned(),
            "zsh" => "zsh".to_owned(),
            _ => String::new(),
        };
    }
    String::new()
}
