use skit_domain::{EntrySettings,parameters::{ParamDecl,ParameterDelivery,ParameterType,ParameterValue,synthesized_placeholder}};
use skit_form::{FormDrift,FormSource,form_plan};

#[test]
fn test_prompt_plan_fields_follow_managed_list(){
 let settings=EntrySettings{params:vec!["a".to_owned(),"api_key".to_owned()],parameters:vec![synthesized_placeholder("a"),synthesized_placeholder("api_key")],..EntrySettings::default()};
 let plan=form_plan("prompt","{{a}} {{api_key}} {{skip}}\n",&settings);assert_eq!(plan.source,FormSource::Command);assert_eq!(plan.fields.iter().map(|f|f.declaration.name.as_str()).collect::<Vec<_>>(),["a","api_key"]);assert!(plan.fields.iter().all(|f|f.declaration.delivery==ParameterDelivery::Placeholder&&f.declaration.required));assert!(plan.fields[1].declaration.secret);assert!(plan.drift.is_empty());
}

#[test]
fn test_prompt_plan_reports_drift_for_gone_managed_names(){
 let settings=EntrySettings{params:vec!["a".to_owned(),"b".to_owned()],parameters:vec![synthesized_placeholder("a"),synthesized_placeholder("b")],..EntrySettings::default()};let plan=form_plan("prompt","only {{a}} now\n",&settings);assert_eq!(plan.fields.iter().map(|f|f.declaration.name.as_str()).collect::<Vec<_>>(),["a","b"]);assert_eq!(plan.drift.len(),1);assert!(matches!(&plan.drift[0],FormDrift::PromptMissing{names} if names==&["b"]));
}

#[test]
fn test_prompt_plan_declared_rows_enrich_schema_and_env_riders_ride(){
 let mut n=synthesized_placeholder("n");n.parameter_type=ParameterType::Int;n.default=Some(ParameterValue::Integer(3));n.required=false;let mut extra=ParamDecl::new("EXTRA");extra.delivery=ParameterDelivery::Env;
 let settings=EntrySettings{params:vec!["n".to_owned()],parameters:vec![n,extra],..EntrySettings::default()};let plan=form_plan("prompt","{{n}}\n",&settings);assert_eq!(plan.fields.len(),2);assert_eq!((plan.fields[0].declaration.name.as_str(),plan.fields[0].declaration.delivery,plan.fields[0].declaration.parameter_type,plan.fields[0].declaration.default.clone(),plan.fields[0].declaration.required),("n",ParameterDelivery::Placeholder,ParameterType::Int,Some(ParameterValue::Integer(3)),false));assert_eq!((plan.fields[1].declaration.name.as_str(),plan.fields[1].declaration.delivery,plan.fields[1].declaration.parameter_type),("EXTRA",ParameterDelivery::Env,ParameterType::Str));
}

#[test]
fn test_command_plan_is_unaffected_by_the_trait_refactor(){
 let settings=EntrySettings{
     params:vec!["size".to_owned(),"out".to_owned()],
     parameters:vec![synthesized_placeholder("size"),synthesized_placeholder("out")],
     template:"convert {size} {out}".to_owned(),
     ..EntrySettings::default()
 };
 let plan=form_plan("command","",&settings);
 assert_eq!(plan.source,FormSource::Command);
 assert_eq!(plan.fields.iter().map(|f|f.declaration.name.as_str()).collect::<Vec<_>>(),["size","out"]);
 assert!(plan.fields.iter().all(|f|f.declaration.delivery==ParameterDelivery::Placeholder&&f.declaration.required),"command fields lost the frozen synthesized-placeholder contract: {plan:?}");
 assert!(plan.drift.is_empty());
 assert!(plan.degradation.is_none());
 // Rust's frontend-neutral FormPlan intentionally has no body-text member at all; unlike Python's
 // legacy plan.text, there is therefore no channel through which command template text can leak as
 // prompt body text. The exact template remains in EntrySettings above while the plan carries only
 // its two synthesized fields.
}
