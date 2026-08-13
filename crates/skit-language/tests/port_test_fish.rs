//! Public-surface ports of Python v0.4 `tests/test_fish.py` at `main@206f9ef`.
//!
//! Rust replaced the Python hand tokenizer/dequote/classify helpers with a tree-sitter document.
//! Those private helper contracts are accounted in the companion manifest, not reimplemented here.
//! Every executable test below uses a published semantic, CLI-surface, reconcile, or managed-block
//! boundary. `rust_additive_*` tests split Python parametrized/multi-case rows so an early failure
//! cannot hide a later Fish contract.

use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Path, PathBuf}};

use skit_domain::parameters::{
    ParamDecl, ParameterBinding, ParameterDelivery, ParameterType, ParameterValue,
};
use skit_language::{
    CliSurface, DegradationReason, ParseOutcome, ParsedDocument, managed_params, parse_document,
    write_managed_params,
};

fn document(source: &str) -> ParsedDocument {
    match parse_document("fish", source) {
        ParseOutcome::Parsed(document) => document,
        other => panic!("expected parsed Fish source, got {other:?}"),
    }
}

fn candidates(source: &str) -> BTreeMap<String, ParamDecl> {
    document(source)
        .analysis()
        .candidates
        .into_iter()
        .map(|candidate| (candidate.declaration.name.clone(), candidate.declaration))
        .collect()
}

fn fields(source: &str) -> Vec<ParamDecl> {
    match document(source).cli_surface() {
        CliSurface::Static(surface) => {
            assert_eq!(surface.framework, "argparse");
            surface
                .fields
                .into_iter()
                .map(|field| field.declaration)
                .collect()
        }
        other => panic!("expected a static Fish argparse surface, got {other:?}"),
    }
}

fn field_map(fields: &[ParamDecl]) -> BTreeMap<&str, &ParamDecl> {
    fields.iter().map(|field| (field.name.as_str(), field)).collect()
}

fn names(fields: &[ParamDecl]) -> Vec<&str> {
    fields.iter().map(|field| field.name.as_str()).collect()
}

#[test]
fn test_oneline_idiom_int() {
    let c = candidates("set -q PORT; or set PORT 8080\n");
    let port = &c["PORT"];
    assert_eq!(port.binding, ParameterBinding::EnvDefault);
    assert_eq!(port.delivery, ParameterDelivery::Env);
    assert_eq!(port.parameter_type, ParameterType::Int);
    assert_eq!(port.default, Some(ParameterValue::Integer(8080)));
    assert_eq!(port.env_var(), "PORT");
}

#[test]
fn test_newline_continued_or() {
    let c = candidates("set -q PORT\nor set PORT 8080\n");
    assert_eq!(c["PORT"].default, Some(ParameterValue::Integer(8080)));
}

#[test]
fn test_float_and_string_defaults() {
    let c = candidates(
        "set -q RATE; or set RATE 2.5\nset -q REGION; or set REGION us-east-1\n",
    );
    assert_eq!(c["RATE"].parameter_type, ParameterType::Float);
    assert_eq!(c["RATE"].default, Some(ParameterValue::Float(2.5)));
    assert_eq!(c["REGION"].parameter_type, ParameterType::Str);
    assert_eq!(
        c["REGION"].default,
        Some(ParameterValue::String("us-east-1".to_owned()))
    );
}

#[test]
fn rust_additive_float_default() {
    let c = candidates("set -q RATE; or set RATE 2.5\n");
    assert_eq!(c["RATE"].parameter_type, ParameterType::Float);
    assert_eq!(c["RATE"].default, Some(ParameterValue::Float(2.5)));
}

#[test]
fn rust_additive_string_default() {
    let c = candidates("set -q REGION; or set REGION us-east-1\n");
    assert_eq!(c["REGION"].parameter_type, ParameterType::Str);
    assert_eq!(
        c["REGION"].default,
        Some(ParameterValue::String("us-east-1".to_owned()))
    );
}

#[test]
fn test_guarded_set_may_carry_scope_flags() {
    assert_eq!(
        candidates("set -q LOG; or set -gx LOG /var/log\n")["LOG"].default,
        Some(ParameterValue::String("/var/log".to_owned()))
    );
}

#[test]
fn test_secret_name_flagged() {
    assert!(candidates("set -q API_TOKEN; or set API_TOKEN x\n")["API_TOKEN"].secret);
}

#[test]
fn test_suppressed_by_plain_clobber_anywhere() {
    assert!(candidates("set -q PORT; or set PORT 8080\nset PORT 9090\n").is_empty());
}

#[test]
fn test_clobber_before_the_idiom_also_suppresses() {
    assert!(candidates("set PORT 9090\nset -q PORT; or set PORT 8080\n").is_empty());
}

#[test]
fn test_unrelated_clobber_does_not_suppress() {
    let c = candidates("set OTHER 1\nset -q PORT; or set PORT 8080\n");
    assert!(c.contains_key("PORT"));
    assert!(!c.contains_key("OTHER"));
}

#[test]
fn test_underscore_name_skipped() {
    assert!(candidates("set -q _P; or set _P 1\n").is_empty());
}

#[test]
fn test_first_occurrence_wins_on_duplicate_idiom() {
    let c = candidates("set -q PORT; or set PORT 8080\nset -q PORT; or set PORT 1\n");
    assert_eq!(c["PORT"].default, Some(ParameterValue::Integer(8080)));
}

#[test]
fn test_query_without_following_set_is_not_a_candidate() {
    assert!(candidates("set -q PORT\necho done\n").is_empty());
}

#[test]
fn test_query_with_no_name_is_ignored() {
    assert!(candidates("set -q; or set PORT 8080\n").is_empty());
}

#[test]
fn test_conditional_set_without_value_is_not_a_candidate() {
    assert!(candidates("set -q PORT; or set PORT\n").is_empty());
}

#[test]
fn test_mismatched_names_are_not_an_idiom() {
    assert!(candidates("set -q PORT; or set OTHER 8080\n").is_empty());
}

#[test]
fn test_unconditional_set_after_query_is_not_an_idiom() {
    assert!(candidates("set -q X; set X 1\n").is_empty());
}

#[test]
fn test_idiom_inside_function_is_not_toplevel() {
    assert!(candidates("function f\n  set -q P; or set P 1\nend\n").is_empty());
}

fn assert_block_ignored(opener: &str) {
    assert!(
        candidates(&format!("{opener}\n  set -q P; or set P 1\nend\n")).is_empty(),
        "Fish env-default inside {opener:?} escaped its block"
    );
}

#[test]
fn test_idiom_inside_every_block_kind_is_ignored() {
    for opener in ["if true", "while true", "for x in 1", "begin", "switch $x"] {
        assert_block_ignored(opener);
    }
}

#[test]
fn rust_additive_fish_if_block_is_ignored() { assert_block_ignored("if true"); }
#[test]
fn rust_additive_fish_while_block_is_ignored() { assert_block_ignored("while true"); }
#[test]
fn rust_additive_fish_for_block_is_ignored() { assert_block_ignored("for x in 1"); }
#[test]
fn rust_additive_fish_begin_block_is_ignored() { assert_block_ignored("begin"); }
#[test]
fn rust_additive_fish_switch_block_is_ignored() { assert_block_ignored("switch $x"); }

#[test]
fn test_toplevel_after_a_closed_block_is_detected() {
    let c = candidates("function f\n  echo hi\nend\nset -q P; or set P 1\n");
    assert_eq!(c["P"].default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_nested_clobber_does_not_suppress_toplevel_idiom() {
    let c = candidates("set -q P; or set P 1\nfunction f\n  set P 9\nend\n");
    assert_eq!(c["P"].default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_stray_end_clamps_depth_at_zero() {
    let c = candidates("end\nset -q P; or set P 1\n");
    assert_eq!(c["P"].default, Some(ParameterValue::Integer(1)));
}

#[test]
fn test_argv_hint() {
    assert!(document("echo $argv\n").analysis().uses_argv);
}

#[test]
fn test_self_location_hints() {
    assert!(document("set d (status dirname)\n").analysis().uses_self_location);
    assert!(document("set f (status filename)\n").analysis().uses_self_location);
    assert!(!document("echo hi\n").analysis().uses_self_location);
}

#[test]
fn rust_additive_fish_status_dirname_is_self_location() {
    assert!(document("set d (status dirname)\n").analysis().uses_self_location);
}
#[test]
fn rust_additive_fish_status_filename_is_self_location() {
    assert!(document("set f (status filename)\n").analysis().uses_self_location);
}
#[test]
fn rust_additive_plain_fish_has_no_self_location_hint() {
    assert!(!document("echo hi\n").analysis().uses_self_location);
}

#[test]
fn test_hint_ignores_commented_argv() {
    assert!(!document("# uses $argv here\necho hi\n").analysis().uses_argv);
}

#[test]
fn test_reconcile_ok_then_drift() {
    let mut port = ParamDecl::new("PORT");
    port.binding = ParameterBinding::EnvDefault;
    port.delivery = ParameterDelivery::Env;
    let ok = document("set -q PORT; or set PORT 8080\n").reconcile(&[port.clone()]);
    assert_eq!(ok.ok.len(), 1);
    assert!(!ok.has_drift());
    let gone = document("echo hi\n").reconcile(&[port]);
    assert_eq!(
        gone.missing.iter().map(|spec| spec.name.as_str()).collect::<Vec<_>>(),
        ["PORT"]
    );
}

#[test]
fn test_argparse_short_long_and_valueless_bool() {
    let actual = fields("argparse 'h/help' 'v/verbose' -- $argv\n");
    let map = field_map(&actual);
    assert_eq!(map["help"].flag, "--help");
    assert_eq!(map["help"].parameter_type, ParameterType::Bool);
    assert_eq!(map["help"].action, "store_true");
    assert_eq!(map["verbose"].parameter_type, ParameterType::Bool);
}

#[test]
fn test_argparse_value_suffixes() {
    let actual = fields("argparse 'n/name=' 'r/retries=?' 'f/file=+' 'g/glob=*' -- $argv\n");
    let map = field_map(&actual);
    assert_eq!(map["name"].parameter_type, ParameterType::Str);
    assert!(!map["name"].multiple);
    assert_eq!(map["retries"].parameter_type, ParameterType::Str);
    assert!(map["file"].multiple);
    assert!(map["glob"].multiple);
    assert!(map["file"].repeat);
    assert!(map["glob"].repeat);
    assert!(!map["name"].repeat);
}

#[test]
fn rust_additive_fish_required_value_is_scalar() {
    let actual = fields("argparse 'n/name=' -- $argv\n");
    assert!(!field_map(&actual)["name"].multiple);
}
#[test]
fn rust_additive_fish_optional_value_is_string() {
    let actual = fields("argparse 'r/retries=?' -- $argv\n");
    assert_eq!(field_map(&actual)["retries"].parameter_type, ParameterType::Str);
}
#[test]
fn rust_additive_fish_plus_value_repeats() {
    let actual = fields("argparse 'f/file=+' -- $argv\n");
    let field = field_map(&actual)["file"];
    assert!(field.multiple && field.repeat);
}
#[test]
fn rust_additive_fish_star_value_repeats() {
    let actual = fields("argparse 'g/glob=*' -- $argv\n");
    let field = field_map(&actual)["glob"];
    assert!(field.multiple && field.repeat);
}

#[test]
fn test_argparse_long_only_and_short_only() {
    let actual = fields("argparse 'dry-run' 'x' -- $argv\n");
    let map = field_map(&actual);
    assert_eq!(map["dry-run"].flag, "--dry-run");
    assert_eq!(map["x"].flag, "-x");
}

#[test]
fn test_argparse_dummy_short_yields_long_only() {
    let actual = fields("argparse 'x-long' -- $argv\n");
    assert_eq!(field_map(&actual)["long"].flag, "--long");
}

#[test]
fn test_argparse_numeric_hash_degrades() {
    let actual = fields("argparse 'm#max' -- $argv\n");
    let max = field_map(&actual)["max"];
    assert_eq!(max.flag, "--max");
    assert!(max.degraded);
}

#[test]
fn test_argparse_validator_is_stripped() {
    let actual = fields("argparse 'v/verbose!_check_it' -- $argv\n");
    let verbose = field_map(&actual)["verbose"];
    assert_eq!(verbose.parameter_type, ParameterType::Bool);
    assert!(!verbose.degraded);
}

#[test]
fn test_argparse_secret_name() {
    assert!(field_map(&fields("argparse 'token=' -- $argv\n"))["token"].secret);
}

#[test]
fn test_argparse_skips_own_options() {
    let actual = fields("argparse -n tool -x 'h,help' -i 'c/city=' -- $argv\n");
    assert_eq!(names(&actual), ["city"]);
}

#[test]
fn test_argparse_attached_own_option_does_not_consume() {
    let actual = fields("argparse --name=tool 'c/city=' -- $argv\n");
    assert_eq!(names(&actual), ["city"]);
}

#[test]
fn test_argparse_after_conditional_prefix_is_found() {
    let actual = fields("or argparse 'h/help' -- $argv\n");
    assert_eq!(names(&actual), ["help"]);
}

#[test]
fn test_argparse_empty_specs_is_zero_field_surface() {
    let actual = fields("argparse -- $argv\n");
    assert!(actual.is_empty());
}

#[test]
fn test_no_argparse_returns_none() {
    assert!(matches!(document("echo hello\n").cli_surface(), CliSurface::Absent));
}

fn assert_dynamic_argparse(source: &str) {
    match document(source).cli_surface() {
        CliSurface::Dynamic(surface) => {
            assert_eq!(surface.framework, "argparse");
            assert_eq!(surface.reason, DegradationReason::DynamicDeclaration);
        }
        other => panic!("expected dynamic Fish argparse surface, got {other:?}"),
    }
}

#[test]
fn test_argparse_variable_specs_degrade_to_dynamic() {
    assert_dynamic_argparse("argparse $specs -- $argv\n");
}

#[test]
fn test_argparse_command_substitution_specs_degrade_to_dynamic() {
    assert_dynamic_argparse("argparse (make_specs) -- $argv\n");
}

#[test]
fn test_argparse_garbage_specs_are_skipped() {
    let actual = fields("argparse '' '=' '!v' '#' '/x' 'ok' -- $argv\n");
    assert_eq!(names(&actual), ["ok"]);
}

#[test]
fn test_argparse_empty_long_falls_back_to_short() {
    let actual = fields("argparse 'x/' -- $argv\n");
    assert_eq!(field_map(&actual)["x"].flag, "-x");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("skit-language lives under <repo>/crates/skit-language")
        .to_path_buf()
}

fn corpus_paths() -> Vec<PathBuf> {
    let mut paths = fs::read_dir(repo_root().join("tests/corpus/fish"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("fish"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn assert_corpus_roundtrip(path: &Path) {
    let text = fs::read_to_string(path).unwrap();
    let analysis = document(&text).analysis();
    assert!(
        analysis
            .candidates
            .iter()
            .all(|candidate| candidate.declaration.binding == ParameterBinding::EnvDefault),
        "{} emitted a non-env-default candidate",
        path.display()
    );
    let specs = analysis
        .candidates
        .iter()
        .map(|candidate| candidate.declaration.clone())
        .collect::<Vec<_>>();
    let written = write_managed_params("fish", &text, &specs).unwrap();
    let read_back = managed_params("fish", &written)
        .into_iter()
        .map(|spec| spec.name)
        .collect::<BTreeSet<_>>();
    let detected = specs.into_iter().map(|spec| spec.name).collect::<BTreeSet<_>>();
    assert_eq!(read_back, detected, "{} did not round-trip", path.display());
}

#[test]
fn test_corpus_analyze_is_total_and_reads_back() {
    let paths = corpus_paths();
    assert_eq!(paths.len(), 6, "the frozen Fish corpus changed");
    for path in paths {
        assert_corpus_roundtrip(&path);
    }
}

fn corpus(name: &str) -> PathBuf { repo_root().join("tests/corpus/fish").join(name) }

#[test]
fn rust_additive_fish_corpus_01_roundtrips() { assert_corpus_roundtrip(&corpus("01_env_idioms.fish")); }
#[test]
fn rust_additive_fish_corpus_02_roundtrips() { assert_corpus_roundtrip(&corpus("02_argparse.fish")); }
#[test]
fn rust_additive_fish_corpus_03_roundtrips() { assert_corpus_roundtrip(&corpus("03_quoting.fish")); }
#[test]
fn rust_additive_fish_corpus_04_roundtrips() { assert_corpus_roundtrip(&corpus("04_block_nesting.fish")); }
#[test]
fn rust_additive_fish_corpus_05_roundtrips() { assert_corpus_roundtrip(&corpus("05_reads_and_consts.fish")); }
#[test]
fn rust_additive_fish_corpus_06_roundtrips() { assert_corpus_roundtrip(&corpus("06_cjk.fish")); }

fn detected_names(name: &str) -> BTreeSet<String> {
    candidates(&fs::read_to_string(corpus(name)).unwrap())
        .into_keys()
        .collect()
}

#[test]
fn test_corpus_expected_detections() {
    assert_eq!(
        detected_names("01_env_idioms.fish"),
        ["PORT", "RATE", "REGION", "LOG_DIR"].into_iter().map(str::to_owned).collect()
    );
    assert_eq!(
        detected_names("04_block_nesting.fish"),
        ["TOP", "ALSO_TOP"].into_iter().map(str::to_owned).collect()
    );
    assert_eq!(
        detected_names("05_reads_and_consts.fish"),
        ["RETRIES"].into_iter().map(str::to_owned).collect()
    );
    assert_eq!(
        detected_names("06_cjk.fish"),
        ["問候", "EMOJI", "CITY"].into_iter().map(str::to_owned).collect()
    );
}
