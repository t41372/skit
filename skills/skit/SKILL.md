---
name: skit
description: Run, inspect, and manage scripts in the user's skit library — their personal hub, manager, and launcher for scripts in many languages (Python, shell, JS/TS, and more), executables, and command templates, each with a typed parameter form, saved presets, and per-script dependencies. Use when the user asks to run/list/add their scripts, mentions skit or "my script", or before writing a new one-off script (the library may already have one that does the job).
license: MIT
compatibility: Requires the skit CLI on PATH (install with `uv tool install skit-cli`)
---

# skit — the user's script library

skit stores the user's scripts in one searchable library and makes them runnable
without remembering flags. It runs many languages — Python, shell (bash/sh/zsh),
JS/TS, fish, PowerShell, Ruby/Perl/Lua/R — plus executables and command templates.
For every script it knows the parameter schema (extracted statically — flags,
prompts, and marked constants all become typed fields), remembers last-used values,
and saves named presets. Python runs through `uv run --script` in an isolated
environment (dependencies declared per script, PEP 723); JS/TS scripts get per-script
npm dependencies (declared with `skit deps`, installed next to the stored copy on
first run); other languages run through their own interpreter or runner. The library
is *the user's curated space*: treat it like their dotfiles.

## Ground rules

1. **Check the library before writing a new script.** If the user asks for something
   a saved script already does, run that instead of regenerating it.
2. **Trust exit codes, never output text.** skit's human output is localized (English,
   繁體中文, 简体中文, …) — string-matching it will break on other machines. The exit
   code and `--json` payloads are the stable contract.
3. **Before a script's first run, `--dry-run` it** and show the user the exact command.
4. **Never add, remove, or overwrite library entries without asking the user first.**
   Propose `skit add` when you've written something reusable; don't add it silently.
5. **Pass `--no-input` on every `skit run` and `skit add`.** It guarantees those never
   block on a prompt; if information is missing, skit fails fast with a named error
   instead. `skit remove` confirms instead of taking `--no-input` — pass `-y`. The
   read commands (`list`, `show`, `params`, …) never prompt and don't take the flag.

## Discover scripts

```bash
skit list                 # every script: name, kind, description
skit show <name>          # one script: parameters, types, defaults, presets
skit list --json          # same data, machine-readable (only if you're scripting over it)
skit show <name> --json   # full parameter schema as JSON
```

`show` is the map: each field's `type` (str/int/float/bool/choice), `required`,
`default`, `choices`, and `source` — where the value goes: `flag` (passed as a real
CLI flag), `inject` (a managed constant or prompt answer, rewritten into a temporary
copy at run time), `env` (delivered as an environment variable), or `placeholder`
(fills the registered command template). The top-level `param_origin`
(`declared`/`reader`/`managed`/`command`/`none`) says where the whole schema came
from. `degraded_reason` non-empty means skit could not model the script's own parser
(subcommands, dynamic args) — pass arguments through after `--` instead. `needs` lists
the external commands the script requires on PATH.

## Run scripts

```bash
skit run <name> --no-input                          # defaults + the user's last-used values
skit run <name> --set width=1200 --set fmt=png --no-input
skit run <name> -p <preset> --no-input              # a saved preset
skit run <name> --no-input -- --verbose input.txt   # raw args to the script's own parser
skit run <name> --set width=800 --dry-run --no-input  # print the command, run nothing
skit run <name> --raw --no-input                    # escape hatch: as-is, no form flags (--set/-p refused)
```

- `--set NAME=VALUE` works for every field kind (flags, injected constants, template
  placeholders). Unknown names are rejected with the valid list (exit 2); values are
  validated against the field's type (exit 125 on mismatch). Values may use tokens —
  `{cwd}`, `{today}`, `{now}`, `{env:VAR}`, a leading `~` — and multi-value fields
  also expand globs.
- Unset fields fall back to: preset > last-used value > the script's own default.
  A required field with no value fails fast (exit 125) rather than prompting.
- **Reuse warning:** with no `--` args given, a script or exe run reuses the *last
  run's* extra args (it says so on stderr). Pass your own `--` args, or use `--raw` (which
  never replays old arguments), when you need a clean slate; `--dry-run` shows exactly
  what would happen.
- Secrets: prefer wiring them to environment variables (see below) over `--set`.
  Secret values never persist to disk and are masked as ••• in dry-run output.

### Exit codes (docker convention)

When the script actually ran, its exit code passes through **untouched** (even if the
script itself exits 125–127 — check stderr when in doubt). When it never launched:

| code | meaning |
| --- | --- |
| 2 | usage error (bad flags, unknown `--set` name, unknown preset) |
| 125 | skit-side failure: missing/invalid parameter value, drift, launch failure |
| 126 | target exists but is not executable |
| 127 | no such script in the library (or launch target missing) |
| 130 | user cancelled the interactive form |

## Add scripts to the library

Always confirm with the user before adding. From a file or stdin:

```bash
skit add path/to/script.py --name resize -d "Resize images to a target width" --no-input
skit add path/to/backup.sh --name backup -d "Nightly database dump" --no-input
skit add - --name fetch-report -d "Pull the weekly report" --no-input   # script text on stdin
skit add path/to/script.py --ref --no-input      # reference the original file, don't copy
skit add path/to/tool --kind shell --name tool -d "Cleanup helper" --no-input   # force the kind
skit add --cmd 'ffmpeg -i {input} -vf scale={width}:-1 {output}' --name scale-video --no-input
```

- Give every entry a `-d` description — `skit list` is the discovery surface.
- The kind is inferred from the extension or shebang. Force it with `--kind`
  (`python`, `shell`, `js`, `ts`, `fish`, `powershell`, `ruby`, …) for an extensionless
  file, or `--exe` for a program.
- **Two kinds of dependency.** Python packages go in the script itself (PEP 723 inline
  metadata) and skit resolves them through uv at run time, nothing installs globally:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = ["requests>=2,<3"]
# ///
```

  Record them afterwards with `skit deps <name> --dep "requests>=2,<3"`. JS/TS
  packages are declared the same way (`skit deps <name> --dep "chalk@^5"`; `skit add`
  suggests them from the script's own imports) and installed into a per-script
  `node_modules` next to the stored copy on first run — copy-mode entries only, since
  a reference entry runs from its own project. JS/TS installs never run package
  lifecycle scripts (npm and bun get `--ignore-scripts`; deno skips them by default), so
  a package that requires its postinstall step won't work. When deno is the resolved
  runner, skit invokes it with `--allow-all` — scripts are not sandboxed. External
  *commands* a script of any kind expects on PATH (a shell script needing `jq`, say)
  are `needs`, checked before every run: `skit deps <name> --need jq --need ffmpeg`.
- **Under `--no-input`, detected parameters are NOT auto-managed.** For a Python or
  shell script that reads constants, prompts, or `${VAR:-}` env-defaults (no
  argparse/getopts), bring its parameters under management so they become form fields
  / `--set` targets:

```bash
skit params <name>                    # see managed + detected-but-unmanaged parameters
skit params <name> --manage WIDTH --manage CITY
skit params <name> --secret API_KEY --env-source API_KEY=OPENAI_API_KEY
```

  With `--env-source`, the secret is read from the environment at run time — the
  value never appears in commands, files, or output. Scripts that parse their own CLI
  (Python argparse/click/typer, shell getopts, JS `util.parseArgs`, fish `argparse`,
  PowerShell `param()`) need no management — skit reads them statically.
- **Declared parameters** (for exe, command, and reader-less kinds like ruby/perl/lua/r)
  are defined by hand on the entry, then behave like any other field:

```bash
skit params <name> --add OUTPUT --type OUTPUT=str --deliver OUTPUT=flag --flag OUTPUT=--out
skit params <name> --add FORMAT --choices FORMAT=png,jpg,webp --default FORMAT=png --optional FORMAT
skit params <name> --rm OUTPUT
```

  `--deliver` picks how the value reaches the program: `flag` (exe), `env` (any kind),
  or `placeholder` (command templates).
- **Shell only:** `skit params <name> --normalize WIDTH` rewrites a bare `WIDTH=800`
  constant into the `${WIDTH:-800}` idiom in skit's *stored copy* (never the user's
  original), so the value is delivered as an environment variable rather than by
  rewriting a temporary copy. Opt-in, and the one edit skit ever makes to script text.

## Presets

Named value sets per script, ideal for recurring jobs:

```bash
skit run <name> --set a=1 --set b=2 --save-preset nightly --dry-run --no-input  # create without running
skit preset list <name> --json
skit run <name> -p nightly --no-input
skit preset delete <name> nightly
```

## Maintenance

```bash
skit doctor --json     # health: uv, library location, drift/missing entries, needs_missing,
                       # mirror {enabled + stored URLs; an axis applies iff enabled and its URL is set}
skit remove <name> -y  # remove an entry (the user's original file is never deleted) — ask first
skit edit <name>       # open the stored source in the user's editor
skit config js.runner deno         # pin the JS/TS runner (default: auto — deno > bun > node)
skit config shell.bash_path /path  # where bash lives on Windows (POSIX auto-detects)
```

If `show`/`run` reports drift (the script changed and its managed parameter
definitions no longer match), `skit params <name> --resync` refreshes them.
