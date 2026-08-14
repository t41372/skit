//! Semantic i18n parity retained from Python v0.4 `tests/test_js_deps.py`.
//!
//! Python's gate parsed gettext `.po` files. Rust ships a compiled static catalog instead, so PO
//! syntax/fuzzy/msgctxt parser tests have no product seam. Placeholder preservation still does: this
//! gate compares named/positional percent conversions and brace placeholders in every shipped row.

use std::collections::BTreeMap;

use skit_i18n::catalog;

fn signature(text: &str) -> BTreeMap<String, usize> {
    let bytes = text.as_bytes();
    let mut out = BTreeMap::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            if let Some(end) = text[index + 1..].find('}') {
                let inside = &text[index + 1..index + 1 + end];
                if !inside.contains('{') && !inside.contains('}') {
                    *out.entry(format!("brace:{inside}")).or_insert(0) += 1;
                    index += end + 2;
                    continue;
                }
            }
        }
        if bytes[index] == b'%' {
            if index + 1 < bytes.len() && bytes[index + 1] == b'%' {
                index += 2;
                continue;
            }
            let mut cursor = index + 1;
            let mut name = None;
            if cursor < bytes.len() && bytes[cursor] == b'(' {
                if let Some(close) = text[cursor + 1..].find(')') {
                    name = Some(&text[cursor + 1..cursor + 1 + close]);
                    cursor += close + 2;
                }
            }
            while cursor < bytes.len()
                && matches!(bytes[cursor], b'#' | b'0' | b'-' | b' ' | b'+' | b'.' | b'0'..=b'9')
            {
                cursor += 1;
            }
            if cursor < bytes.len() && bytes[cursor].is_ascii_alphabetic() {
                let conversion = bytes[cursor] as char;
                let key = match name {
                    Some(name) => format!("percent:{name}:{conversion}"),
                    None => format!("percent-pos:{conversion}"),
                };
                *out.entry(key).or_insert(0) += 1;
                index = cursor + 1;
                continue;
            }
        }
        index += 1;
    }
    out
}

#[test]
fn test_placeholder_parity_passes_the_shipped_catalogs() {
    let rows = catalog();
    assert!(!rows.is_empty());
    for row in rows {
        let expected = signature(row.english);
        assert_eq!(
            signature(row.zh_cn),
            expected,
            "zh-CN placeholder mismatch for {:?}: {:?}",
            row.english,
            row.zh_cn
        );
        assert_eq!(
            signature(row.zh_tw),
            expected,
            "zh-TW placeholder mismatch for {:?}: {:?}",
            row.english,
            row.zh_tw
        );
    }
}