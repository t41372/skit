//! Deterministic path completion independent of one terminal toolkit.

use std::fmt::Debug;
use std::path::{Path, PathBuf};

use crate::tokens::{TokenContext, expand, has_tokens};

/// Maximum directory entries examined for one production completion request.
pub const PATH_SCAN_CAP: usize = 2_000;

/// Host spelling rules that decide when ordinary text looks like a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathInputDialect {
    /// POSIX path activation.
    Posix,
    /// Windows path activation, including backslashes and drive roots.
    Windows,
}

/// Whether a field is explicitly path-typed or uses universal text activation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathCompletionKind {
    /// Complete a nonempty prefix without an activation marker.
    Path,
    /// Complete only path-shaped text.
    Text,
}

/// Ambient roots and token values captured when a run form opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathCompletionContext {
    /// Directory in which the child resolves bare relative paths.
    pub workdir: PathBuf,
    /// Deterministic token expansion inputs, including the only `{cwd}` authority.
    pub tokens: TokenContext,
}

/// One complete path suggestion query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathCompletionRequest {
    /// Complete field text as typed by the user.
    pub value: String,
    /// Field activation policy.
    pub kind: PathCompletionKind,
    /// Complete only the trailing whitespace-delimited piece.
    pub shlexy: bool,
    /// Keep doubled braces literal during lookup.
    pub placeholder_braces: bool,
    /// Host path spelling rules.
    pub dialect: PathInputDialect,
    /// Form roots and tokens.
    pub context: PathCompletionContext,
}

/// One directory item returned by a filesystem adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    /// Display and completion name without a parent path.
    pub name: String,
    /// Whether completion must append `/` for chaining.
    pub is_directory: bool,
}

impl DirectoryEntry {
    /// Construct a plain-file entry.
    #[must_use]
    pub fn file(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_directory: false,
        }
    }

    /// Construct a directory entry.
    #[must_use]
    pub fn directory(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            is_directory: true,
        }
    }
}

/// Name policy that lets a filesystem adapter skip metadata probes for impossible matches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryReadFilter {
    prefix: String,
    include_hidden: bool,
}

impl DirectoryReadFilter {
    /// Construct one prefix and hidden-name policy.
    #[must_use]
    pub fn new(prefix: impl Into<String>, include_hidden: bool) -> Self {
        Self {
            prefix: prefix.into(),
            include_hidden,
        }
    }

    /// Report whether a directory name can match the current completion request.
    #[must_use]
    pub fn accepts(&self, name: &str) -> bool {
        (self.include_hidden || !name.starts_with('.')) && name.starts_with(&self.prefix)
    }
}

/// A directory could not produce a complete trustworthy scan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryReadError {
    /// Opening or iterating the directory failed.
    Unavailable,
}

/// Filesystem port for bounded directory enumeration.
pub trait DirectoryReader: Debug + Send + Sync {
    /// Examine at most `scan_cap` entries in filesystem iteration order.
    ///
    /// Apply `filter` after an entry counts toward the cap but before filesystem metadata probes.
    fn read_directory(
        &self,
        path: &Path,
        scan_cap: usize,
        filter: &DirectoryReadFilter,
    ) -> Result<Vec<DirectoryEntry>, DirectoryReadError>;
}

/// Object-safe completion surface used by asynchronous frontends.
pub trait PathCompletionProvider: Debug + Send + Sync {
    /// Return a complete field value or silence.
    fn complete(&self, request: &PathCompletionRequest) -> Option<String>;
}

/// Shared completion engine over one filesystem adapter.
#[derive(Debug)]
pub struct PathCompletionService<R> {
    reader: R,
    scan_cap: usize,
}

impl<R> PathCompletionService<R> {
    /// Use the production scan limit.
    #[must_use]
    pub const fn new(reader: R) -> Self {
        Self {
            reader,
            scan_cap: PATH_SCAN_CAP,
        }
    }

    /// Use an explicit scan limit for a bounded adapter contract.
    #[must_use]
    pub const fn with_scan_cap(reader: R, scan_cap: usize) -> Self {
        Self { reader, scan_cap }
    }
}

impl<R: DirectoryReader> PathCompletionService<R> {
    /// Complete one request without retaining filesystem results.
    #[must_use]
    pub fn complete(&self, request: &PathCompletionRequest) -> Option<String> {
        complete_with(&self.reader, self.scan_cap, request)
    }
}

impl<R: DirectoryReader> PathCompletionProvider for PathCompletionService<R> {
    fn complete(&self, request: &PathCompletionRequest) -> Option<String> {
        self.complete(request)
    }
}

/// Report whether ordinary text activates universal path completion.
#[must_use]
pub fn looks_pathy(piece: &str, dialect: PathInputDialect) -> bool {
    piece.starts_with('~')
        || piece.starts_with("{cwd}")
        || piece.contains('/')
        || (dialect == PathInputDialect::Windows
            && (piece.contains('\\') || has_windows_drive_root(piece)))
}

fn complete_with(
    reader: &dyn DirectoryReader,
    scan_cap: usize,
    request: &PathCompletionRequest,
) -> Option<String> {
    let piece = trailing_piece(&request.value, request.shlexy)?;
    if piece.is_empty()
        || (request.kind != PathCompletionKind::Path && !looks_pathy(piece, request.dialect))
    {
        return None;
    }
    let (base, prefix) = lookup(piece, request)?;
    let filter = DirectoryReadFilter::new(prefix, prefix.starts_with('.'));
    let mut matches = reader
        .read_directory(&base, scan_cap, &filter)
        .ok()?
        .into_iter()
        .filter(|entry| filter.accepts(&entry.name))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.name.cmp(&right.name));
    for entry in matches {
        let mut remainder = entry.name.get(prefix.len()..)?.to_owned();
        if entry.is_directory {
            remainder.push('/');
        }
        if remainder.is_empty() {
            continue;
        }
        let mut completed = request.value.clone();
        completed.push_str(&remainder);
        return Some(completed);
    }
    None
}

fn trailing_piece(value: &str, shlexy: bool) -> Option<&str> {
    if !shlexy {
        return Some(value);
    }
    let piece = value.rsplit(char::is_whitespace).next().unwrap_or(value);
    (!piece.contains('\'') && !piece.contains('"')).then_some(piece)
}

fn lookup<'a>(piece: &'a str, request: &PathCompletionRequest) -> Option<(PathBuf, &'a str)> {
    let separator = piece
        .char_indices()
        .rev()
        .find(|(_, character)| matches!(character, '/' | '\\'))
        .map(|(index, character)| index + character.len_utf8());
    let (head, prefix) = separator.map_or(("", piece), |end| piece.split_at(end));
    if head.is_empty() {
        if piece.starts_with('~') || piece.starts_with('{') {
            return None;
        }
        return Some((request.context.workdir.clone(), prefix));
    }

    let expanded = if has_tokens(head) {
        expand(head, &request.context.tokens, !request.placeholder_braces).ok()?
    } else {
        head.to_owned()
    };
    let base = PathBuf::from(expanded);
    let base = if base.is_absolute() {
        base
    } else {
        request.context.workdir.join(base)
    };
    Some((base, prefix))
}

fn has_windows_drive_root(piece: &str) -> bool {
    let bytes = piece.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}
