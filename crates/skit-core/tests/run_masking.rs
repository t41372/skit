use std::collections::BTreeMap;
use std::env;

use skit_core::{
    Delivery, Entry, EntryState, LaunchOptions, ParamDecl, Platform, RunRequest, ScriptMeta,
    prepare_raw_run, prepare_run,
};
use tempfile::tempdir;

fn entry(root: &std::path::Path, decls: &[ParamDecl]) -> Result<Entry, Box<dyn std::error::Error>> {
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
            parameters: Some(decls.iter().map(ParamDecl::to_meta_table).collect()),
            extra: Default::default(),
        },
        dir: root.join("data/scripts/demo"),
    })
}

#[test]
fn masked_launch_redacts_secret_argv_and_environment() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(
        root.path(),
        &[
            ParamDecl {
                name: "SECRET_ARG".to_owned(),
                delivery: Delivery::Flag,
                flag: "--secret".to_owned(),
                secret: true,
                ..ParamDecl::default()
            },
            ParamDecl {
                name: "SECRET_ENV".to_owned(),
                delivery: Delivery::Env,
                secret: true,
                ..ParamDecl::default()
            },
        ],
    )?;
    let state = EntryState::default();
    let explicit = BTreeMap::from([
        ("SECRET_ARG".to_owned(), "arg-secret".to_owned()),
        ("SECRET_ENV".to_owned(), "env-secret".to_owned()),
    ]);
    let environment = BTreeMap::new();
    let extra = Vec::new();
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let programs = |_name: &str| None;

    let prepared = prepare_run(
        &entry,
        RunRequest {
            state: &state,
            preset: None,
            explicit: &explicit,
            extra_args: &extra,
            environment: &environment,
            launch_options: &options,
        },
        &programs,
    )?;
    assert!(prepared.launch.argv.iter().any(|arg| arg == "arg-secret"));
    assert!(
        !prepared
            .masked_launch
            .argv
            .iter()
            .any(|arg| arg == "arg-secret")
    );
    assert!(prepared.masked_launch.argv.iter().any(|arg| arg == "•••"));
    assert_eq!(prepared.launch.env_overlay["SECRET_ENV"], "env-secret");
    assert_eq!(prepared.masked_launch.env_overlay["SECRET_ENV"], "•••");
    Ok(())
}

#[test]
fn raw_launch_bypasses_required_form_values_and_only_forwards_supplied_tail()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = entry(
        root.path(),
        &[ParamDecl {
            name: "REQUIRED".to_owned(),
            delivery: Delivery::Env,
            required: true,
            ..ParamDecl::default()
        }],
    )?;
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let programs = |_name: &str| None;
    let launch = prepare_raw_run(
        &entry,
        &["--literal".to_owned(), "value".to_owned()],
        &options,
        &programs,
    )?;
    assert_eq!(launch.argv[launch.argv.len() - 2..], ["--literal", "value"]);
    assert!(launch.env_overlay.is_empty());
    Ok(())
}
