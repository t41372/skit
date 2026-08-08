use std::fs;
use std::path::Path;

use skit_core::{
    AddMode, Binding, Delivery, LibraryRoots, ParamDecl, ParamDefault, ParamType, PythonAddError,
    PythonAddRequest, Store, add_python_file_with_params, effective_uv_metadata,
    read_python_params, sha256_source_hash,
};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn request(source: &Path) -> PythonAddRequest {
    PythonAddRequest {
        source: source.to_owned(),
        name: Some("managed".to_owned()),
        mode: AddMode::Copy,
        description: "Managed demo".to_owned(),
        workdir: None,
        dependencies: vec![" rich>=13 ".to_owned()],
        requires_python: ">=3.12".to_owned(),
        added_at: "2026-08-08T12:00:00+00:00".to_owned(),
    }
}

fn city_param() -> ParamDecl {
    ParamDecl {
        name: "CITY".to_owned(),
        binding: Binding::Const,
        delivery: Delivery::Inject,
        param_type: ParamType::String,
        default: Some(ParamDefault::String("Taipei".to_owned())),
        prompt: "Which city?".to_owned(),
        ..ParamDecl::default()
    }
}

#[test]
fn copy_commits_uv_metadata_and_frozen_params_in_one_payload()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"#!/usr/bin/env python3\r\nCITY = 'Taipei'\r\nprint(CITY)\r\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));

    let entry = add_python_file_with_params(&store, request(&source), &[city_param()])?;
    assert_eq!(entry.meta.source_hash, sha256_source_hash(original));
    assert_eq!(fs::read(&source)?, original);
    assert!(entry.meta.dependencies.is_none());
    assert!(entry.meta.requires_python.is_empty());

    let stored = fs::read_to_string(entry.dir.join("script.py"))?;
    assert!(stored.contains("# requires-python = \">=3.12\"\r\n"));
    assert!(stored.contains("#     \"rich>=13\",\r\n"));
    assert!(stored.contains("# [tool.skit]\r\n"));
    assert!(stored.contains("# name = \"CITY\"\r\n"));
    assert!(stored.ends_with("CITY = 'Taipei'\r\nprint(CITY)\r\n"));
    assert_eq!(read_python_params(&stored), vec![city_param()]);
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["rich>=13".to_owned()], ">=3.12".to_owned())
    );
    Ok(())
}

#[test]
fn existing_pep723_lines_survive_while_tool_skit_is_replaced()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = concat!(
        "# /// script\n",
        "# dependencies = [\"source-dep\"]\n",
        "# [tool.other]\n",
        "# answer = 42\n",
        "# ///\n",
        "CITY = 'Taipei'\n",
    );
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.dependencies = Vec::new();
    input.requires_python.clear();

    let entry = add_python_file_with_params(&store, input, &[city_param()])?;
    let stored = fs::read_to_string(entry.dir.join("script.py"))?;
    assert!(stored.contains("# dependencies = [\"source-dep\"]\n"));
    assert!(stored.contains("# [tool.other]\n# answer = 42\n"));
    assert!(stored.contains("# [tool.skit]\n"));
    assert_eq!(read_python_params(&stored), vec![city_param()]);
    Ok(())
}

#[test]
fn reference_with_new_managed_schema_refuses_before_any_store_write()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"CITY = 'Taipei'\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.mode = AddMode::Reference;

    let result = add_python_file_with_params(&store, input, &[city_param()]);
    assert!(matches!(
        result,
        Err(PythonAddError::ManagedParametersRequireCopy)
    ));
    assert_eq!(fs::read(&source)?, original);
    assert!(store.list()?.is_empty());
    Ok(())
}

#[test]
fn non_utf8_with_new_managed_schema_refuses_without_lossy_reencode_or_store_write()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"CITY = 'Taipei'\n# bad: \xff\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));

    let result = add_python_file_with_params(&store, request(&source), &[city_param()]);
    assert!(matches!(
        result,
        Err(PythonAddError::ManagedParametersRequireUtf8)
    ));
    assert_eq!(fs::read(&source)?, original);
    assert!(store.list()?.is_empty());
    Ok(())
}
