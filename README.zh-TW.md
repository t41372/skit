<img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png" alt="skit — 腳本啟動器 + 參數管家" width="750">

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/t41372/skit?utm_source=badge)
[![Mutation tested: cargo-mutants](https://img.shields.io/badge/mutation%20tested-cargo--mutants-blue)](https://github.com/sourcefrog/cargo-mutants)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Rust 1.97.1](https://img.shields.io/badge/rust-1.97.1-orange)](https://www.rust-lang.org/)
[![TUI: Ratatui](https://img.shields.io/badge/TUI-Ratatui-blue)](https://ratatui.rs/)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

[English](./README.md) | **繁體中文** | [简体中文](./README.zh-CN.md)

**skit 是命令列裡的腳本管理器與啟動器。**

skit 把你的腳本集中收在一處，啟動也毫不費力——支援 Python、shell、JS/TS、可執行檔、提示詞等等。

skit 會讀你的腳本，把命令列旗標、`input()`、寫死的常數這類東西變成一個帶說明的啟動選單——輸入和變數都在畫面上改，不用動腳本。

於是你再也不用擔心明年找不到、或想不起來怎麼用自己（或 AI）寫的那支腳本——丟進 skit 就好，之後要跑隨時都輕鬆。

要記的命令就兩條：

```bash
skit add script.py   # 把腳本收進庫裡
skit                 # 打開選單——選、填、跑
```

不熟命令列？沒關係——skit 就像一個剛好住在命令列裡的 GUI 應用：滑鼠點一點、打打字，所有操作都能完成。快捷鍵也有——提示就在畫面上——但沒有什麼需要學或背的。

你的 AI agent 也能用 skit：你從選單操作，agent 走確定性的 CLI 和 skill——AI 寫完的腳本存進去，之後隨時都能再拿出來用。

<video src="https://github.com/user-attachments/assets/5899c4f2-a65d-4a22-b386-4ed24a62cdce" controls></video>

## 它做什麼

- **收納腳本與提示詞**。`skit add` 把散落各處的腳本與提示詞收進同一個可搜尋的工具庫。
- **不用背旗標，也不用為了改個值開編輯器**。旗標、`input()`、你選擇管理的常數，全部變成啟動選單裡有型別、有說明的欄位。Ratatui 表單會在送出時驗證布林值、選項、數字與路徑。
- **記住你上次填的值**。啟動選單裡的參數下次會自動帶回；清除覆寫值即可使用腳本目前的預設值。常用的存成具名預設——`{cwd}`、`{today}` 這類 token 讓預設跨機器、跨目錄通用。標記為機密的參數永不保存：上次的值、預設、執行歷史裡都不會有它。
- **環境零污染**。Python 腳本的依賴以 PEP 723 語法聲明在腳本開頭，由 uv 在隔離環境裡解析；JS/TS 腳本則有按腳本隔離的 `node_modules`，首次執行時依宣告的套件自動安裝。兩者都不往全域裝任何東西。其他語言沿用你機器上已有的工具——skit 會在執行前檢查腳本聲明的外部命令是否在 `PATH` 上。
- **提示詞當腳本用**。存一份帶參數的提示詞（管理中的 `{{placeholders}}` 變成輸入欄位），交給你的 coding agent 啟動——claude、codex、opencode，或設定任何你喜歡的執行器。
- **滑鼠鍵盤皆可，多語言支援**。直接執行 `skit` 就是完整 TUI；畫面上每個按鍵提示同時也是可點的按鈕。介面有 English、繁體中文、简体中文（[語言](#語言)）。
- **也為 AI agent 而生**。每個 TUI 動作都有對應的 CLI 命令，帶 `--json` 輸出與明確退出碼；官方 [Agent Skill](https://agentskills.io) 教 Claude Code、Codex、Cursor、Gemini CLI 等 agent 先查庫、用現成的、把好用的收進庫——見[給你的 AI agent 用](#給你的-ai-agent-用)。

| 痛點 | skit 的解法 |
| --- | --- |
| 腳本東一支西一支，散落在各個資料夾 | 全部收進同一個選單，附搜尋 |
| 腳本需要特定套件或工具 | Python（PEP 723 + uv）和 JS/TS（npm）都有按腳本隔離的依賴；任何語言都可宣告外部命令，skit 在執行前檢查是否在 `PATH` 上 |
| 命令列旗標轉頭就忘、`input()` 一項項問、常數寫死在原始碼裡等著你手改 | 靜態分析把參數通通讀出來，變成互動選單——原始碼一行不動、零設定。上次的值自動帶回；常用的存成預設 |
| AI 幫你寫的腳本隨對話結束石沉大海，下次又重寫一遍 | agent 先查庫再動手，現成的直接重用；值得留的收進庫裡——一次性腳本變成永久的、參數化的工具 |

不需要為了 skit 特地改你的腳本——這些交給我們處理就好，需要時也會問過你。

## 支援的腳本類型

Python、shell、JS/TS 支援最完整：skit 直接讀程式碼找出參數。其他類型開箱即可啟動。

| 腳本類型 | 執行方式 | 支援的參數偵測 |
| --- | --- | --- |
| **Python** | uv（`uv run --script`） | 命令列旗標（argparse · click · typer）、`input()`、常數 |
| **Shell**（bash/sh/zsh） | 對應的 shell | 命令列旗標（getopts）、`read`、常數、`${VAR:-}` 預設值 |
| **JS / TS** | 依序找 deno、bun、node | 命令列旗標（`util.parseArgs`）、`const` 值 |
| **fish** | fish | 命令列旗標（`argparse`）、`set -q` 環境預設值 |
| **PowerShell** | pwsh | `param()` 定義 |
| **Ruby · Perl · Lua · R** | 各自的直譯器 | — |
| **可執行檔** | 直接執行 | — |
| **命令範本** | skit 填好空格後執行 | — |
| **提示詞** | 你的 coding agent（claude · codex · …） | `{{placeholders}}` |

你的類型沒有自動偵測？手動宣告參數就好——每種類型都享有同樣的啟動選單 / 預設 / `--set` 體驗，連純可執行檔也一樣（宣告的值會以一般命令列參數傳入）。任何條目還能列出它依賴的外部命令（`ffmpeg`、`jq`⋯⋯）；skit 每次執行前都會檢查它們是否在 `PATH` 上。

Python 和 JS/TS 都有按腳本隔離的依賴套件：uv 解析 PEP 723 區塊，npm 式依賴則安裝到存副本旁的 `node_modules`——安裝一律不執行套件的 lifecycle scripts。更細的部分（複製 vs 引用條目、deno 的 `--allow-all`）見[指令稿類型文件](https://t41372.github.io/skit/zh-TW/docs/script-types/)。

skit 會替 Python 引導 uv，但不會替你裝 JS runtime——node、bun 或 deno 得你自己備好。

### 提示詞

提示詞條目是給 AI coding agent 的、可重複使用且帶參數的一段文字。加入一個 `.prompt.md` 檔，或用 `skit add --prompt` 直接起草。skit 會把最多 30 個偵測到的 `{{placeholders}}` 設為管理中的輸入欄位。偵測數量超過 30 個時，skit 預設不管理任何欄位，以免把程式碼範例誤認為變數。可在項目設定或用 `skit params` 選擇要管理的欄位。管理中的欄位完整享有預設、上次值與 `--set` 體驗。

沒有任何轉義規則要學：凡是你沒交給 skit 管理的內容——包括沒管理的 `{{holes}}`——都會逐位元組原樣送達 agent，唯一例外是下文說明的 Pi 警告式單換行降級；每個提示詞還有插值總開關（`--no-interpolate`）。**執行器**在啟動選單上選（或按提示詞釘選）；claude / codex / opencode / amp / antigravity / copilot / cursor / pi 已預先設定，其他 CLI 用 `skit runner add` 註冊。一個誠實的提醒：提示詞不是保密通道——渲染後的內容會落在對方 agent 自己的 session 紀錄裡。各執行器的行為、非互動時的解析規則、Pi 對歧義開頭的警告式降級，以及無 shell 的遞送保證：見[提示詞文件](https://t41372.github.io/skit/zh-TW/docs/prompts/)。

```bash
skit add review.prompt.md            # 管理中的佔位符變成輸入欄位
skit run review                      # 選 agent、填好輸入、出發
skit run review --runner codex --set target=src/app.py --no-input
```

## 安裝

skit 使用 [uv](https://docs.astral.sh/uv/) 0.12.3。如果沒有安裝 uv，skit 會先徵求你的同意，再把這個版本下載到自己的私有目錄。skit 不會改動你的 `PATH` 或全域環境。仍建議參考[官方文件](https://docs.astral.sh/uv/getting-started/installation/)在系統中安裝 uv。

```bash
# 用 uv tool 從 PyPI 安裝 skit（套件名是 skit-cli，裝好的指令是 skit）
uv tool install skit-cli
```

> **人在中國大陸？**這一步 skit 還沒裝上、沒法替你設定，請手動讓 uv 指向鏡像（詳見[中國大陸](#中國大陸)）：
>
> ```bash
> export UV_DEFAULT_INDEX=https://pypi.tuna.tsinghua.edu.cn/simple
> uv tool install skit-cli
> ```

或者，從 main 分支直接安裝開發版本

```bash
uv tool install git+https://github.com/t41372/skit          # 最新開發版
uvx --from git+https://github.com/t41372/skit skit --help   # 或是什麼都不裝，直接試
```

## 更新

```bash
uv tool upgrade skit-cli   # 更新到最新版——想「檢查更新」也用它：已是最新就會直接告訴你
skit --version             # 看目前的版本
```

`uv tool upgrade` 會跟著你當初的安裝來源走：從 PyPI 裝的追 PyPI 正式版，`git+…` 裝的會重新抓 main 分支。

## 從 0.4 升級

0.5 版用 Rust、Ratatui 與 Crossterm 完整取代 Python 與 Textual。執行 `uv tool upgrade skit-cli`
即可，不必匯出或匯入資料。skit 會原地讀取相同的工具庫、組態、預設、上次
使用值與 metadata；未知 TOML 欄位也會保留，啟動時不會重寫舊資料。

終端互動變得更直接：搜尋改為不分大小寫的子字串比對；所有純量型別共用文字編輯器，
送出時才驗證。第一個 Rust 版本不顯示舊的勾選框、選項清單或檔案瀏覽器。新增畫面改成
單一表單；新增後可在設定畫面管理偵測到的來源欄位。這些顯示差異不會改動腳本或狀態。
完整相容邊界見[切換設計](./docs/design/rust-rewrite.md)。

## 用法

整個介面，就兩條命令：

```bash
skit add my_script.py   # 加入腳本
skit add                # 不確定要加什麼？它會問你
skit                    # 打開選單，選腳本，填好輸入，跑
```

其餘一切都在 TUI 裡完成——都在畫面上，滑鼠鍵盤皆可，什麼都不用背。

剩下的 CLI 是給自動化和 AI agent 準備的——每個 TUI 動作都能腳本化：

```bash
skit run my_script -p fast    # 用已存的預設執行
skit run my_script --dry-run  # 印出實際會跑的命令，不真的執行
skit run my_script --set width=800 --no-input   # 直接指定參數值，永不詢問
skit show my_script --json    # 一支腳本的完整參數結構，機器可讀
skit params my_script         # 查看管理中的參數與上次的值
skit deps my_script --dep "requests>=2"   # 設定一支腳本的依賴套件
skit list --json              # 機器可讀的腳本清單
skit config                   # 設定：語言、編輯器、鏡像、表單樣式
skit --help                   # 其餘一切
```

## 給你的 AI agent 用

skit 是給人類和 AI agent 共用的腳本倉庫：同一個庫——你用 TUI，agent 用確定性的
CLI。官方 [Agent Skill](https://agentskills.io) 讓相容的 agent（Claude Code、Codex、
Cursor、Gemini CLI 等）先查你的庫再動手寫新的一次性腳本、直接檢視並執行庫裡現成的
腳本，並在徵得你同意後把它寫出的實用腳本收進庫裡——不再隨 session 結束而消失。

```bash
skit agent install            # 僅偵測到一個 agent 目錄時使用它；否則要求明確選擇
skit agent install claude     # 選擇 claude / codex / agents（--project 只裝進這個 repo）
npx skills add t41372/skit    # 或透過 skills.sh 安裝到 70+ 種 agent
```

## 語言

| 語言 | 狀態 |
| --- | --- |
| English | ✅ 100%，經人工校對 |
| 繁體中文（zh-TW） | ✅ 100%，經人工校對 |
| 简体中文（zh-CN） | ✅ 100%，經人工校對 |

skit 自動跟隨系統語言；想換，在 TUI 偏好設定裡改（自動化場景用 `skit config lang zh-TW`，或 `SKIT_LANG=en skit` 只切這一次）。想要其他語言？開個 issue 或 PR。

## 中國大陸

牆內有四處下載容易卡住：PyPI 套件、npm 套件、uv 從 GitHub 抓取的 Python 建置，還有 skit 引導下載的 uv 本體。skit 可以讓四者都走國內鏡像。

鏡像設定只存在 skit 裡：不碰你的全域 uv 設定；你已經自己設好的鏡像（`UV_DEFAULT_INDEX`、`uv.toml` 等）也不會被覆寫。npm registry 走 `NPM_CONFIG_REGISTRY` 環境變數：你環境裡已有的值仍然優先，但注意 npm 自己把這個變數排在 `~/.npmrc` 之上。

每個生態系各自獨立選擇——不同生態系的鏡像供應商並不相同，沒有哪個供應商能一個名字管全部：

- **首次執行**：偵測到 PyPI/GitHub 連不上時，skit 會主動問要不要開鏡像——每個生態系問一句，Enter 即接受各自的推薦預設。
- **隨時**：TUI「偏好設定 → 鏡像」，或：

```bash
skit config mirror.pypi tsinghua    # Python 套件：tsinghua / aliyun / ustc / 任意 URL / off
skit config mirror.github nju       # Python 建置 + uv 本體：nju / https:// 基底 URL / off
skit config mirror.npm npmmirror    # JS/TS 套件：npmmirror / 任意 URL / off
skit config mirror off              # 總開關：off 保留已存的 URL；`on` 恢復
```

自訂網址：在 TUI 偏好設定（或首次執行精靈）選 `custom`，或直接把 URL 傳給對應的軸。

## 解除安裝

```bash
uv tool uninstall skit-cli
```

這會移除 skit 本身與它在 `PATH` 上的捷徑。你的工具庫與設定存在套件**之外**，所以刻意會留著——重裝一次，一切照舊。想連這些也一併清掉，就刪掉 skit 自己的目錄：

| 作業系統 | 目錄 |
| --- | --- |
| **macOS** | `~/Library/Application Support/skit` |
| **Linux** | `~/.local/share/skit` · `~/.local/state/skit` · `~/.config/skit` |
| **Windows** | `%LOCALAPPDATA%\skit` |

這些目錄裝著你的工具庫、設定、參數預設與上次的值——以及，若 skit 曾自行下載過 uv，那份私有的 `uv` 執行檔（在 `…/skit/bin`，會跟著一起刪掉）。

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux——你若設過 XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME，會依你的設定
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/skit" "${XDG_STATE_HOME:-$HOME/.local/state}/skit" "${XDG_CONFIG_HOME:-$HOME/.config}/skit"
```

```powershell
# Windows（PowerShell）
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

不確定在哪？`skit doctor` 會印出實際的工具庫路徑（也會尊重 `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` 這幾個覆寫變數）。這就是 skit 擁有的全部——它從不動你的 `PATH`、shell 或全域 uv 設定，所以沒別的要善後。uv 的下載快取，以及 uv 抓下來的 Python 版本，是跟你整套 uv 環境共用的，不歸 skit 刪；你若沒在別處用 uv、又想把空間拿回來，`uv cache clean` 可以清掉快取。

## 為什麼會有 skit

skit 源自 [linux.do 上的一個帖子](https://linux.do/t/topic/2512255)

## 開發

開發使用固定版本的 Rust 工具鏈。TDD 流程、完整覆蓋率、cargo-mutants、供應鏈、wheel、
文檔與效能關卡見 [CONTRIBUTING.md](./CONTRIBUTING.md)。

```bash
cargo build --locked
cargo test --locked --workspace --all-targets --all-features
cargo run -p skit-cli-rs -- --help
```

## 授權

[MIT](LICENSE)
