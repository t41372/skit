use std::collections::BTreeMap;
use std::env;

use skit_core::{
    AssemblyError, Delivery, Entry, EntryState, FormField, FormPlan, LaunchOptions, ParamDecl,
    ParamDefault, PlanSource, Platform, PrepareRunError, ScriptMeta, prepare_run, remembered_values,
    resolve_extra_args,
};
use tempfile::tempdir;

fn exe_entry(
    root: &std::path::Path,
    decls: &[ParamDecl],
) -> Result<Entry, Box<dyn std::error::Error>> {
    Ok(Entry {
        slug: "demo".to_owned(),
        meta: ScriptMeta {
            schema: 1,
            name: "Demo".to_owned(),
            kind: "exe".to_owned(),
            mode: "reference".to_owned(),
            source: env::current_exe()?.to_string_lossy().into_owned(),
            source_hash: String::new(),
            added_at: String::new(),
            workdir: "invoke".to_owned(),
            description: String::new(),
            template: String::new(),
            dependencies: None,
            requires_python: String::new(),
            params: None,
            interpreter: String::new(),
            runner: String::new(),
            interpolate: true,
            needs: None,
            parameters: (!decls.is_empty())
                .then(|| decls.iter().map(ParamDecl::to_meta_table).collect()),
            extra: Default::default(),
        },
        dir: root.join("data/scripts/demo"),
    })
}

fn env_field(default: Option<&str>, required: bool) -> ParamDecl {
    ParamDecl {
        name: "VALUE".to_owned(),
        delivery: Delivery::Env,
        default: default.map(|value| ParamDefault::String(value.to_owned())),
        required,
        ..ParamDecl::default()
    }
}

#[test]
fn value_resolution_precedence_flows_into_the_launch_environment()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = exe_entry(root.path(), &[env_field(Some("definition"), false)])?;
    let mut state = EntryState::default();
    state
        .values
        .insert("VALUE".to_owned(), "last-used".to_owned());
    state.presets.insert(
        "prod".to_owned(),
        BTreeMap::from([("VALUE".to_owned(), "preset".to_owned())]),
    );
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let programs = |_name: &str| None;

    let from_preset = prepare_run(
        &entry,
        &state,
        Some("prod"),
        &BTreeMap::new(),
        &[],
        &BTreeMap::new(),
        &options,
        &programs,
    )?;
    assert_eq!(from_preset.values["VALUE"], "preset");
    assert_eq!(from_preset.launch.env_overlay["VALUE"], "preset");

    let explicit = BTreeMap::from([("VALUE".to_owned(), "explicit".to_owned())]);
    let prepared = prepare_run(
        &entry,
        &state,
        Some("prod"),
        &explicit,
        &[],
        &BTreeMap::new(),
        &options,
        &programs,
    )?;
    assert_eq!(prepared.values["VALUE"], "explicit");
    assert_eq!(prepared.launch.env_overlay["VALUE"], "explicit");
    Ok(())
}

#[test]
fn unknown_preset_and_unknown_explicit_key_are_named_refusals()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = exe_entry(root.path(), &[env_field(None, false)])?;
    let state = EntryState::default();
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let programs = |_name: &str| None;

    assert!(matches!(
        prepare_run(
            &entry,
            &state,
            Some("missing"),
            &BTreeMap::new(),
            &[],
            &BTreeMap::new(),
            &options,
            &programs,
        ),
        Err(PrepareRunError::UnknownPreset(name)) if name == "missing"
    ));
    assert!(matches!(
        prepare_run(
            &entry,
            &state,
            None,
            &BTreeMap::from([("OTHER".to_owned(), "x".to_owned())]),
            &[],
            &BTreeMap::new(),
            &options,
            &programs,
        ),
        Err(PrepareRunError::Resolve(error)) if error.key == "OTHER"
    ));
    Ok(())
}

#[test]
fn missing_required_value_stops_before_a_launch_snapshot_exists()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = exe_entry(root.path(), &[env_field(None, true)])?;
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let programs = |_name: &str| None;
    let result = prepare_run(
        &entry,
        &EntryState::default(),
        None,
        &BTreeMap::new(),
        &[],
        &BTreeMap::new(),
        &options,
        &programs,
    );
    assert!(matches!(
        result,
        Err(PrepareRunError::Assembly(AssemblyError::InvalidValues(errors)))
            if errors.contains_key("VALUE")
    ));
    Ok(())
}

#[test]
fn extra_args_explicit_replay_and_forget_are_distinct() {
    let mut state = EntryState::default();
    state.extra_args = vec!["--old".to_owned(), "value".to_owned()];

    let replay = resolve_extra_args(&state, &[], false);
    assert_eq!(replay.args, ["--old", "value"]);
    assert!(replay.replayed);

    let explicit = resolve_extra_args(&state, &["--new".to_owned()], false);
    assert_eq!(explicit.args, ["--new"]);
    assert!(!explicit.replayed);

    let forgotten = resolve_extra_args(&state, &[], true);
    assert!(forgotten.args.is_empty());
    assert!(!forgotten.replayed);
}

#[test]
fn remembered_values_do_not_freeze_defaults_but_keep_delivered_empty() {
    let form = FormPlan {
        source: PlanSource::Declared,
        fields: vec![
            FormField::from_decl(&ParamDecl {
                name: "SAME".to_owned(),
                delivery: Delivery::Env,
                default: Some(ParamDefault::String("today".to_owned())),
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "CLEARED".to_owned(),
                delivery: Delivery::Env,
                default: Some(ParamDefault::String("nonempty".to_owned())),
                ..ParamDecl::default()
            }),
            FormField::from_decl(&ParamDecl {
                name: "UNSET".to_owned(),
                delivery: Delivery::Env,
                ..ParamDecl::default()
            }),
        ],
        ..FormPlan::default()
    };
    let values = BTreeMap::from([
        ("SAME".to_owned(), "today".to_owned()),
        ("CLEARED".to_owned(), String::new()),
        ("UNSET".to_owned(), String::new()),
    ]);

    assert_eq!(
        remembered_values(&form, &values),
        BTreeMap::from([("CLEARED".to_owned(), String::new())])
    );
}
