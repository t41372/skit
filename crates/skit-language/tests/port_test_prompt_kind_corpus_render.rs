use std::{collections::BTreeMap, fs, path::PathBuf};

use skit_language::{placeholder_params, render_prompt_body};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("skit-language lives under <repo>/crates/skit-language")
        .to_path_buf()
}

fn corpus(name: &str) -> Vec<u8> {
    fs::read(repo().join("tests/corpus/prompt").join(name)).unwrap()
}

fn names(text: &str) -> Vec<String> {
    placeholder_params("prompt", text)
        .into_iter()
        .map(|field| field.name)
        .collect()
}

#[test]
fn test_corpus_basic_detection_and_render_byte_identity() {
    let text = String::from_utf8(corpus("01_basic.prompt.md")).unwrap();
    assert_eq!(names(&text), ["target", "focus", "x"]);
    let rendered = render_prompt_body(
        &text,
        &BTreeMap::from([
            ("target".to_owned(), "T".to_owned()),
            ("focus".to_owned(), "F".to_owned()),
        ]),
        true,
    );
    assert!(rendered.contains("Review T for F. Again: T."), "{rendered}");
    assert!(
        rendered.contains(r#"Literals: {code} and JSON {"key": 1} and f'{value}' and {{{handlebars}}}"#),
        "{rendered}"
    );
    assert!(rendered.contains("Unmanaged hole: {{x}}"), "{rendered}");
}

#[test]
fn test_corpus_crlf_preserved_verbatim() {
    let raw = corpus("02_crlf.prompt.md");
    assert!(raw.windows(2).any(|pair| pair == b"\r\n"));
    let text = String::from_utf8(raw).unwrap();
    let rendered = render_prompt_body(
        &text,
        &BTreeMap::from([
            ("task".to_owned(), "X".to_owned()),
            ("repo".to_owned(), "Y".to_owned()),
        ]),
        true,
    );
    assert!(rendered.contains("\r\n"));
    assert_eq!(rendered, text.replace("{{task}}", "X").replace("{{repo}}", "Y"));
}

#[test]
fn test_corpus_cjk_emoji_no_trailing_newline() {
    let raw = corpus("03_cjk_emoji.prompt.md");
    assert!(!raw.ends_with(b"\n"));
    let text = String::from_utf8(raw).unwrap();
    assert_eq!(names(&text), ["目標檔案", "focus"]);
    let rendered = render_prompt_body(
        &text,
        &BTreeMap::from([
            ("目標檔案".to_owned(), "src/主程式.py".to_owned()),
            ("focus".to_owned(), "效能".to_owned()),
        ]),
        true,
    );
    assert!(rendered.contains("審查 src/主程式.py"), "{rendered}");
    assert!(rendered.contains("專注於 效能"), "{rendered}");
    assert!(!rendered.ends_with('\n'));
}

#[test]
fn test_corpus_reserved_prompt_stays_verbatim() {
    let text = String::from_utf8(corpus("05_reserved.prompt.md")).unwrap();
    assert_eq!(names(&text), ["real"]);
    let rendered = render_prompt_body(
        &text,
        &BTreeMap::from([("real".to_owned(), "R".to_owned())]),
        true,
    );
    assert!(rendered.contains("{{prompt}}\tliterally"), "{rendered}");
}

#[test]
fn test_render_body_substitutes_raw_never_quotes() {
    let payload = r#"\'; rm -rf ~; $(touch pwned) `echo hi` "x" {inner} {{deep}}"#;
    let rendered = render_prompt_body(
        "V={{v}} end",
        &BTreeMap::from([("v".to_owned(), payload.to_owned())]),
        true,
    );
    assert_eq!(rendered, format!("V={payload} end"));
}

#[test]
fn test_render_body_empty_value_substitutes_empty() {
    assert_eq!(
        render_prompt_body(
            "[{{v}}]",
            &BTreeMap::from([("v".to_owned(), String::new())]),
            true,
        ),
        "[]"
    );
}
