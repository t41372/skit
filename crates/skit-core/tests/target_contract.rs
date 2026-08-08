use std::path::PathBuf;

use skit_core::EntrySummary;
use tempfile::tempdir;

fn summary(kind: &str, mode: &str, dir: PathBuf, source: &str) -> EntrySummary {
    EntrySummary {
        slug: "demo".to_owned(),
        name: "Demo".to_owned(),
        kind: kind.to_owned(),
        mode: mode.to_owned(),
        description: String::new(),
        source: source.to_owned(),
        dir,
    }
}

#[test]
fn copy_mode_targets_the_historical_stored_filename() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let entry = summary(
        "python",
        "copy",
        root.path().to_path_buf(),
        "/old/original.py",
    );
    assert_eq!(entry.script_path(), root.path().join("script.py"));
    assert!(entry.target_missing());
    std::fs::write(root.path().join("script.py"), "print('ok')\n")?;
    assert!(!entry.target_missing());
    Ok(())
}

#[test]
fn reference_mode_targets_the_original_file() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempdir()?;
    let source = root.path().join("original.sh");
    std::fs::write(&source, "echo ok\n")?;
    let entry = summary(
        "shell",
        "reference",
        root.path().join("entry"),
        &source.to_string_lossy(),
    );
    assert_eq!(entry.script_path(), source);
    assert!(!entry.target_missing());
    Ok(())
}

#[test]
fn exe_uses_source_even_if_hand_edited_to_copy_mode() {
    let entry = summary("exe", "copy", PathBuf::from("entry"), "/missing/tool.exe");
    assert!(entry.target_missing());
}

#[test]
fn command_and_unknown_kinds_have_no_checkable_target() {
    let command = summary("command", "copy", PathBuf::from("entry"), "");
    let unknown = summary("future-kind", "copy", PathBuf::from("entry"), "");
    assert!(!command.target_missing());
    assert!(!unknown.target_missing());
}
