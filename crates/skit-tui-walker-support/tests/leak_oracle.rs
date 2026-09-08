use std::collections::BTreeMap;

use proptest::prelude::*;
use serde_json::{Value, json};
use skit_tui_walker_support::leak_oracle::{
    DraftAllocatorError, DraftAllocatorForm, JsonStringRole, TextSpan, accept_host_path_token,
    exact_fact_spans, exact_text_spans, json_string_occurrences, parse_draft_allocator_token,
};
use skit_tui_walker_support::projection::replace_longest_text_tokens;

#[test]
fn host_path_boundaries_keep_the_existing_projection_grammar() {
    for boundary in " \t\n\u{a0}'\"()[]{}:,=/\\，。；：！？、（）【】「」『』《》".chars()
    {
        let edge = boundary.to_string();
        assert!(accept_host_path_token(&edge, ""), "left {edge:?}");
        assert!(accept_host_path_token("", &edge), "right {edge:?}");
    }
    for edge in ["", ".", "~~⟧"] {
        assert!(accept_host_path_token("", edge), "right {edge:?}");
    }
    for edge in ["a", "界", "🦀", "_", "-", ".", ";", "!", "?", "~"] {
        assert!(!accept_host_path_token(edge, ""), "left {edge:?}");
    }
    for edge in [
        "a",
        "界",
        "🦀",
        "_",
        "-",
        ".py",
        ". Next",
        ";",
        "!",
        "?",
        "~~⟧ next",
    ] {
        assert!(!accept_host_path_token("", edge), "right {edge:?}");
    }
    assert!(!accept_host_path_token("word", "."));
    assert!(!accept_host_path_token("word", "~~⟧"));
}

#[test]
fn exact_fact_spans_keep_complete_tokens_and_utf8_byte_offsets() {
    let token = "路徑/skit-new-123abc.py";
    let text = format!("界{token} 「{token}」 {token}_tail ({token}).");
    let first_start = format!("界{token} 「").len();
    let second_start = format!("界{token} 「{token}」 {token}_tail (").len();
    assert_eq!(
        exact_fact_spans(&text, token),
        vec![
            TextSpan {
                start: first_start,
                end: first_start + token.len()
            },
            TextSpan {
                start: second_start,
                end: second_start + token.len()
            },
        ]
    );
    for suffix in ["", ".", "~~⟧"] {
        assert_eq!(
            exact_fact_spans(&format!("{token}{suffix}"), token),
            vec![TextSpan {
                start: 0,
                end: token.len()
            }]
        );
    }
    for text in [
        "路徑/skit-new-123",
        "skit-new-123abc.py",
        "路徑/skit-new-123abc.py. Next",
    ] {
        assert!(exact_fact_spans(text, token).is_empty(), "{text}");
    }
    assert!(exact_fact_spans("anything", "").is_empty());
}

proptest! {
    #[test]
    fn projected_text_has_no_exact_raw_facts(
        counter in 0_u32..=0xff_ffff,
        segment in "[é界🦀]{1,6}",
        prefix in prop::sample::select(vec!["", "文字 ", "「", "=", "/", "\\", "界", "x", "_", "."]),
        suffix in prop::sample::select(vec!["", ".", ". Next", "~~⟧", "~~⟧ next", "」", "，下一個", "/tail", "\\tail", ".tar", "界", "_"]),
        repeats in 1_usize..6,
    ) {
        let basename = format!("skit-new-{counter:06x}.py");
        let path = format!("/sandbox/{segment}/{basename}");
        let repeated = format!("「{path}」 [{basename}]\n").repeat(repeats);
        let text = format!("{repeated}{prefix}{path}{suffix}");
        let replacements = BTreeMap::from([
            (path.clone(), "@DRAFT_PATH@".to_owned()),
            (basename.clone(), "@DRAFT_NAME@".to_owned()),
        ]);

        let projected = replace_longest_text_tokens(&text, &replacements, accept_host_path_token);

        prop_assert!(projected.starts_with("「@DRAFT_PATH@」 [@DRAFT_NAME@]\n"));
        for raw in replacements.keys() {
            prop_assert!(exact_fact_spans(&projected, raw).is_empty(), "{raw:?} remains in {projected:?}");
        }
        prop_assert_eq!(
            replace_longest_text_tokens(&projected, &replacements, accept_host_path_token),
            projected
        );
    }
}

#[test]
fn json_string_traversal_visits_keys_and_values_at_stable_pointers() {
    let value: Value = serde_json::from_str(
        r#"{"z":["tail",{"key":"value"}],"a/b":{"~key":"nested"},"number":1}"#,
    )
    .unwrap();

    let actual = json_string_occurrences(&value)
        .into_iter()
        .map(|item| (item.pointer, item.role, item.text.to_owned()))
        .collect::<Vec<_>>();

    assert_eq!(
        actual,
        vec![
            ("/a~1b".to_owned(), JsonStringRole::Key, "a/b".to_owned(),),
            (
                "/a~1b/~0key".to_owned(),
                JsonStringRole::Key,
                "~key".to_owned(),
            ),
            (
                "/a~1b/~0key".to_owned(),
                JsonStringRole::Value,
                "nested".to_owned(),
            ),
            (
                "/number".to_owned(),
                JsonStringRole::Key,
                "number".to_owned(),
            ),
            ("/z".to_owned(), JsonStringRole::Key, "z".to_owned()),
            ("/z/0".to_owned(), JsonStringRole::Value, "tail".to_owned(),),
            ("/z/1/key".to_owned(), JsonStringRole::Key, "key".to_owned(),),
            (
                "/z/1/key".to_owned(),
                JsonStringRole::Value,
                "value".to_owned(),
            ),
        ]
    );

    let root_value = json!("root");
    let root = json_string_occurrences(&root_value);
    assert_eq!(root.len(), 1);
    assert_eq!(root[0].pointer, "");
    assert_eq!(root[0].role, JsonStringRole::Value);
    assert_eq!(root[0].text, "root");
    assert!(json_string_occurrences(&json!(null)).is_empty());
}

#[test]
fn exact_spans_include_overlaps_and_use_utf8_byte_offsets() {
    assert_eq!(
        exact_text_spans("aaaa", "aa"),
        vec![
            TextSpan { start: 0, end: 2 },
            TextSpan { start: 1, end: 3 },
            TextSpan { start: 2, end: 4 },
        ]
    );
    assert_eq!(
        exact_text_spans("éé", "é"),
        vec![TextSpan { start: 0, end: 2 }, TextSpan { start: 2, end: 4 },]
    );
    assert!(exact_text_spans("text", "").is_empty());
    assert!(exact_text_spans("text", "absent").is_empty());
}

#[test]
fn allocator_parser_accepts_only_the_three_closed_forms() {
    for (token, form) in [
        ("skit-new-000000", DraftAllocatorForm::Stem),
        ("skit-new-ffffff", DraftAllocatorForm::Stem),
        ("skit-new-123abc.py", DraftAllocatorForm::Script),
        ("skit-new-123abc.prompt.md", DraftAllocatorForm::Prompt),
    ] {
        assert_eq!(parse_draft_allocator_token(token), Ok(form), "{token}");
    }
}

#[test]
fn allocator_parser_refuses_near_misses_with_an_exact_reason() {
    for (token, error) in [
        ("draft-new-123abc.py", DraftAllocatorError::Prefix),
        ("skit-new-ABCDEF.py", DraftAllocatorError::CounterCharacter),
        ("skit-new-abcdeg.py", DraftAllocatorError::CounterCharacter),
        ("skit-new-abcde.py", DraftAllocatorError::CounterWidth),
        ("skit-new-abcdef0.py", DraftAllocatorError::CounterWidth),
        ("skit-new-abcdef.js", DraftAllocatorError::Suffix),
        ("skit-new-abcdef.prompt", DraftAllocatorError::Suffix),
        ("skit-new-abcdef.py.extra", DraftAllocatorError::Suffix),
        (
            "skit-new-abcdef.prompt.md.extra",
            DraftAllocatorError::Suffix,
        ),
    ] {
        assert_eq!(parse_draft_allocator_token(token), Err(error), "{token}");
    }
}
