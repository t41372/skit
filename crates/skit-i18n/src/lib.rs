//! Localize skit presentation text without frontend dependencies.

#![forbid(unsafe_code)]

/// One supported presentation locale.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Locale {
    /// English source text.
    #[default]
    En,
    /// Simplified Chinese.
    ZhCn,
    /// Traditional Chinese.
    ZhTw,
}

/// One complete catalog row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Translation {
    /// ASD-STE100 English source text.
    pub english: &'static str,
    /// Simplified Chinese translation.
    pub zh_cn: &'static str,
    /// Traditional Chinese translation.
    pub zh_tw: &'static str,
}

const CATALOG: &[Translation] = &[
    row(
        "A script, prompt, program, and command library",
        "脚本、提示词、程序与命令库",
        "程式、提示詞、執行檔與命令程式庫",
    ),
    row(
        "List entries in the library",
        "列出程序库中的条目",
        "列出程式庫中的項目",
    ),
    row(
        "Show one entry by exact slug or exact display name",
        "按准确短名或准确显示名称显示一个条目",
        "依完整短名或完整顯示名稱顯示一個項目",
    ),
    row(
        "Add one file as a copied or referenced entry",
        "将一个文件添加为副本或引用条目",
        "將一個檔案新增為副本或參照項目",
    ),
    row(
        "Run one library entry",
        "运行一个程序库条目",
        "執行一個程式庫項目",
    ),
    row(
        "Replace one entry description",
        "替换一个条目的说明",
        "取代一個項目的說明",
    ),
    row(
        "Rename one entry and derive its new slug",
        "重命名一个条目并生成新短名",
        "重新命名一個項目並產生新短名",
    ),
    row("Remove one entry", "删除一个条目", "移除一個項目"),
    row(
        "Open an entry source in the configured editor",
        "在配置的编辑器中打开条目源文件",
        "在設定的編輯器中開啟項目原始檔",
    ),
    row(
        "Read or edit managed and declared parameters",
        "读取或编辑受管和声明的参数",
        "讀取或編輯受管與宣告的參數",
    ),
    row(
        "Read or update dependencies and required commands",
        "读取或更新依赖项和所需命令",
        "讀取或更新相依套件與必要命令",
    ),
    row(
        "Check runtime and library health",
        "检查运行环境与程序库健康状态",
        "檢查執行環境與程式庫健康狀態",
    ),
    row(
        "Read or set skit configuration",
        "读取或设置 skit 配置",
        "讀取或設定 skit 組態",
    ),
    row(
        "Manage prompt runners",
        "管理提示词运行器",
        "管理提示詞執行器",
    ),
    row(
        "Manage named parameter presets",
        "管理命名参数预设",
        "管理具名參數預設",
    ),
    row(
        "Install the official Agent Skill",
        "安装官方 Agent Skill",
        "安裝官方 Agent Skill",
    ),
    row(
        "Open the Ratatui library browser",
        "打开 Ratatui 程序库浏览器",
        "開啟 Ratatui 程式庫瀏覽器",
    ),
    row(
        "Library: all entries",
        "程序库：所有条目",
        "程式庫：所有項目",
    ),
    row(
        "No matching entries. Press [q] Quit.",
        "没有匹配的条目。按 [q] 退出。",
        "沒有相符的項目。按 [q] 結束。",
    ),
    row("No matching entries", "没有匹配的条目", "沒有相符的項目"),
    row("all entries", "所有条目", "所有項目"),
    row("Library", "程序库", "程式庫"),
    row("Search", "搜索", "搜尋"),
    row("Entries", "条目", "項目"),
    row("Details", "详细信息", "詳細資料"),
    row("Quit", "退出", "結束"),
    row("Reload", "重新加载", "重新載入"),
    row(
        "damaged entries hidden",
        "个损坏条目已隐藏",
        "個損毀項目已隱藏",
    ),
    row("Usage:", "用法：", "用法："),
    row("Commands:", "命令：", "命令："),
    row("Options:", "选项：", "選項："),
    row("Arguments:", "参数：", "引數："),
    row("Print help", "显示帮助", "顯示說明"),
    row("Print version", "显示版本", "顯示版本"),
    row("entry not found:", "找不到条目：", "找不到項目："),
    row("confirmation is required", "需要确认", "需要確認"),
    row("Added:", "已添加：", "已新增："),
    row("Removed:", "已删除：", "已移除："),
    row("Renamed:", "已重命名：", "已重新命名："),
    row("Edited:", "已编辑：", "已編輯："),
    row("warning:", "警告：", "警告："),
    row("Slug", "短名", "短名"),
    row("Kind", "类型", "類型"),
    row("Storage mode", "存储模式", "儲存模式"),
    row("Run", "运行", "執行"),
    row("Add", "添加", "新增"),
    row("Edit", "编辑", "編輯"),
    row("Settings", "设置", "設定"),
    row("Presets", "预设", "預設"),
    row("Rename", "重命名", "重新命名"),
    row("Remove", "删除", "移除"),
    row("Preferences", "偏好设置", "偏好設定"),
    row("Health", "健康状态", "健康狀態"),
    row("Runners", "运行器", "執行器"),
    row("Back", "返回", "返回"),
    row("Next field", "下一字段", "下一欄位"),
    row("Cancel", "取消", "取消"),
    row("Confirm removal", "确认删除", "確認移除"),
    row("Remove this entry:", "删除此条目：", "移除此項目："),
    row("Add an entry", "添加条目", "新增項目"),
    row("Source path", "源文件路径", "來源路徑"),
    row("Name", "名称", "名稱"),
    row("Description", "说明", "說明"),
    row(
        "Storage mode (copy or reference)",
        "存储模式（副本或引用）",
        "儲存模式（副本或參照）",
    ),
    row("Command template", "命令模板", "命令範本"),
    row("Prompt runner", "提示词运行器", "提示詞執行器"),
    row("Package dependencies", "软件包依赖项", "套件相依性"),
    row("Python constraint", "Python 版本约束", "Python 版本限制"),
    row("Save", "保存", "儲存"),
    row("Save as preset", "另存为预设", "另存為預設"),
    row("Extra arguments", "额外参数", "額外引數"),
    row(
        "Dry run (true or false)",
        "试运行（true 或 false）",
        "試執行（true 或 false）",
    ),
    row("Working directory", "工作目录", "工作目錄"),
    row("Interpreter", "解释器", "直譯器"),
    row("Required commands", "所需命令", "必要命令"),
    row(
        "Prompt interpolation (true or false)",
        "提示词插值（true 或 false）",
        "提示詞插值（true 或 false）",
    ),
    row(
        "Resync managed source parameters (true or false)",
        "重新同步受管源参数（true 或 false）",
        "重新同步受管來源參數（true 或 false）",
    ),
    row("Manage source parameters", "管理源参数", "管理來源參數"),
    row(
        "Stop managing source parameters",
        "停止管理源参数",
        "停止管理來源參數",
    ),
    row(
        "Normalize shell parameters",
        "规范化 shell 参数",
        "正規化 shell 參數",
    ),
    row("Add parameters", "添加参数", "新增參數"),
    row("Remove parameters", "删除参数", "移除參數"),
    row("Parameter", "参数", "參數"),
    row("source binding", "源绑定", "來源繫結"),
    row("delivery", "传递方式", "傳遞方式"),
    row("type", "类型", "類型"),
    row("default", "默认值", "預設值"),
    row("choices", "选项", "選項"),
    row("is required", "为必填项", "為必填欄位"),
    row("takes multiple values", "接受多个值", "接受多個值"),
    row("repeats its flag", "重复其标志", "重複其旗標"),
    row("prompt", "提示", "提示"),
    row("help", "帮助", "說明"),
    row("is secret", "为敏感值", "為機密值"),
    row(
        "secret environment source",
        "敏感值环境变量来源",
        "機密值環境變數來源",
    ),
    row("environment target", "环境变量目标", "環境變數目標"),
    row("flag", "标志", "旗標"),
    row("flag action", "标志操作", "旗標動作"),
    row("Missing targets", "缺失目标", "遺失目標"),
    row("Data directory", "数据目录", "資料目錄"),
    row("Runner name", "运行器名称", "執行器名稱"),
    row(
        "Remove (true or false)",
        "删除（true 或 false）",
        "移除（true 或 false）",
    ),
    row("Preset name", "预设名称", "預設名稱"),
    row(
        "Action (save or delete)",
        "操作（保存或删除）",
        "動作（儲存或刪除）",
    ),
    row("Apply", "应用", "套用"),
    row("ok", "正常", "正常"),
    row("error", "错误", "錯誤"),
];

const fn row(english: &'static str, zh_cn: &'static str, zh_tw: &'static str) -> Translation {
    Translation {
        english,
        zh_cn,
        zh_tw,
    }
}

/// Return the full catalog for verification and frontend tooling.
#[must_use]
pub const fn catalog() -> &'static [Translation] {
    CATALOG
}

/// Detect a supported locale from a language or POSIX locale spelling.
#[must_use]
pub fn detect_locale(value: Option<&str>) -> Locale {
    let normalized = value
        .unwrap_or_default()
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    if normalized.starts_with("zh-tw") || normalized.starts_with("zh-hant") {
        Locale::ZhTw
    } else if normalized.starts_with("zh-cn")
        || normalized.starts_with("zh-hans")
        || normalized == "zh"
    {
        Locale::ZhCn
    } else {
        Locale::En
    }
}

/// Translate one complete source string when it is in the catalog.
#[must_use]
pub fn text(locale: Locale, english: &str) -> &str {
    if locale == Locale::En {
        return english;
    }
    CATALOG
        .iter()
        .find(|row| row.english == english)
        .map_or(english, |row| localized(locale, row))
}

/// Translate every known fragment in rendered framework or error text.
#[must_use]
pub fn render(locale: Locale, english: &str) -> String {
    if locale == Locale::En {
        return english.to_owned();
    }
    let mut rows = CATALOG.iter().collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.english.len()));
    rows.into_iter().fold(english.to_owned(), |output, row| {
        output.replace(row.english, localized(locale, row))
    })
}

const fn localized(locale: Locale, row: &Translation) -> &'static str {
    match locale {
        Locale::En => row.english,
        Locale::ZhCn => row.zh_cn,
        Locale::ZhTw => row.zh_tw,
    }
}
