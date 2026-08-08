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
    row("Added: {} ({})", "已添加：{} ({})", "已新增：{} ({})"),
    row(
        "Description updated: {} ({})",
        "说明已更新：{} ({})",
        "說明已更新：{} ({})",
    ),
    row(
        "Renamed: {} ({})",
        "已重命名：{} ({})",
        "已重新命名：{} ({})",
    ),
    row("Removed: {}", "已删除：{}", "已移除：{}"),
    row("Edited: {} ({})", "已编辑：{} ({})", "已編輯：{} ({})"),
    row("Set: {}={}", "已设置：{}={}", "已設定：{}={}"),
    row("Added runner: {}", "已添加运行器：{}", "已新增執行器：{}"),
    row("Removed runner: {}", "已删除运行器：{}", "已移除執行器：{}"),
    row("Saved preset: {}", "已保存预设：{}", "已儲存預設：{}"),
    row("Deleted preset: {}", "已删除预设：{}", "已刪除預設：{}"),
    row(
        "Installed completion: {}",
        "已安装补全脚本：{}",
        "已安裝補全指令稿：{}",
    ),
    row(
        "Your draft was kept at {}",
        "草稿已保留在 {}",
        "草稿已保留在 {}",
    ),
    row("Dependencies: {}", "依赖项：{}", "相依套件：{}"),
    row(
        "Python constraint: {}",
        "Python 版本约束：{}",
        "Python 版本限制：{}",
    ),
    row("Required commands: {}", "所需命令：{}", "必要命令：{}"),
    row(
        "First Python run: download private uv {}",
        "首次运行 Python：下载专用 uv {}",
        "首次執行 Python：下載專用 uv {}",
    ),
    row("OK uv: {}", "正常 uv：{}", "正常 uv：{}"),
    row("ERROR uv: not found", "错误 uv：未找到", "錯誤 uv：找不到"),
    row("Entries: {}", "条目：{}", "項目：{}"),
    row(
        "Library: {} ({} bytes)",
        "程序库：{}（{} 字节）",
        "程式庫：{}（{} 位元組）",
    ),
    row("State: {}", "状态数据：{}", "狀態資料：{}"),
    row("Config: {}", "配置：{}", "組態：{}"),
    row("Registry rebuilt: {}", "索引已重建：{}", "索引已重建：{}"),
    row(
        "WARN {}: the launch target is gone from disk",
        "警告 {}：启动目标已不在磁盘上",
        "警告 {}：啟動目標已不在磁碟上",
    ),
    row(
        "WARN {}: form definitions are out of sync; run: skit params {} --resync",
        "警告 {}：表单定义不同步；请运行：skit params {} --resync",
        "警告 {}：表單定義不同步；請執行：skit params {} --resync",
    ),
    row(
        "WARN {}: missing external commands: {}",
        "警告 {}：缺少外部命令：{}",
        "警告 {}：缺少外部命令：{}",
    ),
    row(
        "WARN {}: a run would refuse to start: {}",
        "警告 {}：运行将无法启动：{}",
        "警告 {}：執行將無法啟動：{}",
    ),
    row(
        "WARN malformed prompt runners: {}",
        "警告 提示词运行器格式错误：{}",
        "警告 提示詞執行器格式錯誤：{}",
    ),
    row("WARN {}", "警告 {}", "警告 {}"),
    row(
        "Installed Agent Skill: {}",
        "已安装 Agent Skill：{}",
        "已安裝 Agent Skill：{}",
    ),
    row(
        "Remove \"{}\"? [y/N]: ",
        "删除“{}”？[y/N]：",
        "移除「{}」？[y/N]：",
    ),
    row(
        "No editable entry is named \"{}\". Create a script now? [Y/n]: ",
        "没有名为“{}”的可编辑条目。立即创建脚本吗？[Y/n]：",
        "沒有名為「{}」的可編輯項目。立即建立程式嗎？[Y/n]：",
    ),
    row(
        "Remove runner \"{}\"? [y/N]: ",
        "删除运行器“{}”？[y/N]：",
        "移除執行器「{}」？[y/N]：",
    ),
    row(
        "Delete preset \"{}\"? [y/N]: ",
        "删除预设“{}”？[y/N]：",
        "刪除預設「{}」？[y/N]：",
    ),
    row("Removed:", "已删除：", "已移除："),
    row("Renamed:", "已重命名：", "已重新命名："),
    row("Edited:", "已编辑：", "已編輯："),
    row("warning:", "警告：", "警告："),
    row("warning: {}", "警告：{}", "警告：{}"),
    row("Slug", "短名", "短名"),
    row("Kind", "类型", "類型"),
    row("Kind: {}", "类型：{}", "類型：{}"),
    row("Storage mode", "存储模式", "儲存模式"),
    row("Storage mode: {}", "存储模式：{}", "儲存模式：{}"),
    row("Run", "运行", "執行"),
    row("Run {}", "运行 {}", "執行 {}"),
    row("Add", "添加", "新增"),
    row("Edit", "编辑", "編輯"),
    row("Settings", "设置", "設定"),
    row("Settings for {}", "{} 的设置", "{} 的設定"),
    row("Presets", "预设", "預設"),
    row("Presets for {}: {}", "{} 的预设：{}", "{} 的預設：{}"),
    row("Rename", "重命名", "重新命名"),
    row("Rename {}", "重命名 {}", "重新命名 {}"),
    row("Remove", "删除", "移除"),
    row("Preferences", "偏好设置", "偏好設定"),
    row("Language", "语言", "語言"),
    row("Editor command", "编辑器命令", "編輯器命令"),
    row("Form style", "表单样式", "表單樣式"),
    row("After run", "运行后", "執行後"),
    row("Bash path", "Bash 路径", "Bash 路徑"),
    row(
        "JavaScript runtime",
        "JavaScript 运行时",
        "JavaScript 執行環境",
    ),
    row("Mirror", "镜像", "鏡像"),
    row("PyPI mirror", "PyPI 镜像", "PyPI 鏡像"),
    row("GitHub mirror", "GitHub 镜像", "GitHub 鏡像"),
    row("npm mirror", "npm 镜像", "npm 鏡像"),
    row("Health", "健康状态", "健康狀態"),
    row("Runners", "运行器", "執行器"),
    row("Prompt runners: {}", "提示词运行器：{}", "提示詞執行器：{}"),
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
    row("Parameter {} name", "参数 {} 名称", "參數 {} 名稱"),
    row("{} source binding", "{} 源绑定", "{} 來源繫結"),
    row("{} delivery", "{} 传递方式", "{} 傳遞方式"),
    row("{} type", "{} 类型", "{} 類型"),
    row("{} default", "{} 默认值", "{} 預設值"),
    row("{} choices", "{} 选项", "{} 選項"),
    row("{} is required", "{} 为必填项", "{} 為必填欄位"),
    row("{} takes multiple values", "{} 接受多个值", "{} 接受多個值"),
    row("{} repeats its flag", "{} 重复其标志", "{} 重複其旗標"),
    row("{} prompt", "{} 提示", "{} 提示"),
    row("{} help", "{} 帮助", "{} 說明"),
    row("{} is secret", "{} 为敏感值", "{} 為機密值"),
    row(
        "{} secret environment source",
        "{} 敏感值环境变量来源",
        "{} 機密值環境變數來源",
    ),
    row(
        "{} environment target",
        "{} 环境变量目标",
        "{} 環境變數目標",
    ),
    row("{} flag", "{} 标志", "{} 旗標"),
    row("{} flag action", "{} 标志操作", "{} 旗標動作"),
    row("Missing targets", "缺失目标", "遺失目標"),
    row("Data directory", "数据目录", "資料目錄"),
    row("Runner name", "运行器名称", "執行器名稱"),
    row("Arguments", "参数", "引數"),
    row("Preset choices: {}", "预设选项：{}", "預設選項：{}"),
    row(
        "Prompt runner choices: {}",
        "提示词运行器选项：{}",
        "提示詞執行器選項：{}",
    ),
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
    row(
        "Add a hand-declared parameter",
        "添加手动声明的参数",
        "新增手動宣告的參數",
    ),
    row(
        "Add one direct argv prompt runner",
        "添加直接 argv 提示词运行器",
        "新增直接 argv 提示詞執行器",
    ),
    row(
        "Add one package dependency. Repeat for more than one value",
        "添加一个软件包依赖项。可重复指定多个值",
        "新增一個套件相依性。可重複指定多個值",
    ),
    row(
        "Agent convention: claude, codex, or agents",
        "Agent 约定：claude、codex 或 agents",
        "Agent 慣例：claude、codex 或 agents",
    ),
    row("Arguments after `--`", "`--` 后的参数", "`--` 後的引數"),
    row(
        "Bypass parameter handling and pass only the argument tail",
        "跳过参数处理，仅传递末尾参数",
        "略過參數處理，只傳遞尾端引數",
    ),
    row(
        "Clear all package dependencies",
        "清除所有软件包依赖项",
        "清除所有套件相依性",
    ),
    row(
        "Clear required external commands",
        "清除所需的外部命令",
        "清除必要的外部命令",
    ),
    row(
        "Clear the remembered argument tail before this run",
        "在本次运行前清除记住的末尾参数",
        "在本次執行前清除記住的尾端引數",
    ),
    row("Configuration key", "配置键", "組態鍵"),
    row("Confirm deletion", "确认删除", "確認刪除"),
    row(
        "Confirm the destructive operation",
        "确认破坏性操作",
        "確認破壞性操作",
    ),
    row(
        "Copy the exact public values from the most recent run",
        "复制最近一次运行的准确公开值",
        "複製最近一次執行的完整公開值",
    ),
    row(
        "Delete one named preset",
        "删除一个命名预设",
        "刪除一個具名預設",
    ),
    row(
        "Description shown in the library",
        "程序库中显示的说明",
        "程式庫中顯示的說明",
    ),
    row(
        "Disable enhanced terminal presentation for this run",
        "为本次运行禁用增强终端显示",
        "停用本次執行的增強終端顯示",
    ),
    row(
        "Disable prompt interpolation",
        "禁用提示词插值",
        "停用提示詞插值",
    ),
    row(
        "Disable prompt placeholder insertion",
        "禁用提示词占位符插入",
        "停用提示詞預留位置插入",
    ),
    row(
        "Display name. The source stem is the default",
        "显示名称。默认使用源文件主名",
        "顯示名稱。預設使用來源檔案主名",
    ),
    row(
        "Do not open an interactive form",
        "不要打开交互式表单",
        "不要開啟互動式表單",
    ),
    row(
        "Emit stable machine-readable output",
        "输出稳定的机器可读结果",
        "輸出穩定的機器可讀結果",
    ),
    row(
        "Enable prompt interpolation",
        "启用提示词插值",
        "啟用提示詞插值",
    ),
    row(
        "Entry slug or display name",
        "条目短名或显示名称",
        "項目短名或顯示名稱",
    ),
    row(
        "Force executable kind inference",
        "强制推断为可执行文件类型",
        "強制推斷為執行檔類型",
    ),
    row(
        "Include malformed rows when supported",
        "在支持时包括格式错误的行",
        "支援時包括格式錯誤的資料列",
    ),
    row(
        "Install below this explicit directory",
        "安装到这个指定目录下",
        "安裝到這個指定目錄下",
    ),
    row(
        "Install completion for the current shell",
        "为当前 shell 安装补全",
        "為目前 shell 安裝補全",
    ),
    row(
        "Install the bundled Agent Skill",
        "安装随附的 Agent Skill",
        "安裝隨附的 Agent Skill",
    ),
    row(
        "List configured prompt runners",
        "列出已配置的提示词运行器",
        "列出已設定的提示詞執行器",
    ),
    row("List named presets", "列出命名预设", "列出具名預設"),
    row(
        "Load one named preset",
        "加载一个命名预设",
        "載入一個具名預設",
    ),
    row(
        "Manage one detected source parameter",
        "管理一个检测到的源参数",
        "管理一個偵測到的來源參數",
    ),
    row(
        "Mark fields as optional",
        "将字段标记为可选",
        "將欄位標示為選填",
    ),
    row(
        "Mark fields as required",
        "将字段标记为必填",
        "將欄位標示為必填",
    ),
    row(
        "Mark fields as secret",
        "将字段标记为敏感值",
        "將欄位標示為機密值",
    ),
    row(
        "Normalize one shell constant to an environment default",
        "将一个 shell 常量规范化为环境变量默认值",
        "將一個 shell 常數正規化為環境變數預設值",
    ),
    row(
        "Open entry-kind registry key",
        "开放的条目类型注册键",
        "開放式項目類型登錄鍵",
    ),
    row(
        "Override the skit data directory",
        "覆盖 skit 数据目录",
        "覆寫 skit 資料目錄",
    ),
    row(
        "Pin a prompt runner",
        "固定提示词运行器",
        "固定提示詞執行器",
    ),
    row(
        "Pin a prompt runner. An empty value clears the pin",
        "固定提示词运行器。空值会清除固定设置",
        "固定提示詞執行器。空值會清除固定設定",
    ),
    row(
        "Pin an interpreter or JavaScript runtime",
        "固定解释器或 JavaScript 运行时",
        "固定直譯器或 JavaScript 執行環境",
    ),
    row(
        "Print completion for the current shell",
        "显示当前 shell 的补全脚本",
        "顯示目前 shell 的補全指令稿",
    ),
    row(
        "Print the masked launch command and do not start a child",
        "显示已遮蔽的启动命令，且不启动子进程",
        "顯示已遮蔽的啟動命令，且不啟動子程序",
    ),
    row(
        "Program and arguments. One token must contain `{{prompt}}`",
        "程序与参数。一个参数必须包含 `{{prompt}}`",
        "程式與引數。一個引數必須包含 `{{prompt}}`",
    ),
    row(
        "Rebuild the derived registry",
        "重建派生索引",
        "重建衍生索引",
    ),
    row(
        "Reconcile managed definitions with the current source",
        "将受管定义与当前源文件同步",
        "將受管定義與目前來源同步",
    ),
    row(
        "Reference the original instead of storing a copy",
        "引用原文件，而不存储副本",
        "參照原始檔，而不儲存副本",
    ),
    row(
        "Refuse interactive questions",
        "拒绝交互式问题",
        "拒絕互動式問題",
    ),
    row(
        "Refuse to ask for confirmation",
        "拒绝请求确认",
        "拒絕要求確認",
    ),
    row(
        "Refuse to offer creation when the entry does not exist",
        "条目不存在时不要提议创建",
        "項目不存在時不要提議建立",
    ),
    row("Refuse to prompt", "拒绝提示", "拒絕提示"),
    row(
        "Register a command template instead of a file",
        "注册命令模板，而不是文件",
        "登錄命令範本，而不是檔案",
    ),
    row(
        "Remove a declared parameter",
        "删除已声明的参数",
        "移除已宣告的參數",
    ),
    row(
        "Remove one configured prompt runner",
        "删除一个已配置的提示词运行器",
        "移除一個已設定的提示詞執行器",
    ),
    row(
        "Remove the secret marker from fields",
        "移除字段的敏感值标记",
        "移除欄位的機密值標記",
    ),
    row("Replace a command template", "替换命令模板", "取代命令範本"),
    row("Replace an existing name", "替换现有名称", "取代現有名稱"),
    row(
        "Replace package dependencies. Repeat for more than one value",
        "替换软件包依赖项。可重复指定多个值",
        "取代套件相依性。可重複指定多個值",
    ),
    row(
        "Replace required external commands. Repeat for more than one value",
        "替换所需外部命令。可重复指定多个值",
        "取代必要的外部命令。可重複指定多個值",
    ),
    row(
        "Replace the Python version constraint",
        "替换 Python 版本约束",
        "取代 Python 版本限制",
    ),
    row(
        "Replace the work-directory policy",
        "替换工作目录策略",
        "取代工作目錄原則",
    ),
    row("Replacement description", "替换说明", "取代說明"),
    row("Replacement display name", "替换显示名称", "取代顯示名稱"),
    row("Replacement value", "替换值", "取代值"),
    row("Save a named preset", "保存命名预设", "儲存具名預設"),
    row(
        "Save accepted values as a named preset after the run",
        "运行后将接受的值保存为命名预设",
        "執行後將接受的值儲存為具名預設",
    ),
    row(
        "Select a prompt runner for this run",
        "为本次运行选择提示词运行器",
        "為本次執行選擇提示詞執行器",
    ),
    row(
        "Set a default as NAME=VALUE",
        "以 NAME=VALUE 设置默认值",
        "以 NAME=VALUE 設定預設值",
    ),
    row(
        "Set a flag as NAME=--FLAG. An empty flag makes the field positional",
        "以 NAME=--FLAG 设置标志。空标志会使字段成为位置参数",
        "以 NAME=--FLAG 設定旗標。空旗標會使欄位成為位置引數",
    ),
    row(
        "Set a form prompt as NAME=TEXT",
        "以 NAME=TEXT 设置表单提示",
        "以 NAME=TEXT 設定表單提示",
    ),
    row(
        "Set a parameter type as NAME=TYPE",
        "以 NAME=TYPE 设置参数类型",
        "以 NAME=TYPE 設定參數類型",
    ),
    row(
        "Set a secret environment source as NAME=ENVVAR",
        "以 NAME=ENVVAR 设置敏感值环境变量来源",
        "以 NAME=ENVVAR 設定機密值環境變數來源",
    ),
    row(
        "Set choices as NAME=A,B,C",
        "以 NAME=A,B,C 设置选项",
        "以 NAME=A,B,C 設定選項",
    ),
    row(
        "Set delivery as NAME=DELIVERY",
        "以 NAME=DELIVERY 设置传递方式",
        "以 NAME=DELIVERY 設定傳遞方式",
    ),
    row(
        "Set help text as NAME=TEXT",
        "以 NAME=TEXT 设置帮助文本",
        "以 NAME=TEXT 設定說明文字",
    ),
    row(
        "Set one field for this run",
        "设置本次运行的一个字段",
        "設定本次執行的一個欄位",
    ),
    row(
        "Set the Python version constraint",
        "设置 Python 版本约束",
        "設定 Python 版本限制",
    ),
    row(
        "Source file to register",
        "要注册的源文件",
        "要登錄的來源檔案",
    ),
    row("Stable runner name", "稳定的运行器名称", "穩定的執行器名稱"),
    row(
        "Stop managing one source parameter",
        "停止管理一个源参数",
        "停止管理一個來源參數",
    ),
    row(
        "Treat the source as a prompt entry",
        "将源文件视为提示词条目",
        "將來源視為提示詞項目",
    ),
    row(
        "Use the current project instead of the user directory",
        "使用当前项目，而不是用户目录",
        "使用目前專案，而不是使用者目錄",
    ),
    row(
        "Write a new source in the configured editor, then add it",
        "在已配置的编辑器中编写新源文件，然后添加",
        "在已設定的編輯器中編寫新來源，然後新增",
    ),
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
