use std::fs;
use std::path::Path;

use skit_core::{
    AddMode, LibraryRoots, PythonAddRequest, Store, add_python_file, effective_uv_metadata,
    sha256_source_hash,
};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn request(source: &Path) -> PythonAddRequest {
    PythonAddRequest {
        source: source.to_owned(),
        name: None,
        mode: AddMode::Copy,
        description: "Demo script".to_owned(),
        workdir: None,
        dependencies: Vec::new(),
        requires_python: String::new(),
        added_at: "2026-08-08T12:00:00+00:00".to_owned(),
    }
}

#[test]
fn copy_injects_new_pep723_block_and_clears_meta_axes() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"#!/usr/bin/env python3\r\nprint('ok')\r\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.dependencies = vec![" rich>=13 ".to_owned(), "".to_owned()];
    input.requires_python = ">=3.12,<3.13".to_owned();

    let entry = add_python_file(&store, input)?;
    assert_eq!(entry.meta.name, "job");
    assert_eq!(entry.meta.kind, "python");
    assert_eq!(entry.meta.mode, "copy");
    assert_eq!(entry.meta.workdir, "invoke");
    assert_eq!(entry.meta.description, "Demo script");
    assert_eq!(entry.meta.source_hash, sha256_source_hash(original));
    assert!(entry.meta.dependencies.is_none());
    assert!(entry.meta.requires_python.is_empty());
    assert_eq!(fs::read(&source)?, original);

    let stored = fs::read(entry.dir.join("script.py"))?;
    let stored_text = String::from_utf8(stored)?;
    assert!(stored_text.contains("# requires-python = \">=3.12,<3.13\"\r\n"));
    assert!(stored_text.contains("#     \"rich>=13\",\r\n"));
    assert!(stored_text.ends_with("\r\nprint('ok')\r\n"));
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["rich>=13".to_owned()], ">=3.12,<3.13".to_owned())
    );
    Ok(())
}

#[test]
fn existing_block_is_byte_identical_and_explicit_meta_axes_win()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"# /// script\n# dependencies = [\"source-dep\"]\n# ///\nprint(1)\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.dependencies = vec!["override-dep".to_owned()];
    input.requires_python = ">=3.13".to_owned();

    let entry = add_python_file(&store, input)?;
    assert_eq!(fs::read(entry.dir.join("script.py"))?, original);
    assert_eq!(entry.meta.dependencies, Some(vec!["override-dep".to_owned()]));
    assert_eq!(entry.meta.requires_python, ">=3.13");
    assert_eq!(
        effective_uv_metadata(&entry),
        (vec!["override-dep".to_owned()], ">=3.13".to_owned())
    );
    Ok(())
}

#[test]
fn non_utf8_copy_keeps_bytes_exact_and_records_metadata_axes()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"print('ok')\n# bad byte: \xff\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.dependencies = vec!["rich".to_owned()];
    input.requires_python = ">=3.12".to_owned();

    let entry = add_python_file(&store, input)?;
    assert_eq!(fs::read(entry.dir.join("script.py"))?, original);
    assert_eq!(entry.meta.source_hash, sha256_source_hash(original));
    assert_eq!(entry.meta.dependencies, Some(vec!["rich".to_owned()]));
    assert_eq!(entry.meta.requires_python, ">=3.12");
    Ok(())
}

#[test]
fn reference_never_modifies_original_and_records_axes_in_meta()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"print('ok')\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.mode = AddMode::Reference;
    input.dependencies = vec!["rich".to_owned()];
    input.requires_python = ">=3.12".to_owned();

    let entry = add_python_file(&store, input)?;
    assert_eq!(entry.meta.mode, "reference");
    assert_eq!(entry.meta.workdir, "origin");
    assert_eq!(entry.meta.dependencies, Some(vec!["rich".to_owned()]));
    assert_eq!(entry.meta.requires_python, ">=3.12");
    assert_eq!(fs::read(&source)?, original);
    assert!(!entry.dir.join("script.py").exists());
    assert_eq!(entry.script_path(), source.canonicalize()?);
    Ok(())
}

#[test]
fn explicit_copy_workdir_wins_and_empty_metadata_does_not_inject_block()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("job.py");
    let original = b"print('ok')\n";
    fs::write(&source, original)?;
    let store = Store::new(roots(root.path()));
    let mut input = request(&source);
    input.workdir = Some("store".to_owned());

    let entry = add_python_file(&store, input)?;
    assert_eq!(entry.meta.workdir, "store");
    assert_eq!(fs::read(entry.dir.join("script.py"))?, original);
    assert!(entry.meta.dependencies.is_none());
    assert!(entry.meta.requires_python.is_empty());
    Ok(())
}

#[cfg(unix)]
#[test]
fn python_copy_preserves_source_permission_bits() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir()?;
    let source = root.path().join("job.py");
    fs::write(&source, "print(1)\n")?;
    fs::set_permissions(&source, fs::Permissions::from_mode(0o751))?;
    let store = Store::new(roots(root.path()));

    let entry = add_python_file(&store, request(&source))?;
    let source_mode = fs::metadata(&source)?.permissions().mode() & 0o777;
    let stored_mode = fs::metadata(entry.dir.join("script.py"))?.permissions().mode() & 0o777;
    assert_eq!(stored_mode, source_mode);
    Ok(())
}
