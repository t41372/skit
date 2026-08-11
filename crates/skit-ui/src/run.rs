//! Typed launch-form state and product semantics.

use std::collections::BTreeMap;

use nucleo_matcher::{
    Config as MatcherConfig, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};
use serde::{Deserialize, Serialize};
use skit_application::{
    form_feedback::{GlobCountRequest, glob_count_request},
    path_insertion::{PathInsertionError, RunPathInsertMode, insert_picked_path},
    tokens::{TokenContext, TokenError, has_tokens, preview_typed},
    value_preparation::{ValuePreparationError, validate_form_value},
};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_form::field::{FieldValue, TypedValue};

use crate::FormPurpose;

/// The editing grammar for one text control.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormInputKind {
    /// Unrestricted one-line text.
    #[default]
    Text,
    /// A signed whole number.
    Integer,
    /// A finite number.
    Float,
    /// A path value with completion and a file-picker door.
    Path,
    /// A command-line argument tail.
    Arguments,
}

/// How a closed set of values is presented.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoicePresentation {
    /// Keep all values visible as a radio group.
    #[default]
    Radio,
    /// Use a compact picker that expands on demand.
    Picker,
}

/// State for a mature text-editing widget.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextControl {
    /// Current value.
    pub value: String,
    /// Input grammar and completion policy.
    pub kind: FormInputKind,
    /// Hide cells that show the current value.
    pub secret: bool,
    /// Accept embedded line breaks.
    pub multiline: bool,
}

/// State for a closed-set widget.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChoiceControl {
    /// Stable values in presentation order.
    pub options: Vec<String>,
    /// Current value. An empty value can represent the latest run values.
    pub selected: String,
    /// Use a radio group or a compact picker.
    pub presentation: ChoicePresentation,
}

/// A typed form control. Frontends do not infer controls from labels or field keys.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormControl {
    /// Text edited by a cursor-aware text widget.
    Text(TextControl),
    /// One boolean value.
    Checkbox {
        /// Current state.
        checked: bool,
    },
    /// One value from a closed set.
    Choice(ChoiceControl),
}

impl FormControl {
    /// Return the stable value sent to the host adapter.
    #[must_use]
    pub fn value(&self) -> String {
        match self {
            Self::Text(control) => control.value.clone(),
            Self::Checkbox { checked } => checked.to_string(),
            Self::Choice(control) => control.selected.clone(),
        }
    }

    /// Return the value with the type its control produced.
    ///
    /// A toggle is a Boolean and a picker is a choice. Rendering them as text here would ask every
    /// consumer to parse `"true"` back, and a consumer that parsed it differently would deliver a
    /// different launch from the same screen.
    #[must_use]
    pub fn typed_value(&self) -> FieldValue {
        match self {
            Self::Text(control) => FieldValue::text(&control.value),
            Self::Checkbox { checked } => FieldValue::boolean(*checked),
            Self::Choice(control) => {
                FieldValue::Explicit(TypedValue::Choice(control.selected.clone()))
            }
        }
    }

    pub(crate) fn append(&mut self, value: &str) {
        if let Self::Text(control) = self {
            control.value.push_str(value);
        }
    }

    pub(crate) fn backspace(&mut self) {
        if let Self::Text(control) = self {
            control.value.pop();
        }
    }

    pub(crate) fn set_text(&mut self, value: String) {
        if let Self::Text(control) = self {
            control.value = value;
        }
    }

    fn set_value(&mut self, value: &str) {
        match self {
            Self::Text(control) => control.value = value.to_owned(),
            Self::Checkbox { checked } => *checked = truthy(value),
            Self::Choice(control) if control.options.iter().any(|option| option == value) => {
                control.selected = value.to_owned();
            }
            Self::Choice(_) => {}
        }
    }

    pub(crate) fn toggle(&mut self) {
        if let Self::Checkbox { checked } = self {
            *checked = !*checked;
        }
    }

    fn select(&mut self, value: String) {
        if let Self::Choice(control) = self
            && control.options.contains(&value)
        {
            control.selected = value;
        }
    }
}

/// Invocation-only launch settings that do not become generic visible fields.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunFormOptions {
    /// Preset selected before the form opens.
    pub selected_preset: String,
    /// Preset name requested by the inline CLI invocation.
    pub save_preset: String,
    /// Preserve an inline `--dry-run` request.
    pub dry_run: bool,
    /// Show the editable argument tail. The full workbench always does.
    pub include_extra: bool,
    /// Values fixed by explicit CLI `--set` options and omitted from the inline form.
    pub fixed_values: BTreeMap<String, String>,
}

impl Default for RunFormOptions {
    fn default() -> Self {
        Self {
            selected_preset: String::new(),
            save_preset: String::new(),
            dry_run: false,
            include_extra: true,
            fixed_values: BTreeMap::new(),
        }
    }
}

/// The semantic role of one control on the launch screen.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunFieldRole {
    /// A value declared by the entry.
    Parameter {
        /// Stable parameter name without frontend prefixes.
        name: String,
    },
    /// Prompt runner picker.
    Runner,
    /// Saved-value picker.
    Preset,
    /// Passthrough argument tail.
    ExtraArguments,
}

/// A launch value that the frontend can correct before it asks the host to run.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunValidationError {
    /// A required value is empty.
    Required,
    /// A value does not match its declared scalar type.
    InvalidType,
    /// A value is not one of the declared choices.
    InvalidChoice,
}

/// A whole-form analysis limit with stable presentation semantics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunDegradationNotice {
    /// The source declares subcommands that the launch form cannot model.
    Subcommands,
    /// The analyzer could not recover the source's argument declarations.
    DynamicArguments,
}

/// Completion roots captured when a launch form opens.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunPathContext {
    /// Directory in which the child resolves a bare relative path.
    pub workdir: String,
    /// Directory in which run-time tokens resolve.
    pub invoke_cwd: String,
}

/// Ambient values and entry facts used by live launch-form affordances.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunFormContext {
    /// Open-ended stored entry kind.
    pub entry_kind: String,
    /// Path completion roots. An inline adapter can omit them.
    pub path: Option<RunPathContext>,
    /// Deterministic token-expansion inputs captured for this form.
    pub tokens: TokenContext,
}

/// A token preview failure that stays serializable across frontend boundaries.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTokenError {
    /// A referenced environment variable is absent.
    MissingEnvironment {
        /// Bare environment-variable name.
        name: String,
        /// Full token spelling.
        token: String,
    },
}

/// One discoverable value in the run-time insertion menu.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTokenOption {
    /// Browse with the frontend's mature filesystem picker.
    FileOrFolder,
    /// Insert the directory from which this launch starts.
    RuntimeDirectory,
    /// Insert the directory captured when the form opened as literal text.
    FixedDirectory {
        /// Native path spelling shown and inserted by the frontend.
        path: String,
    },
    /// Insert the local-date token.
    Today,
    /// Insert the local-time token.
    Now,
    /// Insert the current-user home token.
    Home,
    /// Choose or type an environment-variable name.
    Environment,
}

impl RunTokenOption {
    /// Return the direct insertion text, or `None` for chained pickers.
    #[must_use]
    pub fn insertion(&self) -> Option<&str> {
        match self {
            Self::FileOrFolder | Self::Environment => None,
            Self::RuntimeDirectory => Some("{cwd}"),
            Self::FixedDirectory { path } => Some(path),
            Self::Today => Some("{today}"),
            Self::Now => Some("{now}"),
            Self::Home => Some("~"),
        }
    }
}

pub(crate) fn filter_environment_names(names: &[String], query: &str) -> Vec<String> {
    if query.is_empty() {
        return names.to_vec();
    }
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let query_lower = query.to_lowercase();
    let mut matcher = Matcher::new(MatcherConfig::DEFAULT);
    let mut utf32 = Vec::new();
    let mut ranked = names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let score = pattern.score(Utf32Str::new(name, &mut utf32), &mut matcher)?;
            Some((index, name.to_lowercase().starts_with(&query_lower), score))
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.cmp(&left.2))
            .then_with(|| left.0.cmp(&right.0))
    });
    ranked
        .into_iter()
        .map(|(index, _, _)| names[index].clone())
        .collect()
}

/// Live feedback for one visible launch value.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunFieldFeedback {
    /// Expanded token preview when it differs from the raw value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded: Option<String>,
    /// Typed token failure for localized presentation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_error: Option<RunTokenError>,
    /// Match count returned by the host glob port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glob_count: Option<usize>,
}

/// One typed control on the launch screen.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunField {
    /// Stable key used by the existing host submission contract.
    pub key: String,
    /// User-visible label or parameter prompt.
    pub label: String,
    /// Text below the control.
    pub help: String,
    /// Control purpose.
    pub role: RunFieldRole,
    /// Declared scalar type used for labels, validation, and picker policy.
    pub parameter_type: ParameterType,
    /// Split the value into more than one delivery item.
    pub multiple: bool,
    /// Source binding controls token brace semantics.
    pub binding: ParameterBinding,
    /// Runtime delivery controls placeholder brace semantics.
    pub delivery: ParameterDelivery,
    /// Widget state and input grammar.
    pub control: FormControl,
    /// An empty value is invalid.
    pub required: bool,
    /// Definition default that a reset action restores.
    pub default: Option<String>,
    /// Static analysis could not fully model this field.
    pub degraded: bool,
    /// An empty value lets the source ask in the terminal.
    pub input_binding: bool,
    /// Environment fallback for a secret value.
    pub env_source: String,
    /// Current correction message, when submission validation failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<RunValidationError>,
    /// Current token and glob preview.
    #[serde(default)]
    pub feedback: RunFieldFeedback,
}

impl RunField {
    /// Return the secret environment fallback when one exists.
    #[must_use]
    pub fn environment_source(&self) -> Option<&str> {
        (!self.env_source.is_empty()).then_some(self.env_source.as_str())
    }

    /// Report whether the default chip can restore a visible value.
    #[must_use]
    pub fn resettable(&self) -> bool {
        if self.default.is_none() || self.secret() {
            return false;
        }
        match &self.control {
            FormControl::Choice(choice) => self
                .default
                .as_ref()
                .is_some_and(|value| choice.options.contains(value)),
            FormControl::Text(_) | FormControl::Checkbox { .. } => true,
        }
    }

    /// Report whether the value-token menu applies to this field.
    #[must_use]
    pub fn insertable(&self) -> bool {
        matches!(self.control, FormControl::Text(ref control) if !control.secret)
    }

    /// Report whether a file-picker result can be meaningful for this field.
    #[must_use]
    pub fn browsable(&self) -> bool {
        self.insertable()
            && !matches!(
                self.parameter_type,
                ParameterType::Int | ParameterType::Float
            )
    }

    /// Report whether this field masks its current value.
    #[must_use]
    pub fn secret(&self) -> bool {
        matches!(self.control, FormControl::Text(ref control) if control.secret)
    }
}

/// A launch form whose controls retain parameter semantics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunFormView {
    pub(crate) selector: String,
    name: String,
    pub(crate) fields: Vec<RunField>,
    pub(crate) focused: usize,
    #[serde(default)]
    presets: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    initial_values: BTreeMap<String, String>,
    #[serde(default)]
    hidden_values: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<RunFormContext>,
    /// Source drift notices in display order.
    pub drift_lines: Vec<String>,
    /// Whole-form static-analysis limitation, when present.
    pub degraded_reason: Option<String>,
}

impl RunFormView {
    /// Build the complete launch surface from domain declarations.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_declarations(
        selector: impl Into<String>,
        name: impl Into<String>,
        declarations: &[ParamDecl],
        saved: &BTreeMap<String, String>,
        runners: &[String],
        runner_default: &str,
        presets: &BTreeMap<String, BTreeMap<String, String>>,
        extra_arguments: &str,
    ) -> Self {
        let mut fields = Vec::new();
        if !runners.is_empty() {
            let selected = if runners.iter().any(|name| name == runner_default) {
                runner_default.to_owned()
            } else {
                runners[0].clone()
            };
            fields.push(RunField {
                key: "_skit_runner".to_owned(),
                label: "Runner".to_owned(),
                help: String::new(),
                role: RunFieldRole::Runner,
                parameter_type: ParameterType::Choice,
                multiple: false,
                binding: ParameterBinding::None,
                delivery: ParameterDelivery::Flag,
                control: FormControl::Choice(ChoiceControl {
                    options: runners.to_vec(),
                    selected,
                    presentation: ChoicePresentation::Picker,
                }),
                required: true,
                default: None,
                degraded: false,
                input_binding: false,
                env_source: String::new(),
                validation_error: None,
                feedback: RunFieldFeedback::default(),
            });
        }
        if !declarations.is_empty() && !presets.is_empty() {
            fields.push(preset_field(presets.keys().cloned(), String::new()));
        }
        fields.extend(
            declarations
                .iter()
                .map(|declaration| run_parameter_field(declaration, saved)),
        );
        let initial_values = fields
            .iter()
            .filter_map(|field| match &field.role {
                RunFieldRole::Parameter { name } => Some((name.clone(), field.control.value())),
                RunFieldRole::Runner | RunFieldRole::Preset | RunFieldRole::ExtraArguments => None,
            })
            .collect();
        fields.push(RunField {
            key: "_skit_args".to_owned(),
            label: "Extra arguments".to_owned(),
            help: String::new(),
            role: RunFieldRole::ExtraArguments,
            parameter_type: ParameterType::Str,
            multiple: true,
            binding: ParameterBinding::None,
            delivery: ParameterDelivery::Flag,
            control: FormControl::Text(TextControl {
                value: extra_arguments.to_owned(),
                kind: FormInputKind::Arguments,
                secret: false,
                multiline: false,
            }),
            required: false,
            default: None,
            degraded: false,
            input_binding: false,
            env_source: String::new(),
            validation_error: None,
            feedback: RunFieldFeedback::default(),
        });
        let mut view = Self {
            selector: selector.into(),
            name: name.into(),
            fields,
            focused: 0,
            presets: presets.clone(),
            initial_values,
            hidden_values: BTreeMap::from([
                ("_skit_save_preset".to_owned(), String::new()),
                ("_skit_dry_run".to_owned(), "false".to_owned()),
            ]),
            context: None,
            drift_lines: Vec::new(),
            degraded_reason: None,
        };
        view.focus_first_typeable();
        view
    }

    /// Put the boot focus on the first control the user can type or toggle.
    ///
    /// Version 0.4 auto-focuses `"Input, Checkbox, RadioSet"` (`src/skit/tui_form.py:566`), which
    /// the runner and preset dropdowns do not match: the form must be typeable the moment it
    /// opens, whatever optional rows sit above the first parameter.
    fn focus_first_typeable(&mut self) {
        self.focused = self
            .fields
            .iter()
            .position(|field| {
                matches!(
                    field.role,
                    RunFieldRole::Parameter { .. } | RunFieldRole::ExtraArguments
                )
            })
            .unwrap_or(0)
            .min(self.fields.len().saturating_sub(1));
    }

    /// Apply invocation-only state without presenting fake text controls.
    #[must_use]
    pub fn with_options(mut self, options: RunFormOptions) -> Self {
        self.hidden_values
            .insert("_skit_save_preset".to_owned(), options.save_preset);
        self.hidden_values
            .insert("_skit_dry_run".to_owned(), options.dry_run.to_string());
        if let Some(index) = self
            .fields
            .iter()
            .position(|field| matches!(field.role, RunFieldRole::Preset))
        {
            self.select_option(index, &options.selected_preset);
        }
        for (name, value) in options.fixed_values {
            let key = format!("value:{name}");
            if let Some(index) = self.fields.iter().position(|field| field.key == key) {
                self.fields.remove(index);
            }
            self.hidden_values.insert(key, value);
        }
        if !options.include_extra
            && let Some(index) = self
                .fields
                .iter()
                .position(|field| matches!(field.role, RunFieldRole::ExtraArguments))
        {
            let field = self.fields.remove(index);
            self.hidden_values.insert(field.key, field.control.value());
        }
        self.focus_first_typeable();
        self
    }

    /// Attach the deterministic ambient context used by picker and preview actions.
    #[must_use]
    pub fn with_context(mut self, context: RunFormContext) -> Self {
        let extra_label = match context.entry_kind.as_str() {
            "prompt" => "Extra agent arguments",
            "command" => "Extra command arguments",
            _ => "Extra arguments (passed to the script as-is)",
        };
        if let Some(field) = self
            .fields
            .iter_mut()
            .find(|field| matches!(field.role, RunFieldRole::ExtraArguments))
        {
            field.label = extra_label.to_owned();
        }
        self.context = Some(context);
        for index in 0..self.fields.len() {
            self.refresh_feedback(index);
        }
        self
    }

    /// Return the stable operation identity.
    #[must_use]
    pub const fn purpose(&self) -> FormPurpose {
        FormPurpose::Run
    }

    /// Return the entry selector.
    #[must_use]
    pub fn selector(&self) -> &str {
        &self.selector
    }

    /// Return the entry display name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Return controls in focus order.
    #[must_use]
    pub fn fields(&self) -> &[RunField] {
        &self.fields
    }

    /// Report whether the entry declares at least one parameter field.
    #[must_use]
    pub fn has_parameters(&self) -> bool {
        self.fields
            .iter()
            .any(|field| matches!(field.role, RunFieldRole::Parameter { .. }))
    }

    /// Report whether at least one field has a truthful default-restoration action.
    #[must_use]
    pub fn has_resettable_fields(&self) -> bool {
        self.fields.iter().any(RunField::resettable)
    }

    /// Report whether this screen owns a runner picker.
    #[must_use]
    pub fn has_runner_picker(&self) -> bool {
        self.fields
            .iter()
            .any(|field| matches!(field.role, RunFieldRole::Runner))
    }

    /// Add one runner returned by the shared editor and select it immediately.
    ///
    /// The selector prevents an asynchronous host response from attaching a runner to a later
    /// launch form for another entry.
    pub(crate) fn add_and_select_runner(&mut self, selector: &str, runner: String) {
        if self.selector != selector {
            return;
        }
        let Some(field) = self
            .fields
            .iter_mut()
            .find(|field| matches!(field.role, RunFieldRole::Runner))
        else {
            return;
        };
        let FormControl::Choice(choice) = &mut field.control else {
            return;
        };
        if !choice.options.contains(&runner) {
            choice.options.push(runner.clone());
        }
        choice.selected = runner;
        field.validation_error = None;
    }

    /// Return the insertion menu for the focused field when that field accepts tokens.
    #[must_use]
    pub fn focused_token_options(&self) -> Option<Vec<RunTokenOption>> {
        self.token_options(self.focused)
    }

    pub(crate) fn token_options(&self, index: usize) -> Option<Vec<RunTokenOption>> {
        let field = self.fields.get(index)?;
        if !field.insertable() {
            return None;
        }
        let context = self.context.as_ref()?;
        let mut options = vec![
            RunTokenOption::RuntimeDirectory,
            RunTokenOption::FixedDirectory {
                path: context.tokens.cwd.clone(),
            },
            RunTokenOption::Today,
            RunTokenOption::Now,
            RunTokenOption::Home,
            RunTokenOption::Environment,
        ];
        if context.path.is_some() {
            if field.parameter_type == ParameterType::Path {
                options.insert(0, RunTokenOption::FileOrFolder);
            } else {
                options.push(RunTokenOption::FileOrFolder);
            }
        }
        Some(options)
    }

    pub(crate) fn path_picker_contract(
        &self,
        index: usize,
    ) -> Option<(RunPathContext, RunPathInsertMode)> {
        let field = self.fields.get(index)?;
        if !field.browsable() {
            return None;
        }
        let context = self.context.as_ref()?.path.clone()?;
        let mode = match field.role {
            RunFieldRole::ExtraArguments => RunPathInsertMode::Arguments,
            _ if field.multiple => RunPathInsertMode::Shlex,
            RunFieldRole::Parameter { .. } | RunFieldRole::Runner | RunFieldRole::Preset => {
                RunPathInsertMode::Replace
            }
        };
        Some((context, mode))
    }

    /// Return saved preset names in stable order.
    pub fn preset_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.presets.keys().map(String::as_str)
    }

    /// Return the ambient context supplied by the host adapter.
    #[must_use]
    pub const fn context(&self) -> Option<&RunFormContext> {
        self.context.as_ref()
    }

    /// Return a stable whole-form limitation for localized presentation.
    #[must_use]
    pub fn degradation_notice(&self) -> Option<RunDegradationNotice> {
        self.degraded_reason.as_deref().map(|reason| {
            if matches!(reason, "subparsers" | "subcommands") {
                RunDegradationNotice::Subcommands
            } else {
                RunDegradationNotice::DynamicArguments
            }
        })
    }

    /// Report whether one field has a direct filesystem-picker door.
    #[must_use]
    pub fn can_browse_field(&self, index: usize) -> bool {
        self.path_picker_contract(index).is_some()
    }

    /// Report whether one field has a run-time value insertion menu.
    #[must_use]
    pub fn can_insert_field(&self, index: usize) -> bool {
        self.token_options(index).is_some()
    }

    /// Return parameter names that must not be stored in a preset.
    pub fn secret_names(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().filter_map(|field| {
            if !field.secret() {
                return None;
            }
            match &field.role {
                RunFieldRole::Parameter { name } => Some(name.as_str()),
                RunFieldRole::Runner | RunFieldRole::Preset | RunFieldRole::ExtraArguments => None,
            }
        })
    }

    /// Return the active control index.
    #[must_use]
    pub const fn focused(&self) -> usize {
        self.focused
    }

    pub(crate) fn select_option(&mut self, index: usize, value: &str) {
        let is_preset = self
            .fields
            .get(index)
            .is_some_and(|field| matches!(field.role, RunFieldRole::Preset));
        if let Some(field) = self.fields.get_mut(index) {
            field.control.select(value.to_owned());
            field.validation_error = None;
        }
        if is_preset {
            self.apply_preset(value);
        }
    }

    fn apply_preset(&mut self, name: &str) {
        let mut values = self.initial_values.clone();
        if let Some(preset) = self.presets.get(name) {
            values.extend(preset.clone());
        }
        for field in &mut self.fields {
            if let RunFieldRole::Parameter { name } = &field.role
                && let Some(value) = values.get(name)
            {
                field.control.set_value(value);
            }
        }
    }

    /// Return every value the launch delivers, typed.
    ///
    /// A launch delivers all of them, so nothing is omitted for being unchanged: this is not a diff
    /// against a stored record, it is the argument set one run receives.
    pub(crate) fn values(&self) -> crate::SubmittedValues {
        let mut values = self
            .hidden_values
            .iter()
            .map(|(key, value)| (key.clone(), FieldValue::text(value)))
            .collect::<crate::SubmittedValues>();
        values.extend(
            self.fields
                .iter()
                .map(|field| (field.key.clone(), field.control.typed_value())),
        );
        values
    }

    pub(crate) fn reset_field(&mut self, index: usize) {
        let Some(field) = self.fields.get_mut(index) else {
            return;
        };
        if !field.resettable() {
            return;
        }
        let Some(value) = field.default.clone() else {
            return;
        };
        field.control.set_value(&value);
        field.validation_error = None;
    }

    pub(crate) fn validate(&mut self) -> bool {
        let mut valid = true;
        for field in &mut self.fields {
            field.validation_error = validation_error(field);
            valid &= field.validation_error.is_none();
        }
        valid
    }

    pub(crate) fn set_field_value(
        &mut self,
        index: usize,
        value: String,
    ) -> Option<GlobCountRequest> {
        let field = self.fields.get_mut(index)?;
        field.control.set_text(value);
        field.validation_error = None;
        self.refresh_feedback(index)
    }

    pub(crate) fn insert_picked_path(
        &mut self,
        index: usize,
        picked: &str,
        mode: RunPathInsertMode,
    ) -> Result<Option<GlobCountRequest>, PathInsertionError> {
        let Some(field) = self.fields.get(index) else {
            return Ok(None);
        };
        let value = insert_picked_path(&field.control.value(), picked, mode)?;
        Ok(self.set_field_value(index, value))
    }

    pub(crate) fn set_glob_count(&mut self, index: usize, value: &str, count: usize) {
        let Some(field) = self.fields.get_mut(index) else {
            return;
        };
        if field.control.value() == value {
            field.feedback.glob_count = Some(count);
        }
    }

    pub(crate) fn preset_snapshot(&self) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        for field in &self.fields {
            if let RunFieldRole::Parameter { name } = &field.role
                && !field.secret()
            {
                values.insert(name.clone(), field.control.value());
            }
        }
        for (key, value) in &self.hidden_values {
            if let Some(name) = key.strip_prefix("value:")
                && !value.is_empty()
            {
                values
                    .entry(name.to_owned())
                    .or_insert_with(|| value.clone());
            }
        }
        values
    }

    pub(crate) fn refresh_presets(
        &mut self,
        selected: String,
        presets: BTreeMap<String, BTreeMap<String, String>>,
    ) {
        let focused_key = self.fields.get(self.focused).map(|field| field.key.clone());
        self.presets = presets;
        let options = self.presets.keys().cloned().collect::<Vec<_>>();
        if let Some(field) = self
            .fields
            .iter_mut()
            .find(|field| matches!(field.role, RunFieldRole::Preset))
        {
            *field = preset_field(options, selected);
        } else if self.has_parameters() {
            let index = usize::from(
                self.fields
                    .first()
                    .is_some_and(|field| matches!(field.role, RunFieldRole::Runner)),
            );
            self.fields.insert(index, preset_field(options, selected));
        }
        if let Some(key) = focused_key
            && let Some(index) = self.fields.iter().position(|field| field.key == key)
        {
            self.focused = index;
        }
    }

    fn refresh_feedback(&mut self, index: usize) -> Option<GlobCountRequest> {
        let context = self.context.as_ref()?;
        let field = self.fields.get_mut(index)?;
        field.feedback = RunFieldFeedback::default();
        let FormControl::Text(control) = &field.control else {
            return None;
        };
        if control.secret || control.value.is_empty() {
            return None;
        }
        if has_tokens(&control.value) {
            let (expanded, error) = preview_typed(
                &control.value,
                &context.tokens,
                field.delivery != ParameterDelivery::Placeholder,
            );
            if expanded != control.value {
                field.feedback.expanded = Some(expanded);
            }
            field.feedback.token_error = error.map(token_error);
        }
        let cwd = context
            .path
            .as_ref()
            .map_or(context.tokens.cwd.as_str(), |path| path.invoke_cwd.as_str());
        glob_count_request(&control.value, cwd)
    }
}

fn token_error(error: TokenError) -> RunTokenError {
    match error {
        TokenError::MissingEnvironment { name, token } => {
            RunTokenError::MissingEnvironment { name, token }
        }
    }
}

fn preset_field(names: impl IntoIterator<Item = String>, selected: String) -> RunField {
    let mut options = vec![String::new()];
    options.extend(names);
    RunField {
        key: "_skit_preset".to_owned(),
        label: "Preset".to_owned(),
        help: String::new(),
        role: RunFieldRole::Preset,
        parameter_type: ParameterType::Choice,
        multiple: false,
        binding: ParameterBinding::None,
        delivery: ParameterDelivery::Flag,
        control: FormControl::Choice(ChoiceControl {
            options,
            selected,
            presentation: ChoicePresentation::Picker,
        }),
        required: false,
        default: None,
        degraded: false,
        input_binding: false,
        env_source: String::new(),
        validation_error: None,
        feedback: RunFieldFeedback::default(),
    }
}

fn validation_error(field: &RunField) -> Option<RunValidationError> {
    let RunFieldRole::Parameter { name } = &field.role else {
        return None;
    };
    let mut declaration = ParamDecl::new(name);
    declaration.parameter_type = field.parameter_type;
    declaration.multiple = field.multiple;
    declaration.required = field.required;
    declaration.degraded = field.degraded;
    declaration.choices = match &field.control {
        FormControl::Choice(choice) => choice.options.clone(),
        FormControl::Text(_) | FormControl::Checkbox { .. } => Vec::new(),
    };
    declaration.prompt = field.label.clone();
    validate_form_value(&declaration, &field.control.value())
        .err()
        .map(|error| match error {
            ValuePreparationError::Required { .. } => RunValidationError::Required,
            ValuePreparationError::InvalidType { .. } => RunValidationError::InvalidType,
            ValuePreparationError::InvalidChoice { .. } => RunValidationError::InvalidChoice,
        })
}

fn run_parameter_field(declaration: &ParamDecl, saved: &BTreeMap<String, String>) -> RunField {
    let default = declaration.default.as_ref().map(parameter_value_text);
    let value = if declaration.secret {
        String::new()
    } else {
        saved
            .get(&declaration.name)
            .cloned()
            .or_else(|| default.clone())
            .unwrap_or_default()
    };
    let control = match declaration.parameter_type {
        ParameterType::Bool => FormControl::Checkbox {
            checked: truthy(&value),
        },
        ParameterType::Choice if !declaration.choices.is_empty() => {
            let selected = if declaration.choices.contains(&value) {
                value
            } else {
                String::new()
            };
            FormControl::Choice(ChoiceControl {
                options: declaration.choices.clone(),
                selected,
                presentation: ChoicePresentation::Radio,
            })
        }
        parameter_type => FormControl::Text(TextControl {
            value,
            kind: match parameter_type {
                ParameterType::Int => FormInputKind::Integer,
                ParameterType::Float => FormInputKind::Float,
                ParameterType::Path => FormInputKind::Path,
                ParameterType::Str | ParameterType::Choice | ParameterType::Bool => {
                    FormInputKind::Text
                }
            },
            secret: declaration.secret,
            multiline: false,
        }),
    };
    RunField {
        key: format!("value:{}", declaration.name),
        label: if declaration.prompt.is_empty() {
            declaration.name.clone()
        } else {
            declaration.prompt.clone()
        },
        help: declaration.help.clone(),
        role: RunFieldRole::Parameter {
            name: declaration.name.clone(),
        },
        parameter_type: declaration.parameter_type,
        multiple: declaration.multiple,
        binding: declaration.binding,
        delivery: declaration.delivery,
        control,
        required: declaration.required,
        default,
        degraded: declaration.degraded,
        input_binding: declaration.binding == ParameterBinding::Input,
        env_source: declaration.env_source.clone(),
        validation_error: None,
        feedback: RunFieldFeedback::default(),
    }
}

fn parameter_value_text(value: &ParameterValue) -> String {
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => value.to_string(),
        ParameterValue::Bool(value) => value.to_string(),
    }
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "y" | "on"
    )
}
