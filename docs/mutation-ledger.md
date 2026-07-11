# Mutation-testing ledger

`uv run mutmut run` is a hard CI gate: surviving mutants fail the build. This ledger records
every accepted suppression and every known non-issue, each with its justification and the named
test that keeps the behavior pinned. **Read this before adding or removing a
`# pragma: no mutate`** — and when you add one, add its entry here.

The ground rule: `# pragma: no mutate` suppresses *all* mutants on that line, not just the one
you're targeting. A pragma is acceptable only when the mutants it silences are genuinely
equivalent, and any killable behavior on that line stays directly protected by a named test —
what's lost is only the mutation-meta-verification of that test, never behavioral coverage.

## Literal-initializer sites (equivalent `0→None`, trades away `0→1`)

Three sites accept losing the arithmetic (`0→1`) mutant to suppress the equivalent (`0→None`)
one:

| Site | Pinning test |
| --- | --- |
| `src/skit/tui.py` `_fuzzy_match`'s `pos = 0` | `tests/test_tui_cov.py::test_fuzzy_match_subsequence` |
| `src/skit/i18n.py` `_pseudoize`'s `last = 0` | `tests/test_residual_mut.py::test_pseudoize_transforms_full_text_including_first_char` |
| `src/skit/shim.py` `_preamble_line_index`'s `i = 0` | `tests/test_shim.py::test_preamble_inserted_at_end_for_no_docstring_no_future` |

## Mechanical equivalent-mutant sites

- **I/O kwargs**: `read_text(encoding="utf-8", errors="replace")` / `write_text(...,
  encoding="utf-8")` (`"utf-8"`/`"UTF-8"` are aliases; `errors=` never fires on ASCII content).
  Covered behaviorally by
  `tests/test_cli_mut.py::test_plan_for_entry_tolerates_invalid_utf8_bytes` and the various
  read-back assertions.
- **Filesystem-case paths**: `entry.dir / "script.py"` (`"SCRIPT.PY"` resolves to the same file
  on macOS's case-insensitive FS, so the mutant is a false survivor locally). Path correctness
  is asserted where it matters (e.g. `test_editor.py::test_edit_opens_copy_source`).
- **rich Table styling** (`_show_params`) and the interactive `add` temp-file's cosmetic
  `tempfile.mkstemp` prefix: no effect on the captured (no-ANSI) text.
- **Env-var defaults / `subprocess.run(check=False)`** (`src/skit/editor.py`) and the
  `config.load_editor` dict default: all falsy-equivalent, guarded downstream. The killable
  parts (posix split, platform default, exact error message) are still asserted in
  `tests/test_editor.py`.
- **`pep723.split_requirements`'s `quote = ""` sentinel** (init + reset): `""`/`None` are
  falsy-equivalent — the variable is only read via truthiness, and `ch == quote` can never fire
  while it's falsy. Quote handling itself is behaviorally pinned by
  `tests/test_pep723_split.py::test_double_quoted_marker_comma_stays_joined` /
  `test_single_quoted_marker_comma_stays_joined`.
- **`flows.assemble`'s `raw = values.get(f.key, "")`**: a missing key means "unset", and
  `""`/`None` are falsy-equivalent everywhere downstream (`_final_value` / `_resolve_secret`
  only read `raw` via truthiness), so the default is an equivalent mutant. The killable side —
  a missing field must NOT inject a sentinel — stays covered by
  `tests/test_flows.py::test_assemble_degraded_empty_omitted_filled_passed`.
- **`flows._type_error`'s `_coerce(value, f.kind, f.key)`**: `f.key` feeds only
  `ShimValueError.param_name`, which this call discards (the returned message is rebuilt from
  `f.label`), so `f.key → None` is equivalent. The killable value/kind coercion is still pinned
  by `tests/test_flows.py::test_type_error_messages_exact`.
- **`argspec._read_typer_param`'s `has_positional_default=False`** (the Annotated call site):
  `False`/`None` are falsy-equivalent in every branch of `_apply_typer_meta` (both pick
  `call.args` for decls and skip the positional-default read). The killable `True` variant —
  which would misread an Annotated Option's first flag declaration as a value default — is
  behaviorally pinned by
  `tests/test_argspec_click_typer.py::test_annotated_option_positional_decl_is_a_flag_not_a_default`.

- **Agent-skill I/O kwargs** (`src/skit/agentskill.py`): `skill_text()`'s
  `read_text(encoding="utf-8")` and `install_into()`'s `write_text(..., encoding="utf-8")` are
  pragma'd I/O-kwarg sites (the same `"utf-8"`/`"UTF-8"`/locale-default alias equivalence as
  above — and the SKILL.md content is non-ASCII, so the behavior that matters is pinned
  byte-for-byte by `tests/test_agent_install.py::test_skill_text_is_the_bundled_skill` and
  `::test_install_into_writes_and_upgrades`).

## Known non-issues (no pragma, no action needed)

- `src/skit/tokens.py` `expand`'s scanner-index arithmetic (`i += 1` / `i += 2`) shows up as
  **timeout** (not survived): every such mutant pins or rewinds the index and infinite-loops on
  the first multi-character input, which mutmut detects by deadline. Caught-by-timeout is a
  legitimate kill.
- `src/skit/uvman.py` `ensure_uv_downloaded`'s `sys.platform == "win32"` exe-name check shows
  up as **suspicious** (not survived) on non-Windows runs — not our code, a pre-existing item
  left as-is.
- `src/skit/agentskill.py` `skill_text()`'s resource lookup line: `resources.files(None)` is a
  genuine cross-platform equivalent (an anchor of `None` resolves to the calling module's own
  package, i.e. `skit`), and `joinpath("SKILLS", …)` is the macOS case-insensitive-FS false
  survivor already described for `entry.dir / "script.py"` — killed on CI's Linux runner.
  Behavior stays pinned by `tests/test_agent_install.py::test_skill_text_is_the_bundled_skill`.
