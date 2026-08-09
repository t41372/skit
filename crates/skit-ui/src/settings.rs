//! Frontend-neutral entry settings and preset-management state.
//!
//! Every section is a group of [`Field`] values, so the dirty guard is [`any_dirty`] over the
//! whole screen and nothing re-derives a baseline at save time. Version 0.4 captures its baseline
//! when the screen opens for exactly that reason (`src/skit/tui_settings.py:832-844`).
//!
//! A section that does not apply to the kind returns nothing at all. Version 0.4 returns early
//! rather than rendering an empty box (`src/skit/tui_settings.py:423-426`, `:440-446`, `:541-542`,
//! `:820-821`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use skit_domain::parameters::{ParamDecl, ParameterBinding};
use skit_form::{
    field::{
        ChoiceOption, Field, FieldCapabilities, FieldKind, FieldOwner, FieldValue, TypedValue,
    },
    parameter_section::{
        ParameterRow, ParameterSection, ParameterSectionContext, SourceFollowup, parameter_section,
    },
};

use crate::SubmittedValues;

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
/// Stable key of a command entry's own command line.
pub const TEMPLATE_KEY: &str = "template";
/// Stable key of a prompt's variable-insertion switch.
pub const INTERPOLATE_KEY: &str = "interpolate";
/// Stable key of the resync request.
pub const RESYNC_KEY: &str = "source:resync";
/// Stable key of the "manage these constants" offer.
pub const MANAGE_KEY: &str = "source:manage";
/// Stable key of the shell normalize offer.
pub const NORMALIZE_KEY: &str = "source:normalize";
/// Stable key of the add-a-parameter box.
pub const ADD_PARAMETER_KEY: &str = "parameter:add";
/// Stable key prefix of one preset's keep toggle. The preset's own name completes it.
pub const PRESET_PREFIX: &str = "preset:";

/// Return the key of one preset's keep toggle.
///
/// The preset is addressed by name, never by position. Version 0.4 captures the name list when the
/// screen composes for exactly this reason: "a preset added or deleted mid-session (a concurrent
/// skit preset save — the product's own agent-coexistence story) must never shift which name an
/// untick deletes" (`src/skit/tui_settings.py:812-815`).
#[must_use]
pub fn preset_key(name: &str) -> String {
    format!("{PRESET_PREFIX}{name}")
}

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
    /// The fields the run form asks for.
    Parameters,
    /// Remembered sets of run values.
    Presets,
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
            Self::Parameters => "Parameters (the run form's fields)",
            Self::Presets => "Presets",
            Self::Dependencies => "Dependencies",
            Self::Needs => "Needs (external commands)",
        }
    }
}

/// One element of a section, in display order.
///
/// Version 0.4 composes explanatory text and controls as one stream: a hint can introduce a
/// control, sit between two of them, or close the section
/// (`src/skit/tui_settings.py:588-637`, `:639-681`). Two separate lists could only put every
/// sentence at one end, which would move a sentence away from the control it explains.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "item")]
pub enum SettingsItem {
    /// One explanatory line.
    Note(SettingsNote),
    /// One control.
    Field(Box<Field>),
}

/// One rendered section: a heading and its interleaved text and controls.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SettingsSection {
    /// Which section this is.
    pub id: SettingsSectionId,
    /// Explanatory lines and controls in display order.
    pub items: Vec<SettingsItem>,
}

impl SettingsSection {
    /// Build one section from its items.
    #[must_use]
    pub const fn new(id: SettingsSectionId, items: Vec<SettingsItem>) -> Self {
        Self { id, items }
    }

    /// Return every control in display order.
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.items.iter().filter_map(|item| match item {
            SettingsItem::Field(field) => Some(field.as_ref()),
            SettingsItem::Note(_) => None,
        })
    }

    /// Return every control in display order, for editing.
    pub fn fields_mut(&mut self) -> impl Iterator<Item = &mut Field> {
        self.items.iter_mut().filter_map(|item| match item {
            SettingsItem::Field(field) => Some(field.as_mut()),
            SettingsItem::Note(_) => None,
        })
    }
}

impl SettingsItem {
    /// Build one explanatory line with no inserted value.
    #[must_use]
    pub fn note(text: &str) -> Self {
        Self::Note(SettingsNote::plain(text))
    }

    /// Build one explanatory line that names user data.
    #[must_use]
    pub fn note_with(text: &str, argument: impl Into<String>) -> Self {
        Self::Note(SettingsNote::with(text, argument))
    }

    /// Build one control.
    #[must_use]
    pub fn field(field: Field) -> Self {
        Self::Field(Box::new(field))
    }
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
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// Whether the kind keeps its parameter schema in entry metadata rather than in the source.
    pub declared_schema: bool,
    /// How many fields the script's own command-line reader models.
    pub reader_fields: usize,
    /// Parameters the entry declares, from the source block or from entry metadata.
    pub managed: Vec<ParamDecl>,
    /// Constants the analyzer found that nobody manages yet, in detection order.
    pub candidates: Vec<String>,
    /// A command entry's own command line.
    pub template: String,
    /// Whether a prompt turns its `{{name}}` placeholders into form fields.
    pub interpolate: bool,
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
    /// Remembered value sets, by preset name and then by parameter name.
    pub presets: BTreeMap<String, BTreeMap<String, String>>,
    /// Section the screen was opened to show, when a deep link named one.
    ///
    /// Version 0.4's Library gives `s` to `action_settings(section="presets")`
    /// (`src/skit/tui.py:991-992`), and the screen puts that section under the eye on mount
    /// (`src/skit/tui_settings.py:876-882`).
    pub revealed: Option<SettingsSectionId>,
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
    /// (`src/skit/tui_settings.py:408-415`). The chord reaches [`RESYNC_KEY`], which exists only
    /// under the same condition, so the guard and the control cannot drift apart.
    pub resync_available: bool,
    /// Key of the control that owns the keyboard.
    ///
    /// Keyed by field rather than by index, for the reason the runner dropdown is: the focusable
    /// set changes while the screen is open — the custom path box appears and disappears with the
    /// working-directory choice — and an index would silently point at a different control.
    focused: String,
    /// Section a deep link asked the screen to show, until the keyboard moves.
    ///
    /// Version 0.4 scrolls the body to the named section on mount and then leaves the viewport to
    /// the reader (`src/skit/tui_settings.py:876-882`). Keeping it until the first keyboard move
    /// says the same thing without a render-time flag: the anchor is state a person releases, not a
    /// note one pass leaves for another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    revealed: Option<SettingsSectionId>,
    /// The working directory the entry had when the screen opened.
    ///
    /// Version 0.4 keeps this value when the custom box is chosen and left blank, because "an
    /// empty path is not a policy" (`src/skit/tui_settings.py:508-512`). Keeping it here means the
    /// host never re-reads it, so a concurrent write cannot become the fallback.
    stored_workdir: String,
}

/// One typed edit to the settings screen.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "action")]
pub enum SettingsAction {
    /// Replace one field's value.
    SetField {
        /// Stable field key.
        key: String,
        /// New value.
        value: FieldValue,
    },
    /// Move the keyboard to one control.
    Focus {
        /// Stable field key.
        key: String,
    },
    /// Move the keyboard one stop forward.
    FocusNext,
    /// Move the keyboard one stop back.
    FocusPrevious,
    /// Ask the script for its parameter definitions again when the save runs.
    Resync,
    /// Save every axis, after validation.
    Save,
    /// Leave the screen, through the discard guard when anything moved.
    Close,
    /// Define a new prompt runner without leaving the screen.
    NewRunner,
}

/// What the host must do after one settings edit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SettingsEffect {
    /// Nothing outside the screen.
    None,
    /// Persist every axis.
    Save,
    /// Refuse the save and say why. Nothing was written.
    Refused(SettingsError),
    /// Leave the screen; nothing moved.
    Close,
    /// Ask before dropping unsaved work.
    ConfirmDiscard,
    /// Open the runner editor.
    NewRunner,
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

impl SettingsError {
    /// Return the catalog key of the sentence a person reads.
    ///
    /// Both are version 0.4's own wording (`src/skit/tui_settings.py:517-522` and its `A name is
    /// required.` notice), so the shipped translations apply unchanged.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::NameRequired => "A name is required.",
            Self::WorkdirNotAbsolute { .. } => {
                "The working directory must be origin, store, invoke, or an absolute path."
            }
        }
    }
}

impl SettingsView {
    /// Build the screen for one entry.
    #[must_use]
    pub fn from_inputs(inputs: &SettingsInputs) -> Self {
        // The same guard version 0.4's `action_resync` applies.
        let resync_available = inputs.has_analyzer && !inputs.reference_mode;
        let mut sections = vec![basics_section(inputs)];
        sections.extend(storage_section(inputs));
        sections.extend(launch_section(inputs));
        sections.extend(runner_section(inputs));
        sections.extend(parameters_section(inputs, resync_available));
        sections.push(presets_section(inputs));
        sections.extend(dependencies_section(inputs));
        sections.push(needs_section(inputs));
        // A deep link that names a section this entry does not have is not an anchor. Resolving it
        // against the sections that exist keeps the screen from anchoring to nothing.
        let revealed = inputs
            .revealed
            .filter(|id| sections.iter().any(|section| section.id == *id));
        let mut view = Self {
            selector: inputs.selector.clone(),
            title: inputs.name.clone(),
            focused: String::new(),
            stored_workdir: inputs.workdir.clone(),
            sections,
            dependency_flavor: inputs.dependency_flavor,
            resync_available,
            revealed,
        };
        // The keyboard lands in the section the deep link named, so the first key press acts on
        // what the user came for. A section with nothing to edit keeps the anchor instead.
        view.focused = view
            .revealed
            .and_then(|id| view.first_stop_in(id))
            .or_else(|| view.focusable_keys().first().map(|key| (*key).to_owned()))
            .unwrap_or_default();
        view
    }

    /// Return the first control a person can reach inside one section.
    fn first_stop_in(&self, id: SettingsSectionId) -> Option<String> {
        let stops = self.focusable_keys();
        self.sections
            .iter()
            .find(|section| section.id == id)?
            .fields()
            .find(|field| stops.contains(&field.key.as_str()))
            .map(|field| field.key.clone())
    }

    /// Return the section the screen was opened to show, until the keyboard moves.
    #[must_use]
    pub const fn revealed(&self) -> Option<SettingsSectionId> {
        self.revealed
    }

    /// Return every control on the screen in display order.
    pub fn fields(&self) -> impl Iterator<Item = &Field> {
        self.sections.iter().flat_map(SettingsSection::fields)
    }

    /// Return one field by key.
    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Field> {
        self.fields().find(|field| field.key == key)
    }

    /// Return one field by key for editing.
    pub fn field_mut(&mut self, key: &str) -> Option<&mut Field> {
        self.sections
            .iter_mut()
            .flat_map(SettingsSection::fields_mut)
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
        // Every control counts, including one the screen currently hides. Version 0.4 snapshots the
        // widget tree, and a widget with `display = False` is still mounted with its edited value
        // (`src/skit/tui_settings.py:832-844`, `:900-904`). Leaving on a hidden edit must still ask.
        self.fields().any(Field::is_dirty)
    }

    /// Report whether a prompt currently turns its placeholders into form fields.
    ///
    /// Version 0.4 hides the whole declared-parameter body while insertion is off and skips
    /// collecting it on save, because those rows describe a form that does not exist
    /// (`src/skit/tui_settings.py:678-681`, `:900-904`, `:953`).
    fn insertion_on(&self) -> bool {
        self.field(INTERPOLATE_KEY)
            .is_none_or(|field| field.value().as_text() == "true")
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
        let stored = if stored.is_empty() {
            self.stored_workdir.as_str()
        } else {
            stored
        };
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
    /// which is exactly when version 0.4 shows it (`src/skit/tui_settings.py:483-491`), and a
    /// prompt's parameter rows are stops only while variable insertion is on (`:900-904`).
    #[must_use]
    pub fn focusable_keys(&self) -> Vec<&str> {
        let custom = self.custom_workdir_selected();
        let insertion = self.insertion_on();
        self.fields()
            .filter(|field| field.kind.editable())
            .filter(|field| custom || field.key != WORKDIR_PATH_KEY)
            .filter(|field| insertion || !hidden_while_insertion_is_off(&field.key))
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
            // The reader took over, so the deep link stops holding the viewport.
            self.revealed = None;
            return true;
        }
        false
    }

    /// Move the keyboard one stop forward or back, clamping at each end.
    pub fn move_focus(&mut self, forward: bool) {
        self.revealed = None;
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

    /// Put a newly defined agent in the picker and select it.
    ///
    /// Version 0.4 rebuilds the option list exactly as the screen composed it and then selects the
    /// new name, so the agent is pinned by the next save without a second visit
    /// (`src/skit/tui_settings.py:564-586`).
    ///
    /// The selector guard rejects a save that arrived for a different entry, which is the same
    /// check the launch form's picker makes.
    pub fn add_and_select_runner(&mut self, selector: &str, runner: String) {
        if self.selector != selector {
            return;
        }
        let Some(field) = self.field_mut(RUNNER_KEY) else {
            return;
        };
        if let FieldKind::SingleChoice { options } = &mut field.kind
            && !options.iter().any(|option| option.value == runner)
        {
            options.push(ChoiceOption::plain(&runner));
        }
        field.set_value(FieldValue::Explicit(TypedValue::Choice(runner)));
    }

    /// Apply one typed edit and report what the host must do.
    ///
    /// `Close` routes through the discard guard whenever anything moved, so leaving never drops
    /// work silently (`src/skit/tui_settings.py:43-46` is the guard it opens).
    pub fn update(&mut self, action: SettingsAction) -> SettingsEffect {
        match action {
            // Touching a control moves the keyboard to it. A click that changed a value and left
            // the cursor elsewhere would send the next arrow key to a control the user is not
            // looking at. Typing already satisfies this, so it changes nothing there.
            SettingsAction::SetField { key, value } => {
                if self.set_value(&key, value) {
                    self.focus(&key);
                }
                SettingsEffect::None
            }
            SettingsAction::Focus { key } => {
                self.focus(&key);
                SettingsEffect::None
            }
            SettingsAction::FocusNext => {
                self.move_focus(true);
                SettingsEffect::None
            }
            SettingsAction::FocusPrevious => {
                self.move_focus(false);
                SettingsEffect::None
            }
            // The chord reaches the same control a click does, and it exists only where a resync
            // does something (`src/skit/tui_settings.py:408-415`). A screen without the control
            // never advertises the chord, so this can never be the dead chord that guard forbids.
            SettingsAction::Resync => {
                let Some(field) = self.field(RESYNC_KEY) else {
                    return SettingsEffect::None;
                };
                let next = field.value().as_text() != "true";
                self.set_value(RESYNC_KEY, FieldValue::boolean(next));
                self.focus(RESYNC_KEY);
                SettingsEffect::None
            }
            // Version 0.4 completes its validation pass before any write and returns having
            // written nothing on the first refusal (`src/skit/tui_settings.py:939-941`,
            // `:517-523`).
            SettingsAction::Save => match self.validate(&self.stored_workdir) {
                Ok(()) => SettingsEffect::Save,
                Err(error) => SettingsEffect::Refused(error),
            },
            SettingsAction::Close if self.is_dirty() => SettingsEffect::ConfirmDiscard,
            SettingsAction::Close => SettingsEffect::Close,
            SettingsAction::NewRunner if self.has_section(SettingsSectionId::Runner) => {
                SettingsEffect::NewRunner
            }
            SettingsAction::NewRunner => SettingsEffect::None,
        }
    }

    /// Return the axes a save must change, typed, keyed as the host expects.
    ///
    /// Only what moved travels. Every field owns the value it opened with, so this screen already
    /// knows which axes are edits; sending an unchanged axis would ask the host to work that out
    /// again, and the only basis it has is a fresh read — which turns an untouched axis into an
    /// edit the moment a concurrent write moves it underneath. Version 0.4 captures its baseline
    /// when the screen opens for exactly that reason (`src/skit/tui_settings.py:822-834`).
    ///
    /// A read-only control contributes nothing: it never held an edit, so submitting it would ask
    /// the host to write a value the screen only displayed.
    ///
    /// The working directory travels resolved, as one value. The choice and the typed path are two
    /// controls but one policy, and the rule that turns them into it — a known option wins, a blank
    /// custom box keeps what is stored — belongs to [`Self::resolved_workdir`]. Submitting both
    /// halves would ask every host to derive that rule again, and a host that derived it
    /// differently would write a working directory the screen never showed.
    #[must_use]
    pub fn submitted_values(&self) -> SubmittedValues {
        // Only a control a person could reach travels. A hidden control describes something the
        // screen is not currently offering — a prompt with insertion off has no run form for its
        // rows to describe — and version 0.4 skips exactly that set on save
        // (`src/skit/tui_settings.py:953`).
        let reachable = self.focusable_keys();
        let mut values = self
            .fields()
            .filter(|field| field.is_dirty() && reachable.contains(&field.key.as_str()))
            .map(|field| (field.key.clone(), field.value().clone()))
            .collect::<SubmittedValues>();
        // Dependencies travel already split, for the same reason the working directory travels
        // already resolved. Two grammars share this one control — a PEP 508 requirement carries
        // commas inside its own specifier, and the PEP 508 splitter would merge a scoped npm
        // package into its neighbour (`src/skit/tui_settings.py:988-993`) — so a host that split
        // the text itself would have to know which one applies, and a host that guessed would write
        // one bogus requirement where the screen showed a list.
        if let Some(list) = self.dependencies_edit() {
            values.insert(
                DEPENDENCIES_KEY.to_owned(),
                FieldValue::Explicit(TypedValue::Arguments(list)),
            );
        }
        // The two halves are one axis: either one moving means the policy moved.
        if values.remove(WORKDIR_PATH_KEY).is_some() || values.contains_key(WORKDIR_KEY) {
            // A refused path cannot reach here: `Save` validates first. Keeping the stored value is
            // the same answer a blank custom box gets, so nothing is invented on the way out.
            let resolved = self
                .resolved_workdir(&self.stored_workdir)
                .unwrap_or_else(|_| self.stored_workdir.clone());
            values.insert(WORKDIR_KEY.to_owned(), FieldValue::text(resolved));
        }
        values
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

/// Report whether one control describes a run form a prompt is not currently building.
///
/// Version 0.4 hides the declared-parameter body, and everything in it, while variable insertion is
/// off (`src/skit/tui_settings.py:678-681`, `:900-904`).
fn hidden_while_insertion_is_off(key: &str) -> bool {
    key.starts_with("parameter:")
}

fn basics_section(inputs: &SettingsInputs) -> SettingsSection {
    SettingsSection::new(
        SettingsSectionId::Basics,
        vec![
            SettingsItem::note(
                "Renaming keeps everything — remembered values, presets, the stored copy.",
            ),
            SettingsItem::field(Field::new(
                NAME_KEY,
                "Name",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.name),
            )),
            SettingsItem::field(Field::new(
                DESCRIPTION_KEY,
                "Description (shown in the Library)",
                FieldKind::Multiline,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.description),
            )),
        ],
    )
}

/// Storage explains the mode and names the original. A kind with one mode has nothing to say.
fn storage_section(inputs: &SettingsInputs) -> Option<SettingsSection> {
    if !inputs.supports_modes {
        return None;
    }
    let note = if inputs.reference_mode {
        SettingsItem::note_with("Linked to the original: {}", &inputs.source)
    } else {
        SettingsItem::note_with(
            "Keep a copy — your original file is never modified. Source: {}",
            &inputs.source,
        )
    };
    Some(SettingsSection::new(SettingsSectionId::Storage, vec![note]))
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
    let mut items = vec![
        SettingsItem::field(Field::new(
            WORKDIR_KEY,
            "Run in (working directory)",
            FieldKind::SingleChoice { options },
            FieldOwner::EntryPolicy,
            FieldValue::Explicit(TypedValue::Choice(selected)),
        )),
        SettingsItem::field(
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
        ),
    ];
    if inputs.pinnable_interpreter {
        items.push(SettingsItem::field(
            Field::new(
                INTERPRETER_KEY,
                "Interpreter / runtime",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.interpreter),
            )
            .with_help("empty = automatic (shebang, then detection order)"),
        ));
    }
    Some(SettingsSection::new(SettingsSectionId::Launch, items))
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
    Some(SettingsSection::new(
        SettingsSectionId::Runner,
        vec![SettingsItem::field(
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
        )],
    ))
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
    let mut items = vec![SettingsItem::field(
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
    )];
    if flavor == DependencyFlavor::Uv {
        items.push(SettingsItem::field(
            Field::new(
                PYTHON_KEY,
                "Python constraint",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(&inputs.effective_requires_python),
            )
            .with_help("Python constraint, e.g. \">=3.11\" (empty = automatic)"),
        ));
    }
    Some(SettingsSection::new(SettingsSectionId::Dependencies, items))
}

/// Presets: one toggle for each remembered value set, and untick to delete on save.
///
/// Version 0.4 lists them under one sentence and deletes whatever the user unticked
/// (`src/skit/tui_settings.py:800-818`, `:1114-1120`). Creating one belongs to the run form, so an
/// entry with none is told where to go rather than shown an empty box (`:804-808`).
///
/// Every toggle is keyed by the preset's own name. Version 0.4 captures the name list at compose
/// time for the same reason: a concurrent `skit preset save` must never shift which name an untick
/// deletes (`:812-815`).
fn presets_section(inputs: &SettingsInputs) -> SettingsSection {
    if inputs.presets.is_empty() {
        return SettingsSection::new(
            SettingsSectionId::Presets,
            vec![SettingsItem::note(
                "None yet — press Ctrl+S inside the run form to save one.",
            )],
        );
    }
    let mut items = vec![SettingsItem::note("Untick a preset to delete it on save:")];
    items.extend(inputs.presets.iter().map(|(name, values)| {
        let summary = values
            .iter()
            .map(|(field, value)| format!("{field}={value}"))
            .collect::<Vec<_>>()
            .join(", ");
        SettingsItem::field(
            Field::new(
                preset_key(name),
                format!("{name}  {summary}"),
                FieldKind::Boolean,
                FieldOwner::EntryPolicy,
                FieldValue::boolean(true),
            )
            // The label is the user's own preset name and their own values, so a preset named after
            // a catalog phrase must not come back translated.
            .with_verbatim_label()
            .with_help("delete this preset"),
        )
    }));
    SettingsSection::new(SettingsSectionId::Presets, items)
}

/// Every kind can require external commands, so this section always applies.
fn needs_section(inputs: &SettingsInputs) -> SettingsSection {
    SettingsSection::new(
        SettingsSectionId::Needs,
        vec![SettingsItem::field(
            Field::new(
                NEEDS_KEY,
                "Needs (external commands)",
                FieldKind::Text,
                FieldOwner::EntryPolicy,
                FieldValue::text(inputs.needs.join(", ")),
            )
            .with_help("comma separated, e.g. ffmpeg, jq"),
        )],
    )
}

/// The parameter section: what the run form asks for, and what the user may change about it.
///
/// Version 0.4 decides what this section *is* before it draws any row
/// (`src/skit/tui_settings.py:588-631`), which is what keeps a control the save could not keep off
/// the screen. Everything here follows that decision.
fn parameters_section(inputs: &SettingsInputs, resync_available: bool) -> Option<SettingsSection> {
    // An unknown kind gets no editor a newer skit might define.
    if inputs.kind.is_empty() {
        return None;
    }
    let section = parameter_section(
        ParameterSectionContext {
            kind: &inputs.kind,
            reference_mode: inputs.reference_mode,
            declared_schema: inputs.declared_schema,
            has_analyzer: inputs.has_analyzer,
            reader_fields: inputs.reader_fields,
        },
        &inputs.managed,
        &inputs.candidates,
    );
    let items = match section {
        // One line rather than an empty box (`src/skit/tui_settings.py:594-596`).
        ParameterSection::Unsupported => {
            vec![SettingsItem::note("(programs have no managed parameters)")]
        }
        ParameterSection::Reference { rows } => {
            let mut items = vec![SettingsItem::note(
                "skit doesn't write to this file — maintain the [tool.skit] definitions in the source directly.",
            )];
            // `· name (type)`, exactly as version 0.4 prints it (`:605-606`). The text is the
            // script's own, so it is a value and never a catalog key.
            items.extend(rows.into_iter().map(|row| {
                SettingsItem::field(
                    Field::new(
                        format!("parameter:{}:summary", row.name),
                        String::new(),
                        FieldKind::ReadOnly,
                        FieldOwner::SourceBlock,
                        FieldValue::text(format!("· {} ({})", row.name, row.parameter_type)),
                    )
                    .with_verbatim_label(),
                )
            }));
            items
        }
        ParameterSection::Declared { rows } => declared_items(inputs, rows),
        ParameterSection::SourceManaged { rows, followup } => {
            source_managed_items(inputs, rows, followup, resync_available)
        }
    };
    Some(SettingsSection::new(SettingsSectionId::Parameters, items))
}

/// The hand-declared editor: the template or the insertion switch, the rows, then the add box.
///
/// Version 0.4 composes it in this order (`src/skit/tui_settings.py:639-719`).
fn declared_items(inputs: &SettingsInputs, rows: Vec<ParameterRow>) -> Vec<SettingsItem> {
    let mut items = Vec::new();
    // The template is the program, and it stays editable: freezing it forever would force remove
    // and re-add over a typo (`src/skit/tui_settings.py:645-653`).
    if inputs.kind == "command" {
        items.push(SettingsItem::field(Field::new(
            TEMPLATE_KEY,
            "Command template",
            FieldKind::Text,
            FieldOwner::Template,
            FieldValue::text(&inputs.template),
        )));
        items.push(SettingsItem::note(
            "Saving re-reads the {placeholders} from the template.",
        ));
    }
    // The per-prompt master switch, and the sentence that says what "off" means (`:658-677`).
    if inputs.kind == "prompt" {
        items.push(SettingsItem::field(Field::new(
            INTERPOLATE_KEY,
            "Variable insertion ({{name}} placeholders become form fields)",
            FieldKind::Boolean,
            FieldOwner::EntryPolicy,
            FieldValue::boolean(inputs.interpolate),
        )));
        items.push(SettingsItem::note(
            "Off — the body travels to the agent exactly as written.",
        ));
    }
    items.extend(row_items(rows));
    items.push(SettingsItem::field(
        Field::new(
            ADD_PARAMETER_KEY,
            "Add a parameter — type a name, then Save:",
            FieldKind::Text,
            FieldOwner::Declared,
            FieldValue::text(""),
        )
        .with_help("new parameter name"),
    ));
    items
}

/// The block-managed editor: the rows, then whatever else the source allows.
fn source_managed_items(
    inputs: &SettingsInputs,
    rows: Vec<ParameterRow>,
    followup: SourceFollowup,
    resync_available: bool,
) -> Vec<SettingsItem> {
    let all_input_bound = !inputs.managed.is_empty()
        && inputs
            .managed
            .iter()
            .all(|declaration| declaration.binding == ParameterBinding::Input);
    let mut items = row_items(rows);
    match followup {
        SourceFollowup::None => {}
        // Managing a constant would write a block that shadows the script's own reader, so version
        // 0.4 explains instead of offering the checkboxes (`src/skit/tui_settings.py:612-623`).
        SourceFollowup::ReaderDriven => items.push(SettingsItem::note(
            "This script's run form comes from its own command-line arguments. Managing a hardcoded constant here would replace that form — leave it as is.",
        )),
        // The offer's own label is version 0.4's sentence above the checkboxes (`:624-631`).
        SourceFollowup::Offer { candidates } => items.push(SettingsItem::field(Field::new(
            MANAGE_KEY,
            "Detected but not yet managed — tick to manage:",
            FieldKind::MultiChoice {
                options: candidates.iter().map(ChoiceOption::plain).collect(),
            },
            FieldOwner::SourceBlock,
            FieldValue::Explicit(TypedValue::Choices(Vec::new())),
        ))),
    }
    if all_input_bound {
        items.push(SettingsItem::note(
            "Every input() is managed — this script can now run with --no-input.",
        ));
    }
    // The one opt-in semantic edit to a stored script, and the only kind it applies to. Version 0.4
    // advises the same rewrite from the command line (`src/skit/cli.py:4014`, `:4113-4116`).
    if inputs.kind == "shell" && !inputs.reference_mode {
        // A constant that already reads `${NAME:-value}` is the result of this rewrite, so offering
        // it again would be a control whose only outcome is a refusal — the normalizer refuses a
        // value that names itself (`crates/skit-language/src/semantic/shell.rs:747-751`).
        let names = inputs
            .managed
            .iter()
            .filter(|declaration| declaration.binding != ParameterBinding::EnvDefault)
            .map(|declaration| declaration.name.clone())
            .chain(inputs.candidates.iter().cloned())
            .collect::<Vec<_>>();
        if !names.is_empty() {
            items.push(SettingsItem::field(Field::new(
                NORMALIZE_KEY,
                "Change a constant to an environment default — tick to normalize:",
                FieldKind::MultiChoice {
                    options: names.iter().map(ChoiceOption::plain).collect(),
                },
                FieldOwner::SourceBlock,
                FieldValue::Explicit(TypedValue::Choices(Vec::new())),
            )));
        }
    }
    // The chord and the control appear together, so `Ctrl+R` can never be advertised for a screen
    // that has nothing to resync (`src/skit/tui_settings.py:408-415`).
    if resync_available {
        items.push(SettingsItem::field(Field::new(
            RESYNC_KEY,
            "Read the parameter definitions from the script again on save",
            FieldKind::Boolean,
            FieldOwner::SourceBlock,
            FieldValue::boolean(false),
        )));
    }
    items
}

fn row_items(rows: Vec<ParameterRow>) -> Vec<SettingsItem> {
    rows.into_iter()
        .flat_map(|row| row.fields.into_iter().map(SettingsItem::field))
        .collect()
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
                !section.items.is_empty(),
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
            declared_schema: true,
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

    /// The two working-directory controls submit one resolved policy, not two halves.
    ///
    /// Version 0.4's save reads the radio, then the box, then writes one value
    /// (`src/skit/tui_settings.py:493-528`). Submitting both halves would ask the host to derive
    /// that rule again.
    #[test]
    fn the_working_directory_travels_resolved_as_one_value() {
        // Nothing moved, so nothing travels: the host has no axis to change.
        let view = SettingsView::from_inputs(&python_inputs());
        assert!(view.submitted_values().is_empty());

        // A typed folder travels under the one key, resolved.
        let mut view = SettingsView::from_inputs(&python_inputs());
        view.set_value(
            WORKDIR_KEY,
            FieldValue::Explicit(TypedValue::Choice(WORKDIR_CUSTOM.to_owned())),
        );
        view.set_value(WORKDIR_PATH_KEY, FieldValue::text("/srv/jobs"));
        let values = view.submitted_values();
        assert_eq!(
            values.get(WORKDIR_KEY),
            Some(&FieldValue::text("/srv/jobs"))
        );
        assert!(
            !values.contains_key(WORKDIR_PATH_KEY),
            "the typed box is half a policy, not a value a host writes"
        );

        // A blank custom box keeps what is stored, which the view owns rather than re-reads.
        view.set_value(WORKDIR_PATH_KEY, FieldValue::text("   "));
        assert_eq!(
            view.submitted_values().get(WORKDIR_KEY),
            Some(&FieldValue::text("invoke"))
        );
    }

    /// Only an axis a person moved travels, and it keeps the type its control produced.
    ///
    /// The screen owns the answer to "did this move", because every field owns what it opened with.
    /// A host that worked it out again could only compare against a fresh read, which turns an
    /// untouched axis into an edit the moment a concurrent write moves it
    /// (`src/skit/tui_settings.py:822-834`).
    #[test]
    fn only_a_moved_axis_travels_and_it_keeps_its_type() {
        let mut view = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            configured_runners: vec!["claude".to_owned()],
            ..python_inputs()
        });
        assert!(view.submitted_values().is_empty(), "nothing moved");

        view.set_value(NEEDS_KEY, FieldValue::text("ffmpeg, jq"));
        let values = view.submitted_values();
        assert_eq!(values.len(), 1, "only the edited axis: {values:?}");
        assert_eq!(values.get(NEEDS_KEY), Some(&FieldValue::text("ffmpeg, jq")));

        // A picker travels as a choice, not as text a host would have to interpret.
        view.set_value(
            RUNNER_KEY,
            FieldValue::Explicit(TypedValue::Choice("claude".to_owned())),
        );
        assert_eq!(
            view.submitted_values().get(RUNNER_KEY),
            Some(&FieldValue::Explicit(TypedValue::Choice(
                "claude".to_owned()
            )))
        );

        // Clearing an axis is an edit, and it is not the same as never offering it.
        view.set_value(DESCRIPTION_KEY, FieldValue::text(""));
        assert!(!view.submitted_values().contains_key(DESCRIPTION_KEY));
        view.set_value(DESCRIPTION_KEY, FieldValue::text("A tool"));
        view.set_value(DESCRIPTION_KEY, FieldValue::text(""));
        assert!(!view.submitted_values().contains_key(DESCRIPTION_KEY));

        let mut described = SettingsView::from_inputs(&SettingsInputs {
            description: "A tool".to_owned(),
            ..python_inputs()
        });
        described.set_value(DESCRIPTION_KEY, FieldValue::text(""));
        assert_eq!(
            described.submitted_values().get(DESCRIPTION_KEY),
            Some(&FieldValue::text("")),
            "a cleared axis travels as an explicit empty value"
        );
    }

    /// A save refuses before it travels, and says why.
    ///
    /// Version 0.4 completes its validation pass first and returns having written nothing on the
    /// first refusal (`src/skit/tui_settings.py:939-941`).
    #[test]
    fn a_save_that_cannot_be_kept_never_reaches_the_host() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        assert_eq!(view.update(SettingsAction::Save), SettingsEffect::Save);

        view.set_value(
            WORKDIR_KEY,
            FieldValue::Explicit(TypedValue::Choice(WORKDIR_CUSTOM.to_owned())),
        );
        view.set_value(WORKDIR_PATH_KEY, FieldValue::text("relative/path"));
        assert_eq!(
            view.update(SettingsAction::Save),
            SettingsEffect::Refused(SettingsError::WorkdirNotAbsolute {
                value: "relative/path".to_owned()
            })
        );

        view.set_value(WORKDIR_PATH_KEY, FieldValue::text("/srv/jobs"));
        view.set_value(NAME_KEY, FieldValue::text("  "));
        assert_eq!(
            view.update(SettingsAction::Save),
            SettingsEffect::Refused(SettingsError::NameRequired)
        );
        // Every refusal carries wording a person reads.
        assert_eq!(SettingsError::NameRequired.message(), "A name is required.");
        assert!(
            SettingsError::WorkdirNotAbsolute {
                value: String::new()
            }
            .message()
            .contains("absolute path")
        );
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

    /// A newly defined agent joins the picker, is selected, and never lands on the wrong entry.
    ///
    /// Version 0.4 rebuilds the options and selects the new name so the next save pins it
    /// (`src/skit/tui_settings.py:564-586`).
    #[test]
    fn a_new_agent_joins_the_picker_and_is_selected() {
        let inputs = SettingsInputs {
            kind: "prompt".to_owned(),
            configured_runners: vec!["claude".to_owned()],
            ..python_inputs()
        };
        let mut view = SettingsView::from_inputs(&inputs);
        view.add_and_select_runner(&inputs.selector, "local".to_owned());
        let field = view.field(RUNNER_KEY).unwrap();
        assert_eq!(field.value().as_text(), "local");
        let FieldKind::SingleChoice { options } = &field.kind else {
            panic!("the runner needs a closed option set");
        };
        assert_eq!(
            options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["", "claude", "local"]
        );

        // Selecting one the picker already lists adds no duplicate row.
        view.add_and_select_runner(&inputs.selector, "claude".to_owned());
        let FieldKind::SingleChoice { options } = &view.field(RUNNER_KEY).unwrap().kind else {
            panic!("the runner needs a closed option set");
        };
        assert_eq!(options.len(), 3);
        assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "claude");

        // A response that names another entry is refused, so a screen that moved on keeps its pin.
        view.add_and_select_runner("another-entry", "stray".to_owned());
        assert_eq!(view.field(RUNNER_KEY).unwrap().value().as_text(), "claude");

        // A kind that pins no agent has no picker to write through.
        let mut python = SettingsView::from_inputs(&python_inputs());
        python.add_and_select_runner(&inputs.selector, "local".to_owned());
        assert!(python.field(RUNNER_KEY).is_none());
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

    fn declaration(name: &str) -> ParamDecl {
        ParamDecl::new(name)
    }

    fn notes(view: &SettingsView, id: SettingsSectionId) -> Vec<String> {
        view.sections
            .iter()
            .filter(|section| section.id == id)
            .flat_map(|section| section.items.iter())
            .filter_map(|item| match item {
                SettingsItem::Note(note) => Some(note.text.clone()),
                SettingsItem::Field(_) => None,
            })
            .collect()
    }

    fn parameter_keys(view: &SettingsView) -> Vec<String> {
        view.sections
            .iter()
            .filter(|section| section.id == SettingsSectionId::Parameters)
            .flat_map(SettingsSection::fields)
            .map(|field| field.key.clone())
            .collect()
    }

    /// Each shape of the parameter section says what it is, and none of them is an empty box.
    ///
    /// Version 0.4 asks what the section is before it draws any row
    /// (`src/skit/tui_settings.py:588-631`).
    #[test]
    fn the_parameter_section_is_whichever_shape_the_kind_earns() {
        // No analyzer and no declared schema: one line, no controls at all.
        let program = SettingsView::from_inputs(&SettingsInputs {
            kind: "unknown-kind".to_owned(),
            has_analyzer: false,
            ..python_inputs()
        });
        assert_eq!(
            notes(&program, SettingsSectionId::Parameters),
            ["(programs have no managed parameters)"]
        );
        assert!(parameter_keys(&program).is_empty());

        // A linked file is readable and nothing else, and each row reads out as version 0.4 prints
        // it (`:598-606`).
        let linked = SettingsView::from_inputs(&SettingsInputs {
            reference_mode: true,
            managed: vec![declaration("NAME")],
            ..python_inputs()
        });
        assert_eq!(
            notes(&linked, SettingsSectionId::Parameters),
            [
                "skit doesn't write to this file — maintain the [tool.skit] definitions in the source directly."
            ]
        );
        let row = linked.field("parameter:NAME:summary").expect("no row");
        assert_eq!(row.kind, FieldKind::ReadOnly);
        assert_eq!(row.value().as_text(), "· NAME (str)");
        assert!(!row.translate_label, "a parameter name is not catalog copy");

        // A block-managed row offers the three axes a save can keep, and its keep toggle carries
        // the name and the script's own metadata (`:99-107`).
        let managed = SettingsView::from_inputs(&SettingsInputs {
            managed: vec![declaration("GREETING")],
            ..python_inputs()
        });
        assert_eq!(
            parameter_keys(&managed),
            [
                "parameter:GREETING:keep",
                "parameter:GREETING:prompt",
                "parameter:GREETING:secret",
                "parameter:GREETING:env_source",
                RESYNC_KEY,
            ]
        );

        // A hand-declared row is the user's to change, and the add box closes the editor (`:718`).
        let declared = SettingsView::from_inputs(&SettingsInputs {
            kind: "exe".to_owned(),
            declared_schema: true,
            has_analyzer: false,
            managed: vec![declaration("output")],
            ..python_inputs()
        });
        assert_eq!(
            parameter_keys(&declared),
            [
                "parameter:output:keep",
                "parameter:output:type",
                "parameter:output:default",
                "parameter:output:choices",
                "parameter:output:help",
                "parameter:output:flag",
                "parameter:output:required",
                "parameter:output:prompt",
                "parameter:output:secret",
                "parameter:output:env_source",
                ADD_PARAMETER_KEY,
            ]
        );
    }

    /// The offer to manage a constant is a closed option set, and it never appears where managing
    /// one would replace the script's own form.
    ///
    /// Version 0.4 draws a checkbox for each candidate under one sentence (`:624-631`), and it
    /// prints an explanation instead when the script's own reader owns the form (`:610-623`).
    #[test]
    fn the_manage_offer_is_a_closed_set_and_a_reader_driven_script_gets_a_sentence() {
        let offered = SettingsView::from_inputs(&SettingsInputs {
            managed: vec![declaration("GREETING")],
            candidates: vec!["WIDTH".to_owned(), "HEIGHT".to_owned()],
            ..python_inputs()
        });
        let field = offered.field(MANAGE_KEY).expect("no manage offer");
        assert_eq!(
            field.label,
            "Detected but not yet managed — tick to manage:"
        );
        let FieldKind::MultiChoice { options } = &field.kind else {
            panic!("the offer needs an open option set");
        };
        assert_eq!(
            options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["WIDTH", "HEIGHT"]
        );

        // The host splits one comma-separated list, so the ticked set travels in that shape and no
        // translation layer sits between the model and the save.
        let mut offered = offered;
        offered.set_value(
            MANAGE_KEY,
            FieldValue::Explicit(TypedValue::Choices(vec![
                "WIDTH".to_owned(),
                "HEIGHT".to_owned(),
            ])),
        );
        assert_eq!(
            offered
                .submitted_values()
                .get(MANAGE_KEY)
                .map(FieldValue::as_text)
                .as_deref(),
            Some("WIDTH, HEIGHT")
        );

        // Nothing is managed yet and the script reads its own arguments: explain, offer nothing.
        let reader_driven = SettingsView::from_inputs(&SettingsInputs {
            reader_fields: 2,
            candidates: vec!["WIDTH".to_owned()],
            ..python_inputs()
        });
        assert!(reader_driven.field(MANAGE_KEY).is_none());
        assert!(
            notes(&reader_driven, SettingsSectionId::Parameters)
                .iter()
                .any(|note| note.starts_with("This script's run form comes from its own")),
            "the trap must be explained"
        );
    }

    /// Managing every `input()` earns the sentence that says the script is now automatable.
    ///
    /// Version 0.4 prints it under the rows (`src/skit/tui_settings.py:632-636`).
    #[test]
    fn a_fully_managed_input_script_is_told_it_can_now_run_unattended() {
        let mut declared = declaration("NAME");
        declared.binding = ParameterBinding::Input;
        let view = SettingsView::from_inputs(&SettingsInputs {
            managed: vec![declared],
            ..python_inputs()
        });
        assert!(notes(&view, SettingsSectionId::Parameters).contains(
            &"Every input() is managed — this script can now run with --no-input.".to_owned()
        ));

        // A constant-bound row is not an `input()`, so the sentence would be false.
        let view = SettingsView::from_inputs(&SettingsInputs {
            managed: vec![declaration("NAME")],
            ..python_inputs()
        });
        assert!(
            !notes(&view, SettingsSectionId::Parameters)
                .iter()
                .any(|note| note.starts_with("Every input()"))
        );
    }

    /// A command entry edits its own command line here, and a prompt edits its insertion switch.
    ///
    /// Version 0.4 puts both inside the declared editor (`src/skit/tui_settings.py:645-666`).
    #[test]
    fn the_template_and_the_insertion_switch_live_with_the_rows_they_govern() {
        let command = SettingsView::from_inputs(&SettingsInputs {
            kind: "command".to_owned(),
            declared_schema: true,
            has_analyzer: false,
            template: "rg {pattern}".to_owned(),
            ..python_inputs()
        });
        assert_eq!(
            command.field(TEMPLATE_KEY).unwrap().value().as_text(),
            "rg {pattern}"
        );
        assert!(command.field(INTERPOLATE_KEY).is_none());
        assert!(
            notes(&command, SettingsSectionId::Parameters)
                .contains(&"Saving re-reads the {placeholders} from the template.".to_owned())
        );

        let prompt = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            declared_schema: true,
            has_analyzer: false,
            interpolate: true,
            ..python_inputs()
        });
        assert_eq!(
            prompt.field(INTERPOLATE_KEY).unwrap().value(),
            &FieldValue::boolean(true)
        );
        assert!(prompt.field(TEMPLATE_KEY).is_none());
        // A prompt names its fields with placeholders, so a flag would mean nothing (`:653-657`).
        let prompt_rows = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            declared_schema: true,
            has_analyzer: false,
            interpolate: true,
            managed: vec![declaration("topic")],
            ..python_inputs()
        });
        assert!(prompt_rows.field("parameter:topic:flag").is_none());
    }

    /// Turning variable insertion off takes the run form away, so its rows go with it.
    ///
    /// Version 0.4 hides the whole declared body while the switch is off and skips collecting it on
    /// save (`src/skit/tui_settings.py:678-681`, `:900-904`, `:953`). A row that described a form
    /// the entry is not building would otherwise be written from a control nobody could see.
    #[test]
    fn a_prompt_with_insertion_off_neither_shows_nor_submits_its_run_form_rows() {
        let mut view = SettingsView::from_inputs(&SettingsInputs {
            kind: "prompt".to_owned(),
            declared_schema: true,
            has_analyzer: false,
            interpolate: true,
            managed: vec![declaration("topic")],
            ..python_inputs()
        });
        assert!(view.focusable_keys().contains(&"parameter:topic:prompt"));
        view.set_value("parameter:topic:prompt", FieldValue::text("Topic"));
        assert_eq!(
            view.submitted_values()
                .get("parameter:topic:prompt")
                .map(FieldValue::as_text)
                .as_deref(),
            Some("Topic")
        );

        view.set_value(INTERPOLATE_KEY, FieldValue::boolean(false));
        assert!(!view.focusable_keys().contains(&"parameter:topic:prompt"));
        assert!(!view.focusable_keys().contains(&ADD_PARAMETER_KEY));
        let values = view.submitted_values();
        assert!(
            !values.contains_key("parameter:topic:prompt"),
            "a hidden row must not write a form the entry is not building: {values:?}"
        );
        assert_eq!(
            values.get(INTERPOLATE_KEY),
            Some(&FieldValue::boolean(false))
        );
        // The work is still unsaved, so leaving still asks before dropping it.
        assert!(view.is_dirty());
    }

    /// No offer on this screen edits anything until a person ticks it.
    ///
    /// `AGENTS.md` calls `--normalize` "the only opt-in semantic edit to a stored script", and
    /// version 0.4 opens both of its tick-to-act lists unticked (`src/skit/tui_settings.py:629`).
    /// A default-on box would make a save rewrite a script the user only came to rename.
    #[test]
    fn every_tick_to_act_offer_opens_empty_and_a_save_that_touches_none_carries_none() {
        let view = SettingsView::from_inputs(&SettingsInputs {
            kind: "shell".to_owned(),
            managed: vec![declaration("NAME")],
            candidates: vec!["WIDTH".to_owned()],
            ..python_inputs()
        });
        for key in [MANAGE_KEY, NORMALIZE_KEY] {
            let field = view.field(key).unwrap_or_else(|| panic!("{key} is absent"));
            assert_eq!(
                field.value(),
                &FieldValue::Explicit(TypedValue::Choices(Vec::new())),
                "{key} opened already acting"
            );
        }
        assert_eq!(
            view.field(RESYNC_KEY).expect("no resync control").value(),
            &FieldValue::boolean(false),
            "the resync opened already requested"
        );
        // Nothing was ticked, so a save of some other axis carries no semantic edit at all.
        let mut view = view;
        view.set_value(NAME_KEY, FieldValue::text("Renamed"));
        let values = view.submitted_values();
        assert_eq!(
            values.len(),
            1,
            "a rename carried more than a rename: {values:?}"
        );
        assert!(!values.contains_key(NORMALIZE_KEY));
        assert!(!values.contains_key(MANAGE_KEY));
        assert!(!values.contains_key(RESYNC_KEY));
    }

    /// The normalize offer belongs to the one kind and the one storage mode it can rewrite.
    ///
    /// `--normalize` is skit's only opt-in semantic edit to a stored script, and version 0.4 tells
    /// the user to run it from the command line (`src/skit/cli.py:4014`, `:4113-4116`).
    #[test]
    fn only_a_stored_shell_copy_is_offered_the_environment_default_rewrite() {
        let shell = SettingsView::from_inputs(&SettingsInputs {
            kind: "shell".to_owned(),
            managed: vec![declaration("NAME")],
            candidates: vec!["WIDTH".to_owned()],
            ..python_inputs()
        });
        let field = shell.field(NORMALIZE_KEY).expect("no normalize offer");
        let FieldKind::MultiChoice { options } = &field.kind else {
            panic!("the offer needs an open option set");
        };
        // Both a managed constant and an unmanaged one can be rewritten.
        assert_eq!(
            options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["NAME", "WIDTH"]
        );

        // One already rewritten is not offered again: the rewrite is what produced it, so the only
        // outcome would be a refusal.
        let mut rewritten = declaration("NAME");
        rewritten.binding = ParameterBinding::EnvDefault;
        let shell = SettingsView::from_inputs(&SettingsInputs {
            kind: "shell".to_owned(),
            managed: vec![rewritten],
            candidates: vec!["WIDTH".to_owned()],
            ..python_inputs()
        });
        let FieldKind::MultiChoice { options } = &shell.field(NORMALIZE_KEY).unwrap().kind else {
            panic!("the offer needs an open option set");
        };
        assert_eq!(
            options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<Vec<_>>(),
            ["WIDTH"]
        );

        // With nothing left to rewrite there is no control at all, not an empty one.
        let mut rewritten = declaration("NAME");
        rewritten.binding = ParameterBinding::EnvDefault;
        assert!(
            SettingsView::from_inputs(&SettingsInputs {
                kind: "shell".to_owned(),
                managed: vec![rewritten],
                ..python_inputs()
            })
            .field(NORMALIZE_KEY)
            .is_none()
        );

        // A python entry has no shell constant to rewrite.
        assert!(
            SettingsView::from_inputs(&SettingsInputs {
                managed: vec![declaration("NAME")],
                ..python_inputs()
            })
            .field(NORMALIZE_KEY)
            .is_none()
        );
        // A linked shell file is one skit never writes.
        assert!(
            SettingsView::from_inputs(&SettingsInputs {
                kind: "shell".to_owned(),
                reference_mode: true,
                managed: vec![declaration("NAME")],
                ..python_inputs()
            })
            .field(NORMALIZE_KEY)
            .is_none()
        );
    }

    /// The resync control exists exactly where a resync does something, and the chord follows it.
    #[test]
    fn the_resync_control_exists_only_where_the_chord_would_do_something() {
        let mut view = SettingsView::from_inputs(&python_inputs());
        assert!(view.resync_available);
        assert!(view.field(RESYNC_KEY).is_some());

        assert_eq!(view.update(SettingsAction::Resync), SettingsEffect::None);
        assert_eq!(
            view.field(RESYNC_KEY).unwrap().value(),
            &FieldValue::boolean(true)
        );
        assert_eq!(view.focused(), RESYNC_KEY);
        assert_eq!(
            view.submitted_values().get(RESYNC_KEY),
            Some(&FieldValue::boolean(true))
        );

        // A hand-declared kind has no script block to read back, so neither the control nor the
        // chord exists (`src/skit/tui_settings.py:408-415`).
        let mut declared = SettingsView::from_inputs(&SettingsInputs {
            kind: "exe".to_owned(),
            declared_schema: true,
            has_analyzer: false,
            ..python_inputs()
        });
        assert!(!declared.resync_available);
        assert!(declared.field(RESYNC_KEY).is_none());
        assert_eq!(
            declared.update(SettingsAction::Resync),
            SettingsEffect::None
        );
        assert!(declared.submitted_values().is_empty());
    }

    fn presets(pairs: &[(&str, &[(&str, &str)])]) -> BTreeMap<String, BTreeMap<String, String>> {
        pairs
            .iter()
            .map(|(name, values)| {
                (
                    (*name).to_owned(),
                    values
                        .iter()
                        .map(|(field, value)| ((*field).to_owned(), (*value).to_owned()))
                        .collect(),
                )
            })
            .collect()
    }

    /// A preset is a toggle carrying its own name and values, and unticking it deletes it.
    ///
    /// Version 0.4 lists them under one sentence and deletes what the user unticked
    /// (`src/skit/tui_settings.py:809-818`, `:1114-1120`).
    #[test]
    fn a_preset_is_a_named_toggle_and_unticking_it_is_the_delete() {
        let mut view = SettingsView::from_inputs(&SettingsInputs {
            presets: presets(&[
                ("nightly", &[("mode", "fast"), ("count", "3")]),
                ("release", &[("mode", "slow")]),
            ]),
            ..python_inputs()
        });
        assert_eq!(
            notes(&view, SettingsSectionId::Presets),
            ["Untick a preset to delete it on save:"]
        );
        let field = view.field("preset:nightly").expect("no preset toggle");
        // The label is the user's own name and their own values, shown exactly as stored.
        assert_eq!(field.label, "nightly  count=3, mode=fast");
        assert!(!field.translate_label);
        assert_eq!(field.value(), &FieldValue::boolean(true));

        // Untick one: only that one travels, and it travels under its own name.
        view.set_value("preset:nightly", FieldValue::boolean(false));
        let values = view.submitted_values();
        assert_eq!(values.len(), 1, "{values:?}");
        assert_eq!(
            values.get("preset:nightly"),
            Some(&FieldValue::boolean(false))
        );
        assert!(
            !values.contains_key("preset:release"),
            "a preset nobody touched must not be deleted"
        );
    }

    /// An entry with no presets is told where presets come from.
    ///
    /// Version 0.4 says it in one sentence rather than drawing an empty box, because creating one
    /// belongs to the run form (`src/skit/tui_settings.py:803-808`).
    #[test]
    fn an_entry_with_no_presets_is_told_where_they_come_from() {
        let view = SettingsView::from_inputs(&python_inputs());
        assert!(view.has_section(SettingsSectionId::Presets));
        assert_eq!(
            notes(&view, SettingsSectionId::Presets),
            ["None yet — press Ctrl+S inside the run form to save one."]
        );
        assert!(
            view.sections
                .iter()
                .find(|section| section.id == SettingsSectionId::Presets)
                .is_some_and(|section| section.fields().next().is_none()),
            "an empty preset list must offer no control"
        );
    }

    /// The Library's `s` key opens this screen on the presets, and the reader takes the eye back.
    ///
    /// Version 0.4 gives `s` to `action_settings(section="presets")` (`src/skit/tui.py:991-992`)
    /// and scrolls the body to that section on mount (`src/skit/tui_settings.py:876-882`).
    #[test]
    fn the_preset_deep_link_lands_on_the_section_and_releases_it_on_the_first_key() {
        let deep_linked = SettingsInputs {
            revealed: Some(SettingsSectionId::Presets),
            presets: presets(&[("nightly", &[("mode", "fast")])]),
            ..python_inputs()
        };
        let mut view = SettingsView::from_inputs(&deep_linked);
        assert_eq!(view.revealed(), Some(SettingsSectionId::Presets));
        // The keyboard lands on the first preset, so Space deletes what the user came for.
        assert_eq!(view.focused(), "preset:nightly");

        // The first keyboard move hands the viewport back to the reader.
        view.move_focus(true);
        assert_eq!(view.revealed(), None);

        // With no presets the section has nothing to focus, so the anchor is what shows it.
        let empty = SettingsView::from_inputs(&SettingsInputs {
            revealed: Some(SettingsSectionId::Presets),
            ..python_inputs()
        });
        assert_eq!(empty.revealed(), Some(SettingsSectionId::Presets));
        assert_eq!(empty.focused(), NAME_KEY);

        // Opening the screen normally anchors nothing.
        assert_eq!(SettingsView::from_inputs(&python_inputs()).revealed(), None);
    }

    /// An entry of a kind this skit does not know gets no policy controls and no editor.
    ///
    /// Version 0.4 returns early rather than guessing what a newer skit's kind allows
    /// (`src/skit/tui_settings.py:444-445`, `:591-596`). A control built on a guess would offer an
    /// edit no launch could use.
    #[test]
    fn an_unknown_kind_is_offered_neither_a_launch_policy_nor_a_parameter_editor() {
        let view = SettingsView::from_inputs(&SettingsInputs {
            kind: String::new(),
            ..python_inputs()
        });
        assert!(!view.has_section(SettingsSectionId::Launch));
        assert!(!view.has_section(SettingsSectionId::Parameters));
        // The axes that do not depend on the kind are still there.
        assert!(view.has_section(SettingsSectionId::Basics));
        assert!(view.has_section(SettingsSectionId::Needs));
    }

    /// A row's edit travels under that row's own name and moves no other row.
    #[test]
    fn only_a_moved_parameter_axis_travels_and_it_carries_its_own_name() {
        let mut view = SettingsView::from_inputs(&SettingsInputs {
            managed: vec![declaration("FIRST"), declaration("SECOND")],
            ..python_inputs()
        });
        assert!(view.submitted_values().is_empty(), "nothing moved");

        view.set_value("parameter:SECOND:prompt", FieldValue::text("Second"));
        view.set_value("parameter:FIRST:keep", FieldValue::boolean(false));
        let values = view.submitted_values();
        assert_eq!(values.len(), 2, "{values:?}");
        assert_eq!(
            values.get("parameter:SECOND:prompt"),
            Some(&FieldValue::text("Second"))
        );
        assert_eq!(
            values.get("parameter:FIRST:keep"),
            Some(&FieldValue::boolean(false))
        );
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
