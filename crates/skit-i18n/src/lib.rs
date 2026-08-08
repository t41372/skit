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
