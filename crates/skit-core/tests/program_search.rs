use std::fs;
use std::path::PathBuf;

use skit_core::{Platform, ProgramResolver, ProgramSearch};
use tempfile::tempdir;

#[cfg(unix)]
#[test]
fn posix_search_uses_path_order_and_execute_bit() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir()?;
    let first = root.path().join("first");
    let second = root.path().join("second");
    fs::create_dir_all(&first)?;
    fs::create_dir_all(&second)?;
    fs::write(first.join("tool"), b"first")?;
    fs::set_permissions(first.join("tool"), fs::Permissions::from_mode(0o644))?;
    fs::write(second.join("tool"), b"second")?;
    fs::set_permissions(second.join("tool"), fs::Permissions::from_mode(0o755))?;

    let search = ProgramSearch::new(
        Platform::Linux,
        vec![first, second.clone()],
        Vec::<String>::new(),
    );
    assert_eq!(search.resolve("tool"), Some(second.join("tool")));
    Ok(())
}

#[test]
fn windows_search_uses_pathext_case_insensitively_without_posix_execute_bits()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin)?;
    fs::write(bin.join("tool.EXE"), b"program")?;

    let search = ProgramSearch::new(
        Platform::Windows,
        vec![bin.clone()],
        vec![".exe".to_owned(), ".CMD".to_owned()],
    );
    assert_eq!(search.resolve("tool"), Some(bin.join("tool.EXE")));
    assert_eq!(search.resolve("tool.EXE"), Some(bin.join("tool.EXE")));
    Ok(())
}

#[test]
fn windows_search_does_not_claim_arbitrary_extensions() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let bin = root.path().join("bin");
    fs::create_dir_all(&bin)?;
    fs::write(bin.join("tool.py"), b"print(1)")?;

    let search = ProgramSearch::new(
        Platform::Windows,
        vec![bin],
        vec![".EXE".to_owned(), ".CMD".to_owned()],
    );
    assert_eq!(search.resolve("tool.py"), None);
    Ok(())
}

#[test]
fn explicit_path_is_checked_without_searching_other_directories()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let path = root.path().join("direct.CMD");
    fs::write(&path, b"@echo off\r\n")?;
    let search = ProgramSearch::new(
        Platform::Windows,
        vec![PathBuf::from("/should/not/be/used")],
        vec![".CMD".to_owned()],
    );
    assert_eq!(
        search.resolve(path.to_string_lossy().as_ref()),
        Some(path)
    );
    Ok(())
}
