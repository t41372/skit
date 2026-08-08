use std::{fs, path::PathBuf};

use skit_application::glob_expansion::GlobExpander;
use skit_store::FileGlobExpander;
use tempfile::TempDir;

#[test]
fn relative_patterns_match_against_the_configured_cwd_and_return_relative_paths() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("b.txt"), b"").unwrap();
    fs::write(root.path().join("a.txt"), b"").unwrap();
    fs::write(root.path().join("c.md"), b"").unwrap();
    fs::write(root.path().join(".hidden.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    assert_eq!(glob.expand_piece("*.txt"), ["a.txt", "b.txt"]);
    assert_eq!(glob.expand_piece("*.md"), ["c.md"]);
    assert_eq!(glob.expand_piece(".*.txt"), [".hidden.txt"]);
}

#[test]
fn recursive_patterns_are_sorted_and_keep_platform_native_relative_spelling() {
    let root = TempDir::new().unwrap();
    fs::create_dir_all(root.path().join("nested/deeper")).unwrap();
    fs::write(root.path().join("nested/b.rs"), b"").unwrap();
    fs::write(root.path().join("nested/deeper/a.rs"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    let expected = [
        PathBuf::from("nested/b.rs").display().to_string(),
        PathBuf::from("nested/deeper/a.rs").display().to_string(),
    ];
    assert_eq!(glob.expand_piece("**/*.rs"), expected);
}

#[test]
fn no_match_invalid_pattern_and_literal_values_fall_back_to_the_original_piece() {
    let root = TempDir::new().unwrap();
    let glob = FileGlobExpander::new(root.path());

    assert_eq!(glob.expand_piece("missing-*.txt"), ["missing-*.txt"]);
    assert_eq!(glob.expand_piece("["), ["["]);
    assert_eq!(glob.expand_piece("plain.txt"), ["plain.txt"]);
}

#[test]
fn absolute_patterns_return_absolute_matches() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("one.log"), b"").unwrap();
    fs::write(root.path().join("two.log"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    let pattern = root.path().join("*.log").display().to_string();

    assert_eq!(
        glob.expand_piece(&pattern),
        [
            root.path().join("one.log").display().to_string(),
            root.path().join("two.log").display().to_string(),
        ]
    );
}
