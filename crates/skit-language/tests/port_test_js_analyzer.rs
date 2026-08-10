//! Mechanical port of the Python oracle module `tests/test_js_analyzer.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it traces back to
//! its origin, and the Python "WHY" comment is preserved verbatim above it.
//!
//! Concept mapping used throughout:
//! - Python `js.analyze(src, lang=L).candidates[i]` -> `parse_document(L, src).analysis()
//!   .candidates[i]` (a `SemanticCandidate`); `c.name/type/default/secret` read from
//!   `c.declaration`, Python `c.lineno` -> `c.span.start_line`, and Python `c.demoted`/`c.demotion`
//!   map to the candidate-level `c.demotion: Option<DegradationReason>` (present => demoted,
//!   `Accumulator` <=> Python `"accumulator"`).
//! - Python `js.analyze(src).syntax_error is True` -> `parse_document` returns
//!   `ParseOutcome::SyntaxError`; `syntax_error is False` -> `ParseOutcome::Parsed`.
//! - Python `js.reconcile(text, specs, lang=L)` -> the `reconcile` helper below (parse then
//!   `ParsedDocument::reconcile`, with the conservative all-missing report on a syntax error).
//! - Python `js_io.write_params(src, specs)` -> `write_managed_params(kind, src, &specs)` and
//!   `js_io.read_params(out)` -> `managed_params(kind, out)`. The `//` block leader is identical
//!   for the `js` and `ts` kinds, so the ts-file round-trip threads `"ts"` and the rest `"js"`.
//! - Python `cli_reader.read_cli(src, lang=L)` -> `parse_document(L, src).cli_surface()`. A Python
//!   `None` result maps to `CliSurface::Absent` (no parseArgs) or, on a source that does not parse,
//!   to `ParseOutcome::SyntaxError`. A returned spec with `.ok`/`.fields` maps to
//!   `CliSurface::Static`; `.ok == False` with a `.reason == "dynamic"` maps to
//!   `CliSurface::Dynamic { reason: DegradationReason::DynamicDeclaration }`. A field's
//!   `.name/.flag/.type/.action/.default/.multiple/.repeat/.secret/.degraded` read from
//!   `field.declaration`.
//!
//! Bucket 3 (registry dynamic-import degradation and `skit params` CLI / `flows` / `store`
//! integration that lives above `skit-language`) is kept as compiling `#[ignore]` stubs with the
//! WHY comment. `#[ignore]` is used ONLY for those off-crate tests; every analyzer/surface/block
//! test is a live bucket-1 assertion.

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, DegradationReason, ParseOutcome, ParsedDocument, ReconcileReport, SemanticCandidate,
    SemanticField, managed_params, parse_document, write_managed_params,
};

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected valid {kind}, got {other:?}"),
    }
}

/// Python `cands(src, lang=L)` = `js.analyze(src, lang=L).candidates`.
fn cands_lang(kind: &str, source: &str) -> Vec<SemanticCandidate> {
    parsed(kind, source).analysis().candidates
}

/// Python `cands(src)` (default `lang="js"`).
fn cands(source: &str) -> Vec<SemanticCandidate> {
    cands_lang("js", source)
}

/// Python `by_name(src, lang=L)` = `{c.name: c for c in cands(src, lang=L)}`.
fn by_name_lang(kind: &str, source: &str) -> BTreeMap<String, SemanticCandidate> {
    cands_lang(kind, source)
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate))
        .collect()
}

/// Python `by_name(src)` (default `lang="js"`).
fn by_name(source: &str) -> BTreeMap<String, SemanticCandidate> {
    by_name_lang("js", source)
}

/// Candidate names in source order.
fn names(source: &str) -> Vec<String> {
    cands(source)
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

/// Python `_demoted(src)` = `{c.name for c in cands(src) if c.demoted}`.
fn demoted(source: &str) -> BTreeSet<String> {
    cands(source)
        .into_iter()
        .filter(|candidate| candidate.demotion.is_some())
        .map(|candidate| candidate.declaration.name)
        .collect()
}

fn string_set<const N: usize>(values: [&str; N]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

// ---------------------------------------------------------------- const detection

#[test]
fn test_const_number_string_bool() {
    let b = by_name("const W = 800;\nconst N = \"hi\";\nconst T = true;\nconst F = false;\nconst R = 2.5;\n");
    assert_eq!(
        (b["W"].declaration.parameter_type, &b["W"].declaration.default),
        (ParameterType::Int, &Some(ParameterValue::Integer(800)))
    );
    assert_eq!(
        (b["N"].declaration.parameter_type, &b["N"].declaration.default),
        (ParameterType::Str, &Some(ParameterValue::String("hi".to_owned())))
    );
    assert_eq!(
        (b["T"].declaration.parameter_type, &b["T"].declaration.default),
        (ParameterType::Bool, &Some(ParameterValue::Bool(true)))
    );
    assert_eq!(
        (b["F"].declaration.parameter_type, &b["F"].declaration.default),
        (ParameterType::Bool, &Some(ParameterValue::Bool(false)))
    );
    assert_eq!(
        (b["R"].declaration.parameter_type, &b["R"].declaration.default),
        (ParameterType::Float, &Some(ParameterValue::Float(2.5)))
    );
}

#[test]
fn test_template_string_excluded() {
    // A template string may interpolate — never a candidate, even without a `${...}`.
    assert!(cands("const A = `hi ${x}`;\nconst B = `plain`;\n").is_empty());
}

#[test]
fn test_object_and_array_excluded() {
    assert!(cands("const O = {a: 1};\nconst A = [1, 2];\n").is_empty());
}

#[test]
fn test_destructuring_excluded() {
    // object_pattern / array_pattern names are not plain identifiers.
    assert!(cands("const {p, q} = obj;\nconst [m, n] = arr;\n").is_empty());
}

#[test]
fn test_bare_declaration_without_value_skipped() {
    // `let x;` has a declarator but no value node — skipped; the const still lands.
    assert_eq!(names("let x;\nconst Y = 5;\n"), ["Y"]);
}

#[test]
fn test_leading_underscore_skipped() {
    assert_eq!(names("const _HIDDEN = 1;\nconst SHOWN = 2;\n"), ["SHOWN"]);
}

#[test]
fn test_last_write_wins_keeps_first_slot() {
    let b = by_name("const X = 1;\nconst Y = 5;\nconst X = 2;\n");
    assert_eq!(b["X"].declaration.default, Some(ParameterValue::Integer(2)));
    let names = names("const X = 1;\nconst Y = 5;\nconst X = 2;\n");
    let index_of = |target: &str| names.iter().position(|name| name == target).unwrap();
    assert!(index_of("X") < index_of("Y"));
}

#[test]
fn test_multiple_declarators_in_one_statement() {
    let b = by_name("const A = 1, B = 2;\n");
    assert_eq!(
        (&b["A"].declaration.default, &b["B"].declaration.default),
        (
            &Some(ParameterValue::Integer(1)),
            &Some(ParameterValue::Integer(2))
        )
    );
}

#[test]
fn test_comment_between_keyword_and_declarator_is_skipped() {
    // A comment is a named child of the declaration but not a declarator — skipped, const still lands.
    assert_eq!(names("const /* note */ X = 5;\n"), ["X"]);
}

#[test]
fn test_secret_by_name() {
    assert!(by_name("const API_KEY = \"x\";\n")["API_KEY"].declaration.secret);
}

#[test]
fn test_lineno_recorded() {
    // Python `c.lineno == 3` -> the candidate's one-based `span.start_line`.
    let all = cands("\n\nconst X = 5;\n");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].span.start_line, 3);
}

// ---------------------------------------------------------------- demotions

#[test]
fn test_let_and_var_demoted() {
    let b = by_name("let A = 1;\nvar B = 2;\n");
    assert_eq!(b["A"].demotion, Some(DegradationReason::Accumulator));
    assert_eq!(b["B"].demotion, Some(DegradationReason::Accumulator));
}

#[test]
fn test_const_reassigned_is_demoted() {
    assert_eq!(demoted("const C = 1;\nC = 2;\n"), string_set(["C"]));
}

#[test]
fn test_const_augmented_assign_is_demoted() {
    assert_eq!(demoted("const N = 0;\nN += 5;\n"), string_set(["N"]));
}

#[test]
fn test_const_update_expression_is_demoted() {
    assert_eq!(demoted("const N = 0;\nN++;\n"), string_set(["N"]));
}

#[test]
fn test_plain_const_not_demoted() {
    // Python `(c.demoted, c.demotion) == (False, "")` -> no candidate-level demotion reason.
    let all = cands("const STABLE = 7;\n");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].demotion, None);
}

#[test]
fn test_member_reassignment_does_not_demote() {
    // `obj.x = …` reassigns a property, not the top-level binding.
    assert_eq!(demoted("const CFG = 1;\nglobalThis.CFG = 2;\n"), BTreeSet::new());
}

// ---------------------------------------------------------------- type inference

#[test]
fn test_negative_int_is_a_unary_expression_not_a_number_literal() {
    // A leading `-` makes the value a unary_expression, which is NOT in the literal node set — so a
    // negative numeric const is (deliberately, per the design) not offered. Documented limitation.
    assert!(cands("const N = -3;\n").is_empty());
}

#[test]
fn test_exotic_number_literals_are_float_with_source_text_default() {
    let b = by_name("const H = 0xFF;\nconst E = 1e3;\nconst G = 100n;\n");
    assert_eq!(
        (b["H"].declaration.parameter_type, &b["H"].declaration.default),
        (ParameterType::Float, &Some(ParameterValue::String("0xFF".to_owned())))
    );
    assert_eq!(
        (b["E"].declaration.parameter_type, &b["E"].declaration.default),
        (ParameterType::Float, &Some(ParameterValue::String("1e3".to_owned())))
    );
    assert_eq!(
        (b["G"].declaration.parameter_type, &b["G"].declaration.default),
        (ParameterType::Float, &Some(ParameterValue::String("100n".to_owned())))
    );
}

#[test]
fn test_simple_decimal_float() {
    let all = cands("const R = 3.25;\n");
    assert_eq!(all.len(), 1);
    assert_eq!(
        (all[0].declaration.parameter_type, &all[0].declaration.default),
        (ParameterType::Float, &Some(ParameterValue::Float(3.25)))
    );
}

#[test]
fn test_empty_and_escaped_string_values() {
    let b = by_name("const E = \"\";\nconst X = \"a\\\"b\\n\";\n");
    assert_eq!(b["E"].declaration.default, Some(ParameterValue::String(String::new())));
    // fragments + escape sequences, raw
    assert_eq!(
        b["X"].declaration.default,
        Some(ParameterValue::String("a\\\"b\\n".to_owned()))
    );
}

// ---------------------------------------------------------------- TypeScript grammar

#[test]
fn test_ts_annotation_value_still_found() {
    let b = by_name_lang("ts", "const N: number = 5;\nconst S: string = \"x\";\n");
    assert_eq!(
        (b["N"].declaration.parameter_type, &b["N"].declaration.default),
        (ParameterType::Int, &Some(ParameterValue::Integer(5)))
    );
    assert_eq!(
        (b["S"].declaration.parameter_type, &b["S"].declaration.default),
        (ParameterType::Str, &Some(ParameterValue::String("x".to_owned())))
    );
}

#[test]
fn test_ts_only_constructs_parse_under_the_typescript_grammar() {
    let src = "interface I { a: number }\ntype T = number;\nenum E { A }\nconst X: number = 5;\n";
    assert_eq!(
        cands_lang("ts", src)
            .into_iter()
            .map(|candidate| candidate.declaration.name)
            .collect::<Vec<_>>(),
        ["X"]
    );
}

#[test]
fn test_js_grammar_errors_on_typescript_only_syntax() {
    // The js kind must NOT silently parse TS-only syntax — it degrades honestly.
    assert!(matches!(
        parse_document("js", "enum E { A }\nconst X = 5;\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn test_tsx_grammar_branch() {
    // The tsx dialect parses JSX; injected here only to exercise the language resolver's tsx branch.
    // Faithful mapping threads the exact `lang="tsx"` kind to `parse_document`.
    let outcome = parse_document("tsx", "const X = 5;\nconst e = <div/>;\n");
    let ParseOutcome::Parsed(document) = outcome else {
        panic!(
            "FINDING: parse_document has no `tsx` kind; Python js.analyze(lang=\"tsx\") resolves the \
             tsx grammar, parses JSX, and yields the `X` const. Got {outcome:?}"
        );
    };
    assert_eq!(
        document
            .analysis()
            .candidates
            .into_iter()
            .map(|candidate| candidate.declaration.name)
            .collect::<Vec<_>>(),
        ["X"]
    );
}

// UNMAPPED (supervisor adjudication): the oracle's `language_for` (`_LANGUAGES.get(lang, _JS)`) is a
// JS-analyzer-internal totality default -- an unknown `lang` falls back to the javascript grammar.
// The Python pragma marks it a mutation-gate/totality fixture ("maps js/JS/XXjsXX all to _JS"), not a
// product behavior: no entry resolution ever produces an unknown JS-family lang. The Rust equivalent
// is `parse_document`, the TOP-LEVEL kind dispatcher, which intentionally rejects an unknown kind (a
// "pythn" typo must not silently parse as JS) -- so this internal default has no faithful Rust home
// and is not a user-facing behavior. Kept as a compiling stub, not a hidden mismatch.
#[test]
#[ignore = "UNMAPPED: JS-analyzer-internal lang totality (language_for's _JS default); the Rust top-level parse_document rejects unknown kinds by design and no product path yields an unknown JS-family lang"]
fn test_unknown_lang_falls_back_to_javascript() {
    let _ = parse_document;
}

// ---------------------------------------------------------------- degradation

#[test]
fn test_has_error_returns_empty_syntax_error() {
    // Python exposes `result.syntax_error is True` and an empty candidate list. The Rust surface
    // reports invalid syntax as a distinct `ParseOutcome::SyntaxError` variant (no parsed document,
    // hence no candidates), which is the faithful mapping of both assertions.
    assert!(matches!(
        parse_document("js", "const X = ;\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn test_empty_script() {
    // Python: `result.candidates == []` and `result.syntax_error is False`.
    assert!(cands("").is_empty());
    assert!(matches!(parse_document("js", ""), ParseOutcome::Parsed(_)));
}

// ---------------------------------------------------------------- reconcile

/// Python `js.reconcile(text, specs, lang=L)`: parse then reconcile, or the conservative
/// all-missing report when the source has a syntax error.
fn reconcile(kind: &str, source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

/// Python `ParamDecl(name=name, binding="const", type=type)`.
fn const_spec(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.parameter_type = parameter_type;
    declaration
}

#[test]
fn test_reconcile_const_ok() {
    let report = reconcile("js", "const CITY = 800;\n", &[const_spec("CITY", ParameterType::Int)]);
    assert!(!report.has_drift());
    assert_eq!(
        report.ok.iter().map(|pair| pair.stored.name.clone()).collect::<Vec<_>>(),
        ["CITY"]
    );
}

#[test]
fn test_reconcile_const_gone_is_missing() {
    let report = reconcile("js", "const OTHER = 1;\n", &[const_spec("CITY", ParameterType::Int)]);
    assert!(report.has_drift());
    assert_eq!(
        report.missing.iter().map(|declaration| declaration.name.clone()).collect::<Vec<_>>(),
        ["CITY"]
    );
}

#[test]
fn test_reconcile_type_change_is_flagged() {
    let report = reconcile("js", "const N = \"text\";\n", &[const_spec("N", ParameterType::Int)]);
    assert_eq!(
        report.changed.iter().map(|pair| pair.stored.name.clone()).collect::<Vec<_>>(),
        ["N"]
    );
}

#[test]
fn test_reconcile_ts_lang_threaded() {
    // A TS-only file must reconcile under the TS grammar (js grammar would report a syntax error and
    // mark everything missing).
    let src = "interface I { a: number }\nconst X: number = 5;\n";
    let report = reconcile("ts", src, &[const_spec("X", ParameterType::Int)]);
    assert!(!report.has_drift());
}

// ---------------------------------------------------------------- the // block engine

/// Python `ParamDecl(name=name, binding="const", delivery="inject", type="int", default=default)`.
fn inject_int_spec(name: &str, default: i64) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = ParameterType::Int;
    declaration.default = Some(ParameterValue::Integer(default));
    declaration
}

#[test]
fn test_block_roundtrip_on_ts_file() {
    let specs = vec![inject_int_spec("N", 5)];
    let src = "const N: number = 5;\nconsole.log(N);\n";
    let out = write_managed_params("ts", src, &specs).unwrap();
    assert!(out.contains("// [tool.skit]"));
    assert!(out.contains("name = \"N\""));
    assert_eq!(managed_params("ts", &out), specs);
    assert!(out.contains("const N: number = 5;\nconsole.log(N);\n")); // code bytes untouched
}

#[test]
fn test_block_lands_after_a_node_shebang() {
    let specs = vec![inject_int_spec("P", 1)];
    let src = "#!/usr/bin/env node\nconst P = 1;\n";
    let out = write_managed_params("js", src, &specs).unwrap();
    assert!(out.starts_with("#!/usr/bin/env node\n"));
    assert!(out.find("#!").unwrap() < out.find("// /// script").unwrap());
}

#[test]
fn test_block_at_top_when_no_shebang() {
    let specs = vec![inject_int_spec("P", 1)];
    let out = write_managed_params("js", "const P = 1;\n", &specs).unwrap();
    assert!(out.starts_with("// /// script\n"));
}

#[test]
fn test_write_empty_params_is_identity() {
    assert_eq!(write_managed_params("js", "const P = 1;\n", &[]).unwrap(), "const P = 1;\n");
}

// ---------------------------------------------------------------- parseArgs reader

/// Python `read(src, lang=L)` = `cli_reader.read_cli(src, lang=L)` (surface half).
fn surface(kind: &str, source: &str) -> CliSurface {
    parsed(kind, source).cli_surface()
}

/// Fields of a static surface (Python `read(src).fields` for an `.ok` spec).
fn fields(kind: &str, source: &str) -> Vec<SemanticField> {
    match surface(kind, source) {
        CliSurface::Static(static_surface) => static_surface.fields,
        other => panic!("expected a static parseArgs surface, got {other:?}"),
    }
}

/// The one field of a single-field surface (Python `(f,) = read(src).fields`).
fn only_field(kind: &str, source: &str) -> SemanticField {
    let mut all = fields(kind, source);
    assert_eq!(all.len(), 1, "expected exactly one field");
    all.pop().expect("length checked")
}

fn field<'a>(fields: &'a [SemanticField], name: &str) -> &'a SemanticField {
    fields
        .iter()
        .find(|field| field.declaration.name == name)
        .unwrap_or_else(|| panic!("no field named {name}"))
}

fn field_names(fields: &[SemanticField]) -> Vec<String> {
    fields
        .iter()
        .map(|field| field.declaration.name.clone())
        .collect()
}

#[test]
fn test_parseargs_util_member_inline_options() {
    let f = only_field("js", "const {values} = util.parseArgs({options:{name:{type:\"string\"}}});\n");
    assert_eq!(
        (
            f.declaration.name.as_str(),
            f.declaration.flag.as_str(),
            f.declaration.parameter_type
        ),
        ("name", "--name", ParameterType::Str)
    );
}

#[test]
fn test_parseargs_bare_call() {
    let f = only_field("js", "parseArgs({options:{x:{type:\"boolean\"}}});\n");
    assert_eq!(
        (
            f.declaration.parameter_type,
            f.declaration.action.as_str(),
            &f.declaration.default
        ),
        (ParameterType::Bool, "store_true", &Some(ParameterValue::Bool(false)))
    );
}

#[test]
fn test_parseargs_nested_member() {
    assert_eq!(
        field_names(&fields("js", "a.b.parseArgs({options:{x:{type:\"string\"}}});\n")),
        ["x"]
    );
}

#[test]
fn test_parseargs_all_option_features() {
    let src = concat!(
        "parseArgs({options:{",
        "name:{type:\"string\",short:\"n\",default:\"world\"},",
        "verbose:{type:\"boolean\"},",
        "tag:{type:\"string\",multiple:true},",
        "\"dry-run\":{type:\"boolean\",default:false}",
        "}});\n",
    );
    let all = fields("js", src);
    // short is display-only; the long flag is assembled
    assert_eq!(field(&all, "name").declaration.default, Some(ParameterValue::String("world".to_owned())));
    assert_eq!(field(&all, "name").declaration.flag, "--name");
    assert_eq!(
        (
            field(&all, "verbose").declaration.parameter_type,
            field(&all, "verbose").declaration.action.as_str()
        ),
        (ParameterType::Bool, "store_true")
    );
    assert!(field(&all, "tag").declaration.multiple);
    // parseArgs `multiple: true` collects one value per occurrence, so assembly must repeat the
    // flag (`--tag a --tag b`); repeat records that. A non-multiple option keeps repeat False.
    assert!(field(&all, "tag").declaration.repeat);
    assert!(!field(&all, "verbose").declaration.repeat);
    assert_eq!(field(&all, "dry-run").declaration.default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_parseargs_boolean_default_true_applies_literally() {
    let f = only_field("js", "parseArgs({options:{force:{type:\"boolean\",default:true}}});\n");
    assert_eq!(
        (f.declaration.parameter_type, &f.declaration.default),
        (ParameterType::Bool, &Some(ParameterValue::Bool(true)))
    );
}

#[test]
fn test_parseargs_string_key_option() {
    let f = only_field("js", "parseArgs({options:{\"dry-run\":{type:\"boolean\"}}});\n");
    assert_eq!(
        (f.declaration.name.as_str(), f.declaration.flag.as_str()),
        ("dry-run", "--dry-run")
    );
}

#[test]
fn test_parseargs_secret_option_name() {
    let f = only_field("js", "parseArgs({options:{token:{type:\"string\"}}});\n");
    assert!(f.declaration.secret);
}

// ---- degrade / skip paths -------------------------------------------------------

#[test]
fn test_parseargs_identifier_options_whole_spec_degrade() {
    // Python `(spec.ok, spec.reason) == (False, "dynamic")`.
    let CliSurface::Dynamic(dynamic) = surface("js", "parseArgs({options: opts});\n") else {
        panic!("identifier options must degrade the whole spec");
    };
    assert_eq!(dynamic.reason, DegradationReason::DynamicDeclaration);
}

#[test]
fn test_parseargs_spread_in_options_whole_spec_degrade() {
    let CliSurface::Dynamic(dynamic) =
        surface("js", "parseArgs({options:{...common, name:{type:\"string\"}}});\n")
    else {
        panic!("a spread in options must degrade the whole spec");
    };
    assert_eq!(dynamic.reason, DegradationReason::DynamicDeclaration);
}

#[test]
fn test_parseargs_computed_key_skips_just_that_field() {
    let src = "parseArgs({options:{[dyn]:{type:\"string\"}, name:{type:\"string\"}}});\n";
    assert_eq!(field_names(&fields("js", src)), ["name"]);
}

#[test]
fn test_parseargs_empty_string_key_is_skipped() {
    let src = "parseArgs({options:{\"\":{type:\"string\"}, ok:{type:\"string\"}}});\n";
    assert_eq!(field_names(&fields("js", src)), ["ok"]);
}

#[test]
fn test_parseargs_non_object_option_value_degrades_field() {
    let f = only_field("js", "parseArgs({options:{name: someVar}});\n");
    assert_eq!((f.declaration.name.as_str(), f.declaration.degraded), ("name", true));
}

#[test]
fn test_parseargs_unknown_type_string_degrades_field() {
    let f = only_field("js", "parseArgs({options:{n:{type:\"integer\"}}});\n");
    assert!(f.declaration.degraded);
}

#[test]
fn test_parseargs_non_literal_type_value_degrades_field() {
    let f = only_field("js", "parseArgs({options:{n:{type: someType}}});\n");
    assert!(f.declaration.degraded);
}

#[test]
fn test_parseargs_non_literal_default_degrades_field() {
    let f = only_field("js", "parseArgs({options:{n:{type:\"string\", default: fallback}}});\n");
    assert!(f.declaration.degraded);
}

#[test]
fn test_parseargs_ignores_spread_computed_and_numeric_keys_in_spec() {
    // A spread, a computed key, and a numeric key inside the option-spec object are read and skipped,
    // not crashed on — only the real `type` pair is applied.
    let f = only_field("js", "parseArgs({options:{n:{type:\"string\", [dyn]: 1, 0: 2, ...rest}}});\n");
    assert_eq!(
        (f.declaration.name.as_str(), f.declaration.parameter_type),
        ("n", ParameterType::Str)
    );
}

#[test]
fn test_parseargs_option_spec_without_type_keeps_str_and_reads_default() {
    // No `type` key: the reader must skip the type application and still read a default.
    let f = only_field("js", "parseArgs({options:{n:{default:\"hi\"}}});\n");
    assert_eq!(
        (f.declaration.parameter_type, &f.declaration.default),
        (ParameterType::Str, &Some(ParameterValue::String("hi".to_owned())))
    );
}

#[test]
fn test_parseargs_shorthand_property_in_options_is_skipped() {
    // A shorthand property (`{name, real:{...}}`) isn't a pair — skipped, not crashed on.
    let f = only_field("js", "parseArgs({options:{shorthand, real:{type:\"string\"}}});\n");
    assert_eq!(f.declaration.name, "real");
}

#[test]
fn test_parseargs_finds_options_past_a_spread_and_another_key() {
    // A spread and a non-"options" pair sit before `options`: the reader scans past both.
    let src = "parseArgs({...base, allowPositionals: true, options:{n:{type:\"string\"}}});\n";
    assert_eq!(field_names(&fields("js", src)), ["n"]);
}

#[test]
fn test_parseargs_empty_options_object_is_a_readable_zero_field_surface() {
    // Python: `spec is not None`, `spec.ok`, `spec.fields == []` -> a static, empty-field surface.
    assert_eq!(fields("js", "parseArgs({options:{}});\n"), Vec::new());
}

#[test]
fn test_no_parseargs_surface_returns_none() {
    // A plain identifier call that isn't parseArgs (not an identifier match, not a member call).
    assert!(matches!(surface("js", "const x = 5;\nfoo(x);\n"), CliSurface::Absent));
}

#[test]
fn test_parseargs_member_call_that_is_not_parseargs_is_ignored() {
    assert!(matches!(
        surface("js", "console.log(\"x\");\nconst y = 5;\n"),
        CliSurface::Absent
    ));
}

#[test]
fn test_parseargs_with_no_config_object_returns_none() {
    assert!(matches!(surface("js", "parseArgs();\n"), CliSurface::Absent));
}

#[test]
fn test_parseargs_non_object_config_returns_none() {
    assert!(matches!(surface("js", "parseArgs(config);\n"), CliSurface::Absent));
}

#[test]
fn test_parseargs_config_without_options_key_returns_none() {
    assert!(matches!(
        surface("js", "parseArgs({allowPositionals: true});\n"),
        CliSurface::Absent
    ));
}

#[test]
fn test_reader_on_syntax_error_returns_none() {
    // Python `read` returns None because the source does not parse; the Rust reader has no document
    // to project, mapped by the distinct `ParseOutcome::SyntaxError` variant.
    assert!(matches!(
        parse_document("js", "const x = ;\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn test_reader_threads_lang_for_typescript() {
    let src = "interface I {}\nparseArgs({options:{n:{type:\"string\"}}});\n";
    assert_eq!(field_names(&fields("ts", src)), ["n"]);
}

// ---------------------------------------------------------------- registry import guard

#[test]
#[ignore = "UNMAPPED: Python registry dynamic-import degradation (monkeypatch sys.modules so `from skit.langs.javascript import analyzer` is None) has no Rust equivalent; the js analyzer, cli reader, and injector are statically linked into ParsedDocument -> Tier 4 registry"]
fn test_import_guard_degrades_analysis_capabilities_to_none() {
    // Python asserts registry.spec_for("js").{analyzer, cli_reader, injector} are None while
    // params_io stays present after the analyzer import is broken. Rust has no per-language optional
    // analyzer import to degrade; the `//` block engine (managed_params) has no grammar dependency.
    let _ = reconcile;
}

#[test]
#[ignore = "UNMAPPED: skit.flows.plan_for_entry + store.add_script + registry import degradation -> Tier 4 skit-cli/flows (not reachable from skit-language)"]
fn test_plan_degrades_to_none_when_analyzer_missing() {
    // Python: with the js analyzer import broken, flows.plan_for_entry(entry).source == "none" so the
    // entry stays launchable with no inject plan. flows/store live above skit-language.
    let _ = reconcile;
}

// ---------------------------------------------------------------- `skit params` integration

#[test]
#[ignore = "UNMAPPED: `skit params <name> --manage` CLI integration (CliRunner + store.add_script) -> Tier 4 skit-cli"]
fn test_params_manage_writes_block_into_js_copy() {
    // Python drives cli.app to write a `[tool.skit]` block after the shebang of a js copy. The block
    // engine itself is covered by test_block_lands_after_a_node_shebang; the CLI wiring is Tier 4.
    let _ = reconcile;
}

#[test]
#[ignore = "UNMAPPED: `skit params <name>` CLI show output (CliRunner + store.add_script) -> Tier 4 skit-cli"]
fn test_params_show_lists_ts_const() {
    // Python asserts the CLI show view for a .ts entry lists the const CITY. The analyzer half (the
    // ts const candidate) is covered by test_ts_annotation_value_still_found; the CLI is Tier 4.
    let _ = reconcile;
}
