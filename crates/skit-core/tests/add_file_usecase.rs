use std::fs;
use std::path::{Path, PathBuf};

use skit_core::{
    AddFileRequest, AddMode, AddPreparation, AddUseCaseError, LibraryRoots, Store, add_file,
};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn preparation() -> AddPreparation {
    AddPreparation {
        source_hash: "sha256:known".to_owned(),
        added_at: "2026-08-08T04:05:06+00:00".to_owned(),
    }
}

fn request(source: PathBuf) -> AddFileRequest {
    AddFileRequest {
        source,
        name: None,
        kind: None,
        mode: AddMode::Copy,
        description: None,
        workdir: None,
        interpreter: None,
        preparation: preparation(),
    }
}

#[test]
fn shell_copy_infers_kind_interpreter_description_and_invoke_workdir()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("deploy.sh");
    let bytes = b"#!/usr/bin/env zsh\n# Ship it\necho hi\n";
    fs::write(&source, bytes)?;
    let store = Store::new(roots(root.path()));

    let entry = add_file(&store, request(source.clone()))?;

    assert_eq!(entry.meta.name, "deploy");
    assert_eq!(entry.meta.kind, "shell");
    assert_eq!(entry.meta.mode, "copy");
    assert_eq!(entry.meta.source, source.canonicalize()?.to_string_lossy());
    assert_eq!(entry.meta.source_hash, "sha256:known");
    assert_eq!(entry.meta.added_at, "2026-08-08T04:05:06+00:00");
    assert_eq!(entry.meta.workdir, "invoke");
    assert_eq!(entry.meta.description, "Ship it");
    assert_eq!(entry.meta.interpreter, "zsh");
    assert_eq!(fs::read(entry.dir.join("script.sh"))?, bytes);
    Ok(())
}

#[test]
fn reference_mode_keeps_source_in_place_and_defaults_to_origin()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("task.rb");
    fs::write(&source, "# A task\nputs 'ok'\n")?;
    let store = Store::new(roots(root.path()));
    let mut input = request(source.clone());
    input.mode = AddMode::Reference;

    let entry = add_file(&store, input)?;

    assert_eq!(entry.meta.kind, "ruby");
    assert_eq!(entry.meta.mode, "reference");
    assert_eq!(entry.meta.workdir, "origin");
    assert_eq!(entry.meta.description, "A task");
    assert!(!entry.dir.join("script.rb").exists());
    assert_eq!(entry.script_path(), source.canonicalize()?);
    Ok(())
}

#[test]
fn explicit_kind_handles_extensionless_script_and_explicit_fields_win()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("build");
    fs::write(&source, "echo building\n")?;
    let store = Store::new(roots(root.path()));
    let mut input = request(source);
    input.kind = Some("shell".to_owned());
    input.name = Some("Builder".to_owned());
    input.description = Some("Explicit description".to_owned());
    input.workdir = Some("/workspace".to_owned());
    input.interpreter = Some("bash".to_owned());

    let entry = add_file(&store, input)?;

    assert_eq!(entry.meta.name, "Builder");
    assert_eq!(entry.meta.kind, "shell");
    assert_eq!(entry.meta.description, "Explicit description");
    assert_eq!(entry.meta.workdir, "/workspace");
    assert_eq!(entry.meta.interpreter, "bash");
    assert!(entry.dir.join("script.sh").is_file());
    Ok(())
}

#[test]
fn explicit_unknown_or_non_file_lane_kind_is_refused_before_writing()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("tool");
    fs::write(&source, "data\n")?;
    let store = Store::new(roots(root.path()));

    for kind in ["cobol", "command", "prompt"] {
        let mut input = request(source.clone());
        input.kind = Some(kind.to_owned());
        let result = add_file(&store, input);
        assert!(matches!(result, Err(AddUseCaseError::UnsupportedKind(value)) if value == kind));
    }
    assert!(store.list()?.is_empty());
    Ok(())
}

#[test]
fn unknown_inference_is_a_named_error_and_leaves_no_entry() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempdir()?;
    let source = root.path().join("notes");
    fs::write(&source, "plain text\n")?;
    let store = Store::new(roots(root.path()));

    let result = add_file(&store, request(source));

    assert!(matches!(result, Err(AddUseCaseError::UnknownKind)));
    assert!(store.list()?.is_empty());
    Ok(())
}

#[test]
fn missing_source_fails_before_registry_or_script_directory_is_created() {
    let Ok(root) = tempdir() else {
        panic!("failed to create temporary directory");
    };
    let store = Store::new(roots(root.path()));
    let result = add_file(&store, request(root.path().join("missing.sh")));
    assert!(matches!(result, Err(AddUseCaseError::SourceNotFile(_))));
    assert!(!store.roots().data_dir().join("registry.toml").exists());
    assert!(!store.roots().data_dir().join("scripts").exists());
}
