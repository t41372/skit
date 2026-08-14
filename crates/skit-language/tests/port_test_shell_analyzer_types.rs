//! Exact Shell analyzer type/error contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_domain::parameters::{ParameterType,ParameterValue};
use skit_language::{ParseOutcome,SemanticCandidate,parse_document};
fn cands(s:&str)->Vec<SemanticCandidate>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates}
fn one(s:&str)->SemanticCandidate{let mut c=cands(s);assert_eq!(c.len(),1);c.remove(0)}
#[test] fn test_type_leading_zeros_read_as_int(){let c=one("Z=007\n");assert_eq!((c.declaration.parameter_type,c.declaration.default),(ParameterType::Int,Some(ParameterValue::Integer(7))));}
#[test] fn test_type_negative_int(){let c=one("N=-3\n");assert_eq!((c.declaration.parameter_type,c.declaration.default),(ParameterType::Int,Some(ParameterValue::Integer(-3))));}
#[test] fn test_type_negative_float(){let c=one("F=-2.5\n");assert_eq!((c.declaration.parameter_type,c.declaration.default),(ParameterType::Float,Some(ParameterValue::Float(-2.5))));}
#[test] fn test_type_dotted_version_is_str(){let c=one("V=1.5.2\n");assert_eq!((c.declaration.parameter_type,c.declaration.default),(ParameterType::Str,Some(ParameterValue::String("1.5.2".into()))));}
#[test] fn test_type_never_bool(){let c=cands("FLAG=true\nOTHER=false\n");assert_eq!(c.len(),2);assert!(c.iter().all(|c|c.declaration.parameter_type==ParameterType::Str));assert_eq!(one("FLAG=true\n").declaration.default,Some(ParameterValue::String("true".into())));}
#[test] fn test_has_error_returns_empty_syntax_error(){let s="if [[ -n $x ]] { echo hi }\nCONFIG=1\n";assert!(matches!(parse_document("shell",s),ParseOutcome::SyntaxError(_)));assert!(skit_language::detect_candidates("shell",s).is_empty());}
#[test] fn test_empty_script(){let ParseOutcome::Parsed(d)=parse_document("shell","")else{panic!("empty shell must parse")};assert!(d.analysis().candidates.is_empty());}
