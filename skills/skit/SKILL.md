---
name: skit
description: Run, inspect, and manage scripts in the user's skit library — their personal collection of Python scripts, shell one-liners, and executables, each with a typed parameter form, saved presets, and isolated dependencies. Use when the user asks to run/list/add their scripts, mentions skit or "my script", or before writing a new one-off Python script (the library may already have one that does the job).
license: MIT
compatibility: Requires the skit CLI on PATH (install with `uv tool install skit-cli`)
---

# skit — the user's script library

skit stores the user's scripts in one searchable library and makes them runnable
without remembering flags. For every script it knows the parameter schema (extracted
statically — flags, `input()` calls, and marked constants all become typed fields),
remembers last-used values, saves named presets, and runs Python scripts through
`uv run --script` in an isolated environment (dependencies declared per script,
PEP 723). The library is *the user's curated space*: treat it like their dotfiles.

## Ground rules

1. **Check the library before writing a new script.** If the user asks for something
   a saved script already does, run that instead of regenerating it.
2. **Trust exit codes, never output text.** skit's human output is localized (English,
   繁體中文, 简体中文, …) — string-matching it will break on other machines. The exit
   code and `--json` payloads are the stable contract.
3. **Before a script's first run, `--dry-run` it** and show the user the exact command.
4. **Never add, remove, or overwrite library entries without asking the user first.**
   Propose `skit add` when you've written something reusable; don't add it silently.
5. **Pass `--no-input` on every `skit run` and `skit add`** — the only two commands
   that can prompt. It guarantees skit never blocks; if information is missing, skit
   fails fast with a named error instead. The read commands (`list`, `show`, `params`,
   …) never prompt and don't take the flag.

## Discover scripts

```bash
skit list                 # every script: name, kind, description
skit show <name>          # one script: parameters, types, defaults, presets
skit list --json          # same data, machine-readable (only if you're scripting over it)
skit show <name> --json   # full parameter schema as JSON
```

`show` is the map: each field's `type` (str/int/float/bool/choice), `required`,
`default`, `choices`, and `source` — where the value goes: `flag` (passed as a real
CLI flag), `inject` (a managed constant or `input()` answer, injected into a temporary
copy at run time), or `placeholder` (fills the registered command template).
`degraded_reason` non-empty means skit could not model the script's own parser
(subcommands, dynamic args) — pass arguments through after `--` instead.

## Run scripts

```bash
skit run <name> --no-input                          # defaults + the user's last-used values
skit run <name> --set width=1200 --set fmt=png --no-input
skit run <name> -p <preset> --no-input              # a saved preset
skit run <name> --no-input -- --verbose input.txt   # raw args to the script's own parser
skit run <name> --set width=800 --dry-run --no-input  # print the command, run nothing
skit run <name> --raw --no-input                    # escape hatch: no form, no injection
```

- `--set NAME=VALUE` works for every field kind (flags, injected constants, template
  placeholders). Unknown names are rejected with the valid list (exit 2); values are
  validated against the field's type (exit 125 on mismatch). Values may use tokens —
  `{cwd}`, `{today}`, `{now}`, `{env:VAR}`, a leading `~` — and multi-value fields
  also expand globs.
- Unset fields fall back to: preset > last-used value > the script's own default.
  A required field with no value fails fast (exit 125) rather than prompting.
- **Reuse warning:** with no `--` args given, a python/exe run reuses the *last run's*
  extra args (it says so on stderr). Pass your own `--` args, or use `--raw` (which
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
skit add - --name fetch-report -d "Pull the weekly report" --no-input   # script text on stdin
skit add path/to/script.py --ref --no-input      # reference the original file, don't copy
skit add --cmd 'ffmpeg -i {input} -vf scale={width}:-1 {output}' --name scale-video --no-input
```

- Give every entry a `-d` description — `skit list` is the discovery surface.
- Python dependencies belong in the script itself (PEP 723 inline metadata); skit
  resolves them through uv at run time, nothing installs globally:

```python
# /// script
# requires-python = ">=3.12"
# dependencies = ["requests>=2,<3"]
# ///
```

  You can also record them afterwards: `skit deps <name> --dep "requests>=2,<3"`.
- **Under `--no-input`, detected parameters are NOT auto-managed.** For a script that
  reads constants or `input()` (no argparse), bring its parameters under management
  so they become form fields / `--set` targets:

```bash
skit params <name>                    # see managed + detected-but-unmanaged parameters
skit params <name> --manage WIDTH --manage CITY
skit params <name> --secret API_KEY --env-source API_KEY=OPENAI_API_KEY
```

  With `--env-source`, the secret is read from the environment at run time — the
  value never appears in commands, files, or output. Scripts that parse their own
  CLI (argparse/click/typer) need no management at all; skit reads them statically.

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
skit doctor --json     # environment health: uv presence, library location, drift/missing entries
skit remove <name> -y  # remove an entry (the user's original file is never deleted) — ask first
skit edit <name>       # open the stored source in the user's editor
```

If `show`/`run` reports drift (the script changed and its managed parameter
definitions no longer match), `skit params <name> --resync` refreshes them.
