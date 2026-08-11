//! Mechanical port of the Python oracle module `tests/test_fish.py`
//! (`origin/main@206f9ef`). Each `#[test]` keeps its Python `def test_*` name so it traces back to
//! its origin, and the Python "WHY" comment is preserved. Same input bytes, same expected output.
//!
//! ## Concept mapping (same as the shell/analyzer ports)
//! - Python `fa.analyze(src)` -> `parsed(src).analysis()` (a `SemanticAnalysis`); fish is TOTAL, so
//!   `parse_document("fish", ..)` always returns `ParseOutcome::Parsed` (there is no `syntax_error`).
//! - Python `cands(src)` = `{c.name: c for c in fa.analyze(src).candidates}` -> the `cands` helper,
//!   a `BTreeMap<String, SemanticCandidate>`. `c.name/type/default/binding/secret` read from
//!   `c.declaration`; Python `c.env_name` -> `c.declaration.env_var()`.
//! - Python `fa.analyze(src).uses_argv` / `.uses_self_location` -> the same-named `SemanticAnalysis`
//!   fields.
//! - Python `fa.reconcile(text, specs)` -> the `reconcile` helper (parse then
//!   `ParsedDocument::reconcile`). Python `Report.ok` is a list -> `ReconcileReport.ok` (a Vec, so
//!   `assert ok.ok` -> `!report.ok.is_empty()`); `Report.has_drift` -> `report.has_drift()`;
//!   `Report.missing` -> `report.missing`.
//! - The fish argparse reader `fc.read_cli(src)` -> `parsed(src).cli_surface()`. Python
//!   `read_cli(..) is None` -> `CliSurface::Absent`; a spec with fields (including the zero-field
//!   `argparse -- $argv`) -> `CliSurface::Static`; `ArgSpec(ok=False, reason="dynamic")` ->
//!   `CliSurface::Dynamic(_)`. Each `ArgSpec.fields[i]` -> a `SemanticField.declaration`; Python
//!   `f.flag/type/multiple/repeat/secret/action/degraded` read from the declaration.
//! - Python `metawriter.write_params` / `read_params` -> `write_managed_params` / `managed_params`
//!   (the `#`-leader in-file `[tool.skit]` block engine).
//!
//! ## Buckets
//! - **Bucket 1 (pure analyzer / surface / reconcile / round-trip byte-logic):** the bulk. Asserted
//!   directly on the public skit-language output. A bucket-1 test that fails on the reader's actual
//!   behavior stays FAILING — that is the highest-value signal (a candidate fish-reader gap).
//! - **Bucket 2 (execution claim the injected BYTES establish):** NONE. fish has no injector in v1
//!   (const/read injection deferred), so `test_fish.py` has no injection-execution test.
//! - **Bucket 3 (white-box Python scanner internals / CLI-runtime integration):** kept as compiling
//!   `#[ignore]` stubs with their WHY. The fish analyzer/reader in the rewrite is tree-sitter-backed
//!   and shares NONE of the Python hand scanner (`_tokenize`, `_statements`, `_logical_lines`,
//!   `_dequote`, `_strip_comment`, `_classify_set`, `_is_query`), so those unit tests have no
//!   faithful public-observable equivalent; their behavior is exercised through the bucket-1
//!   analyzer/reader tests instead. The CliRunner/flows/store and real-fish-spawn e2e tests live
//!   above skit-language.

use std::collections::{BTreeMap, BTreeSet};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, LanguageError, ParseOutcome, ParsedDocument, ReconcileReport, SemanticCandidate,
    managed_params, parse_document, write_managed_params,
};

// ---------------------------------------------------------------- helpers

fn parsed(source: &str) -> ParsedDocument {
    match parse_document("fish", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("fish is total; expected Parsed, got {other:?}"),
    }
}

/// Python `cands(src)` = `{c.name: c for c in fa.analyze(src).candidates}`.
fn cands(source: &str) -> BTreeMap<String, SemanticCandidate> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate))
        .collect()
}

/// Candidate names for one source (Python `set(cands(src))`).
fn detected_names(source: &str) -> BTreeSet<String> {
    parsed(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| candidate.declaration.name)
        .collect()
}

/// Python `read(src)` fields as a name-keyed declaration map (the `{f.name: f}` dict).
fn fields_by_name(source: &str) -> BTreeMap<String, ParamDecl> {
    surface_fields(source)
        .into_iter()
        .map(|declaration| (declaration.name.clone(), declaration))
        .collect()
}

/// Python `list(fields)` / `[f.name for f in spec.fields]`: field names in source order.
fn field_names(source: &str) -> Vec<String> {
    surface_fields(source)
        .into_iter()
        .map(|declaration| declaration.name)
        .collect()
}

/// The static argparse surface's field declarations, or a panic if the surface is not static
/// (Python `read(src)` first asserts `spec is not None`, i.e. a readable static surface).
fn surface_fields(source: &str) -> Vec<ParamDecl> {
    match parsed(source).cli_surface() {
        CliSurface::Static(surface) => surface
            .fields
            .into_iter()
            .map(|field| field.declaration)
            .collect(),
        other => panic!("expected a static argparse surface, got {other:?}"),
    }
}

/// Python `_spec(name)` for the reconcile matrix: an envdefault/env-delivery declaration.
fn env_default(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.binding = ParameterBinding::EnvDefault;
    declaration.delivery = ParameterDelivery::Env;
    declaration
}

/// Python `fa.reconcile(text, specs)`: parse then reconcile (fish is total, so it always parses).
fn reconcile(source: &str, stored: &[ParamDecl]) -> ReconcileReport {
    match parse_document("fish", source) {
        ParseOutcome::Parsed(document) => document.reconcile(stored),
        _ => ReconcileReport::from_syntax_error(stored),
    }
}

fn string_set<const N: usize>(values: [&str; N]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// Python `FISH_CORPUS = sorted((… / "corpus" / "fish").glob("*.fish"))`. The byte-exact corpus
/// files, embedded so the test is hermetic. `include_str!` preserves the bytes verbatim.
const FISH_CORPUS: [(&str, &str); 6] = [
    (
        "01_env_idioms.fish",
        include_str!("../../../tests/corpus/fish/01_env_idioms.fish"),
    ),
    (
        "02_argparse.fish",
        include_str!("../../../tests/corpus/fish/02_argparse.fish"),
    ),
    (
        "03_quoting.fish",
        include_str!("../../../tests/corpus/fish/03_quoting.fish"),
    ),
    (
        "04_block_nesting.fish",
        include_str!("../../../tests/corpus/fish/04_block_nesting.fish"),
    ),
    (
        "05_reads_and_consts.fish",
        include_str!("../../../tests/corpus/fish/05_reads_and_consts.fish"),
    ),
    (
        "06_cjk.fish",
        include_str!("../../../tests/corpus/fish/06_cjk.fish"),
    ),
];

// ---------------------------------------------------------------- env-default idiom

#[test]
fn test_oneline_idiom_int() {
    let map = cands("set -q PORT; or set PORT 8080\n");
    let c = &map["PORT"];
    assert_eq!(c.declaration.binding, ParameterBinding::EnvDefault);
    assert_eq!(c.declaration.parameter_type, ParameterType::Int);
    assert_eq!(c.declaration.default, Some(ParameterValue::Integer(8080)));
    assert_eq!(c.declaration.env_var(), "PORT");
}

#[test]
fn test_newline_continued_or() {
    // fish continues an `or` at the start of the next line — the same idiom, two lines.
    let map = cands("set -q PORT\nor set PORT 8080\n");
    assert_eq!(
        map["PORT"].declaration.default,
        Some(ParameterValue::Integer(8080))
    );
}

#[test]
fn test_float_and_string_defaults() {
    let map = cands("set -q RATE; or set RATE 2.5\nset -q REGION; or set REGION us-east-1\n");
    assert_eq!(map["RATE"].declaration.parameter_type, ParameterType::Float);
    assert_eq!(
        map["RATE"].declaration.default,
        Some(ParameterValue::Float(2.5))
    );
    assert_eq!(map["REGION"].declaration.parameter_type, ParameterType::Str);
    assert_eq!(
        map["REGION"].declaration.default,
        Some(ParameterValue::String("us-east-1".to_owned()))
    );
}

#[test]
fn test_guarded_set_may_carry_scope_flags() {
    // `or set -gx NAME v` still preserves an inherited value (the `or` only fires when unset).
    let map = cands("set -q LOG; or set -gx LOG /var/log\n");
    assert_eq!(
        map["LOG"].declaration.default,
        Some(ParameterValue::String("/var/log".to_owned()))
    );
}

#[test]
fn test_secret_name_flagged() {
    let map = cands("set -q API_TOKEN; or set API_TOKEN x\n");
    assert!(map["API_TOKEN"].declaration.secret);
}

#[test]
fn test_suppressed_by_plain_clobber_anywhere() {
    // A later unconditional `set PORT 9090` clobbers the inherited value → env would no-op.
    assert!(cands("set -q PORT; or set PORT 8080\nset PORT 9090\n").is_empty());
}

#[test]
fn test_clobber_before_the_idiom_also_suppresses() {
    assert!(cands("set PORT 9090\nset -q PORT; or set PORT 8080\n").is_empty());
}

#[test]
fn test_unrelated_clobber_does_not_suppress() {
    let map = cands("set OTHER 1\nset -q PORT; or set PORT 8080\n");
    assert!(map.contains_key("PORT"));
    assert!(!map.contains_key("OTHER"));
}

#[test]
fn test_underscore_name_skipped() {
    assert!(cands("set -q _P; or set _P 1\n").is_empty());
}

#[test]
fn test_first_occurrence_wins_on_duplicate_idiom() {
    let map = cands("set -q PORT; or set PORT 8080\nset -q PORT; or set PORT 1\n");
    // first occurrence's default
    assert_eq!(
        map["PORT"].declaration.default,
        Some(ParameterValue::Integer(8080))
    );
}

#[test]
fn test_query_without_following_set_is_not_a_candidate() {
    assert!(cands("set -q PORT\necho done\n").is_empty());
}

#[test]
fn test_query_with_no_name_is_ignored() {
    assert!(cands("set -q; or set PORT 8080\n").is_empty());
}

#[test]
fn test_conditional_set_without_value_is_not_a_candidate() {
    assert!(cands("set -q PORT; or set PORT\n").is_empty());
}

#[test]
fn test_mismatched_names_are_not_an_idiom() {
    assert!(cands("set -q PORT; or set OTHER 8080\n").is_empty());
}

#[test]
fn test_unconditional_set_after_query_is_not_an_idiom() {
    // `set -q X; set X 1` (no `or`) is a query then a plain clobber — not an env-default.
    assert!(cands("set -q X; set X 1\n").is_empty());
}

// ---------------------------------------------------------------- block depth

#[test]
fn test_idiom_inside_function_is_not_toplevel() {
    assert!(cands("function f\n  set -q P; or set P 1\nend\n").is_empty());
}

#[test]
fn test_idiom_inside_every_block_kind_is_ignored() {
    for opener in ["if true", "while true", "for x in 1", "begin", "switch $x"] {
        let source = format!("{opener}\n  set -q P; or set P 1\nend\n");
        assert!(cands(&source).is_empty(), "{opener}");
    }
}

#[test]
fn test_toplevel_after_a_closed_block_is_detected() {
    let map = cands("function f\n  echo hi\nend\nset -q P; or set P 1\n");
    assert_eq!(
        map["P"].declaration.default,
        Some(ParameterValue::Integer(1))
    );
}

#[test]
fn test_nested_clobber_does_not_suppress_toplevel_idiom() {
    // A plain `set P 9` inside a function must not suppress a top-level P env-default.
    let map = cands("set -q P; or set P 1\nfunction f\n  set P 9\nend\n");
    assert_eq!(
        map["P"].declaration.default,
        Some(ParameterValue::Integer(1))
    );
}

#[test]
fn test_stray_end_clamps_depth_at_zero() {
    // A leading `end` must not drive depth negative and hide a following top-level idiom.
    let map = cands("end\nset -q P; or set P 1\n");
    assert_eq!(
        map["P"].declaration.default,
        Some(ParameterValue::Integer(1))
    );
}

// ---------------------------------------------------------------- hints

#[test]
fn test_argv_hint() {
    assert!(parsed("echo $argv\n").analysis().uses_argv);
}

#[test]
fn test_self_location_hints() {
    assert!(
        parsed("set d (status dirname)\n")
            .analysis()
            .uses_self_location
    );
    assert!(
        parsed("set f (status filename)\n")
            .analysis()
            .uses_self_location
    );
    assert!(!parsed("echo hi\n").analysis().uses_self_location);
}

#[test]
fn test_hint_ignores_commented_argv() {
    assert!(!parsed("# uses $argv here\necho hi\n").analysis().uses_argv);
}

// ---------------------------------------------------------------- reconcile

#[test]
fn test_reconcile_ok_then_drift() {
    let specs = [env_default("PORT")];
    let ok = reconcile("set -q PORT; or set PORT 8080\n", &specs);
    assert!(!ok.ok.is_empty()); // Python `assert ok.ok`
    assert!(!ok.has_drift());
    let gone = reconcile("echo hi\n", &specs);
    assert_eq!(
        gone.missing
            .iter()
            .map(|declaration| declaration.name.clone())
            .collect::<Vec<_>>(),
        ["PORT"]
    );
}

// ---------------------------------------------------------------- tokenizer internals

#[test]
#[ignore = "UNMAPPED: white-box Python hand-scanner `fa._tokenize`. The rewrite's fish reader is tree-sitter-backed and has no tokenizer to unit-test; the tokenization contract (`;`/word splitting) is exercised through the bucket-1 env-idiom + argparse tests -> Tier 3 white-box"]
fn test_tokenize_semicolon_and_words() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._tokenize` (quotes hold separators). No tokenizer in the tree-sitter reader; covered publicly by 03_quoting.fish's `'hello; world'` in test_corpus_analyze_is_total_and_reads_back -> Tier 3 white-box"]
fn test_tokenize_quotes_hold_separators() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._tokenize` (escaped quote does not close). No tokenizer in the tree-sitter reader -> Tier 3 white-box"]
fn test_tokenize_escaped_quote_does_not_close() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._tokenize` (a `#` at a word boundary ends the line). No tokenizer in the tree-sitter reader; comment handling is grammar-native -> Tier 3 white-box"]
fn test_tokenize_comment_ends_line() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._tokenize` (`a#b` mid-word `#` is literal). No tokenizer in the tree-sitter reader; covered publicly by 03_quoting.fish's `done#notacomment` in the corpus round-trip -> Tier 3 white-box"]
fn test_tokenize_hash_midword_is_literal() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._tokenize` (backslash escape outside a quote). No tokenizer in the tree-sitter reader -> Tier 3 white-box"]
fn test_tokenize_backslash_escape_outside_quote() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._tokenize` totality on an unterminated quote. No tokenizer in the tree-sitter reader; fish reader totality is covered by test_corpus_analyze_is_total_and_reads_back -> Tier 3 white-box"]
fn test_tokenize_unterminated_quote_is_total() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._statements` (empty runs between `;;` dropped). No hand statement-splitter in the tree-sitter reader -> Tier 3 white-box"]
fn test_statements_drop_empty_runs_between_semicolons() {}

// ---------------------------------------------------------------- line continuation

#[test]
#[ignore = "UNMAPPED: white-box `fa._logical_lines` (join a trailing-backslash continuation). Line continuation is grammar-native in the tree-sitter reader, not a hand pre-pass -> Tier 3 white-box"]
fn test_logical_lines_join_continuation() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._logical_lines` (even backslashes are not a continuation). No hand line-joiner in the tree-sitter reader -> Tier 3 white-box"]
fn test_logical_lines_even_backslashes_are_not_a_continuation() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._logical_lines` (a trailing continuation flushes). No hand line-joiner in the tree-sitter reader -> Tier 3 white-box"]
fn test_logical_lines_trailing_continuation_flushes() {}

// ---------------------------------------------------------------- dequote internals

#[test]
#[ignore = "UNMAPPED: white-box `fa._dequote` (single-quote escapes `\\'`/`\\\\`, `\\n` literal). The tree-sitter reader's private `decode_fish_word` is the analogue; value decoding is exercised publicly by the guarded-set / corpus tests -> Tier 3 white-box"]
fn test_dequote_single_quote_escapes() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._dequote` (double-quote escapes `\\\"`/`\\$`, `\\n` literal). No public single-value dequote surface; covered indirectly by the corpus round-trip -> Tier 3 white-box"]
fn test_dequote_double_quote_escapes() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._dequote` (backslash outside a quote and at end-of-word). No public single-value dequote surface -> Tier 3 white-box"]
fn test_dequote_backslash_outside_and_at_end() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._dequote` totality on unterminated quotes. No public single-value dequote surface; reader totality covered by test_corpus_analyze_is_total_and_reads_back -> Tier 3 white-box"]
fn test_dequote_unterminated_quotes_are_total() {}

// ---------------------------------------------------------------- strip_comment internals

#[test]
#[ignore = "UNMAPPED: white-box `fa._strip_comment` (quote-aware trailing-comment removal). Comment stripping is grammar-native in the tree-sitter reader; its effect (comments ignored) is covered by test_hint_ignores_commented_argv -> Tier 3 white-box"]
fn test_strip_comment_paths() {}

// ---------------------------------------------------------------- classify_set / is_query

#[test]
#[ignore = "UNMAPPED: white-box `fa._classify_set` / `fa._SetStmt`. The rewrite's `classify_set` is private and returns a private `SetCommand`; its classification is exercised through every bucket-1 env-idiom + suppression test -> Tier 3 white-box"]
fn test_classify_set_matrix() {}

#[test]
#[ignore = "UNMAPPED: white-box `fa._is_query` (`-q`/`--query`, cluster `-gq`). Private in the rewrite's `classify_set`; the query-detection contract is exercised by the env-idiom detection tests -> Tier 3 white-box"]
fn test_is_query_matrix() {}

// ---------------------------------------------------------------- argparse reader

#[test]
fn test_argparse_short_long_and_valueless_bool() {
    let fields = fields_by_name("argparse 'h/help' 'v/verbose' -- $argv\n");
    assert_eq!(fields["help"].flag, "--help");
    assert_eq!(fields["help"].parameter_type, ParameterType::Bool);
    assert_eq!(fields["help"].action, "store_true");
    assert_eq!(fields["verbose"].parameter_type, ParameterType::Bool);
}

#[test]
fn test_argparse_value_suffixes() {
    let fields =
        fields_by_name("argparse 'n/name=' 'r/retries=?' 'f/file=+' 'g/glob=*' -- $argv\n");
    assert_eq!(fields["name"].parameter_type, ParameterType::Str);
    assert!(!fields["name"].multiple);
    assert_eq!(fields["retries"].parameter_type, ParameterType::Str); // optional attached value
    assert!(fields["file"].multiple);
    assert!(fields["glob"].multiple);
    // `=+`/`=*` are REPEAT grammar: fish's argparse wants `--file a --file b`, and the
    // one-flag-many-values shape would leave `b` as a stray positional.
    assert!(fields["file"].repeat);
    assert!(fields["glob"].repeat);
    assert!(!fields["name"].repeat);
}

#[test]
fn test_argparse_long_only_and_short_only() {
    let fields = fields_by_name("argparse 'dry-run' 'x' -- $argv\n");
    assert_eq!(fields["dry-run"].flag, "--dry-run"); // long name that contains a hyphen
    assert_eq!(fields["x"].flag, "-x"); // single-char short-only
}

#[test]
fn test_argparse_dummy_short_yields_long_only() {
    let fields = fields_by_name("argparse 'x-long' -- $argv\n");
    assert_eq!(fields["long"].flag, "--long");
}

#[test]
fn test_argparse_numeric_hash_degrades() {
    let fields = fields_by_name("argparse 'm#max' -- $argv\n");
    assert_eq!(fields["max"].flag, "--max");
    assert!(fields["max"].degraded);
}

#[test]
fn test_argparse_validator_is_stripped() {
    let fields = fields_by_name("argparse 'v/verbose!_check_it' -- $argv\n");
    assert_eq!(fields["verbose"].parameter_type, ParameterType::Bool);
    assert!(!fields["verbose"].degraded);
}

#[test]
fn test_argparse_secret_name() {
    let fields = fields_by_name("argparse 'token=' -- $argv\n");
    assert!(fields["token"].secret);
}

#[test]
fn test_argparse_skips_own_options() {
    // -n consumes `tool`, -x consumes `'h,help'`, -i takes no value; only c/city is a spec.
    assert_eq!(
        field_names("argparse -n tool -x 'h,help' -i 'c/city=' -- $argv\n"),
        ["city"]
    );
}

#[test]
fn test_argparse_attached_own_option_does_not_consume() {
    assert_eq!(
        field_names("argparse --name=tool 'c/city=' -- $argv\n"),
        ["city"]
    );
}

#[test]
fn test_argparse_after_conditional_prefix_is_found() {
    assert_eq!(field_names("or argparse 'h/help' -- $argv\n"), ["help"]);
}

#[test]
fn test_argparse_empty_specs_is_zero_field_surface() {
    // Python: `spec is not None` and `spec.fields == []` — a readable zero-field static surface.
    let CliSurface::Static(surface) = parsed("argparse -- $argv\n").cli_surface() else {
        panic!("`argparse -- $argv` must be a static zero-field surface");
    };
    assert!(surface.fields.is_empty());
}

#[test]
fn test_no_argparse_returns_none() {
    // Python `read_cli(..) is None` -> no detected CLI surface at all.
    assert!(matches!(
        parsed("echo hello\n").cli_surface(),
        CliSurface::Absent
    ));
}

#[test]
fn test_argparse_variable_specs_degrade_to_dynamic() {
    // A variable spec list (`argparse $specs -- $argv`) is DETECTED but unmodelable: the reader
    // degrades to a dynamic surface (Python `ok=False`, `reason="dynamic"`, `fields=[]`) instead
    // of fabricating a phantom `$specs` flag out of the variable name.
    assert!(matches!(
        parsed("argparse $specs -- $argv\n").cli_surface(),
        CliSurface::Dynamic(_)
    ));
}

#[test]
fn test_argparse_command_substitution_specs_degrade_to_dynamic() {
    // Command substitution (`argparse (make_specs) -- $argv`) is dynamic too — the option set is
    // unknowable statically.
    assert!(matches!(
        parsed("argparse (make_specs) -- $argv\n").cli_surface(),
        CliSurface::Dynamic(_)
    ));
}

#[test]
fn test_argparse_garbage_specs_are_skipped() {
    // Empty spec, a value-suffix with no name (`=`), validator-only, bare and leading separators.
    assert_eq!(
        field_names("argparse '' '=' '!v' '#' '/x' 'ok' -- $argv\n"),
        ["ok"]
    );
}

#[test]
fn test_spec_tokens_all_own_options_no_specs() {
    // Python white-box: `fc._spec_tokens(["-n", "tool"]) == []` — own options exhaust the token
    // list with no spec and no `--`.
    //
    // Ported via public output: `argparse -n tool` (no `--`, no spec) is the observable equivalent,
    // a readable static surface with zero fields (`ArgSpec(fields=[])`).
    let CliSurface::Static(surface) = parsed("argparse -n tool\n").cli_surface() else {
        panic!("own-options-only argparse must be a static zero-field surface");
    };
    assert!(surface.fields.is_empty());
}

#[test]
fn test_argparse_empty_long_falls_back_to_short() {
    let fields = fields_by_name("argparse 'x/' -- $argv\n");
    assert_eq!(fields["x"].flag, "-x");
}

// ---------------------------------------------------------------- registry wiring

#[test]
fn test_registry_capabilities() {
    // Python asserts `registry.spec_for("fish")` has analyzer + cli_reader + params_io but NO
    // injector. The registry object lives in skit-application; each capability is projected here
    // onto its skit-language observable:
    //   analyzer   -> env-idiom detection produces candidates.
    //   cli_reader -> argparse reading produces a static surface.
    //   params_io  -> the `#`-block engine round-trips a managed declaration.
    //   injector   -> is None: fish injection returns UnsupportedKind (no fish arm in the planner).
    assert!(!cands("set -q PORT; or set PORT 8080\n").is_empty()); // analyzer
    assert!(matches!(
        parsed("argparse 'h/help' -- $argv\n").cli_surface(),
        CliSurface::Static(_)
    )); // cli_reader
    let mut managed = ParamDecl::new("PORT");
    managed.binding = ParameterBinding::EnvDefault;
    managed.delivery = ParameterDelivery::Env;
    assert!(write_managed_params("fish", "echo hi\n", std::slice::from_ref(&managed)).is_ok()); // params_io
    // injector is None: MUST-VERIFY — a fish injection plan must degrade, never fabricate a rewrite.
    assert!(matches!(
        parsed("set -q PORT; or set PORT 8080\n").plan_injection(&[managed], &BTreeMap::new()),
        Err(LanguageError::UnsupportedKind { .. })
    ));
}

// ---------------------------------------------------------------- corpus sweep

#[test]
fn test_corpus_analyze_is_total_and_reads_back() {
    // Every emitted candidate is an env-default (v1 scope); the block writer round-trips them.
    for (name, text) in FISH_CORPUS {
        let analysis = parsed(text).analysis(); // fish is total — never a syntax error
        assert!(
            analysis
                .candidates
                .iter()
                .all(|candidate| candidate.declaration.binding == ParameterBinding::EnvDefault),
            "{name}"
        );
        let specs = analysis
            .candidates
            .iter()
            .map(|candidate| candidate.declaration.clone())
            .collect::<Vec<_>>();
        let written = write_managed_params("fish", text, &specs).expect("write_managed_params");
        let read_back = managed_params("fish", &written)
            .into_iter()
            .map(|declaration| declaration.name)
            .collect::<BTreeSet<_>>();
        let names = analysis
            .candidates
            .iter()
            .map(|candidate| candidate.declaration.name.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(read_back, names, "{name}");
    }
}

#[test]
fn test_corpus_expected_detections() {
    let detected = FISH_CORPUS
        .iter()
        .map(|(name, text)| ((*name).to_owned(), detected_names(text)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        detected["01_env_idioms.fish"],
        string_set(["PORT", "RATE", "REGION", "LOG_DIR"])
    );
    assert_eq!(
        detected["04_block_nesting.fish"],
        string_set(["TOP", "ALSO_TOP"])
    );
    assert_eq!(
        detected["05_reads_and_consts.fish"],
        string_set(["RETRIES"])
    );
    assert_eq!(
        detected["06_cjk.fish"],
        string_set(["問候", "EMOJI", "CITY"])
    );
}

// ---------------------------------------------------------------- env-default e2e

#[test]
#[ignore = "UNMAPPED: `skit params <name> --manage` (CliRunner) + store.add_script + flows.plan_for_entry/assemble asserting plan.source=='inject', field.source=='env', asm.env_values -> Tier 4 (skit-cli/flows/store). The `#`-block write half is covered by test_corpus_analyze_is_total_and_reads_back; the env-delivery binding by test_reconcile_ok_then_drift."]
fn test_manage_then_plan_and_assemble_env_delivery() {}

#[test]
#[ignore = "UNMAPPED: spawns the real `fish` on the stored copy and asserts the env overlay beats the script's default on stdout (skipif fish is not installed) -> Tier 3/4 (skit run / process spawn). skit-language produces no run; the env-default detection is covered by test_oneline_idiom_int."]
fn test_env_overlay_overrides_default_in_real_fish() {}
