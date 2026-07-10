![skit — a centralized launcher for your weird scripts](https://raw.githubusercontent.com/t41372/skit/main/docs/assets/banner.png)

[![CI](https://github.com/user/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![Coverage: 100%](https://img.shields.io/badge/coverage-100%25-brightgreen)](https://github.com/t41372/skit/actions/workflows/ci.yml)
[![Mutation tested: mutmut](https://img.shields.io/badge/mutation%20tested-mutmut-blue)](https://github.com/boxed/mutmut)
[![PyPI](https://img.shields.io/pypi/v/skit)](https://pypi.org/project/skit/)
[![Python 3.12+](https://img.shields.io/badge/python-3.12%2B-blue)](https://www.python.org/)
[![Ruff](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/astral-sh/ruff/main/assets/badge/v2.json)](https://github.com/astral-sh/ruff)
[![Types: ty](https://img.shields.io/badge/types-ty-261230.svg)](https://github.com/astral-sh/ty)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

**English** | [中文](./README.zh-TW.md)

Skit is a script launcher and parameter manager. If you have Python scripts scattered everywhere with parameters hard-coded in the source, skit means you never again open an editor to tweak a constant, memorize CLI flags, or babysit virtualenvs — open the menu, pick a script, fill in a form, run.

## What it does

- **One home for your scripts.** `skit add` collects Python scripts, executables, and command templates into a single place. Copy mode preserves your original file byte-for-byte; reference mode never touches it.
- **Any script becomes a form.** skit reads however your script takes input — hard-coded constants and `input()` calls (detected via AST), or an `argparse` / `click` / `typer` command line (read statically) — and turns it into one form to fill in. You never memorize flags or open an editor to tweak a constant again. Values are delivered without changing your source: injected into a temporary copy, or passed as flags at run time.
- **It remembers.** Last-used values are saved automatically. `preset` stores named parameter sets. Secret parameters are structurally prevented from ever touching disk.
- **No environment management.** Scripts run through `uv run --script` with dependencies declared via PEP 723. If uv is missing, skit downloads a private copy for itself (see below).
- **TUI-first, CLI for automation.** Run with no arguments for the Textual workbench (fuzzy search, Enter to run, `p` for script settings, `e` to edit the script, `Del` to remove). Every action is also a CLI command for scripting and CI, with `--json` output and meaningful exit codes.
- **i18n built in.** English, Traditional Chinese, and Simplified Chinese via GNU gettext catalogs — zero runtime dependencies (stdlib `gettext`), with per-message fallback to the source text.

## Requirements: uv (hard requirement)

skit is built on [uv](https://docs.astral.sh/uv/) and does not work without it. uv provides the isolated, reproducible script execution (PEP 723) that makes skit possible.

**You don't strictly have to preinstall it**: if skit can't find uv on your system, it will ask for consent and download a pinned uv binary into skit's own private directory. That copy never touches your `PATH` or global environment.

That said, a system-wide uv is the smoothest experience. Install it with one of:

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

Verify with:

```bash
uv --version
```

## Mainland China (中国大陆) — no VPN needed

Behind the magical unthinkable, three downloads can stall: PyPI packages, the Python interpreters uv fetches (python-build-standalone, from GitHub), and skit's own uv bootstrap. skit can route all three through domestic mirrors — and it **never edits your global uv config or environment**.

- **First run**: if PyPI/GitHub look unreachable, `skit` offers to turn on mirrors — just press Enter.
- **Any time**: run `skit config` to see every setting, or set the mirror directly:

```bash
skit config mirror tsinghua      # or: aliyun / ustc
skit config                      # show every setting (language, editor, mirror, form)
skit config mirror off           # turn off again, e.g. when travelling abroad
```

Defaults: PyPI → Tsinghua / Aliyun / USTC; Python builds & the uv binary → NJU (`mirror.nju.edu.cn`). Pick `custom` in `skit config` to override any URL if a mirror goes down.

To **install skit itself** behind the GFW (skit isn't there yet to configure), point uv at a mirror first:

```bash
export UV_DEFAULT_INDEX=https://pypi.tuna.tsinghua.edu.cn/simple
uv tool install skit
```

Already set `UV_DEFAULT_INDEX` / `UV_PYTHON_INSTALL_MIRROR` (or a `uv.toml`) yourself? skit **defers** to your settings and won't override them.

## Installation

From PyPI (once published):

```bash
uv tool install skit
```

Straight from git (works today, before the first PyPI release):

```bash
uv tool install git+https://github.com/user/skit
```

Or run it without installing anything:

```bash
uvx --from git+https://github.com/user/skit skit --help
```

## Usage

```bash
skit                          # TUI workbench: search, Enter to run, p for settings, e to edit, Del to remove
skit add my_script.py         # Add a script (copy mode; detects dependencies and parameters)
skit add my_script.py --ref   # Reference mode: link to the original file instead of copying
skit add tool.exe --exe       # Register an executable
skit add --cmd "ffmpeg -i {input}" -n conv   # Register a command template (placeholders become a form)
skit add --edit -n scratch    # Write a brand-new script in your editor, then add it
skit run my_script            # Run; the parameter form appears first
skit run my_script -p fast    # Run with a named preset (-p / --preset)
skit run my_script --save-preset fast   # Run, then save these values as a preset
skit run my_script --dry-run  # Print the exact command that would run (tokens/globs expanded), then exit
skit run my_script --raw      # Escape hatch: skip the form and injection, run as-is
skit params my_script         # Show managed parameters and last-used values
skit params my_script --manage WIDTH --secret API_KEY   # Manage a detected constant / mark a secret
skit params my_script --resync   # Reconcile definitions after the script changed
skit preset save my_script fast    # Save a named preset (NAME PRESET_NAME)
skit deps my_script --dep "requests>=2,<3" --dep rich   # View / update dependencies
skit edit my_script           # Open the script's source in your editor
skit list                     # List everything registered
skit remove <name>            # Remove an entry (your original file is left untouched)
skit doctor [--rebuild]       # Self-check / rebuild the index from meta.toml files
skit config                   # Show all settings; e.g. skit config lang zh-TW · skit config mirror tsinghua
```

## Development

Development is driven entirely by uv — see [CONTRIBUTING.md](./CONTRIBUTING.md) for the full workflow and quality gates (ruff, ty strict, pytest with a 100% coverage floor, mutation testing with mutmut, zizmor-audited workflows).

```bash
uv sync --dev
uv run pytest -q
uv run python scripts/serve_preview.py   # TUI web preview (textual-serve, localhost:8000)
```

## License

[MIT](LICENSE)
