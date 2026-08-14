//! Public-consequence port of Python `test_read_cluster_keeps_scanning_past_an_unknown_flag_letter`.
use std::collections::BTreeMap;
use skit_domain::parameters::{ParamDecl,ParameterBinding};
use skit_language::{ParseOutcome,inject_values_for_interpreter,parse_document};
fn inputs(s:&str)->Vec<ParamDecl>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).map(|c|c.declaration).collect()}
#[test]
fn test_read_cluster_keeps_scanning_past_an_unknown_flag_letter(){
    let source="read -er X\n";let d=inputs(source);assert_eq!(d.len(),1);assert_eq!(d[0].name,"input-1");
    let rewritten=inject_values_for_interpreter("shell",source,&d,&BTreeMap::from([("input-1".to_owned(),"a\\b".to_owned())]),Some("bash")).unwrap();
    assert!(rewritten.contains("_skit_read 0 'a\\b' 0 '' -er X"),"{rewritten}");
}
