# Contributing to skit

Thanks for helping out! This document explains the development workflow and the quality gates every change must pass.

## Hard requirement: uv

skit development is **driven entirely by [uv](https://docs.astral.sh/uv/)**. Manually assembled `pip` / `venv` environments are not supported. Every command goes through `uv run`, and uv owns the creation and syncing of the isolated environment.

Install uv (pick one):

```bash
# macOS / Linux
curl -LsSf https://astral.sh/uv/install.sh | sh

# Windows (PowerShell)
powershell -c "irm https://astral.sh/uv/install.ps1 | iex"

# If you already have Homebrew / pipx / cargo
brew install uv        # or
pipx install uv        # or
cargo install --git https://github.com/astral-sh/uv uv
```

Verify:

```bash
uv --version
```

## Getting started

```bash
git clone https://github.com/t41372/skit
cd skit

# Create and sync the environment with dev dependencies (.venv is managed by uv)
uv sync --dev

# Run skit locally
uv run skit --help
```

## Quality gates

Every item below is a hard CI gate — all of them must be green before a merge. Please run the full set locally before opening a PR.

| Purpose | Command | Notes |
|---|---|---|
| Lint | `uv run ruff check` | Rule set lives in `pyproject.toml` (bugbear, bandit, pylint, and more) |
| Formatting | `uv run ruff format` | Run before submitting; CI verifies with `ruff format --check` |
| Types | `uv run ty check` | ty in its strictest mode (`[tool.ty.rules] all = "error"`) |
| Tests | `uv run pytest -q` | Runs across Linux/macOS/Windows × Python 3.12 / 3.13 |
| Coverage | `uv run pytest --cov` | **Floor is 100%** (`fail_under = 100`); anything less fails |
| Mutation testing | `uv run mutmut run` | Surviving mutants fail CI |
| Workflow audit | `uv run zizmor .github/workflows` | Security scan for GitHub Actions |
| i18n in sync | `uv run python scripts/i18n.py compile` | Committed `.mo` must match the `.po` sources (CI checks `git diff`) |
| i18n coverage | `uv run python scripts/i18n_coverage.py` | Fresh `.pot`, 100% non-fuzzy translations, no unwrapped UI literals, no dynamic `gettext()` (mirrored by `tests/test_i18n.py`) |

Run the whole suite in one go (recommended before every PR):

```bash
uv run ruff format --check && \
uv run ruff check && \
uv run ty check && \
uv run pytest --cov && \
uv run mutmut run
```

## Pre-commit hooks (prek)

This project uses [prek](https://github.com/j178/prek) — a faster, Rust-based, pre-commit-compatible runner — for pre-commit checks. Configuration lives in `.pre-commit-config.yaml`.

```bash
# Install the git hook (runs automatically on every commit afterwards)
uvx prek install

# Run against all files manually
uvx prek run --all-files
```

The hooks cover ruff (lint + format), ty, a zizmor audit of `.github/workflows`, and recompiling the i18n catalogs (`.po` → `.mo`) whenever a `.po` changes.

## Testing rules (important)

- **The 100% coverage floor is real, and padding is not accepted.** Never use `# pragma: no cover` to hide reachable branches, and never write hollow "import-only, assert-nothing" tests to game the number. Every test must make a meaningful assertion about observable behavior.
- **Coverage only counts when it survives mutation testing.** Coverage proves "this line executed"; mutation testing proves "this line's logic is actually pinned down by an assertion". When adding code, make sure your assertions kill the mutants mutmut generates.
- `# pragma: no cover` is allowed only for genuinely unreachable or defensive branches, with an inline comment explaining why.
- Secret-related behavior must have a "never touches disk" test (see the existing `argstate` tests).

## Translations (i18n)

User-facing strings use **GNU gettext with source-string message ids**: the English text passed to
`gettext("…")` / `ngettext("…", "…", n)` in the source *is* the message id. The runtime uses only the
stdlib `gettext` module (no third-party runtime dependency); [Babel](https://babel.pocoo.org/) is a
dev-only tool for extraction and compilation. Catalogs live in
`src/skit/locales/<locale>/LC_MESSAGES/skit.{po,mo}`; both the `.po` sources and the compiled `.mo`
are committed (the `.mo` is what ships in the wheel and what the tests load).

All workflows go through `scripts/i18n.py`:

```bash
# Changed or added a UI string? Refresh the template, sync it into every locale, then compile.
uv run python scripts/i18n.py extract     # source strings  -> locales/skit.pot
uv run python scripts/i18n.py update      # skit.pot         -> each locale's .po (new msgids appear untranslated)
# …translate the new/changed msgids in each src/skit/locales/*/LC_MESSAGES/skit.po…
uv run python scripts/i18n.py compile     # .po -> .mo  (also run by the pre-commit hook)

# Add a whole new language (e.g. Japanese, French):
uv run python scripts/i18n.py add ja      # scaffolds locales/ja/LC_MESSAGES/skit.po from the template
```

English needs no catalog — an untranslated msgid falls back to the source text. Because the id *is*
the English, editing an English string changes its id, so `update` will (correctly) flag the
translations as needing review. Keep the committed `.mo` in sync (`compile`) or CI will fail.

## Demo assets (README videos & screenshots)

The README's demo videos (`docs/assets/demo-*.mp4`) and its four-screen TUI screenshot grid
(`docs/assets/tui-*-{en,zh}.png`) are never recorded by hand — a scripted, hermetic
[VHS](https://github.com/charmbracelet/vhs) pipeline renders them, so they can be regenerated
identically whenever the UI changes:

```bash
bash scripts/record_demo.sh          # everything: 2 videos + 8 screenshots
bash scripts/record_demo.sh videos   # docs/assets/demo-en.mp4, docs/assets/demo-zh.mp4
bash scripts/record_demo.sh shots    # docs/assets/tui-{library,form,add,settings}-{en,zh}.png
```

The only host requirement is Docker (or OrbStack). vhs / ttyd / ffmpeg live inside the image
and never touch your machine.

How the pieces fit:

- **`docs/assets/demo/Dockerfile`** — the recording environment: the official VHS image, plus uv,
  skit installed from your working tree, `bat`, `fonts-noto-cjk` (real Han glyphs for the zh
  renders), and a colored prompt (`docs/assets/demo/demo.bashrc`).
- **`docs/assets/demo/demo.tape`** (the video) and **`docs/assets/demo/shots.tape`** (the screenshots) —
  the VHS keystroke choreography. Each tape is written once and drives every locale.
- **`docs/assets/demo/scripts/{en,zh}/`** — the dummy scripts being demoed, one set per language
  (their docstrings and `--help` text are what skit's forms display, so they are localized
  too). `scripts/record_demo.sh` runs each tape once per locale, with `SKIT_LANG` set and that
  language's scripts mounted at `/demo`.
- Tapes and demo scripts are **mounted, not baked** — edit them and re-run, no rebuild.
  Only a change to skit's own source triggers an image rebuild (skit is baked in with
  `uv tool install`), and the script rebuilds automatically anyway.

Tips when editing tapes:

- Expect a short tune loop — render, eyeball, adjust `Sleep` values and `Tab` counts. That's
  normal VHS workflow.
- Type only ASCII in the *shell* scenes: non-ASCII keystrokes garble on the way through ttyd.
  Typing into skit's own TUI inputs is fine.
- VHS has no `End`/`Home` and no mouse. Clear a prefilled field with `Right N` + `Backspace N`;
  mouse interaction can't be recorded at all — a separately captured clip is the only option
  (see *The mouse-operability GIF* below).
- Showing a new screen? Add a `Screenshot "/out/shot-<name>.png"` line to `shots.tape` and the
  matching rename in `record_demo.sh`, then reference it from both READMEs.

### The mouse-operability GIF (`docs/assets/demo-mouse.gif`)

One demo asset is **not** pipeline-generated: `docs/assets/demo-mouse.gif`, the short clip under the
hero video that shows skit driven by mouse alone (design principle #2). VHS drives no mouse, so
this is hand-captured — the one exception to the "never hand-recorded" rule. It's a single
shared clip (English UI, reused verbatim in all three READMEs, not per-locale), and
`record_demo.sh` never touches it, so it goes stale silently if the UI it shows changes.

To regenerate it: screen-record yourself driving skit with the mouse (any tool — the current
clip is a macOS QuickTime recording), then trim, speed up, and optimize into a small
autoplaying GIF. The current one is source `16→37s`, sped `1.5×` to ~14s, 15fps, 1000px wide,
1.3MB:

```bash
SRC="your-recording.mov"    # macOS recording filenames carry a U+202F (narrow no-break
                            # space) before AM/PM — glob the path, don't type it by hand
common="setpts=PTS/1.5,fps=15,scale=1000:-1:flags=lanczos"
ffmpeg -y -ss 16 -t 21 -i "$SRC" -vf "${common},palettegen=stats_mode=diff" pal.png
ffmpeg -y -ss 16 -t 21 -i "$SRC" -i pal.png \
  -lavfi "${common} [x]; [x][1:v] paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle" \
  docs/assets/demo-mouse.gif
```

Keep it short and mouse-motion-focused (light on readable UI text) so it stays useful across
locales and doesn't obviously stale when copy changes. A full-length GIF is a trap — 64s came
out at ≈18MB; trim hard, and prefer speeding the clip up over shipping raw minutes.

## Adding dependencies

```bash
uv add <package>            # runtime dependency
uv add --dev <package>      # dev dependency (goes into [dependency-groups] dev)
```

Never edit `uv.lock` by hand; let uv maintain it and commit it together with your change.

## Commits and PRs

- Write commit messages in the imperative mood, focused on a single change.
- In your PR, explain the motivation and the approach, and confirm every gate above is green.
- When touching `.github/workflows/**`, run `zizmor` locally, and always pin third-party actions to a **commit SHA** (with the version tag noted in a comment beside it).

## License

By submitting a contribution you agree to release it under the project's [MIT License](LICENSE).
