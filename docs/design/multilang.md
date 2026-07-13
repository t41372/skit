# Multi-language support — design & implementation plan

Status: **in progress** on branch `feat/multilang`. This document is the single source of
truth for the design decisions; it was synthesized from three independent architecture
studies (type-system lens, integration/migration lens, language-mechanics lens) plus a
verified facts dossier (tree-sitter APIs/wheels, runtime status, PowerShell/fish parsing).

## Mission

skit grows from "Python deepest, exe/command shallow" to a true multi-language script
library: shell (bash/sh/zsh) and fish, JS/TS, PowerShell, plus a data-driven long tail
(ruby/perl/lua/R/cargo/jbang), with a **declarative parameter schema** so any kind —
including exe and command — gets the full form/preset/--set experience.

Decisions already made (user-approved, not up for re-litigation):

- tree-sitter is a **hard dependency** (C extensions accepted). Contained to analysis:
  launch paths stay stdlib-only, and a failed grammar import degrades `analyzer=None`
  instead of crashing.
- **No runtime downloads** beyond uv: assume user tooling (js → deno/bun/node on PATH,
  rust → cargo, powershell → pwsh/powershell.exe). Detect, honestly refuse otherwise.
- A5 amendment: **opt-in idiom normalization** at add time (`VAR=800` →
  `VAR="${VAR:-800}"`) may edit the stored copy (never the original) with explicit consent.
- Windows shell policy: find bash on PATH → config key (`shell.bash_path`) → honest
  exit 126. Never silently reroute through WSL.
- Shebang sniffing at add time: an extensionless +x file with a recognized shebang is
  suggested as its interpreted kind (was: exe).
- Long-term-optimal / most-elegant choices; implementation cost is not a factor.
  Existing solutions over hand-written algorithms.

Hard gates: ty strictest, 100% coverage floor, mutmut zero survivors, i18n 100%
(zh_CN/zh_TW), agent-skill sync test, golden-corpus byte fidelity.

## Core architecture

### The three axes `kind` used to conflate

1. **kind** (open `str`) — language identity & registry key: `python`, `shell`, `fish`,
   `js`, `ts`, `powershell`, `exe`, `command`, + long-tail rows. `models.Kind = str`.
   No schema bump: all meta.toml changes are additive; old skit reading a new kind
   lists/. shows it and fails run with the existing "Unknown entry kind" LaunchError.
2. **launch payload** (closed sum type) — where exhaustiveness actually matters:
   `ArgvLaunch(argv) | ShellLaunch(command)`, matched exhaustively in `run_entry`.
3. **capabilities** — `Capability | None` fields on a frozen `LangSpec`; the call-site
   idiom everywhere is `if spec.analyzer is None: <degrade>` (ty narrows structurally;
   no casts, no hasattr).

### Registry (`src/skit/langs/`)

```
src/skit/langs/
  base.py            # LEAF: Protocols (Analyzer, CliReader, ParamsIO, Injector,
                     #   SyntaxCheck, LaunchStrategy), LangSpec, CommentSyntax, Family
  registry.py        # REGISTRY: dict[str, LangSpec]; spec_for() -> LangSpec | None;
                     #   infer_kind(path) (extension table + shebang peek); stored_name()
  launch.py          # UvLaunch, DirectLaunch, TemplateLaunch, InterpreterLaunch,
                     #   RunnerLaunch(order=("deno","bun","node")); Launch/LaunchContext,
                     #   ArgvLaunch | ShellLaunch sum type
  table.py           # long-tail data rows (ruby/perl/lua/r/cargo/jbang): launch-only specs
  python/            # analyzer.py, cli_reader.py (was argspec), inject.py (was shim),
                     #   io.py (frozen [tool.skit] disk mapping), __init__.py exposes SPEC
  shell/             # P3: analyzer.py (tree-sitter-bash), inject.py, __init__.py
  javascript/        # P4: analyzer.py (ts-js/ts grammars), cli_reader.py (parseArgs), inject.py
  fish/              # P5: analyzer.py (hand scanner), __init__.py
  powershell/        # P5: cli_reader.py (pwsh native AST subprocess), __init__.py
src/skit/rewrite.py      # neutral absolute-byte-offset splice core (ByteSpan, apply_byte_spans,
                         #   linecol_to_byte adapter for Python ast; write_injected moves here)
src/skit/commentblock.py # neutral comment-prefix-parameterized block engine ("# ///" & "// ///")
src/skit/callmatch.py    # neutral 1:1 call matching (was analyzer._match_inputs + multiset pass)
src/skit/params.py       # ParamDecl — THE universal param model
src/skit/analysis.py     # Analysis, Candidate, Report, drift_lines, edit_specs (language-neutral)
```

- Registration is **explicit aggregation** in `registry.py` (imports each `SPEC`);
  no import-time side-effect registration.
- Python's language modules **move** into `langs/python/` (no facades; all imports
  updated). `pep723.py` stays top-level as the Python-deps module but its block engine
  is extracted into `commentblock.py`; byte-fidelity pinned by the golden corpus and by
  a regression assert that the `#`-bound block regex equals the current frozen literal.
- Every kind label is a **static** `gettext()` literal in one render-side dict
  (Babel-extractable); the registry holds only the locale-independent `glyph`.

```python
@dataclass(frozen=True, slots=True)
class LangSpec:
    kind: str
    family: Family                      # Literal["interpreted", "binary", "template"]
    glyph: str
    launch: LaunchStrategy              # required
    extensions: tuple[str, ...] = ()
    shebangs: tuple[str, ...] = ()      # program basenames recognized in a shebang
    default_interpreter: str = ""       # shell: "bash"
    stored_name: str = ""               # in-store copy filename; python PINNED "script.py"
    comment: CommentSyntax | None = None
    params_io: ParamsIO | None = None   # None => declared params live in meta [[parameters]]
    analyzer: Analyzer | None = None
    cli_reader: CliReader | None = None
    injector: Injector | None = None
    syntax_check: SyntaxCheck | None = None
    supports_modes: bool = False        # copy/reference choice
    supports_deps: bool = False         # PEP-723-style package deps (python only today)
    takes_argv: bool = True             # False for command (placeholders are its argv)
```

### Launcher

`launcher.py` keeps its public functions as thin generic shells:

```python
@dataclass(frozen=True)
class Launch:
    payload: ArgvLaunch | ShellLaunch
    env_overlay: Mapping[str, str] = ...   # delivery=env values
    script_override: Path | None = None
```

`run_entry`: `env = {**os.environ, **config.mirror_env(...), **launch.env_overlay}`
(param env wins last — a deliberate user override). `match launch.payload:` is
exhaustive. `target_missing`/`preflight` route through `spec.launch.target()` /
`.preflight()`; `preflight` also checks `meta.needs` via `shutil.which` (exit 126/127
semantics preserved: missing target 127, unrunnable 126, skit failure 125).

Transparency for env delivery renders a copy-pasteable prefix, secrets masked:
`→ WIDTH=800 API_KEY=••• bash script.sh` (delivery is a real env overlay, not shell text).

### ParamDecl — the one universal param model

`argspec.ArgField` is deleted (readers emit ParamDecl); `flows.FormField` stays as the
render-only projection built by a single `FormField.from_decl`; `metawriter.ParamSpec`
disappears — the frozen on-disk `[tool.skit]` mapping lives in `langs/python/io.py`
(and the shared engine in `commentblock.py`), pinned byte-for-byte by the corpus.

```python
Binding  = Literal["const", "call", "envdefault", "none"]   # source anchor class
Delivery = Literal["inject", "env", "flag", "positional", "placeholder"]
ParamType = Literal["str", "int", "float", "bool", "choice"]

@dataclass
class ParamDecl:
    name: str
    binding: Binding = "none"
    delivery: Delivery = "flag"
    type: ParamType = "str"
    default: str | int | float | bool | None = None
    required: bool = False
    multiple: bool = False
    choices: tuple[str, ...] = ()
    prompt: str = ""            # form label / literal call prompt
    help: str = ""
    secret: bool = False
    env_source: str = ""        # secret VALUE read from this env var (existing concept)
    flag: str = ""              # delivery=flag: "--output"; "" => positional
    action: str = ""            # store_true | store_false
    order: int = -1             # binding=call: site order (python input, shell read)
    env_target: str = ""        # delivery=env: variable to SET (defaults to name)
    degraded: bool = False
```

Validated invariants: `const|call ⇒ inject`; `envdefault ⇒ env`;
`none ⇒ env|flag|positional|placeholder`.

**Storage** (chosen solely by `spec.params_io`, never merged — no precedence bugs):

- comment-capable kinds (python/shell/fish/js/ts/powershell): in the file body's
  `[tool.skit]` block (existing `# /// script` engine; `// ///` for JS). Copy mode:
  skit writes it; reference mode: read-only (A7).
- exe/command/long tail: `meta.toml [[parameters]]` (new key; full ParamDecl mapping).
  The **full serialization format for all five deliveries is defined and tested in P1**,
  before shell/js pile on (no mid-branch format migration).
- command's legacy `params: list[str]` placeholder cache is **retained as a synced
  write-through denormalization** so older skit still prompts placeholders (downgrade
  safety); rich data lives in `[[parameters]]`.
- On-disk key stays `kind` with values `const|input|envdefault|declared`
  (`input` ↔ binding "call" — frozen for python back-compat).

`load_decls(entry)` (in flows) is the one generic loading chain:
declared (params_io or meta) → reconcile if analyzable → else cli_reader → else
legacy placeholders → else none.

**JSON contracts are additive-only**: `show --json` keeps `param_source` legacy tokens,
adds `param_origin: declared|reader|none`; per-field `source` gains `env`/`positional`
values; `doctor --json` adds `needs_missing`. SKILL.md updated in the same commit
(byte-sync + command-tree tests enforce).

### Shell analyzer/shim (tree-sitter-bash, node-walk — no query strings)

Detection (top-level = direct children of `program`):

- **const**: `variable_assignment` (or `declaration_command` export/readonly/declare —
  but never `local`) with literal RHS: `word`/`number` with no expansion children,
  `raw_string`, or `string` whose children are only `string_content`. `VAR=` empty,
  arrays, concatenations, command substitutions excluded. Last-write-wins dedupe like
  python. `readonly`/`declare -r` ⇒ inject-only (normalization refused).
- **envdefault**: `expansion` with `operator` field in `{:-, :=, -, =}` (operator is an
  anonymous token — read `child_by_field_name("operator")`, it does NOT appear in
  S-expr dumps). **Suppressed when the same NAME is bare-assigned at top level**
  (const wins; env delivery would silently no-op — the #1 correctness rule).
- **read**: `command` with `command_name` "read" (also `builtin read`/`command read`);
  `-p` prompt (incl. clustered `-sp`), `-s` ⇒ secret **with certainty**, `-r`/other
  flags preserved. **Excluded as data-reading** when: pipeline right-operand, loop fed
  by redirect/process substitution/here-string, or the read itself has stdin
  redirection. Order-keyed; shares `callmatch` with python input().
- **demotions**: `+=`, arithmetic self-reference `$((VAR+1))`, `((VAR++))`, `let`,
  loop-body reassignment ⇒ demoted "accumulator". Function-body assignments excluded
  outright (top-level scan never sees them).
- **hints**: `$0`/`$BASH_SOURCE`/`${BASH_SOURCE[0]}`/`dirname "$0"` ⇒
  `uses_self_location` (warn when const-rewrite would run from a temp copy);
  `$1/$@/$#/getopts` ⇒ `uses_argv`.
- **types**: `int` iff `^-?\d+$`, `float` iff `^-?\d+\.\d+$`, **bool never inferred**
  (shell has no bool; user may hand-declare). str otherwise.

Delivery:

- envdefault → **env overlay, zero rewrite, no temp copy** ($0-safe path).
- const → byte-span rewrite (tree-sitter absolute offsets → `rewrite.ByteSpan`) on a
  temp copy (`rewrite.write_injected`, 0600, OS tmp). **Quoting normalized, never
  preserved**: int/float bare; str always POSIX single-quoted with `'\''` escaping
  (immune to expansion/word-splitting — closes injection).
- read → per-call-site rewrite `read …` → `_skit_read K 'value' <0|1> 'prompt' …` (span
  replaces only the command name; every original flag/varname survives) + a preamble
  defining `_skit_read`: one-shot per SITE, echoes prompt+value (`***` if secret), then
  falls through to the real read. Site binding = same correctness level as python's
  `_skit_i[K]`.

  **Empirically corrected (the original sketch was wrong — verified on this machine):**
  - macOS `/bin/bash` is **3.2**: no associative arrays, no `[[ -v ]]`. The preamble must
    be POSIX-portable — per-site one-shot state lives in plain `_skit_used_<K>` variables
    set through `eval`, never an array.
  - `command read` **fails in zsh** (zsh's `command` bypasses builtins; the read never
    runs and the variable comes back empty). `builtin read` works in bash 3.2/5, zsh and
    macOS `/bin/sh`, but **dash has no `builtin`**. So the fallthrough keyword is
    dialect-selected: `builtin` for bash/zsh, `command` for sh/dash.
  - The queued value is fed through an unquoted heredoc (`<<EOF` / `$_sv` / `EOF`) — a
    parameter-expansion result, so it is not re-scanned for escapes — and reaches the
    function as a single-quote-escaped **argument**, exactly like a const value. Verified
    working on bash 3.2, zsh and `/bin/sh`, including one-shot fall-through to real stdin.
  - Documented dialect variance (not fought): a value containing a backslash is subject to
    that shell's own `read` escape rules — i.e. exactly what the user typing it would get.
- **Dual syntax gates after injection**: (1) mandatory offline re-parse,
  `tree.root_node.has_error` ⇒ InjectError; (2) `<interpreter> -n <tmp>` hardening
  (bash/zsh/sh dialect from `meta.interpreter`).
- Idiom normalization (opt-in, copy-mode, add-time): `VAR=<literal>` →
  `VAR="${VAR:-<literal>}"` — only when the literal contains none of `` } " ` $ \ ``
  or newline; refused for readonly. Post-normalization the param IS an envdefault.

Reconcile: `Report`/`drift_lines`/`edit_specs` move to neutral `analysis.py`;
`reconcile` becomes `spec.analyzer.reconcile(text, decls)`. envdefault drift has a
dedicated loud message when `${NAME:-}` disappears or NAME becomes bare-assigned
(env would silently no-op). `Report.changed` carries `(decl, new_type)` — no ast types.

### JS/TS (P4)

- Runner: `RunnerLaunch` detection deno > bun > node (`meta.runner` override; config
  default). Node 23.6+ strips TS by default (erasable-only — the rule is permanent;
  transform-types was removed in Node 26); deno runs local pure-compute scripts with
  no permission prompts; bun auto-installs imports. Add never requires a runner; run
  without one ⇒ exit 126 honest error; doctor warns.
- Analyzer: top-level `lexical_declaration` kind=const, `variable_declarator` with
  identifier name + literal value (`string` [text in `string_fragment`], `number`,
  `true`/`false`); `template_string`/objects/arrays/patterns excluded; `let`/`var`
  demoted. Injection: strings via `json.dumps` (valid JS superset), numbers/bools bare;
  gate = tree-sitter `has_error` re-parse (+ optional `node --check` when node is the
  runner).
- cli_reader: `util.parseArgs({options:{…}})` static read → flag-delivery ParamDecls
  (`type string/boolean`, `short`, `default` literal, `multiple`). Spread/computed/
  identifier-reference options ⇒ honest degrade.
- Comment blocks: `commentblock` bound to `//` (`// /// script`). Prior art: only an
  open Bun proposal mirrors PEP 723 — no collision.

### PowerShell (P5) — the language's own parser

At add/params time spawn `pwsh -NoProfile -NonInteractive` (fallback `powershell.exe`
on Windows; identical Parser API) running
`[System.Management.Automation.Language.Parser]::ParseFile` → JSON of
`ParamBlockAst.Parameters` (name, StaticType, DefaultValue extent, Mandatory incl.
`ExpressionOmitted`, ValidateSet) + `ScriptBlockAst.GetHelpContent()` for
description/param help. Pure static parse — executes nothing. No pwsh installed ⇒
Tier-0 degrade. Delivery: named flags (`-Name value`, `[switch]` ⇒ store_true).
No shim/injection — param() IS a CLI surface.

### fish (P5) — hand scanner

No standalone PyPI grammar wheel (verified); language-pack is too heavy. fish syntax is
regular (no heredocs): line scanner with single/double-quote state. Detect top-level
`set NAME value` (exclude `set -l`; `set -x` still env-deliverable), `read -P "prompt"`
(capital P is fish's literal prompt; lowercase -p is a COMMAND — never treat as
literal), `read -s` secret, and the builtin `argparse 'h/help' 'n/name=' … -- $argv`
spec strings (`=` value, `=?` optional-attached, `=+`/`=*` append, `x/long` short+long,
`x-long` dummy-short ⇒ only `_flag_long`) → flag-delivery decls. Env overlay works
natively (inherited env vars are fish globals). Default idiom `set -q VAR; or set VAR x`
recognized as envdefault. Injection for `set` consts via `rewrite` + `fish -n` gate.

### Long tail (P2, data rows in `table.py`)

ruby, perl, lua, r (Rscript), cargo (`cargo -Zscript` unstable; prefer `rust-script`
when found), jbang. Launch-only LangSpecs: interpreter argv + comment syntax
(description extraction + in-file blocks for `#`-comment ones) + declared params.
Every row needs one launch test (kills string-literal mutants).

## Phases (restructured after panel critique)

- **P0 — foundation (no behavior change + audited fixes)**: `langs/` package;
  python modules move; neutral cores (`rewrite`, `commentblock`, `callmatch`,
  `params.ParamDecl`, `analysis`); `syntax_check` capability (python binds
  `compile()`); collapse all ~40 kind branch sites + 10 "script.py" literals; open-str
  Kind; shebang-aware `infer_kind`; audit fixes (params-exe `--manage` dead-end hint,
  doctor uv check scoped to python entries, `takes_argv` preserving cli.py:1102
  semantics). Existing tests green; corpus byte-identical; `--json` unchanged.
- **P1 — declarative schema**: full ParamDecl serialization ([[parameters]] +
  in-file); `skit params --add/--rm/--type/--default/--choices/--deliver/--flag/
  --required/--optional/--help-text`; TUI settings param editor for every kind;
  env/flag/positional delivery exercised on exe/command; placeholder migration
  (secret-override fix); env overlay through assemble→execute→run_entry;
  transparency/dry-run/--set/--no-input contracts; SKILL.md.
- **P2 — every interpreted kind launches**: shell kind (interpreter from
  shebang/extension, meta.interpreter, no +x needed, copy mode `script.sh`),
  fish/js/ts/powershell/long-tail rows; `needs` preflight + doctor + health;
  Windows bash policy; `#`-comment description extraction; edit/peek for text kinds;
  add flows (interactive + --no-input) incl. shebang suggestion.
- **P3 — shell analyzer/shim** (tree-sitter hard dep lands here, with import guard):
  detection/delivery/reconcile as above; onboarding (CLI + TUI review); idiom
  normalization consent; shell corpus (~24 files: CRLF, CJK/emoji, heredocs,
  pipe-fed reads, both-assigned-and-defaulted, quoting-injection payloads,
  self-location, zsh dialect).
- **P4 — JS/TS analyzer + parseArgs reader** + js corpus.
- **P5 — PowerShell reader + fish analyzer**; getopts reader only if time allows.
- **P6 — docs/i18n/positioning**: README×3 capability matrix, AGENTS.md amendments
  (A2 parser-dep rule, A5 normalization), SKILL.md sync, translations to 100%,
  demo-assets regeneration (Docker) or explicit staleness note.

Each phase ends: full gate (ruff/ty/pytest --cov) → commit → opus review loop
(review → verify → fix → re-review to zero) → next phase. Mutmut runs at P0, P3, P6
milestones (survivors fixed by dedicated test-filler agents).

## Dependency pins

```
tree-sitter>=0.25,<0.27          # QueryCursor API era; 0.26 current (LANGUAGE_VERSION 15, MIN 13)
tree-sitter-bash>=0.25,<0.26     # abi3, fullest platform coverage (incl. musllinux aarch64)
tree-sitter-javascript>=0.25,<0.26   # P4
tree-sitter-typescript>=0.23,<0.24   # P4 (ABI 14 — loads fine; two langs: typescript+tsx)
```

Core wheel gaps (accepted): no musllinux-aarch64/free-threaded wheels for the core —
sdist builds there; analyzer degrades to None if a grammar fails to import.

## Top correctness risks (each pinned by a test)

1. env delivery silently no-ops vs bare assignment → suppression rule + loud drift.
2. read/input value binds to wrong call site → `_skit_read K` + shared callmatch;
   corpus test with function-read defined-above/invoked-after.
3. quoting injection via const rewrite/read feed → single-quote normalization +
   dual gates; corpus payload `'; rm -rf ~; $(touch pwned)` asserts no side effect.
4. multibyte/CRLF byte misalignment → single `rewrite` core; CJK/emoji corpus both langs.
5. `while read` data loop hijacked → exclusion rule corpus (zero candidates).
6. block-engine generalization corrupts python bytes → frozen-regex assert + corpus.
7. double-binding two decls to one site → second claimant ⇒ drift error (mirror python).
8. JS template literal / let misdetection → excluded/demoted + corpus.
