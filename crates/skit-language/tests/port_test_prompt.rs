//! Public-surface behavioral ports from `origin/main@206f9ef:tests/test_prompt_kind.py`.
//!
//! This file owns only the public prompt grammar/render/inference slice. Store, runner, launch, CLI,
//! and TUI contracts live at their real public boundaries. The Python suite is authoritative: a red
//! assertion stays red and does not justify changing production code on this branch.

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
fn test_prompt_grammar_is_independent_of_command_templates() {
    assert!(names("{name}").is_empty());
    assert_eq!(
        placeholder_params("command", "{name}")
            .into_iter()
            .map(|parameter| parameter.name)
            .collect::<Vec<_>>(),
        ["name"]
    );
    assert_eq!(names("{{name}}"), ["name"]);
}

#[test]
fn test_corpus_basic_detection_and_render_byte_identity() {
    let text = "# Review helper\n\nReview {{target}} for {{focus}}. Again: {{target}}.\nLiterals: {code} and JSON {\"key\": 1} and f'{value}' and {{{handlebars}}}\nUnmanaged hole: {{x}}\n";
    assert_eq!(names(text), ["target", "focus", "x"]);

    let values = BTreeMap::from([
        ("target".to_owned(), "T".to_owned()),
        ("focus".to_owned(), "F".to_owned()),
    ]);
    let rendered = render_prompt_body(text, &values, true);

    assert!(rendered.contains("Review T for F. Again: T."), "{rendered}");
    assert!(
        rendered.contains("Literals: {code} and JSON {\"key\": 1} and f'{value}' and {{{handlebars}}}"),
        "{rendered}"
    );
    assert!(rendered.contains("Unmanaged hole: {{x}}"), "{rendered}");
}

#[test]
fn test_corpus_crlf_preserved_verbatim() {
    let text = "A={{task}}\r\nB={{repo}}\r\n";
    assert!(text.contains("\r\n"));
    let values = BTreeMap::from([
        ("task".to_owned(), "X".to_owned()),
        ("repo".to_owned(), "Y".to_owned()),
    ]);

    let rendered = render_prompt_body(text, &values, true);
    assert!(rendered.contains("\r\n"));
    assert_eq!(rendered, text.replace("{{task}}", "X").replace("{{repo}}", "Y"));
}

#[test]
fn test_corpus_cjk_emoji_no_trailing_newline() {
    let text = "審查 {{目標檔案}} 🎯\n專注於 {{focus}}";
    assert!(!text.ends_with('\n'));
    assert_eq!(names(text), ["目標檔案", "focus"]);
    let values = BTreeMap::from([
        ("目標檔案".to_owned(), "src/主程式.py".to_owned()),
        ("focus".to_owned(), "效能".to_owned()),
    ]);

    let rendered = render_prompt_body(text, &values, true);
    assert!(rendered.contains("審查 src/主程式.py"), "{rendered}");
    assert!(rendered.contains("專注於 效能"), "{rendered}");
    assert!(!rendered.ends_with('\n'));
}

#[test]
fn test_corpus_reserved_prompt_stays_verbatim() {
    let text = "{{prompt}}\tliterally\n{{real}}";
    assert_eq!(names(text), ["real"]);
    let values = BTreeMap::from([("real".to_owned(), "R".to_owned())]);

    assert_eq!(
        render_prompt_body(text, &values, true),
        "{{prompt}}\tliterally\nR"
    );
}

#[test]
fn test_render_body_substitutes_raw_never_quotes() {
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
fn rust_additive_render_body_unmanaged_hole_stays_verbatim() {
    let values = BTreeMap::from([("managed".to_owned(), "M".to_owned())]);
    assert_eq!(
        render_prompt_body("{{managed}} {{unmanaged}}", &values, true),
        "M {{unmanaged}}"
    );
}

#[test]
fn rust_additive_render_body_interpolation_can_be_disabled_without_touching_bytes() {
    let body = "line1\r\n{{task}}\r\n{{prompt}}";
    let values = BTreeMap::from([("task".to_owned(), "X".to_owned())]);
    assert_eq!(render_prompt_body(body, &values, false), body);
}

#[test]
fn test_infer_kind_compound_suffix() {
    assert_eq!(
        infer_kind(Path::new("notes/review.prompt.md"), None, false),
        Some("prompt")
    );
    assert_eq!(
        infer_kind(Path::new("REVIEW.PROMPT.MD"), None, false),
        Some("prompt")
    );
    assert_eq!(
        infer_kind(Path::new("x.prompt"), None, false),
        Some("prompt")
    );
    assert_eq!(infer_kind(Path::new("notes.md"), None, false), None);
    assert_eq!(infer_kind(Path::new("a.mts"), None, false), Some("ts"));
    assert_eq!(infer_kind(Path::new("b.sh"), None, false), Some("shell"));
}
