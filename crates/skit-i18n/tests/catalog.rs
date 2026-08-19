use skit_i18n::{
    Locale, available_locale_tags, catalog, detect_locale, format_text, kind_label, render,
    requested_locale, text,
};

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
    assert_eq!(detect_locale(Some("x-pseudo")), Locale::Pseudo);
    assert_eq!(detect_locale(Some("C")), Locale::En);
    assert_eq!(detect_locale(Some("fr_FR.UTF-8")), Locale::En);
    assert_eq!(detect_locale(None), Locale::En);
}

#[test]
fn locale_precedence_can_skip_only_an_empty_or_exact_c_candidate() {
    assert_eq!(requested_locale(None), None);
    assert_eq!(requested_locale(Some("")), None);
    assert_eq!(requested_locale(Some("C")), None);
    assert_eq!(requested_locale(Some("  C  ")), Some(Locale::En));
    assert_eq!(requested_locale(Some("zh_TW.UTF-8")), Some(Locale::ZhTw));
    assert_eq!(requested_locale(Some("C.UTF-8")), Some(Locale::En));
    assert_eq!(requested_locale(Some("fr_FR.UTF-8")), Some(Locale::En));
}

#[test]
fn preferences_can_present_shipped_locales_and_the_effective_tag() {
    assert_eq!(available_locale_tags(), &["en", "zh-CN", "zh-TW"]);
    assert_eq!(Locale::En.tag(), "en");
    assert_eq!(Locale::ZhCn.tag(), "zh-CN");
    assert_eq!(Locale::ZhTw.tag(), "zh-TW");
    assert_eq!(Locale::Pseudo.tag(), "x-pseudo");
}

#[test]
fn pseudo_locale_stretches_source_text_without_touching_inserted_values() {
    assert_eq!(text(Locale::Pseudo, "Name"), "⟦Nàmé~~⟧");
    assert_eq!(
        format_text(Locale::Pseudo, "Added: {}", &[&"User-AEIOU"]),
        "⟦Àddéd: {}~~⟧".replace("{}", "User-AEIOU")
    );
    assert_eq!(
        text(Locale::Pseudo, "%(file)s is available"),
        "⟦%(file)s îs àvàîlàblé~~⟧"
    );
    assert_eq!(
        render(Locale::Pseudo, "Usage: skit --help"),
        "⟦Ûsàgé: skît --hélp~~⟧"
    );
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
    assert_eq!(text(Locale::ZhTw, "Library"), "工具庫");
    assert_eq!(text(Locale::ZhCn, "Library"), "工具库");
    assert_eq!(text(Locale::En, "Library"), "Library");
    assert_eq!(text(Locale::ZhTw, "not catalog text"), "not catalog text");

    assert_eq!(
        render(Locale::ZhTw, "Library: all entries"),
        "工具庫：所有項目"
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
    assert_eq!(
        format_text(
            Locale::ZhTw,
            "{}: type changed from {} to {} in the source (still injected — double-check the value)",
            &[&"COUNT", &"str", &"int"],
        ),
        "COUNT:原始碼中的型別已從 str 變為 int(仍會注入,請確認值)"
    );
    assert_eq!(
        format_text(
            Locale::ZhCn,
            "To refresh the definitions, run: skit params {} --resync",
            &[&"trip"],
        ),
        "若要更新定义,请运行:skit params trip --resync"
    );
}

#[test]
fn secret_purge_notice_matches_the_oracle_in_every_locale() {
    let template = "Removed previously stored plaintext value(s) for now-secret parameter(s): {}";
    assert_eq!(
        format_text(Locale::En, template, &[&"A, B"]),
        "Removed previously stored plaintext value(s) for now-secret parameter(s): A, B"
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&"A, B"]),
        "已移除下列刚设为机密的参数先前以明文存储的值:A, B"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&"A, B"]),
        "已移除下列剛設為機密的參數先前以明文儲存的值:A, B"
    );
}

#[test]
fn malformed_prompt_value_warning_matches_the_oracle_in_every_locale() {
    let template = "Ignored a malformed value: {} (expected NAME=text).";
    let item = "--prompt: [red]bad[/red]";
    assert_eq!(
        format_text(Locale::En, template, &[&item]),
        "Ignored a malformed value: --prompt: [red]bad[/red] (expected NAME=text)."
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&item]),
        "已忽略格式错误的值：--prompt: [red]bad[/red]（应为 NAME=text）。"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&item]),
        "已忽略格式錯誤的值：--prompt: [red]bad[/red]（應為 NAME=text）。"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[&item]);
    assert!(pseudo.contains(item), "{pseudo}");
}

#[test]
fn non_secret_environment_source_warning_matches_the_oracle_in_every_locale() {
    let template = "{} isn't secret; --env-source only applies to secret parameters (mark it with --secret first).";
    assert_eq!(
        format_text(Locale::En, template, &[&"WIDTH"]),
        "WIDTH isn't secret; --env-source only applies to secret parameters (mark it with --secret first)."
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&"WIDTH"]),
        "WIDTH 不是机密参数；--env-source 只适用于机密参数（先用 --secret 标记）。"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&"WIDTH"]),
        "WIDTH 不是機密參數；--env-source 只適用於機密參數（先用 --secret 標記）。"
    );
}

#[test]
fn prompt_runner_required_status_matches_the_oracle_in_every_locale() {
    let source = "A prompt needs a configured agent to run with.";
    assert_eq!(text(Locale::En, source), source);
    assert_eq!(
        text(Locale::ZhCn, source),
        "提示词需要一个已配置的 agent 才能运行。"
    );
    assert_eq!(
        text(Locale::ZhTw, source),
        "提示詞需要一個已設定的 agent 才能執行。"
    );
}

#[test]
fn prompt_unmanaged_preview_matches_the_oracle_in_every_locale() {
    let plain = "Detected but not yet managed: {} (use --add to manage them)";
    assert_eq!(
        format_text(Locale::En, plain, &[&"a, b"]),
        "Detected but not yet managed: a, b (use --add to manage them)"
    );
    assert_eq!(
        format_text(Locale::ZhCn, plain, &[&"a, b"]),
        "检测到但尚未管理:a, b(用 --add 管理)"
    );
    assert_eq!(
        format_text(Locale::ZhTw, plain, &[&"a, b"]),
        "偵測到但尚未管理:a, b(用 --add 管理)"
    );

    let singular =
        "Detected but not yet managed: {} … and {} more candidate (use --add to manage them)";
    assert_eq!(
        format_text(Locale::ZhCn, singular, &[&"a, b", &1]),
        "检测到但尚未管理：a, b……另有 1 个（用 --add 管理）"
    );
    let plural =
        "Detected but not yet managed: {} … and {} more candidates (use --add to manage them)";
    assert_eq!(
        format_text(Locale::ZhTw, plural, &[&"a, b", &4]),
        "偵測到但尚未管理：a, b……另有 4 個（用 --add 管理）"
    );
    let pseudo = format_text(Locale::Pseudo, plural, &[&"a, b", &4]);
    assert!(pseudo.contains("möré"), "{pseudo}");
}

#[test]
fn add_onboarding_controls_and_source_facts_are_fully_localized() {
    assert_eq!(
        text(
            Locale::ZhCn,
            "Select the values that skit should manage (Space toggles; Enter accepts)"
        ),
        "选择由 skit 管理的值（空格切换；回车确认）"
    );
    assert_eq!(
        format_text(
            Locale::ZhTw,
            "✓ skit read this script's own arguments ({} field). Running it opens a form — nothing to memorize.",
            &[&1],
        ),
        "✓ skit 已讀取這支腳本自己的參數（1 個欄位）。執行時會開啟表單，無需記憶指令。"
    );
    assert_eq!(
        format_text(
            Locale::ZhCn,
            "{} ({}) = {}{}",
            &[&"API_KEY", &"str", &"value", &" (机密)"],
        ),
        "API_KEY（str）= value (机密)"
    );
}

#[test]
fn kind_labels_are_localized_for_people_and_open_for_newer_metadata() {
    assert_eq!(kind_label(Locale::En, "python"), "Python");
    assert_eq!(kind_label(Locale::ZhTw, "command"), "指令");
    assert_eq!(kind_label(Locale::ZhCn, "exe"), "程序");
    assert_eq!(kind_label(Locale::ZhTw, "future-kind"), "future-kind");
}

#[test]
fn chinese_terms_keep_distinct_product_meanings() {
    assert_eq!(text(Locale::ZhCn, "Kind"), "类型");
    assert_eq!(text(Locale::ZhCn, "Type: {}"), "类型：{}");
    assert_eq!(text(Locale::ZhCn, "Storage mode"), "存储模式");
    assert_eq!(text(Locale::ZhCn, "Choices: {}"), "可选值：{}");
    assert_eq!(text(Locale::ZhCn, "Options:"), "选项：");
    assert_eq!(text(Locale::ZhCn, "Secret: yes"), "敏感值：是");

    assert_eq!(text(Locale::ZhTw, "Kind"), "類型");
    assert_eq!(text(Locale::ZhTw, "Type: {}"), "類型：{}");
    assert_eq!(text(Locale::ZhTw, "Choices: {}"), "可選值：{}");
    assert_eq!(text(Locale::ZhTw, "Options:"), "選項：");
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
        "用法：skit [OPTIONS]\n\n选项：\n  --help  显示帮助"
    );
    assert_eq!(render(Locale::ZhCn, "(Options:)"), "(选项：)");
    // A composable row never replaces part of a longer word.
    assert_eq!(render(Locale::ZhCn, "Print versions"), "Print versions");
    assert_eq!(render(Locale::ZhCn, "Print helpé"), "Print helpé");
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
