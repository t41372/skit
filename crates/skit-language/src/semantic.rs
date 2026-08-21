//! Parser-owned semantic documents and source edit plans.
//!
//! A document owns one syntax tree. Analysis, CLI reflection, reconciliation, and
//! injection planning all read that same tree. The runtime crate never receives the
//! tree or a parser type.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue, coerce_default,
    is_secret_name,
};

use crate::LanguageError;

mod fish;
mod javascript;
mod powershell;
mod shell;

/// A parser failure with a stable source location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseFailure {
    /// Requested entry kind.
    pub kind: String,
    /// One-based line of the first syntax error when available.
    pub line: Option<usize>,
    /// One-based column of the first syntax error when available.
    pub column: Option<usize>,
}

/// The result of creating one owning parser document.
#[derive(Debug)]
pub enum ParseOutcome {
    /// The source has a complete syntax tree.
    Parsed(ParsedDocument),
    /// The selected parser found invalid syntax.
    SyntaxError(ParseFailure),
    /// This entry kind has no parser adapter.
    ParserUnavailable(ParseFailure),
}

/// A byte and line range in the original source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SourceSpan {
    /// Inclusive UTF-8 byte offset.
    pub start: usize,
    /// Exclusive UTF-8 byte offset.
    pub end: usize,
    /// One-based start line.
    pub start_line: usize,
    /// One-based end line.
    pub end_line: usize,
}

impl SourceSpan {
    fn from_node(node: tree_sitter::Node<'_>) -> Self {
        Self {
            start: node.start_byte(),
            end: node.end_byte(),
            start_line: node.start_position().row.saturating_add(1),
            end_line: node.end_position().row.saturating_add(1),
        }
    }

    fn insertion(offset: usize, line: usize) -> Self {
        Self {
            start: offset,
            end: offset,
            start_line: line,
            end_line: line,
        }
    }
}

/// A stable semantic identity independent of a generated form label.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BindingIdentity {
    /// Source-binding category.
    pub binding: ParameterBinding,
    /// Stable key: a constant name, prompt, or reflected destination.
    pub key: String,
    /// Source-order occurrence within equal keys.
    pub occurrence: usize,
    /// Lexical scope path for diagnostics and future structural matching.
    pub scope: Vec<String>,
}

/// Why a static semantic projection is incomplete.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationReason {
    /// The program declares subcommands or more than one command.
    Subcommands,
    /// A declaration is generated in a loop.
    DynamicDeclaration,
    /// A value is used as an accumulator.
    Accumulator,
    /// The parser action has no lossless form projection.
    UnsupportedAction,
    /// A choices expression is not a literal sequence.
    DynamicChoices,
    /// A conversion callable is not statically representable.
    DynamicType,
    /// A default expression cannot be evaluated statically.
    DynamicDefault,
    /// A type annotation has no lossless form projection.
    UnsupportedAnnotation,
    /// A Boolean flag needs a paired negative spelling.
    BooleanDefaultTrue,
}

/// One analyzer candidate with its source identity and range.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticCandidate {
    /// Frontend-neutral declaration.
    pub declaration: ParamDecl,
    /// Structural binding identity.
    pub identity: BindingIdentity,
    /// Original source range.
    pub span: SourceSpan,
    /// Optional reason the candidate must not be selected automatically.
    pub demotion: Option<DegradationReason>,
    /// Whether an empty environment value activates this source default.
    pub empty_uses_default: bool,
}

/// Live source semantics that are not stored in a parameter declaration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceParameterSemantics {
    /// Whether an empty environment value activates the source default.
    pub empty_uses_default: bool,
}

/// Parser-backed language analysis.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SemanticAnalysis {
    /// Source-bound parameter candidates.
    pub candidates: Vec<SemanticCandidate>,
    /// Imported CLI frameworks in source order.
    pub frameworks: Vec<String>,
    /// Whether the source reads `sys.argv`.
    pub uses_argv: bool,
    /// Literal filename arguments used as onboarding hints.
    pub filename_literals: Vec<String>,
    /// Whether the source reads its own location.
    pub uses_self_location: bool,
}

/// One reflected CLI field with structural provenance.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticField {
    /// Frontend-neutral field declaration.
    pub declaration: ParamDecl,
    /// Structural declaration identity.
    pub identity: BindingIdentity,
    /// Original declaration range.
    pub span: SourceSpan,
    /// Typed reason for a lossy projection.
    pub degradation: Option<DegradationReason>,
}

/// A fully static CLI surface, including a valid zero-field surface.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticCliSurface {
    /// Framework adapter that produced this surface.
    pub framework: String,
    /// Fields in the framework's runtime order.
    pub fields: Vec<SemanticField>,
}

/// A detected CLI surface that cannot be represented as one static form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicCliSurface {
    /// Framework adapter that detected the surface.
    pub framework: String,
    /// Typed reason static projection is not sound.
    pub reason: DegradationReason,
}

/// The semantic state of a language-owned CLI surface.
#[derive(Clone, Debug, PartialEq)]
pub enum CliSurface {
    /// The source has no detected CLI framework surface.
    Absent,
    /// The complete surface is static. Its field list can be empty.
    Static(StaticCliSurface),
    /// A surface exists, but its shape is dynamic.
    Dynamic(DynamicCliSurface),
}

/// One stored declaration and its current source candidate.
#[derive(Clone, Debug, PartialEq)]
pub struct ReconcilePair {
    /// Stored declaration.
    pub stored: ParamDecl,
    /// Current semantic candidate.
    pub current: SemanticCandidate,
}

/// Reconciliation between stored declarations and one parsed source version.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReconcileReport {
    /// Declarations whose identities and types still agree.
    pub ok: Vec<ReconcilePair>,
    /// Stored declarations with no current source binding.
    pub missing: Vec<ParamDecl>,
    /// Declarations whose current type differs.
    pub changed: Vec<ReconcilePair>,
    /// Input declarations that needed an ambiguous positional fallback.
    pub rebound: Vec<ReconcilePair>,
    /// Current candidates that are not managed.
    pub new: Vec<SemanticCandidate>,
    /// Current source defaults for unchanged declarations.
    pub current_defaults: BTreeMap<String, ParameterValue>,
    /// Matched defaults for which an empty environment value activates the fallback.
    pub empty_uses_default: BTreeSet<String>,
    /// Whether syntax failure prevented source reconciliation.
    pub syntax_error: bool,
}

impl ReconcileReport {
    /// Build the conservative result for source that does not parse.
    #[must_use]
    pub fn from_syntax_error(stored: &[ParamDecl]) -> Self {
        Self {
            missing: stored.to_vec(),
            syntax_error: true,
            ..Self::default()
        }
    }

    /// Return stored declarations that remain safe to deliver.
    #[must_use]
    pub fn usable(&self) -> Vec<&ParamDecl> {
        self.ok
            .iter()
            .chain(&self.changed)
            .chain(&self.rebound)
            .map(|pair| &pair.stored)
            .collect()
    }

    /// Return whether a stored source binding has drifted.
    #[must_use]
    pub fn has_drift(&self) -> bool {
        !self.missing.is_empty() || !self.changed.is_empty() || !self.rebound.is_empty()
    }
}

/// One non-overlapping replacement against the parsed source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEdit {
    /// Original source range.
    pub span: SourceSpan,
    /// Replacement source bytes encoded as UTF-8.
    pub replacement: String,
}

/// An identity-checked set of source edits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceEditPlan {
    source: String,
    edits: Vec<SourceEdit>,
}

impl SourceEditPlan {
    /// Return the immutable non-overlapping edit sequence.
    #[must_use]
    pub fn edits(&self) -> &[SourceEdit] {
        &self.edits
    }

    /// Apply this plan only to the exact source version that produced it.
    pub fn apply(&self, source: &str) -> Result<String, LanguageError> {
        if source != self.source {
            return Err(LanguageError::SourceChanged);
        }
        let edits = self
            .edits
            .iter()
            .map(|edit| (edit.span.start, edit.span.end, edit.replacement.clone()))
            .collect();
        super::apply_source_edits(source, edits)
    }
}

/// One parser-owned source document.
#[derive(Debug)]
pub struct ParsedDocument {
    kind: String,
    source: String,
    tree: tree_sitter::Tree,
}

impl ParsedDocument {
    /// Return the exact parsed UTF-8 source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    pub(crate) fn syntax_tree(&self) -> &tree_sitter::Tree {
        &self.tree
    }

    pub(crate) fn python_module_description(&self) -> Option<String> {
        if self.kind != "python" {
            return None;
        }
        let statement = named_children(self.tree.root_node())
            .into_iter()
            .find(|node| node.kind() != "comment")?;
        if statement.kind() != "expression_statement" {
            return None;
        }
        let expression = named_children(statement).into_iter().next()?;
        let PythonLiteral::Value(ParameterValue::String(docstring)) =
            python_literal(self, expression)?
        else {
            return None;
        };
        docstring
            .trim()
            .lines()
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
    }

    /// Return parser-backed language analysis from this document's tree.
    #[must_use]
    pub fn analysis(&self) -> SemanticAnalysis {
        match self.kind.as_str() {
            "python" => python_analysis(self),
            "shell" => shell::analysis(self),
            "js" | "ts" | "tsx" => javascript::analysis(self),
            "fish" => fish::analysis(self),
            "powershell" => powershell::analysis(self),
            _ => SemanticAnalysis::default(),
        }
    }

    /// Return the first detected CLI framework surface.
    #[must_use]
    pub fn cli_surface(&self) -> CliSurface {
        match self.kind.as_str() {
            "python" => python_cli_surface(self),
            "shell" => shell::cli_surface(self),
            "js" | "ts" | "tsx" => javascript::cli_surface(self),
            "fish" => fish::cli_surface(self),
            "powershell" => powershell::cli_surface(self),
            _ => CliSurface::Absent,
        }
    }

    /// Reconcile stored source declarations against this source version.
    #[must_use]
    pub fn reconcile(&self, stored: &[ParamDecl]) -> ReconcileReport {
        reconcile_analysis(&self.analysis(), stored)
    }

    /// Return live source semantics for one declaration from this parse session.
    #[must_use]
    pub fn source_parameter_semantics(&self, declaration: &ParamDecl) -> SourceParameterSemantics {
        match self.kind.as_str() {
            "shell" => shell_parameter_semantics(self, declaration),
            _ => SourceParameterSemantics::default(),
        }
    }

    /// Plan binding injection against this exact source version.
    pub fn plan_injection(
        &self,
        declarations: &[ParamDecl],
        values: &BTreeMap<String, String>,
    ) -> Result<SourceEditPlan, LanguageError> {
        self.plan_injection_for_interpreter(declarations, values, None)
    }

    /// Plan binding injection with the resolved shell interpreter when one is available.
    pub fn plan_injection_for_interpreter(
        &self,
        declarations: &[ParamDecl],
        values: &BTreeMap<String, String>,
        interpreter: Option<&str>,
    ) -> Result<SourceEditPlan, LanguageError> {
        match self.kind.as_str() {
            "python" => plan_python_injection(self, declarations, values),
            "shell" => shell::plan_injection(self, declarations, values, interpreter),
            "js" | "ts" | "tsx" => javascript::plan_injection(self, declarations, values),
            kind => Err(LanguageError::UnsupportedKind {
                kind: kind.to_owned(),
            }),
        }
    }

    /// Plan one opt-in shell environment-default normalization.
    pub fn plan_shell_normalization(&self, name: &str) -> Result<SourceEditPlan, LanguageError> {
        if self.kind != "shell" {
            return Err(LanguageError::UnsupportedKind {
                kind: self.kind.clone(),
            });
        }
        shell::normalize(self, name)
    }
}

/// Parse source into an owning document without exposing the parser to launch code.
#[must_use]
pub fn parse_document(kind: &str, source: &str) -> ParseOutcome {
    let mut parser = tree_sitter::Parser::new();
    let parser_result = match kind {
        "python" => parser.set_language(&tree_sitter_python::LANGUAGE.into()),
        "shell" => parser.set_language(&tree_sitter_bash::LANGUAGE.into()),
        "js" => parser.set_language(&tree_sitter_javascript::LANGUAGE.into()),
        "ts" => parser.set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        // The TSX dialect: the TypeScript grammar cannot parse JSX, so tsx needs its own grammar.
        // The oracle's JS analyzer wires the same tsx grammar (langs/javascript/analyzer.py
        // `_LANGUAGES["tsx"]`); the js-family analysis, surface, and injection are shared.
        "tsx" => parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()),
        "fish" => parser.set_language(&tree_sitter_fish::language()),
        "powershell" => parser.set_language(&tree_sitter_powershell::LANGUAGE.into()),
        _ => {
            return ParseOutcome::ParserUnavailable(ParseFailure {
                kind: kind.to_owned(),
                line: None,
                column: None,
            });
        }
    };
    if parser_result.is_err() {
        return ParseOutcome::ParserUnavailable(ParseFailure {
            kind: kind.to_owned(),
            line: None,
            column: None,
        });
    }
    // This grammar requires a statement after a valid script-level param block. Add an analyzer-
    // only empty statement so the parser keeps the complete parameter list. All semantic spans
    // stay inside the original prefix, and the source owned by this document stays byte-exact.
    let powershell_parse_source;
    let parser_source = if kind == "powershell" {
        powershell_parse_source = format!("{source}\n;");
        powershell_parse_source.as_str()
    } else {
        source
    };
    let Some(tree) = parser.parse(parser_source, None) else {
        return ParseOutcome::ParserUnavailable(ParseFailure {
            kind: kind.to_owned(),
            line: None,
            column: None,
        });
    };
    let error = first_fatal_error(tree.root_node(), kind, source);
    if error.is_some() {
        return ParseOutcome::SyntaxError(ParseFailure {
            kind: kind.to_owned(),
            line: error.map(|node| node.start_position().row.saturating_add(1)),
            column: error.map(|node| node.start_position().column.saturating_add(1)),
        });
    }
    ParseOutcome::Parsed(ParsedDocument {
        kind: kind.to_owned(),
        source: source.to_owned(),
        tree,
    })
}

fn first_fatal_error<'tree>(
    node: tree_sitter::Node<'tree>,
    kind: &str,
    source: &str,
) -> Option<tree_sitter::Node<'tree>> {
    // The v0.4 fish adapter is total. The maintained grammar can still emit a local ERROR node for
    // a valid Unicode variable expansion, while preserving every command node that owns skit's
    // semantic surface. Keep that established total contract and consume only complete nodes.
    if kind == "fish" {
        return None;
    }
    if node.is_error() && kind == "powershell" && powershell::recoverable_error(source, node) {
        return None;
    }
    if node.is_error()
        || (node.is_missing()
            && !(kind == "powershell"
                && node.kind() == ";"
                && node.start_byte() == node.end_byte()))
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| first_fatal_error(child, kind, source))
}

/// Parse source once and return its live semantics for one parameter.
#[must_use]
pub fn source_parameter_semantics(
    kind: &str,
    source: &str,
    declaration: &ParamDecl,
) -> SourceParameterSemantics {
    let ParseOutcome::Parsed(document) = parse_document(kind, source) else {
        return SourceParameterSemantics::default();
    };
    document.source_parameter_semantics(declaration)
}

fn shell_parameter_semantics(
    document: &ParsedDocument,
    declaration: &ParamDecl,
) -> SourceParameterSemantics {
    if declaration.binding != ParameterBinding::EnvDefault {
        return SourceParameterSemantics::default();
    }
    let mut semantics = SourceParameterSemantics::default();
    let mut matched = false;
    walk(document.tree.root_node(), &mut |node| {
        if matched || node.kind() != "expansion" {
            return;
        }
        let Some(name) = named_children(node)
            .into_iter()
            .find(|child| child.kind() == "variable_name")
        else {
            return;
        };
        if text(document, name) != declaration.name {
            return;
        }
        let Some(operator) = node.child_by_field_name("operator") else {
            return;
        };
        match text(document, operator) {
            ":-" | ":=" => {
                semantics.empty_uses_default = true;
                matched = true;
            }
            "-" | "=" => matched = true,
            _ => {}
        }
    });
    semantics
}

fn text<'a>(document: &'a ParsedDocument, node: tree_sitter::Node<'_>) -> &'a str {
    document
        .source
        .get(node.byte_range())
        .expect("parser byte ranges refer to the parsed source")
}

fn named_children(node: tree_sitter::Node<'_>) -> Vec<tree_sitter::Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn walk<'tree>(node: tree_sitter::Node<'tree>, visit: &mut impl FnMut(tree_sitter::Node<'tree>)) {
    visit(node);
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk(child, visit);
    }
}

fn trailing_name<'a>(document: &'a ParsedDocument, node: tree_sitter::Node<'_>) -> &'a str {
    match node.kind() {
        "identifier" => text(document, node),
        "parenthesized_expression" => named_children(node)
            .first()
            .map_or("", |child| trailing_name(document, *child)),
        "attribute" => node
            .child_by_field_name("attribute")
            .map_or("", |attribute| text(document, attribute)),
        "call" => node.child_by_field_name("function").map_or("", |function| {
            let function = unwrap_parenthesized(function);
            if matches!(function.kind(), "identifier" | "attribute") {
                trailing_name(document, function)
            } else {
                ""
            }
        }),
        "type" => named_children(node)
            .first()
            .map_or("", |child| trailing_name(document, *child)),
        _ => "",
    }
}

fn scope_path(document: &ParsedDocument, mut node: tree_sitter::Node<'_>) -> Vec<String> {
    let mut path = Vec::new();
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "function_definition" | "class_definition")
            && let Some(name) = parent.child_by_field_name("name")
        {
            path.push(text(document, name).to_owned());
        }
        node = parent;
    }
    path.reverse();
    path
}

#[derive(Clone, Debug, PartialEq)]
enum PythonLiteral {
    Value(ParameterValue),
    None,
    Ellipsis,
}

fn python_literal(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<PythonLiteral> {
    match node.kind() {
        "parenthesized_expression" => named_children(node)
            .into_iter()
            .next()
            .and_then(|child| python_literal(document, child)),
        "true" => Some(PythonLiteral::Value(ParameterValue::Bool(true))),
        "false" => Some(PythonLiteral::Value(ParameterValue::Bool(false))),
        "none" => Some(PythonLiteral::None),
        "ellipsis" => Some(PythonLiteral::Ellipsis),
        "integer" => parse_python_integer(text(document, node))
            .map(ParameterValue::Integer)
            .map(PythonLiteral::Value),
        "float" => text(document, node)
            .replace('_', "")
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(ParameterValue::Float)
            .map(PythonLiteral::Value),
        "string" => decode_python_string(text(document, node))
            .map(ParameterValue::String)
            .map(PythonLiteral::Value),
        "concatenated_string" => {
            let mut joined = String::new();
            for child in named_children(node) {
                let value = match python_literal(document, child) {
                    Some(PythonLiteral::Value(ParameterValue::String(value))) => value,
                    _ => return None,
                };
                joined.push_str(&value);
            }
            Some(PythonLiteral::Value(ParameterValue::String(joined)))
        }
        "unary_operator" => {
            let argument = node.child_by_field_name("argument")?;
            let sign = text(document, node).trim_start().chars().next()?;
            match python_literal(document, argument)? {
                PythonLiteral::Value(ParameterValue::Integer(value)) => match sign {
                    '-' => value
                        .checked_neg()
                        .map(ParameterValue::Integer)
                        .map(PythonLiteral::Value),
                    '+' => Some(PythonLiteral::Value(ParameterValue::Integer(value))),
                    _ => None,
                },
                PythonLiteral::Value(ParameterValue::Float(value)) => match sign {
                    '-' => Some(PythonLiteral::Value(ParameterValue::Float(-value))),
                    '+' => Some(PythonLiteral::Value(ParameterValue::Float(value))),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

fn parse_python_integer(source: &str) -> Option<i64> {
    let source = source.replace('_', "");
    let (radix, digits) = if let Some(value) = source
        .strip_prefix("0x")
        .or_else(|| source.strip_prefix("0X"))
    {
        (16, value)
    } else if let Some(value) = source
        .strip_prefix("0o")
        .or_else(|| source.strip_prefix("0O"))
    {
        (8, value)
    } else if let Some(value) = source
        .strip_prefix("0b")
        .or_else(|| source.strip_prefix("0B"))
    {
        (2, value)
    } else {
        (10, source.as_str())
    };
    i64::from_str_radix(digits, radix).ok()
}

fn decode_python_string(source: &str) -> Option<String> {
    let quote_index = source.find(['\'', '"'])?;
    let prefix = &source[..quote_index];
    let prefix_lower = prefix.to_ascii_lowercase();
    if prefix_lower.contains('b') || prefix_lower.contains('f') {
        return None;
    }
    if !prefix_lower
        .chars()
        .all(|character| matches!(character, 'r' | 'u'))
    {
        return None;
    }
    let rest = &source[quote_index..];
    let quote = if rest.starts_with("'''") {
        "'''"
    } else if rest.starts_with("\"\"\"") {
        "\"\"\""
    } else if rest.starts_with('\'') {
        "'"
    } else {
        // `quote_index` points at either a single or double quote.
        "\""
    };
    let body = rest.strip_prefix(quote)?.strip_suffix(quote)?;
    if prefix_lower.contains('r') {
        return Some(body.to_owned());
    }
    decode_python_escapes(body)
}

fn decode_python_escapes(body: &str) -> Option<String> {
    let mut chars = body.chars().peekable();
    let mut output = String::new();
    while let Some(character) = chars.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\\' | '\'' | '"' => output.push(escaped),
            'a' => output.push('\u{7}'),
            'b' => output.push('\u{8}'),
            'f' => output.push('\u{c}'),
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'v' => output.push('\u{b}'),
            '\n' => {}
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
            }
            'x' => output.push(char::from_u32(read_radix(&mut chars, 2, 16)?)?),
            'u' => output.push(char::from_u32(read_radix(&mut chars, 4, 16)?)?),
            'U' => output.push(char::from_u32(read_radix(&mut chars, 8, 16)?)?),
            digit @ '0'..='7' => {
                let mut digits = String::from(digit);
                for _ in 0..2 {
                    if chars.peek().is_some_and(|next| matches!(next, '0'..='7')) {
                        digits.push(chars.next().expect("peeked octal digit exists"));
                    }
                }
                output.push(char::from_u32(u32::from_str_radix(&digits, 8).ok()?)?);
            }
            other => {
                // Python preserves unknown escapes, including the backslash.
                output.push('\\');
                output.push(other);
            }
        }
    }
    Some(output)
}

fn read_radix(
    chars: &mut std::iter::Peekable<impl Iterator<Item = char>>,
    count: usize,
    radix: u32,
) -> Option<u32> {
    let digits = (0..count)
        .map(|_| chars.next())
        .collect::<Option<String>>()?;
    u32::from_str_radix(&digits, radix).ok()
}

fn literal_value(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<ParameterValue> {
    match python_literal(document, node)? {
        PythonLiteral::Value(value) => Some(value),
        PythonLiteral::None | PythonLiteral::Ellipsis => None,
    }
}

fn parameter_type(value: &ParameterValue) -> ParameterType {
    match value {
        ParameterValue::String(_) => ParameterType::Str,
        ParameterValue::Integer(_) => ParameterType::Int,
        ParameterValue::Float(_) => ParameterType::Float,
        ParameterValue::Bool(_) => ParameterType::Bool,
    }
}

fn assignment_node(statement: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    if statement.kind() == "assignment" {
        Some(statement)
    } else if statement.kind() == "expression_statement" {
        named_children(statement)
            .into_iter()
            .next()
            .filter(|node| node.kind() == "assignment")
    } else {
        None
    }
}

fn block_constants(
    document: &ParsedDocument,
    block: tree_sitter::Node<'_>,
) -> Vec<SemanticCandidate> {
    let mut output = Vec::<SemanticCandidate>::new();
    for statement in named_children(block) {
        let Some(assignment) = assignment_node(statement) else {
            continue;
        };
        let Some(left) = assignment
            .child_by_field_name("left")
            .and_then(simple_assignment_name)
        else {
            continue;
        };
        let name = text(document, left);
        if name.starts_with('_') {
            continue;
        }
        let Some(right) = assignment.child_by_field_name("right") else {
            continue;
        };
        let Some(default) = literal_value(document, right) else {
            continue;
        };
        let mut declaration = ParamDecl::new(name);
        declaration.binding = ParameterBinding::Const;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.parameter_type = parameter_type(&default);
        declaration.default = Some(default);
        declaration.secret = is_secret_name(name);
        let candidate = SemanticCandidate {
            declaration,
            identity: BindingIdentity {
                binding: ParameterBinding::Const,
                key: name.to_owned(),
                occurrence: 0,
                scope: scope_path(document, assignment),
            },
            span: SourceSpan::from_node(assignment),
            demotion: None,
            empty_uses_default: false,
        };
        if let Some(existing) = output
            .iter_mut()
            .find(|current| current.declaration.name == name)
        {
            *existing = candidate;
        } else {
            output.push(candidate);
        }
    }
    output
}

fn is_main_guard(document: &ParsedDocument, statement: tree_sitter::Node<'_>) -> bool {
    if statement.kind() != "if_statement" {
        return false;
    }
    let Some(condition) = statement.child_by_field_name("condition") else {
        return false;
    };
    let condition = unwrap_parenthesized(condition);
    if condition.kind() != "comparison_operator" {
        return false;
    }
    let children = named_children(condition);
    if children.len() != 2
        || !condition
            .children(&mut condition.walk())
            .any(|child| !child.is_named() && text(document, child) == "==")
    {
        return false;
    }
    let is_name = |node: tree_sitter::Node<'_>| {
        let node = unwrap_parenthesized(node);
        node.kind() == "identifier" && text(document, node) == "__name__"
    };
    let is_main = |node: tree_sitter::Node<'_>| {
        matches!(
            python_literal(document, node),
            Some(PythonLiteral::Value(ParameterValue::String(value))) if value == "__main__"
        )
    };
    (is_name(children[0]) && is_main(children[1])) || (is_main(children[0]) && is_name(children[1]))
}

fn unwrap_parenthesized(mut node: tree_sitter::Node<'_>) -> tree_sitter::Node<'_> {
    while node.kind() == "parenthesized_expression" {
        let Some(child) = named_children(node).into_iter().next() else {
            break;
        };
        node = child;
    }
    node
}

fn simple_assignment_name(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let node = unwrap_parenthesized(node);
    if node.kind() == "identifier" {
        return Some(node);
    }
    if node.kind() != "tuple_pattern"
        || node
            .children(&mut node.walk())
            .any(|child| child.kind() == ",")
    {
        return None;
    }
    let mut children = named_children(node).into_iter();
    let child = children.next()?;
    if children.next().is_some() {
        return None;
    }
    simple_assignment_name(child)
}

fn mutated_names(document: &ParsedDocument) -> BTreeSet<String> {
    let mut output = BTreeSet::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "augmented_assignment"
            && let Some(left) = node.child_by_field_name("left")
            && left.kind() == "identifier"
        {
            output.insert(text(document, left).to_owned());
        }
        if matches!(node.kind(), "for_statement" | "while_statement") {
            walk(node, &mut |nested| {
                if let Some(assignment) = assignment_node(nested)
                    && let Some(left) = assignment.child_by_field_name("left")
                    && left.kind() == "identifier"
                {
                    output.insert(text(document, left).to_owned());
                }
                if nested.kind() == "augmented_assignment"
                    && let Some(left) = nested.child_by_field_name("left")
                    && left.kind() == "identifier"
                {
                    output.insert(text(document, left).to_owned());
                }
            });
        }
    });
    output
}

fn python_analysis(document: &ParsedDocument) -> SemanticAnalysis {
    let root = document.tree.root_node();
    let mut candidates = block_constants(document, root);
    let mut seen = candidates
        .iter()
        .map(|candidate| candidate.declaration.name.clone())
        .collect::<BTreeSet<_>>();
    for statement in named_children(root) {
        if is_main_guard(document, statement)
            && let Some(body) = statement.child_by_field_name("consequence")
        {
            for candidate in block_constants(document, body) {
                if seen.insert(candidate.declaration.name.clone()) {
                    candidates.push(candidate);
                }
            }
        }
    }
    let mutated = mutated_names(document);
    for candidate in &mut candidates {
        if mutated.contains(&candidate.declaration.name) {
            candidate.demotion = Some(DegradationReason::Accumulator);
        }
    }

    let calls = input_calls(document);
    let mut prompt_occurrences = BTreeMap::<String, usize>::new();
    for (order, call) in calls.into_iter().enumerate() {
        let prompt = call
            .child_by_field_name("arguments")
            .and_then(|arguments| named_children(arguments).into_iter().next())
            .and_then(|argument| match python_literal(document, argument) {
                Some(PythonLiteral::Value(ParameterValue::String(value))) => Some(value),
                _ => None,
            })
            .unwrap_or_default();
        let occurrence = *prompt_occurrences.entry(prompt.clone()).or_default();
        prompt_occurrences
            .entry(prompt.clone())
            .and_modify(|value| *value = value.saturating_add(1));
        let mut declaration = ParamDecl::new(format!("input-{}", order.saturating_add(1)));
        declaration.binding = ParameterBinding::Input;
        declaration.delivery = ParameterDelivery::Inject;
        declaration.prompt = prompt.clone();
        declaration.order = i64::try_from(order).unwrap_or(i64::MAX);
        declaration.secret = is_secret_name(&prompt);
        candidates.push(SemanticCandidate {
            declaration,
            identity: BindingIdentity {
                binding: ParameterBinding::Input,
                key: prompt,
                occurrence,
                scope: scope_path(document, call),
            },
            span: SourceSpan::from_node(call),
            demotion: None,
            empty_uses_default: false,
        });
    }

    SemanticAnalysis {
        candidates,
        frameworks: imported_frameworks(document),
        uses_argv: tree_has_attribute(document, "sys", "argv"),
        filename_literals: filename_literals(document),
        uses_self_location: tree_has_identifier(document, "__file__"),
    }
}

fn tree_has_attribute(document: &ParsedDocument, object: &str, attribute: &str) -> bool {
    let mut found = false;
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "attribute"
            && node.child_by_field_name("object").is_some_and(|value| {
                let value = unwrap_parenthesized(value);
                value.kind() == "identifier" && text(document, value) == object
            })
            && node
                .child_by_field_name("attribute")
                .is_some_and(|value| text(document, value) == attribute)
        {
            found = true;
        }
    });
    found
}

fn tree_has_identifier(document: &ParsedDocument, name: &str) -> bool {
    let mut found = false;
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "identifier" && text(document, node) == name {
            found = true;
        }
    });
    found
}

fn filename_literals(document: &ParsedDocument) -> Vec<String> {
    let mut positioned = Vec::<(usize, String)>::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() != "call" {
            return;
        }
        let arguments = node
            .child_by_field_name("arguments")
            .expect("a parsed Python call has an argument list");
        for argument in named_children(arguments) {
            let value_node = if argument.kind() == "keyword_argument" {
                argument
                    .child_by_field_name("value")
                    .expect("a parsed keyword argument has a value")
            } else {
                argument
            };
            let Some(PythonLiteral::Value(ParameterValue::String(value))) =
                python_literal(document, value_node)
            else {
                continue;
            };
            if looks_like_filename(&value) {
                positioned.push((value_node.start_byte(), value));
            }
        }
    });
    positioned.sort_by_key(|(position, _)| *position);
    let mut seen = BTreeSet::new();
    positioned
        .into_iter()
        .filter_map(|(_, value)| seen.insert(value.clone()).then_some(value))
        .take(3)
        .collect()
}

fn looks_like_filename(value: &str) -> bool {
    if value.is_empty() || value.contains(char::is_whitespace) || value.contains("://") {
        return false;
    }
    let Some((stem, extension)) = value.rsplit_once('.') else {
        return false;
    };
    (1..=120).contains(&stem.chars().count())
        && (2..=4).contains(&extension.len())
        && extension
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic())
        && extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn imported_frameworks(document: &ParsedDocument) -> Vec<String> {
    let supported = ["argparse", "click", "typer", "docopt", "fire"];
    let mut positioned = Vec::<(usize, String)>::new();
    walk(document.tree.root_node(), &mut |node| {
        if !matches!(node.kind(), "import_statement" | "import_from_statement") {
            return;
        }
        for (ordinal, root) in imported_roots(document, node).into_iter().enumerate() {
            if supported.contains(&root.as_str()) {
                positioned.push((node.start_byte().saturating_add(ordinal), root));
            }
        }
    });
    positioned.sort_by_key(|(position, _)| *position);
    let mut seen = BTreeSet::new();
    positioned
        .into_iter()
        .filter_map(|(_, name)| seen.insert(name.clone()).then_some(name))
        .collect()
}

fn imported_roots(document: &ParsedDocument, statement: tree_sitter::Node<'_>) -> Vec<String> {
    if statement.kind() == "import_statement" {
        children_by_field_name(statement, "name")
            .into_iter()
            .filter_map(|name| {
                let module = if name.kind() == "aliased_import" {
                    name.child_by_field_name("name")?
                } else {
                    name
                };
                first_identifier(document, module)
            })
            .collect()
    } else {
        debug_assert_eq!(statement.kind(), "import_from_statement");
        statement
            .child_by_field_name("module_name")
            .filter(|module| module.kind() != "relative_import")
            .and_then(|module| first_identifier(document, module))
            .into_iter()
            .collect()
    }
}

fn input_calls<'tree>(document: &'tree ParsedDocument) -> Vec<tree_sitter::Node<'tree>> {
    let root = document.tree.root_node();
    if scope_binds_input(document, root) {
        return Vec::new();
    }
    let mut calls = Vec::new();
    walk(root, &mut |node| {
        if node.kind() != "call"
            || !node
                .child_by_field_name("function")
                .is_some_and(|function| {
                    let function = unwrap_parenthesized(function);
                    function.kind() == "identifier" && text(document, function) == "input"
                })
        {
            return;
        }
        let mut ancestor = node.parent();
        while let Some(scope) = ancestor {
            if is_python_scope(scope.kind()) && scope_binds_input(document, scope) {
                return;
            }
            ancestor = scope.parent();
        }
        calls.push(node);
    });
    calls.sort_by_key(tree_sitter::Node::start_byte);
    calls
}

fn scope_binds_input(document: &ParsedDocument, scope: tree_sitter::Node<'_>) -> bool {
    if let Some(parameters) = scope.child_by_field_name("parameters")
        && parameter_names(document, parameters)
            .iter()
            .any(|name| name == "input")
    {
        return true;
    }
    let body = if matches!(
        scope.kind(),
        "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    ) {
        // A comprehension target is a sibling of its element expression in the syntax tree, but
        // both share the comprehension's local scope.
        scope
    } else {
        scope.child_by_field_name("body").unwrap_or(scope)
    };
    body_binds_name(document, body, "input", body.id() == scope.id())
}

fn is_python_scope(kind: &str) -> bool {
    matches!(
        kind,
        "function_definition"
            | "lambda"
            | "class_definition"
            | "list_comprehension"
            | "set_comprehension"
            | "dictionary_comprehension"
            | "generator_expression"
    )
}

fn body_binds_name(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
    expected: &str,
    root: bool,
) -> bool {
    for child in named_children(node) {
        if explicit_binding_names(document, child)
            .iter()
            .any(|name| name == expected)
        {
            return true;
        }
        if child.kind() == "for_in_clause"
            && child
                .child_by_field_name("left")
                .is_some_and(|left| target_binds_name(document, left, expected))
        {
            return true;
        }
        if child.kind() == "for_statement"
            && child
                .child_by_field_name("left")
                .is_some_and(|left| target_binds_name(document, left, expected))
        {
            return true;
        }
        if let Some(assignment) = assignment_node(child)
            && assignment
                .child_by_field_name("left")
                .is_some_and(|left| target_binds_name(document, left, expected))
        {
            return true;
        }
        if matches!(child.kind(), "augmented_assignment" | "named_expression")
            && child
                .child_by_field_name("left")
                .or_else(|| child.child_by_field_name("name"))
                .is_some_and(|left| target_binds_name(document, left, expected))
        {
            return true;
        }
        if matches!(child.kind(), "function_definition" | "class_definition") {
            if child
                .child_by_field_name("name")
                .is_some_and(|name| text(document, name) == expected)
            {
                return true;
            }
            continue;
        }
        if is_python_scope(child.kind()) {
            continue;
        }
        if matches!(child.kind(), "decorated_definition") {
            if named_children(child).into_iter().any(|definition| {
                matches!(
                    definition.kind(),
                    "function_definition" | "class_definition"
                ) && definition
                    .child_by_field_name("name")
                    .is_some_and(|name| text(document, name) == expected)
            }) {
                return true;
            }
            continue;
        }
        if matches!(child.kind(), "import_statement" | "import_from_statement")
            && import_binds_name(document, child, expected)
        {
            return true;
        }
        if !(root && matches!(child.kind(), "function_definition" | "class_definition"))
            && body_binds_name(document, child, expected, false)
        {
            return true;
        }
    }
    false
}

fn target_binds_name(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
    expected: &str,
) -> bool {
    if node.kind() == "identifier" {
        return text(document, node) == expected;
    }
    if matches!(node.kind(), "attribute" | "subscript") {
        return false;
    }
    named_children(node)
        .into_iter()
        .any(|child| target_binds_name(document, child, expected))
}

fn import_binds_name(
    document: &ParsedDocument,
    statement: tree_sitter::Node<'_>,
    expected: &str,
) -> bool {
    import_bound_names(document, statement)
        .iter()
        .any(|name| name == expected || name == "*")
}

#[derive(Debug)]
struct CallParts<'tree> {
    positional: Vec<tree_sitter::Node<'tree>>,
    keywords: BTreeMap<String, tree_sitter::Node<'tree>>,
}

fn call_parts<'tree>(
    document: &ParsedDocument,
    call: tree_sitter::Node<'tree>,
) -> CallParts<'tree> {
    let mut positional = Vec::new();
    let mut keywords = BTreeMap::new();
    if let Some(arguments) = call.child_by_field_name("arguments") {
        for argument in named_children(arguments) {
            if argument.kind() == "keyword_argument" {
                if let (Some(name), Some(value)) = (
                    argument.child_by_field_name("name"),
                    argument.child_by_field_name("value"),
                ) {
                    keywords.insert(text(document, name).to_owned(), value);
                }
            } else {
                positional.push(argument);
            }
        }
    }
    CallParts {
        positional,
        keywords,
    }
}

fn calls_named<'tree>(
    document: &'tree ParsedDocument,
    expected: &str,
) -> Vec<tree_sitter::Node<'tree>> {
    let mut calls = Vec::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() == "call"
            && node
                .child_by_field_name("function")
                .is_some_and(|function| trailing_name(document, function) == expected)
        {
            calls.push(node);
        }
    });
    calls.sort_by_key(tree_sitter::Node::start_byte);
    calls
}

fn python_cli_surface(document: &ParsedDocument) -> CliSurface {
    for reader in [argparse_surface, click_surface, typer_surface] {
        if let Some(surface) = reader(document) {
            return surface;
        }
    }
    CliSurface::Absent
}

fn dynamic_surface(framework: &str, reason: DegradationReason) -> CliSurface {
    CliSurface::Dynamic(DynamicCliSurface {
        framework: framework.to_owned(),
        reason,
    })
}

fn static_surface(framework: &str, fields: Vec<SemanticField>) -> CliSurface {
    CliSurface::Static(StaticCliSurface {
        framework: framework.to_owned(),
        fields,
    })
}

fn semantic_field(
    document: &ParsedDocument,
    call: tree_sitter::Node<'_>,
    ordinal: usize,
    declaration: ParamDecl,
    degradation: Option<DegradationReason>,
) -> SemanticField {
    SemanticField {
        identity: BindingIdentity {
            binding: ParameterBinding::None,
            key: declaration.name.clone(),
            occurrence: ordinal,
            scope: scope_path(document, call),
        },
        span: SourceSpan::from_node(call),
        declaration,
        degradation,
    }
}

fn field_occurrence(fields: &[SemanticField], name: &str) -> usize {
    fields
        .iter()
        .filter(|field| field.declaration.name == name)
        .count()
}

fn argparse_surface(document: &ParsedDocument) -> Option<CliSurface> {
    let calls = calls_named(document, "add_argument");
    if calls.is_empty() {
        return None;
    }
    if !calls_named(document, "add_subparsers").is_empty() {
        return Some(dynamic_surface("argparse", DegradationReason::Subcommands));
    }
    if calls.iter().any(|call| has_loop_ancestor(*call)) {
        return Some(dynamic_surface(
            "argparse",
            DegradationReason::DynamicDeclaration,
        ));
    }
    let env = constant_environment(document);
    let mut fields = Vec::new();
    for call in calls {
        if let Some((declaration, degradation)) = argparse_field(document, call, &env) {
            let ordinal = field_occurrence(&fields, &declaration.name);
            fields.push(semantic_field(
                document,
                call,
                ordinal,
                declaration,
                degradation,
            ));
        }
    }
    Some(static_surface("argparse", fields))
}

fn has_loop_ancestor(mut node: tree_sitter::Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "for_statement" | "while_statement") {
            return true;
        }
        node = parent;
    }
    false
}

fn argparse_field(
    document: &ParsedDocument,
    call: tree_sitter::Node<'_>,
    env: &BTreeMap<String, ParameterValue>,
) -> Option<(ParamDecl, Option<DegradationReason>)> {
    let parts = call_parts(document, call);
    let names = literal_string_arguments(document, &parts.positional)?;
    if names.is_empty() {
        return None;
    }
    let action = parts
        .keywords
        .get("action")
        .and_then(|node| literal_string(document, *node));
    if matches!(action.as_deref(), Some("help" | "version")) {
        return None;
    }
    let positional = !names[0].starts_with('-');
    let flag = if positional {
        String::new()
    } else {
        names
            .iter()
            .find(|name| name.starts_with("--"))
            .unwrap_or(&names[0])
            .clone()
    };
    let name = parts
        .keywords
        .get("dest")
        .and_then(|node| literal_string(document, *node))
        .unwrap_or_else(|| {
            if positional {
                names[0].clone()
            } else {
                flag.trim_start_matches('-').replace('-', "_")
            }
        });
    let nargs = parts
        .keywords
        .get("nargs")
        .and_then(|node| python_literal(document, *node));
    let nargs_string = nargs.as_ref().and_then(literal_as_string);
    let mut declaration = ParamDecl::new(&name);
    declaration.flag = flag;
    declaration.required = if positional {
        !matches!(nargs_string.as_deref(), Some("*" | "?"))
    } else {
        parts
            .keywords
            .get("required")
            .is_some_and(|node| literal_bool(document, *node) == Some(true))
    };
    declaration.multiple = matches!(nargs_string.as_deref(), Some("+" | "*"))
        || matches!(nargs, Some(PythonLiteral::Value(ParameterValue::Integer(value))) if value > 1);
    declaration.help = parts
        .keywords
        .get("help")
        .and_then(|node| literal_string(document, *node))
        .unwrap_or_default();
    declaration.secret = is_secret_name(&name);

    let degradation = match action.as_deref() {
        Some("store_true") => {
            declaration.parameter_type = ParameterType::Bool;
            declaration.action = "store_true".to_owned();
            declaration.default = Some(ParameterValue::Bool(false));
            None
        }
        Some("store_false") => {
            declaration.parameter_type = ParameterType::Bool;
            declaration.action = "store_false".to_owned();
            declaration.default = Some(ParameterValue::Bool(true));
            None
        }
        Some(_) => Some(DegradationReason::UnsupportedAction),
        None => apply_value_keywords(document, &mut declaration, &parts.keywords, env),
    };
    declaration.degraded = degradation.is_some();
    Some((declaration, degradation))
}

fn apply_value_keywords(
    document: &ParsedDocument,
    declaration: &mut ParamDecl,
    keywords: &BTreeMap<String, tree_sitter::Node<'_>>,
    env: &BTreeMap<String, ParameterValue>,
) -> Option<DegradationReason> {
    if let Some(choices) = keywords.get("choices") {
        let Some(values) = literal_choice_list(document, *choices) else {
            return Some(DegradationReason::DynamicChoices);
        };
        declaration.parameter_type = ParameterType::Choice;
        declaration.choices = values;
    }
    if let Some(type_node) = keywords.get("type")
        && !apply_python_type(document, declaration, *type_node)
    {
        return Some(DegradationReason::DynamicType);
    }
    if let Some(default) = keywords.get("default") {
        match resolved_literal(document, *default, env) {
            Some(PythonLiteral::Value(value)) => declaration.default = Some(value),
            Some(PythonLiteral::None) => {}
            Some(PythonLiteral::Ellipsis) | None => {
                return Some(DegradationReason::DynamicDefault);
            }
        }
    }
    None
}

fn literal_string_arguments(
    document: &ParsedDocument,
    arguments: &[tree_sitter::Node<'_>],
) -> Option<Vec<String>> {
    arguments
        .iter()
        .map(|node| literal_string(document, *node))
        .collect()
}

fn literal_string(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<String> {
    match python_literal(document, node)? {
        PythonLiteral::Value(ParameterValue::String(value)) => Some(value),
        _ => None,
    }
}

fn literal_bool(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<bool> {
    match python_literal(document, node)? {
        PythonLiteral::Value(ParameterValue::Bool(value)) => Some(value),
        _ => None,
    }
}

fn literal_as_string(literal: &PythonLiteral) -> Option<String> {
    match literal {
        PythonLiteral::Value(value) => Some(parameter_value_text(value)),
        PythonLiteral::None | PythonLiteral::Ellipsis => None,
    }
}

fn parameter_value_text(value: &ParameterValue) -> String {
    match value {
        ParameterValue::String(value) => value.clone(),
        ParameterValue::Integer(value) => value.to_string(),
        ParameterValue::Float(value) => render_float(*value),
        ParameterValue::Bool(value) => if *value { "True" } else { "False" }.to_owned(),
    }
}

fn literal_choice_list(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
) -> Option<Vec<String>> {
    let node = unwrap_parenthesized(node);
    if !matches!(node.kind(), "list" | "tuple") {
        return None;
    }
    named_children(node)
        .into_iter()
        .map(|child| python_literal(document, child).and_then(|value| literal_as_string(&value)))
        .collect()
}

fn apply_python_type(
    document: &ParsedDocument,
    declaration: &mut ParamDecl,
    node: tree_sitter::Node<'_>,
) -> bool {
    let name = trailing_name(document, node);
    let reflected = match name {
        "int" => Some(ParameterType::Int),
        "float" => Some(ParameterType::Float),
        "str" => Some(ParameterType::Str),
        "Path" | "FileType" => Some(ParameterType::Path),
        _ => None,
    };
    if let Some(parameter_type) = reflected {
        if declaration.parameter_type != ParameterType::Choice {
            declaration.parameter_type = parameter_type;
        }
        true
    } else {
        false
    }
}

fn resolved_literal(
    document: &ParsedDocument,
    node: tree_sitter::Node<'_>,
    env: &BTreeMap<String, ParameterValue>,
) -> Option<PythonLiteral> {
    let node = unwrap_parenthesized(node);
    python_literal(document, node).or_else(|| {
        (node.kind() == "identifier")
            .then(|| {
                env.get(text(document, node))
                    .cloned()
                    .map(PythonLiteral::Value)
            })
            .flatten()
    })
}

fn constant_environment(document: &ParsedDocument) -> BTreeMap<String, ParameterValue> {
    let bindings = bound_name_counts(document);
    if bindings.contains_key("*") {
        return BTreeMap::new();
    }
    block_constants(document, document.tree.root_node())
        .into_iter()
        .filter_map(|candidate| {
            let declaration = candidate.declaration;
            (bindings.get(&declaration.name) == Some(&1) && !declaration.secret)
                .then(|| declaration.default.map(|value| (declaration.name, value)))
                .flatten()
        })
        .collect()
}

fn bound_name_counts(document: &ParsedDocument) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::<String, usize>::new();
    let mut bump = |name: &str| {
        *counts.entry(name.to_owned()).or_default() += 1;
    };
    walk(document.tree.root_node(), &mut |node| match node.kind() {
        "assignment" | "augmented_assignment" | "named_expression" | "delete_statement" => {
            if let Some(target) = node
                .child_by_field_name("left")
                .or_else(|| node.child_by_field_name("name"))
                .or_else(|| named_children(node).into_iter().next())
            {
                for name in target_names(document, target) {
                    bump(&name);
                }
            }
        }
        "for_statement" | "for_in_clause" => {
            if let Some(target) = node.child_by_field_name("left") {
                for name in target_names(document, target) {
                    bump(&name);
                }
            }
        }
        "function_definition" | "class_definition" => {
            if let Some(name) = node.child_by_field_name("name") {
                bump(text(document, name));
            }
            if let Some(parameters) = node.child_by_field_name("parameters") {
                for name in parameter_names(document, parameters) {
                    bump(&name);
                }
            }
        }
        "lambda" => {
            if let Some(parameters) = node.child_by_field_name("parameters") {
                for name in parameter_names(document, parameters) {
                    bump(&name);
                }
            }
        }
        "import_statement" | "import_from_statement" => {
            for name in import_bound_names(document, node) {
                bump(&name);
            }
        }
        "except_clause"
        | "as_pattern"
        | "case_pattern"
        | "dictionary_splat_pattern"
        | "splat_pattern" => {
            for name in explicit_binding_names(document, node) {
                bump(&name);
            }
        }
        _ => {}
    });
    counts
}

fn explicit_binding_names(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Vec<String> {
    match node.kind() {
        "except_clause" | "as_pattern" => node
            .child_by_field_name("alias")
            .into_iter()
            .flat_map(|alias| target_names(document, alias))
            .collect(),
        "case_pattern" => named_children(node)
            .into_iter()
            .filter(|child| child.kind() == "dotted_name")
            .filter_map(|name| {
                let segments = named_children(name);
                match segments.as_slice() {
                    [segment] if segment.kind() == "identifier" => {
                        Some(text(document, *segment).to_owned())
                    }
                    _ => None,
                }
            })
            .collect(),
        "dictionary_splat_pattern" | "splat_pattern" => target_names(document, node),
        _ => Vec::new(),
    }
}

fn target_names(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Vec<String> {
    if node.kind() == "identifier" {
        return vec![text(document, node).to_owned()];
    }
    if node.kind() == "attribute" || node.kind() == "subscript" {
        return Vec::new();
    }
    named_children(node)
        .into_iter()
        .flat_map(|child| target_names(document, child))
        .collect()
}

fn parameter_names(document: &ParsedDocument, parameters: tree_sitter::Node<'_>) -> Vec<String> {
    named_children(parameters)
        .into_iter()
        .filter_map(|parameter| parameter_name_node(parameter))
        .map(|name| text(document, name).to_owned())
        .collect()
}

fn parameter_name_node(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    match node.kind() {
        "identifier" => Some(node),
        "typed_parameter" | "typed_default_parameter" | "default_parameter" => {
            node.child_by_field_name("name").or_else(|| {
                named_children(node)
                    .into_iter()
                    .find(|child| child.kind() == "identifier")
            })
        }
        "list_splat" | "dictionary_splat" | "list_splat_pattern" | "dictionary_splat_pattern" => {
            named_children(node)
                .into_iter()
                .find(|child| child.kind() == "identifier")
        }
        _ => None,
    }
}

fn children_by_field_name<'tree>(
    node: tree_sitter::Node<'tree>,
    field: &str,
) -> Vec<tree_sitter::Node<'tree>> {
    let mut cursor = node.walk();
    node.children_by_field_name(field, &mut cursor).collect()
}

fn first_identifier(document: &ParsedDocument, node: tree_sitter::Node<'_>) -> Option<String> {
    if node.kind() == "identifier" {
        return Some(text(document, node).to_owned());
    }
    named_children(node)
        .into_iter()
        .find_map(|child| first_identifier(document, child))
}

fn import_bound_names(document: &ParsedDocument, statement: tree_sitter::Node<'_>) -> Vec<String> {
    if statement.kind() == "import_from_statement"
        && named_children(statement)
            .iter()
            .any(|child| child.kind() == "wildcard_import")
    {
        return vec!["*".to_owned()];
    }
    children_by_field_name(statement, "name")
        .into_iter()
        .filter_map(|name| {
            if name.kind() == "aliased_import" {
                name.child_by_field_name("alias")
                    .map(|alias| text(document, alias).to_owned())
            } else {
                first_identifier(document, name)
            }
        })
        .collect()
}

#[derive(Clone, Debug)]
struct DecoratedFunction<'tree> {
    function: tree_sitter::Node<'tree>,
    decorators: Vec<tree_sitter::Node<'tree>>,
}

fn decorated_functions(document: &ParsedDocument) -> Vec<DecoratedFunction<'_>> {
    let mut output = Vec::new();
    walk(document.tree.root_node(), &mut |node| {
        if node.kind() != "decorated_definition" {
            return;
        }
        let children = named_children(node);
        let function = node
            .child_by_field_name("definition")
            .expect("a parsed decorated definition has a definition");
        let decorators = children
            .into_iter()
            .filter(|child| child.kind() == "decorator")
            .filter_map(decorator_expression)
            .collect();
        output.push(DecoratedFunction {
            function,
            decorators,
        });
    });
    output.sort_by_key(|item| item.function.start_byte());
    output
}

fn decorator_expression(node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    named_children(node).into_iter().next()
}

fn click_surface(document: &ParsedDocument) -> Option<CliSurface> {
    if !imported_frameworks(document)
        .iter()
        .any(|name| name == "click")
    {
        return None;
    }
    let decorated = decorated_functions(document);
    let has_group = decorated.iter().any(|function| {
        function
            .decorators
            .iter()
            .any(|decorator| trailing_name(document, *decorator) == "group")
    });
    let commands = decorated
        .into_iter()
        .filter(|function| {
            function
                .decorators
                .iter()
                .any(|decorator| trailing_name(document, *decorator) == "command")
        })
        .collect::<Vec<_>>();
    if !has_group && commands.is_empty() {
        return None;
    }
    if has_group || commands.len() > 1 {
        return Some(dynamic_surface("click", DegradationReason::Subcommands));
    }
    let env = constant_environment(document);
    let mut fields = Vec::new();
    for decorator in commands[0].decorators.iter().rev() {
        let decorator = unwrap_parenthesized(*decorator);
        let kind = trailing_name(document, decorator);
        if !matches!(kind, "option" | "argument") || decorator.kind() != "call" {
            continue;
        }
        if let Some((declaration, degradation)) =
            click_field(document, decorator, kind == "argument", &env)
        {
            let ordinal = field_occurrence(&fields, &declaration.name);
            fields.push(semantic_field(
                document,
                decorator,
                ordinal,
                declaration,
                degradation,
            ));
        }
    }
    Some(static_surface("click", fields))
}

fn click_field(
    document: &ParsedDocument,
    call: tree_sitter::Node<'_>,
    positional: bool,
    env: &BTreeMap<String, ParameterValue>,
) -> Option<(ParamDecl, Option<DegradationReason>)> {
    let parts = call_parts(document, call);
    let names = literal_string_arguments(document, &parts.positional)?;
    if names.is_empty() {
        return None;
    }
    let flag = if positional {
        String::new()
    } else {
        names
            .iter()
            .find(|name| name.starts_with("--"))
            .unwrap_or(&names[0])
            .clone()
    };
    let name = if positional {
        names[0].clone()
    } else {
        flag.trim_start_matches('-').replace('-', "_")
    };
    let nargs = parts
        .keywords
        .get("nargs")
        .and_then(|node| python_literal(document, *node));
    let variadic = matches!(
        nargs,
        Some(PythonLiteral::Value(ParameterValue::Integer(-1)))
    );
    let fixed_count =
        matches!(nargs, Some(PythonLiteral::Value(ParameterValue::Integer(value))) if value > 1);
    let repeated = parts
        .keywords
        .get("multiple")
        .is_some_and(|node| literal_bool(document, *node) == Some(true));
    if fixed_count && repeated {
        return None;
    }
    let mut declaration = ParamDecl::new(&name);
    declaration.flag = flag;
    declaration.required = (positional && !variadic)
        || parts
            .keywords
            .get("required")
            .is_some_and(|node| literal_bool(document, *node) == Some(true));
    declaration.multiple = variadic || fixed_count || repeated;
    declaration.repeat = repeated;
    declaration.help = parts
        .keywords
        .get("help")
        .and_then(|node| literal_string(document, *node))
        .unwrap_or_default();
    declaration.secret = is_secret_name(&name);

    let degradation = if parts
        .keywords
        .get("is_flag")
        .is_some_and(|node| literal_bool(document, *node) == Some(true))
    {
        if parts
            .keywords
            .get("default")
            .is_some_and(|node| literal_bool(document, *node) == Some(true))
        {
            Some(DegradationReason::BooleanDefaultTrue)
        } else {
            declaration.parameter_type = ParameterType::Bool;
            declaration.action = "store_true".to_owned();
            declaration.default = Some(ParameterValue::Bool(false));
            None
        }
    } else if let Some(type_node) = parts.keywords.get("type")
        && !apply_click_type(document, &mut declaration, *type_node)
    {
        Some(DegradationReason::DynamicType)
    } else if let Some(default) = parts.keywords.get("default") {
        match resolved_literal(document, *default, env) {
            Some(PythonLiteral::Value(value)) => {
                declaration.default = Some(value);
                parts
                    .keywords
                    .contains_key("count")
                    .then_some(DegradationReason::UnsupportedAction)
            }
            Some(PythonLiteral::None) => parts
                .keywords
                .contains_key("count")
                .then_some(DegradationReason::UnsupportedAction),
            Some(PythonLiteral::Ellipsis) | None => Some(DegradationReason::DynamicDefault),
        }
    } else {
        parts
            .keywords
            .contains_key("count")
            .then_some(DegradationReason::UnsupportedAction)
    };
    declaration.degraded = degradation.is_some();
    Some((declaration, degradation))
}

fn apply_click_type(
    document: &ParsedDocument,
    declaration: &mut ParamDecl,
    node: tree_sitter::Node<'_>,
) -> bool {
    let node = unwrap_parenthesized(node);
    let name = trailing_name(document, node);
    declaration.parameter_type = match name {
        "int" | "INT" => ParameterType::Int,
        "float" | "FLOAT" => ParameterType::Float,
        "str" | "STRING" => ParameterType::Str,
        "Path" | "File" => ParameterType::Path,
        "Choice" if node.kind() == "call" => {
            let parts = call_parts(document, node);
            let Some(choices) = parts
                .positional
                .first()
                .and_then(|choices| literal_choice_list(document, *choices))
            else {
                return false;
            };
            declaration.choices = choices;
            ParameterType::Choice
        }
        _ => return false,
    };
    true
}

fn typer_surface(document: &ParsedDocument) -> Option<CliSurface> {
    if !imported_frameworks(document)
        .iter()
        .any(|name| name == "typer")
    {
        return None;
    }
    let decorated = decorated_functions(document);
    let mut commands = decorated
        .into_iter()
        .filter(|function| {
            function
                .decorators
                .iter()
                .any(|decorator| trailing_name(document, *decorator) == "command")
        })
        .map(|function| function.function)
        .collect::<Vec<_>>();
    if commands.is_empty() {
        let targets = typer_run_targets(document);
        if !targets.is_empty() {
            walk(document.tree.root_node(), &mut |node| {
                if node.kind() == "function_definition"
                    && node
                        .child_by_field_name("name")
                        .is_some_and(|name| targets.contains(text(document, name)))
                {
                    commands.push(node);
                }
            });
            commands.sort_by_key(tree_sitter::Node::start_byte);
        }
    }
    if commands.is_empty() {
        return None;
    }
    if commands.len() > 1 {
        return Some(dynamic_surface("typer", DegradationReason::Subcommands));
    }
    let env = constant_environment(document);
    let Some(parameters) = commands[0].child_by_field_name("parameters") else {
        return Some(static_surface("typer", Vec::new()));
    };
    let mut fields = Vec::new();
    for parameter in named_children(parameters) {
        let Some((declaration, degradation, span_node)) = typer_field(document, parameter, &env)
        else {
            continue;
        };
        let ordinal = field_occurrence(&fields, &declaration.name);
        fields.push(semantic_field(
            document,
            span_node,
            ordinal,
            declaration,
            degradation,
        ));
    }
    Some(static_surface("typer", fields))
}

fn typer_run_targets(document: &ParsedDocument) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for call in calls_named(document, "run") {
        let Some(function) = call.child_by_field_name("function") else {
            continue;
        };
        let function = unwrap_parenthesized(function);
        if function.kind() != "attribute"
            || !function
                .child_by_field_name("object")
                .is_some_and(|object| {
                    object.kind() == "identifier" && text(document, object) == "typer"
                })
        {
            continue;
        }
        let parts = call_parts(document, call);
        if let Some(target) = parts.positional.first()
            && target.kind() == "identifier"
        {
            targets.insert(text(document, *target).to_owned());
        }
    }
    targets
}

fn typer_field<'tree>(
    document: &ParsedDocument,
    parameter: tree_sitter::Node<'tree>,
    env: &BTreeMap<String, ParameterValue>,
) -> Option<(
    ParamDecl,
    Option<DegradationReason>,
    tree_sitter::Node<'tree>,
)> {
    let name_node = parameter_name_node(parameter)?;
    let name = text(document, name_node);
    let annotation = parameter.child_by_field_name("type");
    let default = parameter
        .child_by_field_name("value")
        .map(unwrap_parenthesized);
    let (base_annotation, annotated_meta) = annotation
        .map(|node| annotated_parts(document, node))
        .unwrap_or((None, None));
    let reflected = base_annotation.and_then(|node| typer_annotation_type(document, node));
    let mut declaration = ParamDecl::new(name);
    declaration.flag = format!("--{}", name.replace('_', "-"));
    declaration.secret = is_secret_name(name);
    let mut degradation = if base_annotation.is_some() && reflected.is_none() {
        Some(DegradationReason::UnsupportedAnnotation)
    } else {
        None
    };
    if let Some(parameter_type) = reflected {
        declaration.parameter_type = parameter_type;
    }

    if let Some(meta) = annotated_meta {
        let meta_degradation = apply_typer_meta(document, &mut declaration, meta, env, false);
        degradation = degradation.or(meta_degradation);
        let default_degradation =
            apply_typer_signature_default(document, &mut declaration, default, env);
        degradation = degradation.or(default_degradation);
    } else if let Some(default_node) = default {
        if default_node.kind() == "call"
            && matches!(trailing_name(document, default_node), "Option" | "Argument")
        {
            let meta_degradation =
                apply_typer_meta(document, &mut declaration, default_node, env, true);
            degradation = degradation.or(meta_degradation);
        } else {
            let default_degradation =
                apply_typer_signature_default(document, &mut declaration, default, env);
            degradation = degradation.or(default_degradation);
        }
    } else {
        declaration.flag.clear();
        declaration.required = true;
    }
    degradation = finish_typer_bool(&mut declaration, degradation);
    declaration.degraded = degradation.is_some();
    Some((declaration, degradation, parameter))
}

fn annotated_parts<'tree>(
    document: &ParsedDocument,
    mut annotation: tree_sitter::Node<'tree>,
) -> (
    Option<tree_sitter::Node<'tree>>,
    Option<tree_sitter::Node<'tree>>,
) {
    while annotation.kind() == "type"
        && let Some(child) = named_children(annotation).first()
    {
        annotation = *child;
    }
    let target = if annotation.kind() == "subscript" {
        annotation.child_by_field_name("value")
    } else if annotation.kind() == "generic_type" {
        named_children(annotation).first().copied()
    } else {
        None
    };
    if target.is_none_or(|name| trailing_name(document, name) != "Annotated") {
        return (Some(annotation), None);
    }
    let raw_values = if annotation.kind() == "subscript" {
        named_children(annotation).into_iter().skip(1).collect()
    } else {
        let Some(arguments) = named_children(annotation)
            .into_iter()
            .find(|child| child.kind() == "type_parameter")
        else {
            return (None, None);
        };
        named_children(arguments)
    };
    let values = raw_values
        .into_iter()
        .filter_map(|mut value| {
            while value.kind() == "type" {
                value = *named_children(value).first()?;
            }
            Some(unwrap_parenthesized(value))
        })
        .collect::<Vec<_>>();
    let base = values.first().copied();
    let meta = values.into_iter().skip(1).find(|value| {
        value.kind() == "call" && matches!(trailing_name(document, *value), "Option" | "Argument")
    });
    (base, meta)
}

fn typer_annotation_type(
    document: &ParsedDocument,
    annotation: tree_sitter::Node<'_>,
) -> Option<ParameterType> {
    match trailing_name(document, annotation) {
        "int" => Some(ParameterType::Int),
        "float" => Some(ParameterType::Float),
        "str" => Some(ParameterType::Str),
        "bool" => Some(ParameterType::Bool),
        "Path" => Some(ParameterType::Path),
        _ => None,
    }
}

fn apply_typer_meta(
    document: &ParsedDocument,
    declaration: &mut ParamDecl,
    call: tree_sitter::Node<'_>,
    env: &BTreeMap<String, ParameterValue>,
    has_positional_default: bool,
) -> Option<DegradationReason> {
    let parts = call_parts(document, call);
    let declaration_arguments = if has_positional_default {
        parts.positional.get(1..).unwrap_or_default()
    } else {
        parts.positional.as_slice()
    };
    let flag = declaration_arguments
        .iter()
        .filter_map(|node| literal_string(document, *node))
        .find(|value| value.starts_with("--"));
    if trailing_name(document, call) == "Argument" {
        declaration.flag.clear();
    } else if let Some(flag) = flag {
        declaration.flag = flag;
    }
    declaration.help = parts
        .keywords
        .get("help")
        .and_then(|node| literal_string(document, *node))
        .unwrap_or_default();
    if !has_positional_default {
        return None;
    }
    let default = parts.positional.first()?;
    match resolved_literal(document, *default, env) {
        Some(PythonLiteral::Ellipsis) => {
            declaration.required = true;
            None
        }
        Some(PythonLiteral::Value(value)) => {
            declaration.default = Some(value);
            None
        }
        Some(PythonLiteral::None) => None,
        None => Some(DegradationReason::DynamicDefault),
    }
}

fn apply_typer_signature_default(
    document: &ParsedDocument,
    declaration: &mut ParamDecl,
    default: Option<tree_sitter::Node<'_>>,
    env: &BTreeMap<String, ParameterValue>,
) -> Option<DegradationReason> {
    let Some(default) = default else {
        declaration.required = true;
        return None;
    };
    match resolved_literal(document, default, env) {
        Some(PythonLiteral::Value(value)) => {
            declaration.default = Some(value);
            None
        }
        Some(PythonLiteral::None) => None,
        Some(PythonLiteral::Ellipsis) | None => Some(DegradationReason::DynamicDefault),
    }
}

fn finish_typer_bool(
    declaration: &mut ParamDecl,
    degradation: Option<DegradationReason>,
) -> Option<DegradationReason> {
    if declaration.parameter_type != ParameterType::Bool {
        return degradation;
    }
    if !declaration.required
        && matches!(
            declaration.default,
            None | Some(ParameterValue::Bool(false))
        )
    {
        declaration.action = "store_true".to_owned();
        declaration.default = Some(ParameterValue::Bool(false));
        degradation
    } else {
        declaration.parameter_type = ParameterType::Str;
        degradation.or(Some(DegradationReason::BooleanDefaultTrue))
    }
}

fn reconcile_analysis(analysis: &SemanticAnalysis, stored: &[ParamDecl]) -> ReconcileReport {
    let mut report = ReconcileReport::default();
    let current_inputs = analysis
        .candidates
        .iter()
        .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
        .map(|candidate| {
            (
                candidate.declaration.order,
                candidate.declaration.prompt.clone(),
            )
        })
        .collect::<Vec<_>>();
    let stored_inputs = stored
        .iter()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
        .map(|declaration| (declaration.order, declaration.prompt.clone()))
        .collect::<Vec<_>>();
    let input_matches = match_calls(&stored_inputs, &current_inputs);
    let mut claimed = BTreeSet::<usize>::new();

    for declaration in stored {
        let matched = match declaration.binding {
            ParameterBinding::Input => {
                input_matches
                    .get(&declaration.order)
                    .and_then(|(order, ambiguous)| {
                        analysis
                            .candidates
                            .iter()
                            .enumerate()
                            .find(|(_, candidate)| {
                                candidate.declaration.binding == ParameterBinding::Input
                                    && candidate.declaration.order == *order
                            })
                            .map(|(index, candidate)| (index, candidate, *ambiguous))
                    })
            }
            ParameterBinding::Const | ParameterBinding::EnvDefault => analysis
                .candidates
                .iter()
                .enumerate()
                .find(|(_, candidate)| {
                    candidate.declaration.binding == declaration.binding
                        && candidate.declaration.name == declaration.name
                })
                .map(|(index, candidate)| (index, candidate, false)),
            ParameterBinding::None => None,
        };
        let Some((index, candidate, ambiguous)) = matched else {
            report.missing.push(declaration.clone());
            continue;
        };
        if !claimed.insert(index) {
            report.missing.push(declaration.clone());
            continue;
        }
        let pair = ReconcilePair {
            stored: declaration.clone(),
            current: candidate.clone(),
        };
        if declaration.binding == ParameterBinding::EnvDefault && candidate.empty_uses_default {
            report.empty_uses_default.insert(declaration.name.clone());
        }
        if ambiguous {
            report.rebound.push(pair);
        } else if declaration.binding != ParameterBinding::EnvDefault
            && declaration.parameter_type != candidate.declaration.parameter_type
            && !(matches!(
                declaration.parameter_type,
                ParameterType::Str | ParameterType::Path
            ) && matches!(
                candidate.declaration.parameter_type,
                ParameterType::Str | ParameterType::Path
            ))
        {
            // A const runs the type-drift check. An envdefault is matched by name only: its value
            // arrives from the environment, so a changed inline default type is not drift (the
            // envdefault stays ok through a type change, matching skit/analysis.py reconcile).
            report.changed.push(pair);
        } else {
            if !declaration.secret
                && let Some(default) = &candidate.declaration.default
                && coerce_default(&parameter_value_text(default), declaration.parameter_type)
                    .is_ok()
            {
                report
                    .current_defaults
                    .insert(declaration.name.clone(), default.clone());
            }
            report.ok.push(pair);
        }
    }
    report.new = analysis
        .candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| (!claimed.contains(&index)).then_some(candidate.clone()))
        .collect();
    report
}

pub(super) fn match_calls(
    stored: &[(i64, String)],
    current: &[(i64, String)],
) -> BTreeMap<i64, (i64, bool)> {
    let current_by_order = current.iter().cloned().collect::<BTreeMap<_, _>>();
    let mut by_prompt = BTreeMap::<String, Vec<i64>>::new();
    for (order, prompt) in current {
        if !prompt.is_empty() {
            by_prompt.entry(prompt.clone()).or_default().push(*order);
        }
    }
    let mut exact = BTreeMap::<i64, i64>::new();
    let mut claimed = BTreeSet::<i64>::new();
    let mut stored_by_prompt = BTreeMap::<String, Vec<i64>>::new();
    for (order, prompt) in stored {
        if !prompt.is_empty() {
            stored_by_prompt
                .entry(prompt.clone())
                .or_default()
                .push(*order);
        }
    }
    for (prompt, stored_orders) in stored_by_prompt {
        let current_orders = by_prompt.get(&prompt).cloned().unwrap_or_default();
        if stored_orders.len() > 1 && current_orders.len() == stored_orders.len() {
            let mut stored_orders = stored_orders;
            let mut current_orders = current_orders;
            stored_orders.sort_unstable();
            current_orders.sort_unstable();
            for (stored_order, current_order) in stored_orders.into_iter().zip(current_orders) {
                exact.insert(stored_order, current_order);
                claimed.insert(current_order);
            }
        }
    }
    for (order, prompt) in stored {
        if exact.contains_key(order) || prompt.is_empty() {
            continue;
        }
        let candidates = by_prompt.get(prompt).map(Vec::as_slice).unwrap_or_default();
        if let [candidate] = candidates
            && !claimed.contains(candidate)
        {
            exact.insert(*order, *candidate);
            claimed.insert(*candidate);
        }
    }
    let mut output = BTreeMap::new();
    for (order, prompt) in stored {
        if let Some(current_order) = exact.get(order) {
            output.insert(*order, (*current_order, false));
        } else if current_by_order.contains_key(order) && !claimed.contains(order) {
            output.insert(*order, (*order, !prompt.is_empty()));
        }
    }
    output
}

fn plan_python_injection(
    document: &ParsedDocument,
    declarations: &[ParamDecl],
    values: &BTreeMap<String, String>,
) -> Result<SourceEditPlan, LanguageError> {
    let selected = declarations
        .iter()
        .filter(|declaration| {
            declaration.delivery == ParameterDelivery::Inject
                && values.contains_key(&declaration.name)
        })
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Ok(SourceEditPlan {
            source: document.source.clone(),
            edits: Vec::new(),
        });
    }
    let mut edits = Vec::<SourceEdit>::new();
    let mut matched = BTreeSet::<String>::new();

    for declaration in selected
        .iter()
        .copied()
        .filter(|declaration| declaration.binding == ParameterBinding::Const)
    {
        let targets = constant_targets(document, &declaration.name);
        if targets.is_empty() {
            continue;
        }
        let raw = values
            .get(&declaration.name)
            .expect("selected declarations have accepted values");
        let replacement = typed_python_literal(declaration, raw)?;
        for target in targets {
            edits.push(SourceEdit {
                span: SourceSpan::from_node(target),
                replacement: replacement.clone(),
            });
        }
        matched.insert(declaration.name.clone());
    }

    let calls = input_calls(document);
    let current = calls
        .iter()
        .enumerate()
        .map(|(order, call)| {
            let prompt = call
                .child_by_field_name("arguments")
                .and_then(|arguments| named_children(arguments).into_iter().next())
                .and_then(|argument| literal_string(document, argument))
                .unwrap_or_default();
            (i64::try_from(order).unwrap_or(i64::MAX), prompt)
        })
        .collect::<Vec<_>>();
    let stored = selected
        .iter()
        .copied()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
        .map(|declaration| (declaration.order, declaration.prompt.clone()))
        .collect::<Vec<_>>();
    let bindings = match_calls(&stored, &current);
    let mut queue = BTreeMap::<usize, (String, bool)>::new();
    for declaration in selected
        .iter()
        .copied()
        .filter(|declaration| declaration.binding == ParameterBinding::Input)
    {
        let Some((resolved, _)) = bindings.get(&declaration.order) else {
            continue;
        };
        let Ok(resolved) = usize::try_from(*resolved) else {
            continue;
        };
        let Some(call) = calls.get(resolved) else {
            continue;
        };
        if queue.contains_key(&resolved) {
            continue;
        }
        let Some(function) = call.child_by_field_name("function") else {
            continue;
        };
        let value = values
            .get(&declaration.name)
            .expect("selected declarations have accepted values");
        queue.insert(resolved, (value.clone(), declaration.secret));
        edits.push(SourceEdit {
            span: SourceSpan::from_node(function),
            replacement: format!("_skit_i[{resolved}]"),
        });
        matched.insert(declaration.name.clone());
    }

    if let Some(missing) = selected
        .iter()
        .find(|declaration| !matched.contains(&declaration.name))
    {
        return Err(LanguageError::BindingNotFound {
            name: missing.name.clone(),
        });
    }
    if !queue.is_empty() {
        let offset = python_preamble_offset(document);
        let line = document.source[..offset]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            .saturating_add(1);
        edits.push(SourceEdit {
            span: SourceSpan::insertion(offset, line),
            replacement: python_input_preamble(&queue, newline_style(&document.source)),
        });
    }
    edits.sort_by_key(|edit| (edit.span.start, edit.span.end));
    Ok(SourceEditPlan {
        source: document.source.clone(),
        edits,
    })
}

fn constant_targets<'tree>(
    document: &'tree ParsedDocument,
    expected: &str,
) -> Vec<tree_sitter::Node<'tree>> {
    let mut targets = block_constant_targets(document, document.tree.root_node(), expected);
    for statement in named_children(document.tree.root_node()) {
        if is_main_guard(document, statement)
            && let Some(body) = statement.child_by_field_name("consequence")
        {
            targets.extend(block_constant_targets(document, body, expected));
        }
    }
    targets
}

fn block_constant_targets<'tree>(
    document: &'tree ParsedDocument,
    block: tree_sitter::Node<'tree>,
    expected: &str,
) -> Vec<tree_sitter::Node<'tree>> {
    named_children(block)
        .into_iter()
        .filter_map(assignment_node)
        .filter(|assignment| {
            assignment
                .child_by_field_name("left")
                .and_then(simple_assignment_name)
                .is_some_and(|left| text(document, left) == expected)
        })
        .filter_map(|assignment| assignment.child_by_field_name("right"))
        .filter(|right| literal_value(document, *right).is_some())
        .collect()
}

fn typed_python_literal(declaration: &ParamDecl, raw: &str) -> Result<String, LanguageError> {
    let invalid = || LanguageError::InvalidValue {
        name: declaration.name.clone(),
        value: raw.to_owned(),
        parameter_type: declaration.parameter_type,
    };
    match declaration.parameter_type {
        ParameterType::Int => raw
            .trim()
            .parse::<i64>()
            .map(|value| value.to_string())
            .map_err(|_| invalid()),
        ParameterType::Float => raw
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(render_float)
            .ok_or_else(invalid),
        ParameterType::Bool => canonical_bool(raw)
            .map(|value| if value { "True" } else { "False" }.to_owned())
            .ok_or_else(invalid),
        ParameterType::Str | ParameterType::Choice | ParameterType::Path => Ok(quote_python(raw)),
    }
}

pub(super) fn render_float(value: f64) -> String {
    let mut rendered = value.to_string();
    if !rendered.contains(['.', 'e', 'E']) {
        rendered.push_str(".0");
    }
    rendered
}

pub(super) fn canonical_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "y" | "on" => Some(true),
        "false" | "0" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

fn quote_python(value: &str) -> String {
    let mut output = String::from("'");
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\'' => output.push_str("\\'"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character
                if character <= '\u{ff}' && (character.is_control() || character == '\u{85}') =>
            {
                output.push_str(&format!("\\x{:02x}", u32::from(character)));
            }
            '\u{2028}' => output.push_str("\\u2028"),
            '\u{2029}' => output.push_str("\\u2029"),
            character => output.push(character),
        }
    }
    output.push('\'');
    output
}

fn python_preamble_offset(document: &ParsedDocument) -> usize {
    let statements = named_children(document.tree.root_node());
    let mut index = 0;
    if statements.first().is_some_and(|statement| {
        statement.kind() == "expression_statement"
            && named_children(*statement)
                .first()
                .is_some_and(|expression| {
                    matches!(
                        python_literal(document, *expression),
                        Some(PythonLiteral::Value(ParameterValue::String(_)))
                    )
                })
    }) {
        index = 1;
    }
    while statements
        .get(index)
        .is_some_and(|statement| statement.kind() == "future_import_statement")
    {
        index = index.saturating_add(1);
    }
    statements
        .get(index)
        .map_or(document.source.len(), |statement| statement.start_byte())
}

fn python_input_preamble(queue: &BTreeMap<usize, (String, bool)>, newline: &str) -> String {
    let queue_literal = queue
        .iter()
        .map(|(key, (value, secret))| {
            format!(
                "{key}: ({}, {})",
                quote_python(value),
                if *secret { "True" } else { "False" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let keys = queue
        .keys()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "import sys as _skit_s; _skit_o = input; _skit_q = {{{queue_literal}}}; _skit_i = {{k: (lambda p='', /, k=k: (((_skit_s.stdout.write(str(p) + ('***' if _skit_q[k][1] else _skit_q[k][0]) + chr(10)), _skit_q.pop(k)[0])[1]) if k in _skit_q else _skit_o(p))) for k in [{keys}]}}  # skit:shim{newline}"
    )
}

fn newline_style(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python_literal_for(expression: &str) -> Option<PythonLiteral> {
        let source = format!("value = {expression}\n");
        let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
            panic!("the literal fixture must parse");
        };
        let statement = named_children(document.syntax_tree().root_node())
            .into_iter()
            .next()?;
        let assignment = assignment_node(statement)?;
        let value = assignment.child_by_field_name("right")?;
        python_literal(&document, value)
    }

    fn python_literal_value_for(expression: &str) -> Option<ParameterValue> {
        let source = format!("value = {expression}\n");
        let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
            panic!("the literal fixture must parse");
        };
        let statement = named_children(document.syntax_tree().root_node())
            .into_iter()
            .next()?;
        let assignment = assignment_node(statement)?;
        literal_value(&document, assignment.child_by_field_name("right")?)
    }

    fn python_string_for(expression: &str) -> Option<String> {
        let source = format!("value = {expression}\n");
        let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
            panic!("the literal fixture must parse");
        };
        let statement = named_children(document.syntax_tree().root_node())
            .into_iter()
            .next()?;
        let assignment = assignment_node(statement)?;
        literal_string(&document, assignment.child_by_field_name("right")?)
    }

    fn python_bool_for(expression: &str) -> Option<bool> {
        let source = format!("value = {expression}\n");
        let ParseOutcome::Parsed(document) = parse_document("python", &source) else {
            panic!("the literal fixture must parse");
        };
        let statement = named_children(document.syntax_tree().root_node())
            .into_iter()
            .next()?;
        let assignment = assignment_node(statement)?;
        literal_bool(&document, assignment.child_by_field_name("right")?)
    }

    #[test]
    fn python_integer_literals_cover_every_supported_radix_and_sign() {
        for (source, expected) in [
            ("0", 0),
            ("1_000", 1_000),
            ("0xff", 255),
            ("0X_FF", 255),
            ("0o17", 15),
            ("0O_17", 15),
            ("0b101", 5),
            ("0B_101", 5),
        ] {
            assert_eq!(parse_python_integer(source), Some(expected));
        }
        assert_eq!(parse_python_integer("not-a-number"), None);
        assert_eq!(parse_python_integer("999999999999999999999999"), None);

        assert_eq!(
            python_literal_for("+7"),
            Some(PythonLiteral::Value(ParameterValue::Integer(7)))
        );
        assert_eq!(
            python_literal_for("-7"),
            Some(PythonLiteral::Value(ParameterValue::Integer(-7)))
        );
        assert_eq!(
            python_literal_for("+1.5"),
            Some(PythonLiteral::Value(ParameterValue::Float(1.5)))
        );
        assert_eq!(
            python_literal_for("-1.5"),
            Some(PythonLiteral::Value(ParameterValue::Float(-1.5)))
        );
        assert_eq!(python_literal_for("~1"), None);
        assert_eq!(python_literal_for("~1.5"), None);
        assert_eq!(python_literal_for("-True"), None);
        assert_eq!(python_literal_for("1e999"), None);
    }

    #[test]
    fn python_string_literals_decode_prefixes_quotes_and_concatenation() {
        for (source, expected) in [
            ("'plain'", "plain"),
            ("\"double\"", "double"),
            ("'''triple'''", "triple"),
            ("\"\"\"triple double\"\"\"", "triple double"),
            ("u'Unicode'", "Unicode"),
            ("R'raw\\n'", "raw\\n"),
        ] {
            assert_eq!(decode_python_string(source).as_deref(), Some(expected));
        }
        assert_eq!(decode_python_string("no quotes"), None);
        assert_eq!(decode_python_string("b'bytes'"), None);
        assert_eq!(decode_python_string("f'{value}'"), None);
        assert_eq!(decode_python_string("q'unknown prefix'"), None);
        assert_eq!(decode_python_string("'unterminated"), None);

        assert_eq!(
            python_literal_for("('left' 'right')"),
            Some(PythonLiteral::Value(ParameterValue::String(
                "leftright".to_owned()
            )))
        );
        assert_eq!(python_literal_for("('left' f'{value}')"), None);
        assert_eq!(python_literal_for("None"), Some(PythonLiteral::None));
        assert_eq!(python_literal_for("..."), Some(PythonLiteral::Ellipsis));
        assert_eq!(python_literal_value_for("None"), None);
        assert_eq!(python_literal_value_for("..."), None);
        assert_eq!(python_literal_for("[1]"), None);
        assert_eq!(python_string_for("'text'"), Some("text".to_owned()));
        assert_eq!(python_string_for("1"), None);
        assert_eq!(python_bool_for("True"), Some(true));
        assert_eq!(python_bool_for("'true'"), None);
        assert_eq!(literal_as_string(&PythonLiteral::None), None);
        assert_eq!(literal_as_string(&PythonLiteral::Ellipsis), None);
        assert_eq!(parameter_value_text(&ParameterValue::Float(3.5)), "3.5");
    }

    #[test]
    fn python_escape_decoding_covers_named_numeric_continuation_and_unknown_forms() {
        let source = concat!(
            "\\\\",
            "\\'",
            "\\\"",
            "\\a",
            "\\b",
            "\\f",
            "\\n",
            "\\r",
            "\\t",
            "\\v",
            "\\\n",
            "\\\r\n",
            "\\x41",
            "\\u4e2d",
            "\\U0001F680",
            "\\101",
            "\\q",
        );
        assert_eq!(
            decode_python_escapes(source).as_deref(),
            Some("\\'\"\u{7}\u{8}\u{c}\n\r\t\u{b}A中🚀A\\q")
        );
        assert_eq!(decode_python_escapes("trailing\\"), None);
        assert_eq!(decode_python_escapes("\\xG0"), None);
        assert_eq!(decode_python_escapes("\\u123"), None);
        assert_eq!(decode_python_escapes("\\U00110000"), None);

        let mut short = "a".chars().peekable();
        assert_eq!(read_radix(&mut short, 2, 16), None);
        let mut invalid = "gg".chars().peekable();
        assert_eq!(read_radix(&mut invalid, 2, 16), None);
    }

    #[test]
    fn python_input_binding_targets_cover_comprehensions_decorators_and_attributes() {
        for source in [
            "values = [input() for input in items]\n",
            "def outer(*input, **rest):\n    return input()\n",
            "class input:\n    pass\ninput()\n",
            "@decorator\ndef input():\n    pass\ninput()\n",
            "for input in values:\n    pass\ninput()\n",
            "(input := factory())\ninput()\n",
        ] {
            let ParseOutcome::Parsed(document) = parse_document("python", source) else {
                panic!("the binding fixture must parse");
            };
            assert!(
                document
                    .analysis()
                    .candidates
                    .iter()
                    .all(|candidate| candidate.declaration.binding != ParameterBinding::Input),
                "{source}"
            );
        }

        for source in [
            "obj.input = replacement\ninput()\n",
            "values[input] = replacement\ninput()\n",
        ] {
            let ParseOutcome::Parsed(document) = parse_document("python", source) else {
                panic!("the nonbinding fixture must parse");
            };
            assert_eq!(
                document
                    .analysis()
                    .candidates
                    .iter()
                    .filter(|candidate| candidate.declaration.binding == ParameterBinding::Input)
                    .count(),
                1,
                "{source}"
            );
        }
    }

    #[test]
    fn python_constant_binding_counts_distinguish_real_targets_from_attribute_reads() {
        let source = concat!(
            "CLEAN = 1\n",
            "obj.CLEAN = 2\n",
            "items[CLEAN] = 3\n",
            "LAMBDA = 4\n",
            "callback = lambda LAMBDA: LAMBDA\n",
            "STAR = 5\n",
            "KW = 6\n",
            "def consume(*STAR, **KW):\n    return STAR, KW\n",
        );
        let ParseOutcome::Parsed(document) = parse_document("python", source) else {
            panic!("the constant fixture must parse");
        };

        let environment = constant_environment(&document);
        assert_eq!(environment.get("CLEAN"), Some(&ParameterValue::Integer(1)));
        assert!(!environment.contains_key("LAMBDA"));
        assert!(!environment.contains_key("STAR"));
        assert!(!environment.contains_key("KW"));
    }

    #[test]
    fn typed_python_strings_escape_tabs_and_unicode_line_boundaries() {
        let declaration = ParamDecl::new("VALUE");
        assert_eq!(
            typed_python_literal(&declaration, "left\tright\u{2029}tail").unwrap(),
            "'left\\tright\\u2029tail'"
        );
    }

    #[test]
    fn test_const_default_that_no_longer_fits_the_declared_type_is_not_published() {
        let mut stored = ParamDecl::new("N");
        stored.binding = ParameterBinding::Const;
        stored.delivery = ParameterDelivery::Inject;
        stored.parameter_type = ParameterType::Int;
        stored.default = Some(ParameterValue::Integer(3));

        let mut current = stored.clone();
        current.default = Some(ParameterValue::String("three".to_owned()));
        let analysis = SemanticAnalysis {
            candidates: vec![SemanticCandidate {
                declaration: current,
                identity: BindingIdentity {
                    binding: ParameterBinding::Const,
                    key: "N".to_owned(),
                    occurrence: 0,
                    scope: Vec::new(),
                },
                span: SourceSpan {
                    start: 0,
                    end: 1,
                    start_line: 1,
                    end_line: 1,
                },
                demotion: None,
                empty_uses_default: false,
            }],
            ..SemanticAnalysis::default()
        };

        let report = reconcile_analysis(&analysis, &[stored]);

        assert_eq!(
            report
                .ok
                .iter()
                .map(|pair| pair.stored.name.as_str())
                .collect::<Vec<_>>(),
            ["N"]
        );
        assert!(report.current_defaults.is_empty());
    }
}
