use std::fs;

use skit_core::{
    Binding, Delivery, Entry, ParamDecl, ParamDefault, ParamType, PlanSource, ScriptMeta,
    plan_for_entry, write_python_params,
};
use tempfile::tempdir;

fn entry(root: &std::path::Path, mode: &str, source: &std::path::Path) -> Entry {
    Entry {
        slug: "demo".to_owned(),
        meta: ScriptMeta {
            schema: 1,
            name: "Demo".to_owned(),
            kind: "python".to_owned(),
            mode: mode.to_owned(),
            source: source.to_string_lossy().into_owned(),
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
        dir: root.join("data/scripts/demo"),
    }
}

#[test]
fn frozen_python_params_become_managed_inject_fields() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let stored = root.path().join("data/scripts/demo/script.py");
    fs::create_dir_all(stored.parent().ok_or("missing parent")?)?;
    let params = vec![
        ParamDecl {
            name: "CITY".to_owned(),
            binding: Binding::Const,
            delivery: Delivery::Inject,
            param_type: ParamType::String,
            default: Some(ParamDefault::String("Taipei".to_owned())),
            prompt: "Which city?".to_owned(),
            ..ParamDecl::default()
        },
        ParamDecl {
            name: "input-1".to_owned(),
            binding: Binding::Input,
            delivery: Delivery::Inject,
            param_type: ParamType::String,
            prompt: "Password: ".to_owned(),
            order: 0,
            secret: true,
            env_source: "APP_PASSWORD".to_owned(),
            ..ParamDecl::default()
        },
    ];
    fs::write(&stored, write_python_params("CITY = 'Taipei'\n", &params))?;
    let plan = plan_for_entry(&entry(root.path(), "copy", &stored));

    assert_eq!(plan.source, PlanSource::Managed);
    assert_eq!(plan.source.as_str(), "inject");
    assert_eq!(plan.source.origin(), "managed");
    assert_eq!(plan.fields.len(), 2);
    assert_eq!(plan.fields[0].key, "CITY");
    assert_eq!(plan.fields[0].label, "Which city?");
    assert_eq!(plan.fields[0].default.as_deref(), Some("Taipei"));
    assert_eq!(plan.fields[0].delivery, Delivery::Inject);
    assert_eq!(plan.fields[1].key, "input-1");
    assert!(plan.fields[1].input_binding);
    assert!(plan.fields[1].secret);
    assert_eq!(plan.fields[1].env_source, "APP_PASSWORD");
    Ok(())
}

#[test]
fn reference_python_reads_the_original_frozen_schema() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("original.py");
    let params = vec![ParamDecl {
        name: "COUNT".to_owned(),
        binding: Binding::Const,
        delivery: Delivery::Inject,
        param_type: ParamType::Integer,
        default: Some(ParamDefault::Integer(3)),
        ..ParamDecl::default()
    }];
    fs::write(&source, write_python_params("COUNT = 3\n", &params))?;
    let plan = plan_for_entry(&entry(root.path(), "reference", &source));
    assert_eq!(plan.source, PlanSource::Managed);
    assert_eq!(plan.fields[0].key, "COUNT");
    assert_eq!(plan.fields[0].default.as_deref(), Some("3"));
    Ok(())
}

#[test]
fn malformed_or_non_utf8_python_source_invents_no_managed_fields()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("bad.py");
    fs::write(
        &source,
        b"# /// script\n# tool = 5\n# ///\nprint('ok')\n",
    )?;
    let malformed = plan_for_entry(&entry(root.path(), "reference", &source));
    assert_eq!(malformed.source, PlanSource::None);
    assert!(malformed.fields.is_empty());

    fs::write(&source, [0xff, 0xfe, b'\n'])?;
    let binary = plan_for_entry(&entry(root.path(), "reference", &source));
    assert_eq!(binary.source, PlanSource::None);
    assert!(binary.fields.is_empty());
    Ok(())
}
