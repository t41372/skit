//! The Settings screen's parameter section, as a frontend-neutral plan.
//!
//! Version 0.4 decides what this section is before it decides what any row looks like
//! (`src/skit/tui_settings.py:588-631`), and the decision is the whole point: a parameter the
//! script itself declares is not offered as an editable row, because saving one could not keep the
//! edit. Modelling the decision here means no frontend can render a control the host would then
//! discard, and no host needs a submit-time filter to undo a row it should never have drawn.

use skit_domain::parameters::{ParamDecl, ParameterType};

use crate::field::{
    Field, FieldCapabilities, FieldKind, FieldOwner, FieldValue, ReadOnlyReason, TypedValue,
};

/// The stable key suffix of the keep or remove toggle on one parameter row.
pub const KEEP_KEY: &str = "keep";

/// What the parameter section is for one entry.
#[derive(Clone, Debug, PartialEq)]
pub enum ParameterSection {
    /// The kind keeps no managed parameters at all.
    ///
    /// Version 0.4 says so in one line rather than drawing an empty box
    /// (`src/skit/tui_settings.py:594-596`).
    Unsupported,
    /// The entry links a file skit never writes, so the block is maintained in the source.
    ///
    /// The rows are readable and nothing is editable (`src/skit/tui_settings.py:597-606`).
    Reference {
        /// One read-only line per declared parameter, in declaration order.
        rows: Vec<ReadOnlyParameterRow>,
    },
    /// The entry's schema is hand-declared, so every axis is the user's to change.
    ///
    /// Version 0.4 uses `DeclParamRow` here (`src/skit/tui_settings.py:151-230`).
    Declared {
        /// One editable row per declared parameter.
        rows: Vec<ParameterRow>,
    },
    /// The stored source owns the schema.
    ///
    /// Version 0.4 uses `ParamRow` (`src/skit/tui_settings.py:73-138`) and then explains what else
    /// the user can do with the source, if anything.
    SourceManaged {
        /// One row per parameter the source's skit block declares.
        rows: Vec<ParameterRow>,
        /// What follows the rows.
        followup: SourceFollowup,
    },
}

/// What version 0.4 shows after the source-managed rows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceFollowup {
    /// Nothing to add.
    None,
    /// The script's own reader already supplies the run form.
    ///
    /// Managing a constant here would write a block that shadows that whole form, so version 0.4
    /// explains instead of offering the checkboxes (`src/skit/tui_settings.py:612-623`).
    ReaderDriven,
    /// Constants the analyzer found that nobody manages yet.
    Offer {
        /// Candidate names in detection order.
        candidates: Vec<String>,
    },
}

/// One parameter a person can read but not change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOnlyParameterRow {
    /// Parameter name.
    pub name: String,
    /// Declared type, as version 0.4 prints it in `· name (type)`.
    pub parameter_type: String,
}

/// One editable parameter row and everything it may change.
#[derive(Clone, Debug, PartialEq)]
pub struct ParameterRow {
    /// Parameter name, stable across a session.
    pub name: String,
    /// Read-only header text version 0.4 puts beside the keep toggle.
    pub summary: String,
    /// The keep toggle and every editable axis, in display order.
    pub fields: Vec<Field>,
}

impl ParameterRow {
    /// Return the field for one axis of this row.
    #[must_use]
    pub fn field(&self, axis: &str) -> Option<&Field> {
        let suffix = format!(":{axis}");
        self.fields
            .iter()
            .find(|field| field.key.ends_with(suffix.as_str()))
    }

    /// Report whether this row offers one axis as an editable control.
    #[must_use]
    pub fn offers(&self, axis: &str) -> bool {
        self.field(axis).is_some_and(|field| field.kind.editable())
    }
}

/// Inputs the section decision needs, gathered by the host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParameterSectionContext<'a> {
    /// Entry kind.
    pub kind: &'a str,
    /// Whether the entry links a source skit never writes.
    pub reference_mode: bool,
    /// Whether the kind keeps its schema in entry metadata rather than in the source.
    pub declared_schema: bool,
    /// Whether the kind has a source analyzer at all.
    pub has_analyzer: bool,
    /// How many fields the script's own command-line reader models.
    pub reader_fields: usize,
}

/// Build the parameter section for one entry.
///
/// `managed` are the parameters the source's own skit block declares, and `candidates` are
/// constants the analyzer found that nobody manages yet.
#[must_use]
pub fn parameter_section(
    context: ParameterSectionContext<'_>,
    managed: &[ParamDecl],
    candidates: &[String],
) -> ParameterSection {
    // Version 0.4 asks these questions in this order (`src/skit/tui_settings.py:588-631`).
    if context.declared_schema {
        return ParameterSection::Declared {
            rows: managed
                .iter()
                .enumerate()
                .map(|(index, declaration)| declared_row(index, declaration, context.kind))
                .collect(),
        };
    }
    if !context.has_analyzer {
        return ParameterSection::Unsupported;
    }
    if context.reference_mode {
        return ParameterSection::Reference {
            rows: managed
                .iter()
                .map(|declaration| ReadOnlyParameterRow {
                    name: declaration.name.clone(),
                    parameter_type: declaration.parameter_type.as_str().to_owned(),
                })
                .collect(),
        };
    }
    let rows = managed
        .iter()
        .enumerate()
        .map(|(index, declaration)| source_managed_row(index, declaration))
        .collect();
    // The reader trap only exists while nothing is managed: once a block exists it already serves
    // the form, so managing another constant is additive (`src/skit/tui_settings.py:787-798`).
    let followup = if managed.is_empty() && context.reader_fields > 0 {
        SourceFollowup::ReaderDriven
    } else if candidates.is_empty() {
        SourceFollowup::None
    } else {
        SourceFollowup::Offer {
            candidates: candidates.to_vec(),
        }
    };
    ParameterSection::SourceManaged { rows, followup }
}

/// Build one row for a parameter the source's skit block declares.
///
/// Version 0.4's `ParamRow` prints the name, type and default as read-only text beside the keep
/// toggle and offers exactly three editable axes: the form label, the secret flag and the
/// environment source (`src/skit/tui_settings.py:98-118`). Type and default are the script's own
/// text, so offering them here would promise an edit that saving could not keep.
fn source_managed_row(index: usize, declaration: &ParamDecl) -> ParameterRow {
    let prefix = format!("parameter:{index}");
    let default = declaration
        .default
        .as_ref()
        .map_or_else(String::new, render_default);
    let summary = if default.is_empty() {
        format!(
            "{}  {}",
            declaration.name,
            declaration.parameter_type.as_str()
        )
    } else {
        format!(
            "{}  {} {}",
            declaration.name,
            declaration.parameter_type.as_str(),
            default
        )
    };
    let mut fields = vec![
        keep_field(&prefix, "unmanage this parameter"),
        Field::read_only(
            format!("{prefix}:type"),
            "Type",
            FieldOwner::SourceBlock,
            FieldValue::text(declaration.parameter_type.as_str()),
            ReadOnlyReason::SourceDeclares,
        ),
        Field::read_only(
            format!("{prefix}:default"),
            "Default",
            FieldOwner::SourceBlock,
            FieldValue::text(default),
            ReadOnlyReason::SourceDeclares,
        ),
    ];
    fields.extend(shared_editable_axes(
        &prefix,
        declaration,
        FieldOwner::SourceBlock,
    ));
    ParameterRow {
        name: declaration.name.clone(),
        summary,
        fields,
    }
}

/// Build one row for a parameter the user declared by hand.
///
/// Version 0.4's `DeclParamRow` makes type, default, choices, help, required and — on a binary
/// kind — the flag editable, and keeps delivery as read-only header text because a different
/// command changes it (`src/skit/tui_settings.py:151-230`).
fn declared_row(index: usize, declaration: &ParamDecl, kind: &str) -> ParameterRow {
    let prefix = format!("parameter:{index}");
    let mut fields = vec![
        keep_field(&prefix, "remove this parameter"),
        Field::read_only(
            format!("{prefix}:delivery"),
            "Delivery",
            FieldOwner::Declared,
            FieldValue::text(declaration.delivery.as_str()),
            ReadOnlyReason::FixedAtAddTime,
        ),
        Field::new(
            format!("{prefix}:type"),
            "Type",
            FieldKind::SingleChoice {
                options: parameter_type_options(),
            },
            FieldOwner::Declared,
            FieldValue::Explicit(TypedValue::Choice(
                declaration.parameter_type.as_str().to_owned(),
            )),
        ),
        Field::new(
            format!("{prefix}:default"),
            "Default",
            FieldKind::Text,
            FieldOwner::Declared,
            declaration
                .default
                .as_ref()
                .map_or(FieldValue::Inherit, |value| {
                    FieldValue::text(render_default(value))
                }),
        )
        .with_capabilities(FieldCapabilities {
            reset_default: true,
            ..FieldCapabilities::default()
        }),
        Field::new(
            format!("{prefix}:choices"),
            "Choices",
            FieldKind::Text,
            FieldOwner::Declared,
            FieldValue::text(declaration.choices.join(", ")),
        )
        .with_help("comma separated (for type: choice)"),
        Field::new(
            format!("{prefix}:help"),
            "Help",
            FieldKind::Multiline,
            FieldOwner::Declared,
            FieldValue::text(&declaration.help),
        ),
    ];
    // Version 0.4 shows the flag only for a kind whose launch takes an argument vector.
    if shows_flag(kind) {
        fields.push(
            Field::new(
                format!("{prefix}:flag"),
                "Flag",
                FieldKind::Text,
                FieldOwner::Declared,
                FieldValue::text(&declaration.flag),
            )
            .with_help("--flag (empty = positional)"),
        );
    }
    fields.push(Field::new(
        format!("{prefix}:required"),
        "required",
        FieldKind::Boolean,
        FieldOwner::Declared,
        FieldValue::boolean(declaration.required),
    ));
    fields.extend(shared_editable_axes(
        &prefix,
        declaration,
        FieldOwner::Declared,
    ));
    ParameterRow {
        name: declaration.name.clone(),
        summary: format!("{}  {}", declaration.name, declaration.delivery.as_str()),
        fields,
    }
}

/// The three axes both editable row shapes share.
///
/// Version 0.4 puts the same form label, secret toggle and environment source at the bottom of
/// both rows (`src/skit/tui_settings.py:106-117` and `:216-227`).
fn shared_editable_axes(prefix: &str, declaration: &ParamDecl, owner: FieldOwner) -> Vec<Field> {
    vec![
        Field::new(
            format!("{prefix}:prompt"),
            "Form label:",
            FieldKind::Text,
            owner,
            FieldValue::text(&declaration.prompt),
        ),
        Field::new(
            format!("{prefix}:secret"),
            "secret (never saved to disk)",
            FieldKind::Boolean,
            owner,
            FieldValue::boolean(declaration.secret),
        ),
        Field::new(
            format!("{prefix}:env_source"),
            "env variable to read it from (optional)",
            FieldKind::Text,
            owner,
            FieldValue::text(&declaration.env_source),
        )
        .with_capabilities(FieldCapabilities {
            choose_environment: true,
            ..FieldCapabilities::default()
        }),
    ]
}

fn keep_field(prefix: &str, help: &str) -> Field {
    Field::new(
        format!("{prefix}:{KEEP_KEY}"),
        "keep",
        FieldKind::Boolean,
        FieldOwner::Declared,
        FieldValue::boolean(true),
    )
    .with_help(help)
}

/// Report whether a kind's launch takes an argument vector, so a flag means anything.
const fn shows_flag(kind: &str) -> bool {
    matches!(kind.as_bytes(), b"exe")
}

fn parameter_type_options() -> Vec<crate::field::ChoiceOption> {
    ["str", "int", "float", "bool", "choice", "path"]
        .into_iter()
        .map(crate::field::ChoiceOption::plain)
        .collect()
}

/// Render a declared default the way version 0.4's editable input shows it.
///
/// A boolean uses the `true`/`false` words its coercion round-trips
/// (`src/skit/tui_settings.py:141-148`).
fn render_default(value: &skit_domain::parameters::ParameterValue) -> String {
    use skit_domain::parameters::ParameterValue;
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => value.to_string(),
        ParameterValue::Bool(value) => if *value { "true" } else { "false" }.to_owned(),
    }
}

/// Apply version 0.4's secrecy rule to one collected row.
///
/// Marking a public parameter secret drops the cached default, because the source literal must not
/// be serialized into the block or later exposed through `--json`
/// (`src/skit/tui_settings.py:125-131`). The environment source only survives while the row is
/// secret (`:132`).
pub fn apply_secrecy_rule(declaration: &mut ParamDecl, was_secret: bool) {
    if declaration.secret && !was_secret {
        declaration.default = None;
    }
    if !declaration.secret {
        declaration.env_source.clear();
    }
}

/// Report whether one parameter type names a closed option set.
#[must_use]
pub const fn is_choice(parameter_type: ParameterType) -> bool {
    matches!(parameter_type, ParameterType::Choice)
}

#[cfg(test)]
mod tests {
    use skit_domain::parameters::{ParameterDelivery, ParameterValue};

    use super::*;

    fn context(kind: &'static str) -> ParameterSectionContext<'static> {
        ParameterSectionContext {
            kind,
            reference_mode: false,
            declared_schema: false,
            has_analyzer: true,
            reader_fields: 0,
        }
    }

    fn declaration(name: &str) -> ParamDecl {
        let mut declaration = ParamDecl::new(name);
        declaration.default = Some(ParameterValue::String("World".to_owned()));
        declaration
    }

    /// A parameter the script itself declares is not a row at all.
    ///
    /// Version 0.4 renders rows only for the block-managed set, and when nothing is managed while
    /// the script's own reader models a form it prints one sentence instead of offering
    /// checkboxes — managing a constant would write a block that shadows that whole form
    /// (`src/skit/tui_settings.py:610-623` and `:787-798`).
    #[test]
    fn a_reader_driven_entry_gets_an_explanation_and_no_rows() {
        let section = parameter_section(
            ParameterSectionContext {
                reader_fields: 2,
                ..context("python")
            },
            &[],
            &["NAME".to_owned()],
        );
        let ParameterSection::SourceManaged { rows, followup } = section else {
            panic!("expected the source-managed section");
        };
        assert!(rows.is_empty(), "a reader-derived parameter is not a row");
        assert_eq!(
            followup,
            SourceFollowup::ReaderDriven,
            "the candidates must not be offered while the reader owns the form"
        );
    }

    /// Once a block exists the trap is gone, so unmanaged constants are offered again.
    #[test]
    fn a_managed_entry_offers_the_remaining_candidates() {
        let section = parameter_section(
            ParameterSectionContext {
                reader_fields: 2,
                ..context("python")
            },
            &[declaration("GREETING")],
            &["WIDTH".to_owned()],
        );
        let ParameterSection::SourceManaged { rows, followup } = section else {
            panic!("expected the source-managed section");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            followup,
            SourceFollowup::Offer {
                candidates: vec!["WIDTH".to_owned()]
            }
        );
    }

    /// A block-managed row offers three axes and no more.
    ///
    /// Name, type and default are the script's own text, shown read-only beside the keep toggle
    /// (`src/skit/tui_settings.py:98-105`).
    #[test]
    fn a_source_managed_row_offers_only_the_axes_a_save_can_keep() {
        let section = parameter_section(context("python"), &[declaration("GREETING")], &[]);
        let ParameterSection::SourceManaged { rows, .. } = section else {
            panic!("expected the source-managed section");
        };
        let row = &rows[0];

        for axis in ["prompt", "secret", "env_source", KEEP_KEY] {
            assert!(row.offers(axis), "the row lost its editable {axis}");
        }
        for axis in [
            "type",
            "default",
            "choices",
            "help",
            "required",
            "flag",
            "action",
            "multiple",
            "repeat",
            "delivery",
            "binding",
            "env_target",
        ] {
            assert!(
                !row.offers(axis),
                "a source-managed row offered {axis}, which saving cannot keep"
            );
        }
        // The read-only axes are still present so a person can read them.
        let parameter_type = row.field("type").expect("no type text");
        assert_eq!(parameter_type.kind, FieldKind::ReadOnly);
        assert_eq!(
            parameter_type.read_only_reason,
            Some(ReadOnlyReason::SourceDeclares)
        );
        assert!(row.summary.contains("GREETING"), "{}", row.summary);
        assert!(row.summary.contains("World"), "{}", row.summary);
    }

    /// A hand-declared row is the user's to change, except for its delivery.
    #[test]
    fn a_declared_row_offers_every_axis_except_the_one_add_time_fixed() {
        let section = parameter_section(
            ParameterSectionContext {
                declared_schema: true,
                ..context("exe")
            },
            &[declaration("output")],
            &[],
        );
        let ParameterSection::Declared { rows } = section else {
            panic!("expected the declared section");
        };
        let row = &rows[0];
        for axis in [
            "type",
            "default",
            "choices",
            "help",
            "flag",
            "required",
            "prompt",
            "secret",
            "env_source",
            KEEP_KEY,
        ] {
            assert!(row.offers(axis), "a declared row lost its editable {axis}");
        }
        let delivery = row.field("delivery").expect("no delivery text");
        assert_eq!(delivery.kind, FieldKind::ReadOnly);
        assert_eq!(
            delivery.read_only_reason,
            Some(ReadOnlyReason::FixedAtAddTime)
        );
    }

    /// A command template takes no flag, so the row never offers one.
    #[test]
    fn only_a_binary_kind_offers_a_flag() {
        let command = parameter_section(
            ParameterSectionContext {
                declared_schema: true,
                ..context("command")
            },
            &[declaration("name")],
            &[],
        );
        let ParameterSection::Declared { rows } = command else {
            panic!("expected the declared section");
        };
        assert!(!rows[0].offers("flag"));
        assert!(rows[0].field("flag").is_none());
    }

    /// A linked source is maintained in the file, so nothing here is editable.
    #[test]
    fn a_reference_entry_is_readable_and_nothing_else() {
        let section = parameter_section(
            ParameterSectionContext {
                reference_mode: true,
                ..context("shell")
            },
            &[declaration("NAME")],
            &["OTHER".to_owned()],
        );
        assert_eq!(
            section,
            ParameterSection::Reference {
                rows: vec![ReadOnlyParameterRow {
                    name: "NAME".to_owned(),
                    parameter_type: "str".to_owned(),
                }]
            }
        );
    }

    /// A kind with no analyzer says so in one line instead of drawing an empty box.
    #[test]
    fn a_kind_without_an_analyzer_has_no_parameter_editor() {
        let section = parameter_section(
            ParameterSectionContext {
                has_analyzer: false,
                ..context("exe")
            },
            &[],
            &[],
        );
        assert_eq!(section, ParameterSection::Unsupported);
    }

    /// Marking a public row secret drops the cached literal instead of storing it.
    #[test]
    fn marking_a_row_secret_drops_the_cached_default_and_keeps_env_only_while_secret() {
        let mut row = declaration("TOKEN");
        row.secret = true;
        row.env_source = "TOKEN_ENV".to_owned();
        apply_secrecy_rule(&mut row, false);
        assert!(row.default.is_none(), "a source literal reached the block");
        assert_eq!(row.env_source, "TOKEN_ENV");

        // Turning secrecy back off drops the environment source with it.
        row.secret = false;
        apply_secrecy_rule(&mut row, true);
        assert!(row.env_source.is_empty());

        // A row that was already secret keeps whatever default it had.
        let mut unchanged = declaration("TOKEN");
        unchanged.secret = true;
        apply_secrecy_rule(&mut unchanged, true);
        assert!(unchanged.default.is_some());
    }

    /// Every row keeps its own baseline, so a save compares against what the screen opened with.
    #[test]
    fn every_row_field_carries_the_baseline_the_section_opened_with() {
        let section = parameter_section(context("python"), &[declaration("GREETING")], &[]);
        let ParameterSection::SourceManaged { mut rows, .. } = section else {
            panic!("expected the source-managed section");
        };
        let row = &mut rows[0];
        assert!(row.fields.iter().all(|field| !field.is_dirty()));

        let prompt = row
            .fields
            .iter_mut()
            .find(|field| field.key.ends_with(":prompt"))
            .expect("no form label field");
        prompt.set_value(FieldValue::text("Who to greet"));
        assert!(prompt.is_dirty());
        assert_eq!(prompt.baseline(), &FieldValue::text(""));
    }

    #[test]
    fn a_choice_type_is_recognized_for_its_option_list() {
        assert!(is_choice(ParameterType::Choice));
        assert!(!is_choice(ParameterType::Str));
        assert_eq!(ParameterDelivery::Flag.as_str(), "flag");
    }
}
