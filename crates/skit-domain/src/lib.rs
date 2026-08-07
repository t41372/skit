//! Pure domain values and invariants for skit.
//!
//! This crate deliberately contains no filesystem, process, CLI, terminal, or GUI concepts.

#![forbid(unsafe_code)]

use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

/// A domain-value construction failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DomainError {
    /// A slug was not a canonical skit address.
    #[error("invalid entry slug: {0}")]
    InvalidSlug(String),
    /// An entry kind was blank.
    #[error("entry kind cannot be blank")]
    InvalidKind,
    /// An entry identity was not a UUID.
    #[error("invalid entry id: {0}")]
    InvalidEntryId(String),
}

/// The stable address of an entry directory.
///
/// A slug is not an identity: after removal, the address may legally be reused by another entry.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Slug(String);

impl Slug {
    /// Parse a canonical slug.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value == value.to_lowercase()
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.contains("--")
            && value
                .chars()
                .all(|character| character.is_alphanumeric() || character == '-');
        if valid {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidSlug(value))
        }
    }

    /// Derive the address used by the existing Python implementation.
    #[must_use]
    pub fn from_display_name(name: &str) -> Self {
        let mut output = String::new();
        let mut previous_was_dash = false;

        for character in name.trim().chars().flat_map(char::to_lowercase) {
            if character.is_alphanumeric() {
                output.push(character);
                previous_was_dash = false;
            } else if !previous_was_dash && !output.is_empty() {
                output.push('-');
                previous_was_dash = true;
            }
        }

        while output.ends_with('-') {
            output.pop();
        }
        if output.is_empty() {
            output.push_str("script");
        }
        Self(output)
    }

    /// Return the canonical string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for Slug {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Slug {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// An open entry-kind registry key.
///
/// Unknown kinds remain representable so a newer skit's metadata can still be listed, shown, and
/// removed by an older build.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryKind(String);

impl EntryKind {
    /// Parse a non-blank registry key.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            Err(DomainError::InvalidKind)
        } else {
            Ok(Self(trimmed.to_owned()))
        }
    }

    /// Return the registry key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntryKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for EntryKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EntryKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// The immutable identity of one incarnation of an entry.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryId(String);

impl EntryId {
    /// Parse either a simple or hyphenated UUID and normalize it to lowercase simple form.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        Uuid::parse_str(&value)
            .map(|id| Self(id.simple().to_string()))
            .map_err(|_| DomainError::InvalidEntryId(value))
    }

    /// Mint a new entry identity.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    /// Return the normalized UUID hex.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for EntryId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EntryId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// Whether skit owns a stored copy or launches the original path.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    /// skit owns a copy under `scripts/<slug>`.
    #[default]
    Copy,
    /// skit launches the user's original path.
    Reference,
}

/// Authoritative metadata read from one entry's `meta.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EntryMeta {
    /// Metadata schema number.
    pub schema: u32,
    /// User-facing display name.
    pub name: String,
    /// Open language/entry kind.
    pub kind: EntryKind,
    /// Copy or reference storage policy.
    pub mode: StorageMode,
    /// Original source path or command provenance.
    pub source: String,
    /// Source digest recorded by the current implementation.
    pub source_hash: String,
    /// UTC timestamp as written by the existing implementation.
    pub added_at: String,
    /// Immutable identity; absent for metadata written before identities existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<EntryId>,
    /// Work-directory policy spelling (`origin`, `store`, `invoke`, or an absolute path).
    pub workdir: String,
    /// User-facing description.
    pub description: String,
    /// Forward-compatible metadata fields not yet modeled by the migration kernel.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl EntryMeta {
    /// Construct the smallest valid metadata object for tests and application adapters.
    #[must_use]
    pub fn minimal(name: impl Into<String>, kind: EntryKind) -> Self {
        Self {
            schema: 1,
            name: name.into(),
            kind,
            mode: StorageMode::Copy,
            source: String::new(),
            source_hash: String::new(),
            added_at: String::new(),
            id: None,
            workdir: "origin".to_owned(),
            description: String::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// A fully resolved entry used by application use cases.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entry {
    /// Stable directory address.
    pub slug: Slug,
    /// Authoritative metadata.
    pub meta: EntryMeta,
}

/// The intentionally small projection needed by library listings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EntrySummary {
    /// Stable directory address.
    pub slug: Slug,
    /// Display name.
    pub name: String,
    /// Open language/entry kind.
    pub kind: EntryKind,
    /// Copy or reference mode.
    pub mode: StorageMode,
    /// Display description.
    pub description: String,
    /// Reference target; absent for copied entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}
