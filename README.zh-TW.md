# skit

[![CI](https://github.com/user/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/user/skit/actions/workflows/ci.yml)
[![Coverage: 100%](https://img.shields.io/badge/coverage-100%25-brightgreen)](https://github.com/user/skit/actions/workflows/ci.yml)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit)](https://pypi.org/project/skit/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

[English](./README.md) | **中文**

Skit 是一個腳本啟動器 + 參數管家。如果你寫了一堆散落各處、參數寫死在原始碼裡的 Python 腳本,skit 讓你不再需要開編輯器改參數、不再需要記 CLI 旗標、不再需要管 venv——打開選單,選腳本,填表格,跑。

## 它做什麼

- **收納腳本**:`skit add` 把 Python 腳本、可執行檔、命令模板收進一個地方。copy 模式原檔逐字保存;reference 模式絕不碰原檔。
- **參數變表格**:add 時靜態分析(AST)偵測寫死的常量與 `input()` 呼叫,勾選納管後,每次 run 前跳出表單填值——不改你的原始碼語義,靠注入引擎在執行時替換。
- **記住上次的值**:last-used 自動保存;`preset` 存具名參數組;secret 參數結構性不落盤。
- **免管環境**:經 `uv run --script` 執行,PEP 723 聲明依賴;uv 缺失時自動下載私有副本(見下方)。
- **TUI + CLI 雙介面**:無參數進 Textual 選單(模糊搜尋、Enter 執行、`ctrl+e` 編輯參數);CLI 全功能等價。
- **i18n 一等公民**:en / zh-TW / zh-CN,GNU gettext 目錄(零執行期依賴,用標準庫 `gettext`),缺譯逐條回退原文。

## 前置需求:uv(硬需求)

skit 建立在 [uv](https://docs.astral.sh/uv/) 之上,沒有 uv 就無法運作。uv 提供了讓 skit 成立的隔離、可重現的腳本執行環境(PEP 723)。

**你不一定要事先安裝**:如果 skit 在系統上找不到 uv,會先徵求你的同意,然後把一份釘定版本的 uv binary 下載到 skit 自己的私有目錄。這份副本不會碰你的 `PATH` 或全域環境。

不過,系統層級安裝 uv 的體驗最順。安裝方式擇一:

```bash
# macOS / Linux
curl -LsSf https://astral.sh/uv/install.sh | sh

# Windows (PowerShell)
powershell -c "irm https://astral.sh/uv/install.ps1 | iex"

# Homebrew / pipx / cargo
brew install uv
pipx install uv
cargo install --git https://github.com/astral-sh/uv uv
```

驗證:

```bash
uv --version
```

## 中國大陸 — 免 VPN

在防火長城之後,有三處下載可能受阻:PyPI 套件、uv 抓取的 Python 直譯器(python-build-standalone,來自 GitHub),以及 skit 自己的 uv 引導下載。skit 可把這三者都導向國內鏡像——而且**絕不改動你的全域 uv 設定或環境變數**。

- **首次執行**:若偵測到 PyPI/GitHub 連不上,`skit` 會詢問是否開啟鏡像,按 Enter 即可。
- **隨時**:`skit config` 提供引導式設定(語言 + 鏡像),或直接設定:

```bash
skit config --mirror tsinghua    # 或 aliyun / ustc
skit config --show
skit config --mirror off         # 例如出國時關閉
```

預設:PyPI → 清華 / 阿里雲 / 中科大;Python 發行版與 uv 二進位 → 南京大學(`mirror.nju.edu.cn`)。鏡像掛掉時,在 `skit config` 選 `custom` 可覆寫任一網址。

**安裝 skit 本身**時(此時還沒有 skit 可設定),請先讓 uv 指向鏡像:

```bash
export UV_DEFAULT_INDEX=https://pypi.tuna.tsinghua.edu.cn/simple
uv tool install skit
```

已自行設定 `UV_DEFAULT_INDEX` / `UV_PYTHON_INSTALL_MIRROR`(或 `uv.toml`)?skit 會**尊重你的設定**,不會覆寫。

## 安裝

從 PyPI(發布後):

```bash
uv tool install skit
```

直接從 git 安裝(現在就可用,不必等 PyPI 首發):

```bash
uv tool install git+https://github.com/user/skit
```

或者完全不安裝,直接跑:

```bash
uvx --from git+https://github.com/user/skit skit --help
```

## 用法

```bash
skit                          # TUI 主選單:搜尋、Enter 執行、ctrl+e 編輯參數、Del 刪除
skit add my_script.py         # 加入腳本(copy 模式;偵測依賴與參數候選,互動勾選)
skit add my_script.py --ref   # reference 模式:不複製,連結原檔
skit add tool.exe --exe       # 登記可執行檔
skit add --cmd "ffmpeg -i {input}" --name conv   # 登記命令模板(佔位符變表單)
skit run my_script            # 執行;run 前跳參數表單
skit run my_script --preset fast   # 用具名參數組
skit run my_script --raw      # 逃生門:跳過表單與注入,原樣直跑
skit params my_script         # 查看參數定義 + 上次的值
skit edit my_script --resync  # 對賬:腳本改了之後同步參數定義
skit preset save my_script fast    # 保存具名參數組
skit deps my_script --set requests,rich   # 查看/更新依賴
skit list                     # 列出所有已登記項目
skit remove <name>            # 移除一個項目
skit doctor [--rebuild]       # 自檢 / 從散落的 meta.toml 重建索引
skit lang zh-TW               # 查看/設定介面語言
skit config                   # 互動式設定:語言 + 下載鏡像(中國大陸友善)
```

## 開發

開發流程完全以 uv 驅動——完整工作流與品質閘門(ruff、ty 最嚴格模式、pytest 覆蓋率下限 100%、mutmut 突變測試、zizmor 稽核的 workflows)見 [CONTRIBUTING.md](./CONTRIBUTING.md)。

```bash
uv sync --dev
uv run pytest -q
uv run python scripts/serve_preview.py   # TUI 網頁預覽(textual-serve,localhost:8000)
```

## 授權

[MIT](LICENSE)
