//! Exact Shell stdin/data-read ancestry contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_domain::parameters::ParameterBinding;
use skit_language::{ParseOutcome,parse_document};
fn reads(s:&str)->Vec<skit_language::SemanticCandidate>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).collect()}
#[test] fn test_data_read_pipe_right_operand_excluded(){assert!(reads("cat f | while read -r line; do echo $line; done\n").is_empty());}
#[test] fn test_data_read_pipe_three_stage_excluded(){assert!(reads("a | b | read Z\n").is_empty());}
#[test] fn test_read_first_pipe_operand_is_interactive(){assert_eq!(reads("read X | cat\n").len(),1);}
#[test] fn test_data_read_loop_fed_by_file_redirect_excluded(){assert!(reads("while read -r x; do echo $x; done < f\n").is_empty());}
#[test] fn test_data_read_own_stdin_redirect_excluded(){assert!(reads("read -r x < input.txt\n").is_empty());}
#[test] fn test_data_read_herestring_excluded(){assert!(reads("read -r x <<< \"$data\"\n").is_empty());}
#[test] fn test_data_read_heredoc_loop_excluded(){assert!(reads("while read -r x; do :; done <<EOF\na\nEOF\n").is_empty());}
#[test] fn test_read_with_output_redirect_is_still_interactive(){assert_eq!(reads("read -r x > out.log\n").len(),1);}
