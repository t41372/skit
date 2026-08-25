use std::{fs, path::PathBuf};

use skit_application::{
    form_feedback::{GlobCountPort, GlobCountRequest},
    glob_expansion::GlobExpander,
};
use skit_store::FileGlobExpander;
use tempfile::TempDir;

#[test]
fn test_expand_glob_piece_globs_only_when_glob_chars_present() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("b.txt"), b"").unwrap();
    fs::write(root.path().join("a.txt"), b"").unwrap();
    fs::write(root.path().join("c.md"), b"").unwrap();
    fs::write(root.path().join(".hidden.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());
    assert_eq!(glob.cwd(), root.path());

    assert_eq!(glob.expand_piece("*.txt"), ["a.txt", "b.txt"]);
    assert_eq!(glob.expand_piece("*.md"), ["c.md"]);
    assert_eq!(glob.expand_piece(".*.txt"), [".hidden.txt"]);
    assert_eq!(glob.expand_piece("[ab]?*"), ["a.txt", "b.txt"]);
    assert_eq!(glob.expand_piece("a.txt"), ["a.txt"]);
}

#[test]
fn an_escaped_bracket_pattern_matches_only_the_literal_file() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("data1.csv"), b"").unwrap();
    fs::write(root.path().join("data2.csv"), b"").unwrap();
    fs::write(root.path().join("data[1].csv"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    assert_eq!(glob.expand_piece("data[[]1].csv"), ["data[1].csv"]);
}

#[test]
fn test_expand_glob_piece_supports_recursive_doublestar() {
    let root = TempDir::new().unwrap();
    let nested = root.path().join("nested");
    let deeper = nested.join("deeper");
    fs::create_dir_all(&deeper).unwrap();
    fs::write(nested.join("b.rs"), b"").unwrap();
    fs::write(deeper.join("a.rs"), b"").unwrap();
    fs::write(deeper.join("x.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path());

    let expected = [
        PathBuf::from("nested").join("b.rs").display().to_string(),
        PathBuf::from("nested")
            .join("deeper")
            .join("a.rs")
            .display()
            .to_string(),
    ];
    let rust_pattern = PathBuf::from("**").join("*.rs").display().to_string();
    assert_eq!(glob.expand_piece(&rust_pattern), expected);

    let text_pattern = PathBuf::from("**").join("x.txt").display().to_string();
    assert_eq!(
        glob.expand_piece(&text_pattern),
        [PathBuf::from("nested")
            .join("deeper")
            .join("x.txt")
            .display()
            .to_string()]
    );
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

#[test]
fn live_form_feedback_counts_matches_through_the_same_glob_adapter() {
    let root = TempDir::new().unwrap();
    fs::write(root.path().join("one.txt"), b"").unwrap();
    fs::write(root.path().join("two.txt"), b"").unwrap();
    let glob = FileGlobExpander::new(root.path().join("not-the-request-cwd"));

    assert_eq!(
        glob.count_matches(&GlobCountRequest {
            cwd: root.path().display().to_string(),
            pieces: vec!["*.txt".to_owned(), "literal.md".to_owned()],
        }),
        3
    );
}
