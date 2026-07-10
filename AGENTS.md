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

## Accepted mutation trade-off

`# pragma: no mutate` suppresses *all* mutants on that line, not just the targeted one. Three literal-initializer sites accept this: `src/skit/tui.py` `_fuzzy_match`'s `pos = 0`, `src/skit/i18n.py` `_pseudoize`'s `last = 0`, `src/skit/shim.py` `_preamble_line_index`'s `i = 0` — each trades away the arithmetic (`0→1`) mutant to suppress the equivalent (`0→None`) one. The behaviors stay directly protected by named tests (`tests/test_tui_cov.py::test_fuzzy_match_subsequence`, `tests/test_residual_mut.py::test_pseudoize_transforms_full_text_including_first_char`, `tests/test_shim.py::test_preamble_inserted_at_end_for_no_docstring_no_future`); only mutation-meta-verification of those tests is lost.

A second, mechanical category is pragma'd for genuinely equivalent mutants — the behavior is still covered by named tests, only mutation-meta-verification is lost:

- **I/O kwargs**: `read_text(encoding="utf-8", errors="replace")` / `write_text(..., encoding="utf-8")` (`"utf-8"`/`"UTF-8"` are aliases; `errors=` never fires on ASCII content). Covered behaviorally by `tests/test_cli_mut.py::test_plan_for_entry_tolerates_invalid_utf8_bytes` and the various read-back assertions.
- **Filesystem-case paths**: `entry.dir / "script.py"` (`"SCRIPT.PY"` resolves to the same file on macOS's case-insensitive FS, so the mutant is a false survivor locally). Path correctness is asserted where it matters (e.g. `test_editor.py::test_edit_opens_copy_source`).
- **rich Table styling** (`_show_params`) and the interactive `add` temp-file's cosmetic `tempfile.mkstemp` prefix: no effect on the captured (no-ANSI) text.
- **Env-var defaults / `subprocess.run(check=False)`** (`src/skit/editor.py`) and the `config.load_editor` dict default: all falsy-equivalent, guarded downstream. The killable parts (posix split, platform default, exact error message) are still asserted in `tests/test_editor.py`.
- **`pep723.split_requirements`'s `quote = ""` sentinel** (init + reset): `""`/`None` are falsy-equivalent — the variable is only read via truthiness, and `ch == quote` can never fire while it's falsy. Quote handling itself is behaviorally pinned by `tests/test_pep723_split.py::test_double_quoted_marker_comma_stays_joined` / `test_single_quoted_marker_comma_stays_joined`.
- **`flows.assemble`'s `raw = values.get(f.key, "")`**: a missing key means "unset", and `""`/`None` are falsy-equivalent everywhere downstream (`_final_value` / `_resolve_secret` only read `raw` via truthiness), so the default is an equivalent mutant. The killable side — a missing field must NOT inject a sentinel — stays covered by `tests/test_flows.py::test_assemble_degraded_empty_omitted_filled_passed`.
- **`flows._type_error`'s `_coerce(value, f.kind, f.key)`**: `f.key` feeds only `ShimValueError.param_name`, which this call discards (the returned message is rebuilt from `f.label`), so `f.key → None` is equivalent. The killable value/kind coercion is still pinned by `tests/test_flows.py::test_type_error_messages_exact`.

- **`argspec._read_typer_param`'s `has_positional_default=False`** (the Annotated call site): `False`/`None` are falsy-equivalent in every branch of `_apply_typer_meta` (both pick `call.args` for decls and skip the positional-default read). The killable `True` variant — which would misread an Annotated Option's first flag declaration as a value default — is behaviorally pinned by `tests/test_argspec_click_typer.py::test_annotated_option_positional_decl_is_a_flag_not_a_default`.

`src/skit/tokens.py` `expand`'s scanner-index arithmetic (`i += 1` / `i += 2`) shows up as **timeout** (not survived): every such mutant pins or rewinds the index and infinite-loops on the first multi-character input, which mutmut detects by deadline. Caught-by-timeout is a legitimate kill; no pragma, no action needed.

Not our code, left as-is: `src/skit/uvman.py` `ensure_uv_downloaded`'s `sys.platform == "win32"` exe-name check shows up as **suspicious** (not survived) on non-Windows runs — a pre-existing item, unrelated to the launcher/params work.
