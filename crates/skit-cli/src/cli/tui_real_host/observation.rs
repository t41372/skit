//! Host observations, leak oracle facts, and the tree snapshot of one profile.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{Value, json};
use skit_application::LibraryService;
use skit_runtime::LaunchPlan;
use skit_store::FileStore;
use skit_ui::{Action, AddAction};

use super::{
    adapters::{AllocationOutcome, AllocationPurpose, PortEvent, ProbeResult},
    projection::PathMap,
    seed::WalkerExternalSeed,
    stable_sandbox::SandboxOwner,
};
use crate::cli::tui_host::{PrivateDirectoryPurpose, ProductRoots, TemporaryFilePurpose};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct HostObservation {
    pub(crate) surface: Value,
    pub(crate) state: Value,
    pub(crate) config: Value,
    pub(crate) form_state: BTreeMap<String, Value>,
    pub(crate) prompt_runner: String,
    pub(crate) drafts: Value,
    pub(crate) tree: Vec<TreeRecord>,
    pub(crate) transcript: Vec<Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LeakOracleFacts {
    pub(crate) artifacts: BTreeMap<String, ArtifactLeakOracleFact>,
    pub(crate) renderer_drafts: BTreeMap<String, RendererDraftFact>,
    /// Paths from the process that created this host. These facts never enter recorded JSON.
    pub(crate) ambient_paths: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArtifactLeakOracleFact {
    pub(crate) raw_path_spellings: BTreeSet<String>,
    pub(crate) source_identities: BTreeSet<SourceIdentityLeakOraclePair>,
    pub(crate) modified_values: BTreeSet<ModifiedLeakOraclePair>,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct SourceIdentityLeakOraclePair {
    pub(crate) raw: Vec<u8>,
    pub(crate) projected: Vec<u8>,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ModifiedLeakOraclePair {
    pub(crate) raw: u64,
    pub(crate) projected: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RendererDraftFact {
    pub(crate) raw_path: String,
    pub(crate) projected_path: String,
    pub(crate) raw_kind_picker_basename: String,
    pub(crate) raw_lossy_basename: String,
    pub(crate) projected_basename: String,
    pub(crate) review_names: BTreeSet<RendererReviewNameFact>,
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RendererReviewNameFact {
    pub(crate) kind: String,
    pub(crate) raw_source_path: String,
    pub(crate) projected_source_path: String,
    pub(crate) raw_name: String,
    pub(crate) projected_name: String,
}

pub(super) struct PendingHostObservation {
    pub(super) surface: Value,
    pub(super) config: Value,
    pub(super) form_state: BTreeMap<String, Value>,
    pub(super) prompt_runner: String,
    pub(super) drafts: Value,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ObservationNodeKind {
    File,
    Directory,
    Symlink,
}

impl ObservationNodeKind {
    fn from_metadata(metadata: &fs::Metadata) -> Self {
        if metadata.file_type().is_symlink() {
            Self::Symlink
        } else if metadata.is_file() {
            Self::File
        } else {
            Self::Directory
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Symlink => "symlink",
        }
    }
}

pub(super) fn has_store_owned_copy_payload(entry: &skit_domain::Entry) -> bool {
    entry.meta.mode == skit_domain::StorageMode::Copy
        && !matches!(entry.meta.kind.as_str(), "command" | "exe")
}

#[derive(Debug, Default)]
pub(super) struct ObservationModeProvenance {
    pub(super) exact: BTreeMap<ObservationModeKey, ObservationNodeKind>,
    pub(super) deferred_error: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ObservationModeKey(PathBuf);

impl ObservationModeKey {
    fn from_path(path: &Path) -> Result<Self, String> {
        if !path.is_absolute() {
            return Err(format!(
                "an observation mode provenance path is not absolute: {}",
                path.display()
            ));
        }
        let Some((parent, name)) = path.parent().zip(path.file_name()) else {
            return Err(format!(
                "an observation mode provenance path has no final component: {}",
                path.display()
            ));
        };
        let parent = fs::canonicalize(parent).map_err(|error| {
            format!(
                "could not resolve observation mode provenance parent {}: {error}",
                parent.display()
            )
        })?;
        Ok(Self(parent.join(name)))
    }
}

impl ObservationModeProvenance {
    pub(super) fn from_seed(
        sandbox: &SandboxOwner,
        roots: &ProductRoots,
        external_root: &Path,
        system_temp: &Path,
        seeded_directories: &[PathBuf],
        external: &[WalkerExternalSeed],
        seeded_copy_paths: &[PathBuf],
    ) -> Result<Self, String> {
        let mut provenance = Self::default();
        if sandbox.stable().is_some() {
            for path in [
                roots.data.as_path(),
                roots.state.as_path(),
                roots.config.as_path(),
                roots
                    .home
                    .as_deref()
                    .expect("the walker profile has a home"),
                roots.cwd.as_path(),
                external_root,
                system_temp,
            ] {
                provenance.register(path, ObservationNodeKind::Directory)?;
            }
        }
        for seed in external {
            match seed {
                WalkerExternalSeed::File { path, .. } => {
                    provenance.register(&external_root.join(path), ObservationNodeKind::File)?;
                }
                WalkerExternalSeed::Symlink { path, .. } => {
                    provenance.register(&external_root.join(path), ObservationNodeKind::Symlink)?;
                }
            }
        }
        for path in seeded_directories {
            provenance.register(path, ObservationNodeKind::Directory)?;
        }
        for path in seeded_copy_paths {
            provenance.register(path, ObservationNodeKind::File)?;
        }
        Ok(provenance)
    }

    pub(super) fn register(
        &mut self,
        path: &Path,
        kind: ObservationNodeKind,
    ) -> Result<(), String> {
        let key = ObservationModeKey::from_path(path)?;
        if let Some(previous) = self.exact.get(&key) {
            if previous != &kind {
                return Err(format!(
                    "observation mode provenance conflicts at {}: {} and {}",
                    path.display(),
                    previous.label(),
                    kind.label()
                ));
            }
            return Ok(());
        }
        self.exact.insert(key, kind);
        Ok(())
    }

    pub(super) fn register_if_parent_present(
        &mut self,
        path: &Path,
        kind: ObservationNodeKind,
    ) -> Result<(), String> {
        let Some(parent) = path.parent() else {
            return self.register(path, kind);
        };
        if matches!(
            fs::symlink_metadata(parent),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ) {
            return Ok(());
        }
        self.register(path, kind)
    }

    pub(super) fn expected_kind(&self, path: &Path) -> Result<Option<ObservationNodeKind>, String> {
        let key = ObservationModeKey::from_path(path)?;
        Ok(self.exact.get(&key).copied())
    }

    pub(super) fn register_existing(&mut self, path: &Path) -> Result<(), String> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                let kind = ObservationNodeKind::from_metadata(&metadata);
                self.register(path, kind)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "could not inspect observation mode provenance at {}: {error}",
                path.display()
            )),
        }
    }

    pub(super) fn refresh_live_sources(
        &mut self,
        service: &LibraryService<FileStore>,
    ) -> Result<(), String> {
        if let Some(error) = &self.deferred_error {
            return Err(error.clone());
        }
        let mut entries = service
            .repository()
            .scan_entries()
            .map_err(|error| error.to_string())?;
        entries.sort_by(|left, right| left.slug.cmp(&right.slug));
        for entry in entries {
            if has_store_owned_copy_payload(&entry) {
                if let Ok(path) = service.repository().payload_path(&entry) {
                    self.register(&path, ObservationNodeKind::File)?;
                }
            } else if entry.meta.kind.as_str() != "command"
                && let Ok(path) = service.repository().payload_path(&entry)
            {
                self.register_existing(&path)?;
            }
        }
        for draft in sorted_tui_drafts(service.repository().data_dir()) {
            self.register(&draft.path, ObservationNodeKind::File)?;
        }
        Ok(())
    }

    pub(super) fn record_created_copy(
        &mut self,
        service: &LibraryService<FileStore>,
        action: &Action,
    ) {
        let result = self.try_record_created_copy(service, action);
        if let Err(error) = result
            && self.deferred_error.is_none()
        {
            self.deferred_error = Some(error);
        }
    }

    pub(super) fn try_record_created_copy(
        &mut self,
        service: &LibraryService<FileStore>,
        action: &Action,
    ) -> Result<(), String> {
        let Action::Add(AddAction::CommitFinished {
            result: Ok(slug), ..
        }) = action
        else {
            return Ok(());
        };
        let entry = service.show(slug).map_err(|error| error.to_string())?;
        if !has_store_owned_copy_payload(&entry) {
            return Ok(());
        }
        let path = service
            .repository()
            .payload_path(&entry)
            .map_err(|error| error.to_string())?;
        self.register(&path, ObservationNodeKind::File)
    }

    pub(super) fn register_event(
        &mut self,
        event: &PortEvent,
        roots: &ProductRoots,
    ) -> Result<(), String> {
        match event {
            PortEvent::Allocation {
                purpose,
                path: Some(path),
                outcome: AllocationOutcome::Accepted,
                ..
            } => {
                let kind = match purpose {
                    AllocationPurpose::TemporaryFile(TemporaryFilePurpose::AuthoredDraft)
                    | AllocationPurpose::TemporaryFile(TemporaryFilePurpose::InjectedSource) => {
                        ObservationNodeKind::File
                    }
                    AllocationPurpose::PrivateDirectory(
                        PrivateDirectoryPurpose::DraftQuarantine,
                    ) => ObservationNodeKind::Directory,
                };
                self.register_if_parent_present(path, kind)?;
            }
            PortEvent::Probe {
                operation,
                path,
                result: ProbeResult::Bool(true),
            } if matches!(operation.as_str(), "is_file" | "is_executable") => {
                self.register_managed_executable(path, roots)?;
            }
            PortEvent::Probe {
                result: ProbeResult::Path(Some(path)),
                ..
            } => {
                self.register_managed_executable(path, roots)?;
            }
            PortEvent::Launch { plan, .. } => {
                self.register_managed_executable(&plan.program, roots)?;
            }
            PortEvent::Clock(_)
            | PortEvent::Platform(_)
            | PortEvent::EnvironmentVariable { .. }
            | PortEvent::EnvironmentSnapshot(_)
            | PortEvent::LocalOffset(_)
            | PortEvent::SystemLocale(_)
            | PortEvent::Terminal { .. }
            | PortEvent::Allocation { .. }
            | PortEvent::Probe { .. }
            | PortEvent::Editor { .. }
            | PortEvent::Dependency { .. }
            | PortEvent::Injected { .. }
            | PortEvent::JavaScriptGate { .. }
            | PortEvent::UvConsent { .. }
            | PortEvent::UvFetch { .. }
            | PortEvent::Preference { .. }
            | PortEvent::Output { .. } => {}
        }
        Ok(())
    }

    fn register_managed_executable(
        &mut self,
        path: &Path,
        roots: &ProductRoots,
    ) -> Result<(), String> {
        if path == skit_runtime::managed_uv_path(&roots.data) {
            self.register(path, ObservationNodeKind::File)?;
        }
        Ok(())
    }

    pub(super) fn mode(&self, path: &Path, metadata: &fs::Metadata) -> Result<Option<u32>, String> {
        let actual = ObservationNodeKind::from_metadata(metadata);
        let expected = self.expected_kind(path)?;
        if let Some(expected) = expected
            && expected != actual
        {
            return Err(format!(
                "observation mode provenance expected {} at {}, but found {}",
                expected.label(),
                path.display(),
                actual.label()
            ));
        }
        let raw = portable_mode(metadata);
        if expected.is_some()
            || actual == ObservationNodeKind::Symlink
            || raw.is_some_and(|mode| mode & 0o7000 != 0)
        {
            Ok(raw)
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct TreeRecord {
    pub(crate) path: String,
    pub(crate) kind: String,
    pub(crate) readonly: bool,
    pub(crate) mode: Option<u32>,
    pub(crate) target: Option<String>,
    pub(crate) content: Option<ByteView>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "encoding", content = "data", rename_all = "snake_case")]
pub(crate) enum ByteView {
    Utf8(String),
    Hex(String),
}

pub(super) fn sorted_tui_drafts(data_dir: &Path) -> Vec<skit_ui::DraftSummary> {
    let mut drafts = crate::cli::tui_drafts(data_dir);
    drafts.sort_by(|left, right| {
        fs::read(&left.path)
            .unwrap_or_default()
            .cmp(&fs::read(&right.path).unwrap_or_default())
            .then_with(|| left.path.extension().cmp(&right.path.extension()))
            .then_with(|| left.path.cmp(&right.path))
    });
    drafts
}

pub(super) fn snapshot_tree(
    roots: &ProductRoots,
    external_root: &Path,
    paths: &mut PathMap,
    modes: &ObservationModeProvenance,
) -> Result<Vec<TreeRecord>, String> {
    let mut output = Vec::new();
    for root in [
        roots.data.as_path(),
        roots.state.as_path(),
        roots.config.as_path(),
        roots
            .home
            .as_deref()
            .expect("the walker profile has a home"),
        roots.cwd.as_path(),
        external_root,
    ] {
        snapshot_path(root, paths, modes, &mut output)?;
    }
    output.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(output)
}

pub(super) fn ensure_system_temp_empty(system_temp: &Path, paths: &PathMap) -> Result<(), String> {
    let entries = fs::read_dir(system_temp)
        .map_err(|error| format!("could not inspect walker system temp: {error}"))?
        .map(|entry| entry.map(|entry| entry.path()));
    if let Some(residual) = first_system_temp_residual(entries)? {
        return Err(format!(
            "walker system temp retained an artifact: {}",
            paths.normalize_path(&residual)
        ));
    }
    Ok(())
}

pub(super) fn first_system_temp_residual(
    entries: impl IntoIterator<Item = io::Result<PathBuf>>,
) -> Result<Option<PathBuf>, String> {
    let mut residuals = entries
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("could not inspect walker system temp: {error}"))?;
    residuals.sort();
    Ok(residuals.into_iter().next())
}

fn snapshot_path(
    path: &Path,
    paths: &mut PathMap,
    modes: &ObservationModeProvenance,
    output: &mut Vec<TreeRecord>,
) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    let mapped = paths.normalize_path(path);
    let normalized_path = mapped
        .strip_prefix(&format!("{}/", paths.profile_label))
        .expect("a tree root stays below the walker profile")
        .to_owned();
    let mode = modes.mode(path, &metadata)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(path).map_err(|error| error.to_string())?;
        output.push(TreeRecord {
            path: normalized_path,
            kind: "symlink".to_owned(),
            readonly: metadata.permissions().readonly(),
            mode,
            target: Some(paths.normalize_path(&target)),
            content: None,
        });
        return Ok(());
    }
    if metadata.is_file() {
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        output.push(TreeRecord {
            path: normalized_path,
            kind: "file".to_owned(),
            readonly: metadata.permissions().readonly(),
            mode,
            target: None,
            content: Some(paths.byte_view(path, &bytes)),
        });
        return Ok(());
    }
    output.push(TreeRecord {
        path: normalized_path,
        kind: "directory".to_owned(),
        readonly: metadata.permissions().readonly(),
        mode,
        target: None,
        content: None,
    });
    let mut children = fs::read_dir(path)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        snapshot_path(&child.path(), paths, modes, output)?;
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn portable_mode(metadata: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt as _;
    Some(metadata.permissions().mode() & 0o7777)
}

#[cfg(not(unix))]
pub(super) fn portable_mode(_metadata: &fs::Metadata) -> Option<u32> {
    None
}

#[cfg(unix)]
pub(super) fn escaped_os(value: &OsStr) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    value
        .as_bytes()
        .iter()
        .flat_map(|byte| std::ascii::escape_default(*byte).map(char::from))
        .collect()
}

pub(super) fn escaped_wide_units(units: &[u16]) -> String {
    use std::fmt::Write as _;

    // Windows reserves `\` as a separator, so `\uXXXX` cannot collide with a literal path
    // component. Fixed-width units also keep distinct unpaired surrogates injective.
    let mut escaped = String::with_capacity(units.len().saturating_mul(6));
    for unit in units {
        write!(escaped, "\\u{unit:04x}").expect("writing to a String cannot fail");
    }
    escaped
}

#[cfg(windows)]
pub(super) fn escaped_os(value: &OsStr) -> String {
    use std::os::windows::ffi::OsStrExt as _;

    value.to_str().map_or_else(
        || escaped_wide_units(&value.encode_wide().collect::<Vec<_>>()),
        str::to_owned,
    )
}

#[cfg(not(any(unix, windows)))]
pub(super) fn escaped_os(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

pub(super) fn escaped_path(path: &Path) -> String {
    path.components()
        .map(|component| escaped_os(component.as_os_str()))
        .collect::<Vec<_>>()
        .join("/")
}

pub(super) fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

pub(super) fn readable_plan_files(plan: &LaunchPlan) -> Vec<(PathBuf, Vec<u8>)> {
    std::iter::once(plan.program.as_os_str())
        .chain(plan.args.iter().map(OsStr::new))
        .filter_map(|argument| {
            let path = PathBuf::from(argument);
            fs::read(&path).ok().map(|bytes| (path, bytes))
        })
        .collect()
}

pub(super) fn byte_value(bytes: &[u8]) -> Value {
    match std::str::from_utf8(bytes) {
        Ok(text) => json!({ "encoding": "utf8", "data": text }),
        Err(_) => json!({ "encoding": "hex", "data": encode_hex(bytes) }),
    }
}

pub(super) fn io_error_value(error: &io::Error) -> Value {
    json!({
        "error": {
            "kind": format!("{:?}", error.kind()),
            "reason": error.to_string(),
        }
    })
}
