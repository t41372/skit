use skit_i18n::{Locale, catalog, detect_locale, format_text, render, text};

#[test]
fn locale_detection_accepts_existing_and_standard_spellings() {
    assert_eq!(detect_locale(Some("zh-TW")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh_TW.UTF-8")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-CN")), Locale::ZhCn);
    assert_eq!(detect_locale(Some("zh_Hans_CN.UTF-8")), Locale::ZhCn);
    assert_eq!(detect_locale(Some("C")), Locale::En);
    assert_eq!(detect_locale(Some("fr_FR.UTF-8")), Locale::En);
    assert_eq!(detect_locale(None), Locale::En);
}

#[test]
fn every_catalog_row_has_two_complete_translations() {
    let rows = catalog();
    assert!(!rows.is_empty());
    for row in rows {
        assert!(!row.english.trim().is_empty());
        assert!(
            !row.zh_cn.trim().is_empty(),
            "missing zh-CN: {}",
            row.english
        );
        assert!(
            !row.zh_tw.trim().is_empty(),
            "missing zh-TW: {}",
            row.english
        );
    }
}

#[test]
fn exact_text_and_longest_first_rendering_are_deterministic() {
    assert_eq!(text(Locale::ZhTw, "Library"), "程式庫");
    assert_eq!(text(Locale::ZhCn, "Library"), "程序库");
    assert_eq!(text(Locale::En, "Library"), "Library");
    assert_eq!(text(Locale::ZhTw, "not catalog text"), "not catalog text");

    assert_eq!(
        render(Locale::ZhTw, "Library: all entries"),
        "程式庫：所有項目"
    );
    assert_eq!(
        render(Locale::ZhCn, "No matching entries. Press [q] Quit."),
        "没有匹配的条目。按 [q] 退出。"
    );
}

#[test]
fn formatted_messages_translate_the_template_without_translating_user_values() {
    assert_eq!(
        format_text(Locale::ZhTw, "Added: {} ({})", &[&"Library", &"library"]),
        "已新增：Library (library)"
    );
    assert_eq!(
        format_text(Locale::En, "Added: {} ({})", &[&"Alpha", &"alpha"]),
        "Added: Alpha (alpha)"
    );
    assert_eq!(
        format_text(Locale::ZhTw, "Unknown {}", &[&"value"]),
        "Unknown value"
    );
}

#[test]
fn every_cli_human_message_macro_uses_a_complete_catalog_template() {
    let source = include_str!("../../skit-cli/src/cli.rs");
    let translated = catalog()
        .iter()
        .map(|row| row.english)
        .collect::<std::collections::BTreeSet<_>>();
    for macro_name in ["humanln!(", "humanerrln!("] {
        let mut rest = source;
        while let Some(index) = rest.find(macro_name) {
            rest = &rest[index + macro_name.len()..];
            let quote = rest
                .find('"')
                .expect("human macro needs a literal template");
            rest = &rest[quote + 1..];
            let end = rest.find('"').expect("human macro literal must end");
            let template = &rest[..end];
            assert!(
                translated.contains(template),
                "missing CLI translation template: {template}"
            );
            rest = &rest[end + 1..];
        }
    }
}
