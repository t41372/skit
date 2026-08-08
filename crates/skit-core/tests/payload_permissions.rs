use std::fs;
use std::path::Path;

use skit_core::{AddFileRequest, AddMode, AddPreparation, LibraryRoots, Store, add_file};
use tempfile::tempdir;

fn roots(root: &Path) -> LibraryRoots {
    LibraryRoots::new(root.join("data"), root.join("state"), root.join("config"))
}

fn request(source: &Path) -> AddFileRequest {
    AddFileRequest {
        source: source.to_owned(),
        name: Some("mode-test".to_owned()),
        kind: Some("shell".to_owned()),
        mode: AddMode::Copy,
        description: Some(String::new()),
        workdir: None,
        interpreter: None,
        preparation: AddPreparation {
            added_at: "2026-08-08T12:00:00+00:00".to_owned(),
        },
    }
}

#[cfg(unix)]
#[test]
fn copy_mode_preserves_exact_unix_permission_bits() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir()?;
    let source = root.path().join("tool.sh");
    fs::write(&source, b"#!/bin/sh\necho ok\n")?;
    fs::set_permissions(&source, fs::Permissions::from_mode(0o751))?;
    let store = Store::new(roots(root.path()));

    let entry = add_file(&store, request(&source))?;
    let target = entry.dir.join("script.sh");
    let source_mode = fs::metadata(&source)?.permissions().mode() & 0o777;
    let target_mode = fs::metadata(&target)?.permissions().mode() & 0o777;
    assert_eq!(source_mode, 0o751);
    assert_eq!(target_mode, source_mode);
    Ok(())
}

#[cfg(windows)]
#[test]
fn copy_mode_preserves_windows_readonly_attribute() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("tool.sh");
    fs::write(&source, b"echo ok\r\n")?;
    let mut permissions = fs::metadata(&source)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(&source, permissions)?;
    let store = Store::new(roots(root.path()));

    let entry = add_file(&store, request(&source))?;
    let target = entry.dir.join("script.sh");
    assert!(fs::metadata(&target)?.permissions().readonly());

    let mut source_permissions = fs::metadata(&source)?.permissions();
    source_permissions.set_readonly(false);
    fs::set_permissions(&source, source_permissions)?;
    let mut target_permissions = fs::metadata(&target)?.permissions();
    target_permissions.set_readonly(false);
    fs::set_permissions(&target, target_permissions)?;
    Ok(())
}
