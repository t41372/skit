//! Localize skit presentation text without frontend dependencies.

#![forbid(unsafe_code)]

use std::fmt::{Display, Write as _};

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
    /// Whether [`render`] can replace this row inside composed text.
    ///
    /// Most rows are exact text that [`text`] or [`format_text`] selects as a whole. A short row
    /// such as `help` must never replace part of composed framework output such as `--help`.
    pub composable: bool,
}

macro_rules! row {
    ($english:literal, $zh_cn:literal, $zh_tw:literal $(,)?) => {
        Translation {
            english: $english,
            zh_cn: $zh_cn,
            zh_tw: $zh_tw,
            composable: false,
        }
    };
}

/// Declare one row that [`render`] can also use inside composed text.
macro_rules! composable {
    ($english:literal, $zh_cn:literal, $zh_tw:literal $(,)?) => {
        Translation {
            english: $english,
            zh_cn: $zh_cn,
            zh_tw: $zh_tw,
            composable: true,
        }
    };
}

const CATALOG: &[Translation] = &[
    row!(
        "A script, prompt, program, and command library",
        "脚本、提示词、程序与命令库",
        "程式、提示詞、執行檔與命令程式庫",
    ),
    row!(
        "List entries in the library",
        "列出程序库中的条目",
        "列出程式庫中的項目",
    ),
    row!(
        "Show one entry by exact slug or exact display name",
        "按准确短名或准确显示名称显示一个条目",
        "依完整短名或完整顯示名稱顯示一個項目",
    ),
    row!(
        "Add one file as a copied or referenced entry",
        "将一个文件添加为副本或引用条目",
        "將一個檔案新增為副本或參照項目",
    ),
    row!(
        "Run one library entry",
        "运行一个程序库条目",
        "執行一個程式庫項目",
    ),
    row!(
        "Replace one entry description",
        "替换一个条目的说明",
        "取代一個項目的說明",
    ),
    row!(
        "Rename one entry without changing its slug",
        "重命名一个条目，但不更改其短名",
        "重新命名一個項目，但不變更其短名",
    ),
    row!("Remove one entry", "删除一个条目", "移除一個項目"),
    row!(
        "Open an entry source in the configured editor",
        "在配置的编辑器中打开条目源文件",
        "在設定的編輯器中開啟項目原始檔",
    ),
    row!(
        "Read or edit managed and declared parameters",
        "读取或编辑受管和声明的参数",
        "讀取或編輯受管與宣告的參數",
    ),
    row!(
        "Set source binding as NAME=BINDING",
        "将源绑定设置为 NAME=BINDING",
        "將來源繫結設定為 NAME=BINDING",
    ),
    row!(
        "Allow more than one value for a field",
        "允许字段接受多个值",
        "允許欄位接受多個值",
    ),
    row!(
        "Allow only one value for a field",
        "仅允许字段接受一个值",
        "僅允許欄位接受一個值",
    ),
    row!(
        "Repeat the flag for each value",
        "为每个值重复该选项",
        "為每個值重複該選項",
    ),
    row!(
        "Put all values after one flag",
        "将所有值放在一个选项之后",
        "將所有值放在一個選項之後",
    ),
    row!(
        "Set an environment target as NAME=ENVVAR",
        "将环境目标设置为 NAME=ENVVAR",
        "將環境目標設定為 NAME=ENVVAR",
    ),
    row!(
        "Set a boolean flag action as NAME=ACTION",
        "将布尔选项操作设置为 NAME=ACTION",
        "將布林選項動作設定為 NAME=ACTION",
    ),
    row!(
        "Remove one malformed raw row by its one-based index",
        "按从一开始的索引删除一个格式错误的原始行",
        "依從一開始的索引移除一個格式錯誤的原始列",
    ),
    row!(
        "Read or update dependencies and required commands",
        "读取或更新依赖项和所需命令",
        "讀取或更新相依套件與必要命令",
    ),
    row!(
        "Check runtime and library health",
        "检查运行环境与程序库健康状态",
        "檢查執行環境與程式庫健康狀態",
    ),
    row!(
        "Read or set skit configuration",
        "读取或设置 skit 配置",
        "讀取或設定 skit 組態",
    ),
    row!(
        "Manage prompt runners",
        "管理提示词运行器",
        "管理提示詞執行器",
    ),
    row!(
        "Manage named parameter presets",
        "管理命名参数预设",
        "管理具名參數預設",
    ),
    row!(
        "Install the official Agent Skill",
        "安装官方 Agent Skill",
        "安裝官方 Agent Skill",
    ),
    row!(
        "Open the Ratatui library browser",
        "打开 Ratatui 程序库浏览器",
        "開啟 Ratatui 程式庫瀏覽器",
    ),
    composable!(
        "Library: all entries",
        "程序库：所有条目",
        "程式庫：所有項目",
    ),
    composable!(
        "No matching entries. Press [q] Quit.",
        "没有匹配的条目。按 [q] 退出。",
        "沒有相符的項目。按 [q] 結束。",
    ),
    row!("No matching entries", "没有匹配的条目", "沒有相符的項目"),
    row!("valid", "有效", "有效"),
    row!("row {}", "第 {} 行", "第 {} 列"),
    row!(
        "runner row needs a name and a string argv array",
        "运行器行需要名称和字符串 argv 数组",
        "執行器資料列需要名稱與字串 argv 陣列",
    ),
    row!("type", "类型", "類型"),
    row!("choices", "可选值", "可選值"),
    row!("default", "默认值", "預設值"),
    row!("delivery", "传递方式", "傳遞方式"),
    row!("binding", "源绑定", "來源繫結"),
    row!("flag", "命令行选项", "命令列選項"),
    row!("field", "字段", "欄位"),
    row!("environment target", "环境目标", "環境目標"),
    row!("action", "操作", "動作"),
    row!("help text", "帮助文本", "說明文字"),
    row!("prompt", "提示语", "提示語"),
    row!("environment source", "环境来源", "環境來源"),
    row!("runner removal", "移除提示词运行器", "移除提示詞執行器"),
    row!("preset deletion", "删除预设", "刪除預設"),
    row!("runner remove", "移除提示词运行器", "移除提示詞執行器"),
    row!("read", "读取", "讀取"),
    row!("resolve", "解析", "解析"),
    row!("stage", "暂存", "暫存"),
    row!("start editor for", "启动编辑器以打开", "啟動編輯器以開啟"),
    row!("create", "创建", "建立"),
    row!(
        "could not format add timestamp: {}",
        "无法设置添加时间：{}",
        "無法設定新增時間：{}",
    ),
    row!("write", "写入", "寫入"),
    row!("open", "打开", "開啟"),
    row!("inspect", "检查", "檢查"),
    row!("scan", "扫描", "掃描"),
    row!("lock", "锁定", "鎖定"),
    row!("remove", "删除", "移除"),
    row!("rollback", "回滚", "復原"),
    row!(
        "rollback at {} failed after {}: {}",
        "在 {} 处回滚失败；原操作错误：{}；回滚错误：{}",
        "在 {} 處復原失敗；原始操作錯誤：{}；復原錯誤：{}",
    ),
    row!("backup", "备份", "備份"),
    row!("commit", "提交", "提交"),
    row!("reuse", "重新使用", "重新使用"),
    row!("initialize", "初始化", "初始化"),
    row!("sync", "同步", "同步"),
    row!("chmod", "更改权限", "變更權限"),
    row!("replace", "替换", "取代"),
    row!("rollback remove", "回滚删除操作", "復原移除操作"),
    row!("rollback create", "回滚创建操作", "復原建立操作"),
    row!("replace backup", "替换备份", "取代備份"),
    row!(
        "start package manager in",
        "在此处启动软件包管理器",
        "在此處啟動套件管理程式"
    ),
    row!("open lock for", "打开锁文件", "開啟鎖定檔"),
    row!("create backup", "创建备份", "建立備份"),
    row!(
        "commit dependency backup",
        "提交依赖项备份",
        "提交相依套件備份"
    ),
    row!("scan backup", "扫描备份", "掃描備份"),
    row!("recover backup", "恢复备份", "復原備份"),
    row!("remove backup", "删除备份", "移除備份"),
    row!("allocate cleanup", "分配清理目录", "配置清理目錄"),
    row!("create directory for", "创建目录以存放", "建立目錄以存放"),
    row!("create staged", "创建暂存文件", "建立暫存檔"),
    row!("write staged", "写入暂存文件", "寫入暫存檔"),
    row!("sync staged", "同步暂存文件", "同步暫存檔"),
    row!("install", "安装", "安裝"),
    row!("read permissions for", "读取权限", "讀取權限"),
    row!("set permissions for", "设置权限", "設定權限"),
    row!("sync directory", "同步目录", "同步目錄"),
    row!("run", "运行", "執行"),
    row!("rename", "重命名", "重新命名"),
    row!("start", "启动", "啟動"),
    row!("test", "测试", "測試"),
    row!("all entries", "所有条目", "所有項目"),
    row!("Library", "程序库", "程式庫"),
    row!("Search", "搜索", "搜尋"),
    row!("Entries", "条目", "項目"),
    row!("Details", "详细信息", "詳細資料"),
    row!("Quit", "退出", "結束"),
    row!("Reload", "重新加载", "重新載入"),
    row!(
        "damaged entries hidden",
        "个损坏条目已隐藏",
        "個損毀項目已隱藏",
    ),
    composable!("Usage: ", "用法：", "用法："),
    composable!("Usage:", "用法：", "用法："),
    composable!("Commands:", "命令：", "命令："),
    composable!("Options:", "选项：", "選項："),
    composable!("Arguments:", "参数：", "引數："),
    composable!("Print help", "显示帮助", "顯示說明"),
    composable!("Print version", "显示版本", "顯示版本"),
    composable!("error: ", "错误：", "錯誤："),
    composable!("error:", "错误：", "錯誤："),
    composable!("tip: ", "提示：", "提示："),
    composable!("tip:", "提示：", "提示："),
    composable!(
        "For more information, try '--help'.",
        "如需更多信息，请尝试 '--help'。",
        "如需詳細資訊，請嘗試 '--help'。",
    ),
    composable!(
        "For more information, try '",
        "如需更多信息，请尝试 '",
        "如需詳細資訊，請嘗試 '",
    ),
    composable!(
        "unrecognized subcommand '",
        "无法识别子命令 '",
        "無法識別子命令 '",
    ),
    composable!(
        "unexpected argument '",
        "发现意外参数 '",
        "發現非預期引數 '",
    ),
    composable!("' found", "'。", "'。"),
    composable!("the argument '", "参数 '", "引數 '"),
    composable!("the subcommand '", "子命令 '", "子命令 '"),
    composable!(
        "' cannot be used multiple times",
        "' 不能多次使用",
        "' 不能多次使用",
    ),
    composable!(
        "' cannot be used with",
        "' 不能与以下项同时使用",
        "' 不能與下列項目同時使用",
    ),
    composable!(
        "one or more of the other specified arguments",
        "一个或多个其他指定参数",
        "一個或多個其他指定引數",
    ),
    composable!(
        "equal sign is needed when assigning values to '",
        "为参数赋值时需要等号：'",
        "為引數指派值時需要等號：'",
    ),
    composable!(
        "a value is required for '",
        "参数需要值：'",
        "引數需要值：'",
    ),
    composable!("' but none was supplied", "'，但未提供值", "'，但未提供值",),
    composable!("invalid value '", "无效值 '", "無效值 '"),
    composable!("' for '", "'，对应参数 '", "'，對應引數 '"),
    composable!("[possible values: ", "[可用值：", "[可用值："),
    composable!(
        "the following required arguments were not provided:",
        "未提供以下必需参数：",
        "未提供下列必要引數：",
    ),
    composable!(
        "' requires a subcommand but one was not provided",
        "' 需要子命令，但未提供",
        "' 需要子命令，但未提供",
    ),
    composable!("[subcommands: ", "[子命令：", "[子命令："),
    composable!(
        "invalid UTF-8 was detected in one or more arguments",
        "一个或多个参数包含无效的 UTF-8",
        "一個或多個引數包含無效的 UTF-8",
    ),
    composable!("unexpected value '", "出现意外值 '", "出現非預期值 '"),
    composable!(
        "' found; no more were expected",
        "'；不应再提供更多值",
        "'；不應再提供更多值",
    ),
    composable!(" values required by '", " 个值；参数 '", " 個值；引數 '"),
    composable!(" values required for '", " 个值；参数 '", " 個值；引數 '"),
    composable!("'; only ", "' 要求；只提供了 ", "' 要求；只提供了 "),
    composable!(" was provided", " 个值", " 個值"),
    composable!(" were provided", " 个值", " 個值"),
    composable!(
        "a similar subcommand exists: ",
        "有相似的子命令：",
        "有相似的子命令：",
    ),
    composable!(
        "some similar subcommands exist: ",
        "有一些相似的子命令：",
        "有一些相似的子命令：",
    ),
    composable!(
        "a similar argument exists: ",
        "有相似的参数：",
        "有相似的引数：",
    ),
    composable!(
        "some similar arguments exist: ",
        "有一些相似的参数：",
        "有一些相似的引數：",
    ),
    composable!("a similar value exists: ", "有相似的值：", "有相似的值：",),
    composable!(
        "some similar values exist: ",
        "有一些相似的值：",
        "有一些相似的值：",
    ),
    composable!("to pass '", "要传递 '", "要傳遞 '"),
    composable!(
        "' as a value, use '",
        "' 作为值，请使用 '",
        "' 作為值，請使用 '",
    ),
    composable!("subcommand '", "子命令 '", "子命令 '"),
    composable!(
        "' exists; to use it, remove the '",
        "' 存在；要使用它，请删除前面的 '",
        "' 存在；要使用它，請移除前面的 '",
    ),
    composable!("' before it", "'。", "'。"),
    row!(
        "standard input cannot be an executable entry",
        "标准输入不能用作可执行文件条目",
        "標準輸入不能用作執行檔項目",
    ),
    row!(
        "--no-interpolate only applies to prompt entries",
        "--no-interpolate 仅适用于提示词条目",
        "--no-interpolate 僅適用於提示詞項目",
    ),
    row!(
        "cannot save a preset because the entry has no form fields",
        "无法保存预设，因为该条目没有表单字段",
        "無法儲存預設，因為該項目沒有表單欄位",
    ),
    row!("Added: {} ({})", "已添加：{} ({})", "已新增：{} ({})"),
    row!(
        "Description updated: {} ({})",
        "说明已更新：{} ({})",
        "說明已更新：{} ({})",
    ),
    row!(
        "Renamed: {} ({})",
        "已重命名：{} ({})",
        "已重新命名：{} ({})",
    ),
    row!("Removed: {}", "已删除：{}", "已移除：{}"),
    row!("Edited: {} ({})", "已编辑：{} ({})", "已編輯：{} ({})"),
    row!("Set: {}={}", "已设置：{}={}", "已設定：{}={}"),
    row!("Added runner: {}", "已添加运行器：{}", "已新增執行器：{}"),
    row!("Removed runner: {}", "已删除运行器：{}", "已移除執行器：{}"),
    row!("Saved preset: {}", "已保存预设：{}", "已儲存預設：{}"),
    row!("Deleted preset: {}", "已删除预设：{}", "已刪除預設：{}"),
    row!(
        "Installed completion: {}",
        "已安装补全脚本：{}",
        "已安裝補全指令稿：{}",
    ),
    row!(
        "Your draft was kept at {}",
        "草稿已保留在 {}",
        "草稿已保留在 {}",
    ),
    row!("Dependencies: {}", "依赖项：{}", "相依套件：{}"),
    row!(
        "Python constraint: {}",
        "Python 版本约束：{}",
        "Python 版本限制：{}",
    ),
    row!("Required commands: {}", "所需命令：{}", "必要命令：{}"),
    row!(
        "First Python run: download private uv {}",
        "首次运行 Python：下载专用 uv {}",
        "首次執行 Python：下載專用 uv {}",
    ),
    row!(
        "Added a newline to keep the Pi prompt in message mode",
        "已添加换行符，使 Pi 提示词保持消息模式",
        "已新增換行字元，使 Pi 提示詞保持訊息模式",
    ),
    row!("OK uv: {}", "正常 uv：{}", "正常 uv：{}"),
    row!("OK uv: not required", "正常 uv：不需要", "正常 uv：不需要",),
    row!("ERROR uv: not found", "错误 uv：未找到", "錯誤 uv：找不到"),
    row!("Entries: {}", "条目：{}", "項目：{}"),
    row!(
        "Library: {} ({} bytes)",
        "程序库：{}（{} 字节）",
        "程式庫：{}（{} 位元組）",
    ),
    row!("State: {}", "状态数据：{}", "狀態資料：{}"),
    row!("Config: {}", "配置：{}", "組態：{}"),
    row!("Registry rebuilt: {}", "索引已重建：{}", "索引已重建：{}"),
    row!(
        "WARN {}: the launch target is gone from disk",
        "警告 {}：启动目标已不在磁盘上",
        "警告 {}：啟動目標已不在磁碟上",
    ),
    row!(
        "WARN {}: form definitions are out of sync; run: skit params {} --resync",
        "警告 {}：表单定义不同步；请运行：skit params {} --resync",
        "警告 {}：表單定義不同步；請執行：skit params {} --resync",
    ),
    row!(
        "WARN {}: missing external commands: {}",
        "警告 {}：缺少外部命令：{}",
        "警告 {}：缺少外部命令：{}",
    ),
    row!(
        "WARN {}: a run would refuse to start: {}",
        "警告 {}：运行将无法启动：{}",
        "警告 {}：執行將無法啟動：{}",
    ),
    row!(
        "WARN malformed prompt runners: {}",
        "警告 提示词运行器格式错误：{}",
        "警告 提示詞執行器格式錯誤：{}",
    ),
    row!("WARN {}", "警告 {}", "警告 {}"),
    row!(
        "Installed Agent Skill: {}",
        "已安装 Agent Skill：{}",
        "已安裝 Agent Skill：{}",
    ),
    row!(
        "Remove \"{}\"? [y/N]: ",
        "删除“{}”？[y/N]：",
        "移除「{}」？[y/N]：",
    ),
    row!(
        "No editable entry is named \"{}\". Create a script now? [Y/n]: ",
        "没有名为“{}”的可编辑条目。立即创建脚本吗？[Y/n]：",
        "沒有名為「{}」的可編輯項目。立即建立程式嗎？[Y/n]：",
    ),
    row!(
        "Remove runner \"{}\"? [y/N]: ",
        "删除运行器“{}”？[y/N]：",
        "移除執行器「{}」？[y/N]：",
    ),
    row!(
        "Delete preset \"{}\"? [y/N]: ",
        "删除预设“{}”？[y/N]：",
        "刪除預設「{}」？[y/N]：",
    ),
    composable!("Source saved", "源文件已保存", "來源已儲存"),
    composable!("Entry removed", "条目已删除", "項目已移除"),
    composable!("Entry added", "条目已添加", "項目已新增"),
    composable!("Settings saved", "设置已保存", "設定已儲存"),
    composable!("Preferences saved", "偏好设置已保存", "偏好設定已儲存"),
    composable!(
        "Prompt runners saved",
        "提示词运行器已保存",
        "提示詞執行器已儲存",
    ),
    composable!("Presets saved", "预设已保存", "預設已儲存"),
    composable!("Entry renamed", "条目已重命名", "項目已重新命名"),
    composable!(
        "Run finished with exit status",
        "运行完成，退出状态为",
        "執行完成，結束狀態為",
    ),
    row!("warning: {}", "警告：{}", "警告：{}"),
    row!("Slug", "短名", "短名"),
    row!("Kind", "种类", "種類"),
    row!("Kind: {}", "种类：{}", "種類：{}"),
    row!("Storage mode", "存储模式", "儲存模式"),
    row!("Storage mode: {}", "存储模式：{}", "儲存模式：{}"),
    row!("Source: {}", "来源：{}", "來源：{}"),
    row!("Work directory: {}", "工作目录：{}", "工作目錄：{}"),
    row!("Missing: {}", "缺失：{}", "遺失：{}"),
    row!("Drift: {}", "漂移：{}", "偏移：{}"),
    row!("Interpreter: {}", "解释器：{}", "直譯器：{}"),
    row!("Template: {}", "模板：{}", "範本：{}"),
    row!("Prompt runner: {}", "提示词运行器：{}", "提示詞執行器：{}"),
    row!("Interpolation: {}", "插值：{}", "插值：{}"),
    row!("Parameters:", "参数：", "參數："),
    row!("  {} ({}, {})", "  {}（{}，{}）", "  {}（{}，{}）"),
    row!("Presets: {}", "预设：{}", "預設：{}"),
    row!("Run: skit run {}", "运行：skit run {}", "執行：skit run {}"),
    row!("Parameter: {}", "参数：{}", "參數：{}"),
    row!("Type: {}", "类型：{}", "類型：{}"),
    row!("Delivery: {}", "传递方式：{}", "傳遞方式：{}"),
    row!("Current default: {}", "当前默认值：{}", "目前預設值：{}"),
    row!("Last value: {}", "上次值：{}", "上次值：{}"),
    row!("Choices: {}", "可选值：{}", "可選值：{}"),
    row!("Prompt: {}", "提示：{}", "提示：{}"),
    row!("Help: {}", "帮助：{}", "說明：{}"),
    row!(
        "Environment source: {}",
        "环境变量来源：{}",
        "環境變數來源：{}",
    ),
    row!("Secret: yes", "敏感值：是", "敏感值：是"),
    row!(
        "Unmanaged candidates: {}",
        "未管理候选项：{}",
        "未管理候選項：{}"
    ),
    row!(
        "Source management is not available for a reference entry.",
        "引用条目不能使用来源管理。",
        "參照項目不能使用來源管理。",
    ),
    row!("yes", "是", "是"),
    row!("no", "否", "否"),
    row!("on", "开启", "開啟"),
    row!("off", "关闭", "關閉"),
    row!("not set", "未设置", "未設定"),
    row!("Run", "运行", "執行"),
    row!("Run {}", "运行 {}", "執行 {}"),
    row!("Add", "添加", "新增"),
    row!("Edit", "编辑", "編輯"),
    row!("Settings", "设置", "設定"),
    row!("Settings for {}", "{} 的设置", "{} 的設定"),
    row!("Presets", "预设", "預設"),
    row!("Presets for {}: {}", "{} 的预设：{}", "{} 的預設：{}"),
    row!("Rename", "重命名", "重新命名"),
    row!("Rename {}", "重命名 {}", "重新命名 {}"),
    row!("Remove", "删除", "移除"),
    row!("Preferences", "偏好设置", "偏好設定"),
    row!("Language", "语言", "語言"),
    row!("Editor command", "编辑器命令", "編輯器命令"),
    row!("Form style", "表单样式", "表單樣式"),
    row!("After run", "运行后", "執行後"),
    row!("Bash path", "Bash 路径", "Bash 路徑"),
    row!(
        "JavaScript runtime",
        "JavaScript 运行时",
        "JavaScript 執行環境",
    ),
    row!("Mirror", "镜像", "鏡像"),
    row!("PyPI mirror", "PyPI 镜像", "PyPI 鏡像"),
    row!("GitHub mirror", "GitHub 镜像", "GitHub 鏡像"),
    row!("npm mirror", "npm 镜像", "npm 鏡像"),
    row!("Health", "健康状态", "健康狀態"),
    row!("Runners", "运行器", "執行器"),
    row!("Prompt runners: {}", "提示词运行器：{}", "提示詞執行器：{}"),
    row!("Back", "返回", "返回"),
    row!("Next field", "下一字段", "下一欄位"),
    row!("Cancel", "取消", "取消"),
    row!("Confirm removal", "确认删除", "確認移除"),
    row!("Remove this entry:", "删除此条目：", "移除此項目："),
    row!("Add an entry", "添加条目", "新增項目"),
    row!("Source path", "源文件路径", "來源路徑"),
    row!("Name", "名称", "名稱"),
    row!("Description", "说明", "說明"),
    row!(
        "Storage mode (copy or reference)",
        "存储模式（副本或引用）",
        "儲存模式（副本或參照）",
    ),
    row!("Command template", "命令模板", "命令範本"),
    row!("Prompt runner", "提示词运行器", "提示詞執行器"),
    row!("Package dependencies", "软件包依赖项", "套件相依性"),
    row!("Python constraint", "Python 版本约束", "Python 版本限制"),
    row!("Save", "保存", "儲存"),
    row!("Save as preset", "另存为预设", "另存為預設"),
    row!("Extra arguments", "额外参数", "額外引數"),
    row!(
        "Dry run (true or false)",
        "试运行（true 或 false）",
        "試執行（true 或 false）",
    ),
    row!("Working directory", "工作目录", "工作目錄"),
    row!("Interpreter", "解释器", "直譯器"),
    row!("Required commands", "所需命令", "必要命令"),
    row!(
        "Prompt interpolation (true or false)",
        "提示词插值（true 或 false）",
        "提示詞插值（true 或 false）",
    ),
    row!(
        "Resync managed source parameters (true or false)",
        "重新同步受管源参数（true 或 false）",
        "重新同步受管來源參數（true 或 false）",
    ),
    row!("Manage source parameters", "管理源参数", "管理來源參數"),
    row!(
        "Stop managing source parameters",
        "停止管理源参数",
        "停止管理來源參數",
    ),
    row!(
        "Normalize shell parameters",
        "规范化 shell 参数",
        "正規化 shell 參數",
    ),
    row!("Add parameters", "添加参数", "新增參數"),
    row!("Remove parameters", "删除参数", "移除參數"),
    row!("Parameter {} name", "参数 {} 名称", "參數 {} 名稱"),
    row!("{} source binding", "{} 源绑定", "{} 來源繫結"),
    row!("{} delivery", "{} 传递方式", "{} 傳遞方式"),
    row!("{} type", "{} 类型", "{} 類型"),
    row!("{} default", "{} 默认值", "{} 預設值"),
    row!("{} choices", "{} 可选值", "{} 可選值"),
    row!("{} is required", "{} 为必填项", "{} 為必填欄位"),
    row!("{} takes multiple values", "{} 接受多个值", "{} 接受多個值"),
    row!("{} repeats its flag", "{} 重复其标志", "{} 重複其旗標"),
    row!("{} prompt", "{} 提示", "{} 提示"),
    row!("{} help", "{} 帮助", "{} 說明"),
    row!("{} is secret", "{} 为敏感值", "{} 為機密值"),
    row!(
        "{} secret environment source",
        "{} 敏感值环境变量来源",
        "{} 機密值環境變數來源",
    ),
    row!(
        "{} environment target",
        "{} 环境变量目标",
        "{} 環境變數目標",
    ),
    row!("{} flag", "{} 标志", "{} 旗標"),
    row!("{} flag action", "{} 标志操作", "{} 旗標動作"),
    row!("Missing targets", "缺失目标", "遺失目標"),
    row!("Data directory", "数据目录", "資料目錄"),
    row!("Runner name", "运行器名称", "執行器名稱"),
    row!("Arguments", "参数", "引數"),
    row!("Preset choices: {}", "预设选项：{}", "預設選項：{}"),
    row!(
        "Prompt runner choices: {}",
        "提示词运行器选项：{}",
        "提示詞執行器選項：{}",
    ),
    row!(
        "Remove (true or false)",
        "删除（true 或 false）",
        "移除（true 或 false）",
    ),
    row!("Preset name", "预设名称", "預設名稱"),
    row!(
        "Action (save or delete)",
        "操作（保存或删除）",
        "動作（儲存或刪除）",
    ),
    row!("Apply", "应用", "套用"),
    row!("ok", "正常", "正常"),
    row!("error", "错误", "錯誤"),
    row!(
        "Add a hand-declared parameter",
        "添加手动声明的参数",
        "新增手動宣告的參數",
    ),
    row!(
        "Add one direct argv prompt runner",
        "添加直接 argv 提示词运行器",
        "新增直接 argv 提示詞執行器",
    ),
    row!(
        "Add one package dependency. Repeat for more than one value",
        "添加一个软件包依赖项。可重复指定多个值",
        "新增一個套件相依性。可重複指定多個值",
    ),
    row!(
        "Agent convention: claude, codex, or agents",
        "Agent 约定：claude、codex 或 agents",
        "Agent 慣例：claude、codex 或 agents",
    ),
    row!("Arguments after `--`", "`--` 后的参数", "`--` 後的引數"),
    row!(
        "Bypass parameter handling and pass only the argument tail",
        "跳过参数处理，仅传递末尾参数",
        "略過參數處理，只傳遞尾端引數",
    ),
    row!(
        "Clear all package dependencies",
        "清除所有软件包依赖项",
        "清除所有套件相依性",
    ),
    row!(
        "Clear required external commands",
        "清除所需的外部命令",
        "清除必要的外部命令",
    ),
    row!(
        "Clear the remembered argument tail before this run",
        "在本次运行前清除记住的末尾参数",
        "在本次執行前清除記住的尾端引數",
    ),
    row!("Configuration key", "配置键", "組態鍵"),
    row!("Confirm deletion", "确认删除", "確認刪除"),
    row!(
        "Confirm the destructive operation",
        "确认破坏性操作",
        "確認破壞性操作",
    ),
    row!(
        "Copy the exact public values from the most recent run",
        "复制最近一次运行的准确公开值",
        "複製最近一次執行的完整公開值",
    ),
    row!(
        "Delete one named preset",
        "删除一个命名预设",
        "刪除一個具名預設",
    ),
    row!(
        "Description shown in the library",
        "程序库中显示的说明",
        "程式庫中顯示的說明",
    ),
    row!(
        "Disable enhanced terminal presentation for this run",
        "为本次运行禁用增强终端显示",
        "停用本次執行的增強終端顯示",
    ),
    row!(
        "Disable prompt interpolation",
        "禁用提示词插值",
        "停用提示詞插值",
    ),
    row!(
        "Disable prompt placeholder insertion",
        "禁用提示词占位符插入",
        "停用提示詞預留位置插入",
    ),
    row!(
        "Display name. The source stem is the default",
        "显示名称。默认使用源文件主名",
        "顯示名稱。預設使用來源檔案主名",
    ),
    row!(
        "Do not open an interactive form",
        "不要打开交互式表单",
        "不要開啟互動式表單",
    ),
    row!(
        "Emit stable machine-readable output",
        "输出稳定的机器可读结果",
        "輸出穩定的機器可讀結果",
    ),
    row!(
        "Enable prompt interpolation",
        "启用提示词插值",
        "啟用提示詞插值",
    ),
    row!(
        "Entry slug or display name",
        "条目短名或显示名称",
        "項目短名或顯示名稱",
    ),
    row!(
        "Force executable kind inference",
        "强制推断为可执行文件类型",
        "強制推斷為執行檔類型",
    ),
    row!(
        "Include malformed rows when supported",
        "在支持时包括格式错误的行",
        "支援時包括格式錯誤的資料列",
    ),
    row!(
        "Install below this explicit directory",
        "安装到这个指定目录下",
        "安裝到這個指定目錄下",
    ),
    row!(
        "Install completion for the current shell",
        "为当前 shell 安装补全",
        "為目前 shell 安裝補全",
    ),
    row!(
        "Install the bundled Agent Skill",
        "安装随附的 Agent Skill",
        "安裝隨附的 Agent Skill",
    ),
    row!(
        "List configured prompt runners",
        "列出已配置的提示词运行器",
        "列出已設定的提示詞執行器",
    ),
    row!("List named presets", "列出命名预设", "列出具名預設"),
    row!(
        "Load one named preset",
        "加载一个命名预设",
        "載入一個具名預設",
    ),
    row!(
        "Manage one detected source parameter",
        "管理一个检测到的源参数",
        "管理一個偵測到的來源參數",
    ),
    row!(
        "Mark fields as optional",
        "将字段标记为可选",
        "將欄位標示為選填",
    ),
    row!(
        "Mark fields as required",
        "将字段标记为必填",
        "將欄位標示為必填",
    ),
    row!(
        "Mark fields as secret",
        "将字段标记为敏感值",
        "將欄位標示為機密值",
    ),
    row!(
        "Normalize one shell constant to an environment default",
        "将一个 shell 常量规范化为环境变量默认值",
        "將一個 shell 常數正規化為環境變數預設值",
    ),
    row!(
        "Open entry-kind registry key",
        "开放的条目类型注册键",
        "開放式項目類型登錄鍵",
    ),
    row!(
        "Override the skit data directory",
        "覆盖 skit 数据目录",
        "覆寫 skit 資料目錄",
    ),
    row!(
        "Pin a prompt runner",
        "固定提示词运行器",
        "固定提示詞執行器",
    ),
    row!(
        "Pin a prompt runner. An empty value clears the pin",
        "固定提示词运行器。空值会清除固定设置",
        "固定提示詞執行器。空值會清除固定設定",
    ),
    row!(
        "Pin an interpreter or JavaScript runtime",
        "固定解释器或 JavaScript 运行时",
        "固定直譯器或 JavaScript 執行環境",
    ),
    row!(
        "Print completion for the current shell",
        "显示当前 shell 的补全脚本",
        "顯示目前 shell 的補全指令稿",
    ),
    row!(
        "Print the masked launch command and do not start a child",
        "显示已遮蔽的启动命令，且不启动子进程",
        "顯示已遮蔽的啟動命令，且不啟動子程序",
    ),
    row!(
        "Program and arguments. One token must contain `{{prompt}}`",
        "程序与参数。一个参数必须包含 `{{prompt}}`",
        "程式與引數。一個引數必須包含 `{{prompt}}`",
    ),
    row!(
        "Rebuild the derived registry",
        "重建派生索引",
        "重建衍生索引",
    ),
    row!(
        "Reconcile managed definitions with the current source",
        "将受管定义与当前源文件同步",
        "將受管定義與目前來源同步",
    ),
    row!(
        "Reference the original instead of storing a copy",
        "引用原文件，而不存储副本",
        "參照原始檔，而不儲存副本",
    ),
    row!(
        "Refuse interactive questions",
        "拒绝交互式问题",
        "拒絕互動式問題",
    ),
    row!(
        "Refuse to ask for confirmation",
        "拒绝请求确认",
        "拒絕要求確認",
    ),
    row!(
        "Refuse to offer creation when the entry does not exist",
        "条目不存在时不要提议创建",
        "項目不存在時不要提議建立",
    ),
    row!("Refuse to prompt", "拒绝提示", "拒絕提示"),
    row!(
        "Register a command template instead of a file",
        "注册命令模板，而不是文件",
        "登錄命令範本，而不是檔案",
    ),
    row!(
        "Remove a declared parameter",
        "删除已声明的参数",
        "移除已宣告的參數",
    ),
    row!(
        "Remove one configured prompt runner",
        "删除一个已配置的提示词运行器",
        "移除一個已設定的提示詞執行器",
    ),
    row!(
        "Remove the secret marker from fields",
        "移除字段的敏感值标记",
        "移除欄位的機密值標記",
    ),
    row!("Replace a command template", "替换命令模板", "取代命令範本"),
    row!("Replace an existing name", "替换现有名称", "取代現有名稱"),
    row!(
        "Replace package dependencies. Repeat for more than one value",
        "替换软件包依赖项。可重复指定多个值",
        "取代套件相依性。可重複指定多個值",
    ),
    row!(
        "Replace required external commands. Repeat for more than one value",
        "替换所需外部命令。可重复指定多个值",
        "取代必要的外部命令。可重複指定多個值",
    ),
    row!(
        "Replace the Python version constraint",
        "替换 Python 版本约束",
        "取代 Python 版本限制",
    ),
    row!(
        "Replace the work-directory policy",
        "替换工作目录策略",
        "取代工作目錄原則",
    ),
    row!("Replacement description", "替换说明", "取代說明"),
    row!("Replacement display name", "替换显示名称", "取代顯示名稱"),
    row!("Replacement value", "替换值", "取代值"),
    row!("Save a named preset", "保存命名预设", "儲存具名預設"),
    row!(
        "Save accepted values as a named preset after the run",
        "运行后将接受的值保存为命名预设",
        "執行後將接受的值儲存為具名預設",
    ),
    row!(
        "Select a prompt runner for this run",
        "为本次运行选择提示词运行器",
        "為本次執行選擇提示詞執行器",
    ),
    row!(
        "Set a default as NAME=VALUE",
        "以 NAME=VALUE 设置默认值",
        "以 NAME=VALUE 設定預設值",
    ),
    row!(
        "Set a flag as NAME=--FLAG. An empty flag makes the field positional",
        "以 NAME=--FLAG 设置标志。空标志会使字段成为位置参数",
        "以 NAME=--FLAG 設定旗標。空旗標會使欄位成為位置引數",
    ),
    row!(
        "Set a form prompt as NAME=TEXT",
        "以 NAME=TEXT 设置表单提示",
        "以 NAME=TEXT 設定表單提示",
    ),
    row!(
        "Set a parameter type as NAME=TYPE",
        "以 NAME=TYPE 设置参数类型",
        "以 NAME=TYPE 設定參數類型",
    ),
    row!(
        "Set a secret environment source as NAME=ENVVAR",
        "以 NAME=ENVVAR 设置敏感值环境变量来源",
        "以 NAME=ENVVAR 設定機密值環境變數來源",
    ),
    row!(
        "Set choices as NAME=A,B,C",
        "以 NAME=A,B,C 设置选项",
        "以 NAME=A,B,C 設定選項",
    ),
    row!(
        "Set delivery as NAME=DELIVERY",
        "以 NAME=DELIVERY 设置传递方式",
        "以 NAME=DELIVERY 設定傳遞方式",
    ),
    row!(
        "Set help text as NAME=TEXT",
        "以 NAME=TEXT 设置帮助文本",
        "以 NAME=TEXT 設定說明文字",
    ),
    row!(
        "Set one field for this run",
        "设置本次运行的一个字段",
        "設定本次執行的一個欄位",
    ),
    row!(
        "Set the Python version constraint",
        "设置 Python 版本约束",
        "設定 Python 版本限制",
    ),
    row!(
        "Source file to register",
        "要注册的源文件",
        "要登錄的來源檔案",
    ),
    row!("Stable runner name", "稳定的运行器名称", "穩定的執行器名稱"),
    row!(
        "Stop managing one source parameter",
        "停止管理一个源参数",
        "停止管理一個來源參數",
    ),
    row!(
        "Treat the source as a prompt entry",
        "将源文件视为提示词条目",
        "將來源視為提示詞項目",
    ),
    row!(
        "Use the current project instead of the user directory",
        "使用当前项目，而不是用户目录",
        "使用目前專案，而不是使用者目錄",
    ),
    row!(
        "Write a new source in the configured editor, then add it",
        "在已配置的编辑器中编写新源文件，然后添加",
        "在已設定的編輯器中編寫新來源，然後新增",
    ),
    row!(
        "--edit needs an editor; use standard input as `skit add - --name NAME`",
        "--edit 需要一个编辑器；请使用标准输入，如 `skit add - --name NAME`",
        "--edit 需要一個編輯器；請使用標準輸入，如 `skit add - --name NAME`",
    ),
    row!(
        "--interpolate only applies to prompt entries",
        "--interpolate 仅适用于提示词条目",
        "--interpolate 僅適用於提示詞項目",
    ),
    row!(
        "--interpreter only applies to interpreted entries",
        "--interpreter 仅适用于解释型条目",
        "--interpreter 僅適用於直譯型項目",
    ),
    row!(
        "--normalize applies only to shell entries",
        "--normalize 仅适用于 shell 条目",
        "--normalize 僅適用於 shell 項目",
    ),
    row!(
        "--normalize must be a separate params operation",
        "--normalize 必须作为单独的 params 操作运行",
        "--normalize 必須作為單獨的 params 操作執行",
    ),
    row!(
        "--raw cannot be combined with --set, --preset, or --save-preset",
        "--raw 不能与 --set、--preset 或 --save-preset 一起使用",
        "--raw 不能與 --set、--preset 或 --save-preset 一起使用",
    ),
    row!(
        "--raw does not apply to {} entries because placeholders are part of the artifact",
        "--raw 不适用于 {} 条目，因为占位符是产物的一部分",
        "--raw 不適用於 {} 項目，因為預留位置是產出物的一部分",
    ),
    row!(
        "--runner only applies to prompt entries",
        "--runner 仅适用于提示词条目",
        "--runner 僅適用於提示詞項目",
    ),
    row!(
        "--set needs NAME=VALUE; got {}",
        "--set 需要 NAME=VALUE；收到 {}",
        "--set 需要 NAME=VALUE；收到 {}",
    ),
    row!(
        "--template only applies to command entries",
        "--template 仅适用于命令条目",
        "--template 僅適用於命令項目",
    ),
    row!(
        "JavaScript package installation failed with {}",
        "JavaScript 软件包安装失败，使用的是 {}",
        "JavaScript 套件安裝失敗，使用的是 {}",
    ),
    row!(
        "The environment variable {} isn't set (needed by {}).",
        "环境变量 {} 未设置（{} 需要它）。",
        "環境變數 {} 未設定（{} 需要它）。",
    ),
    row!(
        "a --cmd entry needs an explicit --name",
        "--cmd 条目需要明确的 --name",
        "--cmd 項目需要明確的 --name",
    ),
    row!(
        "a Python constraint applies only to Python entries",
        "Python 版本约束仅适用于 Python 条目",
        "Python 版本限制僅適用於 Python 項目",
    ),
    row!(
        "a Python constraint does not apply to {} entries",
        "Python 版本约束不适用于 {} 条目",
        "Python 版本限制不適用於 {} 項目",
    ),
    row!(
        "a command template cannot be empty",
        "命令模板不能为空",
        "命令範本不能為空",
    ),
    row!(
        "a configuration value needs a key",
        "配置值需要一个键",
        "組態值需要一個鍵",
    ),
    row!(
        "a prompt body is required; pipe it to `skit add - --prompt --name NAME`",
        "需要提示词正文；请通过管道传入 `skit add - --prompt --name NAME`",
        "需要提示詞內容；請透過管道傳入 `skit add - --prompt --name NAME`",
    ),
    row!(
        "a prompt runner command needs {{prompt}} exactly once after the program",
        "提示词运行器命令必须在程序之后正好包含一次 {{prompt}}",
        "提示詞執行器命令必須在程式之後正好包含一次 {{prompt}}",
    ),
    row!(
        "a prompt runner needs a name and command",
        "提示词运行器需要名称和命令",
        "提示詞執行器需要名稱與命令",
    ),
    row!(
        "add needs a source path or --cmd COMMAND",
        "add 需要源文件路径或 --cmd COMMAND",
        "add 需要來源路徑或 --cmd COMMAND",
    ),
    row!(
        "add needs a source path, standard input as `-`, --edit, --prompt, or --cmd",
        "add 需要源文件路径、作为 `-` 的标准输入、--edit、--prompt 或 --cmd",
        "add 需要來源路徑、作為 `-` 的標準輸入、--edit、--prompt 或 --cmd",
    ),
    row!(
        "choice parameter {} has no choices",
        "选项参数 {} 没有可用选项",
        "選項參數 {} 沒有可用選項",
    ),
    row!(
        "command entries do not take package dependencies",
        "命令条目不接受软件包依赖项",
        "命令項目不接受套件相依性",
    ),
    row!(
        "command template needs a value for {}",
        "命令模板需要 {} 的值",
        "命令範本需要 {} 的值",
    ),
    row!(
        "command template placeholder {} is inside shell quotes",
        "命令模板占位符 {} 位于 shell 引号内",
        "命令範本預留位置 {} 位於 shell 引號內",
    ),
    row!(
        "configuration at {} is not valid TOML: {}",
        "{} 处的配置不是有效的 TOML：{}",
        "{} 處的組態不是有效的 TOML：{}",
    ),
    row!(
        "configuration section is not a table: {}",
        "配置节不是表：{}",
        "組態區段不是表格：{}",
    ),
    row!(
        "configure an editor before you use --edit",
        "使用 --edit 之前请先配置编辑器",
        "使用 --edit 之前請先設定編輯器",
    ),
    row!(
        "configure an editor before you use edit",
        "使用 edit 之前请先配置编辑器",
        "使用 edit 之前請先設定編輯器",
    ),
    row!(
        "confirmation is required for {}; pass --yes",
        "{}需要确认；请传入 --yes",
        "{}需要確認；請傳入 --yes",
    ),
    row!(
        "confirmation is required; pass --yes to remove the entry",
        "需要确认；请传入 --yes 以删除该条目",
        "需要確認；請傳入 --yes 以移除該項目",
    ),
    row!(
        "copy entry has more than one possible stored payload",
        "副本条目有多个可能的存储内容",
        "副本項目有多個可能的儲存內容",
    ),
    row!(
        "copy entry has no stored payload",
        "副本条目没有存储内容",
        "副本項目沒有儲存內容",
    ),
    row!(
        "copy-mode payloads require a stored filename",
        "副本模式的内容需要存储文件名",
        "副本模式的內容需要儲存檔名",
    ),
    row!(
        "could not detect the shell; set SHELL before completion setup",
        "无法检测 shell；请在设置补全之前设置 SHELL",
        "無法偵測 shell；請在設定補全之前設定 SHELL",
    ),
    row!(
        "could not determine the home directory for completion setup",
        "无法确定用于补全设置的主目录",
        "無法確定用於補全設定的家目錄",
    ),
    row!(
        "could not determine the platform configuration directory; set SKIT_CONFIG_DIR",
        "无法确定平台配置目录；请设置 SKIT_CONFIG_DIR",
        "無法確定平台組態目錄；請設定 SKIT_CONFIG_DIR",
    ),
    row!(
        "could not determine the platform data directory; pass --data-dir or SKIT_DATA_DIR",
        "无法确定平台数据目录；请传入 --data-dir 或设置 SKIT_DATA_DIR",
        "無法確定平台資料目錄；請傳入 --data-dir 或設定 SKIT_DATA_DIR",
    ),
    row!(
        "could not determine the platform state directory; set SKIT_STATE_DIR",
        "无法确定平台状态目录；请设置 SKIT_STATE_DIR",
        "無法確定平台狀態目錄；請設定 SKIT_STATE_DIR",
    ),
    row!(
        "could not determine the platform {} directory; set the matching SKIT_*_DIR variable",
        "无法确定平台 {} 目录；请设置对应的 SKIT_*_DIR 变量",
        "無法確定平台 {} 目錄；請設定對應的 SKIT_*_DIR 變數",
    ),
    row!(
        "could not determine the user directory",
        "无法确定用户目录",
        "無法確定使用者目錄",
    ),
    row!(
        "could not download uv from {}: {}",
        "无法从 {} 下载 uv：{}",
        "無法從 {} 下載 uv：{}",
    ),
    row!(
        "could not encode JSON output: {}",
        "无法编码 JSON 输出：{}",
        "無法編碼 JSON 輸出：{}",
    ),
    row!(
        "could not encode configuration: {}",
        "无法编码配置：{}",
        "無法編碼組態：{}",
    ),
    row!(
        "could not encode metadata: {}",
        "无法编码元数据：{}",
        "無法編碼中繼資料：{}",
    ),
    row!(
        "could not encode state: {}",
        "无法编码状态数据：{}",
        "無法編碼狀態資料：{}",
    ),
    row!(
        "could not infer the entry kind; pass --kind KIND",
        "无法推断条目类型；请传入 --kind KIND",
        "無法推斷項目類型；請傳入 --kind KIND",
    ),
    row!("could not normalize {}", "无法规范化 {}", "無法正規化 {}",),
    row!(
        "could not read {}: {}",
        "无法读取 {}：{}",
        "無法讀取 {}：{}",
    ),
    row!(
        "could not write output: {}",
        "无法写入输出：{}",
        "無法寫入輸出：{}",
    ),
    row!(
        "could not write staged source {}: {}",
        "无法写入暂存源文件 {}：{}",
        "無法寫入暫存來源 {}：{}",
    ),
    row!(
        "could not {} JavaScript dependencies at {}: {}",
        "无法{} {} 处的 JavaScript 依赖项：{}",
        "無法{} {} 處的 JavaScript 相依套件：{}",
    ),
    row!(
        "could not {} child process: {}",
        "无法{}子进程：{}",
        "無法{}子程序：{}",
    ),
    row!(
        "could not {} configuration at {}: {}",
        "无法{} {} 处的配置：{}",
        "無法{} {} 處的組態：{}",
    ),
    row!(
        "could not {} private uv at {}: {}",
        "无法{} {} 处的专用 uv：{}",
        "無法{} {} 處的專用 uv：{}",
    ),
    row!(
        "could not {} state at {}: {}",
        "无法{} {} 处的状态数据：{}",
        "無法{} {} 處的狀態資料：{}",
    ),
    row!("could not {} {}: {}", "无法{} {}：{}", "無法{} {}：{}",),
    row!(
        "custom working directory must be absolute: {}",
        "自定义工作目录必须是绝对路径：{}",
        "自訂工作目錄必須是絕對路徑：{}",
    ),
    row!(
        "duplicate parameter: {}",
        "重复的参数：{}",
        "重複的參數：{}",
    ),
    row!(
        "entry kind cannot be blank",
        "条目类型不能为空",
        "項目類型不能為空",
    ),
    row!(
        "entry name cannot be blank",
        "条目名称不能为空",
        "項目名稱不能為空",
    ),
    row!(
        "entry name {} is ambiguous; use one of these slugs: {}",
        "条目名称 {} 有歧义；请使用以下短名之一：{}",
        "項目名稱 {} 不明確；請使用以下短名之一：{}",
    ),
    row!("entry not found: {}", "找不到条目：{}", "找不到項目：{}",),
    row!(
        "entry slug suffix space is exhausted",
        "条目短名的后缀空间已用尽",
        "項目短名的後綴空間已用盡",
    ),
    row!(
        "entry {} already exists at slug {}",
        "条目 {} 已存在于短名 {}",
        "項目 {} 已存在於短名 {}",
    ),
    row!(
        "entry {} changed while this operation was underway",
        "条目 {} 在此操作进行期间已更改",
        "項目 {} 在此操作進行期間已變更",
    ),
    row!(
        "entry {} does not have an editable source",
        "条目 {} 没有可编辑的源文件",
        "項目 {} 沒有可編輯的來源",
    ),
    row!(
        "entry {} has corrupt metadata: {}",
        "条目 {} 的元数据已损坏：{}",
        "項目 {} 的中繼資料已損毀：{}",
    ),
    row!(
        "entry {} source changed while this edit was underway (expected {}, found {})",
        "条目 {} 的源文件在此编辑进行期间已更改（预期 {}，实际 {}）",
        "項目 {} 的來源在此編輯進行期間已變更（預期 {}，實際 {}）",
    ),
    row!(
        "extra arguments have invalid quoting",
        "额外参数的引号无效",
        "額外引數的引號無效",
    ),
    row!(
        "inline metadata is not valid: {}",
        "内联元数据无效：{}",
        "內嵌中繼資料無效：{}",
    ),
    row!(
        "invalid Boolean value {}; use true or false",
        "无效的布尔值 {}；请使用 true 或 false",
        "無效的布林值 {}；請使用 true 或 false",
    ),
    row!(
        "invalid JavaScript package specification: {}",
        "无效的 JavaScript 软件包描述：{}",
        "無效的 JavaScript 套件描述：{}",
    ),
    row!(
        "invalid PEP 440 version constraint {}: {}",
        "{} 不是有效的 PEP 440 版本约束：{}",
        "{} 不是有效的 PEP 440 版本限制：{}",
    ),
    row!(
        "invalid PEP 508 requirement {}: {}",
        "{} 不是有效的 PEP 508 依赖描述：{}",
        "{} 不是有效的 PEP 508 相依描述：{}",
    ),
    row!(
        "invalid configuration value for {}: {}",
        "{} 的配置值无效：{}",
        "{} 的組態值無效：{}",
    ),
    row!(
        "invalid entry id: {}",
        "无效的条目 ID：{}",
        "無效的項目 ID：{}",
    ),
    row!(
        "invalid entry mutation: {}",
        "无效的条目变更：{}",
        "無效的項目變更：{}",
    ),
    row!(
        "invalid entry slug: {}",
        "无效的条目短名：{}",
        "無效的項目短名：{}",
    ),
    row!(
        "launch target does not exist: {}",
        "启动目标不存在：{}",
        "啟動目標不存在：{}",
    ),
    row!(
        "launch target is not executable: {}",
        "启动目标不可执行：{}",
        "啟動目標不可執行：{}",
    ),
    row!(
        "lock path has no parent directory",
        "锁文件路径没有父目录",
        "鎖定檔路徑沒有父目錄",
    ),
    row!(
        "managed JavaScript dependencies require copy storage",
        "受管 JavaScript 依赖项需要副本存储",
        "受管 JavaScript 相依套件需要副本儲存",
    ),
    row!(
        "managed dependencies require copy storage",
        "受管依赖项需要副本存储",
        "受管相依套件需要副本儲存",
    ),
    row!(
        "metadata timestamp does not fit registry.toml: {}",
        "元数据时间戳不适合 registry.toml：{}",
        "中繼資料時間戳記不適合 registry.toml：{}",
    ),
    row!(
        "metadata timestamp predates the Unix epoch: {}",
        "元数据时间戳早于 Unix 纪元：{}",
        "中繼資料時間戳記早於 Unix 紀元：{}",
    ),
    row!(
        "no editable entry is named {}",
        "没有名为 {} 的可编辑条目",
        "沒有名為 {} 的可編輯項目",
    ),
    row!(
        "no mirror URLs are stored; set one mirror axis first",
        "没有存储镜像 URL；请先设置一个镜像轴",
        "沒有儲存鏡像 URL；請先設定一個鏡像軸",
    ),
    row!("operation cancelled", "操作已取消", "操作已取消",),
    row!(
        "package dependencies apply only to Python and JavaScript entries",
        "软件包依赖项仅适用于 Python 和 JavaScript 条目",
        "套件相依性僅適用於 Python 與 JavaScript 項目",
    ),
    row!(
        "parameter already exists: {}",
        "参数已存在：{}",
        "參數已存在：{}",
    ),
    row!(
        "parameter {} has a source binding that does not match its delivery",
        "参数 {} 的源绑定与其传递方式不匹配",
        "參數 {} 的來源繫結與其傳遞方式不相符",
    ),
    row!(
        "parameter {} has incompatible settings",
        "参数 {} 的设置互相冲突",
        "參數 {} 的設定互相衝突",
    ),
    row!(
        "parameter {} has invalid {} value {}",
        "参数 {} 的 {} 值 {} 无效",
        "參數 {} 的 {} 值 {} 無效",
    ),
    row!(
        "parameter {} is not managed in the stored source",
        "参数 {} 在存储的源文件中不受管",
        "參數 {} 在儲存的來源中不受管",
    ),
    row!(
        "parameter {} must be one of {}; got {}",
        "参数 {} 必须是 {} 之一；收到 {}",
        "參數 {} 必須是 {} 之一；收到 {}",
    ),
    row!(
        "parameter {} needs a name",
        "参数 {} 需要名称",
        "參數 {} 需要名稱",
    ),
    row!(
        "parameter {} no longer has a matching source binding",
        "参数 {} 不再有匹配的源绑定",
        "參數 {} 不再有相符的來源繫結",
    ),
    row!(
        "parameter {} received multiple values but is not a multi-value flag",
        "参数 {} 收到多个值，但它不是多值选项",
        "參數 {} 收到多個值，但它不是多值選項",
    ),
    row!(
        "preset {} does not exist",
        "预设 {} 不存在",
        "預設 {} 不存在",
    ),
    row!(
        "prompt body is required",
        "需要提示词正文",
        "需要提示詞內容",
    ),
    row!(
        "prompt runner already exists: {}",
        "提示词运行器已存在：{}",
        "提示詞執行器已存在：{}",
    ),
    row!(
        "prompt runner is required",
        "需要提示词运行器",
        "需要提示詞執行器",
    ),
    row!(
        "prompt runner {} is not configured",
        "提示词运行器 {} 未配置",
        "提示詞執行器 {} 未設定",
    ),
    row!(
        "prompt runner {} must contain exactly one {{prompt}} marker outside the program token",
        "提示词运行器 {} 必须在程序参数之外正好包含一个 {{prompt}} 标记",
        "提示詞執行器 {} 必須在程式引數之外正好包含一個 {{prompt}} 標記",
    ),
    row!(
        "prompt runners is not an array",
        "提示词运行器不是数组",
        "提示詞執行器不是陣列",
    ),
    row!(
        "reference entries are edited at their original path",
        "引用条目在其原始路径上编辑",
        "參照項目在其原始路徑上編輯",
    ),
    row!(
        "reference entries do not take managed dependencies",
        "引用条目不接受受管依赖项",
        "參照項目不接受受管相依套件",
    ),
    row!(
        "required command was not found: {}",
        "找不到所需命令：{}",
        "找不到必要命令：{}",
    ),
    row!(
        "required package manager was not found: {}",
        "找不到所需的软件包管理器：{}",
        "找不到必要的套件管理程式：{}",
    ),
    row!(
        "required program was not found: {}",
        "找不到所需程序：{}",
        "找不到必要程式：{}",
    ),
    row!(
        "run source, schema, launch, runner, and interpolation changes as separate params operations",
        "请将源、结构、启动、运行器和插值更改作为单独的 params 操作运行",
        "請將來源、結構、啟動、執行器與插值變更作為單獨的 params 操作執行",
    ),
    row!(
        "runner remove needs a name or --row INDEX",
        "runner remove 需要名称或 --row INDEX",
        "runner remove 需要名稱或 --row INDEX",
    ),
    row!(
        "runtime {} cannot manage JavaScript dependencies",
        "运行时 {} 无法管理 JavaScript 依赖项",
        "執行環境 {} 無法管理 JavaScript 相依套件",
    ),
    row!(
        "select an agent convention or use --to; more than one agent directory exists",
        "请选择一个 agent 约定或使用 --to；存在多个 agent 目录",
        "請選擇一個 agent 慣例或使用 --to；存在多個 agent 目錄",
    ),
    row!(
        "select an agent convention or use --to; no agent directory exists",
        "请选择一个 agent 约定或使用 --to；不存在 agent 目录",
        "請選擇一個 agent 慣例或使用 --to；不存在 agent 目錄",
    ),
    row!(
        "select an entry first",
        "请先选择一个条目",
        "請先選擇一個項目",
    ),
    row!(
        "source is not valid {} syntax",
        "源文件不是有效的 {} 语法",
        "來源不是有效的 {} 語法",
    ),
    row!(
        "source management applies only to a stored copy",
        "来源管理仅适用于存储的副本",
        "來源管理僅適用於儲存的副本",
    ),
    row!(
        "source operation is not supported for entry kind {}",
        "条目类型 {} 不支持来源操作",
        "項目類型 {} 不支援來源操作",
    ),
    row!(
        "standard input cannot be a referenced entry",
        "标准输入不能用作引用条目",
        "標準輸入不能用作參照項目",
    ),
    row!(
        "stored filename must be one safe path component",
        "存储文件名必须是一个安全的路径部分",
        "儲存檔名必須是一個安全的路徑部分",
    ),
    row!(
        "temporary path has no parent directory",
        "临时路径没有父目录",
        "暫存路徑沒有父目錄",
    ),
    row!(
        "terminal I/O failed: {}",
        "终端输入输出失败：{}",
        "終端輸入輸出失敗：{}",
    ),
    row!(
        "the Python stored copy is not readable UTF-8",
        "存储的 Python 副本不是可读的 UTF-8",
        "儲存的 Python 副本不是可讀的 UTF-8",
    ),
    row!(
        "the Python stored copy is not valid UTF-8",
        "存储的 Python 副本不是有效的 UTF-8",
        "儲存的 Python 副本不是有效的 UTF-8",
    ),
    row!(
        "the comment metadata block is not valid TOML: {}",
        "注释元数据块不是有效的 TOML：{}",
        "註解中繼資料區塊不是有效的 TOML：{}",
    ),
    row!(
        "the downloaded uv archive failed checksum verification",
        "下载的 uv 压缩包未通过校验和验证",
        "下載的 uv 封存檔未通過總和檢查碼驗證",
    ),
    row!(
        "the draft is empty and was kept at {}",
        "草稿为空，已保留在 {}",
        "草稿為空，已保留在 {}",
    ),
    row!(
        "the editor command has invalid quoting",
        "编辑器命令的引号无效",
        "編輯器命令的引號無效",
    ),
    row!(
        "the editor command is empty",
        "编辑器命令为空",
        "編輯器命令為空",
    ),
    row!(
        "the editor exited with status {}",
        "编辑器以状态 {} 退出",
        "編輯器以狀態 {} 結束",
    ),
    row!(
        "the entry does not use a pinnable interpreter",
        "该条目不使用可固定的解释器",
        "該項目不使用可固定的直譯器",
    ),
    row!(
        "the inline metadata block is not valid TOML",
        "内联元数据块不是有效的 TOML",
        "內嵌中繼資料區塊不是有效的 TOML",
    ),
    row!(
        "the rendered prompt contains a NUL character",
        "渲染后的提示词包含 NUL 字符",
        "算繪後的提示詞包含 NUL 字元",
    ),
    row!(
        "the rendered prompt makes the command line {} {}; the limit is {} {}",
        "渲染后的提示词使命令行达到 {} {}；上限是 {} {}",
        "算繪後的提示詞使命令列達到 {} {}；上限是 {} {}",
    ),
    row!(
        "the runner arguments have invalid quoting",
        "运行器参数的引号无效",
        "執行器引數的引號無效",
    ),
    row!(
        "the stored source is not valid UTF-8",
        "存储的源文件不是有效的 UTF-8",
        "儲存的來源不是有效的 UTF-8",
    ),
    row!(
        "the uv archive is invalid: {}",
        "uv 压缩包无效：{}",
        "uv 封存檔無效：{}",
    ),
    row!(
        "the work directory must be origin, store, invoke, or an absolute path",
        "工作目录必须是 origin、store、invoke 或绝对路径",
        "工作目錄必須是 origin、store、invoke 或絕對路徑",
    ),
    row!("tool is not a table", "tool 不是表", "tool 不是表格",),
    row!(
        "tool.skit is not a table",
        "tool.skit 不是表",
        "tool.skit 不是表格",
    ),
    row!(
        "unknown agent convention: {}",
        "未知的 agent 约定：{}",
        "未知的 agent 慣例：{}",
    ),
    row!(
        "unknown configuration key: {}",
        "未知的配置键：{}",
        "未知的組態鍵：{}",
    ),
    row!(
        "unknown entry kind: {}",
        "未知的条目类型：{}",
        "未知的項目類型：{}",
    ),
    row!(
        "unknown parameter binding: {}",
        "未知的参数源绑定：{}",
        "未知的參數來源繫結：{}",
    ),
    row!(
        "unknown parameter delivery: {}",
        "未知的参数传递方式：{}",
        "未知的參數傳遞方式：{}",
    ),
    row!(
        "unknown parameter in --set: {}",
        "--set 中的参数未知：{}",
        "--set 中的參數未知：{}",
    ),
    row!(
        "unknown parameter type: {}",
        "未知的参数类型：{}",
        "未知的參數類型：{}",
    ),
    row!("unknown parameter: {}", "未知的参数：{}", "未知的參數：{}",),
    row!("unknown preset: {}", "未知的预设：{}", "未知的預設：{}",),
    row!(
        "unknown prompt runner: {}",
        "未知的提示词运行器：{}",
        "未知的提示詞執行器：{}",
    ),
    row!(
        "unknown source parameter: {}",
        "未知的源参数：{}",
        "未知的來源參數：{}",
    ),
    row!(
        "unsupported platform: {}",
        "不支持的平台：{}",
        "不支援的平台：{}",
    ),
    row!(
        "use --dep or --clear, not both",
        "请使用 --dep 或 --clear，不能同时使用",
        "請使用 --dep 或 --clear，不能同時使用",
    ),
    row!(
        "use --need or --clear-needs, not both",
        "请使用 --need 或 --clear-needs，不能同时使用",
        "請使用 --need 或 --clear-needs，不能同時使用",
    ),
    row!(
        "use source:unmanage to remove the source binding for {}",
        "请使用 source:unmanage 删除 {} 的源绑定",
        "請使用 source:unmanage 移除 {} 的來源繫結",
    ),
    row!(
        "working directory does not exist: {}",
        "工作目录不存在：{}",
        "工作目錄不存在：{}",
    ),
    row!(
        "write path has no parent directory",
        "写入路径没有父目录",
        "寫入路徑沒有父目錄",
    ),
    row!(
        "{} does not take package dependencies; only --need applies",
        "{} 不接受软件包依赖项；只有 --need 适用",
        "{} 不接受套件相依性；只有 --need 適用",
    ),
    row!(
        "{} entries do not take package dependencies",
        "{} 条目不接受软件包依赖项",
        "{} 項目不接受套件相依性",
    ),
    row!(
        "{} is not a valid {} default",
        "{} 不是有效的 {} 默认值",
        "{} 不是有效的 {} 預設值",
    ),
    row!(
        "{} is not valid UTF-8",
        "{} 不是有效的 UTF-8",
        "{} 不是有效的 UTF-8",
    ),
    row!("{} is required.", "{} 为必填项。", "{} 為必填欄位。",),
    row!(
        "{} manages its parameter schema in the stored source",
        "{} 在存储的源文件中管理其参数结构",
        "{} 在儲存的來源中管理其參數結構",
    ),
    row!(
        "{} needs NAME=VALUE",
        "{}需要 NAME=VALUE",
        "{}需要 NAME=VALUE",
    ),
    row!(
        "{} reads from the environment variable {}, but it isn't set.",
        "{} 从环境变量 {} 读取，但该变量未设置。",
        "{} 從環境變數 {} 讀取，但該變數未設定。",
    ),
];

/// Return the full catalog for verification and frontend tooling.
#[must_use]
pub const fn catalog() -> &'static [Translation] {
    CATALOG
}

/// One user-visible message with a stable template and its values.
///
/// The template is the message identity. It must be a catalog row. The values are user data,
/// such as a slug, a path, or a parser detail. skit inserts the values after it translates the
/// template, so a value always stays exactly as the user wrote it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Message {
    template: &'static str,
    values: Vec<Value>,
}

/// One value in a message.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Value {
    /// User data or a third-party detail. It stays exactly as it is.
    Text(String),
    /// Another message. It uses the same locale as its parent.
    Nested(Message),
}

impl Message {
    /// Start one message from its stable English template.
    #[must_use]
    pub const fn new(template: &'static str) -> Self {
        Self {
            template,
            values: Vec::new(),
        }
    }

    /// Start one nested message from a catalog term selected by typed code.
    ///
    /// Use this only for skit-owned closed values such as file-operation names.
    #[must_use]
    pub fn term(template: &'static str) -> Self {
        assert!(
            CATALOG.iter().any(|row| row.english == template),
            "message term is missing from the catalog: {template}"
        );
        Self {
            template,
            values: Vec::new(),
        }
    }

    /// Add one value for the next `{}` hole.
    #[must_use]
    pub fn with(mut self, value: impl Display) -> Self {
        self.values.push(Value::Text(value.to_string()));
        self
    }

    /// Add one value in quotation marks for the next `{}` hole.
    ///
    /// Use this for a name or a value that can be empty or can hold spaces.
    #[must_use]
    pub fn quoted(self, value: impl Display) -> Self {
        let quoted = format!("{value}");
        self.with(format!("{quoted:?}"))
    }

    /// Add one message for the next `{}` hole.
    ///
    /// Use this when one typed error contains another.
    #[must_use]
    pub fn nested(mut self, value: Self) -> Self {
        self.values.push(Value::Nested(value));
        self
    }

    /// Return the stable English template.
    #[must_use]
    pub const fn template(&self) -> &'static str {
        self.template
    }

    /// Return the message in one locale.
    #[must_use]
    pub fn localize(&self, locale: Locale) -> String {
        let values = self
            .values
            .iter()
            .map(|value| match value {
                Value::Text(text) => text.clone(),
                Value::Nested(message) => message.localize(locale),
            })
            .collect::<Vec<_>>();
        let values = values
            .iter()
            .map(|value| value as &dyn Display)
            .collect::<Vec<_>>();
        format_text(locale, self.template, &values)
    }
}

impl Display for Message {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.localize(Locale::En))
    }
}

/// A type that presents one user-visible message in every supported locale.
///
/// Implement this for each typed error that can reach a user. The `message` template makes
/// catalog completeness testable, because a new variant needs a new template.
pub trait Localize {
    /// Return the message for this value.
    fn message(&self) -> Message;
}

/// Detect a supported locale from a language or POSIX locale spelling.
#[must_use]
pub fn detect_locale(value: Option<&str>) -> Locale {
    let normalized = value
        .unwrap_or_default()
        .trim()
        .replace('_', "-")
        .to_ascii_lowercase();
    // Hong Kong and Macau use Traditional Chinese; Singapore uses Simplified.
    if normalized.starts_with("zh-tw")
        || normalized.starts_with("zh-hk")
        || normalized.starts_with("zh-mo")
        || normalized.starts_with("zh-hant")
    {
        Locale::ZhTw
    } else if normalized.starts_with("zh-cn")
        || normalized.starts_with("zh-sg")
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
    CATALOG
        .iter()
        .find(|row| row.english == english)
        .map_or(english, |row| localized(locale, row))
}

/// Translate one template and insert values without translating user data.
#[must_use]
pub fn format_text(locale: Locale, english: &str, values: &[&dyn Display]) -> String {
    let template = text(locale, english);
    let mut parts = template.split("{}");
    let mut output = parts.next().unwrap_or_default().to_owned();
    for value in values {
        if let Some(part) = parts.next() {
            let _ = write!(output, "{value}");
            output.push_str(part);
        } else {
            break;
        }
    }
    for part in parts {
        output.push_str("{}");
        output.push_str(part);
    }
    output
}

/// Translate every known fragment in rendered framework text.
///
/// Use this only for text that a framework composes, such as a Clap usage report. A fragment
/// changes only at word boundaries, so a catalog row never replaces part of a longer word.
/// Typed skit messages use [`Message`], which keeps user values out of the translation.
#[must_use]
pub fn render(locale: Locale, english: &str) -> String {
    let mut rows = CATALOG
        .iter()
        .filter(|row| row.composable)
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.english.len()));
    rows.into_iter().fold(english.to_owned(), |output, row| {
        replace_words(&output, row.english, localized(locale, row))
    })
}

/// Replace each standalone occurrence of `needle`.
///
/// An occurrence is standalone when no word character touches it on a side that starts or ends
/// with a word character. This keeps `on` out of `version` and `not` out of `cannot`.
fn replace_words(haystack: &str, needle: &str, replacement: &str) -> String {
    let mut output = String::with_capacity(haystack.len());
    let mut rest = haystack;
    while let Some(index) = rest.find(needle) {
        let (before, tail) = (&rest[..index], &rest[index + needle.len()..]);
        if joins(before.chars().next_back(), needle.chars().next())
            || joins(needle.chars().next_back(), tail.chars().next())
        {
            let character = rest[index..]
                .chars()
                .next()
                .expect("matched catalog text is not empty");
            let step = index + character.len_utf8();
            output.push_str(&rest[..step]);
            rest = &rest[step..];
            continue;
        }
        output.push_str(before);
        output.push_str(replacement);
        rest = tail;
    }
    output.push_str(rest);
    output
}

fn joins(previous: Option<char>, next: Option<char>) -> bool {
    match (previous, next) {
        (Some(previous), Some(next)) => word_character(previous) && word_character(next),
        _ => false,
    }
}

fn word_character(value: char) -> bool {
    value.is_alphanumeric() || value == '_'
}

const fn localized(locale: Locale, row: &Translation) -> &'static str {
    match locale {
        Locale::En => row.english,
        Locale::ZhCn => row.zh_cn,
        Locale::ZhTw => row.zh_tw,
    }
}
