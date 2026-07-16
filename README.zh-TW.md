![skit — 腳本啟動器 + 參數管家](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png)

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

[English](./README.md) | **繁體中文** | [简体中文](./README.zh-CN.md)

**skit 是腳本的啟動器，也是它們的集中收納處。**

skit 把你的腳本集中收進一處，啟動毫不費力——Python、shell、JS/TS 支援最深，共十餘種類型。

**AI 寫腳本，skit 管腳本。**

而且這個庫是你和 agent 共用的：你從選單和表單操作，AI agent 走確定性的 CLI 操作同一個庫——動手寫新的一次性腳本前先查庫，寫出好用的（經你同意後）收回庫裡，不再隨對話結束而消失。

<video src="https://github.com/user-attachments/assets/9a648986-f782-43be-8dee-acfd6cc0b093" controls></video>

## 它做什麼

- **收納腳本**。`skit add` 把散落各處的腳本收進同一個可搜尋的腳本庫——複製一份收進庫裡，或用引用模式直接指向原檔。
- **參數不再折磨人**。旗標、`input()`、你勾選納管的常數，全部變成表單欄位（choices 變選擇器、布林變勾選框、型別自動把關）。
- **它會記住**。上次填的值自動帶回；常用的一組存成具名組合。標記為機密的參數永不落盤。`{cwd}`、`{today}` 這類值 token 讓同一個組合跨機器、跨目錄通用。
- **環境零污染**。Python 腳本的依賴以 PEP 723 語法聲明在腳本開頭，由 uv 在隔離環境裡解析；JS/TS 腳本則有按腳本隔離的 `node_modules`，首次執行時依宣告的套件自動安裝。兩者都不往全域裝任何東西。其他語言沿用你機器上已有的工具——skit 會在執行前檢查腳本聲明的外部命令是否在 `PATH` 上。
- **滑鼠鍵盤皆可**。直接執行 `skit` 就是完整 TUI；畫面上每個按鍵提示同時也是一顆可點的按鈕。
- **天生適合自動化**。每個 TUI 動作都有對應的 CLI 命令，帶 `--json` 輸出與明確退出碼——shell 腳本、CI、AI agent 都好接。
- **也是你的 agent 的腳本庫**。官方 [Agent Skill](https://agentskills.io) 教會 Claude Code、Codex、Cursor、Gemini CLI 等 agent 完整用法：用 `skit list` 探索、用 `skit show` 讀參數結構、用 `skit run --set … --no-input` 執行。一句 `skit agent install` 就裝好——見[給你的 AI agent 用](#給你的-ai-agent-用)。
- **多語言支持**。English、繁體中文、简体中文，更多語言在路上。見[語言](#語言)。


| 痛點 | skit 的解法 |
| --- | --- |
| 腳本東一支西一支，散落在各個資料夾 | 全部收進同一個選單，附搜尋 |
| 腳本需要特定套件或工具 | Python（PEP 723 + uv）和 JS/TS（npm）都有按腳本隔離的依賴；任何語言都可宣告外部命令，skit 在執行前檢查是否在 `PATH` 上 |
| 命令列旗標轉頭就忘、`input()` 一項項問、常數寫死在原始碼裡，改個值都得開編輯器 | 靜態分析把參數通通讀出來，變成一張互動表單——原始碼一行不動。上次的值自動帶回；常用的存成組合（preset） |
| AI 幫你寫的腳本隨對話結束石沉大海，下次又重寫一遍 | agent 先查庫再動手，現成的直接重用；值得留的收進庫裡——一次性腳本變成永久的、參數化的工具 |

不需要為腳本做任何準備——不用重構，沒有設定檔要維護。AI 上週寫的腳本，和你去年寫完就忘的那支，啟動起來一模一樣。

| ![腳本庫](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-library-zh.png) | ![執行表單](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-form-zh.png) |
|:--:|:--:|
| **腳本庫**——每個動作都在畫面上，滑鼠鍵盤皆可 | **執行表單**——從腳本自己的參數生成 |
| ![加入腳本](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-add-zh.png) | ![腳本設定](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-settings-zh.png) |
| **加入腳本**——靜態偵測參數，勾選即納管 | **腳本設定**——參數、機密、組合、依賴 |

<p align="center">
  <img width="480" alt="只用滑鼠操作 skit——畫面上每個控制項都是可點擊的目標" src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/demo-mouse.gif"><br>
  <em>完全滑鼠可操作性——畫面上每個按鍵提示，也都是可點的按鈕。</em>
</p>

## 腳本語言支援

Python、shell、JS/TS 支援最深——既靜態偵測參數，**也能注入值**；其餘類型則負責啟動、接受宣告式參數，並讀懂腳本自己的 CLI 解析器。

| 類型 | 執行方式 | 偵測參數 | 注入 | 讀腳本自己的 CLI | 依賴 / needs |
| --- | --- | --- | --- | --- | --- |
| **Python** | `uv run --script` | 常數、`input()` | ✅ | argparse · click · typer | PEP 723（uv）+ needs |
| **Shell**（bash/sh/zsh） | 直譯器 | 常數、`${VAR:-}` 環境預設值、`read` | ✅ | getopts | needs |
| **JS / TS** | deno › bun › node | `const` | ✅ | `util.parseArgs` | npm（按腳本）+ needs |
| **fish** | fish | `set -q NAME; or set NAME …` 環境預設值 | — | `argparse` 內建 | needs |
| **PowerShell** | pwsh | — | — | `param()` | needs |
| **Ruby · Perl · Lua · R** | 直譯器 | — | — | — | needs |
| **程式**（exe） | 直接執行 | — | — | — | needs |
| **命令**（command） | 填充範本 | — | — | — | needs |

每種類型也都能手動**宣告**參數，所以連程式和命令範本都享有同樣的表單 / 組合 / `--set` 體驗。**needs** 是 skit 在執行前檢查是否位於 `PATH` 上的外部命令（任何類型皆適用）。Python 和 JS/TS 都有按腳本隔離的依賴套件：uv 解析 PEP 723 區塊，npm 式依賴則安裝到存副本旁的 `node_modules`（`skit add` 會從腳本自己的 import 建議清單）。JS/TS 依賴管理僅適用於複製進庫的條目——reference 模式的腳本沿用自己專案的 `node_modules`——且安裝一律不執行套件的 lifecycle scripts（npm 和 bun 加 `--ignore-scripts`；deno 預設就不跑）。另一個拉平差異的點：skit 自動選中 deno 時會帶 `--allow-all`，同一支腳本在 deno、bun、node 下行為一致。

## 安裝

skit 建立在 [uv](https://docs.astral.sh/uv/) 之上（以 0.11.26 版測試）。還沒裝 uv？skit 會先徵求你同意，再把釘定版本的 uv 下載到自己的私有目錄——不碰你的 `PATH`，也不碰全域環境。當然，參考[官方文檔](https://docs.astral.sh/uv/getting-started/installation/) 安裝 uv 會更好。

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

## 解除安裝

```bash
uv tool uninstall skit-cli
```

這會移除 skit 本身與它在 `PATH` 上的捷徑。你的腳本庫與設定存在套件**之外**，所以刻意會留著——重裝一次，一切照舊。想連這些也一併清掉，就刪掉 skit 自己的目錄：

| 作業系統 | 目錄 |
| --- | --- |
| **macOS** | `~/Library/Application Support/skit` |
| **Linux** | `~/.local/share/skit` · `~/.local/state/skit` · `~/.config/skit` |
| **Windows** | `%LOCALAPPDATA%\skit` |

這些目錄裝著你的腳本庫、設定、參數組合與上次的值——以及，若 skit 曾自行下載過 uv，那份私有的 `uv` 執行檔（在 `…/skit/bin`，會跟著一起刪掉）。

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux——你若設過 XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME，會依你的設定
rm -rf ~/.local/share/skit ~/.local/state/skit ~/.config/skit
```

```powershell
# Windows（PowerShell）
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

不確定在哪？`skit doctor` 會印出實際的腳本庫路徑（也會尊重 `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` 這幾個覆寫變數）。這就是 skit 擁有的全部——它從不動你的 `PATH`、shell 或全域 uv 設定，所以沒別的要善後。uv 的下載快取，以及 uv 抓下來的 Python 版本，是跟你整套 uv 環境共用的，不歸 skit 刪；你若沒在別處用 uv、又想把空間拿回來，`uv cache clean` 可以清掉快取。

## 用法

整個介面，就兩條命令：

```bash
skit add my_script.py   # 加入腳本
skit                    # 打開選單，選腳本，填表單，跑
```

其餘一切都在 TUI 裡完成——都在畫面上，滑鼠鍵盤皆可，什麼都不用背。

剩下的 CLI 是給自動化和 AI agent 準備的——每個 TUI 動作都能腳本化：

```bash
skit run my_script -p fast    # 用已存的組合執行
skit run my_script --dry-run  # 印出實際會跑的命令，不真的執行
skit run my_script --set width=800 --no-input   # 直接指定參數值，永不詢問
skit show my_script --json    # 一支腳本的完整參數結構，機器可讀
skit params my_script         # 查看納管參數與上次的值
skit list --json              # 機器可讀的腳本清單
skit config                   # 設定：語言、編輯器、鏡像、表單樣式
skit --help                   # 其餘一切
```

## 給你的 AI agent 用

skit 是給人類和 AI agent 共用的腳本倉庫：同一個庫——你用表單，agent 用確定性的
CLI。官方 [Agent Skill](https://agentskills.io) 讓相容的 agent（Claude Code、Codex、
Cursor、Gemini CLI 等）先查你的庫再動手寫新的一次性腳本、直接檢視並執行庫裡現成的
腳本，並在徵得你同意後把它寫出的實用腳本收進庫裡——不再隨 session 結束而消失。

```bash
skit agent install            # 從機器上偵測到的 agent 目錄裡挑一個
skit agent install claude     # 或直接指名：claude / codex / agents（--project 只裝進這個 repo）
npx skills add t41372/skit    # 或透過 skills.sh 安裝到 70+ 種 agent
```

## 語言

| 語言 | 狀態 |
| --- | --- |
| English | ✅ 100%，經人工校對 |
| 繁體中文（zh-TW） | ✅ 100%，經人工校對 |
| 简体中文（zh-CN） | ✅ 100%，經人工校對 |

skit 自動跟隨系統語言；想換，在 TUI 偏好設定裡改（自動化場景用 `skit config lang zh-TW`，或 `SKIT_LANG=en skit` 只切這一次）。

## 中國大陸

牆內有四處下載容易卡住：PyPI 套件、npm 套件、uv 從 GitHub 抓取的 Python 建置，還有 skit 引導下載的 uv 本體。skit 可以讓四者都走國內鏡像。

鏡像設定只存在 skit 裡：不碰你的全域 uv 設定；你已經自己設好的鏡像（`UV_DEFAULT_INDEX`、`uv.toml` 等）也不會被覆寫。npm registry 走 `NPM_CONFIG_REGISTRY` 環境變數：你環境裡已有的值仍然優先，但注意 npm 自己把這個變數排在 `~/.npmrc` 之上。

- **首次執行**：偵測到 PyPI/GitHub 連不上時，skit 會主動問要不要開鏡像——按個 Enter 就好。
- **隨時**：TUI「偏好設定 → 鏡像」，或：

```bash
skit config mirror tsinghua   # 或：aliyun / ustc / off
```

預設：PyPI → 清華 / 阿里雲 / 中科大；npm → npmmirror；Python 建置與 uv 二進位 → 南京大學。哪個鏡像掛了，在 TUI 偏好設定（或首次執行精靈）選 `custom` 就能逐項換網址。

## 為什麼會有 skit

skit 源自 [linux.do 上的一個帖子](https://linux.do/t/topic/2512255)

## 開發

開發流程完全跑在 uv 上——完整工作流與品質關卡（ruff、ty 最嚴格模式、pytest 覆蓋率下限 100%、mutmut 突變測試、zizmor 稽核的 workflows）見 [CONTRIBUTING.md](./CONTRIBUTING.md)。

```bash
uv sync --dev
uv run pytest -q
uv run python scripts/serve_preview.py   # TUI 網頁預覽（textual-serve，localhost:8000）
```

## 授權

[MIT](LICENSE)
