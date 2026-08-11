//! Define skit domain values and invariants.
//!
//! This crate does not access files, processes, terminals, or GUI APIs.

#![forbid(unsafe_code)]

pub mod parameters;

use std::{collections::BTreeMap, fmt};

use parameters::ParamDecl;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use serde_json::{Map, Value};
use skit_i18n::{Localize, Message};
use thiserror::Error;
use uuid::Uuid;

/// Report an invalid domain value.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DomainError {
    /// The slug is not a canonical skit address.
    #[error("invalid entry slug: {0}")]
    InvalidSlug(String),
    /// The entry kind is blank.
    #[error("entry kind cannot be blank")]
    InvalidKind,
    /// The entry ID is not a UUID.
    #[error("invalid entry id: {0}")]
    InvalidEntryId(String),
}

impl Localize for DomainError {
    fn message(&self) -> Message {
        match self {
            Self::InvalidSlug(value) => Message::new("invalid entry slug: {}").with(value),
            Self::InvalidKind => Message::new("entry kind cannot be blank"),
            Self::InvalidEntryId(value) => Message::new("invalid entry id: {}").with(value),
        }
    }
}

/// Identify one entry directory.
///
/// A slug is an address. A later entry can reuse it after removal.
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

    /// Make the address used by skit v0.4.
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

    /// Return the canonical text.
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

/// Identify an open entry kind.
///
/// Unknown kinds remain readable. This keeps newer metadata recoverable.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryKind(String);

impl EntryKind {
    /// Parse a non-blank kind.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let trimmed = value.trim();
        if trimmed.is_empty() {
            Err(DomainError::InvalidKind)
        } else {
            Ok(Self(trimmed.to_owned()))
        }
    }

    /// Return the kind text.
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

/// Identify one entry incarnation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntryId(String);

impl EntryId {
    /// Parse a UUID and store lowercase simple text.
    pub fn parse(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        Uuid::parse_str(&value)
            .map(|id| Self(id.simple().to_string()))
            .map_err(|_| DomainError::InvalidEntryId(value))
    }

    /// Create a new entry ID.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4().simple().to_string())
    }

    /// Return the normalized UUID text.
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

/// Select copied or referenced storage.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageMode {
    /// skit owns a copy under `scripts/<slug>`.
    #[default]
    Copy,
    /// skit uses the original path.
    Reference,
}

/// Store authoritative metadata from `meta.toml`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EntryMeta {
    /// Metadata schema number.
    pub schema: u32,
    /// Display name.
    pub name: String,
    /// Entry kind.
    pub kind: EntryKind,
    /// Storage policy.
    pub mode: StorageMode,
    /// Original source path or command origin.
    pub source: String,
    /// Recorded source digest.
    pub source_hash: String,
    /// UTC add timestamp.
    pub added_at: String,
    /// Immutable entry ID. Old metadata can omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<EntryId>,
    /// Work-directory policy.
    pub workdir: String,
    /// Display description.
    pub description: String,
    /// Optional and future metadata fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl EntryMeta {
    /// Create the smallest valid metadata object.
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

/// Give typed access to v0.4 optional metadata fields.
///
/// The on-disk field names do not change. Unknown fields stay in `EntryMeta::extra`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EntrySettings {
    /// Command template text.
    pub template: String,
    /// Managed package dependencies.
    pub dependencies: Vec<String>,
    /// Python version constraint.
    pub requires_python: String,
    /// Legacy command placeholder cache.
    pub params: Vec<String>,
    /// Pinned interpreter or JavaScript runtime.
    pub interpreter: String,
    /// Pinned prompt runner name.
    pub runner: String,
    /// Enable prompt placeholder insertion.
    pub interpolate: bool,
    /// External commands that must exist on PATH.
    pub needs: Vec<String>,
    /// Universal parameter declarations.
    pub parameters: Vec<ParamDecl>,
}

impl Default for EntrySettings {
    fn default() -> Self {
        Self {
            template: String::new(),
            dependencies: Vec::new(),
            requires_python: String::new(),
            params: Vec::new(),
            interpreter: String::new(),
            runner: String::new(),
            interpolate: true,
            needs: Vec::new(),
            parameters: Vec::new(),
        }
    }
}

impl EntrySettings {
    /// Read v0.4 optional fields from metadata.
    #[must_use]
    pub fn from_meta(meta: &EntryMeta) -> Self {
        Self {
            template: extra_string(meta, "template"),
            dependencies: extra_string_list(meta, "dependencies"),
            requires_python: extra_string(meta, "requires_python"),
            params: extra_string_list(meta, "params"),
            interpreter: extra_string(meta, "interpreter"),
            runner: extra_string(meta, "runner"),
            interpolate: !matches!(meta.extra.get("interpolate"), Some(Value::Bool(false))),
            needs: extra_string_list(meta, "needs"),
            parameters: meta
                .extra
                .get("parameters")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .map(|row| ParamDecl::from_meta_map(&row.clone().into_iter().collect()))
                .collect(),
        }
    }

    /// Write the typed fields without changing unknown extension fields.
    pub fn write_to_meta(&self, meta: &mut EntryMeta) {
        for key in [
            "template",
            "dependencies",
            "requires_python",
            "params",
            "interpreter",
            "runner",
            "interpolate",
            "needs",
            "parameters",
        ] {
            meta.extra.remove(key);
        }

        insert_string(&mut meta.extra, "template", &self.template);
        insert_string_list(&mut meta.extra, "dependencies", &self.dependencies);
        insert_string(&mut meta.extra, "requires_python", &self.requires_python);
        insert_string_list(&mut meta.extra, "params", &self.params);
        insert_string(&mut meta.extra, "interpreter", &self.interpreter);
        insert_string(&mut meta.extra, "runner", &self.runner);
        if !self.interpolate {
            meta.extra
                .insert("interpolate".to_owned(), Value::Bool(false));
        }
        insert_string_list(&mut meta.extra, "needs", &self.needs);
        if !self.parameters.is_empty() {
            meta.extra.insert(
                "parameters".to_owned(),
                Value::Array(
                    self.parameters
                        .iter()
                        .map(|parameter| {
                            Value::Object(
                                parameter
                                    .to_meta_map()
                                    .into_iter()
                                    .collect::<Map<String, Value>>(),
                            )
                        })
                        .collect(),
                ),
            );
        }
    }
}

fn extra_string(meta: &EntryMeta, key: &str) -> String {
    meta.extra
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn extra_string_list(meta: &EntryMeta, key: &str) -> Vec<String> {
    meta.extra
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

fn insert_string(extra: &mut BTreeMap<String, Value>, key: &str, value: &str) {
    if !value.is_empty() {
        extra.insert(key.to_owned(), Value::String(value.to_owned()));
    }
}

fn insert_string_list(extra: &mut BTreeMap<String, Value>, key: &str, values: &[String]) {
    if !values.is_empty() {
        extra.insert(
            key.to_owned(),
            Value::Array(values.iter().cloned().map(Value::String).collect()),
        );
    }
}

/// Hold a resolved entry.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Entry {
    /// Stable directory address.
    pub slug: Slug,
    /// Authoritative metadata.
    pub meta: EntryMeta,
}

/// Hold the fields needed by library lists.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EntrySummary {
    /// Stable directory address.
    pub slug: Slug,
    /// Display name.
    pub name: String,
    /// Entry kind.
    pub kind: EntryKind,
    /// Copy or reference mode.
    pub mode: StorageMode,
    /// Display description.
    pub description: String,
    /// Reference target. Copied entries omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}
