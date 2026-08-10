//! Frontend-neutral parameter declarations and their serialization contracts.
//!
//! A parameter has two orthogonal axes: its source binding and its runtime delivery. The
//! source-anchored bindings imply one delivery, while hand-declared parameters keep the delivery
//! axis free. This module intentionally deals in generic JSON-shaped maps rather than TOML so the
//! domain crate remains independent of storage adapters.

mod declarations;
mod secrets;

pub use declarations::{declared_for_template, declared_from_meta};
pub use secrets::{is_secret_name, synthesized_placeholder};

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use skit_i18n::{Localize, Message};
use thiserror::Error;

/// How a parameter is anchored in user-authored source.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterBinding {
    /// A named literal assignment.
    Const,
    /// An interactive prompt call site.
    Input,
    /// An environment-default expansion such as `${NAME:-value}`.
    EnvDefault,
    /// No source anchor; the declaration is hand-authored or reflected from a CLI parser.
    #[default]
    None,
}

impl ParameterBinding {
    /// Return the stable on-disk spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Const => "const",
            Self::Input => "input",
            Self::EnvDefault => "envdefault",
            Self::None => "none",
        }
    }

    fn parse(value: &str, fallback: Self) -> Self {
        match value {
            "const" => Self::Const,
            "input" => Self::Input,
            "envdefault" => Self::EnvDefault,
            "none" => Self::None,
            _ => fallback,
        }
    }

    const fn implied_delivery(self) -> Option<ParameterDelivery> {
        match self {
            Self::Const | Self::Input => Some(ParameterDelivery::Inject),
            Self::EnvDefault => Some(ParameterDelivery::Env),
            Self::None => None,
        }
    }
}

/// How a value reaches the program at runtime.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterDelivery {
    /// Rewrite or intercept a temporary program representation.
    Inject,
    /// Set an environment variable on the child process.
    Env,
    /// Assemble an argv flag or positional argument.
    #[default]
    Flag,
    /// Fill a command-template placeholder.
    Placeholder,
}

impl ParameterDelivery {
    /// Return the stable on-disk spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inject => "inject",
            Self::Env => "env",
            Self::Flag => "flag",
            Self::Placeholder => "placeholder",
        }
    }

    fn parse(value: &str, fallback: Self) -> Self {
        match value {
            "inject" => Self::Inject,
            "env" => Self::Env,
            "flag" => Self::Flag,
            "placeholder" => Self::Placeholder,
            _ => fallback,
        }
    }
}

/// The scalar/form type of one parameter.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    /// Free text.
    #[default]
    Str,
    /// A signed integer.
    Int,
    /// A finite floating-point number.
    Float,
    /// A boolean with the shared accepted word set.
    Bool,
    /// One value from a declared choice list.
    Choice,
    /// A path-shaped string; existence is not a domain invariant.
    Path,
}

impl ParameterType {
    /// Return the stable on-disk spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Str => "str",
            Self::Int => "int",
            Self::Float => "float",
            Self::Bool => "bool",
            Self::Choice => "choice",
            Self::Path => "path",
        }
    }

    fn parse(value: &str, fallback: Self) -> Self {
        match value {
            "str" => Self::Str,
            "int" => Self::Int,
            "float" => Self::Float,
            "bool" => Self::Bool,
            "choice" => Self::Choice,
            "path" => Self::Path,
            _ => fallback,
        }
    }
}

/// A serializable parameter default.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ParameterValue {
    /// Text, choice, or path value.
    String(String),
    /// Signed integer value.
    Integer(i64),
    /// Finite floating-point value.
    Float(f64),
    /// Boolean value.
    Bool(bool),
}

impl ParameterValue {
    fn to_json(&self) -> Value {
        match self {
            Self::String(value) => Value::String(value.clone()),
            Self::Integer(value) => Value::Number(Number::from(*value)),
            Self::Float(value) => Value::Number(
                Number::from_f64(*value).expect("parameter float defaults must be finite"),
            ),
            Self::Bool(value) => Value::Bool(*value),
        }
    }

    fn from_json(value: &Value) -> Option<Self> {
        match value {
            Value::String(value) => Some(Self::String(value.clone())),
            Value::Bool(value) => Some(Self::Bool(*value)),
            Value::Number(value) => value.as_i64().map(Self::Integer).or_else(|| {
                value
                    .as_f64()
                    .filter(|number| number.is_finite())
                    .map(Self::Float)
            }),
            Value::Null | Value::Array(_) | Value::Object(_) => None,
        }
    }
}

/// A symbolic parameter-model invariant violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterInvariant {
    /// A source binding was paired with a delivery other than the one it implies.
    BindingDeliveryMismatch,
    /// A choice parameter declared no possible choices.
    ChoiceWithoutChoices,
}

/// One universal parameter declaration.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParamDecl {
    /// Stable form/value key.
    pub name: String,
    /// Source anchoring semantics.
    pub binding: ParameterBinding,
    /// Runtime transport semantics.
    pub delivery: ParameterDelivery,
    /// Scalar/form type.
    pub parameter_type: ParameterType,
    /// Optional typed default.
    pub default: Option<ParameterValue>,
    /// Whether an empty value is refused.
    pub required: bool,
    /// Whether flag delivery accepts multiple values.
    pub multiple: bool,
    /// Whether multiple values repeat a flag rather than following one flag.
    pub repeat: bool,
    /// Closed values for a choice field.
    pub choices: Vec<String>,
    /// Form label or literal source prompt.
    pub prompt: String,
    /// Form help text.
    pub help: String,
    /// Whether the value must never be persisted.
    pub secret: bool,
    /// Environment variable from which a secret value is read.
    pub env_source: String,
    /// Flag spelling; blank means positional delivery.
    pub flag: String,
    /// Boolean flag action such as `store_true` or `store_false`.
    pub action: String,
    /// Source call-order key for input bindings.
    pub order: i64,
    /// Environment variable to set; blank defaults to the declaration name.
    pub env_target: String,
    /// Whether a static reader could not fully model the declaration.
    pub degraded: bool,
}

impl ParamDecl {
    /// Construct a declaration with the historical hand-declared defaults.
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            binding: ParameterBinding::None,
            delivery: ParameterDelivery::Flag,
            parameter_type: ParameterType::Str,
            default: None,
            required: false,
            multiple: false,
            repeat: false,
            choices: Vec::new(),
            prompt: String::new(),
            help: String::new(),
            secret: false,
            env_source: String::new(),
            flag: String::new(),
            action: String::new(),
            order: -1,
            env_target: String::new(),
            degraded: false,
        }
    }

    /// Return the variable set by environment delivery.
    #[must_use]
    pub fn env_var(&self) -> &str {
        if self.env_target.is_empty() {
            &self.name
        } else {
            &self.env_target
        }
    }

    /// Encode the frozen in-file `[tool.skit]` declaration shape.
    #[must_use]
    pub fn to_block_map(&self) -> BTreeMap<String, Value> {
        self.to_block_values()
            .into_iter()
            .map(|(key, value)| (key, value.to_json()))
            .collect()
    }

    /// Encode the frozen in-file declaration with its closed scalar value model.
    #[must_use]
    pub fn to_block_values(&self) -> BTreeMap<String, ParameterValue> {
        let mut output = BTreeMap::from([
            ("name".to_owned(), ParameterValue::String(self.name.clone())),
            (
                "kind".to_owned(),
                ParameterValue::String(self.binding.as_str().to_owned()),
            ),
            (
                "type".to_owned(),
                ParameterValue::String(self.parameter_type.as_str().to_owned()),
            ),
        ]);
        if let Some(default) = &self.default {
            output.insert("default".to_owned(), default.clone());
        }
        insert_nonempty_parameter_value(&mut output, "prompt", &self.prompt);
        if self.order >= 0 {
            output.insert("order".to_owned(), ParameterValue::Integer(self.order));
        }
        if self.secret {
            output.insert("secret".to_owned(), ParameterValue::Bool(true));
        }
        insert_nonempty_parameter_value(&mut output, "env_source", &self.env_source);
        output
    }

    /// Decode a user-editable block without allowing malformed scalars to escape the boundary.
    #[must_use]
    pub fn from_block_map(input: &BTreeMap<String, Value>) -> Self {
        let binding = ParameterBinding::parse(
            &string_value(input.get("kind"), "const"),
            ParameterBinding::Const,
        );
        let mut declaration = Self::new(string_value(input.get("name"), ""));
        declaration.binding = binding;
        declaration.delivery = binding
            .implied_delivery()
            .unwrap_or(ParameterDelivery::Flag);
        declaration.parameter_type =
            ParameterType::parse(&string_value(input.get("type"), "str"), ParameterType::Str);
        declaration.default = input.get("default").and_then(ParameterValue::from_json);
        declaration.prompt = string_value(input.get("prompt"), "");
        declaration.order = integer_value(input.get("order")).unwrap_or(-1);
        declaration.secret = input.get("secret").is_some_and(truthy);
        declaration.env_source = string_value(input.get("env_source"), "");
        declaration
    }

    /// Encode the full `meta.toml [[parameters]]` row while omitting default values.
    #[must_use]
    pub fn to_meta_map(&self) -> BTreeMap<String, Value> {
        let mut output = BTreeMap::from([
            ("name".to_owned(), Value::String(self.name.clone())),
            (
                "delivery".to_owned(),
                Value::String(self.delivery.as_str().to_owned()),
            ),
            (
                "type".to_owned(),
                Value::String(self.parameter_type.as_str().to_owned()),
            ),
        ]);
        if self.binding != ParameterBinding::None {
            output.insert(
                "binding".to_owned(),
                Value::String(self.binding.as_str().to_owned()),
            );
        }
        if let Some(default) = &self.default {
            output.insert("default".to_owned(), default.to_json());
        }
        if !self.choices.is_empty() {
            output.insert(
                "choices".to_owned(),
                Value::Array(self.choices.iter().cloned().map(Value::String).collect()),
            );
        }
        if self.order >= 0 {
            output.insert("order".to_owned(), Value::Number(Number::from(self.order)));
        }
        insert_true(&mut output, "required", self.required);
        insert_true(&mut output, "multiple", self.multiple);
        insert_true(&mut output, "repeat", self.repeat);
        insert_nonempty(&mut output, "prompt", &self.prompt);
        insert_nonempty(&mut output, "help", &self.help);
        insert_true(&mut output, "secret", self.secret);
        insert_nonempty(&mut output, "env_source", &self.env_source);
        insert_nonempty(&mut output, "flag", &self.flag);
        insert_nonempty(&mut output, "action", &self.action);
        insert_nonempty(&mut output, "env_target", &self.env_target);
        output
    }

    /// Decode a user-editable metadata row using total, backward-compatible defaults.
    #[must_use]
    pub fn from_meta_map(input: &BTreeMap<String, Value>) -> Self {
        let mut declaration = Self::new(string_value(input.get("name"), ""));
        declaration.binding = ParameterBinding::parse(
            &string_value(input.get("binding"), "none"),
            ParameterBinding::None,
        );
        declaration.delivery = ParameterDelivery::parse(
            &string_value(input.get("delivery"), "flag"),
            ParameterDelivery::Flag,
        );
        declaration.parameter_type =
            ParameterType::parse(&string_value(input.get("type"), "str"), ParameterType::Str);
        declaration.default = input.get("default").and_then(ParameterValue::from_json);
        declaration.choices = match input.get("choices") {
            Some(Value::Array(values)) => values.iter().map(stringify).collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        declaration.order = integer_value(input.get("order")).unwrap_or(-1);
        declaration.required = input.get("required").is_some_and(truthy);
        declaration.multiple = input.get("multiple").is_some_and(truthy);
        declaration.repeat = input.get("repeat").is_some_and(truthy);
        declaration.prompt = string_value(input.get("prompt"), "");
        declaration.help = string_value(input.get("help"), "");
        declaration.secret = input.get("secret").is_some_and(truthy);
        declaration.env_source = string_value(input.get("env_source"), "");
        declaration.flag = string_value(input.get("flag"), "");
        declaration.action = string_value(input.get("action"), "");
        declaration.env_target = string_value(input.get("env_target"), "");
        declaration
    }

    /// Return the first symbolic invariant violation, or `None` when the declaration is coherent.
    #[must_use]
    pub fn validate(&self) -> Option<ParameterInvariant> {
        if self
            .binding
            .implied_delivery()
            .is_some_and(|delivery| delivery != self.delivery)
        {
            return Some(ParameterInvariant::BindingDeliveryMismatch);
        }
        if self.parameter_type == ParameterType::Choice && self.choices.is_empty() {
            return Some(ParameterInvariant::ChoiceWithoutChoices);
        }
        None
    }

    /// Repair the source-binding delivery implication while leaving free declarations unchanged.
    #[must_use]
    pub fn normalized(mut self) -> Self {
        if let Some(delivery) = self.binding.implied_delivery() {
            self.delivery = delivery;
        }
        self
    }
}

/// A typed-default coercion failure.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("{value:?} is not a valid {parameter_type} default")]
pub struct DefaultCoercionError {
    value: String,
    parameter_type: &'static str,
}

impl Localize for DefaultCoercionError {
    fn message(&self) -> Message {
        Message::new("{} is not a valid {} default")
            .quoted(&self.value)
            .with(self.parameter_type)
    }
}

/// Coerce one user-entered default according to the declaration's scalar type.
pub fn coerce_default(
    value: &str,
    parameter_type: ParameterType,
) -> Result<ParameterValue, DefaultCoercionError> {
    let invalid = || DefaultCoercionError {
        value: value.to_owned(),
        parameter_type: parameter_type.as_str(),
    };
    match parameter_type {
        ParameterType::Int => value
            .parse::<i64>()
            .map(ParameterValue::Integer)
            .map_err(|_| invalid()),
        ParameterType::Float => value
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite())
            .map(ParameterValue::Float)
            .ok_or_else(invalid),
        ParameterType::Bool => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "y" | "on" => Ok(ParameterValue::Bool(true)),
            "false" | "0" | "no" | "n" | "off" => Ok(ParameterValue::Bool(false)),
            _ => Err(invalid()),
        },
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => {
            Ok(ParameterValue::String(value.to_owned()))
        }
    }
}

fn insert_nonempty(output: &mut BTreeMap<String, Value>, key: &str, value: &str) {
    if !value.is_empty() {
        output.insert(key.to_owned(), Value::String(value.to_owned()));
    }
}

fn insert_nonempty_parameter_value(
    output: &mut BTreeMap<String, ParameterValue>,
    key: &str,
    value: &str,
) {
    if !value.is_empty() {
        output.insert(key.to_owned(), ParameterValue::String(value.to_owned()));
    }
}

fn insert_true(output: &mut BTreeMap<String, Value>, key: &str, value: bool) {
    if value {
        output.insert(key.to_owned(), Value::Bool(true));
    }
}

fn string_value(value: Option<&Value>, default: &str) -> String {
    value.map_or_else(|| default.to_owned(), stringify)
}

fn stringify(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Null => "None".to_owned(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn integer_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        // A stored `order` that is a non-integer float truncates toward zero, matching the oracle's
        // `int(d.get("order", -1))` (Python `int(1.9) == 1`), not degrading to the -1 default.
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value as i64)),
        Value::String(value) => value.parse().ok(),
        Value::Bool(_) | Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}
