# Path-aware parameter entry — design

Status: **draft v1** (2026-07-19, pending adversarial design review). Resolves
[#7](https://github.com/t41372/skit/issues/7) — "Make file selector more intuitive".
Read `docs/design/multilang.md` first (the `ParamDecl` universal model and the kind
registry) and `docs/design/prompt.md` for the run form's current shape — this design is
an additive layer on both. Ships from `feat/path-picker`, which lands **after**
`feat/prompt-kind` merges; file:line references are as of base `91a70ea` and are
anchored by symbol name where the form is in flux.

## Mission

Typing a file path into the run form is skit's highest-friction moment: the user is one
directory away from a file whose name their shell would complete in two keystrokes, and
skit makes them type every character blind. Fix it in three layers — **complete while
typing** (ghost text on any field that looks like it holds a path), **browse on demand**
(a type-to-filter picker modal reachable from the existing insert menu), and **know when
a field IS a path** (a real `path` parameter type that analyzers detect and users can
declare). Zero new dependencies: Textual's built-in `Input.suggester` plus the modal
patterns already in `tui_form.py`.

Sibling issue [#9](https://github.com/t41372/skit/issues/9) (seed args from the shell
into the TUI) attacks the same pain from the shell side and is **deliberately a separate
branch** — see Deferred.

## Decisions already made (maintainer-approved 2026-07-19, not up for re-litigation)

1. **Scope is #7 only.** #9 gets its own design and branch afterwards.
2. **The picker is a type-to-filter directory list**, mirroring `EnvPickerModal` — not a
   `DirectoryTree`. Typing filters the current directory; Enter descends into a
   directory or picks a file; Backspace on an empty filter ascends.
3. **`path` becomes the sixth `ParamType`**, *and* the completion affordances are
   universal: every free-text field gets path completion once its text looks like a
   path. The type upgrades ergonomics (bare-prefix completion, picker-first insert
   menu); it is never the price of admission — that would violate zero-memorization.
4. **`path` carries str semantics.** No existence validation, no coercion, no
   `exists=`/`dir_okay=` modelling (deferred). Headless behavior is byte-identical to
   `str`; the type changes what the TUI *offers*, never what a run *requires*.

## Core architecture

### 1. The `path` type (signal layer)

- `params.py`: `"path"` joins `ParamType` and `_TYPES` (`params.py:45,60`), and thus
  `as_param_type`/`ALLOWED_TYPES` and the `edit_declared` validator. `coerce_default`
  treats it exactly like `str`.
- Both serialization homes already carry a `type` key — the frozen `[tool.skit]` block
  shape (`to_block_dict`) and meta `[[parameters]]` (`to_meta_dict`). `"path"` is a new
  legal **value**; the frozen key set is untouched. An older skit reading
  `type = "path"` degrades it to `"str"` via `_coerce_literal` — graceful by
  construction, pinned by a test.
- `flows.FormField`: the `kind` whitelists admit `"path"`; `validate_value` treats it as
  `str` (no checks). `tui_form._type_label` gains a translated label.

### 2. Analyzer detection (Python auto-detects; others declare)

The Python static reader already *sees* path-ness and discards it. Stop discarding:

- argparse: `type=Path` → `"path"` (today: comment-acknowledged, emitted as str,
  `argspec.py:186`). `type=argparse.FileType(...)` → `"path"` (today: degrades the whole
  field to free-text — this is a strict upgrade; the value the user supplies *is* a
  filename).
- click: `type=click.Path(...)` / `click.File(...)` → `"path"` (today: degraded).
  Keyword arguments (`exists=`, `dir_okay=`, …) are ignored in P1 — deferred.
- typer: `Path` annotation → `"path"` (today: mapped to `"str"` in
  `_ANNOTATION_KINDS`).

Shell, JS/TS, fish, PowerShell get **no name-based heuristics** — inferring "path" from
a parameter being called `file` is guessing, and skit does not guess. Those languages
reach `path` via declared `type = "path"`; the universal affordance covers the
undeclared case regardless.

New golden-corpus inputs cover each detection (argparse `Path`/`FileType`, `click.Path`,
`click.File`, typer `Path`) — byte-exact, excluded from fixers, like all corpus files.

### 3. One completion root, shared with launch (correctness fix)

Relative paths in a value are resolved by the **child process** at its resolved workdir
— `launcher._resolve_workdir(entry, invoke_cwd)` — which for reference-mode entries
defaults to `"origin"`, not the user's cwd. The form's live glob feedback currently
evaluates at `Path.cwd()` unconditionally (`tui_form.py`, `FieldRow._live_feedback`),
so its ✓/✗ can lie for exactly those entries. This design introduces one shared helper
— the form's **path root** = `_resolve_workdir(entry, Path.cwd())` — used by the
suggester, the picker, *and* the existing glob feedback. The divergence dies as a side
effect and is pinned by a test (an `"origin"`-workdir entry's feedback counts files at
the origin directory, not the launch cwd).

### 4. Ghost suggester (typing layer)

All new TUI code lives in a **new module `src/skit/tui_pathpick.py`** (suggester +
picker modal together); `tui_form.py` takes only thin hooks. This is deliberate
conflict containment — the run form is under heavy rework on `feat/prompt-kind` —
and it keeps the eventual rebase near-trivial.

`PathSuggester(Suggester)` — async, fish-style ghost text, `→` accepts:

- **Activation**: a `path`-typed field suggests always (a bare prefix completes against
  the path root's listing). Any other free-text field suggests only once the trailing
  token *looks like a path*: starts with `./`, `../`, `/`, `~`, `{cwd}`, or contains a
  separator. Secret fields never suggest (they are already non-insertable).
- **Token-aware**: for `multiple` fields the last shlex piece is completed. Leading
  `~`/`{cwd}`/`{env:NAME}` are expanded (`tokens.expand`) to *find* the directory, but
  the ghost text preserves the user's typed prefix verbatim — the stored value stays
  intent, exactly as today.
- Directories complete with a trailing separator so completion chains; hidden entries
  appear only when the typed segment starts with `.`; listings are capped at a stated
  constant so a node_modules-sized directory cannot stall the loop. Case sensitivity
  follows the filesystem.

### 5. File picker modal (browse layer)

`FilePickerModal` mirrors `EnvPickerModal` (filter `Input` above an `OptionList`,
height-tiered via `tui_layout` classes, Esc chip) plus one piece of state: the current
directory, starting at the path root.

- **Rows**: a pinned first row *"(use this directory)"* that selects the current dir
  itself — directory selection needs no extra chord and is mouse/keyboard-symmetric by
  being an ordinary row — then subdirectories, then files, alphabetical. Typing
  filters.
- **Keys** (each a footer chip, each with a positive pilot test): `Enter` on a
  directory descends, on a file inserts; `Backspace` with an empty filter ascends to
  the parent; `Esc` cancels. No new global chords.
- **Inserted value**: relative to the path root when the selection is inside it,
  absolute otherwise. Navigating above the root is allowed — this is the user's
  machine and their own run; the picker is a convenience, not a sandbox, and no
  security boundary is claimed.
- **Entry points**: `TokenMenuModal` gains a *"File or folder…"* row available to every
  insertable field, chaining into the picker the way *"Environment variable…"* chains
  into `EnvPickerModal`. On a `path`-typed field that row is **first and highlighted**,
  so `Ctrl+T, Enter` is the two-keystroke browse path. `Ctrl+T` keeps its one grammar
  meaning — insert a value — and no per-type chord is introduced.

## CLI surface (additive only)

- `skit params NAME --type field=path` — an existing operation, one new legal value;
  round-trips through both the `[tool.skit]` block and meta `[[parameters]]`.
- Every `--json` face that reports a parameter `type` may now say `"path"` — a new
  value on an existing key, additive.
- `skit run` / `--set` / `-- passthrough` behavior is unchanged: no new validation, no
  new flags. Headless remains deterministic; the non-interactive contract is untouched.
- No shell completion of `--set` values in P1 (deferred, pairs naturally with #9).

## i18n

New msgids: the picker title, *"File or folder…"*, *"(use this directory)"*, the
Up/Cancel chips, and the `path` type label. Standard workflow (`extract` → `update` →
translate zh_CN + zh_TW to 100% → `compile`), watching for fuzzy-match damage.

## Correctness & security risks, each pinned by a test

1. **Preview/launch divergence** — glob feedback and completion both use the resolved
   workdir (§3); test with an `"origin"`-workdir entry.
2. **Event-loop stalls** — the suggester is async and listings are capped; a test
   exercises the cap.
3. **A suggestion is never a value** — ghost text uncommitted until explicitly
   accepted; the assembled command uses the Input's actual text only.
4. **Secrets stay dark** — no suggestions, no picker, no history: secret fields are
   excluded at the same gate as `insertable`, and `argstate` already strips secret
   values structurally.
5. **Forward-compat degrade** — `type = "path"` read by the previous coercion rules
   yields `"str"`, never an error (`_coerce_literal` test).
6. **Windows** — inserted separators use `os.sep`; `~` expansion is for lookup only and
   never rewritten into the stored value.
7. **Corpus fidelity** — new analyzer corpus files are byte-exact and fixer-excluded.
8. **Merge-conflict containment** — all new TUI code in `tui_pathpick.py`; the
   `tui_form.py` diff is hook-sized, and P1b is sequenced after the prompt-kind form
   work stabilizes.

The usual hard gates apply: ruff, ty strictest, 100% coverage, mutmut zero survivors,
i18n 100%, agent-skill sync (the skill's `skit params` teaching must still resolve —
check whether it enumerates types), golden-corpus byte fidelity.

## Phases

- **P1a — signal core (low-conflict, can land while prompt-kind is open)**:
  `params.py` type axis, Python analyzer detection + corpus, declared/meta/CLI/JSON
  round-trip, tests.
- **P1b — TUI (after the prompt-kind form stabilizes)**: `tui_pathpick.py` (suggester +
  picker), token-menu row, the shared path root + glob-feedback fix, pilot tests,
  i18n.
- **P1c — ship polish**: demo-asset regeneration (the run form's visible copy changes),
  agent-skill sync if the CLI teaching mentions types, README only if screenshots are
  referenced anew.

## Deferred (explicitly out of scope)

- **#9 seed args** (`skit -- ./file 3` carried into the TUI insert menu) — own design,
  own branch.
- `click.Path(exists=…, dir_okay=…, file_okay=…)` semantics: validation and picker
  filtering (files-only / dirs-only).
- Shell completion of `--set name=<path>` values and of `--` passthrough args.
- Fuzzy matching; recency-ranked suggestions sourced from `argstate` last-used values.
- A tree-view (DirectoryTree) or hybrid picker as a later iteration.
