//! Exact Shell `read` option/prompt contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_domain::parameters::ParameterBinding;
use skit_language::{ParseOutcome,SemanticCandidate,parse_document};
fn reads(s:&str)->Vec<SemanticCandidate>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).collect()}
#[test]
fn test_read_prompt_and_order_keys(){let r=reads("read -p \"Name: \" NAME\nread -p \"Age: \" AGE\n");assert_eq!(r.len(),2);assert_eq!((&r[0].declaration.name,r[0].declaration.order,&r[0].declaration.prompt),( &"input-1".to_owned(),0,&"Name: ".to_owned()));assert_eq!((&r[1].declaration.name,r[1].declaration.order,&r[1].declaration.prompt),( &"input-2".to_owned(),1,&"Age: ".to_owned()));}
#[test] fn test_read_secret_certainty_via_dash_s(){let r=reads("read -s -p \"Enter value: \" V\n");assert_eq!(r.len(),1);assert!(r[0].declaration.secret);}
#[test] fn test_read_clustered_sp(){let r=reads("read -sp \"PIN: \" PIN\n");assert_eq!(r.len(),1);assert!(r[0].declaration.secret);assert_eq!(r[0].declaration.prompt,"PIN: ");}
#[test] fn test_read_clustered_rp_not_secret(){let r=reads("read -rp \"Confirm: \" C\n");assert_eq!(r.len(),1);assert!(!r[0].declaration.secret);assert_eq!(r[0].declaration.prompt,"Confirm: ");}
#[test] fn test_read_multiple_varnames_share_prompt(){let r=reads("read -p \"Two: \" FIRST LAST\n");assert_eq!(r.iter().map(|c|c.declaration.name.as_str()).collect::<Vec<_>>(),["input-1","input-2"]);assert!(r.iter().all(|c|c.declaration.prompt=="Two: "));}
#[test] fn test_read_dynamic_prompt_collapses_to_empty(){let r=reads("read -p \"$MSG\" V\n");assert_eq!(r.len(),1);assert_eq!(r[0].declaration.prompt,"");}
#[test] fn test_read_prompt_from_bare_word(){let r=reads("read -p Enter: V\n");assert_eq!(r.len(),1);assert_eq!(r[0].declaration.prompt,"Enter:");}
#[test] fn test_read_attached_prompt(){let r=reads("read -pHello V\n");assert_eq!(r.len(),1);assert_eq!(r[0].declaration.prompt,"Hello");}
#[test] fn test_read_value_flags_skip_their_argument(){let r=reads("read -t 5 -u 0 V\n");assert_eq!(r.len(),1);assert_eq!(r[0].declaration.name,"input-1");}
#[test] fn test_read_attached_value_flag_not_consumed(){let r=reads("read -t5 W\n");assert_eq!(r.len(),1);assert_eq!(r[0].declaration.name,"input-1");}
#[test] fn test_reframing_reads_are_excluded_from_candidacy(){for s in ["read -n 3 X\n","read -N 5 X\n","read -d : X\n","read -n3 X\n"]{assert!(reads(s).is_empty(),"{s:?}");}}
#[test] fn test_custom_ifs_reads_are_excluded_from_candidacy(){assert!(reads("IFS=: read A B\n").is_empty());assert!(reads("IFS= read -r LINE\n").is_empty());assert_eq!(reads("read -p \"p: \" A B\n").len(),2);}
