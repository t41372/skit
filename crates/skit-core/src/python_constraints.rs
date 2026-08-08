use std::error::Error as StdError;
use std::fmt;
use std::str::FromStr;

use pep440_rs::VersionSpecifiers;
use pep508_rs::Requirement;

/// One invalid user-supplied Python metadata value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonMetadataValidationError {
    pub field: &'static str,
    pub value: String,
    pub reason: String,
}

impl fmt::Display for PythonMetadataValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {} {:?}: {}",
            self.field, self.value, self.reason
        )
    }
}

impl StdError for PythonMetadataValidationError {}

/// Trim and validate one PEP 508 dependency specification.
///
/// Empty/whitespace-only values are dropped, matching the historical repeatable
/// `--dep` contract. Nonempty values are returned with surrounding whitespace removed
/// but otherwise preserve the user's spelling.
///
/// # Errors
///
/// Returns a named validation error when the requirement does not parse as PEP 508.
pub fn normalize_python_dependency(
    value: &str,
) -> Result<Option<String>, PythonMetadataValidationError> {
    let cleaned = value.trim();
    if cleaned.is_empty() {
        return Ok(None);
    }
    Requirement::from_str(cleaned).map_err(|source| PythonMetadataValidationError {
        field: "Python dependency",
        value: cleaned.to_owned(),
        reason: source.to_string(),
    })?;
    Ok(Some(cleaned.to_owned()))
}

/// Trim and validate a PEP 440 `requires-python` constraint.
///
/// Empty, `-`, and `none` all mean automatic/no explicit constraint. Every other
/// spelling must parse as a comma-separated PEP 440 version-specifier set.
///
/// # Errors
///
/// Returns a named validation error when the constraint is not valid PEP 440.
pub fn normalize_requires_python(value: &str) -> Result<String, PythonMetadataValidationError> {
    let cleaned = value.trim();
    if cleaned.is_empty() || matches!(cleaned.to_ascii_lowercase().as_str(), "-" | "none") {
        return Ok(String::new());
    }
    VersionSpecifiers::from_str(cleaned).map_err(|source| PythonMetadataValidationError {
        field: "Python constraint",
        value: cleaned.to_owned(),
        reason: source.to_string(),
    })?;
    Ok(cleaned.to_owned())
}
