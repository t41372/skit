//! Public-observable port of Python `test_analyzer_and_injector_share_one_read_enumeration`.
use std::collections::BTreeMap;
use skit_domain::parameters::ParameterBinding;
use skit_language::{ParseOutcome,inject_values_for_interpreter,parse_document};
#[test]
fn test_analyzer_and_injector_share_one_read_enumeration(){
    for source in ["read -n 3 CODE\nread NAME\n","IFS=: read A B\nread NAME\n","read P\nread Q\nread R\n","cmd | while read x; do :; done\nread TOP\n"] {
        let ParseOutcome::Parsed(document)=parse_document("shell",source)else{panic!("fixture must parse")};
        let declarations=document.analysis().candidates.into_iter().filter(|c|c.declaration.binding==ParameterBinding::Input).map(|c|c.declaration).collect::<Vec<_>>();
        let values=declarations.iter().map(|d|(d.name.clone(),format!("v{}",d.order))).collect::<BTreeMap<_,_>>();
        let rewritten=inject_values_for_interpreter("shell",source,&declarations,&values,Some("bash")).unwrap();
        assert_eq!(rewritten.matches("_skit_read ").count(),declarations.len(),"source={source:?}\n{rewritten}");
        assert!(matches!(parse_document("shell",&rewritten),ParseOutcome::Parsed(_)),"{rewritten}");
    }
}
