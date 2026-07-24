<img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png" alt="skit — script launcher and parameter manager" width="750">

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

**English** | [繁體中文](./README.zh-TW.md) | [简体中文](./README.zh-CN.md)

**skit is a script manager and launcher in your terminal.**

skit keeps your scripts in one place and makes them painless to launch — Python, shell, JS/TS, executables, prompts, and more.

It's painless because skit reads each script and turns its CLI flags, `input()` calls, and hard-coded constants into a launch menu with descriptions — you edit inputs and variables on screen, without ever touching the script.

So you can finally stop worrying about where to store your script and whether you still know how to use it next year — put it in skit and forget about it.

There are exactly two commands to remember:

```bash
skit add script.py   # put a script in the library
skit                 # open the menu — pick, fill in the inputs, run
```

Your AI agent gets the same library: you use it from a menu, agents use it through a deterministic CLI and a skill, so scripts get saved and reused.

<video src="https://github.com/user-attachments/assets/8a1f27cd-f2f2-42db-977a-b4f8ea207340" controls></video>

## What it does

- **One home for your scripts and prompts.** `skit add` collects scattered scripts and prompts into one searchable library (with fuzzy search).
- **No flags to memorize, no opening an editor just to change a value.** Flags, `input()` calls, and the constants you select become fields in the launch menu — typed, described, automatic. Choices turn into pickers, booleans into checkboxes; paths complete as you type and can open a file browser.
- **Remembers your last inputs.** Input parameters in the launch menu come back prefilled on the next run; `↺ default` (Ctrl+O) restores the script's own default. Save favorites as named presets — tokens like `{cwd}` and `{today}` keep them portable. Parameters marked secret are never saved: not in last-used values, presets, or run history.
- **No environment mess.** Python scripts declare their dependencies inline (PEP 723) and run via uv in isolated environments; JS/TS scripts get a per-script `node_modules`, installed from their declared packages on first run. Nothing global either way. Other languages use the tools already on your machine — skit checks that declared external commands are on your `PATH` before running.
- **Prompts as scripts.** Store a parameterized prompt (managed `{{placeholders}}` become input fields) and launch it with your coding agent — claude, codex, opencode, or any prompt runner you like.
- **Mouse or keyboard, in your language.** Plain `skit` opens the full TUI; every key hint on screen is also a clickable button — you don't need to be a terminal expert. Ships in English, 繁體中文, and 简体中文 ([Languages](#languages)).
- **Built for AI agents too.** Every TUI action is also a CLI command with `--json` output and meaningful exit codes, and the official [Agent Skill](https://agentskills.io) teaches Claude Code, Codex, Cursor, Gemini CLI, and friends to check your library first, run what's there, and save the good ones — see [Works with your AI agent](#works-with-your-ai-agent).

| Problem | What skit does |
| --- | --- |
| Scripts scattered all over the place | One central menu, with search |
| Scripts that need specific packages or tools | Per-script dependencies for Python (PEP 723 + uv) and JS/TS (npm); for any language, skit checks declared external commands on your `PATH` |
| CLI flags you don't remember, `input()` prompts, constants meant to be edited by hand | Static analysis turns them all into an interactive menu — no code changes, no config. Last-used values come prefilled; favorites save as presets. |
| The weird script an AI wrote for you gets lost with the chat session | Agents check the library first, reuse what's there, and save the keepers — one-off scripts become permanent, parameterized tools |

No need to modify your script for skit — we will take care of it, and will ask you interactively when we need to.

| ![The library menu](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-library-en.png) | ![The launch menu](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-form-en.png) |
|:--:|:--:|
| **The library** — every action on screen, mouse or keyboard | **The launch menu** — generated from the script's own parameters |
| ![Adding a script](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-add-en.png) | ![Script settings](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-settings-en.png) |
| **Adding a script** — parameters detected statically; you choose which ones skit manages | **Script settings** — parameters, secrets, presets, dependencies |

<p align="center">
  <img width="480" alt="Driving skit with the mouse alone — every control on screen is a click target" src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/demo-mouse.gif"><br>
  <em>Fully mouse operable — every key hint on screen is also a button.</em>
</p>

## Supported script types

Python, shell, and JS/TS get the fullest support: skit finds their parameters by reading the code. Every other type launches out of the box.

| Script type | How it runs | Supported parameter detection |
| --- | --- | --- |
| **Python** | uv (`uv run --script`) | CLI flags (argparse · click · typer), `input()` prompts, constants |
| **Shell** (bash/sh/zsh) | the matching shell | CLI flags (getopts), `read` prompts, constants, `${VAR:-}` defaults |
| **JS / TS** | deno, bun, or node — first found | CLI flags (`util.parseArgs`), `const` values |
| **fish** | fish | CLI flags (`argparse`), `set -q` env-defaults |
| **PowerShell** | pwsh | `param()` definitions |
| **Ruby · Perl · Lua · R** | their interpreter | — |
| **Executables** | run directly | — |
| **Command templates** | skit fills the blanks, runs the command | — |
| **Prompts** | your coding agent (claude · codex · …) | `{{placeholders}}` |

No auto-detection for your type? Declare parameters by hand — every type gets the same launch menu / preset / `--set` experience, even a plain executable (declared values are passed as ordinary command-line arguments). Any entry can also list the external commands it depends on (`ffmpeg`, `jq`, …); skit checks they're on your `PATH` before each run.

Python and JS/TS get isolated per-script package dependencies: uv resolves the PEP 723 block, and npm-style deps install into a `node_modules` next to the stored copy — installs never run package lifecycle scripts. The finer points (copied vs referenced entries, deno's `--allow-all` behavior) are in the [docs](https://t41372.github.io/skit/en/docs/script-types/).

skit bootstraps uv for Python, but never a JS runtime — you supply node, bun, or deno.

### Prompts

A prompt entry is a reusable, parameterized piece of text for an AI coding agent. Add a `.prompt.md` file (or draft one with `skit add --prompt`); in interactive add review, choose which detected `{{placeholders}}` become input fields. Up to 30 are selected by default; when detection exceeds 30, none are selected by default, so code samples are not mistaken for variables. Managed fields get the full preset / last-values / `--set` experience.

There are no escape sequences to learn: anything you don't manage — unmanaged `{{holes}}` included — reaches the agent byte-for-byte, and a per-prompt switch (`--no-interpolate`) turns insertion off entirely. The **runner** is picked in the launch menu (or pinned per prompt); claude / codex / opencode / amp / antigravity / copilot / cursor / pi come preconfigured, and `skit runner add` registers any other CLI. One honest caveat: prompts are not a secrets channel — the rendered text ends up in the receiving agent's own session logs. Runner behaviors, non-interactive resolution rules, and the no-shell delivery guarantee: [docs](https://t41372.github.io/skit/en/docs/prompts/).

```bash
skit add review.prompt.md            # managed placeholders become input fields
skit run review                      # pick the agent, fill in the inputs, go
skit run review --runner codex --set target=src/app.py --no-input
```

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

## Usage

Two commands are the whole interface:

```bash
skit add my_script.py   # add a script
skit add                # not sure what you're adding? it asks
skit                    # open the menu, pick it, fill in the inputs, run
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
TUI, agents get a deterministic CLI. The official [Agent Skill](https://agentskills.io)
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

skit follows your system language; switch it in the TUI preferences (for automation: `skit config lang zh-TW`, or `SKIT_LANG=zh-CN skit` for one run). Want another language? Open an issue or PR.

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

They hold your tool library, config, presets, and last-used values — plus, if skit ever bootstrapped its own uv, the private `uv` binary (in `…/skit/bin`, deleted along with the rest).

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux — honors XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME if you've set them
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/skit" "${XDG_STATE_HOME:-$HOME/.local/state}/skit" "${XDG_CONFIG_HOME:-$HOME/.config}/skit"
```

```powershell
# Windows (PowerShell)
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

Not sure where yours landed? `skit doctor` prints the resolved library path (and respects any `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` overrides). That's everything skit owns — it never writes to your `PATH`, shell, or global uv config, so nothing else needs undoing. The uv download cache and any Python builds uv fetched are shared with the rest of your uv setup, not skit's to remove; if you don't use uv elsewhere and want the space back, `uv cache clean` clears the cache.

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
