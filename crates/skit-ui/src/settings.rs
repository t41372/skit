//! Frontend-neutral entry settings and preset-management state.
//!
//! Every section is a group of [`Field`] values, so the dirty guard is [`any_dirty`] over the
//! whole screen and nothing re-derives a baseline at save time. Version 0.4 captures its baseline
//! when the screen opens for exactly that reason (`src/skit/tui_settings.py:832-844`).
//!
//! A section that does not apply to the kind returns nothing at all. Version 0.4 returns early
//! rather than rendering an empty box (`src/skit/tui_settings.py:423-426`, `:440-446`, `:541-542`,
//! `:820-821`).

use serde::{Deserialize, Serialize};
use skit_form::field::{
    ChoiceOption, Field, FieldCapabilities, FieldKind, FieldOwner, FieldValue, TypedValue,
    any_dirty,
};

/// Stable key of the entry name field.
pub const NAME_KEY: &str = "name";
/// Stable key of the entry description field.
pub const DESCRIPTION_KEY: &str = "description";
/// Stable key of the working-directory choice.
pub const WORKDIR_KEY: &str = "workdir";
/// Stable key of the typed custom working directory.
pub const WORKDIR_PATH_KEY: &str = "workdir:path";
/// Stable key of the interpreter override.
pub const INTERPRETER_KEY: &str = "interpreter";
/// Stable key of the prompt runner pin.
pub const RUNNER_KEY: &str = "runner";
/// Stable key of the package dependency list.
pub const DEPENDENCIES_KEY: &str = "dependencies";
/// Stable key of the Python version constraint.
pub const PYTHON_KEY: &str = "python";
/// Stable key of the required external commands.
pub const NEEDS_KEY: &str = "needs";

/// The working-directory value that means "a folder the user typed".
pub const WORKDIR_CUSTOM: &str = "custom";

/// Which package manager installs an entry's dependencies.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyFlavor {
    /// uv installs them, and a Python version constraint applies.
    Uv,
    /// npm installs them into the stored copy.
    Npm,
}

/// One section of the settings screen.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsSectionId {
    /// Name and description.
    Basics,
    /// How the entry stores its source.
    Storage,
    /// Where it runs and what runs it.
    Launch,
    /// Which agent a prompt runs with.
    Runner,
    /// Package dependencies and any language version constraint.
    Dependencies,
    /// External commands the launch requires.
    Needs,
}

impl SettingsSectionId {
    /// Return the catalog key of this section's heading.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Basics => "Basics",
            Self::Storage => "Storage",
            Self::Launch => "Run in (working directory)",
            Self::Runner => "Runner (the agent this prompt runs with)",
            Self::Dependencies => "Dependencies",
            Self::Needs => "Needs (external commands)",
        }
    }
}

/// One rendered section: a heading, any explanatory lines, and its fields.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsSection {
    /// Which section this is.
    pub id: SettingsSectionId,
    /// Explanatory lines shown under the heading, as catalog keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<SettingsNote>,
    /// Editable fields in display order.
    pub fields: Vec<Field>,
}

/// One explanatory line and the user data it names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SettingsNote {
    /// Catalog key of the sentence.
    pub text: String,
    /// Values inserted into the sentence, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<String>,
}

impl SettingsNote {
    fn plain(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            arguments: Vec::new(),
        }
    }

    fn with(text: &str, argument: impl Into<String>) -> Self {
        Self {
            text: text.to_owned(),
            arguments: vec![argument.into()],
        }
    }
}

/// Everything the settings screen needs, read once when it opens.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettingsInputs {
    /// Stable entry selector.
    pub selector: String,
    /// Entry kind.
    pub kind: String,
    /// Current display name.
    pub name: String,
    /// Current description.
    pub description: String,
    /// Original source path, empty when the entry has none.
    pub source: String,
    /// Whether the entry links its original rather than storing a copy.
    pub reference_mode: bool,
    /// Stored working-directory policy.
    pub workdir: String,
    /// Stored interpreter override.
    pub interpreter: String,
    /// Stored prompt runner pin.
    pub runner: String,
    /// Whether the kind offers copy and reference modes at all.
    pub supports_modes: bool,
    /// Whether removal leaves an original file behind.
    pub has_original_file: bool,
    /// Whether the kind keeps a stored copy under its own name.
    pub has_stored_name: bool,
    /// Whether an interpreter override applies to this kind.
    pub pinnable_interpreter: bool,
    /// Whether the kind has a source analyzer.
    pub has_analyzer: bool,
    /// Which installer serves this kind, when any does.
    pub dependency_flavor: Option<DependencyFlavor>,
    /// Effective dependencies a run would install.
    pub effective_dependencies: Vec<String>,
    /// Effective language version constraint.
    pub effective_requires_python: String,
    /// External commands the launch requires.
    pub needs: Vec<String>,
    /// Prompt runners the configuration currently defines.
    pub configured_runners: Vec<String>,
}

/// The complete settings screen as frontend-neutral state.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsView {
    /// Stable entry selector.
    pub selector: String,
    /// Entry name when the screen opened, used in the panel title.
    pub title: String,
    /// Sections in display order. A section that does not apply is absent.
    pub sections: Vec<SettingsSection>,
    /// Which installer serves this entry, when any does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_flavor: Option<DependencyFlavor>,
    /// Whether the resync chord applies to this entry.
    ///
    /// Version 0.4 advertises `Ctrl+R` only for a kind with an analyzer stored as a copy, because
    /// "advertising a key that silently no-ops … teaches a dead chord"
    /// (`src/skit/tui_settings.py:408-415`).
    pub resync_available: bool,
    /// Key of the control that owns the keyboard.
    ///
    /// Keyed by field rather than by index, for the reason the runner dropdown is: the focusable
    /// set changes while the screen is open — the custom path box appears and disappears with the
    /// working-directory choice — and an index would silently point at a different control.
    focused: String,
}

/// One refused settings value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "error")]
pub enum SettingsError {
    /// The entry name was cleared.
    NameRequired,
    /// The typed working directory is neither a known policy nor an absolute path.
    WorkdirNotAbsolute {
        /// The refused value.
        value: String,
    },
}

impl SettingsView {
    /// Build the screen for one entry.
    #[must_use]
    pub fn from_inputs(inputs: &SettingsInputs) -> Self {
        let mut sections = vec![basics_section(inputs)];
        sections.extend(storage_section(inputs));
        sections.extend(launch_section(inputs));
        sections.extend(runner_section(inputs));
        sections.extend(dependencies_section(inputs));
        sections.push(needs_section(inputs));
        Self {
            selector: inputs.selector.clone(),
            title: inputs.name.clone(),
            focused: first_focusable(&sections).unwrap_or_default(),
            sections,
            dependency_flavor: inputs.dependency_flavor,
            // The same guard version 0.4's `action_resync` applies.
            resync_available: inputs.has_analyzer && !inputs.reference_mode,
        }
    }

    /// Return one field by key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.sections
            .iter()
            .flat_map(|section| section.fields.iter())
            .find(|field| field.key == key)
    }

    /// Return one field by key for editing.
    pub fn field_mut(&mut self, key: &str) -> Option<&mut Field> {
        self.sections
            .iter_mut()
            .flat_map(|section| section.fields.iter_mut())
            .find(|field| field.key == key)
    }

    /// Return whether one section is present at all.
    #[must_use]
    pub fn has_section(&self, id: SettingsSectionId) -> bool {
        self.sections.iter().any(|section| section.id == id)
    }

    /// Report whether anything on the screen moved away from what it opened with.
    ///
    /// This is the whole dirty guard. Every field owns its own baseline, so there is no re-read to
    /// get wrong.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.sections
            .iter()
            .any(|section| any_dirty(&section.fields))
    }

    /// Report whether the typed custom path applies to the current choice.
    ///
    /// Version 0.4 shows the box only while the custom option is selected
    /// (`src/skit/tui_settings.py:483-491`).
    #[must_use]
    pub fn custom_workdir_selected(&self) -> bool {
        self.field(WORKDIR_KEY)
            .is_some_and(|field| field.value().as_text() == WORKDIR_CUSTOM)
    }

    /// Resolve the working directory a save would store, refusing an unusable one.
    ///
    /// Version 0.4 keeps the stored value when the custom box is empty, because "an empty path is
    /// not a policy", and refuses anything that is neither a known policy nor absolute — the same
    /// rule the store enforces, checked before any write (`src/skit/tui_settings.py:493-525`).
    ///
    /// # Errors
    ///
    /// Returns [`SettingsError::WorkdirNotAbsolute`] for a typed path that is not absolute.
    pub fn resolved_workdir(&self, stored: &str) -> Result<String, SettingsError> {
        let Some(choice) = self.field(WORKDIR_KEY) else {
            return Ok(String::new());
        };
        let picked = choice.value().as_text();
        if picked != WORKDIR_CUSTOM {
            return Ok(picked);
        }
        let typed = self
            .field(WORKDIR_PATH_KEY)
            .map(|field| field.value().as_text())
            .unwrap_or_default();
        let typed = typed.trim();
        if typed.is_empty() {
            return Ok(stored.to_owned());
        }
        if matches!(typed, "origin" | "store" | "invoke") || is_absolute(typed) {
            return Ok(typed.to_owned());
        }
        Err(SettingsError::WorkdirNotAbsolute {
            value: typed.to_owned(),
        })
    }

    /// Return the dependency list a save should write, or `None` when the axis was untouched.
    ///
    /// Version 0.4 compares against the effective values the screen displayed and travels `None`
    /// for an unchanged axis, so a deps-only save cannot wipe a pin the screen never showed
    /// (`src/skit/tui_settings.py:997-1001`).
    #[must_use]
    pub fn dependencies_edit(&self) -> Option<Vec<String>> {
        let field = self.field(DEPENDENCIES_KEY)?;
        if !field.is_dirty() {
            return None;
        }
        // The two grammars need different splitters. A PEP 508 requirement carries commas inside
        // its own specifier, and the PEP 508 splitter would merge a scoped npm package into its
        // neighbour (`src/skit/tui_settings.py:988-993`).
        Some(match self.dependency_flavor {
            Some(DependencyFlavor::Uv) => {
                crate::add::split_pep508_requirements(&field.value().as_text())
            }
            _ => split_list(&field.value().as_text()),
        })
    }

    /// Return the language constraint a save should write, or `None` when it was untouched.
    #[must_use]
    pub fn requires_python_edit(&self) -> Option<String> {
        let field = self.field(PYTHON_KEY)?;
        if !field.is_dirty() {
            return None;
        }
        let typed = field.value().as_text().trim().to_owned();
        // The add ask's token for "automatic", honored on this intake too
        // (`src/skit/tui_settings.py:971-973`).
        Some(if matches!(typed.to_lowercase().as_str(), "-" | "none") {
            String::new()
        } else {
            typed
        })
    }

    /// Return the external commands a save should store.
    #[must_use]
    pub fn needs_edit(&self) -> Vec<String> {
        self.field(NEEDS_KEY)
            .map(|field| split_list(&field.value().as_text()))
            .unwrap_or_default()
    }

    /// Return the keys a person can move the keyboard to, in display order.
    ///
    /// A read-only row is not a stop: it has nothing to edit, and stopping there would make the
    /// user tab past text. The custom path box is a stop only while the custom option is chosen,
    /// which is exactly when version 0.4 shows it (`src/skit/tui_settings.py:483-491`).
    #[must_use]
    pub fn focusable_keys(&self) -> Vec<&str> {
        let custom = self.custom_workdir_selected();
        self.sections
            .iter()
            .flat_map(|section| section.fields.iter())
            .filter(|field| field.kind.editable())
            .filter(|field| custom || field.key != WORKDIR_PATH_KEY)
            .map(|field| field.key.as_str())
            .collect()
    }

    /// Return the key of the control that owns the keyboard.
    ///
    /// Falls back to the first stop when the focused control stopped being one, which is what
    /// happens the moment the working directory moves off custom.
    #[must_use]
    pub fn focused(&self) -> &str {
        let keys = self.focusable_keys();
        if keys.iter().any(|key| *key == self.focused) {
            return &self.focused;
        }
        keys.first().copied().unwrap_or_default()
    }

    /// Move the keyboard to one control, if it is a stop.
    pub fn focus(&mut self, key: &str) -> bool {
        if self.focusable_keys().contains(&key) {
            self.focused = key.to_owned();
            return true;
        }
        false
    }

    /// Move the keyboard one stop forward or back, clamping at each end.
    pub fn move_focus(&mut self, forward: bool) {
        let keys = self.focusable_keys();
        let Some(last) = keys.len().checked_sub(1) else {
            return;
        };
        let current = keys
            .iter()
            .position(|key| *key == self.focused())
            .unwrap_or(0);
        let next = if forward {
            current.saturating_add(1).min(last)
        } else {
            current.saturating_sub(1)
        };
        self.focused = keys[next].to_owned();
    }

    /// Set one field's value and report whether anything changed.
    ///
    /// A read-only field refuses, so a host cannot write through a control the screen never drew.
    pub fn set_value(&mut self, key: &str, value: FieldValue) -> bool {
        self.field_mut(key)
            .is_some_and(|field| field.set_value(value))
    }

    /// Validate every axis before a save writes anything.
    ///
    /// Version 0.4 completes its validation pass first and returns having written nothing on the
    /// first refusal (`src/skit/tui_settings.py:974-982`).
    ///
    /// # Errors
    ///
    /// Returns the first refusal in section order.
    pub fn validate(&self, stored_workdir: &str) -> Result<(), SettingsError> {
        let name = self
            .field(NAME_KEY)
            .map(|field| field.value().as_text())
            .unwrap_or_default();
        if name.trim().is_empty() {
            return Err(SettingsError::NameRequired);
        }
        self.resolved_workdir(stored_workdir)?;
        Ok(())
    }
}

/// Return the first control a person can move the keyboard to.
fn first_focusable(sections: &[SettingsSection]) -> Option<String> {
    sections
        .iter()
        .flat_map(|section| section.fields.iter())
        .find(|field| field.kind.editable())
        .map(|field| field.key.clone())
}

fn basics_section(inputs: &SettingsInputs) -> SettingsSection {
    SettingsSection {
        id: SettingsSectionId::Basics,
        notes: vec![SettingsNote::plain(
            "Renaming keeps everything — remembered values, presets, the stored copy.",
        )],
        fields: vec![
            Field::new(
                NAME_KEY,
                "Name",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.name),
            ),
            Field::new(
                DESCRIPTION_KEY,
                "Description (shown in the Library)",
                FieldKind::Multiline,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.description),
            ),
        ],
    }
}

/// Storage explains the mode and names the original. A kind with one mode has nothing to say.
fn storage_section(inputs: &SettingsInputs) -> Option<SettingsSection> {
    if !inputs.supports_modes {
        return None;
    }
    let note = if inputs.reference_mode {
        SettingsNote::with("Linked to the original: {}", &inputs.source)
    } else {
        SettingsNote::with(
            "Keep a copy — your original file is never modified. Source: {}",
            &inputs.source,
        )
    };
    Some(SettingsSection {
        id: SettingsSectionId::Storage,
        notes: vec![note],
        fields: Vec::new(),
    })
}

/// Launch offers a kind-aware working directory and, for some kinds, an interpreter.
fn launch_section(inputs: &SettingsInputs) -> Option<SettingsSection> {
    // An unknown kind gets no policy controls a newer skit might define
    // (`src/skit/tui_settings.py:444-445`).
    if inputs.kind.is_empty() {
        return None;
    }
    let mut options = Vec::new();
    // A command template has no folder of its own, and a reference-only program has no stored
    // copy — offering either would be a label that resolves as something else
    // (`src/skit/tui_settings.py:448-457`).
    if inputs.has_original_file {
        options.push(ChoiceOption::labelled("origin", "The source file's folder"));
    }
    if inputs.has_stored_name {
        options.push(ChoiceOption::labelled("store", "skit's stored-copy folder"));
    }
    options.push(ChoiceOption::labelled(
        "invoke",
        "Wherever skit is run from",
    ));
    options.push(ChoiceOption::labelled(
        WORKDIR_CUSTOM,
        "A fixed folder (type it below)",
    ));

    let known = options
        .iter()
        .any(|option| option.value == inputs.workdir && option.value != WORKDIR_CUSTOM);
    let selected = if known {
        inputs.workdir.clone()
    } else {
        WORKDIR_CUSTOM.to_owned()
    };
    let mut fields = vec![
        Field::new(
            WORKDIR_KEY,
            "Run in (working directory)",
            FieldKind::SingleChoice { options },
            FieldOwner::EntryPolicy,
            FieldValue::Explicit(TypedValue::Choice(selected)),
        ),
        Field::new(
            WORKDIR_PATH_KEY,
            "/absolute/path",
            FieldKind::Path { directory: true },
            FieldOwner::EntryPolicy,
            FieldValue::text(if known { "" } else { inputs.workdir.as_str() }),
        )
        .with_capabilities(FieldCapabilities {
            browse: true,
            ..FieldCapabilities::default()
        }),
    ];
    if inputs.pinnable_interpreter {
        fields.push(
            Field::new(
                INTERPRETER_KEY,
                "Interpreter / runtime",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.interpreter),
            )
            .with_help("empty = automatic (shebang, then detection order)"),
        );
    }
    Some(SettingsSection {
        id: SettingsSectionId::Launch,
        notes: Vec::new(),
        fields,
    })
}

/// Only a prompt pins an agent.
fn runner_section(inputs: &SettingsInputs) -> Option<SettingsSection> {
    if inputs.kind != "prompt" {
        return None;
    }
    // A pin whose configuration row is gone keeps its own option and stays selected, so opening
    // settings and saving something unrelated never silently clears it
    // (`src/skit/tui_settings.py:547-557`).
    let stale = !inputs.runner.is_empty() && !inputs.configured_runners.contains(&inputs.runner);
    let mut options = vec![ChoiceOption::labelled("", "ask on the run form")];
    if stale {
        options.push(
            ChoiceOption::labelled(&inputs.runner, "{} (no longer configured)")
                .with_detail(&inputs.runner),
        );
    }
    options.extend(inputs.configured_runners.iter().map(ChoiceOption::plain));
    Some(SettingsSection {
        id: SettingsSectionId::Runner,
        notes: Vec::new(),
        fields: vec![
            // Value-keyed, so a runner list that changed while the screen was open can never shift
            // an index mapping (`src/skit/tui_settings.py:554-556`).
            Field::new(
                RUNNER_KEY,
                "Runner (the agent this prompt runs with)",
                FieldKind::SingleChoice { options },
                FieldOwner::EntryPolicy,
                FieldValue::Explicit(TypedValue::Choice(inputs.runner.clone())),
            )
            // Custom agents are first-class: the door to define one is always present, even when
            // the configured list was deliberately emptied (`:558-562`).
            .with_capabilities(FieldCapabilities {
                new_runner: true,
                ..FieldCapabilities::default()
            }),
        ],
    })
}

/// Dependencies apply only where an installer serves the kind.
fn dependencies_section(inputs: &SettingsInputs) -> Option<SettingsSection> {
    // An npm entry in reference mode runs from its own project, whose node_modules already serves
    // it, so offering the field would record dependencies the launch never uses
    // (`src/skit/tui_settings.py:850-857`).
    let flavor = inputs.dependency_flavor?;
    if flavor == DependencyFlavor::Npm && inputs.reference_mode {
        return None;
    }
    // Effective values, never raw meta: an add-time pin lives in the stored copy's own block with
    // meta deliberately blank, and prefilling from meta made "untouched blank" indistinguishable
    // from "user cleared" (`src/skit/tui_settings.py:822-834`). The same read is the baseline,
    // because `Field` keeps whatever it opened with.
    let mut fields = vec![
        Field::new(
            DEPENDENCIES_KEY,
            "Dependencies",
            FieldKind::Text,
            FieldOwner::EntryPolicy,
            FieldValue::text(inputs.effective_dependencies.join(", ")),
        )
        .with_help(match flavor {
            DependencyFlavor::Uv => "comma separated, e.g. requests>=2,<3, rich",
            DependencyFlavor::Npm => "comma separated, e.g. chalk@^5, zod",
        }),
    ];
    if flavor == DependencyFlavor::Uv {
        fields.push(
            Field::new(
                PYTHON_KEY,
                "Python constraint",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.effective_requires_python),
            )
            .with_help("Python constraint, e.g. \">=3.11\" (empty = automatic)"),
        );
    }
    Some(SettingsSection {
        id: SettingsSectionId::Dependencies,
        notes: Vec::new(),
        fields,
    })
}

/// Every kind can require external commands, so this section always applies.
fn needs_section(inputs: &SettingsInputs) -> SettingsSection {
    SettingsSection {
        id: SettingsSectionId::Needs,
        notes: Vec::new(),
        fields: vec![
            Field::new(
                NEEDS_KEY,
                "Needs (external commands)",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(inputs.needs.join(", ")),
            )
            .with_help("comma separated, e.g. ffmpeg, jq"),
        ],
    }
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn is_absolute(value: &str) -> bool {
    value.starts_with('/')
        || value.starts_with('~')
        || value
            .as_bytes()
            .get(1)
            .is_some_and(|byte| *byte == b':' && value.len() > 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python_inputs() -> SettingsInputs {
        SettingsInputs {
            selector: "tool".to_owned(),
            kind: "python".to_owned(),
            name: "Tool".to_owned(),
            source: "/home/ada/tool.py".to_owned(),
            workdir: "invoke".to_owned(),
            supports_modes: true,
            has_original_file: true,
            has_stored_name: true,
            has_analyzer: true,
            dependency_flavor: Some(DependencyFlavor::Uv),
            effective_dependencies: vec!["requests>=2,<3".to_owned()],
            effective_requires_python: ">=3.11".to_owned(),
            ..SettingsInputs::default()
        }
    }

    /// A section that does not apply is absent, not empty.
    ///
    /// Version 0.4 returns early rather than rendering a heading with nothing under it
    /// (`src/skit/tui_settings.py:423-426`, `:541-542`, `:820-821`).
    #[test]
    fn a_section_that_does_not_apply_is_absent_rather_than_empty() {
        let view = SettingsView::from_inputs(&python_inputs());
        // A python entry pins no agent.
        assert!(!view.has_section(SettingsSectionId::Runner));
        // Every present section carries either a field or something to say.
        for section in &view.sections {
            assert!(
                !section.fields.is_empty() || !section.notes.is_empty(),
                "{:?} rendered an empty box",
                section.id
            );
        }

        // A command template has one storage mode and no installer.
        let command = SettingsInputs {
            kind: "command".to_owned(),
            supports_modes: false,
            has_original_file: false,
            has_stored_name: false,
            has_analyzer: false,
            dependency_flavor: None,
            ..python_inputs()
        };
        let view = SettingsView::from_inputs(&command);
        assert!(!view.has_section(SettingsSectionId::Storage));
        assert!(!view.has_section(SettingsSectionId::Dependencies));
        // Needs applies to every kind: a command template can require ffmpeg too.
        assert!(view.has_section(SettingsSectionId::Needs));
    }

    /// The working-directory options follow the kind, and custom holds an unknown stored value.
    #[test]
    fn the_working_directory_offers_only_the_places_this_kind_has() {
        let view = SettingsView::from_inputs(&python_inputs());
        let FieldKind::SingleChoice { options } = &view.field(WORKDIR_KEY).unwrap().kind else {
            panic!("the working directory needs a closed option set");
        };
        let values = options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(values, ["origin", "store", "invoke", WORKDIR_CUSTOM]);

        // A command template has no folder of its own and no stored copy.
        let command = SettingsInputs {
            kind: "command".to_owned(),
            has_original_file: false,
            has_stored_name: false,
            ..python_inputs()
        };
        let view = SettingsView::from_inputs(&command);
        let FieldKind::SingleChoice { options } = &view.field(WORKDIR_KEY).unwrap().kind else {
            panic!("the working directory needs a closed option set");
        };
        let values = options
            .iter()
            .map(|option| option.value.as_str())
            .collect::<Vec<_>>();
        assert_eq!(values, ["invoke", WORKDIR_CUSTOM]);
    }

    /// A stored working directory that is none of the known policies preselects custom.
    #[test]
    fn a_typed_working_directory_preselects_custom_and_shows_its_box() {
        let view = SettingsView::from_inputs(&SettingsInputs {
            workdir: "/srv/jobs".to_owned(),
            ..python_inputs()
        });
        assert_eq!(view.field(WORKDIR_KEY).unwrap().value().as_text(), "custom");
        assert!(view.custom_workdir_selected());
        assert_eq!(
            view.field(WORKDIR_PATH_KEY).unwrap().value().as_text(),
            "/srv/jobs"
        );

        // A known policy leaves the box empty and hidden.
        let view = SettingsView::from_inputs(&python_inputs());
        assert!(!view.custom_workdir_selected());
        assert_eq!(view.field(WORKDIR_PATH_KEY).unwrap().value().as_text(), "");
    }

    /// An unusable typed folder is refused before anything is written, and a blank keeps the stored
    /// value because an empty path is not a policy.
    #[test]
    fn the_working_directory_is_validated_before_any_write() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        view.field_mut(WORKDIR_KEY)
            .unwrap()
            .set_value(FieldValue::Explicit(TypedValue::Choice(
                WORKDIR_CUSTOM.to_owned(),
            )));

        view.field_mut(WORKDIR_PATH_KEY)
            .unwrap()
            .set_value(FieldValue::text("relative/path"));
        assert_eq!(
            view.validate("invoke"),
            Err(SettingsError::WorkdirNotAbsolute {
                value: "relative/path".to_owned()
            })
        );

        view.field_mut(WORKDIR_PATH_KEY)
            .unwrap()
            .set_value(FieldValue::text("  "));
        assert_eq!(view.resolved_workdir("invoke").unwrap(), "invoke");

        view.field_mut(WORKDIR_PATH_KEY)
            .unwrap()
            .set_value(FieldValue::text("/srv/jobs"));
        assert_eq!(view.resolved_workdir("invoke").unwrap(), "/srv/jobs");
        assert!(view.validate("invoke").is_ok());
    }

    /// A cleared name is refused before a save writes anything.
    #[test]
    fn a_cleared_name_is_refused_in_the_validation_pass() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        view.field_mut(NAME_KEY)
            .unwrap()
            .set_value(FieldValue::text("  "));
        assert_eq!(view.validate("invoke"), Err(SettingsError::NameRequired));
    }

    /// A pin whose configuration row is gone stays selected and says so.
    #[test]
    fn a_stale_runner_pin_survives_an_unrelated_save() {
        let view = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            runner: "retired".to_owned(),
            configured_runners: vec!["claude".to_owned()],
            ..python_inputs()
        });
        let field = view.field(RUNNER_KEY).unwrap();
        assert_eq!(field.value().as_text(), "retired");
        let FieldKind::SingleChoice { options } = &field.kind else {
            panic!("the runner needs a closed option set");
        };
        // Value-keyed: the pin is present as its own option, so no index can shift it.
        let stale = options
            .iter()
            .find(|option| option.value == "retired")
            .expect("the stale pin lost its option");
        assert_eq!(stale.label, "{} (no longer configured)");
        assert_eq!(stale.detail, "retired");
        assert_eq!(options[0].value, "", "the opt-out option comes first");
        // Nothing moved, so an unrelated save writes no runner change.
        assert!(!view.is_dirty());

        // The door to define a new agent is present even with no configured runners at all.
        let empty = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            configured_runners: Vec::new(),
            ..python_inputs()
        });
        assert!(empty.field(RUNNER_KEY).unwrap().capabilities.new_runner);
    }

    /// Dependencies prefill from the effective values and use that same read as the baseline.
    #[test]
    fn dependencies_prefill_from_effective_values_and_travel_only_when_touched() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        assert_eq!(
            view.field(DEPENDENCIES_KEY).unwrap().value().as_text(),
            "requests>=2,<3"
        );
        assert_eq!(view.field(PYTHON_KEY).unwrap().value().as_text(), ">=3.11");

        // Untouched axes travel as "do not touch", so a save cannot wipe a pin nobody edited.
        assert_eq!(view.dependencies_edit(), None);
        assert_eq!(view.requires_python_edit(), None);

        view.field_mut(DEPENDENCIES_KEY)
            .unwrap()
            .set_value(FieldValue::text("requests>=2,<3, rich"));
        assert_eq!(
            view.dependencies_edit(),
            Some(vec!["requests>=2,<3".to_owned(), "rich".to_owned()])
        );
        // The constraint stayed untouched even though the other axis moved.
        assert_eq!(view.requires_python_edit(), None);

        // The add ask's "automatic" token clears the constraint.
        view.field_mut(PYTHON_KEY)
            .unwrap()
            .set_value(FieldValue::text("none"));
        assert_eq!(view.requires_python_edit(), Some(String::new()));
    }

    /// An npm entry that runs from its own project is offered no dependency field.
    #[test]
    fn a_linked_npm_entry_records_no_dependencies_the_launch_would_ignore() {
        let linked = SettingsInputs {
            kind: "js".to_owned(),
            dependency_flavor: Some(DependencyFlavor::Npm),
            reference_mode: true,
            ..python_inputs()
        };
        assert!(!SettingsView::from_inputs(&linked).has_section(SettingsSectionId::Dependencies));

        let stored = SettingsInputs {
            reference_mode: false,
            ..linked
        };
        let view = SettingsView::from_inputs(&stored);
        assert!(view.has_section(SettingsSectionId::Dependencies));
        // npm has no language constraint field.
        assert!(view.field(PYTHON_KEY).is_none());
    }

    /// The resync chord is advertised only where it does something.
    #[test]
    fn resync_is_advertised_only_for_an_analyzed_copy() {
        assert!(SettingsView::from_inputs(&python_inputs()).resync_available);
        assert!(
            !SettingsView::from_inputs(&SettingsInputs {
                reference_mode: true,
                ..python_inputs()
            })
            .resync_available
        );
        assert!(
            !SettingsView::from_inputs(&SettingsInputs {
                has_analyzer: false,
                ..python_inputs()
            })
            .resync_available
        );
    }

    /// The dirty guard is the fields' own baselines and nothing else.
    #[test]
    fn the_dirty_guard_reads_the_baselines_the_screen_opened_with() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        assert!(!view.is_dirty());
        view.field_mut(DESCRIPTION_KEY)
            .unwrap()
            .set_value(FieldValue::text("A tool"));
        assert!(view.is_dirty());
        // Typing the original text back is not an edit.
        view.field_mut(DESCRIPTION_KEY)
            .unwrap()
            .set_value(FieldValue::text(""));
        assert!(!view.is_dirty());
    }

    /// An interpreter override applies only where a kind can take one.
    #[test]
    fn only_a_pinnable_kind_offers_an_interpreter() {
        assert!(
            SettingsView::from_inputs(&python_inputs())
                .field(INTERPRETER_KEY)
                .is_none()
        );
        let shell = SettingsInputs {
            kind: "shell".to_owned(),
            pinnable_interpreter: true,
            ..python_inputs()
        };
        assert!(
            SettingsView::from_inputs(&shell)
                .field(INTERPRETER_KEY)
                .is_some()
        );
    }

    /// The keyboard stops only where there is something to edit.
    #[test]
    fn focus_skips_a_row_with_nothing_to_edit() {
        let view = SettingsView::from_inputs(&python_inputs());
        let stops = view.focusable_keys();
        assert_eq!(stops.first(), Some(&NAME_KEY));
        // Storage is explanatory text; it contributes no stop.
        assert!(!stops.iter().any(|key| key.contains("storage")));
        // Every stop is an editable control.
        for key in &stops {
            assert!(
                view.field(key).unwrap().kind.editable(),
                "{key} is not editable"
            );
        }
    }

    /// The custom path box is a stop only while the custom option is chosen.
    ///
    /// The focusable set therefore changes while the screen is open, which is why the cursor is
    /// keyed by field rather than by index.
    #[test]
    fn the_custom_path_joins_and_leaves_the_focus_order_with_its_choice() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        assert!(!view.focusable_keys().contains(&WORKDIR_PATH_KEY));

        view.set_value(
            WORKDIR_KEY,
            FieldValue::Explicit(TypedValue::Choice(WORKDIR_CUSTOM.to_owned())),
        );
        assert!(view.focusable_keys().contains(&WORKDIR_PATH_KEY));

        // Focus the box, then move the choice away: the cursor must not point at a control that
        // is no longer there.
        assert!(view.focus(WORKDIR_PATH_KEY));
        assert_eq!(view.focused(), WORKDIR_PATH_KEY);
        view.set_value(
            WORKDIR_KEY,
            FieldValue::Explicit(TypedValue::Choice("invoke".to_owned())),
        );
        assert!(!view.focusable_keys().contains(&WORKDIR_PATH_KEY));
        assert_eq!(
            view.focused(),
            NAME_KEY,
            "the cursor pointed at a control that is no longer a stop"
        );
    }

    /// Moving the keyboard walks the stops in display order and clamps at each end.
    #[test]
    fn the_keyboard_walks_every_stop_and_clamps_at_both_ends() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        let stops = view
            .focusable_keys()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(stops.len() > 2, "{stops:?}");

        for expected in &stops {
            assert_eq!(view.focused(), expected);
            view.move_focus(true);
        }
        // Past the end it stays on the last stop rather than wrapping to the top.
        assert_eq!(view.focused(), stops.last().unwrap());

        for expected in stops.iter().rev() {
            assert_eq!(view.focused(), expected);
            view.move_focus(false);
        }
        assert_eq!(view.focused(), stops.first().unwrap());
    }

    /// A read-only control refuses a write, so nothing can reach a value the screen never offered.
    #[test]
    fn a_read_only_control_cannot_be_written_through_the_view() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        assert!(!view.set_value("missing", FieldValue::text("x")));
        assert!(view.set_value(NAME_KEY, FieldValue::text("Renamed")));
        assert_eq!(view.field(NAME_KEY).unwrap().value().as_text(), "Renamed");
    }

    /// The whole screen round-trips as data for a future non-terminal frontend.
    #[test]
    fn the_settings_view_round_trips_through_json() {
        let view = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            runner: "claude".to_owned(),
            configured_runners: vec!["claude".to_owned()],
            ..python_inputs()
        });
        let json = serde_json::to_string(&view).unwrap();
        let restored: SettingsView = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, view);
    }
}
