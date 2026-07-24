<img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png" alt="skit — 脚本启动器 + 参数管家" width="750">

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

[English](./README.md) | [繁體中文](./README.zh-TW.md) | **简体中文**

**skit 是终端里的脚本管理器与启动器。**

skit 把你的脚本集中收在一处，并让启动脚本非常容易 —— 支持 Python、shell、JS/TS、可执行文件、提示词等等。

skit 会读你的脚本，把命令行参数、`input()`、写死的常量之类的东西变成一个带说明的启动菜单 —— 输入和变量都在画面上改，不用改脚本。

于是你再也不用担心明年找不到或忘了怎么用你（或 AI）写的脚本了 —— 直接塞 skit 里，什么时候要跑都很轻松。

要记的命令就两条：

```bash
skit add script.py   # 把脚本收进库里
skit                 # 打开菜单——选、填、跑
```

你的 AI agent 也能用 skit：你从菜单操作，agent 走确定性的 CLI 和 skill—— AI 写完脚本存进去，之后也能轻松调用。

<video src="https://github.com/user-attachments/assets/5899c4f2-a65d-4a22-b386-4ed24a62cdce" controls></video>

## 它做什么

- **收纳脚本与提示词**。`skit add` 把散落各处的脚本与提示词收进同一个可搜索的库（支持模糊搜索）。
- **不用背命令行参数，也不用为了改个值打开编辑器**。命令行参数、`input()`、你选择管理的常量，全部变成启动菜单里的字段——有类型、有说明、全自动。choices 变选择器、布尔变复选框；路径边打边补全，还有文件浏览器。
- **记住你上次填的值**。启动菜单里的参数下次会自动带回；`↺ 默认值`（Ctrl+O）一键改回脚本自己的默认。常用的存成命名组合——`{cwd}`、`{today}` 这类 token 让组合跨机器、跨目录通用。标记为机密的参数永不保存：上次的值、组合、运行历史里都不会有它。
- **环境零污染**。Python 脚本的依赖以 PEP 723 语法声明在脚本开头，由 uv 在隔离环境里解析；JS/TS 脚本则有按脚本隔离的 `node_modules`，首次运行时按声明的包自动安装。两者都不往全局装任何东西。其他语言沿用你机器上已有的工具——skit 会在运行前检查脚本声明的外部命令是否在 `PATH` 上。
- **提示词当脚本用**。存一份带参数的提示词（管理中的 `{{占位符}}` 变成输入字段），交给你的 coding agent 启动——claude、codex、opencode，或设置任何你喜欢的执行器。
- **鼠标键盘皆可，多语言支持**。直接运行 `skit` 就是完整 TUI；画面上每个快捷键提示同时也是可点的按钮——不用是终端高手。界面有 English、繁體中文、简体中文（[语言](#语言)）。
- **也为 AI agent 而生**。每个 TUI 动作都有对应的 CLI 命令，带 `--json` 输出和明确退出码；官方 [Agent Skill](https://agentskills.io) 教 Claude Code、Codex、Cursor、Gemini CLI 等 agent 先查库、用现成的、把好用的收进库——见[给你的 AI agent 用](#给你的-ai-agent-用)。

| 痛点 | skit 的解法 |
| --- | --- |
| 脚本东一个西一个，散落在各个文件夹 | 全部收进同一个菜单，带搜索 |
| 脚本需要特定包或工具 | Python（PEP 723 + uv）和 JS/TS（npm）都有按脚本隔离的依赖；任何语言都可声明外部命令，skit 在运行前检查是否在 `PATH` 上 |
| 命令行参数转头就忘、`input()` 一项项问、常量写死在源码里等着你手改 | 静态分析把参数统统读出来，变成交互菜单——源码一行不动、零配置。上次的值自动带回；常用的存成组合 |
| AI 帮你写的脚本随对话结束石沉大海，下次又重写一遍 | agent 先查库再动手，现成的直接重用；值得留的收进库里——一次性脚本变成永久的、参数化的工具 |

不需要专门为了 skit 修改脚本 —— 我们会搞定，如果有必要我们会问你。

| ![工具库](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-library-zh.png) | ![启动菜单](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-form-zh.png) |
|:--:|:--:|
| **工具库**——每个动作都在画面上，鼠标键盘皆可 | **启动菜单**——从脚本自己的参数生成 |
| ![加入脚本](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-add-zh.png) | ![脚本设置](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-settings-zh.png) |
| **加入脚本**——静态检测参数；哪些交给 skit 管理由你决定 | **脚本设置**——参数、机密、组合、依赖 |

<p align="center">
  <img width="480" alt="只用鼠标操作 skit——画面上每个控件都是可点击的目标" src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/demo-mouse.gif"><br>
  <em>完全鼠标可操作性——画面上每个按键提示，也都是可点的按钮。</em>
</p>

## 支持的脚本类型

Python、shell、JS/TS 支持最完整：skit 直接读代码找出参数。其他类型开箱即可启动。

| 脚本类型 | 运行方式 | 支持的参数检测 |
| --- | --- | --- |
| **Python** | uv（`uv run --script`） | 命令行参数（argparse · click · typer）、`input()`、常量 |
| **Shell**（bash/sh/zsh） | 对应的 shell | 命令行参数（getopts）、`read`、常量、`${VAR:-}` 默认值 |
| **JS / TS** | 依次找 deno、bun、node | 命令行参数（`util.parseArgs`）、`const` 值 |
| **fish** | fish | 命令行参数（`argparse`）、`set -q` 环境默认值 |
| **PowerShell** | pwsh | `param()` 定义 |
| **Ruby · Perl · Lua · R** | 各自的解释器 | — |
| **可执行文件** | 直接执行 | — |
| **命令模板** | skit 填好空格后执行 | — |
| **提示词** | 你的 coding agent（claude · codex · …） | `{{占位符}}` |

你的类型没有自动检测？手动声明参数就好——每种类型都享有同样的启动菜单 / 组合 / `--set` 体验，连纯可执行文件也一样（声明的值会以普通命令行参数传入）。任何条目还能列出它依赖的外部命令（`ffmpeg`、`jq`……）；skit 每次运行前都会检查它们是否在 `PATH` 上。

Python 和 JS/TS 都有按脚本隔离的依赖包：uv 解析 PEP 723 块，npm 式依赖则安装到库内副本旁的 `node_modules`——安装一律不执行包的 lifecycle scripts。更细的部分（复制 vs 引用条目、deno 的 `--allow-all`）见[文档（英文）](https://t41372.github.io/skit/en/docs/script-types/)。

skit 会替 Python 引导 uv，但不会替你装 JS runtime——node、bun 或 deno 需要你自己准备好。

### 提示词

提示词条目是给 AI coding agent 的、可复用且带参数的一段文字。添加一个 `.prompt.md` 文件（或用 `skit add --prompt` 直接起草）后，交互加入时可在复核页中选择哪些检测到的 `{{占位符}}` 要变成输入字段。候选不超过 30 个时默认全选；超过 30 个时默认一个都不选，以免把代码示例误认为变量。管理中的字段完整享有组合 / 上次值 / `--set` 体验。

没有任何转义规则要学：凡是你没交给 skit 管理的内容——包括没管理的 `{{占位符}}`——一律逐字节原样送达 agent；每个提示词还有插值总开关（`--no-interpolate`）。**执行器**在启动菜单上选（或按提示词固定）；claude / codex / opencode / amp / antigravity / copilot / cursor / pi 已预先配置，其他 CLI 用 `skit runner add` 注册。一个诚实的提醒：提示词不是保密通道——渲染后的内容会落在对方 agent 自己的 session 记录里。各执行器的行为、非交互时的解析规则、无 shell 的递送保证：见[文档（英文）](https://t41372.github.io/skit/en/docs/prompts/)。

```bash
skit add review.prompt.md            # 管理中的占位符变成输入字段
skit run review                      # 选 agent、填好输入、出发
skit run review --runner codex --set target=src/app.py --no-input
```

## 安装

skit 建立在 [uv](https://docs.astral.sh/uv/) 之上（以 0.11.26 版本测试）。还没装 uv？skit 会先征得你同意，再把锁定版本的 uv 下载到自己的私有目录——不碰你的 `PATH`，也不碰全局环境。当然，参考[官方文档](https://docs.astral.sh/uv/getting-started/installation/) 安装 uv 会更好。

```bash
# 用 uv tool 从 PyPI 安装 skit（包名是 skit-cli，装好的命令是 skit）
uv tool install skit-cli
```

> **人在中国大陆？**这一步 skit 还没装上、没法替你设置，请手动让 uv 指向镜像（详见[中国大陆](#中国大陆)）：
>
> ```bash
> export UV_DEFAULT_INDEX=https://pypi.tuna.tsinghua.edu.cn/simple
> uv tool install skit-cli
> ```

或者，从 main 分支直接安装开发版本

```bash
uv tool install git+https://github.com/t41372/skit          # 最新开发版
uvx --from git+https://github.com/t41372/skit skit --help   # 或是什么都不装，直接试
```

## 更新

```bash
uv tool upgrade skit-cli   # 更新到最新版——想「检查更新」也用它：已是最新会直接告诉你
skit --version             # 看当前版本
```

`uv tool upgrade` 会跟随你当初的安装来源：从 PyPI 装的追 PyPI 正式版，`git+…` 装的会重新拉取 main 分支。

## 用法

整个界面，就两条命令：

```bash
skit add my_script.py   # 加入脚本
skit add                # 不确定要加什么？它会问你
skit                    # 打开菜单，选脚本，填好输入，跑
```

其余一切都在 TUI 里完成——都在画面上，鼠标键盘皆可，什么都不用背。

剩下的 CLI 是给自动化和 AI agent 准备的——每个 TUI 动作都能脚本化：

```bash
skit run my_script -p fast    # 用已存的组合执行
skit run my_script --dry-run  # 打印实际会跑的命令，不真的执行
skit run my_script --set width=800 --no-input   # 直接指定参数值，从不询问
skit show my_script --json    # 一个脚本的完整参数结构，机器可读
skit params my_script         # 查看管理中的参数与上次的值
skit deps my_script --dep "requests>=2"   # 设置一个脚本的依赖包
skit list --json              # 机器可读的脚本清单
skit config                   # 设置：语言、编辑器、镜像、表单样式
skit --help                   # 其余一切
```

## 给你的 AI agent 用

skit 是给人类和 AI agent 共用的脚本仓库：同一个库——你用 TUI，agent 用确定性的
CLI。官方 [Agent Skill](https://agentskills.io) 让兼容的 agent（Claude Code、Codex、
Cursor、Gemini CLI 等）先查你的库再动手写新的一次性脚本、直接查看并运行库里现成的
脚本，并在征得你同意后把它写出的实用脚本收进库里——不再随 session 结束而消失。

```bash
skit agent install            # 从机器上检测到的 agent 目录里挑一个
skit agent install claude     # 或直接指名：claude / codex / agents（--project 只装进这个 repo）
npx skills add t41372/skit    # 或通过 skills.sh 安装到 70+ 种 agent
```

## 语言

| 语言 | 状态 |
| --- | --- |
| English | ✅ 100%，经人工校对 |
| 繁體中文（zh-TW） | ✅ 100%，经人工校对 |
| 简体中文（zh-CN） | ✅ 100%，经人工校对 |

skit 自动跟随系统语言；想换，在 TUI 偏好设置里改（自动化场景用 `skit config lang zh-CN`，或 `SKIT_LANG=en skit` 只切这一次）。想要其他语言？开个 issue 或 PR。

## 中国大陆

墙内有四处下载容易卡住：PyPI 包、npm 包、uv 从 GitHub 拉取的 Python 构建，还有 skit 引导下载的 uv 本体。skit 可以让四者都走国内镜像。

镜像设置只存在 skit 里：不碰你的全局 uv 配置；你已经自己配好的镜像（`UV_DEFAULT_INDEX`、`uv.toml` 等）也不会被覆盖。npm registry 走 `NPM_CONFIG_REGISTRY` 环境变量：你环境里已有的值仍然优先，但注意 npm 自己把这个变量排在 `~/.npmrc` 之上。

每个生态各自独立选择——不同生态的镜像供应商并不相同，没有哪个供应商能一个名字管全部：

- **首次运行**：检测到 PyPI/GitHub 连不上时，skit 会主动问要不要开镜像——每个生态问一句，回车即接受各自的推荐预设。
- **随时**：TUI「偏好设置 → 镜像」，或：

```bash
skit config mirror.pypi tsinghua    # Python 包：tsinghua / aliyun / ustc / 任意 URL / off
skit config mirror.github nju       # Python 构建 + uv 本体：nju / https:// 基底 URL / off
skit config mirror.npm npmmirror    # JS/TS 包：npmmirror / 任意 URL / off
skit config mirror off              # 总开关：off 保留已存的 URL；`on` 恢复
```

自定义地址：在 TUI 偏好设置（或首次运行向导）选 `custom`，或直接把 URL 传给对应的轴。

## 卸载

```bash
uv tool uninstall skit-cli
```

这会移除 skit 本身与它在 `PATH` 上的快捷方式。你的工具库与设置存在包**之外**，所以会刻意保留——重装一次，一切照旧。想连这些也一并清掉，就删掉 skit 自己的目录：

| 操作系统 | 目录 |
| --- | --- |
| **macOS** | `~/Library/Application Support/skit` |
| **Linux** | `~/.local/share/skit` · `~/.local/state/skit` · `~/.config/skit` |
| **Windows** | `%LOCALAPPDATA%\skit` |

这些目录装着你的工具库、设置、参数组合与上次的值——以及，若 skit 曾自行下载过 uv，那份私有的 `uv` 可执行文件（在 `…/skit/bin`，会跟着一起删掉）。

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux——你若设过 XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME，会按你的设置
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/skit" "${XDG_STATE_HOME:-$HOME/.local/state}/skit" "${XDG_CONFIG_HOME:-$HOME/.config}/skit"
```

```powershell
# Windows（PowerShell）
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

不确定在哪？`skit doctor` 会打印出实际的工具库路径（也会尊重 `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` 这几个覆写变量）。这就是 skit 拥有的全部——它从不动你的 `PATH`、shell 或全局 uv 配置，所以没别的要善后。uv 的下载缓存，以及 uv 拉取的 Python 版本，是跟你整套 uv 环境共用的，不归 skit 删；你若没在别处用 uv、又想把空间拿回来，`uv cache clean` 可以清掉缓存。

## 为什么会有 skit

skit 源自 [linux.do 上的一个帖子](https://linux.do/t/topic/2512255)

## 开发

开发流程完全跑在 uv 上——完整工作流与质量关卡（ruff、ty 最严格模式、pytest 覆盖率下限 100%、mutmut 变异测试、zizmor 审计的 workflows）见 [CONTRIBUTING.md](./CONTRIBUTING.md)。

```bash
uv sync --dev
uv run pytest -q
uv run python scripts/serve_preview.py   # TUI 网页预览（textual-serve，localhost:8000）
```

## 许可证

[MIT](LICENSE)
