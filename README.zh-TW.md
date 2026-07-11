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

**skit 是 Python 腳本的啟動器，也是它們的集中收納處。**

**AI 寫腳本，skit 管腳本。**

<video src="https://github.com/user-attachments/assets/bd4f78ba-0a1f-4b73-b02e-bc1dc1d11c4c" controls></video>

## 它做什麼

- **收納腳本**。`skit add` 把散落各處的腳本收進同一個可搜尋的腳本庫——複製一份收進庫裡，或用連結模式直接指向原檔。
- **參數不再折磨人**。旗標、`input()`、你勾選納管的常數，全部變成表單欄位（choices 變選擇器、布林變勾選框、型別自動把關）。
- **它會記住**。上次填的值自動帶回；常用的一組存成具名組合。標記為機密的參數永不落盤。`{cwd}`、`{today}` 這類值 token 讓同一個組合跨機器、跨目錄通用。
- **環境零污染**。skit 把每支腳本的依賴用標準 PEP 723 語法聲明在腳本開頭，執行時由 uv 在隔離、有快取的環境裡解析——你不用管 venv，也不會往全域裝任何東西。
- **滑鼠鍵盤皆可**。直接執行 `skit` 就是完整 TUI；畫面上每個按鍵提示同時也是一顆可點的按鈕。
- **天生適合自動化**。每個 TUI 動作都有對應的 CLI 命令，帶 `--json` 輸出與明確退出碼——shell 腳本、CI、AI agent 都好接。
- **多語言支持**。English、繁體中文、简体中文，更多語言在路上。見[語言](#語言)。


| 痛點 | skit 的解法 |
| --- | --- |
| 腳本東一支西一支，散落在各個資料夾 | 全部收進同一個選單，附搜尋 |
| 腳本帶著一堆奇怪的第三方依賴 | 每支腳本一個隔離環境——依賴以 PEP 723 聲明在腳本開頭，由 uv 解析 |
| 命令列旗標轉頭就忘、`input()` 一項項問、常數寫死在原始碼裡，改個值都得開編輯器 | 靜態分析把參數通通讀出來，變成一張互動表單——原始碼一行不動。上次的值自動帶回；常用的存成組合（preset） |

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

牆內有三處下載容易卡住：PyPI 套件、uv 從 GitHub 抓取的 Python 建置，還有 skit 引導下載的 uv 本體。skit 可以讓三者都走國內鏡像。

鏡像設定只存在 skit 裡：不碰你的全域 uv 設定；你已經自己設好的鏡像（`UV_DEFAULT_INDEX`、`uv.toml` 等）也不會被覆寫。

- **首次執行**：偵測到 PyPI/GitHub 連不上時，skit 會主動問要不要開鏡像——按個 Enter 就好。
- **隨時**：TUI「偏好設定 → 鏡像」，或：

```bash
skit config mirror tsinghua   # 或：aliyun / ustc / custom / off
```

預設：PyPI → 清華 / 阿里雲 / 中科大；Python 建置與 uv 二進位 → 南京大學。哪個鏡像掛了，選 `custom` 就能逐項換網址。

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
