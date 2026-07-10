# skit UX 重設計 — 資訊架構與介面規格

狀態：**八個介面全部定案並已實作（2026-07-09）**。本文件自足；每節定案標記 ✅。

## 實作狀態（2026-07-09）

已落地：`tokens.py`（值 token 引擎）、`argspec.py`（argparse 靜態讀取）、`flows.py`（統一表單層：plan/prefill/validate/assemble/save_after_run）、`promptform.py`（逐行渲染器）、`inlineform.py`（CLI 迷你表單，Textual inline mode）、`theme.py`（skit-claude 主題：ansi_default 背景跟隨終端＋陶土橘 #D97757 強調）、`tui.py`＋`tui_form/tui_add/tui_settings/tui_prefs/tui_health`（全部畫面）、`cli.py` v2（§8 全表含退出碼 125/126/127、--dry-run、config KEY VALUE、動態補全）、i18n 三語目錄全同步。門檻：ruff/ty/pytest（1216）全綠；targeted mutmut 新 headless 模組 138→57 倖存。

第二波已落地（2026-07-09）：表單「▾插入」token 選單（Ctrl+T 或點欄位標籤的 ▾——動態 {cwd} 與固定路徑並列呈現意圖分岔；環境變數子選單可過濾，未設變數也可全名輸入，缺失留到組裝時具名報錯）；**rename**（store.rename：slug 不動、目錄/狀態/組合全保留，腳本設定基本資料區就地改名，衝突就地報錯不半存）；**click/typer 靜態讀取**（argspec.read_cli 統一入口：argparse → click → typer；click 裝飾器 bottom-up 對齊執行期參數順序、is_flag/Choice/nargs=-1；typer 以函式簽名為表單、Option/Argument/Ellipsis 必填、bool 預設 True 誠實降級；click 讀取前先驗 import，避免劫走 typer 的 @app.command）。

**深度審查＋修復波（2026-07-09）**：獨立 Fable agent 全庫審查產出 15 CONFIRMED＋2 PLAUSIBLE bug、9 架構建議、15 規格偏差——**17 個 bug 全數修復**，要點：C2 隔離補洞（`uv run --script` 對無 block 腳本不隔離，`--no-project` 改為無條件）；焦點模型重構（見 §1）；命令模板 placeholder 補 required＋C3（機密遮罩＋落盤剔除＋回溯清除）；argstate「清空」語義（None=不動、空=清除，快照取代合併）；CLI argv 不二次展開（`assemble(expand_extra=False)`）、`--raw` 真 raw；透明行/dry-run 遮罩機密（`masked_args`）；TUI launch 失敗不記幽靈執行、r 路徑印核對結果、整 parser 降級仍開表單（誠實聲明＋逃生欄）；加入面板重掃保留用戶輸入、共用類型推斷；settings Esc 髒檢查＋s 深連結落地；相同 prompt 的 input() multiset 匹配（漂移永不收斂 bug）；取消表單退出碼 130。

**mutation 戰役完成**：`tokens`/`argspec`/`flows` 零倖存（獨立 Opus agent 打底 57→0，click/typer 新碼再 132→0；5 條 tokens 掃描位移變異為 timeout＝無限迴圈被偵測捕獲，已記入 docs/mutation-ledger.md 分類）。

**A1 已落地（2026-07-09）**：`flows.execute(entry, plan, asm, *, emit)` + `flows.transparency_lines` + `RunOutcome` 收攏投遞管線（inject→透明行→run_entry→清理→分類），cli.run 與 tui._execute 現在都只是薄殼（prompt/suspend/exit-code 映射/status 各自處理，投遞完全共用）。順帶抹平一個殘留裝飾差異（`k = v` vs `k = 'v'`），並把 TUI 的 ShimError 訊息升級到與 CLI 一致的 resync 指引。targeted mutmut 新函式零倖存。

**A5 已落地（2026-07-09）**：`ParamSpec.from_candidate` 收攏四處 Candidate→ParamSpec 轉換（cli/tui_add/tui_settings/reconcile）；`store.dir_size`/`store.human_size` 取代 cli/tui_health 兩份重複並補直接單元測試（零倖存）。**A6 已落地（2026-07-09）**：typer `Annotated[int, typer.Option(...)]` 現代寫法（AI 腳本主流）靜態讀取——`_annotated_parts` 拆解、legacy 與 Annotated 共用 `_apply_typer_meta`；typer 助手零倖存。

**A2 短期防陷阱已落地（2026-07-09）**：腳本設定畫面對「本身就是 argparse/click/typer 驅動、尚未納管任何參數」的腳本，不再列出「勾選納管常數」的選項（納管會寫入 [tool.skit] 而 plan_for_entry 偏好注入來源→整張 argparse 表單被蓋掉），改顯示說明。存檔對這類腳本 write_params(text, []) 位元不變、不翻轉來源。審查建議的即時修法。

尚未落地（backlog）：**A2 完整共存**（同一腳本的 [tool.skit] 注入欄位 ＋ argparse flag 欄位並存於一張表單——動核心 FormPlan/assemble/execute 的單來源模型，屬功能而非缺陷，值得使用者定方向）；A8 settings 儲存交易性（六連寫合併）；規格偏差 3/4/7/9/10/11/12（加入面板 Enter 高亮、TUI 來源步寫新腳本選項、Ctrl+S 列值、CLI add 迷你面板、搜尋比對排序、uv 版本探測、語言自名顯示）；健康檢查網路探測。

## 視覺語言（2026-07-09 改版）✅ 定案：btop 風格

使用者裁定：整體美學向 **btop** 靠攏（排版與元件仍依本文件各節）。落地於 `theme.py`（`CHROME_CSS` ＋ theme variables）＋各畫面 CSS：

- **畫布**：維持 `ansi_default` 背景（透明終端直接透出，等同 btop 的 theme_background off）；`ansi=True` 保留終端原生 ANSI 色。但 ansi color system 會把 link/scrollbar/表頭硬塞成藍色（`link-color: ansi_blue`、`scrollbar: ansi_blue`、`datatable--header: ansi_bright_blue`）——全部在 theme variables / CHROME_CSS 覆蓋，畫面上不得出現這批藍。
- **面板**：每個主要區域一個圓角框、標題在框線上（`border: round` ＋ `border_title`），框色沿用 btop 四色系：清單綠 `#3D7B46`、加入橄欖 `#8A882E`、詳情/設定/偏好靛 `#4B44B0`、執行表單/危險確認栗 `#923535`（`$skit-box-*`）。**邊框畫在內容容器上、不畫在 Screen 上**——Screen 有框會位移座標系，pilot 對 dock:bottom footer 的點擊會判 OutOfBounds。
- **選取／游標**：深陶土底 `#5A2D1E` ＋ 亮白字（btop selected_bg 語法），取代舊的整條 accent 橘底（在暗底上可讀性災難、也是使用者點名的「highlight 尤其差」）。
- **footer chip＝一顆按鈕**：`tui_footer.chip()` 單一 `[on #2A211C @click=…]` span 同時載 pill 底色與動作，鍵名 accent、說明常色——鍵與說明不再像兩顆鈕；link 底線關閉（`link-style: none`），hover 整顆變暖底。所有 modal（移除確認、放棄變更、組合命名、說明、token/env 選單）的鍵提示一律用同一套 chip，滑鼠永遠有路。
- **表單排版**：空的 preview/error 列 `display:none`（原本每欄位吃兩行空白＝稀疏怪版）；`RadioSet > RadioButton width:auto`（Textual 預設 1fr 會把兩個選項撒滿整行）；bool 欄位＝無框 Checkbox＋動態 on/off 標籤；欄位 max-width 100。**Static 預設 width:1fr，放進 auto 寬容器會塌成 0 寬**（說明 overlay 曾整個隱形、preset 空狀態提示被擠出畫面）——modal 內 Static 一律 `width:auto`。

## 核心定位重構（2026-07-09）✅ 定案

**北極星原則（使用者原話）：「用戶不應該需要背命令和命令行參數，就算腳本是他們自己寫的。」**

**差異化重述**：skit 的核心不是「把寫死的常數變表單」，而是「**任何腳本都變成一張可戳的表單，無論它怎麼吃輸入**」。注入只是眾多「值送進腳本」的方式之一。

前提認知：現在大多數腳本是 AI 寫的，AI 偏好 argparse ＋ 開頭自帶依賴宣告（PEP 723）。因此 argparse **是主流情況，不是邊緣**。現狀把 argparse 當二等公民（TUI 無法輸入 args，必須 `skit run x -- flags` 並自行背 flag，已於 tui.py 核實）→ 對最常見的現代腳本沒用，必須修。

**統一表單模型**：三種來源都生成同一套 pokeable 表單，只有「值的送法」不同：

| 來源 | 偵測 | 送法 |
|---|---|---|
| 寫死的具名常數 | AST 常數掃描（Layer 2） | 注入臨時副本（原檔不動 A5） |
| `input()` 呼叫 | AST | 注入臨時副本 |
| argparse / click / typer | **靜態讀 add_argument / 裝飾器** | 組成命令列 flag 透傳 |

argparse 靜態讀取：`choices` → ←/→ 選擇器；`action=store_true` → 勾選框；`type=int/float` → 型別欄；`required=True` → 必填標記；`help=` → 欄位提示；`default=` → 預填值。降級：動態建構 / subparser / 自訂型別（default 非字面量）→ 該欄變自由文字欄＋help 提示＋留空用腳本預設，另備「額外參數（透傳）」逃生欄。click/typer 後補。

**文案原則**：標準的名字可以出現（精確、可搜尋），但句子本身要讓不懂該術語的人也讀得懂（腳本可能是 AI 寫的，用戶未必懂自己腳本用的語法）。範例（使用者定稿）：依賴區塊標題用「**腳本開頭已用 PEP 723 語法聲明依賴：**」——點名標準，但後面直接列出「需要 Python 3.9 以上 / 會自動裝：Pillow」這種白話內容，術語不擋理解。參數區塊同理：「skit 看懂了這支腳本接受哪些參數，執行時會給你一張表單」。

### argparse → 表單 對照規格（v1，click/typer 後補）

偵測方式：AST 靜態讀 `add_argument(...)` 呼叫，只認字面量引數；不執行使用者腳本。

| argparse 宣告 | 表單控件 |
|---|---|
| positional（`nargs="+"` / `"*"`） | 文字欄，標「可多個」；含萬用字元時由 skit 自行展開 glob（TUI 下沒有 shell 幫忙展開） |
| `required=True` / 必填 positional | 「必填」標記，空值擋提交 |
| `choices=[...]` | ←/→ 分段選擇器 |
| `action="store_true"/"store_false"` | 勾選框 |
| `type=int` / `type=float` | 數字欄＋即時驗證 |
| `default=<字面量>` | 預填值 |
| `help="..."` | 欄位下的 dim 提示 |
| 欄名 KEY/TOKEN/SECRET/PASSWORD 命中 | 遮罩欄，值不落盤（C3 對所有來源一體適用） |

**降級規則（誠實優於聰明）**：
- 單一欄位讀不懂（自訂 `type=`、default 非字面量、`action="append"` 等）→ 該欄降為自由文字欄＋help 提示＋dim「留空＝用腳本自己的預設」，留空時不傳該 flag。
- 整個 parser 讀不懂（動態建構、subparsers、迴圈生成）→ 表單只剩「額外參數（透傳）」逃生欄＋說明「skit 沒能看懂這支的參數宣告，請直接輸入參數」。
- 任何情況下表單都保留「額外參數（透傳）」欄，補沒被模型化的東西。

**值 token（✅ 2026-07-09 定案）**：執行時求值的佔位符，可嵌在字串中間：`{cwd}`（執行時所在目錄）、`{today}`（YYYY-MM-DD）、`{now}`（HH-MM-SS）、`{env:NAME}`、`~` 展開。每個文字欄右緣「▾ 插入」選單（可點可鍵盤）：第一、二項並列「執行時所在目錄 {cwd}」與「此刻目錄（固定字面值）」——動態意圖 vs 凍結值的分岔明示給使用者選；「環境變數…」列 os.environ 變數名可過濾。欄位下方即時預覽展開結果（灰字）。規則：(1) 上次值/參數組合**存 token 原文不存展開值**（存意圖，與 glob 同構）；(2) 執行前 dim 命令列顯示展開後終值；(3) 逃逸 `{{`。全部 stdlib 實作。

**表單預填模型**：表單永遠預填上次的值（首次＝腳本預設值）；頂部參數組合選擇器的基準 chip 是「上次」，具名組合並列，切換即整組套入（再手改不影響組合本身）。

**r（快速路徑）vs Enter（覆核路徑）**：r 對選中腳本跳過表單、用上次的值直接執行。沒跑過→狀態列提示先按 Enter；r 不跳過執行前核對；上次的值填不滿（腳本新增必填參數）→ 退回開表單並標紅缺欄，絕不默默組壞命令。重跑存的是表單原始值（如 `shots/*.png`），glob 每次執行時重新展開——保留意圖而非凍結檔名清單。

**Ctrl+S 存成參數組合**：表單內彈命名小窗（列出將儲存的值），存後留在表單（儲存與執行不綁定）；名稱衝突就地提示「會覆蓋現有的 X」；機密參數自動剔除並提示（C3）。

**透明規則**：表單提交後、腳本輸出前，dim 顯示 skit 實際組出的命令列。注入路徑的值不在命令列上，格式分兩行：「→ 注入：NAME = '展開後終值'（寫進臨時副本，跑完即刪；你的原始檔不變）」＋「→ 命令列」。首次執行顯示「uv: 安裝 X…（首次執行，之後有快取）」解釋等待。信任來自透明，也讓使用者被動學會腳本用法（北極星是「不用背」，不是「不讓你知道」）。glob 由 skit 展開且在表單內即時回饋命中數（「✓ 符合 3 個檔案」）。

**終端歸屬（C5/C6 保留不動）**：表單收起後子行程直接繼承真實 TTY（launcher 的 subprocess.run 不帶 stdout/stderr/stdin 參數）——輸出即時、顏色/進度條/isatty 全部如常，腳本中途 input() 可正常互動。與 Layer 2 的分工：納管的 input() 在表單先答、注入後執行中不再問；未納管的留在執行時即時問；兩者可共存（「哪些先答、哪些當場答」是 add 時的使用者決定，A4 延伸）。全部 input() 納管後腳本即可 --no-input 自動化（參數管理畫面應提示這點）。**考慮過但不做**：tee 留存輸出——會讓腳本的 stdout 變 pipe（isatty=False，顏色與進度條退化），為留紀錄毀掉直通體驗；「上次執行」只記時間＋exit code。

**組裝規則**：表單裡有值的欄位一律顯式傳入（可重現性優先——腳本作者日後改預設值，preset 行為不漂移）；勾選框只在勾選時傳 flag；降級欄留空不傳；引號安全交給 shlex/list2cmdline（沿用 launcher 現有平台邏輯）。值解析順序與 Layer 2 完全一致：本次輸入 > preset > 上次值 > 腳本宣告的 default。

## 已定案的整體決策（2026-07-09）

1. **TUI 為主體，CLI 為捷徑。** 裸 `skit` 進入完整工作檯（收藏、執行、參數、preset、設定、doctor 全在 TUI 內完成）；CLI 保留給自動化 / SSH / 肌肉記憶。
2. **一份流程，兩種渲染。** 互動流程抽成 headless flow（純狀態機，不碰 stdio），TUI 用 Textual 畫面渲染、CLI 用 prompt 序列渲染。延續 tui.py「presentation only」哲學。
3. **安全原則可視化。** 原檔不動（A5/A7）、secret 不落盤（C3）、注入到臨時副本 —— 從 dim 提示文字升級為 badge / 遮罩欄位 / 詳情面板資訊。
4. **命令面大膽收斂，不留 alias**（pre-release，無相容包袱）：
   - 砍 `skit lang` → 併入 `skit config`。
   - 砍 `config` 三連問精靈 → 無旗標時顯示現況並指向 TUI 設定畫面。
   - `params` 旗標保留（自動化用）；互動式參數編輯走 TUI。
   - `preset` 子命令保留給自動化；互動路徑走 TUI；新增 `run NAME --save-preset X`。
5. **`skit run` 參數表單預設是迷你 TUI 表單**（Textual inline mode，小窗口就地展開，不進 alternate screen、不清 scrollback），與 TUI 內的表單畫面共用同一 flow。逐行問答保留為使用者選項：config `run_form = "plain"` 或 `--plain` 旗標；`TERM=dumb` 自動降級 plain；非 TTY 走非互動契約（不出表單）。
6. **非互動契約不變**：pipe/CI/`--no-input` 下不猜、不問、不默默組壞命令。

## TUI 畫面地圖

```
skit（裸命令）
└─ Library 主畫面（搜尋 + 清單 + 詳情面板）
   ├─ Enter → 執行流程（參數表單畫面 → suspend 直通執行 → 回 Library）
   ├─ a     → 加入面板（來源步 → 單一覆核面板，可回頭；見 §3）
   ├─ p     → 腳本設定畫面（✅ 合併定案：基本資料／參數／參數組合／依賴 四分區）
   ├─ s     → 直達腳本設定的「參數組合」分區（深連結，同一畫面）
   ├─ e     → 編輯腳本（suspend 進編輯器）
   ├─ Del   → 移除 modal
   ├─ ,     → 偏好設定畫面（語言/編輯器/鏡像；改名避免與「腳本設定」撞詞）
   ├─ D     → 健康檢查畫面（doctor）
   └─ ?     → 快捷鍵說明 overlay
```

## 用語表（Apple UX 審查後定稿，2026-07-09；中文介面硬規範）

| 用 | 不用 | 概念 |
|---|---|---|
| 腳本 | 條目、項目 | 收藏的任何東西（總稱） |
| 說明 | 描述 | description 欄位 |
| 存法：複製一份 / 連結原檔 | 來源模式、copy/reference | 收納方式 |
| 參數組合 | preset（中文介面） | 存下來的一組參數值 |
| 參數 | 選項、引數、arguments | 表單收集的一切 |
| 加入 / 移除 | 新增、添加 / 刪除 | 進出收藏（「移除」不動原始檔——用詞承載 A5 信任承諾） |
| 執行 | 運行、跑 | run |
| 核對 | 對帳、漂移、reconcile | 執行前的定義同步 |
| 編輯腳本 | 編輯（單獨出現） | 用外部編輯器開腳本原始碼。「編輯」必須帶受詞——模擬實測：在滿是可編輯欄位的面板裡，裸「編輯」會被讀成「編輯這些欄位」 |
| 腳本設定 / 偏好設定 | 設定（單獨出現） | 腳本設定＝選中腳本的管理畫面（p）；偏好設定＝skit 全域設定（,）。裸「設定」禁用，兩者必撞詞 |
| 健康檢查 | doctor、健檢、檢查 | 環境與腳本庫自檢（D）。「健檢」太壓縮，使用者反饋看不懂（2026-07-09） |

文案定稿（審查產出）：
- 型別提示用「整數／小數／文字／開關」，平時淡化、驗證失敗才顯眼。
- 「額外參數（原樣傳給腳本）」，不用「透傳」。
- 結束橫幅先給結論：「✓ 完成」／「✗ 失敗（代碼 N）」；Library 狀態列同格式。
- 移除確認：「移除 X？你的原始檔不會被刪除。y 移除　Esc 保留」（動詞按鍵，不用 y/n=確認/取消）。
- 空狀態三件套：「你收藏的腳本會出現在這裡」＋「按 a 加入第一個，或在終端執行 skit add <路徑>」。
- 注入失敗指向可修復動作：「腳本內容和表單對不上了：找不到 X。到參數畫面（p）按『重新同步』即可修復。」
- 存法選項每項寫一好處＋一代價（連結原檔須講明「skit 不寫入這個檔案，參數定義要自己維護」）。
- 類型 badge 用文字：Python／程式／命令（`$` 可作命令前綴符），禁用 ⌘（macOS 修飾鍵誤導）。
- 表單欄位標籤＝使用者設的 prompt，未設則顯示原變數名；skit 不翻譯變數名。
- 無障礙硬規則：狀態一律「圖形＋文字」雙載體，不得只靠顏色傳義。
- 加入面板視覺層級：「Enter 加入」是唯一高亮落點（squint test），其餘區塊次級對比。

## 統一互動語彙

- **可發現性假定（使用者定案，2026-07-09）：大多數 TUI 使用者永遠不會按 ?。** 所有操作必須常駐可見——footer 允許雙行（第一行＝作用於選中腳本的操作，第二行＝全域操作）；? 保留但只是輔助，任何功能不得只靠 ? 才能被發現。螢幕空間傾向多放資訊，不追求極簡留白。
- Enter 確認/進入；Esc 返回上一層（到頂才是離開）；雙 Ctrl+C 隨處可退；精靈一律可回上一步。
- 顏色語義：綠=成功、黃=漂移/可修復警告、紅=錯誤、dim=輔助說明。
- 破壞性動作一律 modal + y/n；其餘不設確認關卡。

## CLI 命令面（收斂後）

`add` `run` `list` `remove` `edit` `params` `preset` `deps` `doctor` `config` — `lang` 移除。

---

## 介面規格（逐一設計中）

### 0. 加入面板・候選信號與提示規格

**Meta 規則：UI 上每一句智能提示必須對應一條有名字的、確定性的 AST 規則；措辭自信度不得超過規則自信度。全部 stdlib ast，不引依賴、不碰 LLM。**

| 規則 | 偵測 | UI 效果 |
|---|---|---|
| 累加器降級 | 字面量初值＋他處 AugAssign 或迴圈內重賦（小寫命名為輔助信號） | 預設不勾＋⚠「看起來是迴圈的累加變數」 |
| 行內檔名提示 | 像檔名的字串字面量（副檔名 regex）直接當呼叫引數、未具名 | 💡 建議抽成具名常數（最多列 2–3 個）。**做不到的不許提示**（如 'RGB' 這種需領域知識的，已明確排除） |
| argv 提示 | AST 出現 sys.argv 下標/切片 | ℹ「會吃命令列參數，表單有額外參數欄」 |
| argparse 欄位數 | 靜態讀 add_argument 字面量 | 「看懂了…共 N 項」 |
| uv 版本探測 | 開面板時 `uv python find`（timeout＋快取） | 「自動（目前 3.13）」 |

**勾選預設跟著信號走**：乾淨候選（全大寫、賦值一次、未被改動）→ 預設勾；有降級信號 → 預設不勾＋說明原因。

**e（加入面板內）**：條目尚未加入，e 開啟的是使用者原始檔（明示「原始檔」；是使用者在自己編輯器改自己的檔，A5 無涉）；編輯器關閉返回後面板**當場重掃**，候選/提示即時更新（編輯→回來→重掃迴路）。

### 1. Library 主畫面 — ✅ 定案

- 版面：搜尋列＋左清單＋右側詳情直欄（窄於 80 欄自動收合、Tab 開關）＋狀態列＋按鍵列。
- **焦點模型（2026-07-09 深度審查後修正）**：表格為預設焦點、單字母鍵＝動作、`/` 進入搜尋（框內所有字母照常打字、上下鍵仍驅動清單、Enter 執行最上匹配並回表格、Esc 回表格）。原「type-to-search 常駐 focus」與單鍵操作在同一鍵盤上不可共存（Input 吞掉可列印按鍵，footer 廣告的鍵全部失效＝審查 B2）——k9s/lazygit 式取捨。**制度化教訓：footer 每宣告一個鍵，必須有一條正向 pilot 測試。**
- 排序：**最近活動優先**（max(加入時間, 上次執行)——剛加入的要馬上跑，不能因沒跑過而沉底）；搜尋時改用比對排序。加入完成後游標落在新條目上。
- 清單欄位：名稱＋類型 badge（⬡ py / ▶ exe / ⌘ cmd，reference 另標）＋健康 glyph。描述移入詳情面板。
- 健康狀態兩級：清單 ⚠ 只做便宜檢查（目標檔消失）；漂移檢查（讀檔＋reconcile）選中時懶算、按 mtime 快取，顯示於詳情面板。
- 詳情面板＝安全原則展示位：copy「✓ 原始檔不會被 skit 修改」、reference「↗ 連結原檔：路徑」、secret 一律 `•••🔒`；另列參數摘要（含上次值）、preset、依賴、上次執行（時間＋exit code）。
- 空狀態：歡迎卡片＋「按 a 加入第一個腳本」。
- 文案審查修訂（2026-07-09 第二輪）：頂欄「skit · 腳本庫」；搜尋 placeholder「搜尋名稱或說明…」；狀態列只報結果不帶按鍵提示（r 提示與「作用於選中者」語義矛盾，已拆）；「Python · 副本由 skit 保管」。
- Footer（可發現性假定定案後）：**雙行全鍵常駐**——第一行（選中腳本）「Enter 執行 · r 重跑（情境鍵，跑過才出現）· p 腳本設定 · e 編輯腳本 · Del 移除」；第二行（全域）「a 加入腳本 · s 參數組合 · , 偏好設定 · D 健康檢查 · ? 說明」。
- ✅ 已定案（使用者拍板「合併」）：p/s/deps/名稱說明編輯合併為「腳本設定」畫面（見 §4）。
- 執行後：狀態列顯示 `上次：<名稱> ✓ 完成`／`✗ 失敗（代碼 N）`（失敗黃色）；重跑走 footer 的 r 情境鍵。
- 資料需求：argstate 新增 `last_run_at`、`last_exit_code`。

### 2. 執行流程 + 參數表單 — ✅ 定案

同一個 headless run flow（解析參數 → 表單 → 驗證 → 送值 → 執行 → 記錄），三種呈現：

| 情境 | 呈現 |
|---|---|
| TUI 內 Enter | 全尺寸表單畫面（Textual Screen，Esc 回腳本庫） |
| 終端 `skit run NAME` | 迷你 TUI 表單（Textual inline mode：就地展開、不進 alternate screen、不清 scrollback） |
| `--plain`／config `form="plain"`／`TERM=dumb` | 逐行問答（同一 flow 的 prompt 渲染） |
| 非 TTY／`--no-input` | 不出表單（非互動契約） |

**畫面解剖**（標題「執行 <名稱>」——目的優先，不用「<名稱> ─ 參數」）：
1. 參數組合列：基準 chip「上次」＋具名組合 chips，←/→ 或滑鼠切換，切換即整組套入；空狀態「（還沒有——填好後按 Ctrl+S 可存一組）」。
2. 欄位區（來源見「統一表單模型」；控件對照見 argparse 規格表）：標籤＝prompt 或原變數名；欄下 dim 提示＝腳本作者的 help 原文（skit 不翻譯）；型別提示「整數／小數／文字／開關」平時淡化；必填空值擋提交；型別即時驗證（紅框＋「X 需要整數，你輸入的是 "abc"」）；glob 即時回饋「✓ 符合 N 個檔案」；機密欄遮罩＋「不會存檔」＋（若設）「將從 $NAME 讀取」；文字欄右緣「▾ 插入」token 選單＋欄下展開預覽；降級欄＝自由文字＋「留空＝用腳本自己的預設」。
3. 「額外參數（原樣傳給腳本）」欄（永遠存在），預填上次的 extra args。
4. 頂部黃色核對橫幅（僅有 drift 時）：「已略過消失的 X／TYPE 已變的 Y」。
5. Footer：「【Enter 執行】 Ctrl+S 存成參數組合 Esc 取消」。

**執行段**：表單收起 → dim 透明行（argparse＝組裝後命令列；注入＝「→ 注入：NAME='終值'（寫進臨時副本，跑完即刪；你的原始檔不變）」＋命令列；首次「uv: 安裝 X…」）→ TTY 直通（見終端歸屬）→ 結束橫幅「✓ 完成」／「✗ 失敗（代碼 N）」＋「按 Enter 返回」（TUI 路徑）→ 回腳本庫：狀態列更新、r 亮起、argstate 寫入（token/glob 原文、extra args、last_run_at、last_exit_code；機密剔除）。臨時副本 finally 必刪。

其餘規則（值 token、預填模型、r vs Enter、Ctrl+S、透明、終端歸屬、組裝）見文件前段各定案塊。

### 3. 加入面板 — ✅ 定案

**入口**：TUI `a` → 先出「來源」步（路徑輸入〔含 ~ 展開與補全〕／寫一支新腳本〔開編輯器〕／登記一條命令〔模板＋placeholder 預覽〕）→ 覆核面板。CLI `skit add PATH` 直接開在覆核面板（迷你表單）；`add -` 從 stdin；`-e` 先開編輯器再進面板；`--no-input`/非 TTY 不開面板、逕採偵測建議（誠實契約）。類型自動推斷（.py→Python、執行位→程式），`--exe` 僅為覆寫。

**覆核面板**（單一面板、常駐編輯狀態、可回頭改；「【Enter 加入】」是唯一高亮落點）：
- 名稱（預設檔名 stem）／說明（預設 docstring 首行，「← 從腳本開頭的說明文字抓來的」；無則「（腳本開頭沒有說明文字，可以自己寫一句）」）。
- 存法單選，各寫一好處＋一代價（複製一份「skit 保管副本，你的原始檔永遠不會被改動」／連結原檔「改動原檔立即生效；但 skit 不寫入這個檔案，參數定義要自己維護」）。
- 依賴區兩型態：腳本自帶宣告 → 唯讀「腳本開頭已用 PEP 723 語法聲明依賴：」＋白話列點；否則 → 逐列可編輯清單（✕ 移除／＋ 新增）＋「Python [自動]（uv 會挑符合的最新版·目前 X）」。
- 參數區三型態：argparse → 「✓ skit 看懂了這支腳本接受哪些參數（…共 N 項）。執行時會給你一張表單，不用記任何指令。」；常數/input() 候選 → 勾選清單（信號驅動預設與 ⚠/💡/ℹ 提示，規格見 §0；型別用中文；Space 勾選）；讀不懂 → 誠實聲明＋指出額外參數欄仍可用。
- Footer：「【Enter 加入】 Space 勾選 e 編輯腳本 Esc 取消」。`e`＝開原始檔（明示），返回後**當場重掃**（§0 迴路）。
- 完成 → 回腳本庫，游標落在新條目，狀態列「✓ 已加入 X」。

### 4. 腳本設定畫面 — ✅ 定案（p 進入；s 深連結到參數組合分區）

單一畫面四分區（Tab／滑鼠移動；Enter 儲存變更＝一次原子寫入 `[tool.skit]`；Esc 有未存變更彈「放棄變更？」）：

1. **基本資料**：名稱／說明就地編輯（填補「加入後無處改」缺口）；存法與來源路徑唯讀展示。
2. **參數**：納管勾選清單（含未納管候選＋信號提示，同 §0）；每列展開「欄位提示」小輸入框（取代 --prompt 旗標湯）＋「機密」勾選（勾選當下就地提示「skit 之前記住的這個值也會一併刪掉」；機密規格見下塊）；「↻ 重新同步——腳本改版後，核對這些定義還對不對得上」，跑完就地報告；提示「全部 input() 都納管後，這支可以用 --no-input 自動化」。
3. **參數組合**：每組列值摘要，可改名／刪除；套用不在此（在執行表單）；空狀態指路 Ctrl+S。
4. **依賴**：逐列編輯＋Python 約束（同加入面板型態）。

**條目類型變體**：連結原檔 → 參數分區唯讀＋「skit 不寫入這個檔案——參數定義請直接在檔案裡維護」；命令模板 → 參數分區顯示模板與 placeholder；程式 → 僅基本資料分區。

**機密參數規格（2026-07-09 對話定調）**：
- 三效果：輸入遮罩／值永不落盤（上次值＋參數組合皆剔除）／對執行零影響（腳本照常拿到值）。
- 勾選框＝使用者的選擇權：名字含 KEY/TOKEN/SECRET/PASSWORD 只是預設打勾，可取消（取消＝普通參數，照常記住）。預設不存的理由：狀態檔是明文 TOML（備份/dotfile 同步/螢幕分享風險），skit 不是密碼管理器。
- 文案修正：「已存的明文值會被清除」→ 勾選當下就地提示「skit 之前記住的這個值也會一併刪掉」（回溯清除只在「後來才標機密」情境發生）。
- **✅ 定案：機密值來源選項**——「每次執行時：問我 ／ 從環境變數讀 [NAME]」。skit 存變數名不存值：不重打、零明文、零依賴。鑰匙圈（keyring）視為 v2+，不進 v1。
- 誠實邊界（文件照實寫）：保證「靜態不落盤」，非執行瞬間隱形——注入路徑短暫存在於臨時副本（跑完即刪）、argparse 路徑出現在該行程命令列（ps 可見）。
### 6. 偏好設定畫面 — ✅ 定案

- 單一畫面全部可見（不搞連問精靈）：介面語言／編輯器／互動表單（迷你表單 vs 逐行問答）／下載鏡像（中國大陸加速）。
- 介面語言用**下拉選單**（Textual Select；使用者指示：語言清單會變長，分段選擇器不可擴充）。首項「自動·跟隨系統」，各語言以自己的語言顯示；清單過長時加打字過濾。
- 設定改名「互動表單」（原「執行表單」/run_form）：同一機制也管加入面板等所有互動流程；config 鍵為 `form = "tui" | "plain"`。
- 每項自帶「目前實際生效什麼」：語言顯示「目前生效：X」；編輯器留空時顯示 fallback 實際解析結果（「目前是 vim」，當場讀 $VISUAL/$EDITOR）。
- 語言選項以各自語言顯示（English 寫 English）；儲存後立即套用、介面當場換語言。
- 鏡像「自訂」就地展開三 URL 欄（PyPI／Python 下載／uv 主程式）；uv 欄行內強制 https（紅字「uv 主程式會被下載執行，鏡像必須是 https://」）。
- 儲存語義同腳本設定：Enter 儲存、Esc 未存變更先問。
- CLI 對應（§8 v2 定稿為準）：`skit config [KEY [VALUE]]` git-config 文法；`form` 是其中一個 KEY。

### 7. 健康檢查畫面 — ✅ 定案（Library footer 為「D 健康檢查」）

- 逐項清單：uv 可用（版本＋路徑）／腳本庫索引（registry↔meta.toml 對照）／失聯的腳本（target_missing）／表單定義過期（全庫批次 reconcile——唯一主動全掃處）／下載鏡像狀態（含阻塞探測）；尾行「庫位置＋數量＋大小」。
- 每個 ⚠ 是可選中行，Enter 跳到該腳本（Library 選中）；R 重建索引（=doctor --rebuild）就地刷新。
- 檢查項只列既有能力可查的，不發明。CLI `skit doctor` 輸出同清單文字版，退出碼不變。

### 8. CLI 命令面 — ✅ 定案（v2，2026-07-09 依頂級 CLI 原則重寫）

10 命令（lang 已併入 config）。互動路徑全走共用表單層（偏好 `form=tui|plain` ＋各命令 `--plain`）；非互動契約原樣。

| 命令 | 旗標 | 備註 |
|---|---|---|
| `skit` | `-V/--version` | 開 TUI |
| `add [PATH\|-]` | `-n -d --ref --cmd -e/--edit --dep（可重複） --python --no-input --plain`；`--exe` 降為罕用覆寫 | **類型自動推斷**：.py→python、有執行位→程式，不再要求 --exe；`-`＝從 stdin 收腳本（`pbpaste \| skit add -`）；TTY 開加入面板 |
| `run NAME [-- args]` | `-p/--preset --save-preset --plain --no-input --raw --dry-run（新）` | `--dry-run`＝印出 token 展開＋組裝後的最終命令即退（透明原則的除錯器） |
| `list` | `--json` | JSON 增列 last_run_at／last_exit |
| `remove NAME` | `-y` | 確認文案含「原始檔不會被刪除」 |
| `edit NAME` | — | 不存在時（限 TTY）提議新建 |
| `params NAME` | `--manage --unmanage（原 --add/--remove，避免與頂級命令撞詞、且更誠實——只能納管偵測到的候選） --secret --no-secret --prompt NAME=text --env-source NAME=ENVVAR --resync --json（讀取時）` | 自動化保留；互動走 TUI 腳本設定 |
| `preset save/list/delete` | save 增 `--from-last`；list 增 `--json` | 自動化用；互動走表單 Ctrl+S |
| `deps NAME` | `--dep PKG（可重複，取代逗號分隔的 --set） --clear（顯式清空，不用空字串魔法） --python --json` | 修逗號劈裂 bug（見下） |
| `doctor` | `--rebuild --json` | 輸出=健康檢查清單文字版 |
| `config [KEY [VALUE]]` | `--json` | **git config 文法**：裸=列全部、KEY=讀、KEY VALUE=寫（`skit config form plain`）。--show/--lang/--editor/--mirror/--form 全砍 |

**全域合約**：
- 退出碼（docker 慣例）：`run` 透傳腳本退出碼且保持純淨；skit 自身錯誤=125、目標不可執行=126、找不到=127、用法錯誤=2。其他命令：0 成功、1 失敗、2 用法錯誤。
- 動態補全：`run/edit/remove/params/deps <TAB>` 補腳本名；`-p <TAB>` 補該腳本的組合名。
- 每個 `--help` 帶 1–2 行實例（epilog）；尊重 `NO_COLOR`（rich 原生支援）。
- 值內不發明語法：清單一律可重複旗標，不用逗號拼接。

> **🐛 逗號劈裂 bug——✅ 已修（2026-07-09）**：`deps --set`／`add --deps`／互動依賴問答曾用 `split(",")` 劈依賴清單，PEP 508 含逗號的版本約束（`"requests>=2,<3"`）會被劈成兩個壞項。現由 `pep723.split_requirements()` 處理（括號/引號/接續運算子感知），三個呼叫點全換；測試 `tests/test_pep723_split.py`（targeted mutmut 零倖存，兩個 falsy-equivalent 哨兵照慣例 pragma 並記入 docs/mutation-ledger.md）。
