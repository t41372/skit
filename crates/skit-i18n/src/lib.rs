//! Localize skit presentation text without frontend dependencies.

#![forbid(unsafe_code)]

use std::{
    borrow::Cow,
    fmt::{Display, Write as _},
};

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
    /// Stretched English used to expose hard-coded or clipped user-visible text.
    Pseudo,
}

impl Locale {
    /// Return the canonical public tag for this locale.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
            Self::ZhTw => "zh-TW",
            Self::Pseudo => "x-pseudo",
        }
    }
}

/// Return locale tags that the Preferences picker can offer.
///
/// The pseudo-locale stays available as an explicit configuration value, but it is a test aid and
/// is not part of the normal picker.
#[must_use]
pub const fn available_locale_tags() -> &'static [&'static str] {
    &["en", "zh-CN", "zh-TW"]
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
        "列出工具库中的条目",
        "列出工具庫中的項目",
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
        "运行一个工具库条目",
        "執行一個工具庫項目",
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
        "Remove one malformed raw row by its zero-based index or `container`",
        "按从零开始的索引或 `container` 删除一个格式错误的原始行",
        "依從零開始的索引或 `container` 移除一個格式錯誤的原始列",
    ),
    row!("Show version", "显示版本", "顯示版本"),
    row!(
        "Read or update dependencies and required commands",
        "读取或更新依赖项和所需命令",
        "讀取或更新相依套件與必要命令",
    ),
    row!(
        "Check runtime and library health",
        "检查运行环境与工具库健康状态",
        "檢查執行環境與工具庫健康狀態",
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
        "打开 Ratatui 工具库浏览器",
        "開啟 Ratatui 工具庫瀏覽器",
    ),
    composable!(
        "Library: all entries",
        "工具库：所有条目",
        "工具庫：所有項目",
    ),
    composable!(
        "No matching entries. Press [q] Quit.",
        "没有匹配的条目。按 [q] 退出。",
        "沒有相符的項目。按 [q] 結束。",
    ),
    row!("No matching entries", "没有匹配的条目", "沒有相符的項目"),
    row!("(use this directory)", "(使用此目录)", "(使用此目錄)"),
    row!("valid", "有效", "有效"),
    row!(
        "could not read a parameter row: {}",
        "无法读取参数行:{}",
        "無法讀取參數列:{}",
    ),
    row!("Preset:", "参数组合：", "參數組合："),
    row!(
        "Network to PyPI / GitHub looks slow or blocked.",
        "检测到访问 PyPI / GitHub 缓慢或受阻。",
        "偵測到存取 PyPI / GitHub 緩慢或受阻。",
    ),
    row!(
        "Configure mirrors for faster installs (mainland China)?",
        "是否配置镜像以加速下载(中国大陆)?",
        "是否設定鏡像以加速下載(中國大陸)?",
    ),
    row!(
        "Choose one of: {}",
        "请选择其中之一:{}",
        "請選擇其中之一:{}"
    ),
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
    row!(
        "{}'s stored copy isn't valid UTF-8, so skit can't rewrite the script's own dependency block — and that block is what uv reads. Edit it in the script itself: skit edit {}",
        "{} 存储的副本不是合法的 UTF-8，因此 skit 无法改写脚本自己的依赖区块——而 uv 读取的正是该区块。请直接在脚本中修改：skit edit {}",
        "{} 儲存的副本不是合法的 UTF-8，因此 skit 無法改寫腳本自己的相依套件區塊——而 uv 讀取的正是該區塊。請直接在腳本中修改：skit edit {}",
    ),
    row!("rollback", "回滚", "復原"),
    row!(
        "rollback at {} failed after {}: {}",
        "在 {} 处回滚失败；原操作错误：{}；回滚错误：{}",
        "在 {} 處復原失敗；原始操作錯誤：{}；復原錯誤：{}",
    ),
    row!(
        "{} was removed from the library, but its files couldn't be fully deleted: {} — close any program using them, then delete the folder (or run `skit doctor --rebuild` to restore the entry and retry).",
        "{} 已从工具库移除，但无法完整删除其文件：{} — 请关闭正在使用这些文件的程序，然后删除该文件夹（或运行 `skit doctor --rebuild` 恢复条目后重试）。",
        "{} 已從工具庫移除，但無法完整刪除其檔案：{} — 請關閉正在使用這些檔案的程式，然後刪除該資料夾（或執行 `skit doctor --rebuild` 復原項目後重試）。",
    ),
    row!("backup", "备份", "備份"),
    row!(
        "{} is corrupt and could not be parsed. It has been backed up to {} before this change; recover any lost settings from that file.",
        "{} 已损坏而无法解析。更改前已备份至 {};请从该文件恢复任何丢失的设置。",
        "{} 已損毀而無法解析。變更前已備份至 {};請從該檔案復原任何遺失的設定。",
    ),
    row!(
        "{} is corrupt and could not be parsed, and it could not be backed up either; the settings it contained will be lost when this change is saved.",
        "{} 已损坏而无法解析,且无法备份;保存此更改后,其中的设置将会丢失。",
        "{} 已損毀而無法解析,且無法備份;儲存此變更後,其中的設定將會遺失。",
    ),
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
    row!(
        "The built-in amp runner is one-shot: amp -x runs this prompt once and does not open an interactive session.",
        "内置 amp 执行器为单次运行模式：amp -x 只运行此提示词一次，不会打开交互式会话。",
        "內建 amp 執行器為單次執行模式：amp -x 只執行此提示詞一次，不會開啟互動工作階段。",
    ),
    row!(
        "Secret-marked values are never saved by skit, but this prompt sends them to the selected agent as plaintext; the agent may log or sync them.",
        "标记为机密的值不会由 skit 保存，但此提示词会以明文将它们发送给所选 agent；agent 可能记录或同步这些值。",
        "標記為機密的值不會由 skit 儲存，但此提示詞會以明文將它們傳送給所選 agent；agent 可能記錄或同步這些值。",
    ),
    row!(
        "Warning: Pi would interpret the beginning of this prompt as a CLI option, file, or package command. skit prepended one newline and is continuing; the prompt delivered to Pi is one character longer than the rendered text.",
        "警告：Pi 会把这个提示词的开头解释为命令行选项、文件或包管理命令。skit 已在前面添加一个换行符并继续运行；传递给 Pi 的提示词比渲染后的文本多一个字符。",
        "警告：Pi 會把這個提示詞的開頭解讀為命令列選項、檔案或套件管理命令。skit 已在前面加上一個換行字元並繼續執行；傳給 Pi 的提示詞比渲染後的文字多一個字元。",
    ),
    row!("rename", "重命名", "重新命名"),
    row!("start", "启动", "啟動"),
    row!("test", "测试", "測試"),
    row!("all entries", "所有条目", "所有項目"),
    row!("Library", "工具库", "工具庫"),
    row!("Search", "搜索", "搜尋"),
    row!("Entries", "条目", "項目"),
    row!("Details", "详细信息", "詳細資料"),
    row!("Quit", "退出", "結束"),
    row!("Reload", "重新加载", "重新載入"),
    row!("Rerun", "重新运行", "重新執行"),
    row!("Entry settings", "条目设置", "項目設定"),
    row!("Edit source", "编辑源文件", "編輯來源"),
    row!("Add entry", "添加条目", "新增項目"),
    row!("Detail pane", "详情窗格", "詳細資料窗格"),
    row!(
        "The copy is kept by skit; your original file is never modified.",
        "副本由 skit 保管；你的原始文件永远不会被改动。",
        "副本由 skit 保管；你的原始檔永遠不會被改動。",
    ),
    row!(
        "Linked to the original: {}",
        "链接原文件:{}",
        "連結原檔：{}",
    ),
    row!(
        "(no description — add one in Entry settings)",
        "（没有说明——可在条目设置中添加）",
        "（沒有說明——可在條目設定中加入）",
    ),
    row!("Health check", "健康检查", "健康檢查"),
    row!(
        "Issues (Enter jumps to the entry):",
        "问题（按回车跳转到条目）：",
        "問題（按 Enter 跳至項目）：",
    ),
    row!("uv: {}", "uv：{}", "uv：{}"),
    row!("uv: not required", "uv：不需要", "uv：不需要"),
    row!(
        "uv: not found. Install it from https://docs.astral.sh/uv/getting-started/installation/",
        "uv：未找到。请从 https://docs.astral.sh/uv/getting-started/installation/ 安装。",
        "uv：找不到。請從 https://docs.astral.sh/uv/getting-started/installation/ 安裝。",
    ),
    row!(
        "{} entry registered",
        "已登记 {} 个条目",
        "已登記 {} 個條目"
    ),
    row!(
        "{} entries registered",
        "已登记 {} 个条目",
        "已登記 {} 個條目",
    ),
    row!(
        "Malformed agent (runner) rows in config: {} — fix them in Preferences → Manage agents",
        "配置中格式错误的代理（运行器）行：{}——请在“偏好设置 → 管理代理”中修复",
        "組態中格式錯誤的代理（執行器）資料列：{}——請在「偏好設定 → 管理代理」中修復",
    ),
    row!("Mirrors: off", "镜像：关闭", "鏡像：關閉"),
    row!("Mirrors: {}", "镜像：{}", "鏡像：{}"),
    row!(
        "Mirrors: off (saved: {})",
        "镜像：关闭（已保存：{}）",
        "鏡像：關閉（已儲存：{}）",
    ),
    row!(
        "Library: {} ({} · {})",
        "工具库：{}（{} 个条目 · {}）",
        "工具庫：{}（{} 個項目 · {}）",
    ),
    row!(
        "(shown in the Library — you can write one line)",
        "（显示在工具库中——可以自己写一句）",
        "（顯示在工具庫中——可以自己寫一句）",
    ),
    row!(
        "Index rebuilt: {} entry",
        "索引已重建：{} 个条目",
        "索引已重建：{} 個項目"
    ),
    row!(
        "Index rebuilt: {} entries",
        "索引已重建：{} 个条目",
        "索引已重建：{} 個項目",
    ),
    row!("Jump to entry", "跳转到条目", "跳至項目"),
    row!("Rebuild index", "重建索引", "重建索引"),
    row!(
        "New agent (runner)",
        "新建代理（运行器）",
        "新增代理（執行器）"
    ),
    row!(
        "A prompt needs a configured agent to run with.",
        "提示词需要一个已配置的 agent 才能运行。",
        "提示詞需要一個已設定的 agent 才能執行。",
    ),
    row!(
        "Edit agent (runner)",
        "编辑代理（运行器）",
        "編輯代理（執行器）"
    ),
    row!("Name, e.g. aider", "名称，例如 aider", "名稱，例如 aider"),
    row!(
        "Command, e.g. aider --message {{prompt}}",
        "命令，例如 aider --message {{prompt}}",
        "命令，例如 aider --message {{prompt}}",
    ),
    row!(
        "{{prompt}} marks where the prompt text goes. Each word becomes one argument — quotes group words, and no shell is involved.",
        "{{prompt}} 标记提示词文本的位置。每个词会成为一个参数；引号可组合多个词，且不会调用 shell。",
        "{{prompt}} 標記提示詞文字的位置。每個詞會成為一個引數；引號可組合多個詞，且不會呼叫 shell。",
    ),
    row!(
        "The agents prompt entries run with. Pick one to edit or remove it.",
        "提示词条目使用这些代理运行。请选择一个代理进行编辑或删除。",
        "提示詞項目使用這些代理執行。請選擇一個代理來編輯或移除。",
    ),
    row!(
        "No agents configured yet.",
        "尚未配置代理。",
        "尚未設定代理。",
    ),
    row!(
        "Remove the malformed prompt runner container?",
        "删除格式错误的提示词运行器容器？",
        "移除格式錯誤的提示詞執行器容器？",
    ),
    row!(
        "Remove malformed runner row \"{}\"?",
        "删除格式错误的运行器行“{}”？",
        "移除格式錯誤的執行器資料列「{}」？",
    ),
    row!(
        "Remove the agent \"{}\"?",
        "删除代理“{}”？",
        "移除代理「{}」？",
    ),
    row!(
        "{} prompt pins this runner and will need another runner before it can run again.",
        "有 {} 个提示词固定使用此运行器，需要改用其他运行器后才能再次运行。",
        "有 {} 個提示詞固定使用此執行器，需要改用其他執行器後才能再次執行。",
    ),
    row!("Keep", "保留", "保留"),
    row!(
        "the launch target is gone from disk",
        "启动目标已不在磁盘上",
        "啟動目標已不在磁碟上",
    ),
    row!(
        "form definitions are out of sync (open Entry settings → Resync)",
        "表单定义不同步（请打开“条目设置 → 重新同步”）",
        "表單定義不同步（請開啟「項目設定 → 重新同步」）",
    ),
    row!(
        "missing external command(s): {}",
        "缺少外部命令：{}",
        "缺少外部命令：{}",
    ),
    row!(
        "a run would refuse to start — {}",
        "运行将被拒绝启动——{}",
        "執行將被拒絕啟動——{}",
    ),
    row!(
        "the prompt value isn't a table; repair it before runner management",
        "prompt 值不是表；请先修复再管理运行器",
        "prompt 值不是表格；請先修復再管理執行器",
    ),
    row!(
        "the prompt.runners value isn't a list; repair it before runner management",
        "prompt.runners 值不是列表；请先修复再管理运行器",
        "prompt.runners 值不是清單；請先修復再管理執行器",
    ),
    row!(
        "Type the agent's command, e.g. mycli run {{prompt}}",
        "请输入 agent 的命令,例如 mycli run {{prompt}}",
        "請輸入 agent 的命令,例如 mycli run {{prompt}}",
    ),
    row!(
        "A runner needs a command — e.g. skit runner add mycli mycli run {{prompt}}",
        "执行器需要命令——例如 skit runner add mycli mycli run {{prompt}}",
        "執行器需要命令——例如 skit runner add mycli mycli run {{prompt}}",
    ),
    row!(
        "A runner command must contain the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
        "执行器命令必须恰好包含一个 {{prompt}} 槽位——渲染后的提示词会放在那里。",
        "執行器命令必須恰好包含一個 {{prompt}} 槽位——渲染後的提示詞會放在那裡。",
    ),
    row!(
        "The command needs the {{prompt}} slot exactly once — that's where the rendered prompt lands.",
        "命令必须恰好包含一个 {{prompt}} 槽位——渲染后的提示词会放在那里。",
        "命令必須恰好包含一個 {{prompt}} 槽位——渲染後的提示詞會放在那裡。",
    ),
    row!(
        "{{prompt}} can't be the command itself — the first word must be the program to run.",
        "{{prompt}} 不能是命令本身——第一个词必须是要运行的程序。",
        "{{prompt}} 不能是命令本身——第一個詞必須是要執行的程式。",
    ),
    row!(
        "Runner commands take only the {{prompt}} slot — single-brace text is literal, and other {{holes}} aren't supported.",
        "执行器命令只接受 {{prompt}} 槽位——单大括号是字面文字,其他 {{占位符}} 不支持。",
        "執行器命令只接受 {{prompt}} 槽位——單大括號是字面文字,其他 {{佔位符}} 不支援。",
    ),
    row!(
        "The command must be a list of text arguments.",
        "命令必须是文本参数列表。",
        "命令必須是文字引數清單。",
    ),
    row!(
        "This runner row isn't a table.",
        "此执行器行不是表格。",
        "此執行器列不是表格。",
    ),
    row!(
        "Another row already uses this runner name.",
        "另一行已使用此执行器名称。",
        "另一列已使用此執行器名稱。",
    ),
    row!(
        "This runner row is malformed.",
        "此执行器行格式错误。",
        "此執行器列格式錯誤。",
    ),
    row!(
        "Unbalanced quotes in the command.",
        "命令里的引号不成对。",
        "命令裡的引號不成對。",
    ),
    row!("Help", "帮助", "說明"),
    row!("Previous field", "上一个字段", "上一個欄位"),
    row!("Close", "关闭", "關閉"),
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
    row!("Show version.", "显示版本。", "顯示版本。"),
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
        "Added: {} ({} mode)",
        "已添加：{}（{} 模式）",
        "已新增：{}（{} 模式）",
    ),
    row!("Added: {}", "已添加：{}", "已新增：{}"),
    row!("Description: {}", "说明：{}", "說明：{}"),
    row!("Managed parameters: {}", "受管参数：{}", "受管參數：{}",),
    row!(
        "Updated {}. Managed parameters: {}",
        "已更新 {}。受管理的参数:{}",
        "已更新 {}。受管理的參數:{}",
    ),
    row!(
        "Updated {}. Declared parameters: {}",
        "已更新 {}。已声明的参数:{}",
        "已更新 {}。已宣告的參數:{}",
    ),
    row!(
        "The run form now asks for the managed parameters — the script's own command-line form ({}) is set aside until they are removed (--unmanage).",
        "运行表单现在会询问这些管理的参数——脚本自己的命令行表单（{}）会先搁置，直到它们被移除（--unmanage）为止。",
        "執行表單現在會詢問這些管理的參數——腳本自己的命令列表單（{}）會先擱置，直到它們被移除（--unmanage）為止。",
    ),
    row!(
        "Variable insertion is off for {} — turn it on first with: skit params {} --interpolate",
        "{} 的变量插入已关闭——请先开启:skit params {} --interpolate",
        "{} 的變量插入已關閉——請先開啟:skit params {} --interpolate",
    ),
    row!(
        "Variable insertion is off — the body travels to the agent exactly as written. Turn it on with: skit params {} --interpolate",
        "变量插入已关闭——正文会原封不动送达 agent。开启:skit params {} --interpolate",
        "變量插入已關閉——內文會原封不動送達 agent。開啟:skit params {} --interpolate",
    ),
    row!(
        "Variable insertion is off — the body travels to the agent exactly as written (turn it on with: skit params {} --interpolate)",
        "变量插入已关闭——正文将按原样发送给代理（可使用以下命令开启：skit params {} --interpolate）",
        "變數插入已關閉——內容將依原樣傳送給代理（可使用以下命令開啟：skit params {} --interpolate）",
    ),
    row!(
        "Detected {} placeholders — too many to manage automatically, so none were. Manage the ones you need with: skit params {} --add NAME, or turn insertion off with --no-interpolate.",
        "检测到 {} 个占位符——数量过多，无法自动管理，因此未管理任何占位符。可使用 skit params {} --add NAME 管理所需占位符，或使用 --no-interpolate 关闭变量插入。",
        "偵測到 {} 個預留位置——數量過多，無法自動管理，因此未管理任何預留位置。可使用 skit params {} --add NAME 管理所需預留位置，或使用 --no-interpolate 關閉變數插入。",
    ),
    row!(
        "Detected parameters: {} (the run form asks for them; your last values are remembered)",
        "检测到的参数：{}（运行表单会询问这些参数，并记住你上次输入的值）",
        "偵測到的參數：{}（執行表單會詢問這些參數，並記住你上次輸入的值）",
    ),
    row!(
        "Secret parameter values are never saved by skit: {}",
        "skit 永远不会保存机密参数值：{}",
        "skit 永遠不會儲存機密參數值：{}",
    ),
    row!(
        "When this prompt runs, the selected agent receives those values as plaintext and may log or sync them.",
        "运行此提示词时，所选代理会以明文接收这些值，并可能记录或同步它们。",
        "執行此提示詞時，所選代理會以明文接收這些值，並可能記錄或同步它們。",
    ),
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
    row!("Added runner: {}", "已添加运行器：{}", "已新增執行器：{}"),
    row!(
        "Runner {} added: {}",
        "运行器 {} 已添加：{}",
        "執行器 {} 已新增：{}",
    ),
    row!(
        "Runner {} updated: {}",
        "运行器 {} 已更新：{}",
        "執行器 {} 已更新：{}",
    ),
    row!(
        "Runner {} removed.",
        "运行器 {} 已删除。",
        "執行器 {} 已移除。",
    ),
    row!(
        "The runner {} already exists — pass --force to replace its command.",
        "运行器 {} 已存在，请传入 --force 替换其命令。",
        "執行器 {} 已存在，請傳入 --force 取代其命令。",
    ),
    row!(
        "The runner {} already exists — pick another name.",
        "执行器 {} 已存在——请换一个名称。",
        "執行器 {} 已存在——請換一個名稱。",
    ),
    row!(
        "No agents are configured. Add one with: skit runner add mycli -- mycli run {{prompt}}",
        "尚未配置 Agent。请运行以下命令添加：skit runner add mycli -- mycli run {{prompt}}",
        "尚未設定 Agent。請執行以下命令新增：skit runner add mycli -- mycli run {{prompt}}",
    ),
    row!(
        "Reading the script from stdin needs an explicit --name.",
        "从 stdin 读脚本必须用 --name 指定名称。",
        "從 stdin 讀腳本必須用 --name 指定名稱。",
    ),
    row!(
        "Nothing arrived on stdin, so there is nothing to add.",
        "stdin 没有收到任何内容，没有东西可添加。",
        "stdin 沒有收到任何內容，沒有東西可加入。",
    ),
    row!(
        "No runner selected for {}. Pass --runner NAME, or pin one with: skit params {} --runner NAME",
        "{} 尚未选择执行器。请带上 --runner NAME,或固定一个:skit params {} --runner NAME",
        "{} 尚未選擇執行器。請帶上 --runner NAME,或釘選一個:skit params {} --runner NAME",
    ),
    row!(
        "The runner {} isn't configured (known: {}). Manage runners with: skit runner list",
        "执行器 {} 未配置(已知:{})。管理执行器:skit runner list",
        "執行器 {} 未設定(已知:{})。管理執行器:skit runner list",
    ),
    row!(
        "The built-in amp preset uses amp -x and runs the prompt once; it does not open an interactive session.",
        "内置 amp 预设使用 amp -x，只运行一次提示词，不会打开交互式会话。",
        "內建 amp 預設使用 amp -x，只執行一次提示詞，不會開啟互動式工作階段。",
    ),
    row!(
        "Unknown runner: {}. Configured runners: {}",
        "未知运行器：{}。已配置的运行器：{}",
        "未知執行器：{}。已設定的執行器：{}",
    ),
    row!(
        "Unknown runner row: {}. Inspect with: skit runner list --all",
        "未知执行器行：{}。请用 skit runner list --all 检查",
        "未知執行器列：{}。請用 skit runner list --all 檢查",
    ),
    row!(
        "Unknown runner row: container. Inspect with: skit runner list --all",
        "未知执行器行：container。请用 skit runner list --all 检查",
        "未知執行器列：container。請用 skit runner list --all 檢查",
    ),
    row!(
        "Runner row {} is valid. Remove the agent by name instead: skit runner remove {}",
        "运行器行 {} 有效。请改为按名称删除 Agent：skit runner remove {}",
        "執行器資料列 {} 有效。請改為依名稱移除 Agent：skit runner remove {}",
    ),
    row!(
        "1 prompt pins this runner and will need another runner before it can run again.",
        "有 1 个提示词固定使用此运行器，需要改用其他运行器后才能再次运行。",
        "有 1 個提示詞固定使用此執行器，需要改用其他執行器後才能再次執行。",
    ),
    row!(
        "{} prompts pin this runner and will need another runner before they can run again.",
        "有 {} 个提示词固定使用此运行器，需要改用其他运行器后才能再次运行。",
        "有 {} 個提示詞固定使用此執行器，需要改用其他執行器後才能再次執行。",
    ),
    row!(
        "Confirmation is required; pass --yes to remove the runner.",
        "需要确认；请传入 --yes 删除运行器。",
        "需要確認；請傳入 --yes 移除執行器。",
    ),
    row!(
        "Remove the agent \"{}\"? [y/N]: ",
        "删除 Agent“{}”？[y/N]：",
        "移除 Agent「{}」？[y/N]：",
    ),
    row!(
        "Remove runner row {} (\"{}\")? [y/N]: ",
        "删除运行器行 {}（“{}”）？[y/N]：",
        "移除執行器資料列 {}（「{}」）？[y/N]：",
    ),
    row!(
        "Remove the malformed prompt runner container? [y/N]: ",
        "删除格式错误的提示词运行器容器？[y/N]：",
        "移除格式錯誤的提示詞執行器容器？[y/N]：",
    ),
    row!(
        "The runner row changed before it could be removed; inspect again.",
        "运行器行在删除前已更改；请重新检查。",
        "執行器資料列在移除前已變更；請重新檢查。",
    ),
    row!(
        "Malformed runner row {} removed.",
        "格式错误的运行器行 {} 已删除。",
        "格式錯誤的執行器資料列 {} 已移除。",
    ),
    row!(
        "Malformed prompt runner container removed.",
        "格式错误的提示词运行器容器已删除。",
        "格式錯誤的提示詞執行器容器已移除。",
    ),
    row!("Removed runner: {}", "已删除运行器：{}", "已移除執行器：{}"),
    row!("Saved preset: {}", "已保存预设：{}", "已儲存預設：{}"),
    row!(
        "Preset \"{}\" saved.",
        "已存成参数组合“{}”。",
        "已存成參數組合「{}」。",
    ),
    row!("Deleted preset: {}", "已删除预设：{}", "已刪除預設：{}"),
    row!(
        "{} has no form fields, so there's nothing to save.",
        "{} 没有表单字段，没有东西可存。",
        "{} 沒有表單欄位，沒有東西可存。",
    ),
    row!(
        "Reusing your last arguments: {}",
        "沿用上次的参数:{}",
        "沿用上次的參數:{}",
    ),
    row!(
        "{} has no remembered values yet — run it once first.",
        "{} 还没有记住的值——先运行一次。",
        "{} 還沒有記住的值——先執行一次。",
    ),
    row!(
        "Secret values are never stored in presets; skipped: {}",
        "机密值不会存入参数组合;已跳过:{}",
        "機密值不會存入參數組合;已略過:{}",
    ),
    row!(
        "Preset \"{}\" saved for {}.",
        "参数组合“{}”已为 {} 保存。",
        "參數組合「{}」已為 {} 儲存。",
    ),
    row!(
        "No presets for {} yet. Create one with: skit run {} --save-preset <preset>",
        "{} 还没有参数组合。创建一个：skit run {} --save-preset <组合名>",
        "{} 還沒有參數組合。建立一個：skit run {} --save-preset <組合名>",
    ),
    row!(
        "Preset \"{}\" deleted from {}.",
        "参数组合“{}”已从 {} 删除。",
        "參數組合「{}」已從 {} 刪除。",
    ),
    row!(
        "Unknown preset \"{}\". Available: {}",
        "没有名为“{}”的参数组合。现有:{}",
        "沒有名為「{}」的參數組合。現有:{}",
    ),
    row!(
        "Leave empty to use the script's own default.",
        "留空＝用脚本自己的默认值。",
        "留空＝用腳本自己的預設。",
    ),
    row!(
        "Leave empty and the script will ask you in the terminal.",
        "留空＝运行时脚本自己在终端问你。",
        "留空＝執行時腳本自己在終端機問你。",
    ),
    row!("required", "必填", "必填"),
    row!("whole number", "整数", "整數"),
    row!("number", "数字", "數字"),
    row!("text", "文本", "文字"),
    row!("on/off", "开/关", "開/關"),
    row!("path", "路径", "路徑"),
    row!("a whole number", "一个整数", "一個整數"),
    row!("a number", "一个数字", "一個數字"),
    row!("on or off", "开或关", "開或關"),
    row!("never saved to disk", "永不保存到磁盘", "永不儲存至磁碟"),
    row!("browse", "浏览", "瀏覽"),
    row!("insert", "插入", "插入"),
    row!("New agent…", "新建代理…", "新增代理…"),
    row!(
        "none yet — fill the form and press Ctrl+S to save one",
        "尚无参数组合——填写表单后按 Ctrl+S 保存一个",
        "尚無參數組合——填寫表單後按 Ctrl+S 儲存一個",
    ),
    row!(
        "This script has subcommands skit can't model — type everything into the extra-arguments field.",
        "此脚本含有 skit 无法建模的子命令——请在额外参数字段中输入全部内容。",
        "此指令稿含有 skit 無法建模的子命令——請在額外參數欄位中輸入全部內容。",
    ),
    row!(
        "skit couldn't read this script's argument declarations — type everything into the extra-arguments field.",
        "skit 无法读取此脚本的参数声明——请在额外参数字段中输入全部内容。",
        "skit 無法讀取此指令稿的參數宣告——請在額外參數欄位中輸入全部內容。",
    ),
    row!(
        "{} needs {} — you typed {}.",
        "{} 需要{}——你输入的是 {}。",
        "{} 需要{}——你輸入的是 {}。",
    ),
    row!(
        "{} must be one of: {}",
        "{} 必须是以下值之一：{}",
        "{} 必須是以下值之一：{}",
    ),
    row!(
        "✓ matches {} file(s)",
        "✓ 匹配 {} 个文件",
        "✓ 符合 {} 個檔案",
    ),
    row!("⚠ matches no files yet", "⚠ 尚未匹配文件", "⚠ 尚未符合檔案"),
    row!("Extra agent arguments", "额外代理参数", "額外代理參數"),
    row!("Extra command arguments", "额外命令参数", "額外命令參數"),
    row!(
        "Extra arguments (passed to the script as-is)",
        "额外参数（原样传给脚本）",
        "額外參數（原樣傳給指令稿）",
    ),
    row!(
        "Leave empty to read it from the environment variable {}.",
        "留空＝从环境变量 {} 读取。",
        "留空＝從環境變數 {} 讀取。",
    ),
    row!(
        "Enter to read it from the environment variable {}.",
        "直接按 Enter 就从环境变量 {} 读取。",
        "直接按 Enter 就從環境變數 {} 讀取。",
    ),
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
    row!(
        "The draft's #! names no interpreter skit knows — add it with: skit add {} --kind <language>\nYour draft was kept at {}",
        "草稿的 #! 指定了 skit 不认识的解释器——请用以下命令添加:skit add {} --kind <语言>\n你的草稿保留在 {}",
        "草稿的 #! 指定了 skit 不認識的直譯器——請用以下指令加入:skit add {} --kind <語言>\n你的草稿保留在 {}",
    ),
    row!("# New prompt", "# 新提示词", "# 新提示詞"),
    row!(
        "Nothing was written, so no prompt was added.",
        "没有写入任何内容，因此没有添加提示词。",
        "沒有寫入任何內容，因此沒有加入提示詞。",
    ),
    row!(
        "Nothing was written, so no script was added.",
        "没有写入任何内容,因此没有加入脚本。",
        "沒有寫入任何內容,因此沒有加入腳本。",
    ),
    row!(
        "Nothing was written, so nothing was added.",
        "没有写入任何内容，因此没有添加任何条目。",
        "未寫入任何內容，因此未新增任何項目。",
    ),
    row!(
        "What would you like to add?",
        "你想添加什么？",
        "你想新增什麼？",
    ),
    row!(
        "A file you already have — a script, program, or prompt",
        "已有文件——脚本、程序或提示词",
        "已有檔案——指令稿、程式或提示詞",
    ),
    row!(
        "A new script, written in your editor",
        "在编辑器中编写新脚本",
        "在編輯器中撰寫新指令稿",
    ),
    row!(
        "A new AI-agent prompt, written in your editor",
        "在编辑器中编写新的 AI 代理提示词",
        "在編輯器中撰寫新的 AI 代理提示詞",
    ),
    row!(
        "A command template (e.g. ffmpeg -i {input})",
        "命令模板（例如 ffmpeg -i {input}）",
        "指令範本（例如 ffmpeg -i {input}）",
    ),
    row!("Which one?", "选择哪一个？", "選擇哪一個？"),
    row!(
        "Choose a number from 1 to 4.",
        "请选择 1 到 4 之间的数字。",
        "請選擇 1 到 4 之間的數字。",
    ),
    row!("Path to the file", "文件路径", "檔案路徑"),
    row!("Name in skit", "skit 中的名称", "skit 中的名稱"),
    row!(
        "Cancelled — nothing was added.",
        "已取消——未添加任何条目。",
        "已取消——未新增任何項目。",
    ),
    row!(
        "{} need a source — pass the path in the same command (skit add PATH …) (nothing was added).",
        "{} 需要源文件——请在同一命令中传入路径（skit add PATH …）（未添加任何条目）。",
        "{} 需要來源檔案——請在同一指令中傳入路徑（skit add PATH …）（未新增任何項目）。",
    ),
    row!(
        "{} need a source — pass the path in the same command (skit add PATH …), or pick a lane outright with {} (nothing was added).",
        "{} 需要源文件——请在同一命令中传入路径（skit add PATH …），或直接用 {} 选择添加方式（未添加任何条目）。",
        "{} 需要來源檔案——請在同一指令中傳入路徑（skit add PATH …），或直接用 {} 選擇新增方式（未新增任何項目）。",
    ),
    row!(
        "Deleted the draft {}.",
        "已删除草稿 {}。",
        "已刪除草稿 {}。",
    ),
    row!(
        "refusing to remove a file outside skit's drafts directory",
        "拒绝删除 skit 草稿目录以外的文件",
        "拒絕移除 skit 草稿目錄以外的檔案",
    ),
    row!(
        "skit's drafts path is not an owned directory: {}",
        "skit 的草稿路径不是其自有目录：{}",
        "skit 的草稿路徑不是其自有目錄：{}",
    ),
    row!(
        "source changed while the add review was open; review it again",
        "源文件在添加审核期间发生了变化；请重新审核",
        "來源檔案在新增檢查期間已變更；請重新檢查",
    ),
    row!("Dependencies: {}", "依赖:{}", "依賴:{}"),
    row!(
        "Dependencies of {} updated: {}",
        "{} 的依赖已更新:{}",
        "{} 的依賴已更新:{}",
    ),
    row!(
        "Python constraint of {} updated: {}",
        "{} 的 Python 版本约束已更新:{}",
        "{} 的 Python 版本約束已更新:{}",
    ),
    row!(
        "Python constraint: {}",
        "Python 版本约束:{}",
        "Python 版本約束:{}",
    ),
    row!("Required commands: {}", "所需命令：{}", "必要命令：{}"),
    row!(
        "Needs of {} updated: {}",
        "{} 所需的命令已更新：{}",
        "{} 所需的命令已更新：{}",
    ),
    row!(
        "First run — downloading uv {}…",
        "首次运行:正在下载 uv {}…",
        "首次執行:正在下載 uv {}…",
    ),
    row!("uv installed at: {}", "uv 已安装:{}", "uv 已安裝:{}"),
    row!(
        "skit needs Astral's uv to run Python scripts, but it wasn't found on this system. Download uv {} into skit's private directory ({})? This won't touch your PATH or global environment. [Y/n]",
        "skit 需要 Astral uv 才能运行 Python 脚本,但系统上找不到。要下载 uv {} 到 skit 的私有目录({})吗?不会动到你的 PATH 或全局环境。[Y/n]",
        "skit 需要 Astral uv 才能執行 Python 腳本,但系統上找不到。要下載 uv {} 到 skit 的私有目錄({})嗎?不會動到你的 PATH 或全局環境。[Y/n]",
    ),
    row!(
        "Download declined. Install uv yourself (https://docs.astral.sh/uv/getting-started/installation/) and skit will pick it up automatically.",
        "已取消下载。你可以自行安装 uv(https://docs.astral.sh/uv/getting-started/installation/),skit 会自动检测并使用。",
        "已取消下載。你可以自行安裝 uv(https://docs.astral.sh/uv/getting-started/installation/),skit 會自動偵測並使用。",
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
        "工具库：{}（{} 字节）",
        "工具庫：{}（{} 位元組）",
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
        "Ignored malformed runner row(s) in config: {}. Inspect and repair with: skit runner list --all",
        "已忽略配置中格式错误的执行器行：{}。检查并修复：skit runner list --all",
        "已忽略設定中格式錯誤的執行器列：{}。檢查並修復：skit runner list --all",
    ),
    row!("WARN {}", "警告 {}", "警告 {}"),
    row!(
        "Agent directories on this machine:",
        "此计算机上的 Agent 目录：",
        "此電腦上的 Agent 目錄：",
    ),
    row!("user", "用户", "使用者"),
    row!("project", "项目", "專案"),
    row!(
        "Install where? [1-{}] (1): ",
        "安装到哪里？[1-{}] (1)：",
        "要安裝到哪裡？[1-{}] (1)：",
    ),
    row!(
        "Choose a number from 1 to {}.",
        "请选择 1 到 {} 之间的数字。",
        "請選擇 1 到 {} 之間的數字。",
    ),
    row!(
        "Write the skill into {}? [Y/n] ",
        "将 Skill 写入 {}？[Y/n] ",
        "要將 Skill 寫入 {} 嗎？[Y/n] ",
    ),
    row!(
        "Cancelled — nothing was written.",
        "已取消，未写入任何内容。",
        "已取消，未寫入任何內容。",
    ),
    row!(
        "Use a named target (with optional --project) or --to — not both.",
        "请使用命名目标（可选 --project）或 --to，不能同时使用。",
        "請使用具名目標（可選 --project）或 --to，不能同時使用。",
    ),
    row!(
        "Unknown target {}. Valid targets: claude, codex, agents.",
        "未知目标 {}。有效目标：claude、codex、agents。",
        "未知目標 {}。有效目標：claude、codex、agents。",
    ),
    row!(
        "Nothing installed: name a target (claude, codex, agents) or pass --to DIR.",
        "未安装任何内容：请指定目标（claude、codex、agents）或传入 --to DIR。",
        "未安裝任何內容：請指定目標（claude、codex、agents）或傳入 --to DIR。",
    ),
    row!(
        "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). Pass --to DIR to choose one yourself.",
        "未检测到 Agent 目录（~/.claude、~/.codex、./.agents 等）。请传入 --to DIR 自行选择。",
        "未偵測到 Agent 目錄（~/.claude、~/.codex、./.agents 等）。請傳入 --to DIR 自行選擇。",
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
    row!(
        "Run finished with exit status {}",
        "运行完成，退出状态为 {}",
        "執行完成，結束狀態為 {}",
    ),
    row!("Error: {}", "错误：{}", "錯誤：{}"),
    row!(
        "{} hasn't run yet — press Enter to fill the form first.",
        "{} 尚未运行，请先按 Enter 填写表单。",
        "{} 尚未執行，請先按 Enter 填寫表單。",
    ),
    row!("→ inject: {}", "→ 注入：{}", "→ 注入：{}"),
    row!(
        "  (written to a temporary copy, deleted after the run; your original file is untouched)",
        "  （写入临时副本；运行后删除；原始文件不变）",
        "  （寫入暫存副本；執行後刪除；原始檔不變）",
    ),
    row!("→ {}", "→ {}", "→ {}"),
    row!("warning: {}", "警告：{}", "警告：{}"),
    row!(
        "No entries yet. Add one with: skit add <path>",
        "还没有任何条目。用 skit add <path> 添加一个。",
        "還沒有任何條目。用 skit add <path> 加入一個。",
    ),
    row!("⚠ missing: {}", "⚠ 缺失:{}", "⚠ 遺失:{}"),
    row!(
        "Prompt {} isn't valid UTF-8 (invalid byte at offset {}).",
        "提示词 {} 不是有效的 UTF-8（偏移量 {} 处存在无效字节）。",
        "提示詞 {} 不是有效的 UTF-8（位移 {} 處有無效位元組）。",
    ),
    row!("Slug", "短名", "短名"),
    row!("Kind", "类型", "類型"),
    row!("Kind: {}", "类型：{}", "類型：{}"),
    row!("Python", "Python", "Python"),
    row!("Shell", "Shell", "Shell"),
    row!("fish", "fish", "fish"),
    row!("JavaScript", "JavaScript", "JavaScript"),
    row!("TypeScript", "TypeScript", "TypeScript"),
    row!("PowerShell", "PowerShell", "PowerShell"),
    row!("Ruby", "Ruby", "Ruby"),
    row!("Perl", "Perl", "Perl"),
    row!("Lua", "Lua", "Lua"),
    row!("R", "R", "R"),
    row!("Program", "程序", "程式"),
    row!("Command", "命令", "指令"),
    row!("Prompt", "提示词", "提示詞"),
    row!("Storage mode", "存储模式", "儲存模式"),
    row!("Storage mode: {}", "存储模式：{}", "儲存模式：{}"),
    row!("Source: {}", "来源:{}", "來源:{}"),
    row!("Work directory: {}", "工作目录：{}", "工作目錄：{}"),
    row!("Working directory: {}", "工作目录:{}", "工作目錄:{}"),
    row!("Missing: {}", "缺失：{}", "遺失：{}"),
    row!(
        "Missing parameter values: {}",
        "缺少参数值:{}",
        "缺少參數值:{}"
    ),
    // A backtick substitution strips one layer of backslashes before the inner command
    // parses, so the escape this branch writes arrives bare. Version 0.4 refuses instead of
    // assembling a command that quietly means something else (`src/skit/langs/launch.py:296`).
    row!(
        "Can't safely fill in a value inside double quotes nested in a `…` command substitution — the shell strips one layer of escaping there. Rewrite that part of the template with $(…) instead of backticks.",
        "没办法安全地把值填进嵌套在 `…` 命令替换里的双引号——shell 在那里会多剥掉一层转义。请把模板的那一段改用 $(…)，不要用反引号。",
        "沒辦法安全地把值填進巢狀在 `…` 命令替換裡的雙引號——shell 在那裡會多剝掉一層跳脫。請把模板的那一段改用 $(…)，不要用反引號。",
    ),
    row!("Drift: {}", "漂移：{}", "偏移：{}"),
    row!("Interpreter: {}", "解释器:{}", "直譯器:{}"),
    row!("Template: {}", "模板：{}", "範本：{}"),
    row!("Needs: {}", "所需命令：{}", "所需命令：{}"),
    row!("Command template: {}", "命令模板:{}", "命令模板:{}"),
    row!("Row", "行", "列"),
    row!("Status", "状态", "狀態"),
    row!("container", "容器", "容器"),
    row!("Runner", "执行器", "執行器"),
    row!("Runner: {}", "执行器:{}", "執行器:{}"),
    row!("(asks at run time)", "(运行时询问)", "(執行時詢問)"),
    row!(
        "Variable insertion: off (the body travels as written)",
        "变量插入:关闭(正文原样送达)",
        "變量插入:關閉(內文原樣送達)",
    ),
    row!(
        "skit could not model this script's own arguments; pass them after -- instead.",
        "skit 无法读懂这个脚本自己的参数声明；请把参数放在 -- 之后直接传入。",
        "skit 無法讀懂這支腳本自己的參數宣告；請把參數放在 -- 之後直接傳入。",
    ),
    row!(
        "No form fields — arguments after -- go to the selected agent.",
        "没有表单字段——接在 -- 之后的参数会传给所选 agent。",
        "沒有表單欄位——接在 -- 之後的參數會傳給所選 agent。",
    ),
    row!(
        "No form fields — arguments after -- are appended to the command.",
        "没有表单字段——接在 -- 之后的参数会附加到命令末尾。",
        "沒有表單欄位——接在 -- 之後的參數會附加到指令末尾。",
    ),
    row!(
        "No form fields — arguments after -- pass straight through to the script.",
        "没有表单字段——接在 -- 之后的参数会透传给脚本。",
        "沒有表單欄位——接在 -- 之後的參數會透傳給腳本。",
    ),
    row!(
        "Run it: skit run {}",
        "运行:skit run {}",
        "執行:skit run {}"
    ),
    row!("Parameter", "参数", "參數"),
    row!("Type", "类型", "型別"),
    row!("Required", "必填", "必填"),
    row!("Default", "默认值", "預設值"),
    row!("Choices", "可选值", "可選值"),
    row!("Secret", "机密", "機密"),
    row!("•••", "•••", "•••"),
    row!(
        "The parameter definitions for {} have drifted from the script:",
        "{} 的参数定义与脚本内容已漂移:",
        "{} 的參數定義與腳本內容已漂移:",
    ),
    row!(
        "{} is no longer read from the environment (its ${...:-default} was removed or overridden by a plain assignment) — your value would be silently ignored. Re-add or resync.",
        "{} 不再从环境变量读取(其 ${...:-default} 已被删除,或被普通赋值覆盖)——你设置的值会被悄悄忽略。请重新添加或执行 resync。",
        "{} 不再從環境變數讀取(其 ${...:-default} 已被移除,或被普通賦值覆蓋)——你設定的值會被默默忽略。請重新加入或執行 resync。",
    ),
    row!(
        "{}: injection target no longer exists (dropped from this run's form)",
        "{}:找不到注入目标(已从本次表单剔除)",
        "{}:找不到注入目標(已從本次表單剔除)",
    ),
    row!(
        "{}: type changed from {} to {} in the source (still injected — double-check the value)",
        "{}:源码中的类型已从 {} 变为 {}(仍会注入,请确认值)",
        "{}:原始碼中的型別已從 {} 變為 {}(仍會注入,請確認值)",
    ),
    row!(
        "{}: its prompt no longer matches a unique input/read call; falling back to position (still injected — double-check this lands on the right question, especially if it's a secret)",
        "{}:其提示文字已无法对应到唯一的 input/read 调用;改以位置对应(仍会注入——请再次确认其对应到正确的问题,尤其是机密参数)",
        "{}:其提示文字已無法對應到唯一的 input/read 呼叫;改以位置對應(仍會注入——請再次確認其對應到正確的問題,尤其是機密參數)",
    ),
    row!(
        "To refresh the definitions, run: skit params {} --resync",
        "若要更新定义,请运行:skit params {} --resync",
        "若要更新定義,請執行:skit params {} --resync",
    ),
    row!(
        "No longer in the prompt (the value would be ignored): {} — edit the body or update parameters with: skit params {}",
        "提示词中已不存在(其值会被忽略):{}——编辑正文,或更新参数:skit params {}",
        "提示詞中已不存在(其值會被忽略):{}——編輯內文,或更新參數:skit params {}",
    ),
    row!("Prompt runner: {}", "提示词运行器：{}", "提示詞執行器：{}"),
    row!("Interpolation: {}", "插值：{}", "插值：{}"),
    row!(
        "Prompt placeholders (the run form asks for them):",
        "提示词的占位符(运行表单会询问):",
        "提示詞的佔位符(執行表單會詢問):",
    ),
    row!(
        "Declared environment variables (set on the run):",
        "声明的环境变量（运行时设置）：",
        "宣告的環境變數（執行時設定）：",
    ),
    row!("default {}", "默认 {}", "預設 {}"),
    row!("optional", "选填", "選填"),
    row!("secret", "机密", "機密"),
    row!(
        "No longer in the prompt (the value would be ignored): {} — remove with --rm, or edit the body.",
        "提示词中已不存在(其值会被忽略):{}——用 --rm 移除,或编辑正文。",
        "提示詞中已不存在(其值會被忽略):{}——用 --rm 移除,或編輯內文。",
    ),
    row!("Parameters:", "参数：", "參數："),
    row!("  {} ({}, {})", "  {}（{}，{}）", "  {}（{}，{}）"),
    row!("Presets: {}", "参数组合:{}", "參數組合:{}"),
    row!("Run: skit run {}", "运行：skit run {}", "執行：skit run {}"),
    row!("Parameter: {}", "参数：{}", "參數：{}"),
    row!(
        "{} has no managed parameters.",
        "{} 没有管理的参数。",
        "{} 沒有管理的參數。",
    ),
    row!(
        "{} has no managed parameters. Use --manage to bring a detected candidate under management.",
        "{} 没有管理的参数。用 --manage 把检测到的候选纳入管理。",
        "{} 沒有管理的參數。用 --manage 把偵測到的候選納入管理。",
    ),
    row!(
        "Detected but not yet managed: {}",
        "检测到但尚未管理：{}",
        "偵測到但尚未管理：{}",
    ),
    row!(
        "Detected but not yet managed: {} (use --manage to manage them)",
        "检测到但尚未管理：{}（用 --manage 管理）",
        "偵測到但尚未管理：{}（用 --manage 管理）",
    ),
    row!(
        "Detected but not yet managed: {} (use --add to manage them)",
        "检测到但尚未管理:{}(用 --add 管理)",
        "偵測到但尚未管理:{}(用 --add 管理)",
    ),
    row!(
        "Detected but not yet managed: {} … and {} more candidate (use --add to manage them)",
        "检测到但尚未管理：{}……另有 {} 个（用 --add 管理）",
        "偵測到但尚未管理：{}……另有 {} 個（用 --add 管理）",
    ),
    row!(
        "Detected but not yet managed: {} … and {} more candidates (use --add to manage them)",
        "检测到但尚未管理：{}……另有 {} 个（用 --add 管理）",
        "偵測到但尚未管理：{}……另有 {} 個（用 --add 管理）",
    ),
    row!(
        "Reference mode: skit never writes the original file — manage parameters by editing its [tool.skit] block in the source directly.",
        "参照模式：skit 绝不写入原始文件——请直接编辑源码中的 [tool.skit] 区块来管理参数。",
        "參照模式：skit 絕不寫入原始檔案——請直接編輯原始碼中的 [tool.skit] 區塊來管理參數。",
    ),
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
    row!(
        "{} isn't secret; --env-source only applies to secret parameters (mark it with --secret first).",
        "{} 不是机密参数；--env-source 只适用于机密参数（先用 --secret 标记）。",
        "{} 不是機密參數；--env-source 只適用於機密參數（先用 --secret 標記）。",
    ),
    row!("Secret: yes", "敏感值：是", "敏感值：是"),
    row!(
        "Removed previously stored plaintext value(s) for now-secret parameter(s): {}",
        "已移除下列刚设为机密的参数先前以明文存储的值:{}",
        "已移除下列剛設為機密的參數先前以明文儲存的值:{}",
    ),
    row!("yes", "是", "是"),
    row!("no", "否", "否"),
    row!("on", "开启", "開啟"),
    row!("off", "关闭", "關閉"),
    row!("not set", "未设置", "未設定"),
    row!(" (secret)", " (机密)", " (機密)"),
    row!("{} ({}) = {}{}", "{}（{}）= {}{}", "{}（{}）= {}{}",),
    row!(
        "input() #{}: {}{}",
        "input() 第 {} 个：{}{}",
        "input() 第 {} 個：{}{}",
    ),
    row!(
        "⚠ looks like a loop accumulator — probably not a parameter",
        "⚠ 看起来是循环的累加变量，多半不是参数",
        "⚠ 看起來是迴圈的累加變數，多半不是參數",
    ),
    row!(
        "Select the values that skit should manage (Space toggles; Enter accepts)",
        "选择由 skit 管理的值（空格切换；回车确认）",
        "選擇由 skit 管理的值（空白鍵切換；Enter 確認）",
    ),
    row!(
        "Found {} parameter candidate (constants / input() calls):",
        "找到 {} 个候选参数（常量 / input() 调用）：",
        "找到 {} 個候選參數（常數 / input() 呼叫）：",
    ),
    row!(
        "Found {} parameter candidates (constants / input() calls):",
        "找到 {} 个候选参数（常量 / input() 调用）：",
        "找到 {} 個候選參數（常數 / input() 呼叫）：",
    ),
    row!(
        "This script reads command-line arguments; the run form has an extra-arguments field for them.",
        "这个脚本会读取命令行参数；运行表单中有一个额外参数字段可供填写。",
        "這支腳本會讀取命令列參數；執行表單中有一個額外參數欄位可供填寫。",
    ),
    row!(
        "This script parses its own arguments ({}); skit couldn't model them statically, so the run form offers an extra-arguments field.",
        "这个脚本会自行解析参数（{}）；skit 无法静态建模这些参数，因此运行表单会提供一个额外参数字段。",
        "這支腳本會自行解析參數（{}）；skit 無法靜態建立這些參數的模型，因此執行表單會提供一個額外參數欄位。",
    ),
    row!(
        "✓ skit read this script's own arguments ({} field). Running it opens a form — nothing to memorize.",
        "✓ skit 已读取这个脚本自己的参数（{} 个字段）。运行时会打开表单，无需记忆命令。",
        "✓ skit 已讀取這支腳本自己的參數（{} 個欄位）。執行時會開啟表單，無需記憶指令。",
    ),
    row!(
        "✓ skit read this script's own arguments ({} fields). Running it opens a form — nothing to memorize.",
        "✓ skit 已读取这个脚本自己的参数（{} 个字段）。运行时会打开表单，无需记忆命令。",
        "✓ skit 已讀取這支腳本自己的參數（{} 個欄位）。執行時會開啟表單，無需記憶指令。",
    ),
    row!(
        "💡 {} are written directly inside the code, so skit can't turn them into form fields. To manage one, first give it a name at the top of the script, e.g. OUTPUT = '…' (skit edit {}).",
        "💡 {} 直接写在代码中，因此 skit 无法将其转换为表单字段。要管理其中一个，请先在脚本顶部为其命名，例如 OUTPUT = '…'（skit edit {}）。",
        "💡 {} 直接寫在程式碼中，因此 skit 無法將其轉換為表單欄位。若要管理其中一個，請先在腳本頂端為其命名，例如 OUTPUT = '…'（skit edit {}）。",
    ),
    row!(
        "Reference mode never touches the original file, so parameter setup was skipped.",
        "引用模式绝不会更改原始文件，因此已跳过参数设置。",
        "參照模式絕不會變更原始檔案，因此已略過參數設定。",
    ),
    row!(
        "The script declares its own dependencies (PEP 723): {}",
        "脚本声明了自己的依赖项（PEP 723）：{}",
        "腳本宣告了自己的相依套件（PEP 723）：{}",
    ),
    row!("Run", "运行", "執行"),
    row!("Run {}", "运行 {}", "執行 {}"),
    row!("Add", "添加", "新增"),
    row!("Edit", "编辑", "編輯"),
    row!("Settings", "设置", "設定"),
    row!("Settings for {}", "{} 的设置", "{} 的設定"),
    row!("Preset", "预设", "預設"),
    // Version 0.4 ships `参数组合` for this msgid. Product rule 1 makes the shipped `.po` text
    // authoritative, and this is the heading of the settings section that manages them.
    row!("Presets", "参数组合", "參數組合"),
    row!(
        "None yet — press Ctrl+S inside the run form to save one.",
        "还没有——在运行表单里按 Ctrl+S 就能存一组。",
        "還沒有——在執行表單裡按 Ctrl+S 就能存一組。",
    ),
    row!(
        "Untick a preset to delete it on save:",
        "取消勾选的参数组合会在保存时删除：",
        "取消勾選的參數組合會在儲存時刪除：",
    ),
    row!("delete this preset", "删除这个参数组合", "刪除這個參數組合"),
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
    // The entry-settings screen. Every row here is version 0.4 text, so each translation is the
    // one the shipped catalog gives (`src/skit/locales/*/LC_MESSAGES/skit.po`).
    row!("Entry settings · {}", "条目设置 · {}", "條目設定 · {}",),
    row!("Basics", "基本资料", "基本資料"),
    // Both refusals reach the footer through `render`, which replaces a composable row inside
    // composed text, so each is declared that way. Each is a whole sentence, so it can never
    // replace part of another.
    composable!("A name is required.", "必须提供名称。", "必須提供名稱。",),
    composable!(
        "The working directory must be origin, store, invoke, or an absolute path.",
        "工作目录必须是 origin、store、invoke,或一个绝对路径。",
        "工作目錄必須是 origin、store、invoke,或一個絕對路徑。",
    ),
    row!("Storage", "存法", "存法"),
    row!(
        "Run in (working directory)",
        "运行位置(工作目录)",
        "執行位置(工作目錄)",
    ),
    row!(
        "Runner (the agent this prompt runs with)",
        "执行器(此提示词使用的 AI agent)",
        "執行器(此提示詞使用的 AI agent)",
    ),
    row!("Dependencies", "依赖", "依賴"),
    row!(
        "Needs (external commands)",
        "所需命令（外部命令）",
        "所需命令（外部命令）",
    ),
    // The parameter section, in the order version 0.4 composes it
    // (`src/skit/tui_settings.py:588-719`).
    row!(
        "Parameters (the run form's fields)",
        "参数（运行表单的字段）",
        "參數（執行表單的欄位）",
    ),
    row!(
        "(programs have no managed parameters)",
        "（程序没有管理的参数）",
        "（程式沒有管理的參數）",
    ),
    row!(
        "Detected but not yet managed — tick to manage:",
        "检测到但尚未管理——勾选即可管理：",
        "偵測到但尚未管理——勾選即可管理：",
    ),
    row!(
        "This script's run form comes from its own command-line arguments. Managing a hardcoded constant here would replace that form — leave it as is.",
        "这个脚本的运行表单来自它自己的命令行参数。在这里管理一个写死的常量会取代那张表单——保持原样即可。",
        "這支腳本的執行表單來自它自己的命令列參數。在這裡管理一個寫死的常數會取代那張表單——維持原樣即可。",
    ),
    row!(
        "Every input() is managed — this script can now run with --no-input.",
        "所有 input() 都已管理——这个脚本现在可以用 --no-input 自动化。",
        "所有 input() 都已管理——這支腳本現在可以用 --no-input 自動化。",
    ),
    row!(
        "Saving re-reads the {placeholders} from the template.",
        "保存时会从模板重新读取 {placeholders}。",
        "儲存時會從模板重新讀取 {placeholders}。",
    ),
    row!(
        "Variable insertion ({{name}} placeholders become form fields)",
        "变量插入({{name}} 占位符会成为表单字段)",
        "變量插入({{name}} 佔位符會成為表單欄位)",
    ),
    row!(
        "Off — the body travels to the agent exactly as written.",
        "已关闭——正文会原封不动送达 agent。",
        "已關閉——內文會原封不動送達 agent。",
    ),
    row!(
        "Add a parameter — type a name, then Save:",
        "新增参数——输入名称后保存：",
        "新增參數——輸入名稱後儲存：",
    ),
    row!("new parameter name", "新参数名称", "新參數名稱"),
    // New in 0.5: the settings screen offers the rewrite version 0.4 advised from the command line
    // (`src/skit/cli.py:4014`), and the resync version 0.4 gave only a chord.
    row!(
        "Change a constant to an environment default — tick to normalize:",
        "把常量改为环境默认值——勾选即可规范化：",
        "把常數改為環境預設值——勾選即可正規化：",
    ),
    row!(
        "Read the parameter definitions from the script again on save",
        "保存时重新从脚本读取参数定义",
        "儲存時重新從腳本讀取參數定義",
    ),
    row!("New agent", "新增 agent", "新增 agent"),
    row!("Resync", "重新同步", "重新同步"),
    row!(
        "Description (shown in the Library)",
        "说明（显示在工具库）",
        "說明（顯示在工具庫）",
    ),
    row!(
        "Renaming keeps everything — remembered values, presets, the stored copy.",
        "改名不影响任何东西——记住的值、参数组合、保管的副本都会保留。",
        "改名不影響任何東西——記住的值、參數組合、保管的副本都會保留。",
    ),
    row!(
        "Keep a copy — your original file is never modified. Source: {}",
        "复制一份——你的原始文件永远不会被改动。来源：{}",
        "複製一份——你的原始檔永遠不會被改動。來源：{}",
    ),
    row!(
        "The source file's folder",
        "来源文件所在的文件夹",
        "來源檔案所在的資料夾"
    ),
    row!(
        "skit's stored-copy folder",
        "skit 保管副本的文件夹",
        "skit 保管副本的資料夾"
    ),
    row!(
        "Wherever skit is run from",
        "运行 skit 的所在位置",
        "執行 skit 的所在位置"
    ),
    row!(
        "A fixed folder (type it below)",
        "固定文件夹(在下方输入)",
        "固定資料夾(在下方輸入)",
    ),
    row!("/absolute/path", "/绝对/路径", "/絕對/路徑"),
    row!(
        "Interpreter / runtime",
        "解释器 / 运行时",
        "直譯器 / 執行環境"
    ),
    row!(
        "empty = automatic (shebang, then detection order)",
        "留空 = 自动(先看 shebang,再按检测顺序)",
        "留空 = 自動(先看 shebang,再依偵測順序)",
    ),
    row!(
        "comma separated, e.g. requests>=2,<3, rich",
        "逗号分隔，例如 requests>=2,<3, rich",
        "逗號分隔，例如 requests>=2,<3, rich",
    ),
    row!(
        "comma separated, e.g. chalk@^5, zod",
        "逗号分隔，例如 chalk@^5, zod",
        "逗號分隔，例如 chalk@^5, zod",
    ),
    row!(
        "Python constraint, e.g. \">=3.11\" (empty = automatic)",
        "Python 约束，例如 \">=3.11\"（留空＝自动）",
        "Python 約束，例如 \">=3.11\"（留空＝自動）",
    ),
    row!(
        "comma separated, e.g. ffmpeg, jq",
        "逗号分隔，例如 ffmpeg, jq",
        "逗號分隔，例如 ffmpeg, jq",
    ),
    // Why one row refuses an edit. Version 0.4 shows the linked-file sentence verbatim
    // (`src/skit/tui_settings.py:598-604`); the other three name refusals this version added.
    row!(
        "skit doesn't write to this file — maintain the [tool.skit] definitions in the source directly.",
        "skit 不写入这个文件——请直接在源码里维护 [tool.skit] 定义。",
        "skit 不寫入這個檔案——請直接在原始碼裡維護 [tool.skit] 定義。",
    ),
    row!(
        "The script declares this. Change it in the source.",
        "脚本自己声明了这一项。请在源码中修改。",
        "指令稿自己宣告了這一項。請在原始碼中修改。",
    ),
    row!(
        "Set when the entry was added. A different command changes it.",
        "在添加条目时设定。请用另一个命令修改。",
        "在新增項目時設定。請用另一個命令修改。",
    ),
    row!(
        "This value follows another field.",
        "此值跟随另一个字段。",
        "此值跟隨另一個欄位。",
    ),
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
        "工具库中显示的说明",
        "工具庫中顯示的說明",
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
        "--edit opens your editor, which --no-input forbids — pipe the script in instead: skit add - -n NAME",
        "--edit 会打开你的编辑器，而 --no-input 禁止这么做——请改用管道把脚本传进来：skit add - -n NAME",
        "--edit 會開啟你的編輯器，而 --no-input 禁止這麼做——請改用管道把腳本傳進來：skit add - -n NAME",
    ),
    row!(
        "Writing a new script in an editor needs an interactive terminal.",
        "用编辑器新建脚本需要交互式终端。",
        "用編輯器新建腳本需要互動式終端機。",
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
        "--raw runs the script as-is; --set, --preset, and --save-preset do not apply.",
        "--raw 会原样运行脚本;--set、--preset、--save-preset 都不适用。",
        "--raw 會原樣執行腳本;--set、--preset、--save-preset 都不適用。",
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
    // The run path's own refusal. Version 0.4 ships this exact sentence and these exact
    // translations (`src/skit/cli.py:2891`), so they stay byte-identical.
    row!(
        "--runner only applies to prompt entries.",
        "--runner 只适用于提示词条目。",
        "--runner 只適用於提示詞項目。",
    ),
    row!(
        "--row must be a non-negative index or 'container'.",
        "--row 必须是非负索引或“container”。",
        "--row 必須是非負索引或「container」。",
    ),
    row!(
        "Malformed --set (expected NAME=VALUE): {}",
        "--set 格式错误(应为 NAME=VALUE):{}",
        "--set 格式錯誤(應為 NAME=VALUE):{}",
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
        "A Python constraint doesn't apply to {} scripts.",
        "Python 约束不适用于 {} 脚本。",
        "Python 約束不適用於 {} 腳本。",
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
        "--prompt with no path opens your editor, which --no-input forbids — pipe the body in instead: skit add - --prompt -n NAME",
        "--prompt 未带路径时会打开你的编辑器，而 --no-input 禁止这么做——请改用管道把正文传进来：skit add - --prompt -n NAME",
        "--prompt 未帶路徑時會開啟你的編輯器，而 --no-input 禁止這麼做——請改用管道把內文傳進來：skit add - --prompt -n NAME",
    ),
    row!(
        "a prompt runner command needs {{prompt}} exactly once after the program",
        "提示词运行器命令必须在程序之后正好包含一次 {{prompt}}",
        "提示詞執行器命令必須在程式之後正好包含一次 {{prompt}}",
    ),
    // The human face of each runner-row problem. `PromptRunnerRow::reason` keeps the stable
    // symbolic token for machine readers; only these sentences are translated.
    row!(
        "a prompt runner needs a name",
        "提示词执行器需要名称",
        "提示詞執行器需要名稱",
    ),
    row!(
        "a prompt runner argv must be a list of strings",
        "提示词执行器的 argv 必须是字符串列表",
        "提示詞執行器的 argv 必須是字串清單",
    ),
    row!(
        "a prompt runner command needs nonempty arguments",
        "提示词执行器命令需要非空参数",
        "提示詞執行器命令需要非空引數",
    ),
    row!(
        "a prompt runner command supports only the {{prompt}} slot",
        "提示词执行器命令只支持 {{prompt}} 槽位",
        "提示詞執行器命令只支援 {{prompt}} 插槽",
    ),
    row!(
        "{{prompt}} cannot be the prompt runner program",
        "{{prompt}} 不能作为提示词执行器的程序",
        "{{prompt}} 不能作為提示詞執行器的程式",
    ),
    row!(
        "the prompt runner row is not a table",
        "提示词执行器行不是表格",
        "提示詞執行器列不是表格",
    ),
    row!(
        "another row already uses this prompt runner name",
        "另一行已使用此提示词执行器名称",
        "另一列已使用此提示詞執行器名稱",
    ),
    // Version 0.4 ships these three rebuild lines with this exact punctuation
    // (`src/skit/locales/*/LC_MESSAGES/skit.po`), so they stay byte-identical.
    row!(
        "{}: meta.toml is missing; skipped",
        "{}:缺 meta.toml,已跳过",
        "{}:缺 meta.toml,已略過",
    ),
    row!(
        "{}: meta.toml is corrupt ({}); skipped",
        "{}:meta.toml 损坏({}),已跳过",
        "{}:meta.toml 損毀({}),已略過",
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
        "Provide a source path — or pipe the text in (skit add - -n NAME; add --prompt for an AI-agent prompt), or register a command template with --cmd.",
        "请提供来源路径——或把文本管道进来（skit add - -n NAME；AI-agent 提示词再加 --prompt），或用 --cmd 登记一条命令模板。",
        "請提供來源路徑——或把文字管線進來（skit add - -n NAME；AI-agent 提示詞再加 --prompt），或用 --cmd 登記一條命令模板。",
    ),
    row!(
        "choice parameter {} has no choices",
        "选项参数 {} 没有可用选项",
        "選項參數 {} 沒有可用選項",
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
    row!(
        "{} isn't a script or an executable — pass --kind <language> for an extensionless script, --prompt for an AI-agent prompt, --exe for a program, or --cmd for a command template.",
        "{} 不是脚本也不是可执行文件——没有扩展名的脚本请加 --kind <language>，AI-agent 提示词请加 --prompt，程序请加 --exe，命令模板请用 --cmd。",
        "{} 不是腳本也不是可執行檔——沒有副檔名的腳本請加 --kind <language>，AI-agent 提示詞請加 --prompt，程式請加 --exe，命令範本請用 --cmd。",
    ),
    row!(
        "The piped text's #! names no interpreter skit knows — pass --kind <language> to choose one.",
        "管道文本的 #! 指定了 skit 不认识的解释器——请传入 --kind <language> 进行选择。",
        "管線文字的 #! 指定了 skit 不認識的直譯器——請傳入 --kind <language> 進行選擇。",
    ),
    row!(
        "The #! in {} names no interpreter skit knows — pass --kind <language> to choose one, or --exe to run it directly.",
        "{} 的 #! 指定了 skit 不认识的解释器——请用 --kind <语言> 指定一个，或用 --exe 直接运行。",
        "{} 的 #! 指定了 skit 不認識的直譯器——請用 --kind <語言> 指定一個，或用 --exe 直接執行。",
    ),
    row!(
        "The #! line pins a python version — recording requires-python {} (change it with --python).",
        "#! 行指定了 python 版本——记录 requires-python {}（可用 --python 更改）。",
        "#! 行指定了 python 版本——記錄 requires-python {}（可用 --python 更改）。",
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
    row!("File not found: {}", "找不到文件：{}", "找不到檔案：{}",),
    row!("Can't read {}: {}", "无法读取 {}：{}", "無法讀取 {}：{}",),
    row!("Not a file: {}", "不是文件：{}", "不是檔案：{}",),
    row!(
        "{} is a directory. Add it as a program that runs directly?",
        "{} 是一个目录。要作为直接运行的程序添加吗?",
        "{} 是一個目錄。要當作直接執行的程式加入嗎?",
    ),
    row!(
        "{} is a directory — pass --exe to add it as a program that runs directly.",
        "{} 是一个目录——加 --exe 可把它作为直接运行的程序加入。",
        "{} 是一個目錄——加 --exe 可把它作為直接執行的程式加入。",
    ),
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
        "The name {} is already taken — pick another name.",
        "名称 {} 已被占用——请换一个名称。",
        "名稱 {} 已被使用——請換一個名稱。",
    ),
    row!(
        "The name {} is already taken.",
        "名称 {} 已被使用。",
        "名稱 {} 已被使用。",
    ),
    row!(
        "entry {} changed while this operation was underway",
        "条目 {} 在此操作进行期间已更改",
        "項目 {} 在此操作進行期間已變更",
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
        "{} isn't a Python version constraint (e.g. \">=3.11\" or \">=3.12,<3.13\").",
        "{} 不是 Python 版本约束(例如 \">=3.11\" 或 \">=3.12,<3.13\")。",
        "{} 不是 Python 版本約束(例如 \">=3.11\" 或 \">=3.12,<3.13\")。",
    ),
    row!(
        "{} isn't a package requirement (e.g. \"requests\" or \"rich>=13,<16\").",
        "{} 不是软件包依赖(例如 \"requests\" 或 \"rich>=13,<16\")。",
        "{} 不是套件需求(例如 \"requests\" 或 \"rich>=13,<16\")。",
    ),
    row!(
        "Unknown language: {}. Available: {}",
        "未知语言:{}。可用语言:{}",
        "未知語言:{}。可用語言:{}",
    ),
    row!(
        "Unknown setting: {}. Available: {}",
        "未知的设置：{}。可用：{}",
        "未知的設定：{}。可用：{}",
    ),
    row!(
        "Unknown form style: {}. Choose from: tui, plain",
        "未知的表单形态：{}。可选：tui、plain",
        "未知的表單形態：{}。可選：tui、plain",
    ),
    row!(
        "Unknown after-run behavior: {}. Choose from: exit, stay",
        "未知的运行后行为：{}。可选：exit、stay",
        "未知的執行後行為：{}。可選：exit、stay",
    ),
    row!(
        "Unknown JS runner: {}. Choose from: {}",
        "未知的 JS 运行时：{}。可选：{}",
        "未知的 JS 執行環境：{}。可選：{}",
    ),
    row!(
        "Unknown mirror value: {}. \"mirror\" is the master switch (on / off); mirrors are picked per ecosystem: mirror.pypi ({}), mirror.github ({}), mirror.npm ({}) — each also takes a URL or \"off\".",
        "未知的 mirror 值：{}。“mirror”只是总开关（on / off）；镜像按生态各自挑选：mirror.pypi（{}）、mirror.github（{}）、mirror.npm（{}）——每项也接受 URL 或“off”。",
        "未知的 mirror 值：{}。「mirror」只是總開關（on / off）；鏡像按生態系各自挑選：mirror.pypi（{}）、mirror.github（{}）、mirror.npm（{}）——每項也接受 URL 或「off」。",
    ),
    row!(
        "Unknown {} value: {}. Choose from: {}, off — or give a full URL.",
        "未知的 {} 值：{}。可选：{}、off——或直接给出完整 URL。",
        "未知的 {} 值：{}。可選：{}、off——或直接給完整 URL。",
    ),
    row!(
        "Unknown mirror.github value: {}. Choose from: {}, off — or give an https:// github-release base URL (the uv binary is downloaded and executed, so https:// is required).",
        "未知的 mirror.github 值：{}。可选：{}、off——或给出一个 https:// 的 github-release 基底 URL（uv 主程序会被下载并执行，因此必须是 https://）。",
        "未知的 mirror.github 值：{}。可選：{}、off——或給一個 https:// 的 github-release 基底 URL（uv 主程式會被下載並執行，因此必須是 https://）。",
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
        "Nothing to enable: no mirror URLs are saved. Set an axis first: mirror.pypi / mirror.github / mirror.npm.",
        "没有可启用的镜像：尚未保存任何镜像 URL。请先设置某个轴：mirror.pypi / mirror.github / mirror.npm。",
        "沒有可啟用的鏡像：尚未儲存任何鏡像 URL。請先設定某個軸：mirror.pypi / mirror.github / mirror.npm。",
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
        "prompt body doesn't exist: {}",
        "提示词正文不存在：{}",
        "提示詞內容不存在：{}",
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
        "Reference-mode entries take no managed dependencies — they run from their own project. Add it as a copy, or drop --dep.",
        "reference 模式条目不受理依赖管理——它从自己的项目运行。以复制模式加入，或去掉 --dep。",
        "reference 模式條目不受理依賴管理——它從自己的專案執行。以複製模式加入，或拿掉 --dep。",
    ),
    row!(
        "{} is a reference-mode entry: it runs from its own project, which already provides its packages. Dependency management applies to copies.",
        "{} 是 reference 模式条目：它从自己的项目运行，包由该项目提供。依赖管理仅适用于复制进库的条目。",
        "{} 是 reference 模式條目：它從自己的專案執行，套件由該專案提供。依賴管理僅適用於複製進庫的條目。",
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
        "No JavaScript runtime found (looked for: {}). Install deno, bun, or node — or pick one with: skit config js.runner <name>",
        "找不到 JavaScript 运行时（查找过：{}）。请安装 deno、bun 或 node——或用 skit config js.runner <name> 指定一个。",
        "找不到 JavaScript 執行環境（查找過：{}）。請安裝 deno、bun 或 node——或用 skit config js.runner <name> 指定一個。",
    ),
    row!(
        "run source, schema, launch, runner, and interpolation changes as separate params operations",
        "请将源、结构、启动、运行器和插值更改作为单独的 params 操作运行",
        "請將來源、結構、啟動、執行器與插值變更作為單獨的 params 操作執行",
    ),
    row!(
        "Pass exactly one runner name or --row INDEX.",
        "请只传一个执行器名称或 --row INDEX。",
        "請只傳一個執行器名稱或 --row INDEX。",
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
        "source changed after semantic edit planning",
        "源文件在语义编辑规划后已更改",
        "來源在語義編輯規劃後已變更",
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
        "--prompt names the kind outright — drop --edit/--exe/--kind/--cmd.",
        "--prompt 已直接指定类型——请去掉 --edit/--exe/--kind/--cmd。",
        "--prompt 已直接指定類型——請去掉 --edit/--exe/--kind/--cmd。",
    ),
    row!("stdin ('-')", "stdin（'-'）", "stdin（'-'）"),
    row!("a file path", "文件路径", "檔案路徑"),
    row!(
        "{} each pick a different way to add — use exactly one (nothing was added).",
        "{} 各自代表一种不同的添加方式——请只用其中一种（未添加任何内容）。",
        "{} 各自代表一種不同的加入方式——請只用其中一種（未加入任何內容）。",
    ),
    row!(
        "a --cmd template takes only --name/--description",
        "--cmd 模板只接受 --name/--description",
        "--cmd 樣板只接受 --name/--description",
    ),
    row!(
        "stdin authors a brand-new copy, and --ref/--exe need an existing file",
        "stdin 会撰写一份全新副本，而 --ref/--exe 需要现成的文件",
        "stdin 會撰寫一份全新副本，而 --ref/--exe 需要現成的檔案",
    ),
    row!(
        "--edit drafts a fresh script: its kind comes from the shebang you write (e.g. #!/usr/bin/env bash), --ref/--exe need an existing file, and a prompt is drafted with skit add --prompt",
        "--edit 会起草一个全新脚本：它的类型取自你写的 shebang（例如 #!/usr/bin/env bash），--ref/--exe 需要现成的文件，而提示词要用 skit add --prompt 起草",
        "--edit 會草擬一支全新腳本：它的類型取自你寫的 shebang（例如 #!/usr/bin/env bash），--ref/--exe 需要現成的檔案，而提示詞要用 skit add --prompt 草擬",
    ),
    row!(
        "a drafted prompt takes only --name/--description/--runner/--no-interpolate",
        "草稿提示词只接受 --name/--description/--runner/--no-interpolate",
        "草稿提示詞只接受 --name/--description/--runner/--no-interpolate",
    ),
    row!(
        "{} can't apply here — {} (nothing was added).",
        "{} 在这里无法应用——{}(未添加任何内容)。",
        "{} 在這裡無法套用——{}(未加入任何內容)。",
    ),
    row!(
        "--no-interpolate only applies to prompt entries — add one with --prompt.",
        "--no-interpolate 只适用于提示词条目——用 --prompt 添加一个。",
        "--no-interpolate 只適用於提示詞項目——用 --prompt 加入一個。",
    ),
    row!(
        "--runner only applies to prompt entries — add one with --prompt.",
        "--runner 只适用于提示词条目——用 --prompt 添加一个。",
        "--runner 只適用於提示詞項目——用 --prompt 加入一個。",
    ),
    row!(
        "--ref can't apply here — stdin authors a brand-new copy, and --ref/--exe need an existing file (nothing was added).",
        "--ref 在这里无法应用——stdin 会撰写一份全新副本，而 --ref/--exe 需要现成的文件(未添加任何内容)。",
        "--ref 在這裡無法套用——stdin 會撰寫一份全新副本，而 --ref/--exe 需要現成的檔案(未加入任何內容)。",
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
        "terminal host effects did not settle",
        "终端宿主操作未能稳定结束",
        "終端宿主操作未能穩定結束",
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
        "Downloaded uv failed its checksum (the mirror may be compromised or the file corrupt). Expected {}, got {}.",
        "下载的 uv 未通过校验(镜像可能被篡改,或文件已损坏)。期望 {},实际为 {}。",
        "下載的 uv 未通過校驗(鏡像可能遭竄改,或檔案已損毀)。預期 {},實際為 {}。",
    ),
    row!(
        "No pinned checksum for platform {}; refusing to run an unverified uv.",
        "没有为平台 {} 预置校验和,拒绝运行未经校验的 uv。",
        "沒有為平台 {} 預置校驗碼,拒絕執行未經校驗的 uv。",
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
        "the rendered prompt contains a NUL byte; process arguments cannot contain NUL bytes",
        "渲染后的提示词包含 NUL 字节；进程参数不能包含 NUL 字节",
        "算繪後的提示詞包含 NUL 位元組；處理程序引數不能包含 NUL 位元組",
    ),
    row!(
        "the rendered prompt makes the command line {} {} — over this platform's limit of {} {}. Shorten the prompt or its parameter values.",
        "渲染后的提示词使命令行达到 {} {}，超过此平台的 {} {} 上限。请缩短提示词或参数值。",
        "算繪後的提示詞使命令列達到 {} {}，超過此平台的 {} {} 上限。請縮短提示詞或參數值。",
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
        "Unknown parameter for --set: {}. This entry's parameters: {}",
        "--set 指定了未知参数：{}。此条目的参数：{}",
        "--set 指定了未知參數：{}。此條目的參數：{}",
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
        "value {} for parameter {} is not a valid {}",
        "值 {} 对参数 {} 不是有效的 {} 值",
        "值 {} 對參數 {} 不是有效的 {} 值",
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
        "{} entries don't take package dependencies — drop --dep.",
        "{} 条目不接受依赖包——去掉 --dep。",
        "{} 條目不接受依賴套件——拿掉 --dep。",
    ),
    row!(
        "--dep/--python are python flags, but the draft's shebang names {} — drop them, or keep the python shebang.",
        "--dep/--python 是 python 专用标志，但草稿的 shebang 指定的是 {}——去掉它们，或改回 python shebang。",
        "--dep/--python 是 python 專用旗標，但草稿的 shebang 指定的是 {}——拿掉它們，或改回 python shebang。",
    ),
    row!(
        "{} is not a valid {} default",
        "{} 不是有效的 {} 默认值",
        "{} 不是有效的 {} 預設值",
    ),
    row!(
        "{} is empty, but {} is filled and they are read on the same line — a shell `read` would hand your value to {}. Fill {} in, or clear {}.",
        "{} 为空，但 {} 已填写，且它们从同一行读取——shell `read` 会把您的值交给 {}。请填写 {}，或清空 {}。",
        "{} 是空的，但 {} 已填寫，而且它們從同一行讀取——shell `read` 會把您的值交給 {}。請填寫 {}，或清空 {}。",
    ),
    row!(
        "{} can't contain a line break: a shell `read` takes ONE line, so everything after the break would be thrown away.",
        "{} 不能包含换行符：shell `read` 只读取一行，因此换行符之后的所有内容都会被丢弃。",
        "{} 不能包含換行符：shell `read` 只讀取一行，因此換行符之後的所有內容都會被丟棄。",
    ),
    row!(
        "{} is read on the same line as other values, so its value can't contain spaces or tabs — the shell would split it across the other fields. Only the LAST value on a `read` line may contain spaces.",
        "{} 与其他值从同一行读取，因此其值不能包含空格或制表符——shell 会将其拆分到其他字段中。只有 `read` 行中的最后一个值可以包含空格。",
        "{} 與其他值從同一行讀取，因此其值不能包含空格或定位字元——shell 會將其拆分到其他欄位中。只有 `read` 行中的最後一個值可以包含空格。",
    ),
    row!(
        "{} starts or ends with a space or tab, which a shell `read` strips off the line — the script would receive it trimmed. Remove the surrounding whitespace.",
        "{} 以空格或制表符开头或结尾，shell `read` 会从该行去除这些字符——脚本收到的值会被裁剪。请删除两端的空白字符。",
        "{} 以空格或定位字元開頭或結尾，shell `read` 會從該行移除這些字元——程式收到的值會被裁剪。請移除兩端的空白字元。",
    ),
    row!(
        "{} is not valid UTF-8",
        "{} 不是有效的 UTF-8",
        "{} 不是有效的 UTF-8",
    ),
    row!("{} is required.", "{} 为必填项。", "{} 為必填欄位。",),
    row!(
        "{} is on by default, so its flag could only ever turn it on again. Declare the flag that turns it OFF instead (--no-{} and the like), with default false.",
        "{} 默认已开启，因此它的选项只能再次开启它。请改为声明关闭它的选项（例如 --no-{}），并将默认值设为 false。",
        "{} 預設已開啟，因此它的選項只能再次開啟它。請改為宣告關閉它的選項（例如 --no-{}），並將預設值設為 false。",
    ),
    row!(
        "{} manages its parameters from the script itself — use --manage / --unmanage, or edit the [tool.skit] block.",
        "{} 从脚本自身管理参数——请使用 --manage / --unmanage，或直接编辑 [tool.skit] 区块。",
        "{} 由腳本自身管理參數——請使用 --manage / --unmanage，或直接編輯 [tool.skit] 區塊。",
    ),
    row!("Select", "选择", "選擇"),
    row!("Interface language", "界面语言", "介面語言"),
    row!(
        "Automatic (follow the system)",
        "自动（跟随系统）",
        "自動（跟隨系統）",
    ),
    row!("Currently in effect: {}", "当前生效：{}", "目前生效：{}",),
    row!("Editor", "编辑器", "編輯器"),
    row!(
        "e.g. code --wait (empty = use $VISUAL / $EDITOR)",
        "例如 code --wait（留空 = 使用 $VISUAL / $EDITOR）",
        "例如 code --wait（留空 = 使用 $VISUAL / $EDITOR）",
    ),
    row!(
        "Empty means: {} (from $VISUAL / $EDITOR)",
        "留空表示：{}（来自 $VISUAL / $EDITOR）",
        "留空表示：{}（來自 $VISUAL / $EDITOR）",
    ),
    row!("Interactive form", "交互式表单", "互動式表單"),
    row!(
        "Mini form — opens in place, fully clickable",
        "迷你表单——原地打开，所有控件均可点击",
        "迷你表單——原地開啟，所有控制項均可點選",
    ),
    row!(
        "Line-by-line prompts — plainest, best over slow terminals",
        "逐行提示——最朴素，最适合慢速终端",
        "逐行提示——最單純，最適合慢速終端機",
    ),
    row!(
        "Used by terminal runs: `skit run` parameter prompts and the `skit add` review panel.",
        "用于终端运行：`skit run` 参数提示和 `skit add` 检查面板。",
        "用於終端機執行：`skit run` 參數提示與 `skit add` 檢查面板。",
    ),
    row!(
        "After a run (from this menu)",
        "从此菜单运行后",
        "從此選單執行後",
    ),
    row!(
        "Quit skit — leave the run's output in the terminal",
        "退出 skit——在终端中保留运行输出",
        "結束 skit——在終端機中保留執行輸出",
    ),
    row!(
        "Return to the Library immediately",
        "立即返回工具库",
        "立即回到工具庫",
    ),
    row!(
        "Automatic — the first of deno / bun / node found",
        "自动——使用找到的第一个 deno / bun / node",
        "自動——使用找到的第一個 deno / bun / node",
    ),
    row!(
        "Runs js/ts entries that don't pin their own runtime.",
        "运行未固定自身运行时的 js/ts 条目。",
        "執行未固定自身執行環境的 js/ts 項目。",
    ),
    row!(
        "Shell on Windows",
        "Windows 上的 Shell",
        "Windows 上的 Shell"
    ),
    row!(
        "Path to bash.exe (empty = Git Bash / WSL detection)",
        "bash.exe 路径（留空 = 检测 Git Bash / WSL）",
        "bash.exe 路徑（留空 = 偵測 Git Bash / WSL）",
    ),
    row!(
        "Shell scripts need an explicit bash here.",
        "Shell 脚本需要在此明确指定 bash。",
        "Shell 指令稿需要在此明確指定 bash。",
    ),
    row!(
        "Agents (prompt runners)",
        "代理（提示词运行器）",
        "代理（提示詞執行器）"
    ),
    row!("No agents configured.", "尚未配置代理。", "尚未設定代理。"),
    row!(
        "{} agent configured: {}",
        "已配置 {} 个代理：{}",
        "已設定 {} 個代理：{}",
    ),
    row!(
        "{} agents configured: {}",
        "已配置 {} 个代理：{}",
        "已設定 {} 個代理：{}",
    ),
    row!("Manage agents…", "管理代理…", "管理代理…"),
    row!(
        "Teach an AI agent skit…",
        "教 AI 代理使用 skit…",
        "教 AI 代理使用 skit…",
    ),
    row!(
        "Teach an AI agent to use skit",
        "教 AI agent 使用 skit",
        "教 AI agent 使用 skit",
    ),
    row!(
        "No agent directories detected (~/.claude, ~/.codex, ./.agents, …). Install by hand with: skit agent install --to DIR",
        "没有检测到 agent 目录（~/.claude、~/.codex、./.agents 等）。可用 skit agent install --to DIR 手动安装。",
        "沒有偵測到 agent 目錄（~/.claude、~/.codex、./.agents 等）。可用 skit agent install --to DIR 手動安裝。",
    ),
    row!(
        "Installed the skit Agent Skill: {}",
        "已安装 skit Agent Skill：{}",
        "已安裝 skit Agent Skill：{}",
    ),
    row!(
        "Download mirrors (mainland-China acceleration)",
        "下载镜像（中国大陆加速）",
        "下載鏡像（中國大陸加速）",
    ),
    row!(
        "Each ecosystem is its own choice — mirror vendors differ per axis.",
        "每个生态系统独立选择——各项的镜像供应商不同。",
        "每個生態系統獨立選擇——各項的鏡像供應商不同。",
    ),
    row!(
        "Master switch — \"off\" pauses mirrors but keeps the saved URLs.",
        "总开关——“off”会暂停镜像，但保留已保存的 URL。",
        "總開關——「off」會暫停鏡像，但保留已儲存的 URL。",
    ),
    row!(
        "PyPI index (Python packages)",
        "PyPI 索引（Python 软件包）",
        "PyPI 索引（Python 套件）",
    ),
    row!("PyPI index URL", "PyPI 索引 URL", "PyPI 索引 URL"),
    row!(
        "GitHub releases (Python builds, the uv binary)",
        "GitHub 发布包（Python 构建、uv 二进制文件）",
        "GitHub 發行檔（Python 建置、uv 二進位檔）",
    ),
    row!(
        "github-release mirror base URL",
        "github-release 镜像基础 URL",
        "github-release 鏡像基礎 URL",
    ),
    row!(
        "npm registry (JS/TS packages)",
        "npm 注册表（JS/TS 软件包）",
        "npm 登錄檔（JS/TS 套件）",
    ),
    row!("npm registry URL", "npm 注册表 URL", "npm 登錄檔 URL"),
    row!("custom", "自定义", "自訂"),
    row!(
        "A custom choice needs a URL.",
        "自定义选项需要 URL。",
        "自訂選項需要 URL。"
    ),
    row!("auto ({})", "自动（{}）", "自動（{}）"),
    row!(
        "default ($VISUAL / $EDITOR)",
        "默认（$VISUAL / $EDITOR）",
        "預設（$VISUAL / $EDITOR）",
    ),
    row!(
        "auto (bash on PATH)",
        "自动（使用 PATH 上的 bash）",
        "自動（使用 PATH 上的 bash）",
    ),
    row!(
        "auto (deno > bun > node)",
        "自动（deno > bun > node）",
        "自動（deno > bun > node）",
    ),
    row!(
        "Mirrors are switched off — run `skit config mirror on` to activate them.",
        "镜像当前处于关闭状态——运行 `skit config mirror on` 启用。",
        "鏡像目前是關閉狀態——執行 `skit config mirror on` 啟用。",
    ),
    row!(
        "The uv binary is downloaded and executed, so the github-release base URL must use https:// (got: {}).",
        "uv 二进制文件会被下载并执行，因此 github-release 基础 URL 必须使用 https://（当前为：{}）。",
        "uv 二進位檔會被下載並執行，因此 github-release 基礎 URL 必須使用 https://（目前為：{}）。",
    ),
    row!("No such file: {}", "找不到文件：{}", "找不到檔案：{}"),
    row!("Saved {}.", "已保存 {}。", "已儲存 {}。"),
    row!(
        "skit reconciles parameter drift at run time; review managed parameters with: skit params {}",
        "skit 会在运行时自动校对参数差异;可用 skit params {} 查看管理中的参数",
        "skit 會在執行時自動校對參數差異;可用 skit params {} 檢視管理中的參數",
    ),
    row!(
        "{} has no editable source (programs and command templates run as-is).",
        "{} 没有可编辑的源码（程序和命令模板按原样运行）。",
        "{} 沒有可編輯的原始碼（程式與命令模板按原樣執行）。",
    ),
    row!(
        "{}: the referenced source file is gone: {}",
        "{}:reference 原文件已消失:{}",
        "{}:reference 原檔已消失:{}",
    ),
    row!(
        "{} has no stored copy to edit.",
        "{} 没有可编辑的副本。",
        "{} 沒有可編輯的副本。",
    ),
    row!(
        "Editing the original file (reference mode): {}",
        "正在编辑原始文件(reference 模式):{}",
        "正在編輯原始檔案(reference 模式):{}",
    ),
    row!(
        "Could not launch the editor ({}): {}. Set one with: skit config editor <cmd>",
        "无法启动编辑器（{}）：{}。可用 skit config editor <cmd> 设置。",
        "無法啟動編輯器（{}）：{}。可用 skit config editor <cmd> 設定。",
    ),
    row!(
        "Could not write the skill there: {}",
        "无法将 Skill 写入该位置：{}",
        "無法將 Skill 寫入該位置：{}",
    ),
    row!(
        "{} needs NAME=VALUE",
        "{}需要 NAME=VALUE",
        "{}需要 NAME=VALUE",
    ),
    row!(
        "Ignored a malformed value: {} (expected NAME=text).",
        "已忽略格式错误的值：{}（应为 NAME=text）。",
        "已忽略格式錯誤的值：{}（應為 NAME=text）。",
    ),
    row!(
        "{} reads from the environment variable {}, but it isn't set.",
        "{} 从环境变量 {} 读取，但该变量未设置。",
        "{} 從環境變數 {} 讀取，但該變數未設定。",
    ),
    row!("Write a script…", "编写脚本…", "編寫指令稿…"),
    row!("Draft a prompt…", "起草提示词…", "起草提示詞…"),
    row!("Delete draft…", "删除草稿…", "刪除草稿…"),
    row!("Continue", "继续", "繼續"),
    row!(
        "Keep a copy — skit stores it; your original file is never modified",
        "保留副本——skit 会存储它；绝不会修改原始文件",
        "保留副本——skit 會儲存它；絕不會修改原始檔案",
    ),
    row!(
        "Link the original — edits take effect immediately, but skit won't write to the file, so parameter definitions are yours to maintain",
        "链接原始文件——编辑会立即生效，但 skit 不会写入该文件，因此你需要自行维护参数定义",
        "連結原始檔案——編輯會立即生效，但 skit 不會寫入該檔案，因此你需要自行維護參數定義",
    ),
    row!(
        "Link the original — edits take effect immediately; skit never writes to the file",
        "链接原始文件——编辑会立即生效；skit 绝不会写入该文件",
        "連結原始檔案——編輯會立即生效；skit 絕不會寫入該檔案",
    ),
    row!(
        "Link the original: skit never writes to the file.",
        "链接原始文件：skit 绝不会写入该文件。",
        "連結原始檔案：skit 絕不會寫入該檔案。",
    ),
    row!(
        "Link the original: parameter setup is skipped — skit never writes to the file.",
        "链接原始文件：已跳过参数设置——skit 绝不会写入该文件。",
        "連結原始檔案：已略過參數設定——skit 絕不會寫入該檔案。",
    ),
    row!(
        "npm dependencies apply to stored copies only, so none are recorded.",
        "npm 依赖项仅适用于存储的副本，因此不会记录任何依赖项。",
        "npm 相依套件僅適用於儲存的副本，因此不會記錄任何相依套件。",
    ),
    row!(
        "Tick the ones the run form should ask for:",
        "勾选运行表单应询问的项目：",
        "勾選執行表單應詢問的項目：",
    ),
    row!(
        "No {{name}} placeholders detected — the body travels to the agent as written.",
        "未检测到 {{name}} 占位符——正文会原样发送给代理。",
        "未偵測到 {{name}} 預留位置——正文會原樣傳送給代理。",
    ),
    row!("Choose variables…", "选择变量…", "選擇變數…"),
    row!(
        "ask on the run form",
        "在运行表单中询问",
        "在執行表單中詢問",
    ),
    row!("Edit script", "编辑脚本", "編輯指令稿"),
    row!("Edit prompt", "编辑提示词", "編輯提示詞"),
    row!(
        "Path to a script, executable, or prompt:",
        "脚本、可执行文件或提示词的路径：",
        "指令稿、可執行檔或提示詞的路徑：",
    ),
    row!("Name for the command", "命令名称", "指令名稱"),
    row!("Description (optional)", "说明（可选）", "說明（選填）"),
    row!(
        "…or resume a kept draft:",
        "…或继续保留的草稿：",
        "…或繼續保留的草稿：",
    ),
    row!(
        "…or start from a blank page:",
        "…或从空白页开始：",
        "…或從空白頁開始：",
    ),
    row!("…and {} more", "…以及另外 {} 个", "…以及另外 {} 個"),
    row!(
        "The #! in {} names no interpreter skit knows. What is it?",
        "{} 中的 #! 指定了 skit 不认识的解释器。它是什么？",
        "{} 中的 #! 指定了 skit 不認識的直譯器。它是什麼？",
    ),
    row!(
        "What is {}? skit can't tell from the name.",
        "{} 是什么？skit 无法从名称判断。",
        "{} 是什麼？skit 無法從名稱判斷。",
    ),
    row!(
        "A program (run it directly)",
        "一个程序（直接运行）",
        "一個程式（直接執行）",
    ),
    row!(
        "A prompt for an AI agent",
        "给 AI agent 的提示词",
        "給 AI agent 的提示詞",
    ),
    row!(
        "💡 {} are written directly inside the code, so skit can't turn them into form fields. To manage one, first give it a name at the top of the script, e.g. OUTPUT = '…' (Ctrl+E edits it now).",
        "💡 {} 直接写在代码中，因此 skit 无法将其转换为表单字段。要管理其中一个，请先在脚本顶部为其命名，例如 OUTPUT = '…'（Ctrl+E 可立即编辑）。",
        "💡 {} 直接寫在程式碼中，因此 skit 無法將其轉換為表單欄位。若要管理其中一個，請先在指令稿頂端為其命名，例如 OUTPUT = '…'（Ctrl+E 可立即編輯）。",
    ),
    row!(
        "Detected {} placeholders — probably not written for insertion. Tick only the ones you need, or untick the switch above.",
        "检测到 {} 个占位符——它们可能并非用于插入。请仅勾选需要的项目，或取消勾选上方开关。",
        "偵測到 {} 個預留位置——它們可能並非用於插入。請僅勾選需要的項目，或取消勾選上方開關。",
    ),
    row!(
        "Choose prompt variables",
        "选择提示词变量",
        "選擇提示詞變數"
    ),
    row!("type to filter…", "输入以筛选…", "輸入以篩選…"),
    row!("Select all variables", "选择所有变量", "選擇所有變數"),
    row!("Done", "完成", "完成"),
    row!("Toggle", "切换", "切換"),
    row!(
        "Your entries will appear here.",
        "你的条目会显示在此处。",
        "你的項目會顯示在此處。",
    ),
    row!("{}/{} entry", "{}/{} 个条目", "{}/{} 個項目"),
    row!("{}/{} entries", "{}/{} 个条目", "{}/{} 個項目"),
    row!("Discard", "放弃更改", "捨棄變更"),
    row!("Keep editing", "继续编辑", "繼續編輯"),
    row!(
        "Discard unsaved changes?",
        "要放弃未保存的更改吗？",
        "要捨棄未儲存的變更嗎？",
    ),
    row!("Insert a run-time value", "插入运行时值", "插入執行階段值",),
    row!("Environment variable", "环境变量", "環境變數"),
    row!(
        "Insert a file or folder",
        "插入文件或文件夹",
        "插入檔案或資料夾",
    ),
    row!(
        "The entry's working directory is missing — starting here instead.",
        "条目的工作目录不存在——改为从此处开始。",
        "項目的工作目錄不存在——改為從此處開始。",
    ),
    row!(
        "Runner picked on the run form",
        "执行器在运行表单上选择",
        "執行器在執行表單上選擇",
    ),
    row!(
        "{} (no longer configured)",
        "{}(已不在配置中)",
        "{}(已不在設定中)",
    ),
    row!("Runs with {}", "以 {} 运行", "以 {} 執行"),
    row!("Parameters  {}", "参数  {}", "參數  {}"),
    row!("Presets  {}", "参数组合  {}", "參數組合  {}"),
    row!("Depends on  {}", "依赖  {}", "依賴  {}"),
    row!(
        "Last run  {} · {}",
        "上次运行  {} · {}",
        "上次執行  {} · {}"
    ),
    row!("just now", "刚刚", "剛剛"),
    row!("{} min ago", "{} 分钟前", "{} 分鐘前"),
    row!("{} h ago", "{} 小时前", "{} 小時前"),
    row!("{} d ago", "{} 天前", "{} 天前"),
    row!("finished", "完成", "完成"),
    row!("failed (code {})", "失败（代码 {}）", "失敗（代碼 {}）"),
    row!("Not run yet", "还没运行过", "還沒執行過"),
    row!(
        "The script changed — skit checks the form against it before every run.",
        "脚本改过了——每次运行前 skit 会自动核对表单。",
        "腳本改過了——每次執行前 skit 會自動核對表單。",
    ),
    row!(
        "Press a to add the first one,",
        "按 a 添加第一个，",
        "按 a 加入第一個，",
    ),
    row!(
        "or run: skit add <path> in a terminal.",
        "或在终端运行 skit add <路径>。",
        "或在終端執行 skit add <路徑>。",
    ),
    row!("Back to list", "返回列表", "回到清單"),
    row!(
        "Press Ctrl+C again to quit",
        "再次按 Ctrl+C 退出",
        "再次按 Ctrl+C 結束",
    ),
    row!(
        "The runner row changed before it could be saved; inspect again.",
        "运行器行在保存前已更改；请重新检查。",
        "執行器資料列在儲存前已變更；請重新檢查。",
    ),
    row!(
        "The prompt pins changed before the runner could be removed; inspect again.",
        "提示词固定项在删除运行器前已更改；请重新检查。",
        "提示詞固定項在移除執行器前已變更；請重新檢查。",
    ),
    row!(
        "Your original file will not be deleted.",
        "你的原始文件不会被删除。",
        "你的原始檔不會被刪除。",
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
    // Hong Kong, Macau, and Taiwan use Traditional Chinese.
    if normalized == "x-pseudo" {
        Locale::Pseudo
    } else if normalized.starts_with("zh-tw")
        || normalized.starts_with("zh-hk")
        || normalized.starts_with("zh-mo")
        || normalized.starts_with("zh-hant")
    {
        Locale::ZhTw
    } else if normalized == "zh"
        || normalized.starts_with("zh-")
        || normalized.starts_with("zh.")
        || normalized.starts_with("zh@")
    {
        // Simplified Chinese is the default for the Chinese macrolanguage. Mainland China,
        // Singapore, and Malaysia use Simplified Chinese. A bare "zh" tag, and any Chinese tag
        // with no Traditional hint, also use Simplified Chinese. The Traditional branch runs
        // first, so an explicit script subtag wins over a region subtag.
        Locale::ZhCn
    } else {
        Locale::En
    }
}

/// Resolve one locale-precedence candidate.
///
/// An absent, empty, or exact POSIX `C` value does not express a language preference. Other
/// unsupported language families resolve to English and stop the precedence chain.
#[must_use]
pub fn requested_locale(value: Option<&str>) -> Option<Locale> {
    let value = value?;
    if value.is_empty() || value.eq_ignore_ascii_case("c") {
        None
    } else {
        Some(detect_locale(Some(value)))
    }
}

/// Read the platform locale through the maintained operating-system adapter.
#[must_use]
pub fn system_locale() -> Locale {
    sys_locale::get_locales()
        .find_map(|value| requested_locale(Some(&value)))
        .unwrap_or_default()
}

/// Translate one complete source string when it is in the catalog.
#[must_use]
pub fn text<'a>(locale: Locale, english: &'a str) -> Cow<'a, str> {
    let translated = CATALOG
        .iter()
        .find(|row| row.english == english)
        .map_or(english, |row| localized(locale, row));
    if locale == Locale::Pseudo {
        Cow::Owned(pseudoize(translated))
    } else {
        Cow::Borrowed(translated)
    }
}

/// Return the localized human label for a registered entry kind.
///
/// Machine-facing JSON keeps the open-ended raw kind. A kind written by a newer skit also stays
/// raw because this version cannot name it honestly.
#[must_use]
pub fn kind_label<'a>(locale: Locale, kind: &'a str) -> Cow<'a, str> {
    let english = match kind {
        "python" => "Python",
        "shell" => "Shell",
        "fish" => "fish",
        "js" => "JavaScript",
        "ts" => "TypeScript",
        "powershell" => "PowerShell",
        "ruby" => "Ruby",
        "perl" => "Perl",
        "lua" => "Lua",
        "r" => "R",
        "exe" => "Program",
        "command" => "Command",
        "prompt" => "Prompt",
        _ => return Cow::Borrowed(kind),
    };
    text(locale, english)
}

/// Return the localized ask-face choice label for a registered entry kind.
///
/// The interpreted kinds keep their [`kind_label`] names. The exe and prompt choices
/// answer the picker's question about one unclassified file, so they describe the entry
/// instead of naming a language.
#[must_use]
pub fn kind_choice_label<'a>(locale: Locale, kind: &'a str) -> Cow<'a, str> {
    match kind {
        "exe" => text(locale, "A program (run it directly)"),
        "prompt" => text(locale, "A prompt for an AI agent"),
        _ => kind_label(locale, kind),
    }
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
    if locale == Locale::Pseudo {
        return pseudoize(english);
    }
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
        Locale::Pseudo => row.english,
    }
}

fn pseudoize(source: &str) -> String {
    let characters = source.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(source.len().saturating_add(6));
    output.push('⟦');
    let mut index = 0;
    while index < characters.len() {
        if characters[index] == '%' {
            if characters.get(index.saturating_add(1)) == Some(&'(')
                && let Some(close) = characters[index.saturating_add(2)..]
                    .iter()
                    .position(|character| *character == ')')
                    .map(|offset| index.saturating_add(2).saturating_add(offset))
                && characters
                    .get(close.saturating_add(1))
                    .is_some_and(|specifier| matches!(specifier, 's' | 'd' | 'r'))
            {
                for character in &characters[index..=close.saturating_add(1)] {
                    output.push(*character);
                }
                index = close.saturating_add(2);
                continue;
            }
            if characters
                .get(index.saturating_add(1))
                .is_some_and(|specifier| "sdrifgeExXoc%".contains(*specifier))
            {
                output.push('%');
                output.push(characters[index.saturating_add(1)]);
                index = index.saturating_add(2);
                continue;
            }
        }
        output.push(match characters[index] {
            'a' => 'à',
            'e' => 'é',
            'i' => 'î',
            'o' => 'ö',
            'u' => 'û',
            'A' => 'À',
            'E' => 'É',
            'I' => 'Î',
            'O' => 'Ö',
            'U' => 'Û',
            character => character,
        });
        index = index.saturating_add(1);
    }
    output.push_str("~~⟧");
    output
}
