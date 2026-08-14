//! Exact Shell read/non-read classification contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_domain::parameters::ParameterBinding;
use skit_language::{ParseOutcome,parse_document};
fn reads(s:&str)->Vec<skit_language::SemanticCandidate>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).collect()}
#[test] fn test_non_read_command_ignored(){assert!(reads("echo hello\nprintf '%s' x\n").is_empty());}
#[test] fn test_builtin_without_read_is_not_a_read(){assert!(reads("builtin pwd\ncommand ls\n").is_empty());}
#[test] fn test_bare_builtin_is_not_a_read(){assert!(reads("builtin\n").is_empty());}
#[test] fn test_read_secret_by_varname_and_prompt(){let a=reads("read PASSWORD\n");assert_eq!(a.len(),1);assert!(a[0].declaration.secret);let b=reads("read -p \"API key: \" K\n");assert_eq!(b.len(),1);assert!(b[0].declaration.secret);}
