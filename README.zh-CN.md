![skit — 脚本启动器 + 参数管家](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png)

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

[English](./README.md) | [繁體中文](./README.zh-TW.md) | **简体中文**

**skit 是脚本的启动器，也是它们的集中收纳处。**

skit 把你的脚本集中收进一处，启动毫不费力——Python、shell、JS/TS 支持最深，共十余种类型。

**AI 写脚本，skit 管脚本。**

而且这个库是你和 agent 共用的：你从菜单和表单操作，AI agent 走确定性的 CLI 操作同一个库——动手写新的一次性脚本前先查库，写出好用的（经你同意后）收回库里，不再随对话结束而消失。

<video src="https://github.com/user-attachments/assets/9a648986-f782-43be-8dee-acfd6cc0b093" controls></video>

## 它做什么

- **收纳脚本**。`skit add` 把散落各处的脚本收进同一个可搜索的脚本库——复制一份收进库里，或用引用模式直接指向原始文件。
- **参数不再折磨人**。命令行参数、`input()`、你勾选纳管的常量，全部变成表单字段（choices 变选择器、布尔变复选框、类型自动把关）。
- **它会记住**。上次填的值自动带回；常用的一组存成命名组合。标记为机密的参数永不落盘。`{cwd}`、`{today}` 这类值 token 让同一个组合跨机器、跨目录通用。
- **环境零污染**。Python 脚本的依赖以 PEP 723 语法声明在脚本开头，由 uv 在隔离环境里解析；JS/TS 脚本则有按脚本隔离的 `node_modules`，首次运行时按声明的包自动安装。两者都不往全局装任何东西。其他语言沿用你机器上已有的工具——skit 会在运行前检查脚本声明的外部命令是否在 `PATH` 上。
- **鼠标键盘皆可**。直接运行 `skit` 就是完整 TUI；画面上每个快捷键提示同时也是一个可点的按钮。
- **天生适合自动化**。每个 TUI 动作都有对应的 CLI 命令，带 `--json` 输出和明确退出码——shell 脚本、CI、AI agent 都好接。
- **也是你的 agent 的脚本库**。官方 [Agent Skill](https://agentskills.io) 教会 Claude Code、Codex、Cursor、Gemini CLI 等 agent 完整用法：用 `skit list` 探索、用 `skit show` 读参数结构、用 `skit run --set … --no-input` 运行。一句 `skit agent install` 就装好——见[给你的 AI agent 用](#给你的-ai-agent-用)。
- **多语言支持**。English、繁體中文、简体中文，更多语言在路上。见[语言](#语言)。


| 痛点 | skit 的解法 |
| --- | --- |
| 脚本东一个西一个，散落在各个文件夹 | 全部收进同一个菜单，带搜索 |
| 脚本需要特定包或工具 | Python（PEP 723 + uv）和 JS/TS（npm）都有按脚本隔离的依赖；任何语言都可声明外部命令，skit 在运行前检查是否在 `PATH` 上 |
| 命令行参数转头就忘、`input()` 一项项问、常量写死在源码里，改个值都得开编辑器 | 静态分析把参数统统读出来，变成一张交互表单——源码一行不动。上次的值自动带回；常用的存成组合（preset） |
| AI 帮你写的脚本随对话结束石沉大海，下次又重写一遍 | agent 先查库再动手，现成的直接重用；值得留的收进库里——一次性脚本变成永久的、参数化的工具 |

不需要为脚本做任何准备——不用重构，没有配置要维护。AI 上周写的脚本，和你去年写完就忘的那个，启动起来一模一样。

| ![脚本库](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-library-zh.png) | ![执行表单](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-form-zh.png) |
|:--:|:--:|
| **脚本库**——每个动作都在画面上，鼠标键盘皆可 | **执行表单**——从脚本自己的参数生成 |
| ![加入脚本](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-add-zh.png) | ![脚本设置](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-settings-zh.png) |
| **加入脚本**——静态检测参数，勾选即纳管 | **脚本设置**——参数、机密、组合、依赖 |

<p align="center">
  <img width="480" alt="只用鼠标操作 skit——画面上每个控件都是可点击的目标" src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/demo-mouse.gif"><br>
  <em>完全鼠标可操作性——画面上每个按键提示，也都是可点的按钮。</em>
</p>

## 脚本语言支持

Python、shell、JS/TS 支持最深——既静态检测参数，**也能注入值**；其余类型则负责启动、接受声明式参数，并读懂脚本自己的 CLI 解析器。

| 类型 | 运行方式 | 检测参数 | 注入 | 读脚本自己的 CLI | 依赖 / needs |
| --- | --- | --- | --- | --- | --- |
| **Python** | `uv run --script` | 常量、`input()` | ✅ | argparse · click · typer | PEP 723（uv）+ needs |
| **Shell**（bash/sh/zsh） | 解释器 | 常量、`${VAR:-}` 环境默认值、`read` | ✅ | getopts | needs |
| **JS / TS** | deno › bun › node | `const` | ✅ | `util.parseArgs` | npm（按脚本）+ needs |
| **fish** | fish | `set -q NAME; or set NAME …` 环境默认值 | — | `argparse` 内建 | needs |
| **PowerShell** | pwsh | — | — | `param()` | needs |
| **Ruby · Perl · Lua · R** | 解释器 | — | — | — | needs |
| **程序**（exe） | 直接执行 | — | — | — | needs |
| **命令**（command） | 填充模板 | — | — | — | needs |

每种类型也都能手动**声明**参数，所以连程序和命令模板都享有同样的表单 / 组合 / `--set` 体验。**needs** 是 skit 在运行前检查是否位于 `PATH` 上的外部命令（任何类型都适用）。Python 和 JS/TS 都有按脚本隔离的依赖包：uv 解析 PEP 723 块，npm 式依赖则安装到库内副本旁的 `node_modules`（`skit add` 会从脚本自己的 import 建议清单）。JS/TS 依赖管理仅适用于复制进库的条目——reference 模式的脚本沿用自己项目的 `node_modules`——且安装一律不执行包的 lifecycle scripts（npm 和 bun 加 `--ignore-scripts`；deno 默认就不跑）。另一个拉平差异的点：skit 自动选中 deno 时会带 `--allow-all`，同一个脚本在 deno、bun、node 下行为一致。skit 会替 Python 引导 uv，但不会替你安装 JS runtime——node、bun 或 deno 需要你自己准备好。

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

## 卸载

```bash
uv tool uninstall skit-cli
```

这会移除 skit 本身与它在 `PATH` 上的快捷方式。你的脚本库与设置存在包**之外**，所以会刻意保留——重装一次，一切照旧。想连这些也一并清掉，就删掉 skit 自己的目录：

| 操作系统 | 目录 |
| --- | --- |
| **macOS** | `~/Library/Application Support/skit` |
| **Linux** | `~/.local/share/skit` · `~/.local/state/skit` · `~/.config/skit` |
| **Windows** | `%LOCALAPPDATA%\skit` |

这些目录装着你的脚本库、设置、参数组合与上次的值——以及，若 skit 曾自行下载过 uv，那份私有的 `uv` 可执行文件（在 `…/skit/bin`，会跟着一起删掉）。

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux——你若设过 XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME，会按你的设置
rm -rf ~/.local/share/skit ~/.local/state/skit ~/.config/skit
```

```powershell
# Windows（PowerShell）
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

不确定在哪？`skit doctor` 会打印出实际的脚本库路径（也会尊重 `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` 这几个覆写变量）。这就是 skit 拥有的全部——它从不动你的 `PATH`、shell 或全局 uv 配置，所以没别的要善后。uv 的下载缓存，以及 uv 拉取的 Python 版本，是跟你整套 uv 环境共用的，不归 skit 删；你若没在别处用 uv、又想把空间拿回来，`uv cache clean` 可以清掉缓存。

## 用法

整个界面，就两条命令：

```bash
skit add my_script.py   # 加入脚本
skit                    # 打开菜单，选脚本，填表单，跑
```

其余一切都在 TUI 里完成——都在画面上，鼠标键盘皆可，什么都不用背。

剩下的 CLI 是给自动化和 AI agent 准备的——每个 TUI 动作都能脚本化：

```bash
skit run my_script -p fast    # 用已存的组合执行
skit run my_script --dry-run  # 打印实际会跑的命令，不真的执行
skit run my_script --set width=800 --no-input   # 直接指定参数值，从不询问
skit show my_script --json    # 一个脚本的完整参数结构，机器可读
skit params my_script         # 查看纳管参数与上次的值
skit deps my_script --dep "requests>=2"   # 设置一个脚本的依赖包
skit list --json              # 机器可读的脚本清单
skit config                   # 设置：语言、编辑器、镜像、表单样式
skit --help                   # 其余一切
```

## 给你的 AI agent 用

skit 是给人类和 AI agent 共用的脚本仓库：同一个库——你用表单，agent 用确定性的
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

skit 自动跟随系统语言；想换，在 TUI 偏好设置里改（自动化场景用 `skit config lang zh-CN`，或 `SKIT_LANG=en skit` 只切这一次）。

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
