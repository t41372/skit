//! Public-surface behavioral ports from `origin/main@206f9ef:tests/test_prompt_kind.py`.
//!
//! The Python prompt grammar is the oracle. A red test in this file is intentionally retained as
//! a parity finding; this branch does not change the language implementation to make it green.

use std::{collections::BTreeMap, path::Path};

use skit_language::{infer_kind, placeholder_params, render_prompt_body};

fn names(text: &str) -> Vec<String> {
    placeholder_params("prompt", text)
        .into_iter()
        .map(|parameter| parameter.name)
        .collect()
}

#[test]
fn test_placeholder_names_dedupes_in_body_order() {
    let text = "a {{b}} c {{a}} d {{b}} {{_x1}} {{9bad}} {{ spaced }} {{a-b}}";
    assert_eq!(names(text), ["b", "a", "_x1"]);
}

#[test]
fn test_placeholder_names_single_braces_are_never_candidates() {
    let text = r#"JSON {"key": 1} f-string {value} shell ${HOME} empty {} plain {word}"#;
    assert!(names(text).is_empty());
}

#[test]
fn test_placeholder_names_brace_adjacent_is_not_a_candidate() {
    assert!(names("{{{raw}}} and {{{x}} and {{y}}}").is_empty());
}

#[test]
fn test_placeholder_names_reserved_name_excluded() {
    assert_eq!(names("{{prompt}} {{real}}"), ["real"]);
}

#[test]
fn test_placeholder_names_accept_unicode_identifiers_and_reject_non_names() {
    let text = "{{任务}} {{café}} {{é}} {{not-a-name}} {{💥}} {{}}";
    assert_eq!(names(text), ["任务", "café", "é"]);
}

#[test]
fn test_placeholder_names_high_cardinality_stays_ordered_and_complete() {
    let expected = (0..10_000)
        .map(|index| format!("field_{index}"))
        .collect::<Vec<_>>();
    let text = expected
        .iter()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect::<Vec<_>>()
        .join(" ");

    assert_eq!(names(&text), expected);
}

#[test]
fn test_render_body_substitutes_raw_never_quotes_or_rescans() {
    let payload = r#"'; rm -rf ~; $(touch pwned) `echo hi` "x" {inner} {{deep}}"#;
    let values = BTreeMap::from([("v".to_owned(), payload.to_owned())]);

    let rendered = render_prompt_body("V={{v}} end", &values, true);

    assert_eq!(rendered, format!("V={payload} end"));
}

#[test]
fn test_render_body_empty_value_substitutes_empty() {
    let values = BTreeMap::from([("v".to_owned(), String::new())]);
    assert_eq!(render_prompt_body("[{{v}}]", &values, true), "[]");
}

#[test]
fn test_render_body_unmanaged_hole_stays_verbatim() {
    let values = BTreeMap::from([("managed".to_owned(), "M".to_owned())]);
    assert_eq!(
        render_prompt_body("{{managed}} {{unmanaged}}", &values, true),
        "M {{unmanaged}}"
    );
}

#[test]
fn test_render_body_interpolation_can_be_disabled_without_touching_bytes() {
    let body = "line1\r\n{{task}}\r\n{{prompt}}";
    let values = BTreeMap::from([("task".to_owned(), "X".to_owned())]);
    assert_eq!(render_prompt_body(body, &values, false), body);
}

#[test]
fn test_corpus_crlf_shape_is_preserved_by_rendering() {
    let body = "A={{task}}\r\nB={{repo}}\r\n";
    let values = BTreeMap::from([
        ("task".to_owned(), "X".to_owned()),
        ("repo".to_owned(), "Y".to_owned()),
    ]);

    assert_eq!(
        render_prompt_body(body, &values, true),
        "A=X\r\nB=Y\r\n"
    );
}

#[test]
fn test_corpus_cjk_emoji_no_trailing_newline() {
    let body = "審查 {{目標檔案}} 🎯\n專注於 {{focus}}";
    let values = BTreeMap::from([
        ("目標檔案".to_owned(), "src/主程式.py".to_owned()),
        ("focus".to_owned(), "效能".to_owned()),
    ]);

    let rendered = render_prompt_body(body, &values, true);
    assert_eq!(rendered, "審查 src/主程式.py 🎯\n專注於 效能");
    assert!(!rendered.ends_with('\n'));
}

#[test]
fn test_corpus_reserved_prompt_stays_verbatim() {
    let body = "{{prompt}}\tliterally\n{{real}}";
    let values = BTreeMap::from([("real".to_owned(), "R".to_owned())]);

    assert_eq!(render_prompt_body(body, &values, true), "{{prompt}}\tliterally\nR");
}

#[test]
fn test_infer_kind_compound_suffix() {
    assert_eq!(infer_kind(Path::new("notes/review.prompt.md"), None, false), Some("prompt"));
    assert_eq!(infer_kind(Path::new("REVIEW.PROMPT.MD"), None, false), Some("prompt"));
    assert_eq!(infer_kind(Path::new("x.prompt"), None, false), Some("prompt"));
    assert_eq!(infer_kind(Path::new("notes.md"), None, false), None);
    assert_eq!(infer_kind(Path::new("a.mts"), None, false), Some("ts"));
    assert_eq!(infer_kind(Path::new("b.sh"), None, false), Some("shell"));
}
