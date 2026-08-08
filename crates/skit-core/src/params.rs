use std::fmt;

/// Where a parameter originates in source code.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Binding {
    Const,
    Input,
    EnvDefault,
    #[default]
    None,
}

impl Binding {
    /// Stable metadata spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Const => "const",
            Self::Input => "input",
            Self::EnvDefault => "envdefault",
            Self::None => "none",
        }
    }

    fn from_value(value: Option<&toml::Value>) -> Self {
        match value.and_then(toml::Value::as_str) {
            Some("const") => Self::Const,
            Some("input") => Self::Input,
            Some("envdefault") => Self::EnvDefault,
            _ => Self::None,
        }
    }
}

/// How a value reaches the launched program.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Delivery {
    Inject,
    Env,
    #[default]
    Flag,
    Placeholder,
}

impl Delivery {
    /// Stable metadata and machine-interface spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inject => "inject",
            Self::Env => "env",
            Self::Flag => "flag",
            Self::Placeholder => "placeholder",
        }
    }

    fn from_value(value: Option<&toml::Value>) -> Self {
        match value.and_then(toml::Value::as_str) {
            Some("inject") => Self::Inject,
            Some("env") => Self::Env,
            Some("placeholder") => Self::Placeholder,
            _ => Self::Flag,
        }
    }
}

/// User-facing parameter type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParamType {
    #[default]
    String,
    Integer,
    Float,
    Boolean,
    Choice,
    Path,
}

impl ParamType {
    /// Stable metadata and machine-interface spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "str",
            Self::Integer => "int",
            Self::Float => "float",
            Self::Boolean => "bool",
            Self::Choice => "choice",
            Self::Path => "path",
        }
    }

    fn from_value(value: Option<&toml::Value>) -> Self {
        match value.and_then(toml::Value::as_str) {
            Some("int") => Self::Integer,
            Some("float") => Self::Float,
            Some("bool") => Self::Boolean,
            Some("choice") => Self::Choice,
            Some("path") => Self::Path,
            _ => Self::String,
        }
    }
}

/// A TOML-safe scalar default.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamDefault {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
}

impl fmt::Display for ParamDefault {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => formatter.write_str(value),
            Self::Integer(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Boolean(value) => write!(formatter, "{value}"),
        }
    }
}

impl ParamDefault {
    fn from_value(value: Option<&toml::Value>) -> Option<Self> {
        match value? {
            toml::Value::String(value) => Some(Self::String(value.clone())),
            toml::Value::Integer(value) => Some(Self::Integer(*value)),
            toml::Value::Float(value) => Some(Self::Float(*value)),
            toml::Value::Boolean(value) => Some(Self::Boolean(*value)),
            _ => None,
        }
    }

    fn to_value(&self) -> toml::Value {
        match self {
            Self::String(value) => toml::Value::String(value.clone()),
            Self::Integer(value) => toml::Value::Integer(*value),
            Self::Float(value) => toml::Value::Float(*value),
            Self::Boolean(value) => toml::Value::Boolean(*value),
        }
    }
}

/// Universal parameter declaration used by every language and frontend.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub binding: Binding,
    pub delivery: Delivery,
    pub param_type: ParamType,
    pub default: Option<ParamDefault>,
    pub required: bool,
    pub multiple: bool,
    pub repeat: bool,
    pub choices: Vec<String>,
    pub prompt: String,
    pub help: String,
    pub secret: bool,
    pub env_source: String,
    pub flag: String,
    pub action: String,
    pub order: i64,
    pub env_target: String,
    pub degraded: bool,
}

impl ParamDecl {
    /// Return the environment variable that receives this parameter.
    #[must_use]
    pub fn env_var(&self) -> &str {
        if self.env_target.is_empty() {
            &self.name
        } else {
            &self.env_target
        }
    }

    /// Total, coercing read of one hand-editable `[[parameters]]` row.
    #[must_use]
    pub fn from_meta_table(row: &toml::Table) -> Self {
        let name = scalar_text(row.get("name")).unwrap_or_default();
        let choices = row
            .get("choices")
            .and_then(toml::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| scalar_text(Some(item)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            name,
            binding: Binding::from_value(row.get("binding")),
            delivery: Delivery::from_value(row.get("delivery")),
            param_type: ParamType::from_value(row.get("type")),
            default: ParamDefault::from_value(row.get("default")),
            required: truthy(row.get("required")),
            multiple: truthy(row.get("multiple")),
            repeat: truthy(row.get("repeat")),
            choices,
            prompt: scalar_text(row.get("prompt")).unwrap_or_default(),
            help: scalar_text(row.get("help")).unwrap_or_default(),
            secret: truthy(row.get("secret")),
            env_source: scalar_text(row.get("env_source")).unwrap_or_default(),
            flag: scalar_text(row.get("flag")).unwrap_or_default(),
            action: scalar_text(row.get("action")).unwrap_or_default(),
            order: order_value(row.get("order")),
            env_target: scalar_text(row.get("env_target")).unwrap_or_default(),
            degraded: truthy(row.get("degraded")),
        }
    }

    /// Serialize one declaration into the additive metadata format.
    #[must_use]
    pub fn to_meta_table(&self) -> toml::Table {
        let mut row = toml::Table::new();
        row.insert("name".to_owned(), toml::Value::String(self.name.clone()));
        row.insert(
            "delivery".to_owned(),
            toml::Value::String(self.delivery.as_str().to_owned()),
        );
        row.insert(
            "type".to_owned(),
            toml::Value::String(self.param_type.as_str().to_owned()),
        );
        if self.binding != Binding::None {
            row.insert(
                "binding".to_owned(),
                toml::Value::String(self.binding.as_str().to_owned()),
            );
        }
        if let Some(default) = &self.default {
            row.insert("default".to_owned(), default.to_value());
        }
        if self.required {
            row.insert("required".to_owned(), toml::Value::Boolean(true));
        }
        if self.multiple {
            row.insert("multiple".to_owned(), toml::Value::Boolean(true));
        }
        if self.repeat {
            row.insert("repeat".to_owned(), toml::Value::Boolean(true));
        }
        if !self.choices.is_empty() {
            row.insert(
                "choices".to_owned(),
                toml::Value::Array(
                    self.choices
                        .iter()
                        .map(|value| toml::Value::String(value.clone()))
                        .collect(),
                ),
            );
        }
        insert_nonempty(&mut row, "prompt", &self.prompt);
        insert_nonempty(&mut row, "help", &self.help);
        if self.secret {
            row.insert("secret".to_owned(), toml::Value::Boolean(true));
        }
        insert_nonempty(&mut row, "env_source", &self.env_source);
        insert_nonempty(&mut row, "flag", &self.flag);
        insert_nonempty(&mut row, "action", &self.action);
        if self.order != -1 {
            row.insert("order".to_owned(), toml::Value::Integer(self.order));
        }
        insert_nonempty(&mut row, "env_target", &self.env_target);
        if self.degraded {
            row.insert("degraded".to_owned(), toml::Value::Boolean(true));
        }
        row
    }
}

/// Read valid metadata rows while dropping nameless rows that cannot key a field.
#[must_use]
pub fn declared_from_meta(rows: &[toml::Table]) -> Vec<ParamDecl> {
    rows.iter()
        .map(ParamDecl::from_meta_table)
        .filter(|decl| !decl.name.is_empty())
        .collect()
}

/// Create the declaration for a template placeholder that has no explicit schema row.
#[must_use]
pub fn synthesized_placeholder(name: &str) -> ParamDecl {
    ParamDecl {
        name: name.to_owned(),
        delivery: Delivery::Placeholder,
        required: true,
        secret: is_secret_name(name),
        ..ParamDecl::default()
    }
}

/// Conservative name-only secret heuristic used by existing skit metadata.
#[must_use]
pub fn is_secret_name(name: &str) -> bool {
    const HINTS: &[&str] = &["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD"];
    let upper = name.to_ascii_uppercase();
    HINTS.iter().any(|hint| upper.contains(hint))
}

fn insert_nonempty(row: &mut toml::Table, key: &str, value: &str) {
    if !value.is_empty() {
        row.insert(key.to_owned(), toml::Value::String(value.to_owned()));
    }
}

fn scalar_text(value: Option<&toml::Value>) -> Option<String> {
    match value? {
        toml::Value::String(value) => Some(value.clone()),
        toml::Value::Integer(value) => Some(value.to_string()),
        toml::Value::Float(value) => Some(value.to_string()),
        toml::Value::Boolean(value) => Some(value.to_string()),
        _ => None,
    }
}

fn truthy(value: Option<&toml::Value>) -> bool {
    match value {
        None => false,
        Some(toml::Value::Boolean(value)) => *value,
        Some(toml::Value::Integer(value)) => *value != 0,
        Some(toml::Value::Float(value)) => *value != 0.0,
        Some(toml::Value::String(value)) => !value.is_empty(),
        Some(toml::Value::Array(value)) => !value.is_empty(),
        Some(toml::Value::Table(value)) => !value.is_empty(),
        Some(toml::Value::Datetime(_)) => true,
    }
}

fn order_value(value: Option<&toml::Value>) -> i64 {
    match value {
        None => -1,
        Some(toml::Value::Integer(value)) => *value,
        Some(toml::Value::Boolean(value)) => i64::from(*value),
        Some(toml::Value::Float(value)) => *value as i64,
        Some(toml::Value::String(value)) => value.parse().unwrap_or(-1),
        _ => -1,
    }
}
