# Path-aware parameter entry — design

Status: **review-clean** (v4, 2026-07-19; three adversarial design review rounds —
round 3 signed CLEAN with the two final line edits applied). Resolves [#7](https://github.com/t41372/skit/issues/7) — "Make file
selector more intuitive". Read `docs/design/multilang.md` first (the `ParamDecl`
universal model and the kind registry) and `docs/design/prompt.md` for the run form's
current shape — this design is an additive layer on both. Ships from
`feat/path-picker`, which lands **after** `feat/prompt-kind` merges; file:line
references are as of base `91a70ea` and are anchored by symbol name where the form is
in flux.

Revision notes (v1 → v2, from the adversarial review — each verified against the code
before being adopted):

- **v1's "one completion root" conflated three coordinate systems and would have made
  the glob feedback lie.** Glob pieces are expanded by *skit itself* at assemble time
  against `Path.cwd()` (`flows._split_multi` / `_expand_glob_piece`, called with
  `cwd=Path.cwd()` from both frontends — `cli.py:2590`, `tui.py:782`); the child never
  sees a pattern. Re-rooting the ✓/✗ feedback at the resolved workdir would have made
  it disagree with the actual expansion. §3 now splits the roots by value shape and
  retracts the "divergence dies as a side effect" claim; the *real* pre-existing
  divergence (glob expansion at invoke cwd vs. the child resolving the resulting
  relative paths at an `origin`/`store` workdir) is out of scope and Deferred.
- **Picker insertion is now specified per field shape.** v1 inherited
  `insert_text_at_cursor` (`tui_form.py:716`) by silence; on a prefilled path field the
  flagship `Ctrl+T, Enter` journey would have produced `data/a.csvdata/b.csv`. §5:
  single-value fields are **replaced**; `multiple`/extra-args get an appended,
  shlex-quoted piece; token/env rows keep at-cursor.
- **Separators and quoting:** picker and suggester insert `/` on all platforms (never
  `os.sep` — `multiple`/extra-args values are re-parsed with POSIX `shlex.split`
  (`flows.py:654-664`), which eats backslashes), and any piece appended to a
  shlex-parsed field is `shlex.quote`d (spaces in filenames are the normal case a
  picker exists for).
- **The `[tool.skit]` block home now carries `path` for real instead of pretending.**
  v1 claimed CLI round-trip through the block; in fact declared-schema ops are refused
  on `params_io` kinds (`cli.py:3456` — "manages its parameters from the script
  itself"), and a hand-edited `type = "path"` would have been flagged as permanent
  drift by `reconcile` (the analyzer re-derives `str` for a string constant) with
  `--resync` destroying the declaration. Rather than scope the block out (which would
  fork the type enumeration per kind across the settings screen and the `bad-type`
  message — a lane asymmetry), reconcile learns one compatibility rule: **a declared
  `path` on a source-derived `str` is a refinement, not drift**, and resync preserves
  it. Pinned both ways (§1).
- **Picker mechanics specified at the seams the precedent behaves differently:**
  Enter on the focused filter Input fires `Input.Submitted` (in `EnvPickerModal` that
  accepts the *typed text*; the picker must instead act on the highlighted row); the
  filter clears on descend; Backspace-ascend needs a priority binding that delegates
  `delete_left` while the filter is non-empty (the documented, guarded exception to the
  editing-chord rule — a single-Input modal with delegation preserved) and no-ops at
  the filesystem root; the modal header shows the current directory.
- **Nonexistent path root is handled** (the CLI inline form opens with no preflight,
  `cli.py:2514-2541`): suggester goes silent; the picker opens at the nearest existing
  ancestor, else the invoke cwd, with a notice.
- **Honest forward-compat:** older-skit *reads* degrade `path` → `str`; an older skit
  that *writes* the schema (edit ops, settings save, `--resync`) persists the degraded
  type for every row. Accepted, stated.
- Smaller: the i18n section enumerates the four *changed* closed-set msgids, not just
  the new strings; SKILL.md enumerates field types (`skills/skit/SKILL.md:46`) so the
  skill sync moves into P1a where `--json` starts emitting `"path"`; the ghost-accept
  `→` gesture's mouse story is argued, not assumed.

Revision notes (v2 → v3, from review round 2 — the round verified each v1 resolution
against the code instead of rubber-stamping it):

- **Deleted v2's fabricated `--manage`-argparse claim.** There is no operation that
  manages an argparse param: `--manage` draws exclusively from the analyzer's
  const/input candidates (`analysis.py:311`, `_apply_add`), never from the CLI-reader
  lane. The true statement stands alone: the reader lane is in-memory per read — no
  storage, no drift surface, no predicate needed there.
- **The picker header shows the absolute current directory, always.** v2's
  "relative to the path root while inside it" would render `.` at the moment the
  picker opens — hiding the root in exactly the `origin`/`store`/absolute-workdir
  cases the header exists for. The header is also the one surface where a user
  *learns* that this entry's relative paths resolve somewhere other than their cwd.
- **§3 row 2 is now a two-step rule** (it was incoherent for `{env:X}`): expand the
  token prefix first — expansion failure, e.g. an unset variable (`tokens.expand`
  raises `TokenError`, `tokens.py:93-99`), means no suggestion — then complete inside
  an *absolute* expansion, or hand a *relative* expansion to row 1 (the child resolves
  it at the workdir, so the path root is the correct base). The glob-feedback
  agreement claim is scoped to `multiple`/extra-args values — `assemble` never
  expands a glob typed into a single-value field.
- **The Backspace "guarded exception" is gone — a no-exception mechanism exists.**
  The filter is an Input subclass overriding `action_delete_left`: empty value posts
  an Ascend message, otherwise `super()` — editing is byte-identical *by
  construction*, and a plain non-priority screen binding covers Backspace while the
  OptionList has focus. Zero priority bindings; the constitutional rule stays intact
  instead of bending under a paragraph of justification.
- **Windows activation:** the path-ish recognition rules gain `\`-containing and
  drive-letter (`X:\`, `X:/`) forms on Windows — v2 fixed the insert side but left
  recognition POSIX-only, deadening the universal affordance for Windows typists.
- **The picker's result is a discriminated type**, not a raw string: three insertion
  regimes (replace / append-quoted / at-cursor) now flow through the single
  `action_insert_token` channel (`tui_form.py:713-717`), so the callback must be able
  to tell a picked path from a token by construction.
- Factual tightenings: the meta-home kind set is *every* kind without `params_io`
  (exe, command, prompt, powershell, and the ruby/perl/lua/r long tail —
  `cli.py:3433`), not the three v2 named; "all copy-mode scripts" → the copy-mode
  *default* (legacy copy-mode entries persisted with `"origin"` exist and are
  recovered at `launcher.py:54-61`).

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
- Both serialization homes carry it — the `[tool.skit]` block (`to_block_dict`) and
  meta `[[parameters]]` (`to_meta_dict`) — as a new legal **value** of the existing
  `type` key; the frozen key set is untouched.
- **Reconcile compatibility (the piece that makes the block home real):** the declared
  lane re-derives a param's type from its source constant, which for a string literal
  is always `"str"` — without help, a declared `path` is eternal drift
  (`analysis.py:400-402`) whose own banner recommends the `--resync` that erases it
  (`analysis.py:285-291`). One predicate — *stored `"path"` is compatible with derived
  `"str"`* — is applied at both consumers: `reconcile` does not report it as changed,
  and `_apply_resync` preserves the declared `path` instead of rewriting it. Any other
  mismatch (e.g. declared `path` over a derived `int` constant) stays honest drift.
  Pinned by tests in both directions.
- Routing is unchanged: declared-schema CLI ops remain refused on `params_io` kinds
  (`cli.py:3456`) — the documented route there is `--manage` / editing the block, which
  now works. `skit params NAME --type field=path` works where `--type` works today
  (every meta-home kind — see CLI surface).
- **Forward compat, both halves:** an older skit *reading* `type = "path"` degrades it
  to `"str"` via `_coerce_literal` (`params.py:154` block, `:205` meta) — graceful,
  pinned by a test. An older skit that *writes* the schema (any `edit_declared` op, a
  settings-screen save, `--resync`) persists the degraded `str` for every row,
  silently deleting the typing. That loss is accepted and documented; no mechanism
  pretends otherwise.
- `flows.FormField`: the `kind` whitelists admit `"path"`; `validate_value` treats it
  as `str` (no checks). `tui_form._type_label` gains a translated label.

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

The CLI-reader lane is in-memory per read — nothing is stored, so no drift surface
exists there. (There is no operation that persists an argparse param into the managed
schema: `--manage` draws exclusively from the analyzer's const/input candidates,
`analysis.py:311`.)

Shell, JS/TS, fish, PowerShell get **no name-based heuristics** — inferring "path" from
a parameter being called `file` is guessing, and skit does not guess. Those languages
reach `path` via declared `type = "path"`; the universal affordance covers the
undeclared case regardless.

New golden-corpus inputs cover each detection (argparse `Path`/`FileType`, `click.Path`,
`click.File`, typer `Path`) — byte-exact, excluded from fixers, like all corpus files.

### 3. Completion roots — three coordinate systems, named honestly

A value can reach the filesystem three different ways, and the completion machinery
must use the matching root for each — one slogan-root would lie:

| value shape | who resolves it, when | root the form must use |
| --- | --- | --- |
| bare relative path | the **child process**, at its resolved workdir | `launcher._resolve_workdir(entry, Path.cwd())` — the **path root** |
| `~` / `{cwd}` / `{env:X}` prefix | `tokens.expand` at assemble time substitutes text; the child resolves the result | two-step rule below |
| glob piece in a `multiple`/extra-args field | **skit** at assemble time, `Path.cwd()` | invoke cwd — unchanged |

- The **suggester and picker** root bare input at the path root. A token-prefixed
  lookup follows a **two-step rule**: expand the prefix first (`tokens.expand`; an
  expansion failure — e.g. an unset `{env:X}`, which raises `TokenError` — means no
  suggestion); if the expansion is **absolute** (`~`, `{cwd}` always are), complete
  inside it; if it is **relative** (an env var holding `data/sub`), the child will
  resolve the substituted text at its workdir, so hand it to row 1 and complete
  against the path root.
- The **glob feedback line stays exactly as it is** — for `multiple`/extra-args
  values it evaluates where `assemble` will expand, and they keep agreeing; a glob
  typed into a single-value field is never expanded by `assemble` at all
  (`flows.py:549-578`), so feedback there is a hint either way. v1's claim that this design fixes a
  preview/launch divergence is retracted: the real divergence is that `assemble`
  expands globs at the invoke cwd while a non-`invoke` workdir child resolves the
  resulting relative paths elsewhere — a **pre-existing** behavior this design must
  not silently change. Deferred, with the fix sketched there.
- For `workdir="invoke"` (the copy-mode default, and pinned for command and prompt
  entries — legacy copy-mode entries persisted with `"origin"` exist and are recovered
  at `launcher.py:54-61`) all three roots coincide with `Path.cwd()` — the common case
  has one root.
- **Nonexistent path root** (reference-mode entry whose origin vanished; the CLI
  inline form runs no preflight): the suggester suggests nothing; the picker opens at
  the nearest existing ancestor, else the invoke cwd, and says so in its header.

### 4. Ghost suggester (typing layer)

All new TUI code lives in a **new module `src/skit/tui_pathpick.py`** (suggester +
picker modal together); `tui_form.py` takes only thin hooks. This is deliberate
conflict containment — the run form is under heavy rework on `feat/prompt-kind` —
and it keeps the eventual rebase near-trivial.

`PathSuggester(Suggester)` — async, fish-style ghost text, `→` accepts:

- **Activation**: a `path`-typed field suggests always (a bare prefix completes against
  the path root's listing). Any other free-text field suggests only once the trailing
  token *looks like a path*: starts with `./`, `../`, `/`, `~`, `{cwd}`, or contains a
  `/` — and on Windows additionally when it contains a `\` or starts with a drive
  letter (`X:\`, `X:/`); completions still insert `/` separators, which every consumer
  resolves. Secret fields never suggest (they are already non-insertable).
- **Token-aware**: for `multiple` fields the last shlex piece is completed. Leading
  `~`/`{cwd}`/`{env:NAME}` are expanded (`tokens.expand`) to *find* the directory —
  per §3's two-step rule — but the ghost text preserves the user's
  typed prefix verbatim; the stored value stays intent, exactly as today.
- Directories complete with a trailing `/` (all platforms — see §5 on separators) so
  completion chains; hidden entries appear only when the typed segment starts with
  `.`; listings are capped at a stated constant so a node_modules-sized directory
  cannot stall the loop. Case sensitivity follows the filesystem.
- **Mouse story, argued**: `→`-to-accept is keyboard sugar over typing, not a
  mouse-orphaned capability — the *action* ("get this path into the field") is fully
  mouse-operable via the `▾ insert` link / footer chip → *"File or folder…"* → picker
  rows. `→` is deliberately not footer-advertised (it is the stock Textual suggester
  gesture, like `Ctrl+A`-home on an Input), so the advertised-key pilot-test policy
  does not attach; the suggester itself is unit-tested.

### 5. File picker modal (browse layer)

`FilePickerModal` mirrors `EnvPickerModal`'s bones (filter `Input` above an
`OptionList`, height-tiered via `tui_layout` classes, Esc chip) plus directory state,
with the seams specified where the precedent behaves differently:

- **Header shows the current directory, absolute, always** — at open the current dir
  *is* the root, and a relative rendering would show `.` precisely on the
  `origin`/`store`/absolute-workdir entries the header exists for. This header is the
  one surface where a user ever *learns* that this entry's relative paths resolve
  somewhere other than their cwd (§3).
- **Rows**: a pinned first row *"(use this directory)"* that selects the current dir
  itself — directory selection needs no extra chord and is mouse/keyboard-symmetric by
  being an ordinary row — then subdirectories (trailing `/`), then files,
  alphabetical. Typing filters; hidden entries only when the filter starts with `.`;
  listings capped at the same constant as the suggester.
- **Keys** (each a footer chip, each with a positive pilot test):
  - `Enter` — acts on the **highlighted row** (descend into a directory / insert a
    file), including when fired from the focused filter Input: the `Input.Submitted`
    handler routes to the highlighted option. (The `EnvPickerModal` precedent accepts
    the raw typed text on submit; the picker must not — a half-typed filter is not a
    path.)
  - `Backspace` — ascends to the parent **only when the filter is empty**; while the
    filter has text it deletes normally. Mechanism: the filter is an Input subclass
    overriding `action_delete_left` — empty value posts an Ascend message, otherwise
    `super()` — so editing is byte-identical *by construction*; a plain non-priority
    screen binding covers Backspace while the OptionList has focus. No priority
    binding, no exception to the editing-chord rule needed. At the filesystem root it
    no-ops.
  - `Esc` — cancels.
  - The filter **clears on descend** (a sticky filter would land every descend on an
    empty list).
- **Inserted value**: relative to the path root when the selection is inside it,
  absolute otherwise; separators are `/` on every platform (Windows accepts them
  everywhere skit or a child will resolve the value, and POSIX `shlex.split` — which
  re-parses `multiple`/extra-args values, `flows.py:654-664` — eats backslashes).
  Navigating above the root is allowed — this is the user's machine and their own run;
  the picker is a convenience, not a sandbox, and no security boundary is claimed.
- **Insertion semantics, per field shape** (the flagship journey must survive a
  prefilled field):
  - single-value free-text/path field → the picked value **replaces** the field's
    text;
  - `multiple` field / extra-args row → the picked value is **appended** as a new
    piece, `shlex.quote`d (spaces in filenames are the normal case);
  - the token menu's existing rows (`{cwd}`, `~`, env var, …) keep their at-cursor
    insertion — tokens compose into larger values; a picked path *is* the value.
  - Because three regimes now flow through the single `action_insert_token` channel
    (`tui_form.py:713-717`), the picker's dismissal result is a **discriminated
    type** (a picked-path wrapper vs. a plain token string), so the callback can tell
    them apart by construction rather than by sniffing the text.
- **Entry points**: `TokenMenuModal` gains a *"File or folder…"* row available to every
  insertable field, chaining into the picker the way *"Environment variable…"* chains
  into `EnvPickerModal`. On a `path`-typed field that row is **first and highlighted**,
  so `Ctrl+T, Enter` is the two-keystroke browse path. `Ctrl+T` keeps its one grammar
  meaning — insert a value — and no per-type chord is introduced.

## CLI surface (additive only)

- `skit params NAME --type field=path` — an existing operation, one new legal value,
  on the kinds where `--type` operates today: every kind without `params_io` (exe,
  command, prompt, powershell, and the ruby/perl/lua/r long tail — `cli.py:3433`);
  `params_io` kinds keep
  their existing routing (schema lives in the script's block, edited there or via
  `--manage`) and the block now carries `path` without drift (§1).
- Every `--json` face that reports a parameter `type` may now say `"path"` — a new
  value on an existing key, additive. The Agent Skill enumerates the type set
  (`skills/skit/SKILL.md:46`), so the skill + packaged-copy sync is part of **P1a**,
  not ship polish.
- `skit run` / `--set` / `-- passthrough` behavior is unchanged: no new validation, no
  new flags. Headless remains deterministic; the non-interactive contract is untouched.
- No shell completion of `--set` values in P1 (deferred, pairs naturally with #9).

## i18n

New msgids: the picker title/header, *"File or folder…"*, *"(use this directory)"*, the
Up/Cancel chips, the nonexistent-root notice, and the `path` type label. **Changed**
msgids (the closed-set enumerations — exactly the fuzzy-match hazard): the `--type`
help text (`cli.py:3234`), the `bad-type:` message (`cli.py:3847-3849`), the
settings-screen type placeholder (`tui_settings.py:152`) and its unknown-type notify
(`tui_settings.py:680`). Standard workflow (`extract` → `update` → translate zh_CN +
zh_TW to 100% → `compile`), checking each formerly-translated enumeration by hand.

## Correctness & security risks, each pinned by a test

1. **Root confusion** — the three coordinate systems of §3 each get a test: bare
   relative completion on an `origin`-workdir entry roots at the origin dir; a
   `{cwd}`-prefixed lookup roots at the invoke cwd; an unset `{env:X}` prefix
   suggests nothing (never a traceback); a relative-valued `{env:X}` completes
   against the path root; glob feedback matches what `assemble` expands for
   `multiple`/extra-args values.
2. **Field corruption** — replace-vs-append semantics per field shape (§5), pinned on
   a prefilled single field and a populated `multiple` field; a picked
   space-containing filename survives `_split_multi` round-trip intact.
3. **Reconcile fight** — declared `path` over a derived `str` is not drift and
   survives `--resync`; declared `path` over a derived `int` IS drift. Both pinned.
4. **Event-loop stalls** — the suggester and picker listings are async/capped; a test
   exercises the cap. (The synchronous glob feedback is unchanged by this design and
   keeps its current cost profile.)
5. **A suggestion is never a value** — ghost text uncommitted until explicitly
   accepted; the assembled command uses the Input's actual text only.
6. **Secrets stay dark** — no suggestions, no picker, no history: secret fields are
   excluded at the same gate as `insertable`, and `argstate` already strips secret
   values structurally.
7. **Forward-compat degrade** — `type = "path"` read by the previous coercion rules
   yields `"str"`, never an error (`_coerce_literal` test); the write-side stripping
   by older versions is documented as accepted (§1), not silently claimed away.
8. **Windows** — inserted values use `/` separators; `~`/token expansion is for lookup
   only and never rewritten into the stored value; the space-and-backslash cases from
   revision note 3 are pinned, and the `\`/drive-letter activation forms (§4) get
   their own recognition tests.
9. **Missing root degrade** — suggester silent, picker at nearest existing ancestor
   with notice (§3), pinned via a vanished-origin reference entry.
10. **Corpus fidelity** — new analyzer corpus files are byte-exact and fixer-excluded.
11. **Merge-conflict containment** — all new TUI code in `tui_pathpick.py`; the
    `tui_form.py` diff is hook-sized, and P1b is sequenced after the prompt-kind form
    work stabilizes.

The usual hard gates apply: ruff, ty strictest, 100% coverage, mutmut zero survivors,
i18n 100%, agent-skill sync, golden-corpus byte fidelity.

## Phases

- **P1a — signal core (low-conflict, can land while prompt-kind is open)**:
  `params.py` type axis, the reconcile/resync compatibility predicate, Python analyzer
  detection + corpus, declared/meta/CLI/JSON round-trip, SKILL.md + packaged-copy
  sync, tests.
- **P1b — TUI (after the prompt-kind form stabilizes)**: `tui_pathpick.py` (suggester +
  picker), token-menu row, the per-shape roots of §3, pilot tests, i18n.
- **P1c — ship polish**: demo-asset regeneration (the run form's visible copy changes),
  README only if screenshots are referenced anew.

## Deferred (explicitly out of scope)

- **#9 seed args** (`skit -- ./file 3` carried into the TUI insert menu) — own design,
  own branch.
- **Unifying the glob-expansion root**: `assemble` expands `multiple`/extra-args globs
  at the invoke cwd while a non-`invoke` workdir child resolves the resulting relative
  paths at its own workdir — a pre-existing divergence. The defensible end state is
  expanding at `_resolve_workdir(...)` (both frontends' `assemble` call sites), owned
  explicitly as a behavior change for existing presets; not smuggled into this design.
- `click.Path(exists=…, dir_okay=…, file_okay=…)` semantics: validation and picker
  filtering (files-only / dirs-only).
- A type op for the in-file (`params_io`) lane, so block-home types can be edited
  without hand-editing the script.
- Shell completion of `--set name=<path>` values and of `--` passthrough args.
- Fuzzy matching; recency-ranked suggestions sourced from `argstate` last-used values.
- A tree-view (DirectoryTree) or hybrid picker as a later iteration.
