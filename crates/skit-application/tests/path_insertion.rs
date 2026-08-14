use skit_application::path_insertion::{
    ArgumentDialect, RunPathInsertMode, insert_picked_path_for_dialect,
};

#[test]
fn a_single_value_is_replaced_and_parsed_values_append_one_literal_piece() {
    assert_eq!(
        insert_picked_path_for_dialect(
            "old",
            "reports/final.txt",
            RunPathInsertMode::Replace,
            ArgumentDialect::Posix,
        )
        .unwrap(),
        "reports/final.txt"
    );

    let shlex = insert_picked_path_for_dialect(
        "--before",
        "data sets/data*.csv",
        RunPathInsertMode::Shlex,
        ArgumentDialect::Posix,
    )
    .unwrap();
    assert_eq!(
        shlex::split(&shlex).unwrap(),
        ["--before", "data sets/data[*].csv"]
    );

    let args = insert_picked_path_for_dialect(
        "",
        "data sets/data?.csv",
        RunPathInsertMode::Arguments,
        ArgumentDialect::Posix,
    )
    .unwrap();
    assert_eq!(shlex::split(&args).unwrap(), ["data sets/data[?].csv"]);
}

#[test]
fn glob_metacharacters_use_the_same_spelling_in_each_argument_dialect() {
    let picked = "data[1]*?.csv";
    let expected = "data[[]1][*][?].csv";

    assert_eq!(
        insert_picked_path_for_dialect(
            "old",
            picked,
            RunPathInsertMode::Replace,
            ArgumentDialect::Posix,
        )
        .unwrap(),
        picked
    );

    for mode in [RunPathInsertMode::Shlex, RunPathInsertMode::Arguments] {
        let encoded =
            insert_picked_path_for_dialect("", picked, mode, ArgumentDialect::Posix).unwrap();
        assert_eq!(shlex::split(&encoded).unwrap(), [expected]);
    }

    let encoded = insert_picked_path_for_dialect(
        "",
        picked,
        RunPathInsertMode::Arguments,
        ArgumentDialect::Windows,
    )
    .unwrap();
    assert_eq!(split_windows_argument(&encoded), expected);
}

#[test]
fn windows_argument_text_round_trips_quotes_spaces_and_trailing_backslashes() {
    for picked in [
        "plain.txt",
        "two words.txt",
        r#"quote\"inside.txt"#,
        r"C:\Program Files\folder\",
    ] {
        let encoded = insert_picked_path_for_dialect(
            "--before",
            picked,
            RunPathInsertMode::Arguments,
            ArgumentDialect::Windows,
        )
        .unwrap();
        let tail = encoded.strip_prefix("--before ").unwrap();
        assert_eq!(split_windows_argument(tail), picked);
    }
}

fn split_windows_argument(text: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\\' {
            let start = index;
            while index < chars.len() && chars[index] == '\\' {
                index += 1;
            }
            let count = index - start;
            if index < chars.len() && chars[index] == '"' {
                out.extend(std::iter::repeat_n('\\', count / 2));
                if count % 2 == 1 {
                    out.push('"');
                } else {
                    quoted = !quoted;
                }
                index += 1;
            } else {
                out.extend(std::iter::repeat_n('\\', count));
            }
            continue;
        }
        if chars[index] == '"' {
            quoted = !quoted;
        } else {
            out.push(chars[index]);
        }
        index += 1;
    }
    assert!(!quoted);
    out
}
