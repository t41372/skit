![skit — script launcher and parameter manager](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png)

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

**English** | [繁體中文](./README.zh-TW.md) | [简体中文](./README.zh-CN.md)

**skit is a launcher and a home for your scripts.**

skit stores your scripts in one place and makes them painless to launch.

**AI writes the scripts. skit gives them a home.**

And it's a home you share with your agent: the library you drive from a menu, AI agents
drive through a deterministic CLI — checking it before writing yet another one-off
script, and (with your OK) saving the good ones back, so they outlive the chat.

<video src="https://github.com/user-attachments/assets/d27121fe-5855-4270-91b0-b0ee9e5d11ee" controls></video>

## What it does

- **One home for your scripts.** `skit add` collects scattered scripts into a searchable library — keep a copy in the library, or reference the original file.
- **Parameters without the pain.** Flags, `input()` calls, and the constants you tick become form fields (choices → pickers, booleans → checkboxes, types enforced).
- **It remembers.** Last-used values come back automatically; save favorites as named presets. Parameters marked secret never touch disk. Tokens like `{cwd}` and `{today}` keep presets portable.
- **No environment mess.** Python scripts declare their dependencies inline (PEP 723) and run via uv in isolated environments; JS/TS scripts get a per-script `node_modules`, installed from their declared packages on first run. Nothing global either way. Other languages use the tools already on your machine — skit checks that declared external commands are on your `PATH` before running.
- **Mouse or keyboard.** Plain `skit` opens the full TUI; every key hint on screen is also a clickable button.
- **Automation-ready.** Every TUI action is also a CLI command with `--json` output and meaningful exit codes — for shell scripts, CI, and AI agents.
- **Your agent's script library too.** The official [Agent Skill](https://agentskills.io) teaches Claude Code, Codex, Cursor, Gemini CLI, and friends the whole drill: discover scripts with `skit list`, read a parameter schema with `skit show`, run with `skit run --set … --no-input`. One `skit agent install` away — see [Works with your AI agent](#works-with-your-ai-agent).
- **Speaks your language.** English, 繁體中文, and 简体中文, with more to come. See [Languages](#languages).

| Problem | What skit does |
| --- | --- |
| Scripts scattered all over the place | One central menu, with search |
| Scripts that need specific packages or tools | Per-script dependencies for Python (PEP 723 + uv) and JS/TS (npm); for any language, skit checks declared external commands on your `PATH` |
| CLI flags you forget ten minutes later, `input()` prompts, hard-coded constants meant to be edited by hand | Static analysis extracts them all into an interactive form — no code changes. Last-used values come prefilled; favorites save as presets. |
| The weird script an AI wrote for you dies with the chat session | Agents check the library first, reuse what's there, and save the keepers — one-off scripts become permanent, parameterized tools |

Nothing to set up per script — no refactoring, no config to maintain. The script an AI wrote last week and the one you barely remember from last year launch the same way.

| ![The library menu](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-library-en.png) | ![The run form](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-form-en.png) |
|:--:|:--:|
| **The library** — every action on screen, mouse or keyboard | **The run form** — generated from the script's own parameters |
| ![Adding a script](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-add-en.png) | ![Script settings](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-settings-en.png) |
| **Adding a script** — parameters detected statically, tick to manage | **Script settings** — parameters, secrets, presets, dependencies |

<p align="center">
  <img width="480" alt="Driving skit with the mouse alone — every control on screen is a click target" src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/demo-mouse.gif"><br>
  <em>Fully mouse operable — every key hint on screen is also a button.</em>
</p>

## Language support

Python, shell, and JS/TS get static parameter detection **and** value injection. The rest launch out of the box, accept declared parameters, and work with their own CLI parsers.

| Kind | Runs via | Params detected | Injection | Reads its own CLI | Deps / needs |
| --- | --- | --- | --- | --- | --- |
| **Python** | `uv run --script` | constants, `input()` | ✅ | argparse · click · typer | PEP 723 (uv) + needs |
| **Shell** (bash/sh/zsh) | interpreter | constants, `${VAR:-}` env-defaults, `read` | ✅ | getopts | needs |
| **JS / TS** | deno › bun › node | `const` | ✅ | `util.parseArgs` | npm (per script) + needs |
| **fish** | fish | `set -q NAME; or set NAME …` env-defaults | — | `argparse` builtin | needs |
| **PowerShell** | pwsh | — | — | `param()` | needs |
| **Ruby · Perl · Lua · R** | interpreter | — | — | — | needs |
| **Programs** (exe) | direct exec | — | — | — | needs |
| **Commands** | template fill | — | — | — | needs |

You can also declare parameters by hand for any kind — so even plain executables and command templates get the same form / preset / `--set` experience. **needs** are external commands — skit checks they're on your `PATH` before each run (any kind). Python and JS/TS get isolated per-script package dependencies: uv resolves the PEP 723 block, and npm-style deps install into a `node_modules` next to the stored copy (`skit add` suggests them from the script's own imports). Managed JS/TS deps apply to copied entries — a referenced script keeps using its own project's `node_modules` — and installs never run package lifecycle scripts (npm and bun get `--ignore-scripts`; deno skips them by default). One more evener: when deno is the runner skit picks, it passes `--allow-all`, so the same script behaves the same under deno, bun, and node. skit bootstraps uv for Python, but never a JS runtime — you supply node, bun, or deno.

## Install

skit is built on [uv](https://docs.astral.sh/uv/) (tested against 0.11.26). Don't have it? skit asks first, then downloads a pinned uv into its own private directory — your `PATH` and global environment stay untouched. A system-wide [install](https://docs.astral.sh/uv/getting-started/installation/) is still preferred.

```bash
# Install skit with uv tool from PyPI (the package is named skit-cli; the command is skit)
uv tool install skit-cli
```


> **In mainland China?** Set the mirror by hand for this one command (details in [Mainland China (中国大陆)](#mainland-china-中国大陆)):
>
> ```bash
> export UV_DEFAULT_INDEX=https://pypi.tuna.tsinghua.edu.cn/simple
> uv tool install skit-cli
> ```

Or install the latest dev version from the main branch.

```bash
uv tool install git+https://github.com/t41372/skit          # latest development version
uvx --from git+https://github.com/t41372/skit skit --help   # try it without installing
```

## Update

```bash
uv tool upgrade skit-cli   # update to the latest release — also how you "check": it says up to date if you are
skit --version             # the version you're on
```

`uv tool upgrade` follows whatever source you installed from: PyPI installs track PyPI releases, `git+…` installs re-fetch the main branch.

## Uninstall

```bash
uv tool uninstall skit-cli
```

That removes skit and its `PATH` shim. Your library and settings live **outside** the package, so they survive on purpose — reinstall and you're right back where you left off. To erase those too, delete skit's own directories:

| OS | Directories |
| --- | --- |
| **macOS** | `~/Library/Application Support/skit` |
| **Linux** | `~/.local/share/skit` · `~/.local/state/skit` · `~/.config/skit` |
| **Windows** | `%LOCALAPPDATA%\skit` |

They hold your script library, config, presets, and last-used values — plus, if skit ever bootstrapped its own uv, the private `uv` binary (in `…/skit/bin`, deleted along with the rest).

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux — honors XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME if you've set them
rm -rf ~/.local/share/skit ~/.local/state/skit ~/.config/skit
```

```powershell
# Windows (PowerShell)
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

Not sure where yours landed? `skit doctor` prints the resolved library path (and respects any `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` overrides). That's everything skit owns — it never writes to your `PATH`, shell, or global uv config, so nothing else needs undoing. The uv download cache and any Python builds uv fetched are shared with the rest of your uv setup, not skit's to remove; if you don't use uv elsewhere and want the space back, `uv cache clean` clears the cache.

## Usage

Two commands are the whole interface:

```bash
skit add my_script.py   # add a script
skit                    # open the menu, pick it, fill in the form, run
```

Everything else happens inside the TUI — on screen, mouse or keyboard, nothing to memorize.

The rest of the CLI exists for automation and AI agents — every TUI action, scriptable:

```bash
skit run my_script -p fast    # run with a saved preset
skit run my_script --dry-run  # print the exact command, don't run it
skit run my_script --set width=800 --no-input   # set values explicitly, never prompt
skit show my_script --json    # one script's full parameter schema, machine-readable
skit params my_script         # show managed parameters and last-used values
skit deps my_script --dep "requests>=2"   # set a script's package dependencies
skit list --json              # machine-readable listing
skit config                   # settings: language, editor, mirror, form style
skit --help                   # everything else
```

## Works with your AI agent

skit is a script repository for humans *and* AI agents: one library — you get the
forms, agents get a deterministic CLI. The official [Agent Skill](https://agentskills.io)
teaches compatible agents (Claude Code, Codex, Cursor, Gemini CLI, and many more) to
check your library before writing yet another one-off script, to inspect and run what's
already there, and to offer to save the useful scripts they write — so they outlive the
session that created them.

```bash
skit agent install            # pick one of the agent directories found on your machine
skit agent install claude     # or name it: claude / codex / agents (--project for this repo only)
npx skills add t41372/skit    # or install through skills.sh into 70+ agents
```

## Languages

| Language | Status |
| --- | --- |
| English | ✅ 100%, human-reviewed |
| 繁體中文 (zh-TW) | ✅ 100%, human-reviewed |
| 简体中文 (zh-CN) | ✅ 100%, human-reviewed |

skit follows your system language; switch it in the TUI preferences (for automation: `skit config lang zh-TW`, or `SKIT_LANG=zh-CN skit` for one run).

## Mainland China (中国大陆)

Four downloads tend to fail in mainland China: PyPI packages, npm packages, the Python builds uv fetches from GitHub, and skit's own uv bootstrap. skit can route all four through domestic mirrors.

Mirror settings live inside skit only: your global uv config is never touched, and existing mirror settings (`UV_DEFAULT_INDEX`, `uv.toml`, …) are respected. The npm registry rides `NPM_CONFIG_REGISTRY`: an existing value of that variable in your environment still wins, but note npm itself ranks it above `~/.npmrc`.

Each ecosystem is its own, independent choice — mirror vendors differ per ecosystem, so no single vendor name spans them:

- **First run**: if PyPI/GitHub look unreachable, skit offers mirror setup — one question per ecosystem, Enter accepts each one's recommended preset.
- **Any time**: TUI Preferences → mirrors, or:

```bash
skit config mirror.pypi tsinghua    # Python packages: tsinghua / aliyun / ustc / a URL / off
skit config mirror.github nju       # Python builds + the uv binary: nju / an https:// base URL / off
skit config mirror.npm npmmirror    # JS/TS packages: npmmirror / a URL / off
skit config mirror off              # master switch: off keeps the URLs; `on` restores them
```

Custom URLs: pick `custom` in TUI Preferences (or the first-run wizard), or pass a URL to the axis key directly.

## Why skit exists

skit began as an answer to [a linux.do forum thread](https://linux.do/t/topic/2512255) (in Chinese).

## Development

Development runs entirely on uv — see [CONTRIBUTING.md](./CONTRIBUTING.md) for the full workflow and quality gates (ruff, ty strict, 100% test coverage, mutation testing with mutmut, zizmor-audited workflows).

```bash
uv sync --dev
uv run pytest -q
uv run python scripts/serve_preview.py   # TUI web preview (textual-serve, localhost:8000)
```

## License

[MIT](LICENSE)
