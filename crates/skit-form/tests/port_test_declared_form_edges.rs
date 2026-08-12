//! Exact-name form edge ports from Python v0.4 `tests/test_declared_params.py`.

use skit_domain::{
    EntrySettings,
    parameters::{ParamDecl, ParameterDelivery},
};
use skit_form::{FormSource, form_plan};

fn env(name: &str) -> ParamDecl {
    let mut declaration = ParamDecl::new(name);
    declaration.delivery = ParameterDelivery::Env;
    declaration
}

#[test]
fn test_unknown_kind_entry_still_gets_none_plan() {
    let plan = form_plan("martian", "", &EntrySettings::default());
    assert_eq!(plan.source, FormSource::None);
    assert!(plan.declarations().is_empty());
}

#[test]
fn test_exe_with_only_placeholder_rows_falls_through_to_none() {
    let mut slot = ParamDecl::new("slot");
    slot.delivery = ParameterDelivery::Placeholder;
    let settings = EntrySettings {
        parameters: vec![slot],
        ..EntrySettings::default()
    };
    let plan = form_plan("exe", "", &settings);
    assert_eq!(plan.source, FormSource::None);
    assert!(plan.declarations().is_empty());
}

#[test]
fn test_reader_kind_declared_env_rider_merges_not_erases() {
    let settings = EntrySettings {
        parameters: vec![env("LOGLEVEL")],
        ..EntrySettings::default()
    };
    let plan = form_plan("powershell", "param([string]$Region)\n", &settings);
    assert_eq!(
        plan.source,
        FormSource::Reader,
        "a declared env rider must not replace the script's readable param() surface"
    );
    assert_eq!(
        plan.declarations()
            .iter()
            .map(|declaration| (declaration.name.as_str(), declaration.delivery))
            .collect::<Vec<_>>(),
        [
            ("Region", ParameterDelivery::Flag),
            ("LOGLEVEL", ParameterDelivery::Env),
        ]
    );
}

#[test]
fn test_reader_kind_declared_rows_stand_alone_when_no_readable_surface() {
    let settings = EntrySettings {
        parameters: vec![env("LOGLEVEL")],
        ..EntrySettings::default()
    };
    let plan = form_plan("powershell", "Write-Output 'hi'\n", &settings);
    assert_eq!(plan.source, FormSource::Declared);
    assert_eq!(
        plan.declarations()
            .iter()
            .map(|declaration| (declaration.name.as_str(), declaration.delivery))
            .collect::<Vec<_>>(),
        [("LOGLEVEL", ParameterDelivery::Env)]
    );
}
