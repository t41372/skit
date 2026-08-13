//! Exact Shell analyzer hint contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_language::{ParseOutcome,SemanticAnalysis,parse_document};
fn analysis(s:&str)->SemanticAnalysis{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis()}
#[test] fn test_uses_self_location_dollar_zero(){assert!(analysis("D=$(dirname \"$0\")\n").uses_self_location);}
#[test] fn test_uses_self_location_bash_source_and_subscript(){assert!(analysis("echo \"$BASH_SOURCE ${BASH_SOURCE[0]}\"\n").uses_self_location);}
#[test] fn test_no_self_location(){assert!(!analysis("X=1\n").uses_self_location);}
#[test] fn test_uses_argv_positional(){assert!(analysis("echo \"$1 $2\"\n").uses_argv);}
#[test] fn test_uses_argv_special_at_hash_star(){assert!(analysis("echo \"$@ $# $*\"\n").uses_argv);}
#[test] fn test_uses_argv_getopts_and_shift(){assert!(analysis("getopts \"ab\" o\n").uses_argv);assert!(analysis("shift\n").uses_argv);}
#[test] fn test_dollar_zero_is_not_argv(){assert!(!analysis("echo \"$0\"\n").uses_argv);}
#[test] fn test_other_special_variables_are_not_argv(){assert!(!analysis("echo $? $$ $!\n").uses_argv);}
#[test] fn test_no_argv(){assert!(!analysis("X=1\n").uses_argv);}
