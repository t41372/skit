//! Public-surface ports of Python v0.4 `tests/test_js_analyzer.py` at `main@206f9ef`.
//!
//! The two Python import-failure registry contracts have no Rust equivalent and are accounted
//! separately. Everything here goes through the real parser-owned semantic, reconcile, CLI-surface,
//! or managed-block API. Rust-only row splits use `rust_additive_*` and do not count as parity.

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, DegradationReason, ParseOutcome, ParsedDocument, ReconcileReport, SemanticAnalysis,
    managed_params, parse_document, write_managed_params,
};

fn parsed(kind: &str, source: &str) -> ParsedDocument {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected parsed {kind} source, got {other:?}"),
    }
}

fn analysis(kind: &str, source: &str) -> SemanticAnalysis {
    parsed(kind, source).analysis()
}

fn candidate_map(kind: &str, source: &str) -> BTreeMap<String, skit_language::SemanticCandidate> {
    analysis(kind, source)
        .candidates
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate))
        .collect()
}

fn candidate_names(kind: &str, source: &str) -> Vec<String> {
    analysis(kind, source)
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

fn const_decl(name: &str, parameter_type: ParameterType) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::Const;
    declaration.delivery = ParameterDelivery::Inject;
    declaration.parameter_type = parameter_type;
    declaration
}

fn reader(kind: &str, source: &str) -> Option<CliSurface> {
    match parse_document(kind, source) {
        ParseOutcome::Parsed(document) => match document.cli_surface() {
            CliSurface::Absent => None,
            surface => Some(surface),
        },
        ParseOutcome::SyntaxError(_) | ParseOutcome::ParserUnavailable(_) => None,
    }
}

fn reader_fields(kind: &str, source: &str) -> Vec<ParamDecl> {
    match reader(kind, source) {
        Some(CliSurface::Static(surface)) => {
            assert_eq!(surface.framework, "parseArgs");
            surface
                .fields
                .into_iter()
                .map(|field| field.declaration)
                .collect()
        }
        other => panic!("expected static parseArgs surface, got {other:?}"),
    }
}

fn field_map(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields.iter().map(|field| (field.name.as_str(), field)).collect()
}

fn names(fields: &[ParamDecl]) -> Vec<&str> {
    fields.iter().map(|field| field.name.as_str()).collect()
}

#[test]
fn test_const_number_string_bool() {
    let b = candidate_map(
        "js",
        "const W = 800;\nconst N = \"hi\";\nconst T = true;\nconst F = false;\nconst R = 2.5;\n",
    );
    assert_eq!(b["W"].declaration.parameter_type, ParameterType::Int);
    assert_eq!(b["W"].declaration.default, Some(ParameterValue::Integer(800)));
    assert_eq!(b["N"].declaration.parameter_type, ParameterType::Str);
    assert_eq!(b["N"].declaration.default, Some(ParameterValue::String("hi".to_owned())));
    assert_eq!(b["T"].declaration.parameter_type, ParameterType::Bool);
    assert_eq!(b["T"].declaration.default, Some(ParameterValue::Bool(true)));
    assert_eq!(b["F"].declaration.parameter_type, ParameterType::Bool);
    assert_eq!(b["F"].declaration.default, Some(ParameterValue::Bool(false)));
    assert_eq!(b["R"].declaration.parameter_type, ParameterType::Float);
    assert_eq!(b["R"].declaration.default, Some(ParameterValue::Float(2.5)));
}

#[test]
fn test_template_string_excluded() {
    assert!(candidate_names("js", "const A = `hi ${x}`;\nconst B = `plain`;\n").is_empty());
}

#[test]
fn test_object_and_array_excluded() {
    assert!(candidate_names("js", "const O = {a: 1};\nconst A = [1, 2];\n").is_empty());
}

#[test]
fn test_destructuring_excluded() {
    assert!(candidate_names("js", "const {p, q} = obj;\nconst [m, n] = arr;\n").is_empty());
}

#[test]
fn test_bare_declaration_without_value_skipped() {
    assert_eq!(candidate_names("js", "let x;\nconst Y = 5;\n"), ["Y"]);
}

#[test]
fn test_leading_underscore_skipped() {
    assert_eq!(candidate_names("js", "const _HIDDEN = 1;\nconst SHOWN = 2;\n"), ["SHOWN"]);
}

#[test]
fn test_last_write_wins_keeps_first_slot() {
    let source = "const X = 1;\nconst Y = 5;\nconst X = 2;\n";
    let b = candidate_map("js", source);
    assert_eq!(b["X"].declaration.default, Some(ParameterValue::Integer(2)));
    let names = candidate_names("js", source);
    assert!(names.iter().position(|name| name == "X") < names.iter().position(|name| name == "Y"));
}

#[test]
fn test_multiple_declarators_in_one_statement() {
    let b = candidate_map("js", "const A = 1, B = 2;\n");
    assert_eq!(b["A"].declaration.default, Some(ParameterValue::Integer(1)));
    assert_eq!(b["B"].declaration.default, Some(ParameterValue::Integer(2)));
}

#[test]
fn test_comment_between_keyword_and_declarator_is_skipped() {
    assert_eq!(candidate_names("js", "const /* note */ X = 5;\n"), ["X"]);
}

#[test]
fn test_secret_by_name() {
    assert!(candidate_map("js", "const API_KEY = \"x\";\n")["API_KEY"].declaration.secret);
}

#[test]
fn test_lineno_recorded() {
    let analysis = analysis("js", "\n\nconst X = 5;\n");
    let [candidate] = analysis.candidates.as_slice() else { panic!("expected one candidate") };
    assert_eq!(candidate.span.start_line, 3);
}

fn demoted(source: &str) -> BTreeSet<String> {
    analysis("js", source)
        .candidates
        .into_iter()
        .filter(|candidate| candidate.demotion.is_some())
        .map(|candidate| candidate.declaration.name)
        .collect()
}

#[test]
fn test_let_and_var_demoted() {
    let b = candidate_map("js", "let A = 1;\nvar B = 2;\n");
    assert_eq!(b["A"].demotion, Some(DegradationReason::Accumulator));
    assert_eq!(b["B"].demotion, Some(DegradationReason::Accumulator));
}

#[test]
fn test_const_reassigned_is_demoted() {
    assert_eq!(demoted("const C = 1;\nC = 2;\n"), ["C".to_owned()].into_iter().collect());
}

#[test]
fn test_const_augmented_assign_is_demoted() {
    assert_eq!(demoted("const N = 0;\nN += 5;\n"), ["N".to_owned()].into_iter().collect());
}

#[test]
fn test_const_update_expression_is_demoted() {
    assert_eq!(demoted("const N = 0;\nN++;\n"), ["N".to_owned()].into_iter().collect());
}

#[test]
fn test_plain_const_not_demoted() {
    let analysis = analysis("js", "const STABLE = 7;\n");
    let [candidate] = analysis.candidates.as_slice() else { panic!("expected one candidate") };
    assert_eq!(candidate.demotion, None);
}

#[test]
fn test_member_reassignment_does_not_demote() {
    assert!(demoted("const CFG = 1;\nglobalThis.CFG = 2;\n").is_empty());
}

#[test]
fn test_negative_int_is_a_unary_expression_not_a_number_literal() {
    assert!(candidate_names("js", "const N = -3;\n").is_empty());
}

#[test]
fn test_exotic_number_literals_are_float_with_source_text_default() {
    let b = candidate_map("js", "const H = 0xFF;\nconst E = 1e3;\nconst G = 100n;\n");
    for (name, raw) in [("H", "0xFF"), ("E", "1e3"), ("G", "100n")] {
        assert_eq!(b[name].declaration.parameter_type, ParameterType::Float);
        assert_eq!(
            b[name].declaration.default,
            Some(ParameterValue::String(raw.to_owned()))
        );
    }
}

#[test]
fn rust_additive_js_hex_literal_keeps_source_text() {
    let b = candidate_map("js", "const H = 0xFF;\n");
    assert_eq!(b["H"].declaration.default, Some(ParameterValue::String("0xFF".to_owned())));
}
#[test]
fn rust_additive_js_exponent_literal_keeps_source_text() {
    let b = candidate_map("js", "const E = 1e3;\n");
    assert_eq!(b["E"].declaration.default, Some(ParameterValue::String("1e3".to_owned())));
}
#[test]
fn rust_additive_js_bigint_literal_keeps_source_text() {
    let b = candidate_map("js", "const G = 100n;\n");
    assert_eq!(b["G"].declaration.default, Some(ParameterValue::String("100n".to_owned())));
}

#[test]
fn test_simple_decimal_float() {
    let analysis = analysis("js", "const R = 3.25;\n");
    let [candidate] = analysis.candidates.as_slice() else { panic!("expected one candidate") };
    assert_eq!(candidate.declaration.parameter_type, ParameterType::Float);
    assert_eq!(candidate.declaration.default, Some(ParameterValue::Float(3.25)));
}

#[test]
fn test_empty_and_escaped_string_values() {
    let b = candidate_map("js", "const E = \"\";\nconst X = \"a\\\"b\\n\";\n");
    assert_eq!(b["E"].declaration.default, Some(ParameterValue::String(String::new())));
    assert_eq!(
        b["X"].declaration.default,
        Some(ParameterValue::String("a\\\"b\\n".to_owned()))
    );
}

#[test]
fn test_ts_annotation_value_still_found() {
    let b = candidate_map("ts", "const N: number = 5;\nconst S: string = \"x\";\n");
    assert_eq!(b["N"].declaration.parameter_type, ParameterType::Int);
    assert_eq!(b["N"].declaration.default, Some(ParameterValue::Integer(5)));
    assert_eq!(b["S"].declaration.parameter_type, ParameterType::Str);
    assert_eq!(b["S"].declaration.default, Some(ParameterValue::String("x".to_owned())));
}

#[test]
fn test_ts_only_constructs_parse_under_the_typescript_grammar() {
    let source = "interface I { a: number }\ntype T = number;\nenum E { A }\nconst X: number = 5;\n";
    assert_eq!(candidate_names("ts", source), ["X"]);
}

#[test]
fn test_js_grammar_errors_on_typescript_only_syntax() {
    assert!(matches!(
        parse_document("js", "enum E { A }\nconst X = 5;\n"),
        ParseOutcome::SyntaxError(_)
    ));
}

#[test]
fn test_tsx_grammar_branch() {
    let source = "const X = 5;\nconst e = <div/>;\n";
    let document = parsed("tsx", source);
    assert_eq!(
        document.analysis().candidates.into_iter().map(|c| c.declaration.name).collect::<Vec<_>>(),
        ["X"]
    );
}

#[test]
fn test_unknown_lang_falls_back_to_javascript() {
    assert_eq!(candidate_names("brainfuck", "const X = 5;\n"), ["X"]);
}

#[test]
fn test_has_error_returns_empty_syntax_error() {
    assert!(matches!(parse_document("js", "const X = ;\n"), ParseOutcome::SyntaxError(_)));
}

#[test]
fn test_empty_script() {
    let document = parsed("js", "");
    assert!(document.analysis().candidates.is_empty());
}

fn reconcile(kind: &str, source: &str, specs: &[ParamDecl]) -> ReconcileReport {
    parsed(kind, source).reconcile(specs)
}

#[test]
fn test_reconcile_const_ok() {
    let report = reconcile("js", "const CITY = 800;\n", &[const_decl("CITY", ParameterType::Int)]);
    assert!(!report.has_drift());
    assert_eq!(report.ok.iter().map(|pair| pair.stored.name.as_str()).collect::<Vec<_>>(), ["CITY"]);
}

#[test]
fn test_reconcile_const_gone_is_missing() {
    let report = reconcile("js", "const OTHER = 1;\n", &[const_decl("CITY", ParameterType::Int)]);
    assert!(report.has_drift());
    assert_eq!(report.missing.iter().map(|spec| spec.name.as_str()).collect::<Vec<_>>(), ["CITY"]);
}

#[test]
fn test_reconcile_type_change_is_flagged() {
    let report = reconcile("js", "const N = \"text\";\n", &[const_decl("N", ParameterType::Int)]);
    assert_eq!(report.changed.iter().map(|pair| pair.stored.name.as_str()).collect::<Vec<_>>(), ["N"]);
}

#[test]
fn test_reconcile_ts_lang_threaded() {
    let source = "interface I { a: number }\nconst X: number = 5;\n";
    let report = reconcile("ts", source, &[const_decl("X", ParameterType::Int)]);
    assert!(!report.has_drift());
}

fn managed_const(name: &str, parameter_type: ParameterType, default: ParameterValue) -> ParamDecl {
    let mut declaration = const_decl(name, parameter_type);
    declaration.default = Some(default);
    declaration
}

#[test]
fn test_block_roundtrip_on_ts_file() {
    let specs = [managed_const("N", ParameterType::Int, ParameterValue::Integer(5))];
    let source = "const N: number = 5;\nconsole.log(N);\n";
    let output = write_managed_params("ts", source, &specs).unwrap();
    assert!(output.contains("// [tool.skit]"), "{output}");
    assert!(output.contains("name = \"N\""), "{output}");
    assert_eq!(managed_params("ts", &output), specs);
    assert!(output.contains(source), "{output}");
}

#[test]
fn test_block_lands_after_a_node_shebang() {
    let specs = [managed_const("P", ParameterType::Int, ParameterValue::Integer(1))];
    let source = "#!/usr/bin/env node\nconst P = 1;\n";
    let output = write_managed_params("js", source, &specs).unwrap();
    assert!(output.starts_with("#!/usr/bin/env node\n"), "{output}");
    assert!(output.find("#!") < output.find("// /// script"));
}

#[test]
fn test_block_at_top_when_no_shebang() {
    let specs = [managed_const("P", ParameterType::Int, ParameterValue::Integer(1))];
    let output = write_managed_params("js", "const P = 1;\n", &specs).unwrap();
    assert!(output.starts_with("// /// script\n"), "{output}");
}

#[test]
fn test_write_empty_params_is_identity() {
    assert_eq!(
        write_managed_params("js", "const P = 1;\n", &[]).unwrap(),
        "const P = 1;\n"
    );
}

#[test]
fn test_parseargs_util_member_inline_options() {
    let actual = reader_fields("js", "const {values} = util.parseArgs({options:{name:{type:\"string\"}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.name, "name");
    assert_eq!(field.flag, "--name");
    assert_eq!(field.parameter_type, ParameterType::Str);
}

#[test]
fn test_parseargs_bare_call() {
    let actual = reader_fields("js", "parseArgs({options:{x:{type:\"boolean\"}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.action, "store_true");
    assert_eq!(field.default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_parseargs_nested_member() {
    assert_eq!(names(&reader_fields("js", "a.b.parseArgs({options:{x:{type:\"string\"}}});\n")), ["x"]);
}

#[test]
fn test_parseargs_all_option_features() {
    let source = concat!(
        "parseArgs({options:{",
        "name:{type:\"string\",short:\"n\",default:\"world\"},",
        "verbose:{type:\"boolean\"},",
        "tag:{type:\"string\",multiple:true},",
        "\"dry-run\":{type:\"boolean\",default:false}",
        "}});\n",
    );
    let actual = reader_fields("js", source);
    let fields = field_map(&actual);
    assert_eq!(fields["name"].default, Some(ParameterValue::String("world".to_owned())));
    assert_eq!(fields["name"].flag, "--name");
    assert_eq!(fields["verbose"].parameter_type, ParameterType::Bool);
    assert_eq!(fields["verbose"].action, "store_true");
    assert!(fields["tag"].multiple);
    assert!(fields["tag"].repeat);
    assert!(!fields["verbose"].repeat);
    assert_eq!(fields["dry-run"].default, Some(ParameterValue::Bool(false)));
}

#[test]
fn test_parseargs_boolean_default_true_applies_literally() {
    let actual = reader_fields("js", "parseArgs({options:{force:{type:\"boolean\",default:true}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.parameter_type, ParameterType::Bool);
    assert_eq!(field.default, Some(ParameterValue::Bool(true)));
}

#[test]
fn test_parseargs_string_key_option() {
    let actual = reader_fields("js", "parseArgs({options:{\"dry-run\":{type:\"boolean\"}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.name, "dry-run");
    assert_eq!(field.flag, "--dry-run");
}

#[test]
fn test_parseargs_secret_option_name() {
    let actual = reader_fields("js", "parseArgs({options:{token:{type:\"string\"}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert!(field.secret);
}

fn assert_dynamic(source: &str) {
    match reader("js", source) {
        Some(CliSurface::Dynamic(surface)) => {
            assert_eq!(surface.framework, "parseArgs");
            assert_eq!(surface.reason, DegradationReason::DynamicDeclaration);
        }
        other => panic!("expected dynamic parseArgs surface, got {other:?}"),
    }
}

#[test]
fn test_parseargs_identifier_options_whole_spec_degrade() {
    assert_dynamic("parseArgs({options: opts});\n");
}

#[test]
fn test_parseargs_spread_in_options_whole_spec_degrade() {
    assert_dynamic("parseArgs({options:{...common, name:{type:\"string\"}}});\n");
}

#[test]
fn test_parseargs_computed_key_skips_just_that_field() {
    assert_eq!(
        names(&reader_fields("js", "parseArgs({options:{[dyn]:{type:\"string\"}, name:{type:\"string\"}}});\n")),
        ["name"]
    );
}

#[test]
fn test_parseargs_empty_string_key_is_skipped() {
    assert_eq!(
        names(&reader_fields("js", "parseArgs({options:{\"\":{type:\"string\"}, ok:{type:\"string\"}}});\n")),
        ["ok"]
    );
}

#[test]
fn test_parseargs_non_object_option_value_degrades_field() {
    let actual = reader_fields("js", "parseArgs({options:{name: someVar}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.name, "name");
    assert!(field.degraded);
}

#[test]
fn test_parseargs_unknown_type_string_degrades_field() {
    let actual = reader_fields("js", "parseArgs({options:{n:{type:\"integer\"}}});\n");
    assert!(actual[0].degraded);
}

#[test]
fn test_parseargs_non_literal_type_value_degrades_field() {
    let actual = reader_fields("js", "parseArgs({options:{n:{type: someType}}});\n");
    assert!(actual[0].degraded);
}

#[test]
fn test_parseargs_non_literal_default_degrades_field() {
    let actual = reader_fields("js", "parseArgs({options:{n:{type:\"string\", default: fallback}}});\n");
    assert!(actual[0].degraded);
}

#[test]
fn test_parseargs_ignores_spread_computed_and_numeric_keys_in_spec() {
    let actual = reader_fields("js", "parseArgs({options:{n:{type:\"string\", [dyn]: 1, 0: 2, ...rest}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.name, "n");
    assert_eq!(field.parameter_type, ParameterType::Str);
}

#[test]
fn test_parseargs_option_spec_without_type_keeps_str_and_reads_default() {
    let actual = reader_fields("js", "parseArgs({options:{n:{default:\"hi\"}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.parameter_type, ParameterType::Str);
    assert_eq!(field.default, Some(ParameterValue::String("hi".to_owned())));
}

#[test]
fn test_parseargs_shorthand_property_in_options_is_skipped() {
    let actual = reader_fields("js", "parseArgs({options:{shorthand, real:{type:\"string\"}}});\n");
    let [field] = actual.as_slice() else { panic!("expected one field") };
    assert_eq!(field.name, "real");
}

#[test]
fn test_parseargs_finds_options_past_a_spread_and_another_key() {
    assert_eq!(
        names(&reader_fields("js", "parseArgs({...base, allowPositionals: true, options:{n:{type:\"string\"}}});\n")),
        ["n"]
    );
}

#[test]
fn test_parseargs_empty_options_object_is_a_readable_zero_field_surface() {
    assert!(reader_fields("js", "parseArgs({options:{}});\n").is_empty());
}

#[test]
fn test_no_parseargs_surface_returns_none() {
    assert!(reader("js", "const x = 5;\nfoo(x);\n").is_none());
}

#[test]
fn test_parseargs_member_call_that_is_not_parseargs_is_ignored() {
    assert!(reader("js", "console.log(\"x\");\nconst y = 5;\n").is_none());
}

#[test]
fn test_parseargs_with_no_config_object_returns_none() {
    assert!(reader("js", "parseArgs();\n").is_none());
}

#[test]
fn test_parseargs_non_object_config_returns_none() {
    assert!(reader("js", "parseArgs(config);\n").is_none());
}

#[test]
fn test_parseargs_config_without_options_key_returns_none() {
    assert!(reader("js", "parseArgs({allowPositionals: true});\n").is_none());
}

#[test]
fn test_reader_on_syntax_error_returns_none() {
    assert!(reader("js", "const x = ;\n").is_none());
}

#[test]
fn test_reader_threads_lang_for_typescript() {
    let source = "interface I {}\nparseArgs({options:{n:{type:\"string\"}}});\n";
    assert_eq!(names(&reader_fields("ts", source)), ["n"]);
}
