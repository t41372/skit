<img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png" alt="skit — script launcher and parameter manager" width="750">

[![CI](https://github.com/t41372/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/t41372/skit/branch/main/graph/badge.svg)](https://codecov.io/gh/t41372/skit)
[![CodSpeed](https://img.shields.io/endpoint?url=https://codspeed.io/badge.json)](https://app.codspeed.io/t41372/skit?utm_source=badge)
[![Mutation tested: cargo-mutants](https://img.shields.io/badge/mutation%20tested-cargo--mutants-blue)](https://github.com/sourcefrog/cargo-mutants)
[![PyPI](https://img.shields.io/pypi/v/skit-cli)](https://pypi.org/project/skit-cli/)
[![Rust 1.97.1](https://img.shields.io/badge/rust-1.97.1-orange)](https://www.rust-lang.org/)
[![TUI: Ratatui](https://img.shields.io/badge/TUI-Ratatui-blue)](https://ratatui.rs/)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

**English** | [繁體中文](./README.zh-TW.md) | [简体中文](./README.zh-CN.md)

**skit is a script manager and launcher in your terminal.**

skit keeps your scripts in one place and makes them painless to launch — Python, shell, JS/TS, executables, prompts, and more.

It is painless because skit reads each script and turns its CLI flags, `input()` calls, and hard-coded constants into a launch menu with descriptions — you edit inputs and variables on screen, without ever touching the script.

So you can finally stop worrying about where to store your script and whether you still know how to use it next year — put it in skit and forget about it.

There are exactly two commands to remember:

```bash
skit add script.py   # put a script in the library
skit                 # open the menu — pick, fill in the inputs, run
```

Not a terminal person? That is fine — skit works like a GUI app that happens to live in your terminal: point, click, and type your way through everything. Hotkeys exist too — hinted right on screen — but there is nothing to learn or memorize.

Your AI agent gets the same library: you use it from a menu, agents use it through a deterministic CLI and a skill, so scripts get saved and reused.

### The interface

<p align="center">
  <img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-library-en.png" alt="skit library" width="49%">
  <img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-form-en.png" alt="skit run form" width="49%">
  <img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-add-en.png" alt="skit Add workflow" width="49%">
  <img src="https://raw.githubusercontent.com/t41372/skit/main/docs/assets/tui-settings-en.png" alt="skit settings" width="49%">
</p>

## What it does

- **One home for your scripts and prompts.** `skit add` collects scattered scripts and prompts into one searchable library.
- **No flags to memorize, no opening an editor just to change a value.** Flags, `input()` calls, and the constants you select become typed and described fields in the launch menu. The Ratatui form validates booleans, choices, numbers, and paths when you submit it.
- **Remembers your last inputs.** Input parameters in the launch menu come back prefilled on the next run. Clear an override to use the script's current default. Save favorites as named presets — tokens like `{cwd}` and `{today}` keep them portable. Parameters marked secret are never saved: not in last-used values, presets, or run history.
- **No environment mess.** Python scripts declare their dependencies inline (PEP 723) and run via uv in isolated environments; JS/TS scripts get a per-script `node_modules`, installed from their declared packages on first run. Nothing global either way. Other languages use the tools already on your machine — skit checks that declared external commands are on your `PATH` before running.
- **Prompts as scripts.** Store a parameterized prompt (managed `{{placeholders}}` become input fields) and launch it with your coding agent — claude, codex, opencode, or any prompt runner you like.
- **Mouse or keyboard, in your language.** Plain `skit` opens the full TUI; every key hint on screen is also a clickable button. Ships in English, 繁體中文, and 简体中文 ([Languages](#languages)).
- **Built for AI agents too.** Every TUI action is also a CLI command with `--json` output and meaningful exit codes, and the official [Agent Skill](https://agentskills.io) teaches Claude Code, Codex, Cursor, Gemini CLI, and friends to check your library first, run what is there, and save the good ones — see [Works with your AI agent](#works-with-your-ai-agent).

| Problem | What skit does |
| --- | --- |
| Scripts scattered all over the place | One central menu, with search |
| Scripts that need specific packages or tools | Per-script dependencies for Python (PEP 723 + uv) and JS/TS (npm); for any language, skit checks declared external commands on your `PATH` |
| CLI flags you do not remember, `input()` prompts, constants meant to be edited by hand | Static analysis turns them all into an interactive menu — no code changes, no config. Last-used values come prefilled; favorites save as presets. |
| The weird script an AI wrote for you gets lost with the chat session | Agents check the library first, reuse what is there, and save the keepers — one-off scripts become permanent, parameterized tools |

No need to modify your script for skit — we will take care of it, and will ask you interactively when we need to.

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

No auto-detection for your type? Declare parameters by hand — every type gets the same launch menu / preset / `--set` experience, even a plain executable (declared values are passed as ordinary command-line arguments). Any entry can also list the external commands it depends on (`ffmpeg`, `jq`, …); skit checks they are on your `PATH` before each run.

Python and JS/TS get isolated per-script package dependencies: uv resolves the PEP 723 block, and npm-style deps install into a `node_modules` next to the stored copy — installs never run package lifecycle scripts. The finer points (copied vs referenced entries, deno's `--allow-all` behavior) are in the [docs](https://t41372.github.io/skit/en/docs/script-types/).

skit bootstraps uv for Python, but never a JS runtime — you supply node, bun, or deno.

### Prompts

A prompt entry stores reusable text for an AI coding agent. Add a `.prompt.md` file, or use
`skit add --prompt` to create one. skit can make up to 30 detected `{{placeholders}}` into input
fields. If the source has more than 30 placeholders, skit does not manage any of them by default.
This limit prevents skit from treating code samples as variables. Use Entry settings or
`skit params` to select the fields that you want to manage. Managed fields support presets,
last-used values, and `--set`.

skit does not use escape sequences for prompts. It sends all unmanaged text byte-for-byte. This
includes unmanaged `{{holes}}`. Pi can require one exception. skit warns you and adds one newline
when Pi can parse the opening text as an option, file, or package command. Use `--no-interpolate`
to turn off all insertion for one prompt.

Select the **runner** in the launch menu, or pin one runner for the prompt. skit includes claude,
codex, opencode, amp, antigravity, copilot, cursor, and pi. Use `skit runner add` to register a
different CLI. Do not use prompts for secrets. The receiving agent can store the rendered text in
its session logs. Read the [prompt documentation](https://t41372.github.io/skit/en/docs/prompts/)
for runner behavior, non-interactive selection, the Pi fallback, and the no-shell delivery rule.

```bash
skit add review.prompt.md            # managed placeholders become input fields
skit run review                      # pick the agent, fill in the inputs, go
skit run review --runner codex --set target=src/app.py --no-input
```

## Install

skit uses [uv](https://docs.astral.sh/uv/) 0.12.3. If uv is not installed, skit asks for
consent and then downloads this version into its private directory. skit does not change your
`PATH` or global environment. A system-wide
[install](https://docs.astral.sh/uv/getting-started/installation/) is still preferred.

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
skit --version             # the version you are on
```

`uv tool upgrade` follows whatever source you installed from: PyPI installs track PyPI releases, `git+…` installs re-fetch the main branch.

## Upgrade from 0.4

Version 0.5 replaces the Python and Textual implementation with Rust, Ratatui, and Crossterm. Run
`uv tool upgrade skit-cli`; no export or import is necessary. skit reads the same library,
configuration, presets, remembered values, and metadata in place. It keeps unknown TOML fields and
does not rewrite data during startup.

The terminal workflow is more direct. Search now uses case-insensitive substring matching. Forms
use one text editor for every scalar type and validate values when you submit. The first Rust
release does not show the old checkbox, choice-list, or file-browser widgets. The add screen is one
form; use the settings screen after the add to manage detected source fields. These presentation
changes do not change stored scripts or state. See the [cutover design](./docs/design/rust-rewrite.md)
for the complete compatibility boundary.

## Usage

Two commands are the whole interface:

```bash
skit add my_script.py   # add a script
skit add                # not sure what you are adding? it asks
skit                    # open the menu, pick it, fill in the inputs, run
```

Everything else happens inside the TUI — on screen, mouse or keyboard, nothing to memorize.

The rest of the CLI exists for automation and AI agents — every TUI action, scriptable:

```bash
skit run my_script -p fast    # run with a saved preset
skit run my_script --dry-run  # print the exact command, do not run it
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
check your library before writing yet another one-off script, to inspect and run what is
already there, and to offer to save the useful scripts they write — so they outlive the
session that created them.

```bash
skit agent install            # use the sole detected agent directory; otherwise, select one
skit agent install claude     # select claude / codex / agents (--project limits it to this repo)
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

That removes skit and its `PATH` shim. Your library and settings live **outside** the package, so they survive on purpose — reinstall and you are right back where you left off. To erase those too, delete skit's own directories:

| OS | Directories |
| --- | --- |
| **macOS** | `~/Library/Application Support/skit` |
| **Linux** | `~/.local/share/skit` · `~/.local/state/skit` · `~/.config/skit` |
| **Windows** | `%LOCALAPPDATA%\skit` |

They hold your tool library, config, presets, and last-used values — plus, if skit ever bootstrapped its own uv, the private `uv` binary (in `…/skit/bin`, deleted along with the rest).

```bash
# macOS
rm -rf ~/Library/Application\ Support/skit

# Linux — honors XDG_DATA_HOME / XDG_STATE_HOME / XDG_CONFIG_HOME if you have set them
rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/skit" "${XDG_STATE_HOME:-$HOME/.local/state}/skit" "${XDG_CONFIG_HOME:-$HOME/.config}/skit"
```

```powershell
# Windows (PowerShell)
Remove-Item -Recurse -Force $env:LOCALAPPDATA\skit
```

Not sure where yours landed? `skit doctor` prints the resolved library path (and respects any `SKIT_DATA_DIR` / `SKIT_STATE_DIR` / `SKIT_CONFIG_DIR` overrides). That is everything skit owns — it never writes to your `PATH`, shell, or global uv config, so nothing else needs undoing. The uv download cache and any Python builds uv fetched are shared with the rest of your uv setup, not skit's to remove; if you do not use uv elsewhere and want the space back, `uv cache clean` clears the cache.

## Why skit exists

skit began as an answer to [a linux.do forum thread](https://linux.do/t/topic/2512255) (in Chinese).

## Development

Development uses the pinned Rust toolchain. See [CONTRIBUTING.md](./CONTRIBUTING.md) for the TDD
workflow and the complete coverage, cargo-mutants, supply-chain, wheel, documentation, and benchmark
gates.

```bash
cargo build --locked
cargo test --locked --workspace --all-targets --all-features
cargo run -p skit-cli-rs -- --help
```

## License

[MIT](LICENSE)
