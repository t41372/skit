use skit_i18n::{Locale, catalog, detect_locale, format_text, render, text};

#[test]
fn locale_detection_accepts_existing_and_standard_spellings() {
    assert_eq!(detect_locale(Some("zh-TW")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh_TW.UTF-8")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-CN")), Locale::ZhCn);
    assert_eq!(detect_locale(Some("zh_Hans_CN.UTF-8")), Locale::ZhCn);
    // Traditional Chinese regions and Singapore must not fall back to English.
    assert_eq!(detect_locale(Some("zh-HK")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh_MO.UTF-8")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-Hant-HK")), Locale::ZhTw);
    assert_eq!(detect_locale(Some("zh-SG")), Locale::ZhCn);
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
    assert_eq!(format_text(Locale::En, "Library", &[&"unused"]), "Library");
    assert_eq!(format_text(Locale::En, "{} {} {}", &[&"one"]), "one {} {}");
}

#[test]
fn pi_prompt_mode_warning_is_complete() {
    assert_eq!(
        text(
            Locale::ZhCn,
            "Added a newline to keep the Pi prompt in message mode"
        ),
        "已添加换行符，使 Pi 提示词保持消息模式"
    );
    assert_eq!(
        text(
            Locale::ZhTw,
            "Added a newline to keep the Pi prompt in message mode"
        ),
        "已新增換行字元，使 Pi 提示詞保持訊息模式"
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

/// Remove each inline `#[cfg(test)]` module so only shipped text is collected.
fn without_test_modules(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(index) = rest.find("#[cfg(test)]") {
        output.push_str(&rest[..index]);
        rest = &rest[index..];
        let Some(open) = rest.find('{') else { break };
        // `#[cfg(test)] mod tests;` has no body, so keep the rest of the file.
        if rest[..open].contains(';') {
            let semicolon = rest.find(';').expect("the declaration ends with ;");
            rest = &rest[semicolon + 1..];
            continue;
        }
        let mut depth = 0;
        let mut cursor = open;
        for (offset, character) in rest[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        cursor = open + offset + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = &rest[cursor..];
    }
    output.push_str(rest);
    output
}

/// Collect every `Message::new("...")` template in the workspace source tree.
fn message_templates() -> std::collections::BTreeMap<String, Vec<String>> {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the i18n crate is inside the crates directory")
        .to_owned();
    let mut sources = Vec::new();
    let mut pending = crates
        .read_dir()
        .expect("the crates directory is readable")
        .map(|item| {
            item.expect("each crate directory item is readable")
                .path()
                .join("src")
        })
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    while let Some(directory) = pending.pop() {
        for item in std::fs::read_dir(&directory).expect("each source directory is readable") {
            let path = item.expect("each source item is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|value| value == "rs") {
                sources.push(path);
            }
        }
    }
    assert!(sources.len() > 20, "the source walk found too few files");

    let mut templates = std::collections::BTreeMap::<String, Vec<String>>::new();
    for path in sources {
        let source = std::fs::read_to_string(&path).expect("each Rust source is UTF-8");
        let source = without_test_modules(&source);
        let mut rest = source.as_str();
        while let Some(index) = rest.find("Message::new(") {
            rest = &rest[index + "Message::new(".len()..];
            let Some(quote) = rest.find('"') else { break };
            // A non-literal argument means the template is not a stable catalog key.
            assert!(
                rest[..quote].trim_start().is_empty(),
                "Message::new needs a literal template in {}",
                path.display()
            );
            rest = &rest[quote + 1..];
            let mut end = 0;
            let bytes = rest.as_bytes();
            while bytes[end] != b'"' {
                end += if bytes[end] == b'\\' { 2 } else { 1 };
            }
            let template = rest[..end].replace("\\\"", "\"").replace("\\n", "\n");
            templates
                .entry(template)
                .or_default()
                .push(path.display().to_string());
            rest = &rest[end + 1..];
        }
    }
    templates
}

#[test]
fn every_typed_message_template_is_a_complete_catalog_row() {
    let translated = catalog()
        .iter()
        .map(|row| row.english)
        .collect::<std::collections::BTreeSet<_>>();
    let templates = message_templates();
    assert!(
        templates.len() > 100,
        "the scan found only {} templates",
        templates.len()
    );
    let missing = templates
        .keys()
        .filter(|template| !translated.contains(template.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "add a complete catalog row for each template:\n{}",
        missing.join("\n")
    );
}

#[test]
fn a_typed_message_keeps_its_values_out_of_the_translation() {
    // `not`, `on`, and `list` are catalog words. A value must never be translated.
    let message = skit_i18n::Message::new("entry not found: {}").with("list-me");
    assert_eq!(message.template(), "entry not found: {}");
    assert_eq!(message.localize(Locale::ZhCn), "找不到条目：list-me");
    assert_eq!(message.localize(Locale::En), "entry not found: list-me");
    assert_eq!(message.to_string(), "entry not found: list-me");

    let quoted = skit_i18n::Message::new("preset {} does not exist").quoted("on off");
    assert_eq!(quoted.localize(Locale::ZhTw), "預設 \"on off\" 不存在");

    let nested = skit_i18n::Message::new("invalid entry mutation: {}")
        .nested(skit_i18n::Message::new("entry name cannot be blank"));
    assert_eq!(
        nested.localize(Locale::ZhCn),
        "无效的条目变更：条目名称不能为空"
    );
    assert_eq!(
        nested.localize(Locale::En),
        "invalid entry mutation: entry name cannot be blank"
    );
}

#[test]
fn framework_rendering_changes_only_whole_words() {
    // These English words contain the catalog rows `on`, `not`, and `list`.
    for sample in ["version", "cannot", "listen", "monitor"] {
        assert_eq!(render(Locale::ZhCn, sample), sample);
        assert_eq!(render(Locale::ZhTw, sample), sample);
    }
    // Clap composes its report, so whole framework words still change.
    assert_eq!(
        render(
            Locale::ZhCn,
            "Usage: skit [OPTIONS]\n\nOptions:\n  --help  Print help"
        ),
        "用法： skit [OPTIONS]\n\n选项：\n  --help  显示帮助"
    );
    assert_eq!(render(Locale::ZhCn, "(Options:)"), "(选项：)");
    // A composable row never replaces part of a longer word.
    assert_eq!(render(Locale::ZhCn, "Print versions"), "Print versions");
    assert_eq!(render(Locale::ZhTw, "reUsage:"), "reUsage:");
}

#[test]
fn every_tui_completion_status_has_two_complete_translations() {
    for (english, zh_cn, zh_tw) in [
        ("Source saved", "源文件已保存", "來源已儲存"),
        ("Entry removed", "条目已删除", "項目已移除"),
        ("Entry added", "条目已添加", "項目已新增"),
        ("Settings saved", "设置已保存", "設定已儲存"),
        ("Preferences saved", "偏好设置已保存", "偏好設定已儲存"),
        (
            "Prompt runners saved",
            "提示词运行器已保存",
            "提示詞執行器已儲存",
        ),
        ("Presets saved", "预设已保存", "預設已儲存"),
        ("Entry renamed", "条目已重命名", "項目已重新命名"),
        (
            "Run finished with exit status",
            "运行完成，退出状态为",
            "執行完成，結束狀態為",
        ),
    ] {
        assert_eq!(render(Locale::ZhCn, english), zh_cn);
        assert_eq!(render(Locale::ZhTw, english), zh_tw);
    }
    assert_eq!(
        render(Locale::ZhTw, "Run finished with exit status 7"),
        "執行完成，結束狀態為 7"
    );
}
