//! Exact Shell demotion contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use std::collections::BTreeSet;
use skit_language::{DegradationReason,ParseOutcome,SemanticCandidate,parse_document};
fn cands(s:&str)->Vec<SemanticCandidate>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates}
fn demoted(s:&str)->BTreeSet<String>{cands(s).into_iter().filter(|c|c.demotion.is_some()).map(|c|c.declaration.name).collect()}
fn one(s:&str)->SemanticCandidate{let mut c=cands(s);assert_eq!(c.len(),1);c.remove(0)}
#[test] fn test_demote_plus_equals(){assert_eq!(demoted("N=0\nN+=1\n"),["N"].into_iter().map(str::to_owned).collect());}
#[test] fn test_demote_arithmetic_self_reference(){assert_eq!(demoted("TOTAL=100\nTOTAL=$((TOTAL - 1))\n"),["TOTAL"].into_iter().map(str::to_owned).collect());}
#[test] fn test_demote_postfix_increment(){assert_eq!(demoted("N=0\n((N++))\n"),["N"].into_iter().map(str::to_owned).collect());}
#[test] fn test_demote_arithmetic_compound_assignment(){assert_eq!(demoted("N=0\n((N += 5))\n"),["N"].into_iter().map(str::to_owned).collect());}
#[test] fn test_demote_let_target(){assert_eq!(demoted("M=1\nlet M=M+1\n"),["M"].into_iter().map(str::to_owned).collect());}
#[test] fn test_demote_loop_body_reassignment(){assert_eq!(demoted("SUM=0\nfor i in 1 2; do SUM=$((SUM + i)); done\n"),["SUM"].into_iter().map(str::to_owned).collect());}
#[test] fn test_non_mutated_const_not_demoted(){assert_eq!(one("STABLE=7\n").demotion,None);}
#[test] fn test_arithmetic_read_only_does_not_demote(){assert!(demoted("N=3\n(( N > 5 )) && echo big\n").is_empty());}
#[test] fn test_subscript_assignment_is_not_a_const_or_mutation(){assert!(cands("arr[0]=5\n").is_empty());}
#[test] fn test_subscript_loop_reassignment_ignored(){assert!(cands("arr[0]=1\nfor i in 1 2; do arr[i]=$i; done\n").is_empty());}
#[test] fn test_arithmetic_subscript_mutation_has_no_named_target(){assert!(cands("(( arr[0] += 1 ))\n").is_empty());}
#[test] fn test_let_with_non_identifier_argument(){let c=one("COUNT=0\nlet COUNT=1 999\n");assert_eq!(c.declaration.name,"COUNT");assert_eq!(c.demotion,Some(DegradationReason::Accumulator));}
#[test] fn test_postfix_on_subscript_marks_the_base_name(){assert_eq!(demoted("arr=1\n((arr[0]++))\n"),["arr"].into_iter().map(str::to_owned).collect());}
