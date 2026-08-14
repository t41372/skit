//! Exact Shell reconcile contracts from Python v0.4 `tests/test_shell_analyzer.py`.
use skit_domain::parameters::{ParamDecl,ParameterBinding,ParameterDelivery,ParameterType};
use skit_language::{ParseOutcome,parse_document};
fn reconcile(source:&str,specs:&[ParamDecl])->skit_language::ReconcileReport{let ParseOutcome::Parsed(d)=parse_document("shell",source)else{panic!("fixture must parse")};d.reconcile(specs)}
fn env(name:&str)->ParamDecl{let mut d=ParamDecl::new(name);d.binding=ParameterBinding::EnvDefault;d.delivery=ParameterDelivery::Env;d.parameter_type=ParameterType::Str;d}
#[test]
fn test_reconcile_const_and_input_parity(){let mut city=ParamDecl::new("CITY");city.binding=ParameterBinding::Const;city.parameter_type=ParameterType::Str;let mut input=ParamDecl::new("input-1");input.binding=ParameterBinding::Input;input.order=0;input.prompt="Name: ".into();let r=reconcile("CITY=Taipei\nread -p \"Name: \" NAME\n",&[city,input]);assert!(!r.has_drift());assert_eq!(r.ok.iter().map(|p|p.stored.name.as_str()).collect::<std::collections::BTreeSet<_>>(),["CITY","input-1"].into_iter().collect());}
#[test] fn test_reconcile_envdefault_ok(){let r=reconcile("echo \"${PORT:-8080}\"\n",&[env("PORT")]);assert!(!r.has_drift());assert_eq!(r.ok.iter().map(|p|p.stored.name.as_str()).collect::<Vec<_>>(),["PORT"]);}
#[test] fn test_reconcile_envdefault_default_change_is_still_ok(){let r=reconcile("echo \"${PORT:-9090}\"\n",&[env("PORT")]);assert!(!r.has_drift());assert_eq!(r.ok.iter().map(|p|p.stored.name.as_str()).collect::<Vec<_>>(),["PORT"]);}
#[test] fn test_reconcile_envdefault_gone_is_missing(){let r=reconcile("echo hello\n",&[env("PORT")]);assert!(r.has_drift());assert_eq!(r.missing.iter().map(|d|d.name.as_str()).collect::<Vec<_>>(),["PORT"]);}
#[test] fn test_reconcile_envdefault_bare_assignment_shadow_is_missing(){let r=reconcile("PORT=8080\necho \"${PORT:-9090}\"\n",&[env("PORT")]);assert!(r.has_drift());assert_eq!(r.missing.iter().map(|d|d.name.as_str()).collect::<Vec<_>>(),["PORT"]);}
#[test] fn test_envdefault_unmanaged_is_new_not_drift(){let r=reconcile("echo \"${LOG_LEVEL:-info}\"\n",&[]);assert!(!r.has_drift());assert_eq!(r.new.iter().map(|c|c.declaration.name.as_str()).collect::<Vec<_>>(),["LOG_LEVEL"]);}
