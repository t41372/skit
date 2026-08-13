//! Exact public-surface ports of Shell const/env-default/suppression contracts from Python v0.4
//! `tests/test_shell_analyzer.py` at `main@206f9ef`.

use std::collections::{BTreeMap, BTreeSet};
use skit_domain::parameters::{ParameterBinding, ParameterDelivery, ParameterType, ParameterValue};
use skit_language::{ParseOutcome, SemanticCandidate, parse_document};

fn candidates(source: &str) -> Vec<SemanticCandidate> {
    let ParseOutcome::Parsed(document) = parse_document("shell", source) else {
        panic!("fixture must parse as shell: {source:?}");
    };
    document.analysis().candidates
}
fn by_name(source: &str) -> BTreeMap<String, SemanticCandidate> {
    candidates(source).into_iter().map(|c| (c.declaration.name.clone(), c)).collect()
}
fn names(source: &str) -> Vec<String> {
    candidates(source).into_iter().map(|c| c.declaration.name).collect()
}

#[test]
fn test_const_word_number_raw_double_quoted() {
    let b=by_name("A=plain\nB=42\nC='raw text'\nD=\"double q\"\n");
    assert_eq!((b["A"].declaration.parameter_type,b["A"].declaration.default.clone()),(ParameterType::Str,Some(ParameterValue::String("plain".into()))));
    assert_eq!((b["B"].declaration.parameter_type,b["B"].declaration.default.clone()),(ParameterType::Int,Some(ParameterValue::Integer(42))));
    assert_eq!((b["C"].declaration.parameter_type,b["C"].declaration.default.clone()),(ParameterType::Str,Some(ParameterValue::String("raw text".into()))));
    assert_eq!((b["D"].declaration.parameter_type,b["D"].declaration.default.clone()),(ParameterType::Str,Some(ParameterValue::String("double q".into()))));
}
#[test]
fn test_const_excludes_empty_array_concat_expansion_cmdsub() {
    assert!(candidates("EMPTY=\nQUOTED_EMPTY=''\nARR=(1 2 3)\nCONCAT=a$B\nSUBBED=$(date)\nEXPANDED=${OTHER}\n").is_empty());
}
#[test]
fn test_const_leading_underscore_skipped() { assert_eq!(names("_HIDDEN=1\nSHOWN=2\n"),["SHOWN"]); }
#[test]
fn test_const_last_write_wins_keeps_first_slot() {
    let source="X=1\nY=5\nX=2\n"; let b=by_name(source);
    assert_eq!(b["X"].declaration.default,Some(ParameterValue::Integer(2)));
    let n=names(source); assert!(n.iter().position(|x|x=="X")<n.iter().position(|x|x=="Y"));
}
#[test]
fn test_const_plus_equals_is_not_a_literal_const() { assert!(candidates("N+=1\n").is_empty()); }
#[test]
fn test_declaration_export_declare_typeset_included_local_excluded() {
    assert_eq!(names("export A=1\ndeclare -i B=2\ntypeset C=three\nlocal D=4\n").into_iter().collect::<BTreeSet<_>>(),["A","B","C"].into_iter().map(str::to_owned).collect());
}
#[test]
fn test_readonly_and_declare_r_excluded() { assert_eq!(names("readonly LOCKED=1\ndeclare -r FROZEN=2\ntypeset -rx ALSO=3\nOPEN=4\n"),["OPEN"]); }
#[test]
fn test_envdefault_all_four_operators() {
    let b=by_name(": \"${A:-x}\"\n: \"${B:=y}\"\n: \"${C-z}\"\n: \"${D=w}\"\n");
    assert_eq!(b.keys().cloned().collect::<BTreeSet<_>>(),["A","B","C","D"].into_iter().map(str::to_owned).collect());
    assert!(b.values().all(|c|c.declaration.binding==ParameterBinding::EnvDefault));
    assert_eq!(b["A"].declaration.default,Some(ParameterValue::String("x".into())));
}
#[test]
fn test_envdefault_non_default_operators_ignored() { assert!(candidates(": \"${VAR:?missing}\"\necho \"${#LIST}\"\n").is_empty()); }
#[test]
fn test_envdefault_type_inference_on_default() {
    let b=by_name(": \"${PORT:-8080}\"\n: \"${RATIO:-1.5}\"\n: \"${NAME:-guest}\"\n");
    assert_eq!((b["PORT"].declaration.parameter_type,b["PORT"].declaration.default.clone()),(ParameterType::Int,Some(ParameterValue::Integer(8080))));
    assert_eq!((b["RATIO"].declaration.parameter_type,b["RATIO"].declaration.default.clone()),(ParameterType::Float,Some(ParameterValue::Float(1.5))));
    assert_eq!(b["NAME"].declaration.parameter_type,ParameterType::Str);
}
#[test]
fn test_envdefault_empty_default() {
    let c=candidates(": \"${OPT:-}\"\n"); let [c]=c.as_slice() else{panic!("expected one candidate")};
    assert_eq!((c.declaration.parameter_type,c.declaration.default.clone()),(ParameterType::Str,Some(ParameterValue::String(String::new()))));
}
#[test]
fn test_envdefault_subscript_skipped() { assert!(candidates("echo \"${ARR[0]:-x}\"\n").is_empty()); }
#[test]
fn test_envdefault_dedupes_by_name_first_default_wins() {
    let c=candidates("echo \"${MODE:-first}\"\necho \"${MODE:-second}\"\n"); let [c]=c.as_slice() else{panic!("expected one candidate")};
    assert_eq!(c.declaration.default,Some(ParameterValue::String("first".into())));
}
#[test]
fn test_envdefault_carries_env_name() {
    let c=candidates(": \"${TOKEN_URL:-http://x}\"\n"); let [c]=c.as_slice() else{panic!("expected one candidate")};
    assert_eq!(c.declaration.name,"TOKEN_URL"); assert_eq!(c.declaration.env_var(),"TOKEN_URL"); assert_eq!(c.declaration.delivery,ParameterDelivery::Env);
}
#[test]
fn test_self_idiom_is_envdefault_not_suppressed() {
    let b=by_name("PORT=\"${PORT:-8080}\"\nNAME=${NAME:-guest}\n");
    assert_eq!(b["PORT"].declaration.binding,ParameterBinding::EnvDefault); assert_eq!(b["NAME"].declaration.binding,ParameterBinding::EnvDefault);
}
#[test]
fn test_suppression_bare_literal_assignment_wins() {
    let b=by_name("PORT=8080\necho \"${PORT:-9090}\"\n"); assert_eq!(b["PORT"].declaration.binding,ParameterBinding::Const);
    assert_eq!(b.values().filter(|c|c.declaration.name=="PORT"&&c.declaration.binding==ParameterBinding::EnvDefault).count(),0);
}
#[test]
fn test_suppression_cmdsub_assignment_shadows_envdefault() { assert!(candidates("HOST=$(hostname)\necho \"${HOST:-local}\"\n").is_empty()); }
#[test]
fn test_suppression_only_targets_the_shadowed_name() {
    let b=by_name("PORT=8080\necho \"${PORT:-9090}\"\necho \"${MODE:-auto}\"\n"); assert_eq!(b["MODE"].declaration.binding,ParameterBinding::EnvDefault);
}
