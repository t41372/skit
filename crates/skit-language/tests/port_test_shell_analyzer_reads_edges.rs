//! Exact Shell `read` edge contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_domain::parameters::ParameterBinding;
use skit_language::{ParseOutcome,parse_document};
fn reads(s:&str)->Vec<skit_language::SemanticCandidate>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).collect()}
#[test] fn test_read_end_of_options_marker(){let r=reads("read -- -weird\n");assert_eq!(r.len(),1);assert_eq!(r[0].declaration.prompt,"");}
#[test] fn test_read_single_dash_is_a_varname(){assert_eq!(reads("read -\n").len(),1);}
#[test] fn test_read_non_word_argument_skipped(){let r=reads("read \"$dyn\" REAL\n");assert_eq!(r.iter().map(|c|c.declaration.name.as_str()).collect::<Vec<_>>(),["input-1"]);}
#[test] fn test_read_dash_p_at_end_no_argument(){assert!(reads("read -p\n").is_empty());}
#[test] fn test_builtin_and_command_read_recognized(){assert_eq!(reads("builtin read X\n").len(),1);assert_eq!(reads("command read Y\n").len(),1);}
