//! Public-consequence port of Python `test_read_flags_do_not_read_letters_from_an_attached_value`.
use std::collections::BTreeMap;
use skit_domain::parameters::{ParamDecl,ParameterBinding};
use skit_language::{ParseOutcome,inject_values_for_interpreter,parse_document};
fn inputs(s:&str)->Vec<ParamDecl>{let ParseOutcome::Parsed(d)=parse_document("shell",s)else{panic!("fixture must parse")};d.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).map(|c|c.declaration).collect()}
fn rewrite(source:&str)->String{let d=inputs(source);assert_eq!(d.len(),1);inject_values_for_interpreter("shell",source,&d,&BTreeMap::from([("input-1".to_owned(),"a\\b".to_owned())]),Some("bash")).unwrap()}
#[test]
fn test_read_flags_do_not_read_letters_from_an_attached_value(){
    for source in ["read -pSure? X\n","read -pEnter X\n","read -idefault X\n"]{assert_eq!(inputs(source).len(),1,"{source:?}");}
    assert!(inputs("read -n3 X\n").is_empty());
    let prompt=rewrite("read -pSure? X\n");assert!(prompt.contains("_skit_read 0 'a\\\\b' 0 'Sure?' -pSure? X"),"{prompt}");
    let raw=rewrite("read -r X\n");assert!(raw.contains("_skit_read 0 'a\\b' 0 '' -r X"),"{raw}");
}
