# skit

[![CI](https://github.com/user/skit/actions/workflows/ci.yml/badge.svg)](https://github.com/user/skit/actions/workflows/ci.yml)
[![Coverage: 100%](https://img.shields.io/badge/coverage-100%25-brightgreen)](https://github.com/user/skit/actions/workflows/ci.yml)
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
- **Parameters become a form.** At add time, skit statically analyzes your script (via AST) to detect hard-coded constants and `input()` calls. Check the ones you want managed, and every run starts with a form to fill in — your source semantics are untouched; an injection engine swaps values in at run time.
- **It remembers.** Last-used values are saved automatically. `preset` stores named parameter sets. Secret parameters are structurally prevented from ever touching disk.
- **No environment management.** Scripts run through `uv run --script` with dependencies declared via PEP 723. If uv is missing, skit downloads a private copy for itself (see below).
- **TUI and CLI, equal citizens.** Run with no arguments to get a Textual menu (fuzzy search, Enter to run, `ctrl+e` to edit parameters); everything is also available as CLI commands.
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
skit                          # TUI main menu: search, Enter to run, ctrl+e to edit params, Del to remove
skit add my_script.py         # Add a script (copy mode; detects dependencies and parameter candidates)
skit add my_script.py --ref   # Reference mode: link to the original file instead of copying
skit add tool.exe --exe       # Register an executable
skit add --cmd "ffmpeg -i {input}" --name conv   # Register a command template (placeholders become a form)
skit run my_script            # Run; a parameter form appears first
skit run my_script --preset fast   # Run with a named preset
skit run my_script --raw      # Escape hatch: skip the form and injection, run as-is
skit params my_script         # Show parameter definitions and last-used values
skit edit my_script --resync  # Reconcile: sync definitions after the script changed
skit preset save my_script fast    # Save a named preset
skit deps my_script --set requests,rich   # View / update dependencies
skit list                     # List everything registered
skit remove <name>            # Remove an entry
skit doctor [--rebuild]       # Self-check / rebuild the index from meta.toml files
skit lang zh-TW               # Show or set the display language
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
