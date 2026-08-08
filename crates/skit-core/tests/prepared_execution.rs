use std::collections::BTreeMap;
use std::fs;

use skit_core::{
    Binding, Delivery, Entry, EntryState, LaunchOptions, ParamDecl, ParamDefault, ParamType,
    Platform, PrepareExecutionError, PythonInjectError, RunRequest, ScriptMeta, prepare_execution,
    write_python_params,
};
use tempfile::tempdir;

fn python_entry(root: &std::path::Path, text: &str) -> Result<Entry, Box<dyn std::error::Error>> {
    let dir = root.join("data/scripts/demo");
    fs::create_dir_all(&dir)?;
    fs::write(dir.join("script.py"), text)?;
    Ok(Entry {
        slug: "demo".to_owned(),
        meta: ScriptMeta {
            schema: 1,
            name: "Demo".to_owned(),
            kind: "python".to_owned(),
            mode: "copy".to_owned(),
            source: root.join("original.py").to_string_lossy().into_owned(),
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
            parameters: None,
            extra: Default::default(),
        },
        dir,
    })
}

#[test]
fn managed_const_run_materializes_one_ephemeral_snapshot_without_touching_store()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let params = vec![ParamDecl {
        name: "CITY".to_owned(),
        binding: Binding::Const,
        delivery: Delivery::Inject,
        param_type: ParamType::String,
        default: Some(ParamDefault::String("Taipei".to_owned())),
        ..ParamDecl::default()
    }];
    let stored_text = write_python_params("CITY = 'Taipei'\nprint(CITY)\n", &params);
    let entry = python_entry(root.path(), &stored_text)?;
    let state = EntryState::default();
    let explicit = BTreeMap::from([("CITY".to_owned(), "Paris".to_owned())]);
    let environment = BTreeMap::new();
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let uv = root.path().join("fake-uv");
    let programs = |name: &str| (name == "uv").then(|| uv.clone());
    let temp_path;
    {
        let execution = prepare_execution(
            &entry,
            RunRequest {
                state: &state,
                preset: None,
                explicit: &explicit,
                extra_args: &[],
                environment: &environment,
                launch_options: &options,
            },
            &programs,
        )?;
        temp_path = execution
            .injected_path()
            .ok_or("expected injected temp snapshot")?
            .to_owned();
        assert!(temp_path.exists());
        let injected = fs::read_to_string(&temp_path)?;
        assert!(injected.contains("CITY = \"Paris\""));
        assert_eq!(fs::read_to_string(entry.script_path())?, stored_text);

        let actual_script = execution
            .run
            .launch
            .argv
            .windows(2)
            .find(|pair| pair[0] == "--script")
            .map(|pair| pair[1].as_str());
        let masked_script = execution
            .run
            .masked_launch
            .argv
            .windows(2)
            .find(|pair| pair[0] == "--script")
            .map(|pair| pair[1].as_str());
        assert_eq!(actual_script, Some(temp_path.to_string_lossy().as_ref()));
        assert_eq!(masked_script, actual_script);
    }
    assert!(!temp_path.exists());
    Ok(())
}

#[test]
fn supplied_managed_input_refuses_before_a_temp_snapshot_exists()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let params = vec![ParamDecl {
        name: "input-1".to_owned(),
        binding: Binding::Input,
        delivery: Delivery::Inject,
        prompt: "Name: ".to_owned(),
        order: 0,
        ..ParamDecl::default()
    }];
    let entry = python_entry(
        root.path(),
        &write_python_params("name = input('Name: ')\n", &params),
    )?;
    let state = EntryState::default();
    let explicit = BTreeMap::from([("input-1".to_owned(), "Ada".to_owned())]);
    let environment = BTreeMap::new();
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let uv = root.path().join("fake-uv");
    let programs = |name: &str| (name == "uv").then(|| uv.clone());
    let result = prepare_execution(
        &entry,
        RunRequest {
            state: &state,
            preset: None,
            explicit: &explicit,
            extra_args: &[],
            environment: &environment,
            launch_options: &options,
        },
        &programs,
    );
    assert!(matches!(
        result,
        Err(PrepareExecutionError::PythonInject(
            PythonInjectError::ManagedInputUnsupported(name)
        )) if name == "input-1"
    ));
    Ok(())
}

#[test]
fn unmanaged_python_keeps_the_stored_script_without_temp_materialization()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = python_entry(root.path(), "print('ok')\n")?;
    let state = EntryState::default();
    let explicit = BTreeMap::new();
    let environment = BTreeMap::new();
    let options = LaunchOptions::new(Platform::Linux, root.path());
    let uv = root.path().join("fake-uv");
    let programs = |name: &str| (name == "uv").then(|| uv.clone());
    let execution = prepare_execution(
        &entry,
        RunRequest {
            state: &state,
            preset: None,
            explicit: &explicit,
            extra_args: &[],
            environment: &environment,
            launch_options: &options,
        },
        &programs,
    )?;
    assert!(execution.injected_path().is_none());
    assert_eq!(
        execution.run.launch.argv.last().map(String::as_str),
        Some(entry.script_path().to_string_lossy().as_ref())
    );
    Ok(())
}
