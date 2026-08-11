//! Frontend-neutral planning for Python dependency metadata edits.

use thiserror::Error;

use crate::{
    LanguageError, LosslessSource, UvMetadata, has_uv_metadata_block_bytes, read_uv_metadata,
    write_uv_metadata_bytes,
};

/// One atomic Python dependency metadata edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UvMetadataEditPlan {
    /// Values that a later Python launch must use.
    pub effective: UvMetadata,
    /// Values to persist in entry metadata.
    pub stored: UvMetadata,
    /// Replacement source bytes when the source block can carry the edit.
    pub rewritten_source: Option<Vec<u8>>,
}

/// Report why a Python dependency metadata edit cannot be delivered.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum UvMetadataEditError {
    /// An invalid UTF-8 copy has an authoritative source block that cannot be rewritten.
    #[error("the invalid UTF-8 source has its own dependency block")]
    NonUtf8OwnBlock,
    /// The existing source metadata cannot be parsed or rewritten.
    #[error(transparent)]
    Language(#[from] LanguageError),
}

/// Read the Python dependency values that govern a run.
///
/// Stored metadata wins on each nonempty axis. A Python copy's source block fills only a blank
/// axis. The reversible text view matches the v0.4 replacement-decode read contract without
/// changing source bytes.
#[must_use]
pub fn effective_uv_metadata_bytes(source: Option<&[u8]>, stored: &UvMetadata) -> UvMetadata {
    let mut effective = stored.clone();
    if (!effective.dependencies.is_empty() && !effective.requires_python.is_empty())
        || source.is_none()
    {
        return effective;
    }
    let source = LosslessSource::from_bytes(source.expect("source presence was checked"));
    let Some(block) = read_uv_metadata(source.normalized_text()) else {
        return effective;
    };
    if effective.dependencies.is_empty() {
        effective.dependencies = block.dependencies;
    }
    if effective.requires_python.is_empty() {
        effective.requires_python = block.requires_python;
    }
    effective
}

/// Plan an edit while preserving the distinction between an untouched and a cleared axis.
///
/// Invalid UTF-8 without a source block keeps its bytes and carries the edit in metadata. If an
/// invalid UTF-8 copy owns a block, metadata cannot override that block, so an actual edit is
/// refused before either representation changes.
pub fn plan_uv_metadata_edit(
    source: Option<&[u8]>,
    stored: &UvMetadata,
    dependencies: Option<Vec<String>>,
    requires_python: Option<String>,
) -> Result<UvMetadataEditPlan, UvMetadataEditError> {
    let dependencies = dependencies.map(normalize_dependencies);
    let requires_python = requires_python.map(|value| normalize_python_constraint(&value));
    let mut effective = effective_uv_metadata_bytes(source, stored);
    let mut next_stored = stored.clone();

    if let Some(next) = &dependencies {
        effective.dependencies.clone_from(next);
        next_stored.dependencies.clone_from(next);
    }
    if let Some(next) = &requires_python {
        effective.requires_python.clone_from(next);
        next_stored.requires_python.clone_from(next);
    }

    let edited = dependencies.is_some() || requires_python.is_some();
    let rewritten_source = match (source, edited) {
        (_, false) | (None, true) => None,
        (Some(bytes), true) if std::str::from_utf8(bytes).is_err() => {
            if has_uv_metadata_block_bytes(bytes) {
                return Err(UvMetadataEditError::NonUtf8OwnBlock);
            }
            None
        }
        (Some(bytes), true) => {
            let rewritten = write_uv_metadata_bytes(
                bytes,
                &effective.dependencies,
                &effective.requires_python,
            )?;
            (rewritten != bytes).then_some(rewritten)
        }
    };

    Ok(UvMetadataEditPlan {
        effective,
        stored: next_stored,
        rewritten_source,
    })
}

fn normalize_dependencies(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

fn normalize_python_constraint(value: &str) -> String {
    let trimmed = value.trim();
    if matches!(trimmed.to_ascii_lowercase().as_str(), "-" | "none") {
        String::new()
    } else {
        trimmed.to_owned()
    }
}
