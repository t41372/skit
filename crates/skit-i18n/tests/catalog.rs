use std::fmt::Display;

use skit_i18n::{
    Locale, available_locale_tags, catalog, detect_locale, format_text, kind_label, render,
    requested_locale, text,
};

fn assert_zh(template: &str, args: &[&dyn Display], simplified: &str, traditional: &str) {
    assert_eq!(format_text(Locale::ZhCn, template, args), simplified);
    assert_eq!(format_text(Locale::ZhTw, template, args), traditional);
}

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
        text(Locale::Pseudo, "%s has %d item(s) at 100%%"),
        "⟦%s hàs %d îtém(s) àt 100%%~~⟧"
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
fn owned_draft_cleanup_warning_is_complete_in_every_locale() {
    let source = "The kept draft changed before cleanup. skit kept it at {}.";
    let path = "/data/drafts/skit-new-task.py";
    assert_eq!(
        format_text(Locale::En, source, &[&path]),
        format!("The kept draft changed before cleanup. skit kept it at {path}.")
    );
    assert_eq!(
        format_text(Locale::ZhCn, source, &[&path]),
        format!("保留的草稿在清理前发生了更改。skit 将它保留在 {path}。")
    );
    assert_eq!(
        format_text(Locale::ZhTw, source, &[&path]),
        format!("保留的草稿在清理前發生了變更。skit 將它保留在 {path}。")
    );
}

#[test]
fn owned_draft_quarantine_restore_error_is_complete_in_every_locale() {
    let source = "could not restore quarantined draft {} to {}: {}";
    let quarantine = "/data/drafts/.skit-quarantine-1";
    let original = "/data/drafts/skit-new-task.py";
    let reason = "already exists";
    assert_eq!(
        format_text(Locale::En, source, &[&quarantine, &original, &reason]),
        format!("could not restore quarantined draft {quarantine} to {original}: {reason}")
    );
    assert_eq!(
        format_text(Locale::ZhCn, source, &[&quarantine, &original, &reason]),
        format!("无法将隔离的草稿 {quarantine} 恢复到 {original}：{reason}")
    );
    assert_eq!(
        format_text(Locale::ZhTw, source, &[&quarantine, &original, &reason]),
        format!("無法將隔離的草稿 {quarantine} 還原到 {original}：{reason}")
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
fn managed_parameter_receipt_matches_the_oracle_in_every_locale() {
    let template = "Updated {}. Managed parameters: {}";
    let name = "[blue]a[/blue]";
    assert_eq!(
        format_text(Locale::En, template, &[&name, &"CITY, RETRIES"]),
        "Updated [blue]a[/blue]. Managed parameters: CITY, RETRIES"
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&name, &"CITY, RETRIES"]),
        "已更新 [blue]a[/blue]。受管理的参数:CITY, RETRIES"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&name, &"—"]),
        "已更新 [blue]a[/blue]。受管理的參數:—"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[&name, &"CITY"]);
    assert!(pseudo.contains(name), "{pseudo}");
}

#[test]
fn declared_parameter_receipt_matches_the_oracle_in_every_locale() {
    let template = "Updated {}. Declared parameters: {}";
    assert_eq!(
        format_text(Locale::En, template, &[&"prog", &"width"]),
        "Updated prog. Declared parameters: width"
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&"prog", &"a, b"]),
        "已更新 prog。已声明的参数:a, b"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&"prog", &"—"]),
        "已更新 prog。已宣告的參數:—"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[&"prog", &"width"]);
    assert!(pseudo.contains("prog"), "{pseudo}");
    assert!(pseudo.contains("width"), "{pseudo}");
}

#[test]
fn managed_parameter_flip_note_matches_the_oracle_in_every_locale() {
    let template = "The run form now asks for the managed parameters — the script's own command-line form ({}) is set aside until they are removed (--unmanage).";
    assert_eq!(
        format_text(Locale::En, template, &[&"getopts"]),
        "The run form now asks for the managed parameters — the script's own command-line form (getopts) is set aside until they are removed (--unmanage)."
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&"getopts"]),
        "运行表单现在会询问这些管理的参数——脚本自己的命令行表单（getopts）会先搁置，直到它们被移除（--unmanage）为止。"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&"getopts"]),
        "執行表單現在會詢問這些管理的參數——腳本自己的命令列表單（getopts）會先擱置，直到它們被移除（--unmanage）為止。"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[&"getopts"]);
    assert!(pseudo.contains("getopts"), "{pseudo}");
}

#[test]
fn add_dispatch_messages_match_the_oracle_in_every_locale() {
    let template = "{} is a directory — pass --exe to add it as a program that runs directly.";
    assert_eq!(
        format_text(Locale::En, template, &[&"bundle"]),
        "bundle is a directory — pass --exe to add it as a program that runs directly."
    );
    assert_eq!(
        format_text(Locale::ZhCn, template, &[&"bundle"]),
        "bundle 是一个目录——加 --exe 可把它作为直接运行的程序加入。"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[&"bundle"]),
        "bundle 是一個目錄——加 --exe 可把它作為直接執行的程式加入。"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[&"bundle"]);
    assert!(pseudo.contains("bundle"), "{pseudo}");

    for (source, zh_cn, zh_tw) in [
        (
            "--prompt names the kind outright — drop --edit/--exe/--kind/--cmd.",
            "--prompt 已直接指定类型——请去掉 --edit/--exe/--kind/--cmd。",
            "--prompt 已直接指定類型——請去掉 --edit/--exe/--kind/--cmd。",
        ),
        ("stdin ('-')", "stdin（'-'）", "stdin（'-'）"),
        ("a file path", "文件路径", "檔案路徑"),
        (
            "{} each pick a different way to add — use exactly one (nothing was added).",
            "{} 各自代表一种不同的添加方式——请只用其中一种（未添加任何内容）。",
            "{} 各自代表一種不同的加入方式——請只用其中一種（未加入任何內容）。",
        ),
        (
            "a --cmd template takes only --name/--description",
            "--cmd 模板只接受 --name/--description",
            "--cmd 樣板只接受 --name/--description",
        ),
        (
            "stdin authors a brand-new copy, and --ref/--exe need an existing file",
            "stdin 会撰写一份全新副本，而 --ref/--exe 需要现成的文件",
            "stdin 會撰寫一份全新副本，而 --ref/--exe 需要現成的檔案",
        ),
        (
            "--edit drafts a fresh script: its kind comes from the shebang you write (e.g. #!/usr/bin/env bash), --ref/--exe need an existing file, and a prompt is drafted with skit add --prompt",
            "--edit 会起草一个全新脚本：它的类型取自你写的 shebang（例如 #!/usr/bin/env bash），--ref/--exe 需要现成的文件，而提示词要用 skit add --prompt 起草",
            "--edit 會草擬一支全新腳本：它的類型取自你寫的 shebang（例如 #!/usr/bin/env bash），--ref/--exe 需要現成的檔案，而提示詞要用 skit add --prompt 草擬",
        ),
        (
            "a drafted prompt takes only --name/--description/--runner/--no-interpolate",
            "草稿提示词只接受 --name/--description/--runner/--no-interpolate",
            "草稿提示詞只接受 --name/--description/--runner/--no-interpolate",
        ),
        (
            "{} can't apply here — {} (nothing was added).",
            "{} 在这里无法应用——{}(未添加任何内容)。",
            "{} 在這裡無法套用——{}(未加入任何內容)。",
        ),
        (
            "--no-interpolate only applies to prompt entries — add one with --prompt.",
            "--no-interpolate 只适用于提示词条目——用 --prompt 添加一个。",
            "--no-interpolate 只適用於提示詞項目——用 --prompt 加入一個。",
        ),
        (
            "--runner only applies to prompt entries — add one with --prompt.",
            "--runner 只适用于提示词条目——用 --prompt 添加一个。",
            "--runner 只適用於提示詞項目——用 --prompt 加入一個。",
        ),
    ] {
        assert_eq!(text(Locale::En, source), source);
        assert_eq!(text(Locale::ZhCn, source), zh_cn);
        assert_eq!(text(Locale::ZhTw, source), zh_tw);
    }
}

#[test]
fn prompt_editor_no_input_hint_matches_the_oracle_in_every_locale() {
    let template = "--prompt with no path opens your editor, which --no-input forbids — pipe the body in instead: skit add - --prompt -n NAME";
    assert_eq!(format_text(Locale::En, template, &[]), template);
    assert_eq!(
        format_text(Locale::ZhCn, template, &[]),
        "--prompt 未带路径时会打开你的编辑器，而 --no-input 禁止这么做——请改用管道把正文传进来：skit add - --prompt -n NAME"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[]),
        "--prompt 未帶路徑時會開啟你的編輯器，而 --no-input 禁止這麼做——請改用管道把內文傳進來：skit add - --prompt -n NAME"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[]);
    assert!(pseudo.starts_with('⟦'), "{pseudo}");
}

#[test]
fn script_editor_terminal_requirement_matches_the_oracle_in_every_locale() {
    let template = "Writing a new script in an editor needs an interactive terminal.";
    assert_eq!(format_text(Locale::En, template, &[]), template);
    assert_eq!(
        format_text(Locale::ZhCn, template, &[]),
        "用编辑器新建脚本需要交互式终端。"
    );
    assert_eq!(
        format_text(Locale::ZhTw, template, &[]),
        "用編輯器新建腳本需要互動式終端機。"
    );
    let pseudo = format_text(Locale::Pseudo, template, &[]);
    assert!(pseudo.starts_with('⟦'), "{pseudo}");
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

    assert_zh(
        "Ignored a malformed value: {} (expected NAME=VALUE).",
        &[&"--type: bad"],
        "已忽略格式错误的值：--type: bad（应为 NAME=VALUE）。",
        "已忽略格式錯誤的值：--type: bad（應為 NAME=VALUE）。",
    );
    assert_zh(
        "{} isn't a declared parameter; skipped.",
        &[&"x"],
        "x 不是已声明的参数，已跳过。",
        "x 不是已宣告的參數，已跳過。",
    );
    assert_zh(
        "{} is already declared; skipped.",
        &[&"x"],
        "x 已经声明过，已跳过。",
        "x 已經宣告過，已跳過。",
    );
    assert_zh(
        "{}: that delivery isn't available for this kind; skipped.",
        &[&"x"],
        "x：该传递方式不适用于此类型，已跳过。",
        "x：該傳遞方式不適用於此類型，已跳過。",
    );
    assert_zh(
        "{} isn't a template placeholder, so it can't use placeholder delivery; skipped.",
        &[&"x"],
        "x 不是模板占位符，无法使用 placeholder 传递方式，已跳过。",
        "x 不是模板佔位符，無法使用 placeholder 傳遞方式，已跳過。",
    );
    assert_zh(
        "{}: unknown type; skipped (use str, int, float, bool, choice, or path).",
        &[&"x"],
        "x：未知类型，已跳过(可用 str、int、float、bool、choice 或 path)。",
        "x：未知類型，已跳過(可用 str、int、float、bool、choice 或 path)。",
    );
    assert_zh(
        "{}: the default doesn't fit its type; skipped.",
        &[&"x"],
        "x：默认值与其类型不符，已跳过。",
        "x：預設值與其類型不符，已跳過。",
    );
    assert_zh(
        "{} isn't secret; --env-source only applies to secret parameters (mark it with --secret first).",
        &[&"x"],
        "x 不是机密参数；--env-source 只适用于机密参数（先用 --secret 标记）。",
        "x 不是機密參數；--env-source 只適用於機密參數（先用 --secret 標記）。",
    );
    assert_zh(
        "{}: a choice parameter needs choices; set --choices {}=a,b,c.",
        &[&"x", &"x"],
        "x：choice 参数需要可选值，请设置 --choices x=a,b,c。",
        "x：choice 參數需要可選值，請設定 --choices x=a,b,c。",
    );
    assert_zh(
        "{} is on by default, so its flag could only ever turn it on again. Declare the flag that turns it OFF instead (--no-{} and the like), with default false.",
        &[&"x", &"x"],
        "x 默认就是开的，它的标志只会再开一次。请改成声明用来关掉它的那个标志(--no-x 之类)，默认 false。",
        "x 預設就是開的，它的旗標只會再開一次。請改成宣告用來關掉它的那個旗標(--no-x 之類)，預設 false。",
    );
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

    let heading = "Prompt placeholders (the run form asks for them):";
    assert_eq!(
        text(Locale::ZhCn, heading),
        "提示词的占位符(运行表单会询问):"
    );
    assert_eq!(
        text(Locale::ZhTw, heading),
        "提示詞的佔位符(執行表單會詢問):"
    );
    let command_heading = "Command template placeholders (the run form asks for them):";
    assert_eq!(
        text(Locale::ZhCn, command_heading),
        "命令模板的占位符（运行表单会询问）："
    );
    assert_eq!(
        text(Locale::ZhTw, command_heading),
        "命令樣板的佔位符（執行表單會詢問）："
    );
    let environment = "Declared environment variables (set on the run):";
    assert_eq!(
        text(Locale::ZhCn, environment),
        "声明的环境变量（运行时设置）："
    );
    assert_eq!(
        text(Locale::ZhTw, environment),
        "宣告的環境變數（執行時設定）："
    );
    assert_eq!(
        format_text(Locale::ZhCn, "default {}", &[&"•••"]),
        "默认 •••"
    );
    assert_eq!(text(Locale::ZhTw, "optional"), "選填");
    assert_eq!(text(Locale::ZhCn, "secret"), "机密");
    let gone = "No longer in the prompt (the value would be ignored): {} — remove with --rm, or edit the body.";
    assert_eq!(
        format_text(Locale::ZhCn, gone, &[&"a"]),
        "提示词中已不存在(其值会被忽略):a——用 --rm 移除,或编辑正文。"
    );
    assert_eq!(
        format_text(Locale::ZhTw, gone, &[&"a"]),
        "提示詞中已不存在(其值會被忽略):a——用 --rm 移除,或編輯內文。"
    );
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
