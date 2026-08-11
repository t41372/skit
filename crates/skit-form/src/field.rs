//! One typed, frontend-neutral editable field.
//!
//! Every skit editing surface — the launch form, the add review, and the settings screen — asks a
//! user for the same small set of shapes. Modelling them once means a frontend renders a control
//! from its kind instead of guessing from a label, and a host reads an intent instead of inferring
//! one from an empty string.
//!
//! Two rules come straight from version 0.4 and are structural here rather than advisory.
//!
//! An empty string is not an intent. Version 0.4 distinguishes "the user cleared this, deliver an
//! empty value" from "nothing explicit here, the source's own default applies" — its `show --json`
//! exposes the difference as `delivers_empty` (`src/skit/cli.py:2352-2359`). [`FieldValue`] carries
//! that distinction in the type, so `String::is_empty` can never stand in for it.
//!
//! A baseline is captured when a form opens, never re-read when it saves. Version 0.4's settings
//! screen says why: a save-time re-read classifies an untouched field as an edit whenever a
//! concurrent write moves the block underneath (`src/skit/tui_settings.py`, the dirty-guard
//! comment). [`Field`] owns its baseline from construction and [`Field::is_dirty`] takes no
//! argument, so there is no call that could supply a fresher one.

use serde::{Deserialize, Serialize};

/// How one field's argument text is split into pieces.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentDialect {
    /// POSIX shell word splitting, used by a multiple-valued parameter.
    Posix,
    /// The platform's own argument-vector rules, used by the argument tail.
    Argv,
}

/// One selectable option and the text a frontend shows for it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChoiceOption {
    /// Stable value stored and submitted.
    pub value: String,
    /// Text shown to a person. Equal to the value when a kind has no separate wording.
    pub label: String,
    /// Extra description a frontend can show beside the label.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

impl ChoiceOption {
    /// Build one option whose label is its value.
    #[must_use]
    pub fn plain(value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            label: value.clone(),
            value,
            detail: String::new(),
        }
    }

    /// Build one option with separate stored and shown text.
    #[must_use]
    pub fn labelled(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: String::new(),
        }
    }

    /// Attach description text a frontend can show beside the label.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }
}

/// The control one field needs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum FieldKind {
    /// One line of free text.
    Text,
    /// Free text that keeps embedded line breaks.
    Multiline,
    /// Text a frontend must not display and a state store must never persist.
    Secret,
    /// An on or off toggle.
    Boolean,
    /// Exactly one option from a closed set.
    SingleChoice {
        /// Selectable options in display order.
        options: Vec<ChoiceOption>,
    },
    /// Any number of options from a closed set.
    MultiChoice {
        /// Selectable options in display order.
        options: Vec<ChoiceOption>,
    },
    /// A number, whole or fractional.
    Number {
        /// Whether a fractional part is refused.
        integer: bool,
    },
    /// A filesystem path.
    Path {
        /// Whether the value names a directory rather than a file.
        directory: bool,
    },
    /// Text split into an argument vector by one dialect.
    ArgumentList {
        /// Splitting rules this field's text follows.
        dialect: ArgumentDialect,
    },
    /// Text a person reads but cannot change. [`Field::read_only_reason`] says why.
    ReadOnly,
}

impl FieldKind {
    /// Report whether this kind can ever accept an edit.
    #[must_use]
    pub const fn editable(&self) -> bool {
        !matches!(self, Self::ReadOnly)
    }
}

/// One value a field can hold, typed by the control that produced it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum TypedValue {
    /// Free text, possibly empty. An empty string here is a deliberate clear.
    Text(String),
    /// A toggle state.
    Boolean(bool),
    /// A whole number.
    Integer(i64),
    /// A fractional number.
    Decimal(f64),
    /// One selected option value.
    Choice(String),
    /// Every selected option value, in the field's own option order.
    Choices(Vec<String>),
    /// An already-split argument vector.
    Arguments(Vec<String>),
}

impl TypedValue {
    /// Render this value as the text a frontend edits.
    #[must_use]
    pub fn as_text(&self) -> String {
        match self {
            Self::Text(value) | Self::Choice(value) => value.clone(),
            Self::Boolean(value) => if *value { "true" } else { "false" }.to_owned(),
            Self::Integer(value) => value.to_string(),
            Self::Decimal(value) => value.to_string(),
            Self::Choices(values) => values.join(", "),
            Self::Arguments(values) => values.join(" "),
        }
    }
}

/// What one field currently holds.
///
/// The two variants are not interchangeable. `Inherit` means nothing explicit is set here, so a
/// lower-precedence source decides; `Explicit(Text(String::new()))` means the user cleared the
/// field on purpose and an empty value is what a run must deliver.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "value")]
pub enum FieldValue {
    /// No explicit value. Whatever sits below in the precedence order applies.
    Inherit,
    /// An explicit value, including a deliberately empty one.
    Explicit(TypedValue),
}

impl FieldValue {
    /// Build an explicit text value.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Explicit(TypedValue::Text(value.into()))
    }

    /// Build an explicit toggle value.
    #[must_use]
    pub const fn boolean(value: bool) -> Self {
        Self::Explicit(TypedValue::Boolean(value))
    }

    /// Return the explicit value, if this field carries one.
    #[must_use]
    pub const fn explicit(&self) -> Option<&TypedValue> {
        match self {
            Self::Inherit => None,
            Self::Explicit(value) => Some(value),
        }
    }

    /// Report whether the user deliberately cleared this field.
    ///
    /// This is the case an empty string alone cannot express: a cleared text field delivers an
    /// empty value, while an inherited one delivers nothing at all.
    #[must_use]
    pub fn is_cleared(&self) -> bool {
        matches!(self.explicit(), Some(TypedValue::Text(value)) if value.is_empty())
    }

    /// Render the text a frontend edits. An inherited field renders as empty text.
    #[must_use]
    pub fn as_text(&self) -> String {
        self.explicit().map(TypedValue::as_text).unwrap_or_default()
    }
}

/// Which layer of the record owns one field.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOwner {
    /// The user declared it by hand and skit stores it in entry metadata.
    Declared,
    /// The stored source's own skit block declares it.
    SourceBlock,
    /// The script's own command-line reader declares it.
    SourceReader,
    /// A command or prompt template's placeholder declares it.
    Template,
    /// Entry policy that is not a parameter at all.
    EntryPolicy,
}

/// Why a field cannot be edited here.
///
/// A frontend renders the reason instead of inventing wording, so every surface explains one
/// refusal the same way.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyReason {
    /// The script itself declares this, so skit would have to rewrite the user's own code.
    SourceDeclares,
    /// The entry links a file skit never writes.
    ReferenceMode,
    /// The value was fixed when the entry was added and a different command changes it.
    FixedAtAddTime,
    /// The value is derived from another field and follows it.
    Derived,
}

/// What extra help one field offers beyond typing into it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct FieldCapabilities {
    /// Open the filesystem picker.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub browse: bool,
    /// Open the run-time value token menu.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub insert_token: bool,
    /// Open the environment-variable picker.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub choose_environment: bool,
    /// Restore the declared default.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub reset_default: bool,
    /// Define a new prompt runner without leaving the form.
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    pub new_runner: bool,
}

impl FieldCapabilities {
    /// Report whether this field offers any affordance at all.
    #[must_use]
    pub const fn any(self) -> bool {
        self.browse
            || self.insert_token
            || self.choose_environment
            || self.reset_default
            || self.new_runner
    }
}

/// One typed editable field with its own baseline.
///
/// The baseline is whatever the value was when this field was built. Nothing can replace it later:
/// [`Field::is_dirty`] compares against it and takes no argument, so a save-time re-read cannot
/// become the comparison basis. Version 0.4 captures its dirty baseline at open time for exactly
/// this reason — a concurrent write that moves the stored block must not turn an untouched field
/// into an edit.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Field {
    /// Stable key a host reads the submitted value back by.
    pub key: String,
    /// Label text. Application copy unless `translate_label` is false.
    pub label: String,
    /// Control this field needs.
    pub kind: FieldKind,
    /// Layer of the record that owns the field.
    pub owner: FieldOwner,
    /// One-line help shown under the control.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub help: String,
    /// Affordances beyond typing.
    #[serde(default, skip_serializing_if = "is_default_capabilities")]
    pub capabilities: FieldCapabilities,
    /// Whether the label is application copy a catalog translates.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub translate_label: bool,
    /// Why the field refuses edits, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_reason: Option<ReadOnlyReason>,
    value: FieldValue,
    baseline: FieldValue,
}

const fn yes() -> bool {
    true
}

const fn is_yes(value: &bool) -> bool {
    *value
}

fn is_default_capabilities(value: &FieldCapabilities) -> bool {
    *value == FieldCapabilities::default()
}

impl Field {
    /// Build one editable field whose baseline is its current value.
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        kind: FieldKind,
        owner: FieldOwner,
        value: FieldValue,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            owner,
            help: String::new(),
            capabilities: FieldCapabilities::default(),
            translate_label: true,
            read_only_reason: None,
            baseline: value.clone(),
            value,
        }
    }

    /// Build one field a person reads but cannot change, and say why.
    #[must_use]
    pub fn read_only(
        key: impl Into<String>,
        label: impl Into<String>,
        owner: FieldOwner,
        value: FieldValue,
        reason: ReadOnlyReason,
    ) -> Self {
        let mut field = Self::new(key, label, FieldKind::ReadOnly, owner, value);
        field.read_only_reason = Some(reason);
        field
    }

    /// Attach one-line help.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = help.into();
        self
    }

    /// Attach the affordances this field offers.
    #[must_use]
    pub const fn with_capabilities(mut self, capabilities: FieldCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Keep the label as user data rather than catalog copy.
    #[must_use]
    pub const fn with_verbatim_label(mut self) -> Self {
        self.translate_label = false;
        self
    }

    /// Return the current value.
    #[must_use]
    pub const fn value(&self) -> &FieldValue {
        &self.value
    }

    /// Return the value this field opened with.
    #[must_use]
    pub const fn baseline(&self) -> &FieldValue {
        &self.baseline
    }

    /// Replace the current value.
    ///
    /// A field that refuses edits ignores this and reports false, so a host cannot write through a
    /// control a frontend never offered.
    pub fn set_value(&mut self, value: FieldValue) -> bool {
        if !self.kind.editable() {
            return false;
        }
        self.value = value;
        true
    }

    /// Report whether the value moved away from the baseline this field opened with.
    ///
    /// There is deliberately no variant that accepts a caller-supplied baseline.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.value != self.baseline
    }
}

/// Report whether any field in one group moved away from its own baseline.
#[must_use]
pub fn any_dirty(fields: &[Field]) -> bool {
    fields.iter().any(Field::is_dirty)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_field(value: FieldValue) -> Field {
        Field::new("name", "Name", FieldKind::Text, FieldOwner::Declared, value)
    }

    /// An empty string cannot say which of two different intents the user had.
    ///
    /// Version 0.4 reports the difference to automation as `delivers_empty`
    /// (`src/skit/cli.py:2352-2359`): a cleared field delivers an empty value, an inherited one
    /// delivers nothing and the script's own default applies.
    #[test]
    fn a_cleared_field_is_not_an_inherited_one() {
        let cleared = FieldValue::text("");
        let inherited = FieldValue::Inherit;

        assert_ne!(cleared, inherited);
        assert!(cleared.is_cleared());
        assert!(!inherited.is_cleared());
        // Both render as empty text, which is exactly why the text alone cannot carry the intent.
        assert_eq!(cleared.as_text(), "");
        assert_eq!(inherited.as_text(), "");
        assert!(cleared.explicit().is_some());
        assert!(inherited.explicit().is_none());
    }

    /// Moving between the two states is an edit, even though neither shows any text.
    #[test]
    fn clearing_an_inherited_field_is_a_real_edit() {
        let mut field = text_field(FieldValue::Inherit);
        assert!(!field.is_dirty());
        assert!(field.set_value(FieldValue::text("")));
        assert!(field.is_dirty(), "clearing an inherited field is an edit");
    }

    /// The baseline is captured at construction and nothing can replace it.
    ///
    /// Version 0.4 captures its dirty baseline when the settings screen opens, because a save-time
    /// re-read turns an untouched field into an edit whenever a concurrent write moves the stored
    /// block underneath. `is_dirty` takes no argument, so that mistake has no way to be expressed.
    #[test]
    fn the_baseline_is_owned_by_the_field_and_survives_every_edit() {
        let mut field = text_field(FieldValue::text("original"));
        assert_eq!(field.baseline(), &FieldValue::text("original"));

        field.set_value(FieldValue::text("edited"));
        assert!(field.is_dirty());
        assert_eq!(
            field.baseline(),
            &FieldValue::text("original"),
            "an edit must not move the baseline"
        );

        // Typing the original text back is not an edit.
        field.set_value(FieldValue::text("original"));
        assert!(!field.is_dirty());
    }

    /// A read-only field refuses writes rather than accepting one a save cannot keep.
    #[test]
    fn a_read_only_field_cannot_be_written_through() {
        let mut field = Field::read_only(
            "type",
            "Type",
            FieldOwner::SourceReader,
            FieldValue::text("str"),
            ReadOnlyReason::SourceDeclares,
        );
        assert!(!field.kind.editable());
        assert!(!field.set_value(FieldValue::text("int")));
        assert_eq!(field.value(), &FieldValue::text("str"));
        assert!(!field.is_dirty());
        assert_eq!(
            field.read_only_reason,
            Some(ReadOnlyReason::SourceDeclares),
            "a refusal must carry its reason so a frontend can explain it"
        );
    }

    #[test]
    fn a_group_is_dirty_when_any_member_moved() {
        let mut fields = vec![
            text_field(FieldValue::text("a")),
            text_field(FieldValue::Inherit),
        ];
        assert!(!any_dirty(&fields));
        fields[1].set_value(FieldValue::text(""));
        assert!(any_dirty(&fields));
    }

    /// Every typed value renders the text a frontend would put in its control.
    #[test]
    fn typed_values_render_the_text_their_control_edits() {
        assert_eq!(TypedValue::Text("x".to_owned()).as_text(), "x");
        assert_eq!(TypedValue::Boolean(true).as_text(), "true");
        assert_eq!(TypedValue::Boolean(false).as_text(), "false");
        assert_eq!(TypedValue::Integer(3).as_text(), "3");
        assert_eq!(TypedValue::Choice("json".to_owned()).as_text(), "json");
        assert_eq!(
            TypedValue::Choices(vec!["a".to_owned(), "b".to_owned()]).as_text(),
            "a, b"
        );
        assert_eq!(
            TypedValue::Arguments(vec!["--force".to_owned(), "x".to_owned()]).as_text(),
            "--force x"
        );
    }

    /// The model round-trips through JSON so a future non-terminal frontend can consume it.
    #[test]
    fn the_field_model_round_trips_through_json() {
        let mut field = Field::new(
            "format",
            "Format",
            FieldKind::SingleChoice {
                options: vec![
                    ChoiceOption::plain("json"),
                    ChoiceOption::labelled("yaml", "YAML").with_detail("human readable"),
                ],
            },
            FieldOwner::Declared,
            FieldValue::Explicit(TypedValue::Choice("json".to_owned())),
        )
        .with_help("Output format")
        .with_capabilities(FieldCapabilities {
            reset_default: true,
            ..FieldCapabilities::default()
        });
        field.set_value(FieldValue::Explicit(TypedValue::Choice("yaml".to_owned())));

        let json = serde_json::to_string(&field).unwrap();
        let restored: Field = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, field);
        assert!(restored.is_dirty(), "the baseline survives the round trip");
    }
}
