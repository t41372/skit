//! Frontend-neutral add, source-review, and onboarding state.

use std::{
    cmp::Reverse,
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use skit_application::{
    CreateEntry, EntryPayload, SourceIdentity, SourcePermissions, add_workdir, payload_stored_name,
};
use skit_domain::{EntryKind, EntrySettings, StorageMode, parameters::ParamDecl};
use skit_form::{CliFormProjection, OnboardingPlan, onboarding_plan};
use skit_language::{
    BindingIdentity, LosslessSource, UvMetadata, external_dependencies_at,
    has_uv_metadata_block_bytes, infer_draft_kind, infer_kind, placeholder_params,
    python_version_pin, read_uv_metadata, shebang_program, suggest_description,
    validate_pep440_specifiers, validate_pep508_requirement, write_managed_params_bytes,
    write_uv_metadata_bytes,
};

use crate::picker::{ChoicePicker, PickerItem, PickerMode};

/// Maximum draft rows shown before the UI reports an explicit overflow count.
pub const DRAFTS_LISTED: usize = 20;
/// Prompt placeholder count after which onboarding starts with no managed names.
pub const PROMPT_AUTO_MANAGE_LIMIT: usize = 30;
/// Maximum inline prompt-placeholder preview. A searchable picker exposes the rest.
pub const PROMPT_LIST_PREVIEW_LIMIT: usize = 20;

/// One parser-supported or direct-launch entry kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KnownEntryKind {
    /// Python source.
    Python,
    /// POSIX-family shell source.
    Shell,
    /// fish source.
    Fish,
    /// JavaScript source.
    JavaScript,
    /// TypeScript source.
    TypeScript,
    /// PowerShell source.
    PowerShell,
    /// Ruby source.
    Ruby,
    /// Perl source.
    Perl,
    /// Lua source.
    Lua,
    /// R source.
    R,
    /// Directly launched program or directory-shaped application.
    Executable,
    /// Prompt body.
    Prompt,
}

impl KnownEntryKind {
    /// Return the stable registry spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Shell => "shell",
            Self::Fish => "fish",
            Self::JavaScript => "js",
            Self::TypeScript => "ts",
            Self::PowerShell => "powershell",
            Self::Ruby => "ruby",
            Self::Perl => "perl",
            Self::Lua => "lua",
            Self::R => "r",
            Self::Executable => "exe",
            Self::Prompt => "prompt",
        }
    }

    /// Convert a registry spelling without accepting an unknown kind by accident.
    #[must_use]
    pub fn from_registry_str(value: &str) -> Option<Self> {
        match value {
            "python" => Some(Self::Python),
            "shell" => Some(Self::Shell),
            "fish" => Some(Self::Fish),
            "js" => Some(Self::JavaScript),
            "ts" => Some(Self::TypeScript),
            "powershell" => Some(Self::PowerShell),
            "ruby" => Some(Self::Ruby),
            "perl" => Some(Self::Perl),
            "lua" => Some(Self::Lua),
            "r" => Some(Self::R),
            "exe" => Some(Self::Executable),
            "prompt" => Some(Self::Prompt),
            _ => None,
        }
    }

    /// Return kind-pick choices in the latest-main display order.
    #[must_use]
    pub fn picker_choices(offer_executable: bool) -> Vec<Self> {
        let mut choices = vec![
            Self::Fish,
            Self::JavaScript,
            Self::Lua,
            Self::Perl,
            Self::PowerShell,
            Self::Python,
            Self::R,
            Self::Ruby,
            Self::Shell,
            Self::TypeScript,
        ];
        if offer_executable {
            choices.push(Self::Executable);
        }
        choices.push(Self::Prompt);
        choices
    }
}

/// One kept draft. The host supplies stat results once and the reducer keeps their order stable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DraftSummary {
    /// Draft path.
    pub path: PathBuf,
    /// Last-modified timestamp as a sortable host value.
    pub modified: u64,
    /// Host-captured file incarnation. Legacy state and unsupported hosts have no identity.
    #[serde(default)]
    pub identity: Option<SourceIdentity>,
    /// Permissions captured with the row so deletion detects an in-place mode change.
    #[serde(default)]
    pub permissions: SourcePermissions,
}

/// Result of one identity-checked kept-draft deletion.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftDeleteOutcome {
    /// The exact claimed file was removed.
    Removed,
    /// The claimed path was already absent.
    AlreadyMissing,
    /// The path now names a different file. The host returns its refreshed row.
    Changed(DraftSummary),
}

/// A byte-exact source snapshot captured before review.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceSnapshot {
    /// Canonical or otherwise host-resolved source path.
    pub path: PathBuf,
    /// Provenance stored in entry metadata.
    pub source_record: String,
    /// Exact source bytes read for the transaction.
    pub bytes: Vec<u8>,
    /// Permissions stored with a copy payload.
    pub permissions: SourcePermissions,
    /// Whether the snapshot is a regular file.
    pub is_regular: bool,
    /// Whether the source is a directory-shaped executable.
    pub is_directory: bool,
    /// Whether this is skit's only kept copy of authored work.
    pub is_draft: bool,
    /// Host-captured file incarnation. Legacy state and unsupported hosts have no identity.
    #[serde(default)]
    pub identity: Option<SourceIdentity>,
}

impl SourceSnapshot {
    fn text(&self) -> LosslessSource {
        LosslessSource::from_bytes(&self.bytes)
    }

    fn inferred_kind(&self) -> Option<KnownEntryKind> {
        if self.is_directory && !self.is_draft {
            return Some(KnownEntryKind::Executable);
        }
        let text = self.text();
        let shebang = text
            .normalized_text()
            .lines()
            .next()
            .filter(|line| line.starts_with("#!"));
        let inferred = if self.is_draft {
            infer_draft_kind(&self.path, shebang, self.is_executable())
        } else {
            infer_kind(&self.path, shebang, self.is_executable())
        };
        inferred
            .and_then(KnownEntryKind::from_registry_str)
            .filter(|kind| !(self.is_draft && *kind == KnownEntryKind::Executable))
    }

    fn is_executable(&self) -> bool {
        self.permissions
            .unix_mode
            .is_some_and(|mode| mode & 0o111 != 0)
    }

    fn default_name(&self, kind: KnownEntryKind) -> String {
        let stem = self
            .path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("entry");
        if kind == KnownEntryKind::Prompt {
            stem.strip_suffix(".prompt").unwrap_or(stem).to_owned()
        } else {
            stem.to_owned()
        }
    }
}

/// Monotonic identity for an asynchronous host operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddRequestId(u64);

/// Active add surface.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddStage {
    /// Choose a source, command template, or kept draft.
    #[default]
    Source,
    /// Classify an ambiguous text file.
    Kind,
    /// Review all fields before the first write.
    Review,
    /// Confirm deletion of the user's only kept draft copy.
    ConfirmDraftDelete,
    /// A completed create request returned a slug.
    Complete,
    /// The user left without creating an entry.
    Cancelled,
}

/// Typed add-time problem. Renderers localize the stable variants.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddProblem {
    /// A path was not found or could not be inspected.
    SourceUnavailable { path: PathBuf, reason: String },
    /// Command templates require an explicit display name.
    MissingCommandName,
    /// A host returned a malformed or empty kind selection.
    InvalidKind,
    /// A prompt must be valid UTF-8.
    InvalidPromptEncoding,
    /// One PEP 508 requirement is invalid.
    InvalidDependency { value: String },
    /// A requires-python field is not a PEP 440 specifier list.
    InvalidPythonConstraint { value: String },
    /// A lossless source edit could not be planned.
    SourceEdit { reason: String },
    /// The repository refused the atomic create request.
    CommitFailed { reason: String },
    /// The editor could not return a readable snapshot.
    EditFailed { reason: String },
    /// Draft deletion failed.
    DraftDeleteFailed { reason: String },
    /// The selected draft changed and the host refreshed its row.
    DraftChanged { path: PathBuf },
}

/// Non-error feedback from one completed add or draft action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddNotice {
    /// The editor returned unchanged starter content, so no entry was created.
    NothingWritten,
    /// A cancelled review left the user's only authored copy at this path.
    DraftKept(PathBuf),
    /// One confirmed kept draft was deleted.
    DraftDeleted(PathBuf),
}

/// The kind picker opened for one inspected source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KindPickerState {
    filename: String,
    has_shebang: bool,
    choices: Vec<KnownEntryKind>,
    suggested: Option<KnownEntryKind>,
}

impl KindPickerState {
    fn for_source(source: &SourceSnapshot) -> Self {
        let text = source.text();
        let has_shebang = text
            .normalized_text()
            .lines()
            .next()
            .and_then(shebang_program)
            .is_some();
        let suggested = source
            .path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            .then_some(KnownEntryKind::Prompt);
        Self {
            filename: source
                .path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_owned(),
            has_shebang,
            choices: KnownEntryKind::picker_choices(!source.is_draft),
            suggested,
        }
    }

    /// Ambiguous filename shown in the question.
    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Whether the question must explain an unsupported shebang.
    #[must_use]
    pub const fn has_shebang(&self) -> bool {
        self.has_shebang
    }

    /// Ordered typed choices.
    #[must_use]
    pub fn choices(&self) -> &[KnownEntryKind] {
        &self.choices
    }

    /// Likely choice that starts highlighted.
    #[must_use]
    pub const fn suggested(&self) -> Option<KnownEntryKind> {
        self.suggested
    }

    /// Return whether one typed choice is legal for this source.
    #[must_use]
    pub fn offers(&self, kind: KnownEntryKind) -> bool {
        self.choices.contains(&kind)
    }
}

/// Add-source values. A renderer maps each field to a mature input widget.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddSourceState {
    /// Script, executable, prompt, or draft path.
    pub path: String,
    /// Optional command template lane.
    pub command_template: String,
    /// Required command display name.
    pub command_name: String,
    /// Optional command description.
    pub command_description: String,
    drafts: Vec<DraftSummary>,
    /// Highlighted draft row.
    pub selected_draft: Option<usize>,
}

impl AddSourceState {
    /// Create a newest-first, regular-file-only draft projection.
    #[must_use]
    pub fn new(mut drafts: Vec<DraftSummary>) -> Self {
        drafts.sort_by_key(|draft| Reverse(draft.modified));
        Self {
            drafts,
            ..Self::default()
        }
    }

    /// Draft rows that fit the inline list.
    #[must_use]
    pub fn listed_drafts(&self) -> &[DraftSummary] {
        &self.drafts[..self.drafts.len().min(DRAFTS_LISTED)]
    }

    /// Count that must be shown rather than silently hidden.
    #[must_use]
    pub fn draft_overflow(&self) -> usize {
        self.drafts.len().saturating_sub(DRAFTS_LISTED)
    }

    /// Highlighted kept draft.
    #[must_use]
    pub fn selected_draft(&self) -> Option<&DraftSummary> {
        self.selected_draft.and_then(|index| self.drafts.get(index))
    }

    fn remove_draft(&mut self, path: &Path) {
        self.drafts.retain(|draft| draft.path != path);
        self.selected_draft = self
            .selected_draft
            .filter(|index| *index < self.drafts.len());
    }

    fn replace_draft(&mut self, path: &Path, refreshed: DraftSummary) {
        self.remove_draft(path);
        self.drafts.push(refreshed.clone());
        self.drafts.sort_by_key(|draft| Reverse(draft.modified));
        self.selected_draft = self
            .drafts
            .iter()
            .position(|draft| draft.path == refreshed.path);
    }
}

/// Values that a CLI-hosted review can prefill without making any field immutable.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReviewDefaults {
    /// Display name override.
    pub name: Option<String>,
    /// Description override.
    pub description: Option<String>,
    /// Select reference mode when the source is not a draft.
    pub reference: bool,
    /// Explicit dependency list.
    pub dependencies: Vec<String>,
    /// Explicit Python constraint.
    pub requires_python: Option<String>,
    /// Prompt runner pin.
    pub runner: Option<String>,
    /// Prompt runner choices in stored order.
    pub runner_names: Vec<String>,
    /// Last runner selected interactively.
    pub last_runner: Option<String>,
    /// Prompt interpolation master switch.
    pub interpolate: Option<bool>,
}

/// Package-dependency controls that the chosen language supports.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencySurface {
    /// The language has no managed package-dependency lane.
    None,
    /// Editable PEP 508 dependencies and PEP 440 Python constraint.
    Python,
    /// An existing PEP 723 fence is the only authority and is read-only at add time.
    PythonOwned(UvMetadata),
    /// Editable npm dependency list.
    Npm,
}

/// One parser-backed onboarding checkbox.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReviewCandidate {
    /// Complete declaration to write if selected.
    pub declaration: ParamDecl,
    /// Stable parser identity used across edit and rescan.
    pub identity: BindingIdentity,
    /// Whether the checkbox is selected.
    pub selected: bool,
    /// Whether parser semantics demoted this candidate.
    pub demoted: bool,
}

/// One prompt placeholder checkbox.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PromptCandidate {
    /// Placeholder name.
    pub name: String,
    /// Whether this placeholder becomes a run-form field.
    pub selected: bool,
    /// Whether the form must treat the value as secret.
    pub secret: bool,
}

/// Review-lane shape. Controls depend on this enum, never a user-visible string.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewLane {
    /// Parser-backed interpreted source.
    Script,
    /// Direct executable identity review.
    Executable,
    /// Prompt insertion and runner review.
    Prompt,
}

/// One complete pre-commit review.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReviewState {
    source: SourceSnapshot,
    kind: KnownEntryKind,
    lane: ReviewLane,
    name: String,
    description: String,
    storage: StorageMode,
    dependencies_text: String,
    dependencies_edited: bool,
    requires_python: String,
    python_is_auto: bool,
    dependency_surface: DependencySurface,
    interpreter: String,
    onboarding: OnboardingPlan,
    candidates: Vec<ReviewCandidate>,
    prompt_candidates: Vec<PromptCandidate>,
    prompt_flooded: bool,
    interpolate: bool,
    runner: String,
    runner_names: Vec<String>,
    runner_was_picked: bool,
    source_problem: Option<AddProblem>,
}

impl ReviewState {
    /// Analyze one byte-exact source snapshot once and project every review surface from it.
    #[must_use]
    pub fn from_source(
        source: SourceSnapshot,
        kind: KnownEntryKind,
        defaults: ReviewDefaults,
    ) -> Self {
        let lane = match kind {
            KnownEntryKind::Executable => ReviewLane::Executable,
            KnownEntryKind::Prompt => ReviewLane::Prompt,
            _ => ReviewLane::Script,
        };
        let fresh = source.is_draft;
        let storage = if fresh {
            StorageMode::Copy
        } else if kind == KnownEntryKind::Executable || defaults.reference {
            StorageMode::Reference
        } else {
            StorageMode::Copy
        };
        let name = defaults
            .name
            .clone()
            .unwrap_or_else(|| source.default_name(kind));
        let description = defaults
            .description
            .clone()
            .unwrap_or_else(|| suggest_description(kind.as_str(), &source.bytes));
        let text = source.text();
        let normalized = text.normalized_text();
        let onboarding = if lane == ReviewLane::Script {
            onboarding_plan(kind.as_str(), normalized)
        } else {
            OnboardingPlan::default()
        };
        let candidates = onboarding
            .offered_candidates()
            .iter()
            .map(|candidate| ReviewCandidate {
                declaration: candidate.declaration.clone(),
                identity: candidate.identity.clone(),
                selected: candidate.selected_by_default(),
                demoted: candidate.demotion.is_some(),
            })
            .collect();
        let own_uv = kind == KnownEntryKind::Python && has_uv_metadata_block_bytes(&source.bytes);
        let dependency_surface = match kind {
            KnownEntryKind::Python if own_uv => {
                DependencySurface::PythonOwned(read_uv_metadata(normalized).unwrap_or_default())
            }
            KnownEntryKind::Python => DependencySurface::Python,
            KnownEntryKind::JavaScript | KnownEntryKind::TypeScript => DependencySurface::Npm,
            _ => DependencySurface::None,
        };
        let suggested_dependencies = if matches!(
            dependency_surface,
            DependencySurface::Python | DependencySurface::Npm
        ) {
            external_dependencies_at(kind.as_str(), normalized, source.path.parent())
        } else {
            Vec::new()
        };
        let dependencies_text = join_dependencies(
            &dependency_surface,
            if defaults.dependencies.is_empty() {
                &suggested_dependencies
            } else {
                &defaults.dependencies
            },
        );
        let shebang = normalized
            .lines()
            .next()
            .filter(|line| line.starts_with("#!"))
            .and_then(shebang_program);
        let auto_python = (kind == KnownEntryKind::Python && !own_uv)
            .then(|| shebang.and_then(python_version_pin))
            .flatten()
            .unwrap_or_default();
        let requires_python = defaults
            .requires_python
            .clone()
            .unwrap_or_else(|| auto_python.clone());
        let python_is_auto = defaults.requires_python.is_none();
        let interpreter = shebang
            .filter(|_| {
                !matches!(
                    kind,
                    KnownEntryKind::Python | KnownEntryKind::Prompt | KnownEntryKind::Executable
                ) && infer_kind(Path::new("source"), normalized.lines().next(), false)
                    == Some(kind.as_str())
            })
            .unwrap_or_default()
            .to_owned();
        let (prompt_candidates, prompt_flooded, source_problem) = if lane == ReviewLane::Prompt {
            match String::from_utf8(source.bytes.clone()) {
                Ok(strict) => {
                    let declarations = placeholder_params("prompt", &strict);
                    let flooded = declarations.len() > PROMPT_AUTO_MANAGE_LIMIT;
                    (
                        declarations
                            .into_iter()
                            .map(|declaration| PromptCandidate {
                                name: declaration.name,
                                selected: !flooded,
                                secret: declaration.secret,
                            })
                            .collect(),
                        flooded,
                        None,
                    )
                }
                Err(_) => (Vec::new(), false, Some(AddProblem::InvalidPromptEncoding)),
            }
        } else {
            (Vec::new(), false, None)
        };
        let runner = defaults
            .runner
            .as_ref()
            .or(defaults.last_runner.as_ref())
            .filter(|runner| defaults.runner_names.contains(runner))
            .cloned()
            .unwrap_or_default();
        Self {
            source,
            kind,
            lane,
            name,
            description,
            storage,
            dependencies_text,
            dependencies_edited: !defaults.dependencies.is_empty(),
            requires_python,
            python_is_auto,
            dependency_surface,
            interpreter,
            onboarding,
            candidates,
            prompt_candidates,
            prompt_flooded,
            interpolate: defaults.interpolate.unwrap_or(true),
            runner,
            runner_names: defaults.runner_names,
            runner_was_picked: false,
            source_problem,
        }
    }

    /// Current byte-exact source snapshot.
    #[must_use]
    pub const fn source(&self) -> &SourceSnapshot {
        &self.source
    }

    /// Typed entry kind.
    #[must_use]
    pub const fn kind(&self) -> KnownEntryKind {
        self.kind
    }

    /// Review shape.
    #[must_use]
    pub const fn lane(&self) -> ReviewLane {
        self.lane
    }

    /// Whether storage controls must be withheld because this is the user's only draft copy.
    #[must_use]
    pub const fn is_fresh(&self) -> bool {
        self.source.is_draft
    }

    /// Display name field.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Description field.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Storage selection.
    #[must_use]
    pub const fn storage(&self) -> StorageMode {
        self.storage
    }

    /// Parser-backed onboarding plan used by hints and candidate controls.
    #[must_use]
    pub const fn onboarding(&self) -> &OnboardingPlan {
        &self.onboarding
    }

    /// Candidate checkboxes. A modeled nonempty CLI surface makes this list empty.
    #[must_use]
    pub fn candidates(&self) -> &[ReviewCandidate] {
        &self.candidates
    }

    /// Find a candidate by its stable form name.
    #[must_use]
    pub fn candidate(&self, name: &str) -> Option<&ReviewCandidate> {
        self.candidates
            .iter()
            .find(|candidate| candidate.declaration.name == name)
    }

    /// Static reflected field count. `Some(0)` stays distinct from absence and dynamic state.
    #[must_use]
    pub fn modeled_cli_field_count(&self) -> Option<usize> {
        match &self.onboarding.cli_surface {
            CliFormProjection::Static { fields, .. } => Some(fields.len()),
            CliFormProjection::Absent | CliFormProjection::Dynamic { .. } => None,
        }
    }

    /// Dependency-control shape.
    #[must_use]
    pub const fn dependency_surface(&self) -> &DependencySurface {
        &self.dependency_surface
    }

    /// Dependency editor content.
    #[must_use]
    pub fn dependencies_text(&self) -> &str {
        &self.dependencies_text
    }

    /// Python constraint editor content.
    #[must_use]
    pub fn requires_python(&self) -> &str {
        &self.requires_python
    }

    /// Prompt interpolation state.
    #[must_use]
    pub const fn interpolate(&self) -> bool {
        self.interpolate
    }

    /// Available prompt runner names.
    #[must_use]
    pub fn runner_names(&self) -> &[String] {
        &self.runner_names
    }

    /// Selected prompt runner. Empty means ask on the run form.
    #[must_use]
    pub fn runner(&self) -> &str {
        &self.runner
    }

    /// Whether a prompt runner was actively picked and should become the next picker default.
    #[must_use]
    pub const fn runner_was_picked(&self) -> bool {
        self.runner_was_picked
    }

    /// Whether placeholder detection exceeded the honest auto-management limit.
    #[must_use]
    pub const fn prompt_is_flooded(&self) -> bool {
        self.prompt_flooded
    }

    /// Inline prompt candidate preview.
    #[must_use]
    pub fn prompt_preview(&self) -> &[PromptCandidate] {
        &self.prompt_candidates[..self.prompt_candidates.len().min(PROMPT_LIST_PREVIEW_LIMIT)]
    }

    /// Complete prompt candidate list for the searchable picker.
    #[must_use]
    pub fn prompt_candidates(&self) -> &[PromptCandidate] {
        &self.prompt_candidates
    }

    /// Selected placeholder names in source order.
    #[must_use]
    pub fn selected_prompt_names(&self) -> Vec<&str> {
        self.prompt_candidates
            .iter()
            .filter(|candidate| candidate.selected)
            .map(|candidate| candidate.name.as_str())
            .collect()
    }

    /// Build the complete searchable prompt-variable picker from current name-keyed choices.
    #[must_use]
    pub fn prompt_picker(&self) -> ChoicePicker<String> {
        ChoicePicker::new(
            PickerMode::Multiple,
            self.prompt_candidates
                .iter()
                .map(|candidate| PickerItem::new(candidate.name.clone(), candidate.name.clone()))
                .collect(),
            self.prompt_candidates
                .iter()
                .filter(|candidate| candidate.selected)
                .map(|candidate| candidate.name.clone())
                .collect(),
        )
    }

    /// Replace the display name without changing detected source facts.
    pub fn set_name(&mut self, value: impl Into<String>) {
        self.name = value.into();
    }

    /// Replace the description without changing detected source facts.
    pub fn set_description(&mut self, value: impl Into<String>) {
        self.description = value.into();
    }

    /// Select storage. Drafts and executables keep their mandatory mode.
    pub fn set_storage(&mut self, mode: StorageMode) {
        self.storage = if self.source.is_draft {
            StorageMode::Copy
        } else if self.kind == KnownEntryKind::Executable {
            StorageMode::Reference
        } else {
            mode
        };
    }

    /// Replace the package field.
    pub fn set_dependencies_text(&mut self, value: impl Into<String>) {
        if !matches!(self.dependency_surface, DependencySurface::PythonOwned(_)) {
            self.dependencies_text = value.into();
            self.dependencies_edited = true;
        }
    }

    /// Replace the Python constraint and stop future shebang rescans from overriding it.
    pub fn set_requires_python(&mut self, value: impl Into<String>) {
        self.requires_python = normalize_python_automatic(value.into());
        self.python_is_auto = false;
    }

    /// Toggle one parser-backed candidate by stable form name.
    pub fn set_candidate_selected(&mut self, name: &str, selected: bool) {
        if let Some(candidate) = self
            .candidates
            .iter_mut()
            .find(|candidate| candidate.declaration.name == name)
        {
            candidate.selected = selected;
        }
    }

    /// Toggle prompt insertion without losing hidden candidate choices.
    pub fn set_interpolate(&mut self, value: bool) {
        self.interpolate = value;
    }

    /// Toggle one prompt placeholder by name.
    pub fn set_prompt_selected(&mut self, name: &str, selected: bool) {
        if let Some(candidate) = self
            .prompt_candidates
            .iter_mut()
            .find(|candidate| candidate.name == name)
        {
            candidate.selected = selected;
        }
    }

    /// Replace all prompt choices from the complete searchable picker.
    pub fn set_prompt_selection(&mut self, selected: &[String]) {
        for candidate in &mut self.prompt_candidates {
            candidate.selected = selected.contains(&candidate.name);
        }
    }

    /// Select a runner by value rather than by a display index.
    pub fn set_runner(&mut self, runner: &str, picked: bool) {
        if runner.is_empty() || self.runner_names.iter().any(|name| name == runner) {
            self.runner = runner.to_owned();
            self.runner_was_picked |= picked && !runner.is_empty();
        }
    }

    /// Add one runner returned by the runner editor and select it immediately.
    pub fn add_runner(&mut self, runner: String) {
        if !self.runner_names.contains(&runner) {
            self.runner_names.push(runner.clone());
        }
        self.runner = runner;
        self.runner_was_picked = true;
    }

    /// Replace source bytes after the editor returns and rescan one parser session.
    ///
    /// Typed fields stay unchanged. Candidate decisions follow parser binding identity, with a
    /// name fallback for adapters whose stable identity legitimately changes after an edit.
    pub fn rescan(&mut self, bytes: Vec<u8>) {
        let old_candidates = self
            .candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.identity.clone(),
                    candidate.declaration.name.clone(),
                    candidate.selected,
                )
            })
            .collect::<Vec<_>>();
        let old_prompts = self
            .prompt_candidates
            .iter()
            .map(|candidate| (candidate.name.clone(), candidate.selected))
            .collect::<BTreeMap<_, _>>();
        self.source.bytes = bytes;
        let text = self.source.text();
        let normalized = text.normalized_text();
        self.source_problem = None;
        if self.lane == ReviewLane::Script {
            self.onboarding = onboarding_plan(self.kind.as_str(), normalized);
            self.candidates = self
                .onboarding
                .offered_candidates()
                .iter()
                .map(|candidate| {
                    let selected = old_candidates
                        .iter()
                        .find(|(identity, _, _)| identity == &candidate.identity)
                        .or_else(|| {
                            old_candidates
                                .iter()
                                .find(|(_, name, _)| name == &candidate.declaration.name)
                        })
                        .map_or_else(
                            || candidate.selected_by_default(),
                            |(_, _, selected)| *selected,
                        );
                    ReviewCandidate {
                        declaration: candidate.declaration.clone(),
                        identity: candidate.identity.clone(),
                        selected,
                        demoted: candidate.demotion.is_some(),
                    }
                })
                .collect();
            let own_uv = self.kind == KnownEntryKind::Python
                && has_uv_metadata_block_bytes(&self.source.bytes);
            self.dependency_surface = match self.kind {
                KnownEntryKind::Python if own_uv => {
                    DependencySurface::PythonOwned(read_uv_metadata(normalized).unwrap_or_default())
                }
                KnownEntryKind::Python => DependencySurface::Python,
                KnownEntryKind::JavaScript | KnownEntryKind::TypeScript => DependencySurface::Npm,
                _ => DependencySurface::None,
            };
            if own_uv {
                self.dependencies_text.clear();
            } else if !self.dependencies_edited
                && matches!(
                    self.dependency_surface,
                    DependencySurface::Python | DependencySurface::Npm
                )
            {
                self.dependencies_text = join_dependencies(
                    &self.dependency_surface,
                    &external_dependencies_at(
                        self.kind.as_str(),
                        normalized,
                        self.source.path.parent(),
                    ),
                );
            }
            if self.python_is_auto && self.kind == KnownEntryKind::Python {
                self.requires_python = if has_uv_metadata_block_bytes(&self.source.bytes) {
                    String::new()
                } else {
                    normalized
                        .lines()
                        .next()
                        .and_then(shebang_program)
                        .and_then(python_version_pin)
                        .unwrap_or_default()
                };
            }
            self.interpreter = normalized
                .lines()
                .next()
                .and_then(shebang_program)
                .filter(|_| {
                    !matches!(
                        self.kind,
                        KnownEntryKind::Python
                            | KnownEntryKind::Prompt
                            | KnownEntryKind::Executable
                    ) && infer_kind(Path::new("source"), normalized.lines().next(), false)
                        == Some(self.kind.as_str())
                })
                .unwrap_or_default()
                .to_owned();
        } else if self.lane == ReviewLane::Prompt {
            match String::from_utf8(self.source.bytes.clone()) {
                Ok(strict) => {
                    let declarations = placeholder_params("prompt", &strict);
                    self.prompt_flooded = declarations.len() > PROMPT_AUTO_MANAGE_LIMIT;
                    self.prompt_candidates = declarations
                        .into_iter()
                        .map(|declaration| PromptCandidate {
                            selected: old_prompts
                                .get(&declaration.name)
                                .copied()
                                .unwrap_or(!self.prompt_flooded),
                            name: declaration.name,
                            secret: declaration.secret,
                        })
                        .collect();
                }
                Err(_) => {
                    self.prompt_candidates.clear();
                    self.prompt_flooded = false;
                    self.source_problem = Some(AddProblem::InvalidPromptEncoding);
                }
            }
        }
    }

    /// Validate the whole panel and build the one atomic application request.
    pub fn create_entry(&self) -> Result<CreateEntry, AddProblem> {
        if let Some(problem) = &self.source_problem {
            return Err(problem.clone());
        }
        let kind =
            EntryKind::parse(self.kind.as_str().to_owned()).map_err(|_| AddProblem::InvalidKind)?;
        let mode = if self.source.is_draft {
            StorageMode::Copy
        } else if self.kind == KnownEntryKind::Executable {
            StorageMode::Reference
        } else {
            self.storage
        };
        let name = self.name.trim();
        let name = if name.is_empty() {
            self.source.default_name(self.kind)
        } else {
            name.to_owned()
        };
        let mut bytes = self.source.bytes.clone();
        let mut settings = EntrySettings::default();
        settings.interpreter.clone_from(&self.interpreter);
        match self.lane {
            ReviewLane::Executable => {}
            ReviewLane::Prompt => {
                settings.runner = self.runner.trim().to_owned();
                settings.interpolate = self.interpolate;
                if self.interpolate {
                    settings.params = self
                        .prompt_candidates
                        .iter()
                        .filter(|candidate| candidate.selected)
                        .map(|candidate| candidate.name.clone())
                        .collect();
                }
            }
            ReviewLane::Script => {
                let dependencies = self.dependencies()?;
                let python = normalize_python_automatic(self.requires_python.clone());
                if self.kind == KnownEntryKind::Python
                    && !matches!(self.dependency_surface, DependencySurface::PythonOwned(_))
                {
                    for dependency in &dependencies {
                        validate_pep508_requirement(dependency).map_err(|_| {
                            AddProblem::InvalidDependency {
                                value: dependency.clone(),
                            }
                        })?;
                    }
                    if !python.is_empty() {
                        validate_pep440_specifiers(&python).map_err(|_| {
                            AddProblem::InvalidPythonConstraint {
                                value: python.clone(),
                            }
                        })?;
                    }
                }
                if mode == StorageMode::Copy
                    && self.kind == KnownEntryKind::Python
                    && !matches!(self.dependency_surface, DependencySurface::PythonOwned(_))
                    && (!dependencies.is_empty() || !python.is_empty())
                {
                    // `PythonOwned` is the only existing-block case. Validated scalar values make
                    // insertion into a source without a block infallible.
                    bytes = write_uv_metadata_bytes(&bytes, &dependencies, &python)
                        .expect("validated metadata without an existing block must render");
                } else if !matches!(self.dependency_surface, DependencySurface::PythonOwned(_)) {
                    settings.dependencies = if mode == StorageMode::Reference
                        && matches!(
                            self.kind,
                            KnownEntryKind::JavaScript | KnownEntryKind::TypeScript
                        ) {
                        Vec::new()
                    } else {
                        dependencies
                    };
                    if self.kind == KnownEntryKind::Python {
                        settings.requires_python = python;
                    }
                }
                if mode == StorageMode::Copy {
                    let declarations = self
                        .candidates
                        .iter()
                        .filter(|candidate| candidate.selected)
                        .map(|candidate| candidate.declaration.clone())
                        .collect::<Vec<_>>();
                    if !declarations.is_empty() {
                        bytes =
                            write_managed_params_bytes(self.kind.as_str(), &bytes, &declarations)
                                .map_err(|error| AddProblem::SourceEdit {
                                reason: error.to_string(),
                            })?;
                    }
                }
            }
        }
        let stored_name = payload_stored_name(&kind, &self.source.path);
        let workdir = add_workdir(&kind, mode).to_owned();
        let payload = if self.kind == KnownEntryKind::Executable && !self.source.is_regular {
            None
        } else {
            Some(EntryPayload {
                bytes,
                stored_name: Some(stored_name),
                permissions: self.source.permissions,
            })
        };
        Ok(CreateEntry {
            name,
            kind,
            mode,
            source: self.source.source_record.clone(),
            workdir,
            description: self.description.trim().to_owned(),
            payload,
            settings,
        })
    }

    fn dependencies(&self) -> Result<Vec<String>, AddProblem> {
        match self.dependency_surface {
            DependencySurface::None | DependencySurface::PythonOwned(_) => Ok(Vec::new()),
            DependencySurface::Npm => Ok(self
                .dependencies_text
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()),
            DependencySurface::Python => Ok(split_pep508_requirements(&self.dependencies_text)),
        }
    }
}

fn join_dependencies(surface: &DependencySurface, dependencies: &[String]) -> String {
    match surface {
        DependencySurface::None | DependencySurface::PythonOwned(_) => String::new(),
        DependencySurface::Python | DependencySurface::Npm => dependencies.join(", "),
    }
}

fn normalize_python_automatic(value: String) -> String {
    let trimmed = value.trim();
    if trimmed == "-" || trimmed.eq_ignore_ascii_case("none") {
        String::new()
    } else {
        trimmed.to_owned()
    }
}

/// Split a comma-composed field by asking the mature PEP 508 parser which partitions are valid.
/// This keeps commas inside specifiers, extras, markers, and URLs intact.
pub(crate) fn split_pep508_requirements(value: &str) -> Vec<String> {
    let value = value.trim();
    if value.is_empty() {
        return Vec::new();
    }
    let comma_offsets = value
        .char_indices()
        .filter_map(|(index, character)| (character == ',').then_some(index))
        .chain(std::iter::once(value.len()))
        .collect::<Vec<_>>();
    fn partition(value: &str, offsets: &[usize], start: usize) -> Option<Vec<String>> {
        for &end in offsets.iter().filter(|&&offset| offset >= start) {
            let item = value[start..end].trim();
            if item.is_empty() || validate_pep508_requirement(item).is_err() {
                continue;
            }
            if end == value.len() {
                return Some(vec![item.to_owned()]);
            }
            let next = end.saturating_add(1);
            if let Some(mut tail) = partition(value, offsets, next) {
                let mut output = vec![item.to_owned()];
                output.append(&mut tail);
                return Some(output);
            }
        }
        None
    }
    partition(value, &comma_offsets, 0).unwrap_or_else(|| vec![value.to_owned()])
}

/// New authored source kind.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    /// Starter script that can be reclassified from its edited shebang.
    Script,
    /// Starter prompt.
    Prompt,
}

/// Reducer input. Every control identity is typed.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddAction {
    /// Replace the source path field.
    SetSourcePath(String),
    /// Replace the command template field.
    SetCommandTemplate(String),
    /// Replace the command name field.
    SetCommandName(String),
    /// Replace the command description field.
    SetCommandDescription(String),
    /// Highlight a kept draft row.
    SelectDraft(usize),
    /// Continue from the source surface.
    Continue,
    /// Return one byte-exact host inspection.
    SourceInspected {
        /// Matching operation identity.
        request: AddRequestId,
        /// Snapshot or typed host detail.
        result: Result<SourceSnapshot, String>,
    },
    /// Choose or cancel an ambiguous kind.
    PickKind(Option<KnownEntryKind>),
    /// Start a new authored file.
    NewDraft(DraftKind),
    /// Return the authored draft snapshot. `None` means unchanged scaffolding.
    DraftEdited {
        /// Matching operation identity.
        request: AddRequestId,
        /// Edited snapshot, unchanged starter, or editor failure.
        result: Result<Option<SourceSnapshot>, String>,
    },
    /// Request deletion confirmation for the highlighted kept draft.
    DeleteSelectedDraft,
    /// Confirm or cancel kept draft deletion.
    ConfirmDraftDelete(bool),
    /// Return a kept draft deletion result.
    DraftDeleted {
        /// Matching operation identity.
        request: AddRequestId,
        /// Host result.
        result: Result<DraftDeleteOutcome, String>,
    },
    /// Replace review name.
    SetReviewName(String),
    /// Replace review description.
    SetReviewDescription(String),
    /// Select copy or reference storage.
    SetReviewStorage(StorageMode),
    /// Replace dependency field.
    SetReviewDependencies(String),
    /// Replace Python constraint field.
    SetReviewPython(String),
    /// Toggle one parser candidate.
    SetReviewCandidate { name: String, selected: bool },
    /// Toggle prompt interpolation.
    SetPromptInterpolation(bool),
    /// Toggle one prompt placeholder.
    SetPromptCandidate { name: String, selected: bool },
    /// Replace complete prompt placeholder selection.
    SetPromptCandidates(Vec<String>),
    /// Select a prompt runner.
    SetPromptRunner { name: String, picked: bool },
    /// Add a runner returned by the runner editor.
    PromptRunnerAdded(String),
    /// Open the current source in the configured editor.
    EditSource,
    /// Return edited source bytes for parser rescan.
    SourceEdited {
        /// Matching operation identity.
        request: AddRequestId,
        /// Refreshed byte-exact source facts or editor failure.
        result: Result<SourceSnapshot, String>,
    },
    /// Validate and emit the first repository write.
    Save,
    /// Return the atomic repository result.
    CommitFinished {
        /// Matching operation identity.
        request: AddRequestId,
        /// Created slug or repository failure.
        result: Result<String, String>,
    },
    /// Leave the current surface.
    Cancel,
}

/// Side effect requested from an application host.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AddEffect {
    /// Resolve and read one source without mutating it.
    InspectSource {
        request: AddRequestId,
        path: PathBuf,
    },
    /// Create starter content in the drafts directory and open the editor.
    AuthorDraft {
        request: AddRequestId,
        kind: DraftKind,
    },
    /// Delete one confirmed kept draft.
    DeleteDraft {
        request: AddRequestId,
        draft: DraftSummary,
    },
    /// Open the current source and return replacement bytes.
    EditSource {
        request: AddRequestId,
        path: PathBuf,
    },
    /// Commit one complete entry through `LibraryService::add`.
    Commit {
        request: AddRequestId,
        entry: Box<CreateEntry>,
        /// Byte-exact source facts that the host must recheck before it commits.
        source: Option<SourceSnapshot>,
    },
    /// Delete a draft only after a copy entry committed successfully.
    ConsumeDraft(SourceSnapshot),
    /// Tell the host that cancelled authored work remains in the draft list.
    DraftKept(PathBuf),
    /// Remember an actively selected prompt runner for the next interactive picker.
    RememberRunner(String),
    /// Close the host with the new slug.
    Complete(String),
    /// Close without a new entry.
    Cancel,
}

/// Complete add reducer. It never performs file or repository I/O itself.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AddWorkflowState {
    stage: AddStage,
    source: AddSourceState,
    review_defaults: ReviewDefaults,
    kind_picker: Option<KindPickerState>,
    pending_source: Option<SourceSnapshot>,
    review: Option<ReviewState>,
    problem: Option<AddProblem>,
    notice: Option<AddNotice>,
    next_request: u64,
    pending_inspection: Option<AddRequestId>,
    pending_edit: Option<AddRequestId>,
    pending_draft: Option<AddRequestId>,
    pending_delete: Option<(AddRequestId, DraftSummary)>,
    delete_candidate: Option<DraftSummary>,
    pending_commit: Option<AddRequestId>,
    standalone_review: bool,
}

impl AddWorkflowState {
    /// Start at the source surface.
    #[must_use]
    pub fn new(drafts: Vec<DraftSummary>) -> Self {
        Self {
            stage: AddStage::Source,
            source: AddSourceState::new(drafts),
            review_defaults: ReviewDefaults::default(),
            kind_picker: None,
            pending_source: None,
            review: None,
            problem: None,
            notice: None,
            next_request: 0,
            pending_inspection: None,
            pending_edit: None,
            pending_draft: None,
            pending_delete: None,
            delete_candidate: None,
            pending_commit: None,
            standalone_review: false,
        }
    }

    /// Host one review directly for the interactive CLI twin.
    #[must_use]
    pub fn from_review(review: ReviewState) -> Self {
        let mut state = Self::new(Vec::new());
        state.stage = AddStage::Review;
        state.review = Some(review);
        state.standalone_review = true;
        state
    }

    /// Supply the reusable review defaults before any source is inspected.
    #[must_use]
    pub fn with_review_defaults(mut self, defaults: ReviewDefaults) -> Self {
        self.review_defaults = defaults;
        self
    }

    /// Active surface.
    #[must_use]
    pub const fn stage(&self) -> AddStage {
        self.stage
    }

    /// Source surface values.
    #[must_use]
    pub const fn source(&self) -> &AddSourceState {
        &self.source
    }

    /// Defaults applied to every review opened by this workflow.
    #[must_use]
    pub const fn review_defaults(&self) -> &ReviewDefaults {
        &self.review_defaults
    }

    /// Mutable source surface values for mature widget synchronization.
    #[must_use]
    pub fn source_mut(&mut self) -> &mut AddSourceState {
        &mut self.source
    }

    /// Ambiguous kind picker when active.
    #[must_use]
    pub const fn kind_picker(&self) -> Option<&KindPickerState> {
        self.kind_picker.as_ref()
    }

    /// Review state when active.
    #[must_use]
    pub const fn review(&self) -> Option<&ReviewState> {
        self.review.as_ref()
    }

    /// Typed validation or host problem.
    #[must_use]
    pub const fn problem(&self) -> Option<&AddProblem> {
        self.problem.as_ref()
    }

    /// Most recent non-error workflow notice.
    #[must_use]
    pub const fn notice(&self) -> Option<&AddNotice> {
        self.notice.as_ref()
    }

    /// Whether one repository create is in flight.
    #[must_use]
    pub const fn commit_pending(&self) -> bool {
        self.pending_commit.is_some()
    }

    /// Apply one typed action and return zero or more host effects.
    #[must_use]
    pub fn reduce(&mut self, action: AddAction) -> Vec<AddEffect> {
        self.notice = None;
        match action {
            AddAction::SetSourcePath(value) => {
                self.source.path = value;
                self.pending_inspection = None;
                self.problem = None;
            }
            AddAction::SetCommandTemplate(value) => {
                self.source.command_template = value;
                self.problem = None;
            }
            AddAction::SetCommandName(value) => {
                self.source.command_name = value;
                self.problem = None;
            }
            AddAction::SetCommandDescription(value) => {
                self.source.command_description = value;
                self.problem = None;
            }
            AddAction::SelectDraft(index) => {
                if index < self.source.drafts.len() {
                    self.source.selected_draft = Some(index);
                    self.source.path = self.source.drafts[index].path.display().to_string();
                }
            }
            AddAction::Continue => return self.continue_source(),
            AddAction::SourceInspected { request, result } => {
                if self.pending_inspection != Some(request) {
                    return Vec::new();
                }
                self.pending_inspection = None;
                match result {
                    Ok(source) => self.route_source(source),
                    Err(reason) => {
                        self.problem = Some(AddProblem::SourceUnavailable {
                            path: PathBuf::from(self.source.path.trim()),
                            reason,
                        });
                    }
                }
            }
            AddAction::PickKind(picked) => {
                let Some(source) = self.pending_source.take() else {
                    return Vec::new();
                };
                let Some(kind) = picked else {
                    self.stage = AddStage::Source;
                    self.kind_picker = None;
                    return Vec::new();
                };
                if !self
                    .kind_picker
                    .as_ref()
                    .is_some_and(|picker| picker.offers(kind))
                {
                    self.problem = Some(AddProblem::InvalidKind);
                    self.pending_source = Some(source);
                    return Vec::new();
                }
                self.open_review(source, kind);
            }
            AddAction::NewDraft(kind) => {
                let request = self.request();
                self.pending_draft = Some(request);
                return vec![AddEffect::AuthorDraft { request, kind }];
            }
            AddAction::DraftEdited { request, result } => {
                if self.pending_draft != Some(request) {
                    return Vec::new();
                }
                self.pending_draft = None;
                match result {
                    Ok(Some(mut source)) => {
                        source.is_draft = true;
                        self.route_source(source);
                    }
                    Ok(None) => self.notice = Some(AddNotice::NothingWritten),
                    Err(reason) => self.problem = Some(AddProblem::EditFailed { reason }),
                }
            }
            AddAction::DeleteSelectedDraft => {
                let Some(draft) = self.source.selected_draft().cloned() else {
                    return Vec::new();
                };
                self.delete_candidate = Some(draft);
                self.stage = AddStage::ConfirmDraftDelete;
            }
            AddAction::ConfirmDraftDelete(false) => {
                self.delete_candidate = None;
                self.stage = AddStage::Source;
            }
            AddAction::ConfirmDraftDelete(true) => {
                let Some(draft) = self.delete_candidate.take() else {
                    return Vec::new();
                };
                let request = self.request();
                self.pending_delete = Some((request, draft.clone()));
                return vec![AddEffect::DeleteDraft { request, draft }];
            }
            AddAction::DraftDeleted { request, result } => {
                let Some((pending, draft)) = self.pending_delete.take() else {
                    return Vec::new();
                };
                if request != pending {
                    self.pending_delete = Some((pending, draft));
                    return Vec::new();
                }
                self.stage = AddStage::Source;
                match result {
                    Ok(DraftDeleteOutcome::Removed | DraftDeleteOutcome::AlreadyMissing) => {
                        self.problem = None;
                        self.source.remove_draft(&draft.path);
                        self.notice = Some(AddNotice::DraftDeleted(draft.path));
                    }
                    Ok(DraftDeleteOutcome::Changed(refreshed)) => {
                        self.notice = None;
                        let path = refreshed.path.clone();
                        self.source.replace_draft(&draft.path, refreshed);
                        self.problem = Some(AddProblem::DraftChanged { path });
                    }
                    Err(reason) => self.problem = Some(AddProblem::DraftDeleteFailed { reason }),
                }
            }
            AddAction::SetReviewName(value) => {
                if let Some(review) = &mut self.review {
                    review.set_name(value);
                }
                self.problem = None;
            }
            AddAction::SetReviewDescription(value) => {
                if let Some(review) = &mut self.review {
                    review.set_description(value);
                }
                self.problem = None;
            }
            AddAction::SetReviewStorage(mode) => {
                if let Some(review) = &mut self.review {
                    review.set_storage(mode);
                }
                self.problem = None;
            }
            AddAction::SetReviewDependencies(value) => {
                if let Some(review) = &mut self.review {
                    review.set_dependencies_text(value);
                }
                self.problem = None;
            }
            AddAction::SetReviewPython(value) => {
                if let Some(review) = &mut self.review {
                    review.set_requires_python(value);
                }
                self.problem = None;
            }
            AddAction::SetReviewCandidate { name, selected } => {
                if let Some(review) = &mut self.review {
                    review.set_candidate_selected(&name, selected);
                }
            }
            AddAction::SetPromptInterpolation(value) => {
                if let Some(review) = &mut self.review {
                    review.set_interpolate(value);
                }
            }
            AddAction::SetPromptCandidate { name, selected } => {
                if let Some(review) = &mut self.review {
                    review.set_prompt_selected(&name, selected);
                }
            }
            AddAction::SetPromptCandidates(selected) => {
                if let Some(review) = &mut self.review {
                    review.set_prompt_selection(&selected);
                }
            }
            AddAction::SetPromptRunner { name, picked } => {
                if let Some(review) = &mut self.review {
                    review.set_runner(&name, picked);
                }
            }
            AddAction::PromptRunnerAdded(name) => {
                if let Some(review) = &mut self.review {
                    review.add_runner(name);
                }
            }
            AddAction::EditSource => {
                let Some(path) = self
                    .review
                    .as_ref()
                    .map(|review| review.source.path.clone())
                else {
                    return Vec::new();
                };
                let request = self.request();
                self.pending_edit = Some(request);
                return vec![AddEffect::EditSource { request, path }];
            }
            AddAction::SourceEdited { request, result } => {
                if self.pending_edit != Some(request) {
                    return Vec::new();
                }
                self.pending_edit = None;
                match result {
                    Ok(source) => {
                        if let Some(review) = &mut self.review {
                            let bytes = source.bytes.clone();
                            review.source = source;
                            review.rescan(bytes);
                        }
                        self.problem = None;
                    }
                    Err(reason) => self.problem = Some(AddProblem::EditFailed { reason }),
                }
            }
            AddAction::Save => return self.save_review(),
            AddAction::CommitFinished { request, result } => {
                if self.pending_commit != Some(request) {
                    return Vec::new();
                }
                self.pending_commit = None;
                match result {
                    Err(reason) => {
                        self.problem = Some(AddProblem::CommitFailed { reason });
                    }
                    Ok(slug) => {
                        let mut effects = Vec::new();
                        if let Some(review) = &self.review {
                            if review.source.is_draft && review.storage == StorageMode::Copy {
                                effects.push(AddEffect::ConsumeDraft(review.source.clone()));
                            }
                            if review.lane == ReviewLane::Prompt
                                && review.runner_was_picked
                                && !review.runner.is_empty()
                            {
                                effects.push(AddEffect::RememberRunner(review.runner.clone()));
                            }
                        }
                        self.stage = AddStage::Complete;
                        effects.push(AddEffect::Complete(slug));
                        return effects;
                    }
                }
            }
            AddAction::Cancel => {
                if self.stage == AddStage::Review && !self.standalone_review {
                    let draft = self
                        .review
                        .as_ref()
                        .filter(|review| review.source.is_draft)
                        .map(|review| review.source.path.clone());
                    self.review = None;
                    self.stage = AddStage::Source;
                    self.notice = draft.clone().map(AddNotice::DraftKept);
                    return draft.into_iter().map(AddEffect::DraftKept).collect();
                }
                self.stage = AddStage::Cancelled;
                return vec![AddEffect::Cancel];
            }
        }
        Vec::new()
    }

    fn continue_source(&mut self) -> Vec<AddEffect> {
        if !self.source.path.trim().is_empty() {
            let request = self.request();
            self.pending_inspection = Some(request);
            return vec![AddEffect::InspectSource {
                request,
                path: PathBuf::from(self.source.path.trim()),
            }];
        }
        let template = self.source.command_template.trim();
        if template.is_empty() {
            return Vec::new();
        }
        let name = self.source.command_name.trim();
        if name.is_empty() {
            self.problem = Some(AddProblem::MissingCommandName);
            return Vec::new();
        }
        let kind = EntryKind::parse("command").expect("command kind is valid");
        let parameters = placeholder_params("command", template);
        let entry = CreateEntry {
            name: name.to_owned(),
            kind,
            mode: StorageMode::Reference,
            source: String::new(),
            workdir: "invoke".to_owned(),
            description: self.source.command_description.trim().to_owned(),
            payload: None,
            settings: EntrySettings {
                template: template.to_owned(),
                params: parameters
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect(),
                ..EntrySettings::default()
            },
        };
        self.commit(entry, None)
    }

    fn route_source(&mut self, source: SourceSnapshot) {
        if let Some(kind) = source.inferred_kind() {
            self.open_review(source, kind);
        } else {
            self.kind_picker = Some(KindPickerState::for_source(&source));
            self.pending_source = Some(source);
            self.stage = AddStage::Kind;
        }
    }

    fn open_review(&mut self, source: SourceSnapshot, kind: KnownEntryKind) {
        self.review = Some(ReviewState::from_source(
            source,
            kind,
            self.review_defaults.clone(),
        ));
        self.kind_picker = None;
        self.pending_source = None;
        self.problem = None;
        self.stage = AddStage::Review;
    }

    fn save_review(&mut self) -> Vec<AddEffect> {
        if self.pending_commit.is_some() {
            return Vec::new();
        }
        let Some(review) = &self.review else {
            return Vec::new();
        };
        let source = review.source.clone();
        match review.create_entry() {
            Ok(entry) => self.commit(entry, Some(source)),
            Err(problem) => {
                self.problem = Some(problem);
                Vec::new()
            }
        }
    }

    fn commit(&mut self, entry: CreateEntry, source: Option<SourceSnapshot>) -> Vec<AddEffect> {
        let request = self.request();
        self.pending_commit = Some(request);
        vec![AddEffect::Commit {
            request,
            entry: Box::new(entry),
            source,
        }]
    }

    fn request(&mut self) -> AddRequestId {
        self.next_request = self.next_request.saturating_add(1);
        AddRequestId(self.next_request)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use skit_application::{CreateEntry, SourcePermissions};
    use skit_domain::StorageMode;

    use super::*;

    fn source(path: &str, bytes: &[u8]) -> SourceSnapshot {
        SourceSnapshot {
            path: PathBuf::from(path),
            source_record: path.to_owned(),
            bytes: bytes.to_vec(),
            permissions: SourcePermissions {
                readonly: false,
                unix_mode: Some(0o644),
            },
            is_regular: true,
            is_directory: false,
            is_draft: false,
            identity: None,
        }
    }

    fn inspect_effect(effect: &AddEffect) -> Option<(AddRequestId, &Path)> {
        match effect {
            AddEffect::InspectSource { request, path } => Some((*request, path)),
            _ => None,
        }
    }

    fn author_effect(effect: &AddEffect) -> Option<(AddRequestId, DraftKind)> {
        match effect {
            AddEffect::AuthorDraft { request, kind } => Some((*request, *kind)),
            _ => None,
        }
    }

    fn edit_effect(effect: &AddEffect) -> Option<(AddRequestId, &Path)> {
        match effect {
            AddEffect::EditSource { request, path } => Some((*request, path)),
            _ => None,
        }
    }

    fn delete_effect(effect: &AddEffect) -> Option<(AddRequestId, &DraftSummary)> {
        match effect {
            AddEffect::DeleteDraft { request, draft } => Some((*request, draft)),
            _ => None,
        }
    }

    fn commit_effect(effect: &AddEffect) -> Option<(AddRequestId, &CreateEntry)> {
        match effect {
            AddEffect::Commit { request, entry, .. } => Some((*request, entry)),
            _ => None,
        }
    }

    #[test]
    fn effect_extractors_reject_the_wrong_typed_variant() {
        let cancel = AddEffect::Cancel;
        assert!(inspect_effect(&cancel).is_none());
        assert!(author_effect(&cancel).is_none());
        assert!(edit_effect(&cancel).is_none());
        assert!(delete_effect(&cancel).is_none());
        assert!(commit_effect(&cancel).is_none());
    }

    fn inspected(workflow: &mut AddWorkflowState, snapshot: SourceSnapshot) {
        let effects = workflow.reduce(AddAction::Continue);
        let request = effects
            .iter()
            .find_map(inspect_effect)
            .map(|(request, _)| request)
            .expect("path continuation must inspect the source");
        let _ = workflow.reduce(AddAction::SourceInspected {
            request,
            result: Ok(snapshot),
        });
    }

    #[test]
    fn workflow_reducer_edges_keep_requests_bytes_and_rejections_typed() {
        assert_eq!(KnownEntryKind::from_registry_str("future"), None);
        assert_eq!(
            KnownEntryKind::from_registry_str("exe"),
            Some(KnownEntryKind::Executable)
        );

        let mut workflow = AddWorkflowState::new(Vec::new());
        assert!(workflow.reduce(AddAction::Continue).is_empty());
        assert!(
            workflow
                .reduce(AddAction::DraftDeleted {
                    request: AddRequestId(0),
                    result: Ok(DraftDeleteOutcome::Removed),
                })
                .is_empty()
        );
        workflow.source_mut().path = "missing.py".to_owned();
        assert!(
            workflow
                .reduce(AddAction::SetCommandDescription("description".to_owned()))
                .is_empty()
        );
        assert_eq!(workflow.source().command_description, "description");
        let inspect = workflow.reduce(AddAction::Continue);
        let request = inspect
            .iter()
            .find_map(inspect_effect)
            .filter(|(_, path)| *path == Path::new("missing.py"))
            .map(|(request, _)| request)
            .expect("the path must produce one inspect request");
        let before = serde_json::to_value(&workflow).unwrap();
        assert!(
            workflow
                .reduce(AddAction::SourceInspected {
                    request: AddRequestId(request.0.saturating_add(1)),
                    result: Err("stale".to_owned()),
                })
                .is_empty()
        );
        assert_eq!(serde_json::to_value(&workflow).unwrap(), before);
        assert!(
            workflow
                .reduce(AddAction::SourceInspected {
                    request,
                    result: Err("not readable".to_owned()),
                })
                .is_empty()
        );
        assert!(matches!(
            workflow.problem(),
            Some(AddProblem::SourceUnavailable { path, reason })
                if path == &PathBuf::from("missing.py") && reason == "not readable"
        ));

        assert!(workflow.reduce(AddAction::PickKind(None)).is_empty());
        assert!(workflow.reduce(AddAction::EditSource).is_empty());
        assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
        assert!(
            workflow
                .reduce(AddAction::ConfirmDraftDelete(true))
                .is_empty()
        );
        assert!(workflow.reduce(AddAction::Save).is_empty());

        let draft_effect = workflow.reduce(AddAction::NewDraft(DraftKind::Script));
        let draft_request = draft_effect
            .iter()
            .find_map(author_effect)
            .filter(|(_, kind)| *kind == DraftKind::Script)
            .map(|(request, _)| request)
            .expect("new draft must ask the host to run the editor");
        let before = serde_json::to_value(&workflow).unwrap();
        assert!(
            workflow
                .reduce(AddAction::DraftEdited {
                    request: AddRequestId(draft_request.0.saturating_add(1)),
                    result: Err("stale".to_owned()),
                })
                .is_empty()
        );
        assert_eq!(serde_json::to_value(&workflow).unwrap(), before);
        assert!(
            workflow
                .reduce(AddAction::DraftEdited {
                    request: draft_request,
                    result: Ok(None),
                })
                .is_empty()
        );
        assert_eq!(workflow.notice(), Some(&AddNotice::NothingWritten));

        let effects = workflow.reduce(AddAction::NewDraft(DraftKind::Prompt));
        let request = effects
            .iter()
            .find_map(author_effect)
            .map(|(request, _)| request)
            .expect("second draft request");
        assert!(
            workflow
                .reduce(AddAction::DraftEdited {
                    request,
                    result: Err("editor failed".to_owned()),
                })
                .is_empty()
        );
        assert_eq!(
            workflow.problem(),
            Some(&AddProblem::EditFailed {
                reason: "editor failed".to_owned(),
            })
        );

        let mut prompt = source("task.prompt.md", b"Review {{topic}} {{format}}.");
        prompt.is_draft = true;
        let mut review =
            ReviewState::from_source(prompt, KnownEntryKind::Prompt, ReviewDefaults::default());
        review.set_description("old");
        let mut workflow = AddWorkflowState::from_review(review);
        assert!(
            workflow
                .reduce(AddAction::SetReviewDescription("new".to_owned()))
                .is_empty()
        );
        assert!(
            workflow
                .reduce(AddAction::SetReviewCandidate {
                    name: "missing".to_owned(),
                    selected: false,
                })
                .is_empty()
        );
        assert!(
            workflow
                .reduce(AddAction::SetPromptCandidate {
                    name: "topic".to_owned(),
                    selected: false,
                })
                .is_empty()
        );
        assert!(
            workflow
                .reduce(AddAction::SetPromptCandidates(vec!["format".to_owned()]))
                .is_empty()
        );
        assert_eq!(workflow.review().unwrap().description(), "new");
        assert_eq!(
            workflow.review().unwrap().selected_prompt_names(),
            ["format"]
        );
        assert_eq!(workflow.review().unwrap().prompt_candidates().len(), 2);
        assert!(!workflow.review().unwrap().runner_was_picked());

        let effects = workflow.reduce(AddAction::EditSource);
        let edit_request = effects
            .iter()
            .find_map(edit_effect)
            .filter(|(_, path)| *path == Path::new("task.prompt.md"))
            .map(|(request, _)| request)
            .expect("review edit request");
        let before = serde_json::to_value(&workflow).unwrap();
        assert!(
            workflow
                .reduce(AddAction::SourceEdited {
                    request: AddRequestId(edit_request.0.saturating_add(1)),
                    result: Err("stale".to_owned()),
                })
                .is_empty()
        );
        assert_eq!(serde_json::to_value(&workflow).unwrap(), before);
        assert!(
            workflow
                .reduce(AddAction::SourceEdited {
                    request: edit_request,
                    result: Err("editor failed".to_owned()),
                })
                .is_empty()
        );
        assert!(matches!(
            workflow.problem(),
            Some(AddProblem::EditFailed { .. })
        ));

        let effects = workflow.reduce(AddAction::EditSource);
        let request = effects
            .iter()
            .find_map(edit_effect)
            .map(|(request, _)| request)
            .expect("retry edit request");
        let mut invalid = source("task.prompt.md", &[0xff, 0xfe]);
        invalid.is_draft = true;
        assert!(
            workflow
                .reduce(AddAction::SourceEdited {
                    request,
                    result: Ok(invalid),
                })
                .is_empty()
        );
        assert_eq!(workflow.problem(), None);
        assert!(matches!(
            workflow.review().unwrap().create_entry(),
            Err(AddProblem::InvalidPromptEncoding)
        ));

        let effects = workflow.reduce(AddAction::Save);
        assert!(effects.is_empty());
        assert_eq!(workflow.problem(), Some(&AddProblem::InvalidPromptEncoding));

        let mut unknown_draft = source("skit-new-tool.bin", b"plain\n");
        unknown_draft.is_draft = true;
        let mut classify = AddWorkflowState::new(Vec::new());
        assert!(
            classify
                .reduce(AddAction::SetSourcePath("skit-new-tool.bin".to_owned()))
                .is_empty()
        );
        inspected(&mut classify, unknown_draft);
        let picker = classify.kind_picker().expect("draft kind picker");
        assert_eq!(picker.filename(), "skit-new-tool.bin");
        assert!(!picker.has_shebang());
        assert!(!picker.choices().contains(&KnownEntryKind::Executable));
        assert!(
            classify
                .reduce(AddAction::PickKind(Some(KnownEntryKind::Executable)))
                .is_empty()
        );
        assert_eq!(classify.problem(), Some(&AddProblem::InvalidKind));
        assert!(classify.reduce(AddAction::PickKind(None)).is_empty());
        assert_eq!(classify.stage(), AddStage::Source);

        let mut valid_prompt = AddWorkflowState::from_review(ReviewState::from_source(
            source("task.prompt.md", b"Review {{topic}}."),
            KnownEntryKind::Prompt,
            ReviewDefaults::default(),
        ));
        assert!(
            valid_prompt
                .reduce(AddAction::SetReviewStorage(StorageMode::Reference))
                .is_empty()
        );
        assert!(
            valid_prompt
                .reduce(AddAction::SetPromptInterpolation(false))
                .is_empty()
        );
        assert!(!valid_prompt.review().unwrap().interpolate());
        let commit = valid_prompt.reduce(AddAction::Save);
        assert!(matches!(commit.as_slice(), [AddEffect::Commit { .. }]));
        assert!(valid_prompt.commit_pending());
        assert!(valid_prompt.reduce(AddAction::Save).is_empty());
        assert_eq!(
            valid_prompt.reduce(AddAction::Cancel),
            vec![AddEffect::Cancel]
        );

        let invalid_prompt = ReviewState::from_source(
            source("bad.prompt.md", &[0xff]),
            KnownEntryKind::Prompt,
            ReviewDefaults::default(),
        );
        assert!(matches!(
            invalid_prompt.create_entry(),
            Err(AddProblem::InvalidPromptEncoding)
        ));

        let owned_python = ReviewState::from_source(
            source(
                "owned.py",
                b"# /// script\n# dependencies = [\"requests\"]\n# ///\nprint('ok')\n",
            ),
            KnownEntryKind::Python,
            ReviewDefaults {
                dependencies: vec!["ignored-default".to_owned()],
                ..ReviewDefaults::default()
            },
        );
        assert!(matches!(
            owned_python.dependency_surface(),
            DependencySurface::PythonOwned(metadata)
                if metadata.dependencies == ["requests"]
        ));
        assert_eq!(owned_python.dependencies_text(), "");

        let mut javascript = ReviewState::from_source(
            source("tool.ts", b"import chalk from 'chalk';\n"),
            KnownEntryKind::TypeScript,
            ReviewDefaults::default(),
        );
        assert_eq!(javascript.dependency_surface(), &DependencySurface::Npm);
        assert_eq!(javascript.modeled_cli_field_count(), None);
        assert_eq!(javascript.requires_python(), "");
        javascript.rescan(b"import zod from 'zod';\n".to_vec());
        assert_eq!(javascript.dependency_surface(), &DependencySurface::Npm);

        let mut invalid_python = ReviewState::from_source(
            source(
                "invalid.py",
                &[b'p', b'r', b'i', b'n', b't', b'(', b'1', b')', b'\n', 0xff],
            ),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        invalid_python.set_dependencies_text("requests");
        let invalid_python_entry = invalid_python.create_entry().unwrap();
        assert!(
            invalid_python_entry
                .payload
                .as_ref()
                .expect("a copied script has a payload")
                .bytes
                .ends_with(&[0xff])
        );

        let invalid_candidate_source = b"# /// script\n# [tool.skit\n# ///\nA = 1\nprint(A)\n";
        let invalid_candidates = ReviewState::from_source(
            source("invalid-candidates.py", invalid_candidate_source),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        assert!(invalid_candidates.candidate("A").is_some());
        assert!(matches!(
            invalid_candidates.create_entry(),
            Err(AddProblem::SourceEdit { .. })
        ));

        let mut unnamed = ReviewState::from_source(
            source("fallback.py", b"print('ok')\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        unnamed.set_name("  ");
        assert_eq!(unnamed.create_entry().unwrap().name, "fallback");

        let mut directory = source("Demo.app", b"");
        directory.is_regular = false;
        directory.is_directory = true;
        let mut directory_workflow = AddWorkflowState::new(Vec::new());
        assert!(
            directory_workflow
                .reduce(AddAction::SetSourcePath("Demo.app".to_owned()))
                .is_empty()
        );
        inspected(&mut directory_workflow, directory);
        assert_eq!(directory_workflow.stage(), AddStage::Review);
        assert_eq!(
            directory_workflow.review().unwrap().kind(),
            KnownEntryKind::Executable
        );

        for kind in [
            KnownEntryKind::Fish,
            KnownEntryKind::TypeScript,
            KnownEntryKind::PowerShell,
            KnownEntryKind::Ruby,
            KnownEntryKind::Perl,
            KnownEntryKind::Lua,
            KnownEntryKind::R,
        ] {
            assert_eq!(KnownEntryKind::from_registry_str(kind.as_str()), Some(kind));
        }

        let drafts = (0..=DRAFTS_LISTED)
            .map(|index| DraftSummary {
                path: PathBuf::from(format!("skit-new-{index}.py")),
                modified: index as u64,
                identity: None,
                permissions: SourcePermissions::default(),
            })
            .collect();
        assert_eq!(AddSourceState::new(drafts).draft_overflow(), 1);
    }

    #[test]
    fn a_source_path_wins_over_a_filled_command_lane() {
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetSourcePath("tool.unknown".into()));
        let _ = workflow.reduce(AddAction::SetCommandTemplate("echo {name}".into()));
        let _ = workflow.reduce(AddAction::SetCommandName("Echo".into()));

        let effects = workflow.reduce(AddAction::Continue);

        assert!(matches!(
            effects.as_slice(),
            [AddEffect::InspectSource { path, .. }] if path == &PathBuf::from("tool.unknown")
        ));
        assert_eq!(workflow.stage(), AddStage::Source);
    }

    #[test]
    fn an_unknown_file_opens_the_typed_kind_picker() {
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetSourcePath("notes.bin".into()));
        inspected(&mut workflow, source("notes.bin", b"plain text\n"));

        assert_eq!(workflow.stage(), AddStage::Kind);
        let picker = workflow.kind_picker().expect("kind picker");
        assert!(picker.offers(KnownEntryKind::Executable));
        assert_eq!(picker.suggested(), None);

        let _ = workflow.reduce(AddAction::PickKind(Some(KnownEntryKind::Shell)));
        assert_eq!(workflow.stage(), AddStage::Review);
        assert_eq!(workflow.review().unwrap().kind(), KnownEntryKind::Shell);
    }

    #[test]
    fn a_markdown_draft_suggests_prompt_and_never_offers_executable() {
        let mut draft = source("skit-new-note.md", b"Write {{subject}}\n");
        draft.is_draft = true;
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetSourcePath(draft.path.display().to_string()));
        inspected(&mut workflow, draft);

        let picker = workflow.kind_picker().expect("kind picker");
        assert_eq!(picker.suggested(), Some(KnownEntryKind::Prompt));
        assert!(!picker.offers(KnownEntryKind::Executable));
    }

    #[test]
    fn review_defaults_survive_source_inspection_and_prefill_prompt_runner_state() {
        let defaults = ReviewDefaults {
            name: Some("Pinned name".into()),
            description: Some("Pinned description".into()),
            reference: true,
            runner_names: vec!["codex".into(), "claude".into()],
            last_runner: Some("claude".into()),
            interpolate: Some(false),
            ..ReviewDefaults::default()
        };
        let mut workflow = AddWorkflowState::new(Vec::new()).with_review_defaults(defaults.clone());
        let _ = workflow.reduce(AddAction::SetSourcePath("task.prompt.md".into()));

        inspected(
            &mut workflow,
            source("task.prompt.md", b"Review {{topic}}.\n"),
        );

        let review = workflow.review().unwrap();
        assert_eq!(review.name(), "Pinned name");
        assert_eq!(review.description(), "Pinned description");
        assert_eq!(review.storage(), StorageMode::Reference);
        assert_eq!(review.runner_names(), ["codex", "claude"]);
        assert_eq!(review.runner(), "claude");
        assert!(!review.interpolate());
        assert_eq!(workflow.review_defaults(), &defaults);
    }

    #[test]
    fn description_is_derived_after_kind_selection_and_an_explicit_override_wins() {
        let bytes = b"\"\"\"Python documentation.\"\"\"\nprint(1)\n";
        let python = ReviewState::from_source(
            source("ambiguous.text", bytes),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        let shell = ReviewState::from_source(
            source("ambiguous.text", bytes),
            KnownEntryKind::Shell,
            ReviewDefaults::default(),
        );
        let overridden = ReviewState::from_source(
            source("ambiguous.text", bytes),
            KnownEntryKind::Python,
            ReviewDefaults {
                description: Some("User text".into()),
                ..ReviewDefaults::default()
            },
        );

        assert_eq!(python.description(), "Python documentation.");
        assert_eq!(shell.description(), "");
        assert_eq!(overridden.description(), "User text");
    }

    #[test]
    fn unchanged_kept_and_deleted_drafts_publish_neutral_serializable_notices() {
        let mut unchanged = AddWorkflowState::new(Vec::new());
        let effects = unchanged.reduce(AddAction::NewDraft(DraftKind::Script));
        let request = effects
            .iter()
            .find_map(author_effect)
            .map(|(request, _)| request)
            .expect("new draft must ask the host to run the editor");
        assert!(
            unchanged
                .reduce(AddAction::DraftEdited {
                    request,
                    result: Ok(None),
                })
                .is_empty()
        );
        assert_eq!(unchanged.notice(), Some(&AddNotice::NothingWritten));
        let encoded = serde_json::to_string(&unchanged).unwrap();
        let decoded: AddWorkflowState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.notice(), Some(&AddNotice::NothingWritten));

        let mut draft = source("skit-new-task.py", b"print(1)\n");
        draft.is_draft = true;
        let draft_path = draft.path.clone();
        let mut kept = AddWorkflowState::new(Vec::new());
        let _ = kept.reduce(AddAction::SetSourcePath(draft_path.display().to_string()));
        inspected(&mut kept, draft);
        assert_eq!(
            kept.reduce(AddAction::Cancel),
            vec![AddEffect::DraftKept(draft_path.clone())]
        );
        assert_eq!(
            kept.notice(),
            Some(&AddNotice::DraftKept(draft_path.clone()))
        );

        let draft = DraftSummary {
            path: PathBuf::from("skit-new-old.py"),
            modified: 1,
            identity: None,
            permissions: SourcePermissions::default(),
        };
        let mut deleted = AddWorkflowState::new(vec![draft.clone()]);
        let _ = deleted.reduce(AddAction::SelectDraft(0));
        let _ = deleted.reduce(AddAction::DeleteSelectedDraft);
        let effects = deleted.reduce(AddAction::ConfirmDraftDelete(true));
        let request = effects
            .iter()
            .find_map(delete_effect)
            .map(|(request, _)| request)
            .expect("confirmed delete must ask the host to delete");
        assert!(
            deleted
                .reduce(AddAction::DraftDeleted {
                    request: AddRequestId(request.0.saturating_add(1)),
                    result: Ok(DraftDeleteOutcome::Removed),
                })
                .is_empty()
        );
        assert!(deleted.notice().is_none());
        assert_eq!(
            deleted.source().listed_drafts(),
            std::slice::from_ref(&draft)
        );
        let _ = deleted.reduce(AddAction::DraftDeleted {
            request,
            result: Ok(DraftDeleteOutcome::Removed),
        });
        assert_eq!(deleted.notice(), Some(&AddNotice::DraftDeleted(draft.path)));
    }

    #[test]
    fn kept_draft_deletion_requires_confirmation_and_only_removes_the_confirmed_row() {
        let legacy: DraftSummary =
            serde_json::from_str(r#"{"path":"skit-new-legacy.py","modified":0}"#).unwrap();
        assert!(
            legacy.identity.is_none(),
            "legacy UI state has no identity field"
        );

        let first = DraftSummary {
            path: PathBuf::from("skit-new-first.py"),
            modified: 1,
            identity: None,
            permissions: SourcePermissions::default(),
        };
        let second = DraftSummary {
            path: PathBuf::from("skit-new-second.py"),
            modified: 2,
            identity: None,
            permissions: SourcePermissions::default(),
        };
        let mut workflow = AddWorkflowState::new(vec![first.clone(), second.clone()]);
        let _ = workflow.reduce(AddAction::SelectDraft(0));
        assert!(workflow.reduce(AddAction::DeleteSelectedDraft).is_empty());
        assert_eq!(workflow.stage(), AddStage::ConfirmDraftDelete);
        assert!(
            workflow
                .reduce(AddAction::ConfirmDraftDelete(false))
                .is_empty()
        );
        assert_eq!(workflow.source().listed_drafts().len(), 2);

        let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
        let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));
        let (request, draft) = effects
            .iter()
            .find_map(delete_effect)
            .expect("confirmed deletion must emit one typed host request");
        assert_eq!(draft, &second);
        let refreshed = DraftSummary {
            path: second.path.clone(),
            modified: 3,
            identity: None,
            permissions: SourcePermissions::default(),
        };
        assert!(
            workflow
                .reduce(AddAction::DraftDeleted {
                    request,
                    result: Ok(DraftDeleteOutcome::Changed(refreshed.clone())),
                })
                .is_empty()
        );
        assert_eq!(
            workflow.source().listed_drafts(),
            &[refreshed.clone(), first.clone()]
        );
        assert!(matches!(
            workflow.problem(),
            Some(AddProblem::DraftChanged { .. })
        ));

        let _ = workflow.reduce(AddAction::SelectDraft(0));
        let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
        let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));
        let (request, draft) = effects
            .iter()
            .find_map(delete_effect)
            .expect("a retry must use the refreshed draft claim");
        assert_eq!(draft, &refreshed);
        assert!(
            workflow
                .reduce(AddAction::DraftDeleted {
                    request,
                    result: Ok(DraftDeleteOutcome::AlreadyMissing),
                })
                .is_empty()
        );
        assert_eq!(workflow.source().listed_drafts(), &[first]);
        assert!(
            workflow.problem().is_none(),
            "successful AlreadyMissing completion clears the earlier Changed warning"
        );
        assert_eq!(
            workflow.notice(),
            Some(&AddNotice::DraftDeleted(refreshed.path)),
            "AlreadyMissing is an idempotent successful deletion"
        );
    }

    #[test]
    fn draft_delete_error_keeps_the_row_and_never_fabricates_success() {
        let draft = DraftSummary {
            path: PathBuf::from("skit-new-error.py"),
            modified: 1,
            identity: None,
            permissions: SourcePermissions::default(),
        };
        let mut workflow = AddWorkflowState::new(vec![draft.clone()]);
        let _ = workflow.reduce(AddAction::SelectDraft(0));
        let _ = workflow.reduce(AddAction::DeleteSelectedDraft);
        let effects = workflow.reduce(AddAction::ConfirmDraftDelete(true));
        let (request, claimed) = effects
            .iter()
            .find_map(delete_effect)
            .expect("confirmed deletion must carry one claim");
        assert_eq!(claimed, &draft);
        assert!(
            workflow
                .reduce(AddAction::DraftDeleted {
                    request,
                    result: Err("could not quarantine the draft".into()),
                })
                .is_empty()
        );
        assert_eq!(workflow.source().listed_drafts(), &[draft]);
        assert!(matches!(
            workflow.problem(),
            Some(AddProblem::DraftDeleteFailed { .. })
        ));
        assert!(workflow.notice().is_none());
    }

    #[test]
    fn executable_review_is_reference_only_and_directory_payload_is_metadata_only() {
        let mut executable = source("Demo.app", b"");
        executable.is_regular = false;
        executable.is_directory = true;
        let mut review = ReviewState::from_source(
            executable,
            KnownEntryKind::Executable,
            ReviewDefaults::default(),
        );
        review.set_storage(StorageMode::Copy);

        let entry = review.create_entry().unwrap();

        assert_eq!(entry.mode, StorageMode::Reference);
        assert!(entry.payload.is_none());
        assert_eq!(entry.source, "Demo.app");
    }

    #[test]
    fn a_modeled_reader_suppresses_candidates_but_static_zero_does_not() {
        let modeled = ReviewState::from_source(
            source(
                "tool.py",
                b"import argparse\nP = 'x'\np = argparse.ArgumentParser()\np.add_argument('--name')\n",
            ),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        assert!(modeled.candidates().is_empty());
        assert_eq!(modeled.modeled_cli_field_count(), Some(1));

        let zero = ReviewState::from_source(
            source(
                "tool.py",
                b"P = 'x'\np.add_argument('--help', action='help')\n",
            ),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        assert_eq!(zero.modeled_cli_field_count(), Some(0));
        assert_eq!(zero.candidates().len(), 1);
    }

    #[test]
    fn rescan_keeps_existing_ticks_by_binding_identity_and_defaults_new_candidates() {
        let mut review = ReviewState::from_source(
            source("tool.py", b"A = 1\nB = 2\nprint(A, B)\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        review.set_candidate_selected("A", false);
        review.set_name("Kept name");
        review.set_description("Kept description");
        review.set_dependencies_text("requests>=2");

        review.rescan(b"B = 3\nA = 4\nC = 5\nprint(A, B, C)\n".to_vec());

        assert!(!review.candidate("A").unwrap().selected);
        assert!(review.candidate("B").unwrap().selected);
        assert!(review.candidate("C").unwrap().selected);
        assert_eq!(review.name(), "Kept name");
        assert_eq!(review.description(), "Kept description");
        assert_eq!(review.dependencies_text(), "requests>=2");
    }

    #[test]
    fn prompt_flood_defaults_to_none_and_rescan_preserves_name_keyed_ticks() {
        let prompt = (0..=PROMPT_AUTO_MANAGE_LIMIT)
            .map(|index| format!("{{{{h{index}}}}}"))
            .collect::<Vec<_>>()
            .join(" ");
        let mut review = ReviewState::from_source(
            source("task.prompt.md", prompt.as_bytes()),
            KnownEntryKind::Prompt,
            ReviewDefaults::default(),
        );
        assert!(review.prompt_is_flooded());
        assert_eq!(review.prompt_preview().len(), PROMPT_LIST_PREVIEW_LIMIT);
        assert!(review.selected_prompt_names().is_empty());

        review.set_prompt_selected("h3", true);
        review.rescan(b"{{new}} {{h3}} {{h1}}".to_vec());
        assert_eq!(review.selected_prompt_names(), vec!["new", "h3"]);
    }

    #[test]
    fn prompt_picker_carries_the_complete_name_keyed_working_selection() {
        let mut review = ReviewState::from_source(
            source("task.prompt.md", b"{{topic}} {{api_key}} {{format}}"),
            KnownEntryKind::Prompt,
            ReviewDefaults::default(),
        );
        review.set_prompt_selected("topic", false);

        let mut picker = review.prompt_picker();
        picker.set_query("key");

        assert_eq!(
            picker
                .visible_items()
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["api_key"]
        );
        assert!(!picker.is_selected(&"topic".to_owned()));
        assert!(picker.is_selected(&"api_key".to_owned()));
        assert!(picker.is_selected(&"format".to_owned()));
    }

    #[test]
    fn invalid_python_metadata_refuses_before_any_commit_effect() {
        let mut workflow = AddWorkflowState::from_review(ReviewState::from_source(
            source("tool.py", b"print('ok')\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        ));
        let _ = workflow.reduce(AddAction::SetReviewDependencies(
            "not a valid requirement !!!".into(),
        ));

        let effects = workflow.reduce(AddAction::Save);

        assert!(effects.is_empty());
        assert!(matches!(
            workflow.problem(),
            Some(AddProblem::InvalidDependency { .. })
        ));
        assert!(!workflow.commit_pending());

        let _ = workflow.reduce(AddAction::SetReviewDependencies("requests".into()));
        let _ = workflow.reduce(AddAction::SetReviewPython(">>=3.12".into()));
        assert!(workflow.reduce(AddAction::Save).is_empty());
        assert!(matches!(
            workflow.problem(),
            Some(AddProblem::InvalidPythonConstraint { .. })
        ));
    }

    #[test]
    fn python_review_writes_valid_metadata_into_the_copy_request_only() {
        let mut review = ReviewState::from_source(
            source("tool.py", b"#!/usr/bin/env python3.12\nprint('ok')\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        review.set_dependencies_text("requests>=2,<3, rich");

        let entry = review.create_entry().unwrap();
        let payload = entry.payload.unwrap();

        assert_eq!(entry.mode, StorageMode::Copy);
        assert!(entry.settings.dependencies.is_empty());
        assert!(entry.settings.requires_python.is_empty());
        let text = String::from_utf8(payload.bytes).unwrap();
        assert!(text.contains("requests>=2,<3"));
        assert!(text.contains("rich"));
        assert!(text.contains("requires-python = \">=3.12,<3.13\""));
    }

    #[test]
    fn prompt_success_remembers_only_an_actively_picked_runner() {
        let mut workflow = AddWorkflowState::from_review(ReviewState::from_source(
            source("task.prompt.md", b"Review {{topic}}."),
            KnownEntryKind::Prompt,
            ReviewDefaults {
                runner_names: vec!["Agent".into()],
                ..ReviewDefaults::default()
            },
        ));
        let _ = workflow.reduce(AddAction::SetPromptRunner {
            name: "Agent".into(),
            picked: true,
        });
        let effects = workflow.reduce(AddAction::Save);
        let (request, entry) = effects
            .iter()
            .find_map(commit_effect)
            .expect("prompt save must remain one atomic request");
        assert_eq!(entry.settings.runner, "Agent");
        assert!(entry.settings.interpolate);
        assert_eq!(entry.settings.params, vec!["topic"]);
        assert!(
            entry.settings.parameters.is_empty(),
            "detected prompt placeholders stay implicit"
        );

        assert_eq!(
            workflow.reduce(AddAction::CommitFinished {
                request,
                result: Ok("task".into()),
            }),
            vec![
                AddEffect::RememberRunner("Agent".into()),
                AddEffect::Complete("task".into()),
            ]
        );
    }

    #[test]
    fn save_emits_one_complete_create_request_and_only_success_consumes_a_draft() {
        let mut draft = source("skit-new-tool.py", b"VALUE = 1\nprint(VALUE)\n");
        draft.is_draft = true;
        let encoded = serde_json::to_vec(&draft).unwrap();
        assert_eq!(
            serde_json::from_slice::<SourceSnapshot>(&encoded).unwrap(),
            draft,
            "the host identity survives the serialized UI seam"
        );
        let mut legacy = serde_json::to_value(&draft).unwrap();
        legacy.as_object_mut().unwrap().remove("identity");
        assert!(
            serde_json::from_value::<SourceSnapshot>(legacy)
                .unwrap()
                .identity
                .is_none(),
            "a snapshot serialized before identity existed still decodes"
        );
        let mut workflow = AddWorkflowState::from_review(ReviewState::from_source(
            draft.clone(),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        ));

        let effects = workflow.reduce(AddAction::Save);
        let (request, entry) = effects
            .iter()
            .find_map(commit_effect)
            .expect("save must emit exactly one atomic create request");
        assert_create_is_complete(entry);
        assert!(
            workflow
                .reduce(AddAction::CommitFinished {
                    request: AddRequestId(request.0.saturating_add(1)),
                    result: Ok("stale".into()),
                })
                .is_empty(),
            "a stale host completion must not consume the current draft"
        );
        assert_eq!(workflow.stage(), AddStage::Review);
        assert!(
            workflow
                .reduce(AddAction::CommitFinished {
                    request,
                    result: Err("disk full".into()),
                })
                .is_empty()
        );
        assert_eq!(workflow.stage(), AddStage::Review);
        assert_eq!(workflow.review().unwrap().source().bytes, draft.bytes);

        let effects = workflow.reduce(AddAction::Save);
        let request = effects
            .iter()
            .find_map(commit_effect)
            .map(|(request, _)| request)
            .expect("retry must commit");
        let effects = workflow.reduce(AddAction::CommitFinished {
            request,
            result: Ok("tool".into()),
        });
        assert_eq!(
            effects,
            vec![
                AddEffect::ConsumeDraft(draft),
                AddEffect::Complete("tool".into()),
            ]
        );
        assert_eq!(workflow.stage(), AddStage::Complete);
    }

    #[test]
    fn reference_review_never_writes_managed_source_or_npm_dependencies() {
        let mut review = ReviewState::from_source(
            source("tool.js", b"const PORT = 3000;\nconsole.log(PORT);\n"),
            KnownEntryKind::JavaScript,
            ReviewDefaults::default(),
        );
        review.set_storage(StorageMode::Reference);
        review.set_dependencies_text("chalk");
        let original = review.source().bytes.clone();

        let entry = review.create_entry().unwrap();

        assert_eq!(entry.mode, StorageMode::Reference);
        assert_eq!(entry.payload.unwrap().bytes, original);
        assert!(entry.settings.dependencies.is_empty());
        assert!(entry.settings.parameters.is_empty());
    }

    #[test]
    fn a_command_submission_is_atomic_and_requires_a_name() {
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetCommandTemplate("echo {name}".into()));
        assert!(workflow.reduce(AddAction::Continue).is_empty());
        assert_eq!(workflow.problem(), Some(&AddProblem::MissingCommandName));

        let _ = workflow.reduce(AddAction::SetCommandName("Echo".into()));
        let effects = workflow.reduce(AddAction::Continue);
        let (_, entry) = effects
            .iter()
            .find_map(commit_effect)
            .expect("valid command must use the same atomic commit effect");
        assert_eq!(entry.kind.as_str(), "command");
        assert_eq!(entry.settings.params, vec!["name"]);
        assert!(
            entry.settings.parameters.is_empty(),
            "template slots stay implicit until a schema edit"
        );
        assert!(entry.payload.is_none());
    }

    #[test]
    fn stale_source_and_commit_results_cannot_advance_the_workflow() {
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetSourcePath("a.py".into()));
        let first = workflow.reduce(AddAction::Continue);
        let old = first
            .iter()
            .find_map(inspect_effect)
            .map(|(request, _)| request)
            .expect("first inspection request");
        let _ = workflow.reduce(AddAction::SetSourcePath("b.py".into()));
        let _ = workflow.reduce(AddAction::Continue);
        let _ = workflow.reduce(AddAction::SourceInspected {
            request: old,
            result: Ok(source("a.py", b"print(1)\n")),
        });
        assert_eq!(workflow.stage(), AddStage::Source);
    }

    #[test]
    fn cancelling_nested_review_returns_to_source_and_keeps_authored_draft() {
        let mut draft = source("skit-new-tool.py", b"print('kept')\n");
        draft.is_draft = true;
        let path = draft.path.clone();
        let mut workflow = AddWorkflowState::new(Vec::new());
        let _ = workflow.reduce(AddAction::SetSourcePath(path.display().to_string()));
        inspected(&mut workflow, draft);

        let effects = workflow.reduce(AddAction::Cancel);

        assert_eq!(workflow.stage(), AddStage::Source);
        assert_eq!(effects, vec![AddEffect::DraftKept(path)]);
    }

    #[test]
    fn cancelling_a_standalone_review_closes_its_host() {
        let review = ReviewState::from_source(
            source("tool.py", b"print(1)\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        let mut workflow = AddWorkflowState::from_review(review);

        assert_eq!(workflow.reduce(AddAction::Cancel), vec![AddEffect::Cancel]);
        assert_eq!(workflow.stage(), AddStage::Cancelled);
    }

    #[test]
    fn rescan_promotes_any_python_fence_to_read_only_authority() {
        let mut review = ReviewState::from_source(
            source("tool.py", b"import requests\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        review.set_dependencies_text("rich");

        review.rescan(
            b"# /// script\n# dependencies = [\"httpx\"]\n# ///\nimport requests\n".to_vec(),
        );

        assert!(matches!(
            review.dependency_surface(),
            DependencySurface::PythonOwned(metadata) if metadata.dependencies == ["httpx"]
        ));
        let entry = review.create_entry().unwrap();
        assert!(entry.settings.dependencies.is_empty());
        assert_eq!(
            entry.payload.unwrap().bytes,
            b"# /// script\n# dependencies = [\"httpx\"]\n# ///\nimport requests\n"
        );
    }

    #[test]
    fn pep508_parser_partition_keeps_internal_commas() {
        assert_eq!(
            split_pep508_requirements(
                "requests>=2,<3, rich[markdown,syntax]>=13, platformdirs; sys_platform in \"linux,darwin\""
            ),
            vec![
                "requests>=2,<3",
                "rich[markdown,syntax]>=13",
                "platformdirs; sys_platform in \"linux,darwin\"",
            ]
        );
    }

    #[test]
    fn create_projection_uses_kind_storage_workdir_and_payload_name_policy() {
        let python_copy = ReviewState::from_source(
            source("custom.py", b"print(1)\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        )
        .create_entry()
        .unwrap();
        assert_eq!(python_copy.workdir, "invoke");
        assert_eq!(
            python_copy.payload.unwrap().stored_name.as_deref(),
            Some("script.py")
        );

        let mut python_reference = ReviewState::from_source(
            source("custom.py", b"print(1)\n"),
            KnownEntryKind::Python,
            ReviewDefaults::default(),
        );
        python_reference.set_storage(StorageMode::Reference);
        assert_eq!(python_reference.create_entry().unwrap().workdir, "origin");

        let mut prompt_reference = ReviewState::from_source(
            source("review.prompt.md", b"Review this.\n"),
            KnownEntryKind::Prompt,
            ReviewDefaults::default(),
        );
        prompt_reference.set_storage(StorageMode::Reference);
        let prompt = prompt_reference.create_entry().unwrap();
        assert_eq!(prompt.workdir, "invoke");
        assert_eq!(
            prompt.payload.unwrap().stored_name.as_deref(),
            Some("prompt.md")
        );

        let module = ReviewState::from_source(
            source("module.mjs", b"export const value = 1;\n"),
            KnownEntryKind::JavaScript,
            ReviewDefaults::default(),
        )
        .create_entry()
        .unwrap();
        assert_eq!(
            module.payload.unwrap().stored_name.as_deref(),
            Some("script.mjs")
        );

        let executable = ReviewState::from_source(
            source("my-tool", b"binary-ish"),
            KnownEntryKind::Executable,
            ReviewDefaults::default(),
        )
        .create_entry()
        .unwrap();
        assert_eq!(executable.workdir, "origin");
        assert_eq!(
            executable.payload.unwrap().stored_name.as_deref(),
            Some("my-tool")
        );
    }

    fn assert_create_is_complete(entry: &CreateEntry) {
        assert_eq!(entry.mode, StorageMode::Copy);
        assert!(!entry.name.is_empty());
        assert_eq!(entry.kind.as_str(), "python");
        assert!(
            entry
                .payload
                .as_ref()
                .is_some_and(|payload| !payload.bytes.is_empty())
        );
        assert_eq!(entry.workdir, "invoke");
    }
}
