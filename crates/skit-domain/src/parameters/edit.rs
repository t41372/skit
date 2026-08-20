//! Pure edits to hand-declared parameter rows.

use std::collections::{BTreeMap, BTreeSet};

use super::{ParamDecl, ParameterBinding, ParameterDelivery, ParameterType};

/// One name-keyed edit value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NamedEdit<T> {
    /// Stable parameter name.
    pub name: String,
    /// Replacement value.
    pub value: T,
}

impl<T> NamedEdit<T> {
    /// Build one name-keyed edit.
    pub fn new(name: impl Into<String>, value: impl Into<T>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Kind-specific facts needed to edit declared rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclaredEditContext {
    default_delivery: ParameterDelivery,
    allowed_deliveries: Vec<ParameterDelivery>,
    placeholder_names: BTreeSet<String>,
}

impl DeclaredEditContext {
    /// Build a context with one required default delivery.
    pub fn new(
        default_delivery: ParameterDelivery,
        additional_deliveries: impl IntoIterator<Item = ParameterDelivery>,
        placeholder_names: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut allowed_deliveries = vec![default_delivery];
        for delivery in additional_deliveries {
            if !allowed_deliveries.contains(&delivery) {
                allowed_deliveries.push(delivery);
            }
        }
        Self {
            default_delivery,
            allowed_deliveries,
            placeholder_names: placeholder_names.into_iter().collect(),
        }
    }

    /// Return whether this entry kind supports one delivery.
    pub fn allows(&self, delivery: ParameterDelivery) -> bool {
        self.allowed_deliveries.contains(&delivery)
    }

    fn is_placeholder(&self, name: &str) -> bool {
        self.placeholder_names.contains(name)
    }
}

/// One complete declared-schema edit request.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeclaredEditRequest {
    /// Names to add, in option order.
    pub add: Vec<String>,
    /// Names to remove, in option order.
    pub remove: Vec<String>,
    /// Raw parameter-type edits.
    pub parameter_types: Vec<NamedEdit<String>>,
    /// Raw default-value edits.
    pub defaults: Vec<NamedEdit<String>>,
    /// Choice-list edits.
    pub choices: Vec<NamedEdit<Vec<String>>>,
    /// Raw delivery edits.
    pub deliveries: Vec<NamedEdit<String>>,
    /// Flag spelling edits.
    pub flags: Vec<NamedEdit<String>>,
    /// Names to mark required.
    pub required: Vec<String>,
    /// Names to mark optional.
    pub optional: Vec<String>,
    /// Help-text edits.
    pub help: Vec<NamedEdit<String>>,
    /// Prompt-text edits.
    pub prompts: Vec<NamedEdit<String>>,
    /// Names to mark secret.
    pub secret: Vec<String>,
    /// Names to make public.
    pub no_secret: Vec<String>,
    /// Secret environment-source edits.
    pub env_sources: Vec<NamedEdit<String>>,
    /// Rust extension: source-binding edits that the CLI has already validated.
    pub bindings: Vec<NamedEdit<ParameterBinding>>,
    /// Rust extension: names to make multiple.
    pub multiple: Vec<String>,
    /// Rust extension: names to make scalar.
    pub no_multiple: Vec<String>,
    /// Rust extension: names whose flag repeats.
    pub repeat: Vec<String>,
    /// Rust extension: names whose values follow one flag.
    pub no_repeat: Vec<String>,
    /// Rust extension: environment-target edits.
    pub env_targets: Vec<NamedEdit<String>>,
    /// Rust extension: boolean-action edits.
    pub actions: Vec<NamedEdit<String>>,
}

/// One recoverable declared-edit problem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeclaredEditWarning {
    /// A requested name does not exist.
    NotDeclared { name: String },
    /// An add requested an existing name.
    AlreadyDeclared { name: String },
    /// The entry kind cannot use the requested delivery.
    BadDelivery { name: String },
    /// Placeholder delivery named no real placeholder.
    NotAPlaceholder { name: String },
    /// A type spelling is not in the closed set.
    BadType { name: String },
    /// A default cannot be coerced to the row type.
    BadDefault { name: String },
    /// An environment source named a public row.
    EnvSourceNotSecret { name: String },
    /// A choice row has no choices.
    ChoiceWithoutChoices { name: String },
    /// An on-by-default boolean needs a separate flag that turns it off.
    BoolFlagOnByDefault { name: String },
}

impl DeclaredEditWarning {
    /// Return the stable affected name.
    pub fn name(&self) -> &str {
        match self {
            Self::NotDeclared { name }
            | Self::AlreadyDeclared { name }
            | Self::BadDelivery { name }
            | Self::NotAPlaceholder { name }
            | Self::BadType { name }
            | Self::BadDefault { name }
            | Self::EnvSourceNotSecret { name }
            | Self::ChoiceWithoutChoices { name }
            | Self::BoolFlagOnByDefault { name } => name,
        }
    }

    /// Return the stable warning code used by adapters and test inventories.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotDeclared { .. } => "not-declared",
            Self::AlreadyDeclared { .. } => "already-declared",
            Self::BadDelivery { .. } => "bad-delivery",
            Self::NotAPlaceholder { .. } => "not-a-placeholder",
            Self::BadType { .. } => "bad-type",
            Self::BadDefault { .. } => "bad-default",
            Self::EnvSourceNotSecret { .. } => "env-source-not-secret",
            Self::ChoiceWithoutChoices { .. } => "choice-without-choices",
            Self::BoolFlagOnByDefault { .. } => "bool-flag-on-by-default",
        }
    }
}

/// The complete result of one pure declared edit.
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredEditResult {
    /// Final unique rows in stable order.
    pub declarations: Vec<ParamDecl>,
    /// Recoverable problems in deterministic operation order.
    pub warnings: Vec<DeclaredEditWarning>,
    /// Whether the semantic row list changed.
    pub changed: bool,
}

/// Parse one value from the public closed parameter-type set.
pub fn as_param_type(value: &str) -> Option<ParameterType> {
    match value {
        "str" => Some(ParameterType::Str),
        "int" => Some(ParameterType::Int),
        "float" => Some(ParameterType::Float),
        "bool" => Some(ParameterType::Bool),
        "choice" => Some(ParameterType::Choice),
        "path" => Some(ParameterType::Path),
        _ => None,
    }
}

/// Apply one recoverable, fixed-order edit batch without mutating the input rows.
pub fn edit_declared(
    declarations: &[ParamDecl],
    request: &DeclaredEditRequest,
    context: &DeclaredEditContext,
) -> DeclaredEditResult {
    let baseline = unique_rows(declarations);
    let mut output = baseline.clone();
    let mut warnings = Vec::new();

    for name in &request.remove {
        if let Some(index) = output.iter().position(|row| row.name == *name) {
            output.remove(index);
        } else {
            warnings.push(DeclaredEditWarning::NotDeclared { name: name.clone() });
        }
    }
    for name in &request.add {
        if output.iter().any(|row| row.name == *name) {
            warnings.push(DeclaredEditWarning::AlreadyDeclared { name: name.clone() });
            continue;
        }
        let declaration = if context.is_placeholder(name) {
            ParamDecl {
                delivery: ParameterDelivery::Placeholder,
                required: true,
                ..ParamDecl::new(name)
            }
        } else {
            ParamDecl {
                delivery: context.default_delivery,
                ..ParamDecl::new(name)
            }
        };
        output.push(declaration);
    }

    for name in named_order(&request.parameter_types) {
        let Some(row) = output.iter_mut().find(|row| row.name == name) else {
            warnings.push(DeclaredEditWarning::NotDeclared { name });
            continue;
        };
        let value = last_named(&request.parameter_types, &name).expect("name came from this list");
        if let Some(parameter_type) = as_param_type(value) {
            row.parameter_type = parameter_type;
        } else {
            warnings.push(DeclaredEditWarning::BadType { name });
        }
    }

    let changed = output != baseline;
    DeclaredEditResult {
        declarations: output,
        warnings,
        changed,
    }
}

fn unique_rows(declarations: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut rows = Vec::new();
    let mut indices = BTreeMap::<String, usize>::new();
    for declaration in declarations {
        if let Some(index) = indices.get(&declaration.name).copied() {
            rows[index] = declaration.clone();
        } else {
            indices.insert(declaration.name.clone(), rows.len());
            rows.push(declaration.clone());
        }
    }
    rows
}

fn named_order<T>(edits: &[NamedEdit<T>]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    edits
        .iter()
        .filter(|edit| seen.insert(edit.name.clone()))
        .map(|edit| edit.name.clone())
        .collect()
}

fn last_named<'a, T>(edits: &'a [NamedEdit<T>], name: &str) -> Option<&'a T> {
    edits
        .iter()
        .rev()
        .find(|edit| edit.name == name)
        .map(|edit| &edit.value)
}
