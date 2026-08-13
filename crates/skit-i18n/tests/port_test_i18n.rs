//! Public Rust equivalents for the locale/catalog/fallback contracts in Python
//! `tests/test_i18n.py` at `main@206f9ef`.
//!
//! Python-only Babel extraction, gettext plural machinery, fallback-chain internals, and synthetic
//! `.mo` injection are classified in the companion manifest. The tests here use the real static Rust
//! catalog and locale detector. Python v0.4 remains authoritative, so known Rust divergences stay red.

use std::collections::BTreeSet;

use skit_i18n::{Locale, available_locale_tags, catalog, detect_locale, text};

fn catalog_ids_for(locale: Locale) -> BTreeSet<&'static str> {
    catalog()
        .iter()
        .filter(|row| match locale {
            Locale::En | Locale::Pseudo => !row.english.is_empty(),
            Locale::ZhCn => !row.zh_cn.is_empty(),
            Locale::ZhTw => !row.zh_tw.is_empty(),
        })
        .map(|row| row.english)
        .collect()
}

fn assert_catalog_parity(locale: Locale) {
    let english = catalog_ids_for(Locale::En);
    let translated = catalog_ids_for(locale);
    assert_eq!(
        translated, english,
        "{locale:?} catalog ids drifted from English"
    );
}

fn assert_catalog_complete(locale: Locale) {
    for row in catalog() {
        let translated = match locale {
            Locale::ZhCn => row.zh_cn,
            Locale::ZhTw => row.zh_tw,
            Locale::En | Locale::Pseudo => row.english,
        };
        assert!(
            !translated.trim().is_empty(),
            "{locale:?} has an untranslated catalog row: {:?}",
            row.english
        );
    }
}

#[test]
fn test_locales_shipped() {
    assert!(available_locale_tags().contains(&"en"));
    assert!(available_locale_tags().len() >= 3);
    assert_eq!(available_locale_tags(), &["en", "zh-CN", "zh-TW"]);
}

#[test]
fn test_parity_with_pot() {
    let english = catalog_ids_for(Locale::En);
    assert_eq!(
        english.len(),
        catalog().len(),
        "duplicate English msgids make static catalog parity ambiguous"
    );
    assert_catalog_parity(Locale::ZhCn);
    assert_catalog_parity(Locale::ZhTw);
}

#[test]
fn rust_additive_parity_with_pot_zh_cn_row() {
    assert_catalog_parity(Locale::ZhCn);
}

#[test]
fn rust_additive_parity_with_pot_zh_tw_row() {
    assert_catalog_parity(Locale::ZhTw);
}

#[test]
fn test_locale_is_fully_translated() {
    assert_catalog_complete(Locale::ZhCn);
    assert_catalog_complete(Locale::ZhTw);
}

#[test]
fn rust_additive_locale_is_fully_translated_zh_cn_row() {
    assert_catalog_complete(Locale::ZhCn);
}

#[test]
fn rust_additive_locale_is_fully_translated_zh_tw_row() {
    assert_catalog_complete(Locale::ZhTw);
}

#[test]
fn test_exact_match() {
    assert_eq!(detect_locale(Some("zh-TW")), Locale::ZhTw);
}

#[test]
fn test_alias_hant() {
    assert_eq!(detect_locale(Some("zh-HK")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-Hant")), Locale::ZhTw);
}

#[test]
fn test_alias_hans_and_bare_zh() {
    assert_eq!(detect_locale(Some("zh")), Locale::ZhCn);
    assert_eq!(detect_locale(Some("zh-SG")), Locale::ZhCn);
}

#[test]
fn test_posix_style_tag() {
    assert_eq!(detect_locale(Some("zh_TW.UTF-8")), Locale::ZhTw);
}

#[test]
fn test_unknown_falls_back_to_en() {
    assert_eq!(detect_locale(Some("ko-KR")), Locale::En);
    assert_eq!(text(Locale::En, "no-such-message"), "no-such-message");
}

#[test]
fn test_zh_region_with_no_script_hint_defaults_to_simplified() {
    // Python v0.4 treats an otherwise unknown zh-* region as the bare zh macrolanguage, which
    // conventionally means Simplified Chinese. Keep that oracle even if the current Rust detector
    // falls back to English.
    assert_eq!(detect_locale(Some("zh-XX")), Locale::ZhCn);
}

#[test]
fn test_conflicting_script_and_region_lets_script_win() {
    assert_eq!(detect_locale(Some("zh-Hant-CN")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-Hans-TW")), Locale::ZhCn);
}

#[test]
fn test_conflicting_script_and_region_hk_mo_variants() {
    for tag in ["zh-Hans-HK", "zh-Hans-MO"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhCn, "tag={tag}");
    }
}

#[test]
fn test_plain_tags_unaffected_by_script_over_region_precedence() {
    for tag in ["zh-TW", "zh-HK", "zh-Hant"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhTw, "tag={tag}");
    }
    for tag in ["zh-CN", "zh-Hans", "zh"] {
        assert_eq!(detect_locale(Some(tag)), Locale::ZhCn, "tag={tag}");
    }
}

#[test]
fn test_zh_tw_message() {
    assert_eq!(text(Locale::ZhTw, "Name"), "名稱");
}

#[test]
fn test_zh_cn_message() {
    assert_eq!(text(Locale::ZhCn, "Name"), "名称");
}

fn assert_entry_library_copy(locale: Locale, library: &str, script_library: &str) {
    let messages = [
        text(locale, "Library").into_owned(),
        text(locale, "Library: {} ({} · {})").into_owned(),
        text(locale, "(shown in the Library — you can write one line)").into_owned(),
        text(locale, "Return to the Library immediately").into_owned(),
        text(locale, "Description (shown in the Library)").into_owned(),
    ];
    assert!(
        messages.iter().all(|message| message.contains(library)),
        "mixed-entry Library copy stopped using {library:?}: {messages:#?}"
    );
    assert!(
        messages
            .iter()
            .all(|message| !message.contains(script_library)),
        "mixed-entry Library copy narrowed itself to scripts: {messages:#?}"
    );
}

#[test]
fn test_entry_library_copy_does_not_narrow_the_mixed_library_to_scripts() {
    assert_entry_library_copy(Locale::ZhCn, "工具库", "脚本库");
    assert_entry_library_copy(Locale::ZhTw, "工具庫", "腳本庫");
}

#[test]
fn rust_additive_entry_library_copy_zh_cn_row() {
    assert_entry_library_copy(Locale::ZhCn, "工具库", "脚本库");
}

#[test]
fn rust_additive_entry_library_copy_zh_tw_row() {
    assert_entry_library_copy(Locale::ZhTw, "工具庫", "腳本庫");
}

#[test]
fn test_missing_id_returns_source() {
    assert_eq!(text(Locale::En, "no-such-message"), "no-such-message");
}

#[test]
fn test_pseudo_locale() {
    let rendered = text(Locale::Pseudo, "Name");
    assert!(rendered.starts_with('⟦'));
    assert!(rendered.ends_with("~~⟧"));
    assert!(rendered.contains("Nàmé"));
}

#[test]
fn test_pseudo_preserves_placeholder() {
    let source = "%(file)s isn't a .py file — pass --exe if it's an executable";
    let rendered = text(Locale::Pseudo, source);
    assert!(rendered.contains("%(file)s"), "{rendered}");
    assert!(
        rendered
            .replace("%(file)s", "photo.py")
            .contains("photo.py")
    );
}

#[test]
fn test_missing_zh_tw_msgid_falls_back_to_english_not_simplified() {
    assert_eq!(text(Locale::ZhTw, "Name"), "名稱");
    assert_eq!(text(Locale::ZhTw, "New String"), "New String");
    assert_ne!(text(Locale::ZhTw, "New String"), "新字符串");
}

#[test]
fn test_missing_zh_hk_msgid_falls_back_to_english_not_simplified() {
    let locale = detect_locale(Some("zh-HK"));
    assert_eq!(locale, Locale::ZhTw);
    assert_eq!(text(locale, "New String"), "New String");
    assert_ne!(text(locale, "New String"), "新字符串");
}

#[test]
fn test_conflicting_script_and_region_tag_end_to_end() {
    let locale = detect_locale(Some("zh-Hans-HK"));
    assert_eq!(locale, Locale::ZhCn);
    assert_eq!(text(locale, "Name"), "名称");
    assert_ne!(text(locale, "Name"), "名稱");
}
