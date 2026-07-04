# AGENTS.md

This file provides guidance when working with code in this repository.

## What skit is

A script launcher and parameter manager. Users collect scattered Python scripts / executables / command templates into one place, then run them from a TUI menu or CLI. The differentiator (Layer 2): skit statically analyzes a Python script at add time to find hard-coded constants and `input()` calls, and at run time injects form values into a temporary copy — **the user's source is never edited**.

## Commands

Everything goes through `uv` — there is no supported non-uv workflow. `.venv` is managed by uv.

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
uv run zizmor .github/workflows     # GitHub Actions security audit
uv run python scripts/serve_preview.py   # TUI web preview via textual-serve (localhost:8000)
```

Full pre-PR gate (all are hard CI gates):

```bash
uv run ruff format --check && uv run ruff check && uv run ty check && uv run pytest --cov && uv run mutmut run
```
