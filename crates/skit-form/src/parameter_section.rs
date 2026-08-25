//! The Settings screen's parameter section, as a frontend-neutral plan.
//!
//! Version 0.4 decides what this section is before it decides what any row looks like
//! (`src/skit/tui_settings.py:588-631`), and the decision is the whole point: a parameter the
//! script itself declares is not offered as an editable row, because saving one could not keep the
//! edit. Modelling the decision here means no frontend can render a control the host would then
//! discard, and no host needs a submit-time filter to undo a row it should never have drawn.

use skit_domain::parameters::{ParamDecl, ParameterType};

use crate::field::{Field, FieldCapabilities, FieldKind, FieldOwner, FieldValue, TypedValue};

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
    /// Return the stable key prefix every field of this row shares.
    ///
    /// The row is keyed by the parameter's own name, never by its position. A position is a key
    /// into a set that can change: a concurrent `skit params --add` shifts every index after it, so
    /// an index would carry one row's edit onto another row's declaration. The name is the identity
    /// the store merges on, which is why version 0.4 keys its own name-addressed lists the same way
    /// (`src/skit/tui_settings.py:812-815`).
    #[must_use]
    pub fn prefix(&self) -> String {
        row_prefix(&self.name)
    }

    /// Return the field for one axis of this row.
    #[must_use]
    pub fn field(&self, axis: &str) -> Option<&Field> {
        let key = format!("{}:{axis}", self.prefix());
        self.fields.iter().find(|field| field.key == key)
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
                .map(|declaration| declared_row(declaration, context.kind))
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
    let rows = managed.iter().map(source_managed_row).collect();
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
/// Version 0.4's `ParamRow` prints the name, type and default as dim text *inside* the keep
/// toggle's own label and offers exactly three editable axes: the form label, the secret flag and
/// the environment source (`src/skit/tui_settings.py:98-107`). Type and default are the script's own
/// text, so offering them here would promise an edit that saving could not keep — and repeating them
/// as their own read-only rows would say the same thing twice on every row.
fn source_managed_row(declaration: &ParamDecl) -> ParameterRow {
    let prefix = row_prefix(&declaration.name);
    let default = declaration
        .default
        .as_ref()
        .map_or_else(String::new, render_source_default);
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
    let mut fields = vec![keep_field(&prefix, &summary, "unmanage this parameter")];
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
/// Version 0.4's `DeclParamRow` makes type, default, choices, help, required and — where argv is
/// the interface — the flag editable, and keeps delivery as dim header text inside the keep
/// toggle's own label, because a different command changes it
/// (`src/skit/tui_settings.py:151-230`, the label at `:180`).
fn declared_row(declaration: &ParamDecl, kind: &str) -> ParameterRow {
    let prefix = row_prefix(&declaration.name);
    let summary = format!("{}  {}", declaration.name, declaration.delivery.as_str());
    let mut fields = vec![
        keep_field(&prefix, &summary, "remove this parameter"),
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
                    FieldValue::text(render_editable_default(value))
                }),
        )
        .with_capabilities(FieldCapabilities {
            reset_default: true,
            ..FieldCapabilities::default()
        })
        .with_help("default value (optional)"),
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
        )
        .with_help("one-line help shown under the field (optional)"),
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
        summary,
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

/// Return the key prefix every field of one row shares.
fn row_prefix(name: &str) -> String {
    format!("parameter:{name}")
}

/// Build the keep toggle, whose label is the row's own header text.
///
/// Version 0.4 puts the name and its dim metadata inside the checkbox label rather than beside it
/// (`src/skit/tui_settings.py:101` and `:180`). The label is therefore user data — a parameter named
/// after a catalog phrase must not come back translated — so it travels verbatim.
fn keep_field(prefix: &str, summary: &str, help: &str) -> Field {
    Field::new(
        format!("{prefix}:{KEEP_KEY}"),
        summary,
        FieldKind::Boolean,
        FieldOwner::Declared,
        FieldValue::boolean(true),
    )
    .with_verbatim_label()
    .with_help(help)
}

/// Report whether a kind's launch takes an argument vector, so a flag means anything.
///
/// Version 0.4 gates this on the `placeholder_params` trait, not on the family: "a flag only means
/// something where argv is the interface: every kind whose form is NOT placeholders (binaries AND
/// the interpreted meta-schema kinds)" (`src/skit/tui_settings.py:653-657`, and the trait itself at
/// `src/skit/langs/registry.py:271` and `:292`). A command template and a prompt name their fields
/// with placeholders, so every other declared kind — the binary and the interpreted kinds that keep
/// a hand-written schema — offers the flag.
const fn shows_flag(kind: &str) -> bool {
    !matches!(kind.as_bytes(), b"command" | b"prompt")
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
fn render_editable_default(value: &skit_domain::parameters::ParameterValue) -> String {
    use skit_domain::parameters::ParameterValue;
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => value.to_string(),
        ParameterValue::Bool(value) => if *value { "true" } else { "false" }.to_owned(),
    }
}

/// Render one source-owned default as the read-only representation version 0.4 shows.
fn render_source_default(value: &skit_domain::parameters::ParameterValue) -> String {
    use skit_domain::parameters::ParameterValue;
    match value {
        ParameterValue::String(value) => python_string_repr(value),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => format!("{value:?}"),
        ParameterValue::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
    }
}

fn python_string_repr(value: &str) -> String {
    let quote = if value.contains('\'') && !value.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut rendered = String::with_capacity(value.len().saturating_add(2));
    rendered.push(quote);
    for character in value.chars() {
        match character {
            '\\' => rendered.push_str("\\\\"),
            '\n' => rendered.push_str("\\n"),
            '\r' => rendered.push_str("\\r"),
            '\t' => rendered.push_str("\\t"),
            character if character == quote => {
                rendered.push('\\');
                rendered.push(character);
            }
            character if character.is_control() => {
                // Rust's control characters are the C0/C1 sets (U+0000..=U+001F and
                // U+007F..=U+009F), so Python repr always uses its two-digit byte escape here.
                let codepoint = u32::from(character);
                rendered.push_str(&format!("\\x{codepoint:02x}"));
            }
            character => rendered.push(character),
        }
    }
    rendered.push(quote);
    rendered
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
        assert!(matches!(
            section,
            ParameterSection::SourceManaged { rows, followup }
                if rows.is_empty() && followup == SourceFollowup::ReaderDriven
        ));
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
        assert!(matches!(
            section,
            ParameterSection::SourceManaged { rows, followup }
                if rows.len() == 1
                    && followup == SourceFollowup::Offer {
                        candidates: vec!["WIDTH".to_owned()]
                    }
        ));
    }

    /// A block-managed row offers three axes and no more, and reads out the rest in its own label.
    ///
    /// Name, type and default are the script's own text and sit inside the keep toggle's label
    /// (`src/skit/tui_settings.py:99-101`). A person reads them there; no control promises an edit
    /// that saving could not keep.
    #[test]
    fn a_source_managed_row_offers_only_the_axes_a_save_can_keep() {
        let section = parameter_section(context("python"), &[declaration("GREETING")], &[]);
        assert!(matches!(
            section,
            ParameterSection::SourceManaged { rows, .. } if {
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
                        row.field(axis).is_none(),
                        "a source-managed row drew a {axis} control, which saving cannot keep"
                    );
                }
                // The type and the default reach the person through the toggle's own label, which
                // is user data and must never be translated.
                let keep = row.field(KEEP_KEY).expect("no keep toggle");
                assert_eq!(keep.label, row.summary);
                assert!(!keep.translate_label, "a parameter name is not catalog copy");
                assert!(keep.label.contains("GREETING"), "{}", keep.label);
                assert!(keep.label.contains("str"), "{}", keep.label);
                assert!(keep.label.contains("World"), "{}", keep.label);
                true
            }
        ));
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
        assert!(matches!(
            section,
            ParameterSection::Declared { rows } if {
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
                // Delivery is fixed at add time, so it is header text and not a control.
                assert!(row.field("delivery").is_none());
                let keep = row.field(KEEP_KEY).expect("no keep toggle");
                assert_eq!(keep.label, "output  flag");
                assert!(!keep.translate_label);
                true
            }
        ));
    }

    /// Every field of one row is keyed by the parameter's own name, never by its position.
    ///
    /// A position is a key into a set a concurrent `skit params --add` can shift, which would carry
    /// one row's edit onto another row's declaration.
    #[test]
    fn a_row_is_keyed_by_its_name_so_a_shifted_set_cannot_move_an_edit() {
        let section = parameter_section(
            ParameterSectionContext {
                declared_schema: true,
                ..context("exe")
            },
            &[declaration("first"), declaration("second")],
            &[],
        );
        assert!(matches!(
            section,
            ParameterSection::Declared { rows } if {
                assert_eq!(rows[1].prefix(), "parameter:second");
                assert_eq!(
                    rows[1].field("prompt").unwrap().key,
                    "parameter:second:prompt"
                );
                true
            }
        ));
        // Inserting a row ahead of it leaves every key of the surviving row untouched.
        let shifted = parameter_section(
            ParameterSectionContext {
                declared_schema: true,
                ..context("exe")
            },
            &[
                declaration("zeroth"),
                declaration("first"),
                declaration("second"),
            ],
            &[],
        );
        assert!(matches!(
            shifted,
            ParameterSection::Declared { rows } if {
                assert_eq!(
                    rows[2].field("prompt").unwrap().key,
                    "parameter:second:prompt"
                );
                true
            }
        ));
    }

    /// A flag only means something where the argument vector is the interface.
    ///
    /// Version 0.4 gates it on the `placeholder_params` trait (`src/skit/tui_settings.py:653-657`),
    /// so a command template and a prompt have none while every other declared kind does.
    #[test]
    fn only_a_kind_whose_form_is_not_placeholders_offers_a_flag() {
        let section_for = |kind: &'static str| {
            parameter_section(
                ParameterSectionContext {
                    declared_schema: true,
                    ..context(kind)
                },
                &[declaration("name")],
                &[],
            )
        };
        for kind in ["command", "prompt"] {
            assert!(
                matches!(
                    section_for(kind),
                    ParameterSection::Declared { rows } if {
                        let row = &rows[0];
                        !row.offers("flag") && row.field("flag").is_none()
                    }
                ),
                "{kind} drew a meaningless flag"
            );
        }
        // A binary and the interpreted kinds that keep a hand-written schema all take argv.
        for kind in ["exe", "powershell", "ruby", "perl", "lua", "r"] {
            assert!(
                matches!(
                    section_for(kind),
                    ParameterSection::Declared { rows } if rows[0].offers("flag")
                ),
                "{kind} lost its flag"
            );
        }
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
        let mut row = source_managed_row(&declaration("GREETING"));
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
