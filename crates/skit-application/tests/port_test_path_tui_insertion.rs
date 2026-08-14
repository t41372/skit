use skit_application::path_insertion::{
    ArgumentDialect, RunPathInsertMode, insert_picked_path_for_dialect,
};

#[test]
fn test_insert_picked_shapes() {
    assert_eq!(
        insert_picked_path_for_dialect(
            "old.csv",
            "new.csv",
            RunPathInsertMode::Replace,
            ArgumentDialect::Posix,
        )
        .expect("replace path"),
        "new.csv"
    );
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "a b.txt",
            RunPathInsertMode::Shlex,
            ArgumentDialect::Windows,
        )
        .expect("shlex path"),
        "'a b.txt'",
        "multiple-value fields use POSIX shlex even on Windows"
    );
    assert_eq!(
        insert_picked_path_for_dialect(
            "--verbose",
            "a b.txt",
            RunPathInsertMode::Arguments,
            ArgumentDialect::Windows,
        )
        .expect("Windows argv path"),
        "--verbose \"a b.txt\"",
        "extra arguments use the native editable-argv dialect"
    );
    assert_eq!(
        insert_picked_path_for_dialect(
            "--verbose",
            "a b.txt",
            RunPathInsertMode::Arguments,
            ArgumentDialect::Posix,
        )
        .expect("POSIX argv path"),
        "--verbose 'a b.txt'"
    );
}

#[test]
fn test_insert_picked_escapes_glob_metacharacters() {
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "data[1].csv",
            RunPathInsertMode::Shlex,
            ArgumentDialect::Posix,
        )
        .expect("literal glob path"),
        "'data[[]1].csv'"
    );
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "data[1].csv",
            RunPathInsertMode::Arguments,
            ArgumentDialect::Posix,
        )
        .expect("literal argv path"),
        "'data[[]1].csv'"
    );
    assert_eq!(
        insert_picked_path_for_dialect(
            "",
            "data[1].csv",
            RunPathInsertMode::Arguments,
            ArgumentDialect::Windows,
        )
        .expect("literal Windows argv path"),
        "data[[]1].csv"
    );
}
