//! Remaining exact public form-plan projections from Python v0.4 `tests/test_flows.py`.

use skit_domain::{EntrySettings, parameters::{ParamDecl, ParameterBinding, ParameterDelivery}};
use skit_form::{FormSource, form_plan};
use skit_language::{DegradationReason, write_managed_params};

const ARGPARSE: &str = r#"import argparse
ap = argparse.ArgumentParser()
ap.add_argument('--name')
ap.parse_args()
"#;

#[test]
fn test_plan_plain_script_is_none() {
    let source = "print('plain')\n";
    let plan = form_plan("python", source, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::None);
    assert!(plan.fields.is_empty());
    assert!(plan.drift.is_empty());
    assert_eq!(plan.degradation, None);
    assert_eq!(source.as_bytes(), b"print('plain')\n");
}

#[test]
fn test_plan_sources_are_exact_per_field() {
    let settings = EntrySettings { params: vec!["msg".to_owned()], ..EntrySettings::default() };
    let plan = form_plan("command", "echo {msg}", &settings);
    assert_eq!(plan.source, FormSource::Command);
    assert_eq!(plan.fields.len(), 1);
    assert_eq!(plan.fields[0].declaration.delivery, ParameterDelivery::Placeholder);
}

#[test]
fn test_plan_field_sources_inject_and_flag() {
    let mut city = ParamDecl::new("CITY");
    city.binding = ParameterBinding::Const;
    city.delivery = ParameterDelivery::Inject;
    let managed = write_managed_params("python", "CITY = 'x'\n", &[city]).unwrap();
    let inject = form_plan("python", &managed, &EntrySettings::default());
    assert_eq!(inject.source, FormSource::Inject);
    assert!(inject.fields.iter().all(|field| field.declaration.delivery == ParameterDelivery::Inject));

    let flag = form_plan("python", ARGPARSE, &EntrySettings::default());
    assert_eq!(flag.source, FormSource::Reader);
    assert!(flag.fields.iter().all(|field| field.declaration.delivery == ParameterDelivery::Flag));
}

#[test]
fn test_plan_subparsers_degrades_with_reason() {
    let source = concat!(
        "import argparse\n",
        "ap = argparse.ArgumentParser()\n",
        "sub = ap.add_subparsers()\n",
        "p = sub.add_parser('x')\n",
        "p.add_argument('--y')\n",
    );
    let plan = form_plan("python", source, &EntrySettings::default());
    assert_eq!(plan.source, FormSource::Reader);
    assert_eq!(plan.degradation, Some(DegradationReason::Subcommands));
    assert!(plan.fields.is_empty());
    assert_eq!(source.as_bytes(), source.as_bytes());
}
