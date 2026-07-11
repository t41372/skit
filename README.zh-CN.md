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

**skit 是 Python 脚本的启动器，也是它们的集中收纳处。**

**AI 写脚本，skit 管脚本。**

<video src="https://github.com/user-attachments/assets/bd4f78ba-0a1f-4b73-b02e-bc1dc1d11c4c" controls></video>

## 它做什么

- **收纳脚本**。`skit add` 把散落各处的脚本收进同一个可搜索的脚本库——复制一份收进库里，或用引用模式直接指向原始文件。
- **参数不再折磨人**。命令行参数、`input()`、你勾选纳管的常量，全部变成表单字段（choices 变选择器、布尔变复选框、类型自动把关）。
- **它会记住**。上次填的值自动带回；常用的一组存成命名组合。标记为机密的参数永不落盘。`{cwd}`、`{today}` 这类值 token 让同一个组合跨机器、跨目录通用。
- **环境零污染**。skit 把每个脚本的依赖用标准 PEP 723 语法声明在脚本开头，运行时由 uv 在隔离、带缓存的环境里解析——你不用管 venv，也不会往全局装任何东西。
- **鼠标键盘皆可**。直接运行 `skit` 就是完整 TUI；画面上每个快捷键提示同时也是一个可点的按钮。
- **天生适合自动化**。每个 TUI 动作都有对应的 CLI 命令，带 `--json` 输出和明确退出码——shell 脚本、CI、AI agent 都好接。
- **多语言支持**。English、繁體中文、简体中文，更多语言在路上。见[语言](#语言)。


| 痛点 | skit 的解法 |
| --- | --- |
| 脚本东一个西一个，散落在各个文件夹 | 全部收进同一个菜单，带搜索 |
| 脚本带着一堆奇怪的第三方依赖 | 每个脚本一个隔离环境——依赖以 PEP 723 声明在脚本开头，由 uv 解析 |
| 命令行参数转头就忘、`input()` 一项项问、常量写死在源码里，改个值都得开编辑器 | 静态分析把参数统统读出来，变成一张交互表单——源码一行不动。上次的值自动带回；常用的存成组合（preset） |

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

墙内有三处下载容易卡住：PyPI 包、uv 从 GitHub 拉取的 Python 构建，还有 skit 引导下载的 uv 本体。skit 可以让三者都走国内镜像。

镜像设置只存在 skit 里：不碰你的全局 uv 配置；你已经自己配好的镜像（`UV_DEFAULT_INDEX`、`uv.toml` 等）也不会被覆盖。

- **首次运行**：检测到 PyPI/GitHub 连不上时，skit 会主动问要不要开镜像——按个回车就好。
- **随时**：TUI「偏好设置 → 镜像」，或：

```bash
skit config mirror tsinghua   # 或：aliyun / ustc / custom / off
```

默认：PyPI → 清华 / 阿里云 / 中科大；Python 构建与 uv 二进制 → 南京大学。哪个镜像挂了，选 `custom` 就能逐项换地址。

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
