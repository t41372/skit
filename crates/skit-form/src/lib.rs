//! Compose one parameter schema for all skit frontends.

#![forbid(unsafe_code)]

pub mod field;
pub mod parameter_section;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use skit_application::library_detail::{LibraryFormFacts, LibraryFormProjector};
use skit_domain::{
    Entry, EntrySettings,
    parameters::{
        ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
        synthesized_placeholder,
    },
};
use skit_language::{
    BindingIdentity, CliSurface, DegradationReason, LosslessSource, ParseOutcome, ReconcileReport,
    SourceSpan, managed_params, parse_document, placeholder_params,
};

/// Parser-backed form adapter for one Library entry snapshot.
#[derive(Clone, Copy, Debug, Default)]
pub struct FormLibraryProjector;

impl LibraryFormProjector for FormLibraryProjector {
    fn project(&self, entry: &Entry, source: Option<&[u8]>) -> LibraryFormFacts {
        let kind = entry.meta.kind.as_str();
        let source = match source {
            Some(bytes) if kind == "prompt" => {
                String::from_utf8(bytes.to_vec()).unwrap_or_default()
            }
            Some(bytes) => LosslessSource::from_bytes(bytes)
                .normalized_text()
                .to_owned(),
            None => String::new(),
        };
        let plan = form_plan(kind, &source, &EntrySettings::from_meta(&entry.meta));
        LibraryFormFacts {
            declarations: plan.declarations(),
            drifted: !plan.drift.is_empty(),
        }
    }
}

/// The source that owns a prepared form.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FormSource {
    /// No source exposes fields.
    #[default]
    None,
    /// Managed declarations are anchored in source and delivered by injection or environment.
    Inject,
    /// A static language-owned CLI reader exposes the fields.
    Reader,
    /// A command or prompt template owns the fields.
    Command,
    /// Metadata-only flag and environment declarations own the fields.
    Declared,
}

impl FormSource {
    /// Return the stable machine spelling used by `show --json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Inject => "inject",
            Self::Reader => "argparse",
            Self::Command => "command",
            Self::Declared => "declared",
        }
    }
}

/// One source reconciliation fact that a frontend can explain without parsing source syntax.
#[derive(Clone, Debug, PartialEq)]
pub enum FormDrift {
    /// A stored source binding has no current target.
    Missing {
        /// Stored declaration that is not usable for this run.
        declaration: ParamDecl,
    },
    /// A current source target has a different scalar type.
    TypeChanged {
        /// Stored declaration that remains usable until the user resynchronizes it.
        stored: ParamDecl,
        /// Current source declaration used only to explain the change.
        current: ParamDecl,
    },
    /// An interactive input moved and only a positional fallback could match it.
    Rebound {
        /// Stored declaration that remains usable with a warning.
        stored: ParamDecl,
        /// Current source declaration selected by the positional fallback.
        current: ParamDecl,
    },
    /// Managed prompt placeholders that no longer occur in the body.
    PromptMissing {
        /// Missing placeholder names in managed order.
        names: Vec<String>,
    },
}

/// One field after live source semantics have been reconciled.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedField {
    /// Effective declaration. Its default is refreshed from the current source when sound.
    pub declaration: ParamDecl,
    /// Whether an empty value leaves an interactive source prompt active.
    pub input_binding: bool,
    /// Whether an empty environment value activates the source fallback.
    pub empty_uses_default: bool,
}

impl PreparedField {
    fn from_declaration(declaration: ParamDecl) -> Self {
        Self {
            input_binding: declaration.binding == ParameterBinding::Input,
            declaration,
            empty_uses_default: false,
        }
    }

    /// Return whether clearing this field delivers an explicit empty string.
    #[must_use]
    pub fn delivers_empty(&self) -> bool {
        self.declaration.default.is_some()
            && !self.declaration.secret
            && !self.declaration.degraded
            && !self.declaration.multiple
            && !self.input_binding
            && !self.empty_uses_default
            && matches!(
                self.declaration.parameter_type,
                ParameterType::Str | ParameterType::Path
            )
            && matches!(
                self.declaration.delivery,
                ParameterDelivery::Inject | ParameterDelivery::Flag | ParameterDelivery::Env
            )
    }
}

/// One complete frontend-neutral form projection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormPlan {
    /// Source that owns the form.
    pub source: FormSource,
    /// Effective fields in runtime order.
    pub fields: Vec<PreparedField>,
    /// Source reconciliation facts in display order.
    pub drift: Vec<FormDrift>,
    /// Whole-surface degradation when a language CLI cannot be represented statically.
    pub degradation: Option<DegradationReason>,
    /// Whether the parsed source reads its own location.
    pub uses_self_location: bool,
    /// Whether a parsed constant would require a rewritten temporary copy.
    pub has_injectable_const: bool,
}

impl FormPlan {
    /// Return effective declarations without exposing source parsing to a frontend.
    #[must_use]
    pub fn declarations(&self) -> Vec<ParamDecl> {
        self.fields
            .iter()
            .map(|field| field.declaration.clone())
            .collect()
    }
}

/// The form projection of one language-owned CLI surface.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliFormProjection {
    /// The language adapter found no CLI surface.
    #[default]
    Absent,
    /// The adapter produced a complete static field list. The list can be empty.
    Static {
        /// Framework adapter that reflected the surface.
        framework: String,
        /// Frontend-neutral fields in runtime order.
        fields: Vec<ParamDecl>,
    },
    /// A CLI surface exists, but one static form cannot represent it.
    Dynamic {
        /// Framework adapter that detected the surface.
        framework: String,
        /// Typed reason for whole-surface degradation.
        reason: DegradationReason,
    },
}

/// Parser availability for one onboarding analysis.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingParseState {
    /// One complete syntax tree produced all semantic projections.
    Parsed,
    /// The parser found invalid source syntax.
    SyntaxError {
        /// One-based line of the first syntax error when available.
        line: Option<usize>,
        /// One-based column of the first syntax error when available.
        column: Option<usize>,
    },
    /// This entry kind has no parser-backed onboarding adapter.
    #[default]
    ParserUnavailable,
}

/// One source-bound value that onboarding can offer without losing parser provenance.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OnboardingCandidate {
    /// Frontend-neutral declaration proposed by the language adapter.
    pub declaration: ParamDecl,
    /// Structural identity used by reconcile and later source edits.
    pub identity: BindingIdentity,
    /// Exact range in the parsed UTF-8 source.
    pub span: SourceSpan,
    /// Reason this candidate should start unselected.
    pub demotion: Option<DegradationReason>,
    /// Whether an empty environment value activates this source default.
    pub empty_uses_default: bool,
}

impl OnboardingCandidate {
    /// Return whether the candidate should start selected in an interactive review.
    #[must_use]
    pub const fn selected_by_default(&self) -> bool {
        self.demotion.is_none()
    }
}

/// One statically reflected CLI field with parser provenance intact.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OnboardingCliField {
    /// Frontend-neutral CLI declaration.
    pub declaration: ParamDecl,
    /// Structural identity used by future source-aware edits.
    pub identity: BindingIdentity,
    /// Exact declaration range in the parsed UTF-8 source.
    pub span: SourceSpan,
    /// Typed reason this individual field needs a degraded control.
    pub degradation: Option<DegradationReason>,
}

/// Complete parser-backed facts for add-time parameter onboarding.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct OnboardingPlan {
    /// Whether one complete parse session was available.
    pub parse_state: OnboardingParseState,
    /// Source-bound candidates in runtime review order.
    pub candidates: Vec<OnboardingCandidate>,
    /// Imported CLI frameworks in source order.
    pub frameworks: Vec<String>,
    /// Whether the source reads its raw argument vector.
    pub uses_argv: bool,
    /// Filename-shaped literal call arguments in source order.
    pub filename_literals: Vec<String>,
    /// Whether the source reads its own location.
    pub uses_self_location: bool,
    /// Complete language-owned CLI state. Static zero-field surfaces stay static.
    pub cli_surface: CliFormProjection,
    /// Parser provenance for fields in a static CLI surface.
    pub cli_fields: Vec<OnboardingCliField>,
}

impl OnboardingPlan {
    /// Return whether the analyzer detected a CLI framework.
    #[must_use]
    pub fn uses_cli_framework(&self) -> bool {
        !self.frameworks.is_empty()
    }

    /// Return a modeled static CLI field list, including a valid empty list.
    #[must_use]
    pub fn modeled_cli_fields(&self) -> Option<&[ParamDecl]> {
        match &self.cli_surface {
            CliFormProjection::Static { fields, .. } => Some(fields),
            CliFormProjection::Absent | CliFormProjection::Dynamic { .. } => None,
        }
    }

    /// Return candidates that onboarding can offer without replacing a modeled form.
    ///
    /// A static reader with one or more fields is already the program interface. Dynamic,
    /// absent, and valid zero-field surfaces keep the source candidate offer available.
    #[must_use]
    pub fn offered_candidates(&self) -> &[OnboardingCandidate] {
        match &self.cli_surface {
            CliFormProjection::Static { fields, .. } if !fields.is_empty() => &[],
            CliFormProjection::Absent
            | CliFormProjection::Static { .. }
            | CliFormProjection::Dynamic { .. } => &self.candidates,
        }
    }
}

/// Build every add-time semantic fact from one parser-owned source document.
#[must_use]
pub fn onboarding_plan(kind: &str, text: &str) -> OnboardingPlan {
    match parse_document(kind, text) {
        ParseOutcome::Parsed(document) => {
            let analysis = document.analysis();
            let semantic_cli_surface = document.cli_surface();
            let cli_fields = match &semantic_cli_surface {
                CliSurface::Static(surface) => surface
                    .fields
                    .iter()
                    .map(|field| OnboardingCliField {
                        declaration: field.declaration.clone(),
                        identity: field.identity.clone(),
                        span: field.span,
                        degradation: field.degradation,
                    })
                    .collect(),
                CliSurface::Absent | CliSurface::Dynamic(_) => Vec::new(),
            };
            let cli_surface = project_cli_surface(semantic_cli_surface);
            OnboardingPlan {
                parse_state: OnboardingParseState::Parsed,
                candidates: analysis
                    .candidates
                    .into_iter()
                    .map(|candidate| OnboardingCandidate {
                        declaration: candidate.declaration,
                        identity: candidate.identity,
                        span: candidate.span,
                        demotion: candidate.demotion,
                        empty_uses_default: candidate.empty_uses_default,
                    })
                    .collect(),
                frameworks: analysis.frameworks,
                uses_argv: analysis.uses_argv,
                filename_literals: analysis.filename_literals,
                uses_self_location: analysis.uses_self_location,
                cli_surface,
                cli_fields,
            }
        }
        ParseOutcome::SyntaxError(failure) => OnboardingPlan {
            parse_state: OnboardingParseState::SyntaxError {
                line: failure.line,
                column: failure.column,
            },
            ..OnboardingPlan::default()
        },
        ParseOutcome::ParserUnavailable(_) => OnboardingPlan::default(),
    }
}

fn project_cli_surface(surface: CliSurface) -> CliFormProjection {
    match surface {
        CliSurface::Absent => CliFormProjection::Absent,
        CliSurface::Static(surface) => CliFormProjection::Static {
            framework: surface.framework,
            fields: surface
                .fields
                .into_iter()
                .map(|field| field.declaration)
                .collect(),
        },
        CliSurface::Dynamic(surface) => CliFormProjection::Dynamic {
            framework: surface.framework,
            reason: surface.reason,
        },
    }
}

/// Project a language CLI surface without collapsing absent, empty, and dynamic states.
#[must_use]
pub fn cli_form_projection(kind: &str, text: &str) -> CliFormProjection {
    cli_form_facts(kind, text).0
}

fn cli_form_facts(kind: &str, text: &str) -> (CliFormProjection, bool, bool) {
    let ParseOutcome::Parsed(document) = parse_document(kind, text) else {
        return (CliFormProjection::Absent, false, false);
    };
    let analysis = document.analysis();
    let uses_self_location = analysis.uses_self_location;
    let has_injectable_const = analysis.candidates.iter().any(|candidate| {
        candidate.declaration.binding == ParameterBinding::Const
            && candidate.declaration.delivery == ParameterDelivery::Inject
    });
    (
        project_cli_surface(document.cli_surface()),
        uses_self_location,
        has_injectable_const,
    )
}

/// Build the fields that one entry exposes to all frontends.
///
/// Managed source fields take priority over static CLI fields.
/// Metadata flag and environment fields can extend either source form.
/// Command and prompt entries use the managed names stored in `params`.
#[must_use]
pub fn form_params(kind: &str, text: &str, settings: &EntrySettings) -> Vec<ParamDecl> {
    form_plan(kind, text, settings).declarations()
}

/// Build one reconciled form plan for every frontend.
///
/// Managed declarations win over reflected CLI fields. A live source reconciliation removes
/// missing bindings, keeps changed or positionally rebound bindings with explicit drift facts,
/// and refreshes safe defaults from the source. Declared flag and environment fields then extend
/// the selected source form without replacing it.
#[must_use]
pub fn form_plan(kind: &str, text: &str, settings: &EntrySettings) -> FormPlan {
    if kind == "prompt" && !settings.interpolate {
        return FormPlan {
            source: FormSource::Command,
            ..FormPlan::default()
        };
    }
    if kind == "prompt" {
        let fresh = placeholder_params(kind, text)
            .into_iter()
            .map(|field| field.name)
            .collect::<BTreeSet<_>>();
        let gone = settings
            .params
            .iter()
            .filter(|name| !fresh.contains(*name))
            .cloned()
            .collect::<Vec<_>>();
        return FormPlan {
            source: FormSource::Command,
            fields: prepared(template_params(&settings.params, &settings.parameters)),
            drift: if gone.is_empty() {
                Vec::new()
            } else {
                vec![FormDrift::PromptMissing { names: gone }]
            },
            degradation: None,
            uses_self_location: false,
            has_injectable_const: false,
        };
    }
    if kind == "command" {
        return FormPlan {
            source: FormSource::Command,
            fields: prepared(template_params(&settings.params, &settings.parameters)),
            ..FormPlan::default()
        };
    }

    let managed = managed_params(kind, text);
    if !managed.is_empty() {
        let mut plan = managed_form_plan(kind, text, &managed);
        append_riders(&mut plan.fields, &settings.parameters);
        return plan;
    }

    // PowerShell is the v0.4 reader-only kind. Its param() block owns the form when present, and
    // declared flag/environment rows extend that static surface instead of replacing it.
    if kind == "powershell" {
        return reader_only_form_plan(kind, text, &settings.parameters);
    }

    let riders = declared_riders(&settings.parameters, &BTreeSet::new());
    if !riders.is_empty() {
        let (_, uses_self_location, has_injectable_const) = cli_form_facts(kind, text);
        return FormPlan {
            source: FormSource::Declared,
            fields: prepared(riders),
            uses_self_location,
            has_injectable_const,
            ..FormPlan::default()
        };
    }

    let (cli_surface, uses_self_location, has_injectable_const) = cli_form_facts(kind, text);
    match cli_surface {
        CliFormProjection::Static { fields, .. } => FormPlan {
            source: FormSource::Reader,
            fields: prepared(fields),
            uses_self_location,
            has_injectable_const,
            ..FormPlan::default()
        },
        CliFormProjection::Dynamic { reason, .. } => FormPlan {
            source: FormSource::Reader,
            degradation: Some(reason),
            uses_self_location,
            has_injectable_const,
            ..FormPlan::default()
        },
        CliFormProjection::Absent => FormPlan {
            uses_self_location,
            has_injectable_const,
            ..FormPlan::default()
        },
    }
}

fn reader_only_form_plan(kind: &str, text: &str, declared: &[ParamDecl]) -> FormPlan {
    match cli_form_projection(kind, text) {
        CliFormProjection::Static { fields, .. } => {
            let mut fields = prepared(fields);
            append_riders(&mut fields, declared);
            FormPlan {
                source: FormSource::Reader,
                fields,
                ..FormPlan::default()
            }
        }
        // PowerShell is the only reader-only adapter. Its parser publishes either a complete
        // static param() surface or no surface; unlike Python's multi-command frameworks, it has
        // no whole-surface Dynamic state. Keep its non-static fallback exhaustive without an
        // executable branch no real document can produce.
        CliFormProjection::Absent | CliFormProjection::Dynamic { .. } => {
            let riders = declared_riders(declared, &BTreeSet::new());
            if riders.is_empty() {
                FormPlan::default()
            } else {
                FormPlan {
                    source: FormSource::Declared,
                    fields: prepared(riders),
                    ..FormPlan::default()
                }
            }
        }
    }
}

/// Compose fields from a managed source schema that the caller already prepared.
///
/// This keeps source mutation paths on one parse and one write while applying the same declared
/// flag and environment riders as [`form_params`].
#[must_use]
pub fn form_params_from_managed(
    managed: Vec<ParamDecl>,
    settings: &EntrySettings,
) -> Vec<ParamDecl> {
    with_riders(managed, &settings.parameters)
}

fn with_riders(mut fields: Vec<ParamDecl>, declared: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut taken = fields
        .iter()
        .map(|item| item.name.clone())
        .collect::<BTreeSet<_>>();
    for item in declared {
        if matches!(
            item.delivery,
            ParameterDelivery::Flag | ParameterDelivery::Env
        ) && taken.insert(item.name.clone())
        {
            fields.push(item.clone());
        }
    }
    fields
}

fn prepared(declarations: Vec<ParamDecl>) -> Vec<PreparedField> {
    declarations
        .into_iter()
        .map(PreparedField::from_declaration)
        .collect()
}

fn append_riders(fields: &mut Vec<PreparedField>, declared: &[ParamDecl]) {
    let taken = fields
        .iter()
        .map(|field| field.declaration.name.clone())
        .collect::<BTreeSet<_>>();
    fields.extend(prepared(declared_riders(declared, &taken)));
}

fn declared_riders(declared: &[ParamDecl], taken: &BTreeSet<String>) -> Vec<ParamDecl> {
    declared
        .iter()
        .filter(|item| {
            matches!(
                item.delivery,
                ParameterDelivery::Flag | ParameterDelivery::Env
            ) && !taken.contains(&item.name)
        })
        .cloned()
        .collect()
}

fn managed_form_plan(kind: &str, text: &str, managed: &[ParamDecl]) -> FormPlan {
    let parsed = parse_document(kind, text);
    let (uses_self_location, has_injectable_const) = match &parsed {
        ParseOutcome::Parsed(document) => {
            let analysis = document.analysis();
            (
                analysis.uses_self_location,
                analysis.candidates.iter().any(|candidate| {
                    candidate.declaration.binding == ParameterBinding::Const
                        && candidate.declaration.delivery == ParameterDelivery::Inject
                }),
            )
        }
        ParseOutcome::SyntaxError(_) | ParseOutcome::ParserUnavailable(_) => (false, false),
    };
    let mut report = match &parsed {
        ParseOutcome::Parsed(document) => reconciliation_from_language(document.reconcile(managed)),
        // Every kind that can carry managed metadata has a bundled parser. A parser-unavailable
        // result therefore has the same conservative all-missing projection as a syntax error.
        _ => reconciliation_from_language(ReconcileReport::from_syntax_error(managed)),
    };
    if let ParseOutcome::Parsed(document) = &parsed {
        for declaration in managed {
            if document
                .source_parameter_semantics(declaration)
                .empty_uses_default
            {
                report.empty_uses_default.insert(declaration.name.clone());
            }
        }
    }
    let mut fields = Vec::new();
    for pair in &report.ok {
        let mut declaration = pair.stored.clone();
        refresh_default(&mut declaration, &report);
        fields.push(prepared_reconciled_field(declaration, &report));
    }
    fields.extend(
        report
            .changed
            .iter()
            .map(|pair| prepared_reconciled_field(pair.stored.clone(), &report)),
    );
    fields.extend(
        report
            .rebound
            .iter()
            .map(|pair| prepared_reconciled_field(pair.stored.clone(), &report)),
    );

    let mut drift = report
        .missing
        .iter()
        .cloned()
        .map(|declaration| FormDrift::Missing { declaration })
        .collect::<Vec<_>>();
    drift.extend(report.changed.iter().map(|pair| FormDrift::TypeChanged {
        stored: pair.stored.clone(),
        current: pair.current.clone(),
    }));
    drift.extend(report.rebound.iter().map(|pair| FormDrift::Rebound {
        stored: pair.stored.clone(),
        current: pair.current.clone(),
    }));
    FormPlan {
        source: FormSource::Inject,
        fields,
        drift,
        degradation: None,
        uses_self_location,
        has_injectable_const,
    }
}

fn prepared_reconciled_field(declaration: ParamDecl, report: &FormReconciliation) -> PreparedField {
    let empty_uses_default = report.empty_uses_default.contains(&declaration.name);
    let mut field = PreparedField::from_declaration(declaration);
    field.empty_uses_default = empty_uses_default;
    field
}

fn refresh_default(declaration: &mut ParamDecl, report: &FormReconciliation) {
    if declaration.default.is_some()
        && let Some(current) = report.current_defaults.get(&declaration.name)
    {
        declaration.default = Some(current.clone());
    }
}

struct FormReconciliation {
    ok: Vec<DeclarationPair>,
    missing: Vec<ParamDecl>,
    changed: Vec<DeclarationPair>,
    rebound: Vec<DeclarationPair>,
    current_defaults: BTreeMap<String, ParameterValue>,
    empty_uses_default: BTreeSet<String>,
}

struct DeclarationPair {
    stored: ParamDecl,
    current: ParamDecl,
}

fn reconciliation_from_language(report: ReconcileReport) -> FormReconciliation {
    FormReconciliation {
        ok: report
            .ok
            .into_iter()
            .map(|pair| DeclarationPair {
                stored: pair.stored,
                current: pair.current.declaration,
            })
            .collect(),
        missing: report.missing,
        changed: report
            .changed
            .into_iter()
            .map(|pair| DeclarationPair {
                stored: pair.stored,
                current: pair.current.declaration,
            })
            .collect(),
        rebound: report
            .rebound
            .into_iter()
            .map(|pair| DeclarationPair {
                stored: pair.stored,
                current: pair.current.declaration,
            })
            .collect(),
        current_defaults: report.current_defaults,
        empty_uses_default: report.empty_uses_default,
    }
}

fn template_params(managed: &[String], declared: &[ParamDecl]) -> Vec<ParamDecl> {
    let mut unique = Vec::<ParamDecl>::new();
    let mut indices = BTreeMap::<String, usize>::new();
    for item in declared {
        if let Some(index) = indices.get(&item.name).copied() {
            unique[index] = item.clone();
        } else {
            indices.insert(item.name.clone(), unique.len());
            unique.push(item.clone());
        }
    }

    let managed_set = managed.iter().cloned().collect::<BTreeSet<_>>();
    let mut output = Vec::new();
    for name in managed {
        let item = indices
            .get(name)
            .and_then(|index| unique.get(*index))
            .filter(|item| item.delivery == ParameterDelivery::Placeholder)
            .cloned()
            .unwrap_or_else(|| synthesized_placeholder(name));
        output.push(item);
    }
    output.extend(unique.into_iter().filter(|item| {
        item.delivery == ParameterDelivery::Env && !managed_set.contains(&item.name)
    }));
    output
}
