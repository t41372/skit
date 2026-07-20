# AGENTS.md

## What skit is

A script launcher and parameter manager. Users add scattered scripts — Python, shell, JS/TS,
and a dozen kinds in all — into one library, then run them from skit's TUI menu or the CLI.
Read README.md for more info.

## Core design principles

**1. Everything is i18n.** Every user-visible string ships translated — English is the
original locale (msgids *are* the English source), all supported languages ship at 100%.
The one deliberate exemption: machine-facing surfaces — `--json` keys and the bundled
Agent Skill (`skills/skit/SKILL.md`) — are English-only contracts, not UI copy, and stay
outside the i18n gate.

**2. Full mouse AND full keyboard interactivity.** Every TUI action must be operable by mouse
alone and by keyboard alone. Footer chips are buttons (`tui_footer.chip()`): the visible key
hint *is* the click target, and every modal's key hints use the same chips — the mouse always
has a path. The keyboard side is enforced policy: every key a footer advertises must have a
positive pilot test.

Key grammar: a chord keeps one meaning per context class — Ctrl+E always opens `$EDITOR`
on the screen's current subject, Ctrl+N always creates the screen's primary object (a
script on the add step, an agent on a runner picker), Ctrl+T always inserts a value,
Ctrl+R re-runs/refreshes the screen's subject (the run form runs it; Script settings
resyncs its definitions from the script), and Ctrl+S saves/commits the screen's work
(the run form's save-as-preset included). Ctrl+A (cursor-home) and — while an Input has focus —
Ctrl+E (end-of-line) belong to the Input: screen chords for them are never
priority-bound; the chip is the path mid-edit.
Never bind a text-editing chord (Ctrl+K and friends) with `priority=True` on a screen
full of Inputs — the Input's own editing wins there, and the chip stays the mouse path.

The TUI is also responsive: `tui_layout.py` defines the shared size tiers (`-w-narrow`,
`-h-short`, `-h-tiny`, …) that Textual sets as classes on every screen, and all size
adaptation is CSS keyed off those classes — never per-screen width/height math in Python.
Footer rows wrap chip-by-chip (pills are unbreakable), modals never exceed the screen, and
new screens must degrade the same way.

**3. Zero memorization** The user should be able to do everything without
remembering a single line of CLI, hotkeys, or arguments, even if they wrote the script
themselves. The interface should be ergonomic, convenient, and intuitive, following HCI
principles. What this means in practice:

**4. AI-agent & automation friendly.** Every TUI capability is also a CLI command, with
`--json` output, docker-convention exit codes (`run` passes the script's exit code through
untouched; skit errors are 125/126/127), `--no-input`, `--dry-run`, and dynamic completion.
The non-interactive contract is absolute: in a pipe, in CI, or under `--no-input`, never
guess, never prompt, never silently assemble a broken command. When choosing between designs,
prefer the one an agent can drive deterministically.

**5. Verification gate:** 100% test coverage floor, ruff, ty (strictest mode), mutation
testing with mutmut (zero surviving mutants), and the i18n coverage gate are all hard CI
gates.

**6. Self-contained and non-invasive.** skit stays out of the user's global environment. It
never mutates global/system/another tool's settings (shell env, `~/.config/uv/`, …) without an
explicit consent prompt。

## Language & analyzer rules

skit is multi-language (`src/skit/langs/`): Python is deepest, but shell, JS/TS, fish,
PowerShell and a data-driven long tail (ruby/perl/lua/r) all launch, and shell + JS/TS also
get static parameter analysis and value injection. JS/TS additionally get per-script npm
dependencies (`langs/javascript/deps.py` — deliberately stdlib-only, since it runs on the
launch path): declared packages materialize as a `node_modules` next to the stored copy,
installed by the resolved runner's own installer (npm/bun/deno). Two contributor rules follow:

- **Analyzers may depend on their language's parser; launch paths may not (the A2 amendment).**
  The former "the analyzer is stdlib-only" rule is superseded: each language's analyzer may
  pull in its declared parser package — the tree-sitter grammars (`tree-sitter`, `-bash`,
  `-javascript`, `-typescript`) are hard dependencies — but they stay *contained to analysis*.
  Run/launch paths remain stdlib-only, and a grammar that fails to import degrades that kind's
  capabilities to `None` (the `spec.analyzer is None` idiom downstream) instead of crashing. A
  language's analyzer, injector and normalizer share one parse layer and stand or fall together
  behind a single import guard.
- **`--normalize` is the one exception to comment-only edits (the A5 amendment).** skit's edits
  to a script's own text are otherwise confined to comments (the `[tool.skit]` block). `skit
  params <shell-entry> --normalize NAME` carves out exactly one opt-in, consent-gated exception:
  it rewrites a bare `NAME=value` constant into the `${NAME:-value}` env-default idiom. A real
  semantic edit — but only to skit's **stored copy**, never the user's original, and only when
  asked.
- **Prompts are entries too (docs/design/prompt.md).** The `prompt` kind stores a parameterized
  text body fired at a configured agent CLI (a **PromptRunner** — `[[prompt.runners]]` in config;
  never plain "runner" in code, which is the JS-runtime vocabulary). Contributor rules that
  follow: template/non-template decisions key off `LangSpec.placeholder_params`, never
  `family == "template"`; the render path is raw substitution with NO shell and NO quoting
  (`langs/prompt/render.py` — `quote_for_shell` must never touch it); `LaunchStrategy.build`/
  `describe` carry an accept-and-ignore `runner=` keyword on every strategy; runner resolution:
  non-interactive is `--runner` > the entry's pin > exit 126 — no ranking, no guessing;
  interactive runs host the form's runner picker (prefilled pin > last pick), and the last
  *pick* (never a pin left untouched) prefills the next picker from state. Prompts are not a
  secrets channel and no delivery mechanism pretends otherwise.

Golden corpus: `tests/corpus/<lang>/` (and the Python files directly under `tests/corpus/`) are
**byte-exact** analyzer inputs — deliberate CRLF, missing trailing newlines, odd whitespace,
CJK/emoji. They are excluded from the pre-commit fixers (trailing-whitespace, end-of-file-fixer,
mixed-line-ending); "fixing" them destroys exactly what they test.

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
uv run python -m benchmarks run --profile pr --out .bench   # performance pipeline (benchmarks/README.md)
uv run python -m benchmarks check .bench/results.json       # performance budget contract
```

Full pre-PR gate (run before declaring any change done):

```bash
uv run ruff format --check && uv run ruff check && uv run ty check && uv run pytest --cov && uv run mutmut run
```

## Performance pipeline

`benchmarks/` (design: docs/design/benchmarks.md · manual: benchmarks/README.md) is the
measurement side of a contract:

- **Pipeline PRs measure; they never change runtime behavior.** Optimization PRs must
  attach `benchmark-compare` workflow evidence (base vs head), not hand-run numbers.
- **README performance claims must be generated from a `results.json` artifact** —
  commit, date, host, run counts — never hand-written prose.
- `benchmarks/budgets.toml` is two-tier: `enforced` rows fail `benchmarks check` (CI
  runs it with `--require-enforced`); `target` rows are the aspirational contract,
  reported but never gating. Ratchet bounds refresh ONLY from CI artifacts via
  `check --propose` (module censuses are python-version-dependent), in the same PR
  that moves the metric.
- The bench CI job is advisory by policy: never make it a required status check while
  path-filtered.
- The pipeline's own logic (results/budgets/parsers/pipeline/datasets/envinfo/envspec/
  compare/hyperfine builders) sits under the same 100% coverage floor as src/skit; only
  spawn-and-wait orchestration (`suites/`, `micro/`, `__main__.py`) and benchmark
  subjects are exempt, each exemption commented in pyproject's coverage `omit`.

## i18n workflow

New or changed UI strings: `scripts/i18n.py extract` → `update` → translate the new msgids in
each `.po` → `compile`. Watch for pybabel fuzzy-matching a new msgid to an unrelated old
translation — correct the msgstr and remove the `#, fuzzy` marker, or the completeness gate
fails.

## Agent Skill

skit ships an official Agent Skill ([agentskills.io](https://agentskills.io) format) that
teaches AI agents to drive the library through the CLI. Source of truth:
`skills/skit/SKILL.md` (the copy `npx skills add t41372/skit` discovers). After editing it,
sync the packaged copy — `cp skills/skit/SKILL.md src/skit/skills/skit/SKILL.md` — which is
what ships in the wheel and what `skit agent install` writes. `tests/test_agent_skill.py`
enforces the sync byte-for-byte, validates the frontmatter against the spec, and resolves
every `skit …` invocation the skill teaches against the real command tree — renaming a
command or flag fails that test until the skill is updated too.

## Demo assets

The README's demo videos (`docs/assets/demo-*.mp4`) and screenshot grid (`docs/assets/tui-*.png`)
are generated by a scripted, hermetic pipeline — never hand-recorded. If you change any
user-visible UI copy or add a language, the existing assets go stale: regenerate them with
`bash scripts/record_demo.sh` (needs Docker) and reference the new per-locale files from all
three READMEs. Full pipeline docs — tapes, adding a screen, adding a locale — live in
CONTRIBUTING.md under "Demo assets".

**One deliberate exception:** `docs/assets/demo-mouse.gif` — the short mouse-operability clip below
the hero video in all three READMEs — is hand-recorded, because VHS drives no mouse. It's a
single shared clip (not per-locale) that the pipeline cannot regenerate, so it goes stale
silently if the UI it shows changes; re-record and re-trim it by hand (recipe in
CONTRIBUTING.md).

The demo videos (`docs/assets/demo-*.mp4`) are deliberately **not** tracked in git (they'd
balloon history); the README hero videos are uploaded to GitHub user-attachments by hand.
The PNGs, banner, and `demo-mouse.gif` stay tracked — the READMEs hotlink them via
raw.githubusercontent.

## Documentation site

`docs/` doubles as the user-facing documentation site: a Fumadocs (Next.js static-export)
app, deployed to GitHub Pages (https://t41372.github.io/skit/) by
`.github/workflows/docs.yml` on pushes to main. **To change a documentation page, edit the
MDX in `docs/content/docs/`** (sidebar order: `meta.json` there); pages carry deep reference
detail, not TUI walk-throughs. The **landing page is the repo README itself** — `index.mdx`
just `<include>`s it (synced into `docs/.generated/` by the predev/prebuild hook), so fix
landing content in `README.md`, not `index.mdx`. Verify with `cd docs && npm ci &&
npm run build`; the build runs a link checker (`scripts/check-links.mjs`) that fails on any
broken internal link or `#anchor`. Preview with `npm run dev` (http://localhost:3000/skit/en/).
Gotcha: `<include>` and Turbopack only resolve files **inside** `docs/` — never reference a
path above the project root. The docs are English-only for now and sit **outside** the i18n
coverage gate; the scaffolding (`docs/lib/i18n.ts`) is ready for zh content later. README copy
vocabulary applies — the run screen is the "launch menu", never a "form". `docs/assets/` and
`docs/design/` live beside the site and are not published to it.
