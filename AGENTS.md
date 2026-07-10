# AGENTS.md

Direction for working in this repository. Anything not covered here follows ordinary,
widely-adopted engineering best practice — this file records only what is specific to skit,
so that technical decisions can account for the project's global goals.

## What skit is

A script launcher and parameter manager. Users collect scattered Python scripts, executables,
and command templates into one library, then run them from a TUI menu or the CLI. skit reads
however a script takes input — hard-coded constants and `input()` calls (via AST), or an
argparse/click/typer command line (read statically) — and turns it into a form to fill in.
Values are delivered without changing the source: injected into a temporary copy, or passed as
flags at run time. Two trust promises are absolute and surfaced in the UI itself: **the user's
source file is never edited**, and **secret parameters never touch disk**.

The full interface spec and its decided trade-offs live in `docs/ux-redesign.md`.

## Core design principles

**1. Everything is i18n.** Every user-visible string ships translated — English is the
identity locale (msgids *are* the English source), zh_TW and zh_CN ship at 100%. Two
constraints follow from how gettext/Babel actually work: translate **at render time** (a
module-level `gettext()` freezes whichever locale was active at import) and **on string
literals only** (Babel extracts literals — `gettext(LABELS[kind])` never becomes a msgid and
silently ships English everywhere). `scripts/i18n_coverage.py` (mirrored by `tests/test_i18n.py`)
closes each escape route: stale `.pot`, missing or fuzzy translations, unwrapped UI literals,
dynamic gettext arguments.

**2. Full mouse AND full keyboard interactivity.** Every TUI action must be operable by mouse
alone and by keyboard alone. Footer chips are buttons (`tui_footer.chip()`): the visible key
hint *is* the click target, and every modal's key hints use the same chips — the mouse always
has a path. The keyboard side is enforced policy: every key a footer advertises must have a
positive pilot test.

**3. Zero memorization** The user should be able to do everything without
remembering a single line of CLI, hotkeys, or arguments, even if they wrote the script
themselves. The interface should be ergonomic, convenient, and intuitive, following HCI
principles. What this means in practice:

- Any script becomes a form, no matter how it takes input — injection into a temp copy is
  just one delivery mechanism among several.
- Recognition over recall: assume most users never press `?`. Every action stays permanently
  visible (two-line footer); nothing may be discoverable only through the help overlay.
- Transparency, not concealment: before running, skit prints the exact command or injection
  it assembled. "No memorizing" means the user doesn't *have* to know — not that skit hides
  it. Users passively learn their scripts' usage.
- Honesty over cleverness: whatever skit can't statically understand degrades to a labeled
  free-text field plus an "extra arguments" escape hatch, and says so. UI copy may never
  sound more confident than the deterministic rule behind it.
- Copy must read for someone who doesn't know the jargon — the script may be AI-written and
  its owner may not know the syntax it uses. Chinese UI terms follow the wording table in
  `docs/ux-redesign.md`.
- State is always carried by glyph + text together, never by color alone.

**4. AI-agent & automation friendly.** This is where skit is heading (the next milestone —
not fully AI-native yet; build toward it). Premise: most scripts today are written by AI,
which favors argparse + PEP 723, so static CLI reading is the mainstream case, not an edge.
Every TUI capability is also a CLI command, with `--json` output, docker-convention exit codes
(`run` passes the script's exit code through untouched; skit errors are 125/126/127),
`--no-input`, `--dry-run`, and dynamic completion. The non-interactive contract is absolute:
in a pipe, in CI, or under `--no-input`, never guess, never prompt, never silently assemble a
broken command. When choosing between designs, prefer the one an agent can drive
deterministically.

**5. Verification is absolute.** 100% test coverage floor, ruff, ty (strictest mode), mutation
testing with mutmut (zero surviving mutants), and the i18n coverage gate are all hard CI
gates. The gates — not review, not eyeballing — are the definition of done: a surviving mutant
is a regression no test would catch, and an unwrapped string shows English in *every* locale,
so testing in one locale proves nothing. Never weaken a gate to pass it. Suppressions are
earned: `# pragma: no mutate` requires a genuinely equivalent mutant, a pinning test, and an
entry in [docs/mutation-ledger.md](docs/mutation-ledger.md).

## Commands

Everything goes through `uv` — there is no supported non-uv workflow, and skit itself runs
scripts via `uv run --script` (PEP 723).

```bash
uv sync --dev                       # create/sync the dev environment
uv run skit --help                  # run the CLI locally
uv run pytest -q                    # run the test suite
uv run pytest tests/test_store.py -q            # one test file
uv run pytest tests/test_corpus.py -k unicode   # one test by name substring
uv run ruff check                   # lint (rule set in pyproject.toml)
uv run ruff format                  # format (CI checks with --check)
uv run ty check                     # type check, strictest mode (all = "error")
uv run mutmut run                   # mutation testing (surviving mutants fail CI)
uv run python scripts/i18n_coverage.py   # i18n coverage gate (also enforced by test_i18n.py)
uv run zizmor .github/workflows     # GitHub Actions security audit
uv run python scripts/serve_preview.py   # TUI web preview via textual-serve (localhost:8000)
```

Full pre-PR gate (run before declaring any change done):

```bash
uv run ruff format --check && uv run ruff check && uv run ty check && uv run pytest --cov && uv run mutmut run
```

## i18n workflow

New or changed UI strings: `scripts/i18n.py extract` → `update` → translate the new msgids in
each `.po` → `compile`. Watch for pybabel fuzzy-matching a new msgid to an unrelated old
translation — correct the msgstr and remove the `#, fuzzy` marker, or the completeness gate
fails.

## Demo assets

The README's demo videos (`docs/demo-*.mp4`) and screenshot grid (`docs/assets/tui-*.png`)
are generated by a scripted, hermetic pipeline — never hand-recorded. If you change any
user-visible UI copy or add a language, the existing assets go stale: regenerate them with
`bash scripts/record_demo.sh` (needs Docker) and reference the new per-locale files from all
three READMEs. Full pipeline docs — tapes, adding a screen, adding a locale — live in
CONTRIBUTING.md under "Demo assets".
