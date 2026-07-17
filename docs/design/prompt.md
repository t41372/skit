# Prompts as first-class entries — final design

Status: **approved** (v2.2, 2026-07-17; two adversarial review rounds, re-reviewed to zero). Resolves
[#6](https://github.com/t41372/skit/issues/6) — "Make prompts a part of skit?". This
document is the single source of truth for the design, and supersedes the v1 proposal.
Read `docs/design/multilang.md` first — this design is an additive layer on the kind
registry and launch strategies that shipped there.

Revision notes (v1 → v2, all maintainer-decided):

- **Dropped the "secret & length → stdin" mechanism.** The threat model does not hold: a
  prompt's entire content is handed to an agent CLI that writes it — verbatim, in
  plaintext — into its own session logs (`~/.claude/`, Codex session files, …). Shuffling
  the same text between argv and stdin protects nothing; it is security theater. The
  honest stance replaces it (see "Delivery & limits").
- **v1 launches the agent's interactive session** (the prompt as opening message); the v1
  proposal had print/exec seeds (`claude -p`, `codex exec`) and deferred interactive as
  "session management". That framing was wrong: skit already hands the terminal to every
  interactive TUI script it launches — an interactive agent needs no more "management"
  than `htop` does. Print mode is now the deferred half.
- **Runners are argv token lists executed without a shell** (`ArgvLaunch`), not shell
  command strings. Eliminates the quoting-injection surface by construction and makes
  multi-line prompts (the norm) safe on Windows, where `shell=True` means cmd.exe and
  quoted newlines are a minefield.
- **Five seed runners** — claude, codex, opencode, amp, antigravity. gemini-cli is
  deliberately excluded (dead).
- **`family="interpreted"`, not `"template"`** — `base.LangSpec.has_original_file` is
  defined as `family != "template"`, so a template-family prompt in reference mode would
  be treated as having no original file (wrong removal messaging, wrong drift story). A
  prompt has an original file, copy/reference modes, and an editable stored body: that IS
  the interpreted shape, mechanically.
- **`workdir` pinned to `"invoke"`** for prompt entries, both modes — agents work on the
  repo the user is standing in, never the store or the prompt file's directory.
- **Runner selection is remembered**: no global ranking, no global default. The user picks
  at add time; skit remembers the last-picked runner in state and prefills it for the next
  prompt added.
- Bare `.md` is **asked about** at add time, never claimed outright.

Revision notes (v2 → v2.1, from the independent adversarial review — each was verified
against the code before being adopted):

- **Compound-suffix inference is a real `registry.infer_kind` change**, not free:
  `Path.suffix` on `review.prompt.md` is `".md"`, so a `".prompt.md"` key in the
  extension map is unreachable. Specified below.
- **The placeholder form path must be opened to the prompt kind explicitly**: today
  `flows._declared_plan` produces placeholder fields only for `family == "template"`
  (`flows.py:206`), and its non-template branch drops `delivery="placeholder"` rows.
  Without a change, a prompt entry gets an **empty form** and the launch gets no values.
  This — not the runner kwarg — is the load-bearing integration point. Specified below
  (`LangSpec.placeholder_params`).
- **The two-stage render gets a new no-quote renderer**: `TemplateLaunch._render`
  shell-quotes every substituted value (`quote_for_shell`, `launch.py:216`) — correct for
  `ShellLaunch`, corrupting for prompt text. Only the token regex is shared.
- **The runner-threading surface is stated honestly**: the optional kwarg touches all five
  existing strategies (build + describe), `flows.execute`, `launcher.run_entry` /
  `_payload` / `describe_command`, and `flows.transparency_lines`; the interactive ask
  lives in the CLI/TUI, never in headless launcher code; `preflight` checks the pin only.
- **Reference-mode workdir**: `store.add_script` force-sets `workdir="origin"` for
  reference mode (`store.py:319`), which would land the agent in the prompt file's
  directory. An explicit-override amendment is specified.
- Smaller: internal naming de-collided from the JS "runner" (`PromptRunner`,
  `[[prompt.runners]]`, `load_prompt_runners`); a seed marker so an emptied runner list
  stays empty; `runner add` parses flag-bearing argv; body text gets **no** `{cwd}`/
  `{today}` expansion (value tokens work inside field values, as everywhere); extra argv
  appends to the rendered runner argv (mirroring `TemplateLaunch`); P1 ships a stub
  launch strategy.

Round-2 verification additions (v2.1 → v2.2):

- **The trait migration covers all five `family == "template"` decision sites**, not just
  `flows._declared_plan` — the same test recurs in `skit params`' human view, the
  `--deliver` allowed set, and two TUI settings-editor spots (enumerated in the flows
  amendment below). Leaving any behind splits the read/run experience.
- `show --json` wording corrected (`fields`, not `parameters`); `PromptRunner` is
  annotated under `TYPE_CHECKING` in the launch layer; the P1 stub also implements
  `describe`/`target` so `--dry-run`/peek don't crash.

## Mission

skit grows from "a launcher for scripts" to "a launcher for scripts **and prompts**". In
2026 a prompt is a reusable, parameterized artifact just like a script — worth storing,
editing, and firing at a coding agent (Claude Code, Codex, opencode, or whatever comes
next) with the same form/preset/`--set`/last-values ergonomics every other kind already
gets.

The feature composes two shapes skit already has:

- a **prompt** is a stored text *document* with `{placeholder}` holes → the
  document-payload machinery (stored copy, edit, peek, modes) is reused as-is; the
  parameter-form machinery is reused through **one bounded, specified change**
  (`placeholder_params`, below);
- a **runner** is a named *argv template* with one reserved `{prompt}` slot
  (`["claude", "{prompt}"]`) → reuses the `TemplateLaunch` token grammar for the fill,
  and `ArgvLaunch` for the spawn.

Running a prompt is a **two-stage render**: fill the prompt's own placeholders from the
form/`--set` values → substitute the rendered text into the chosen runner's `{prompt}`
token → exec, no shell in between. Two bounded integration points (the placeholder form
path and the runner threading), one new kind, one config list, one launch strategy.

Decisions already made (maintainer-approved on the issue and in review; not up for
re-litigation):

- **skit hard-codes no specific agent.** Runners are user-editable argv templates in
  config; skit ships five seed presets (claude / codex / opencode / amp / antigravity) as
  data, materialized into the user's config on first need. "Other weird stuff" (the
  issue's words) is supported the moment the user adds a runner — no code change.
- **Naming: "runner", not "agent".** `skit agent install` already means "install skit's
  own Agent Skill into an AI tool" — a different verb. The prompt-execution target is a
  **runner**, managed under a separate `skit runner …` command tree. In *code and
  config*, the name is scoped to avoid the existing JS-runtime "runner" vocabulary
  (`RunnerLaunch`, `js.runner`): the model is `PromptRunner`, the config section is
  `[[prompt.runners]]`, the loaders are `load_prompt_runners`/`save_prompt_runners`.
- **The prompt is a document, not a one-liner.** It lives as an editable payload file
  (`prompt.md`), so `skit edit`, `skit show`, multi-line authoring, and copy-into-library
  come for free. This is the whole increment over the existing `command` kind.
- **v1 is interactive.** The seeds open the agent's own session with the rendered prompt
  as the opening message — the issue's "quick command to launch the task". Non-interactive
  print/exec runners are deferred (they are just *more argv templates* when they come).
- **No secret special-casing.** See "Delivery & limits".
- Long-term-optimal choices; reuse existing machinery over new subsystems.

Hard gates (unchanged, all blocking): ty strictest, 100% coverage floor, mutmut zero
survivors, i18n 100% (zh_CN/zh_TW), agent-skill sync test, golden-corpus byte fidelity.

## Core architecture

### The `prompt` kind (a 13th registry row)

A comment-described text kind whose payload is the prompt body. It reuses the
`_interpreted` builder shape but launches through a new `PromptLaunch`; the "interpreter"
slot is unused — the runner is a separate axis (see the seam below).

```python
def _prompt_spec() -> LangSpec:
    from .prompt import analyzer   # pure stdlib regex placeholder scan — no grammar, no guard
    return LangSpec(
        kind="prompt",
        family="interpreted",       # has an original file, copy/reference modes, editable body
        glyph="✎",                  # (final glyph TBD)
        launch=launch.PromptLaunch(),
        extensions=(".prompt.md", ".prompt"),   # compound suffix — see inference amendment
        stored_name="prompt.md",
        params_io=None,             # v1: declared params live in meta.toml [[parameters]]
        analyzer=Analyzer(analyze=analyzer.analyze, reconcile=analyzer.reconcile),
        supports_modes=True,
        takes_argv=False,           # reuse-last-args stays off; extra argv still appends (below)
        placeholder_params=True,    # opens the placeholder form path (see flows amendment)
    )
```

**Inference amendment (`registry.infer_kind`).** `Path.suffix` yields only the last
component (`review.prompt.md` → `".md"`), so compound registered extensions are
unreachable through the current `_extension_map().get(path.suffix.lower())`. `infer_kind`
gains a compound-aware match: test the lowercased *filename* with `endswith` against all
registered extensions, longest first, before the existing single-suffix behavior. `.prompt`
already works via plain `suffix`; `.prompt.md` requires this amendment. Covered by a unit
test on both spellings plus a non-prompt `.md`.

**The placeholder form path (`flows._declared_plan` amendment).** Today the branch that
turns placeholders into form fields is gated on `lang.family == "template"`
(`flows.py:206-212`), and the non-template declared branch admits only
`delivery in ("flag", "env")` (`flows.py:213-228`) — so an interpreted-family prompt would
get a **fieldless** form and `asm.command_values` would stay empty. The gate changes from
the family test to a new spec trait: `LangSpec.placeholder_params: bool = False`, set
`True` by the `command` kind (whose behavior is unchanged — same branch, same
`params.declared_for_template` call, fields in body order via the `meta.params` cache) and
by `prompt`. Placeholder-delivered values then flow exactly as they do for command
entries: `FormField.from_decl` → `asm.command_values` → `run_entry(values=…)` → `build` →
stage-1 render. The plan's `source` tag stays `"command"` — it names the delivery family,
not the kind. This is the design's load-bearing integration point and gets its own tests
(form fields appear, in body order; `--set`/`--no-input` reach the render; mutation-tested).

The same template/non-template decision recurs at **four more sites**, and the trait
migration covers all of them in P1 — leaving any one behind splits the read/run
experience (the run form prompts for a placeholder that `skit params` doesn't list, or
the settings editor offers a flag input on a placeholder row):

- `cli.py:1588` — `skit params <name>`'s human view routes template kinds to
  `_show_command_params` (placeholders synthesized from the `meta.params` cache) and
  everything else to the declared-rows-only table;
- `cli.py:2116` — the allowed `--deliver` set: `("env", "placeholder")` for template
  kinds vs `("flag", "env")` otherwise (a prompt must accept `placeholder`, and a bare
  `--add` must not default to an inert `flag` row);
- `tui_settings.py:393` — the declared-schema editor's flag-input column
  (`show_flag = family != "template"`);
- `tui_settings.py:404` — a newly typed param's default delivery (template kinds
  auto-map body-placeholder names to `placeholder`).

All four gate on `spec.placeholder_params` exactly like `_declared_plan`. The two
remaining `family == "template"` sites (`tui_settings.py`/`tui.py` rendering
`meta.template`) are correct untouched — a prompt has no template line to render.

Param storage: **`[[parameters]]` in `meta.toml`** (the exe/command path, `params_io=None`).
Markdown has no comment syntax that survives rendering, so v1 does not invent an in-body
`[tool.skit]` convention; the placeholder *cache* (`meta.params`, the write-through list of
`{names}` found in the body) keeps the form populated and ordered, and rich per-param data
is opt-in via `skit params`. Revisit an in-body block only if users ask.

New meta fields: `runner: str = ""` (the entry's pinned runner name, serialized only when
non-empty, exactly like `interpreter`).

**Workdir amendment (`store.add_script`).** The doc pins `workdir="invoke"` for both
modes, but `add_script` force-sets `"origin"` for reference mode (`store.py:319-320`),
which would drop the agent into the prompt file's directory. Amendment: an *explicit*
`workdir` argument wins in both modes (`if workdir is not None: … elif mode ==
"reference": "origin" else: "invoke"`); the prompt add flow always passes
`workdir="invoke"`. No behavior change for any existing caller (none passes `workdir` with
reference mode today — asserted by a test). The user can still change it later in settings
like any entry.

Add-time inference: `.prompt.md` / `.prompt` map to `prompt` (with the amendment above). A
bare `.md` stays outside the table — in the interactive add flow (CLI prompt or TUI add
panel) skit *offers* `prompt` as the likely kind and asks; under `--no-input`/pipe an
explicit `--prompt` (mirroring `--exe`/`--cmd`) is required, never a guess. (`skit params`
already has an unrelated `--prompt` option meaning "form-label text"; different command,
acceptable overlap, watched in review.) This dovetails with
[#10](https://github.com/t41372/skit/issues/10) (make adding weird stuff intuitive): the
ask-what-kind affordance built here is the same one #10 needs, and should land as one
add-flow change, not two.

### Placeholder detection (the analyzer)

The prompt body is scanned for `{name}` tokens using the **same token pattern**
`TemplateLaunch` uses (`_TEMPLATE_TOKEN_RE`: `{name}`, with `{{`/`}}` escapes) — the
pattern is shared; the render body is not (see the seam below). Each distinct `{name}`
becomes a declared-param candidate offered in the existing tick-to-manage add panel —
prompts often contain code snippets, so false positives (`{x}` inside a JSON example) are
expected; unticked candidates are left verbatim at render time, and `{{` escapes a literal
brace. No tree-sitter, no import guard: a pure stdlib regex scan living in
`langs/prompt/`, never degrading to `None`.

Reserved name: **`prompt` is not a legal placeholder in a prompt body.** Not a mechanical
collision (body holes are stage 1, the runner slot is stage 2) but an ergonomic guard: a
form field named "prompt" on a prompt entry, and a future per-runner `{prompt}` hole,
would be endless confusion. Rejected at add/params time with a clear message.

Value tokens: `{cwd}`/`{today}` expand **inside field values** (the existing
`flows._final_value` pipeline), same as every kind. The body itself gets **no** token
expansion — a literal `{today}` in a prompt body is just another placeholder candidate,
and an unticked one passes through to the agent verbatim.

Reconcile mirrors the command path: if the user removes a `{hole}` from the body, the
matching declared param drifts and `skit params`/doctor report it.

### The runner registry (config, `[[prompt.runners]]`)

Runners live in `config.toml`, following the `[js] runner` precedent for a typed config
section — scoped under `prompt.` to keep the two "runner" vocabularies apart. An array of
tables, each a name + **argv token list**:

```toml
[prompt]
runners_seeded = true   # written once at seed time; an emptied list stays empty

[[prompt.runners]]
name = "claude"
argv = ["claude", "{prompt}"]

[[prompt.runners]]
name = "codex"
argv = ["codex", "{prompt}"]

[[prompt.runners]]
name = "opencode"
argv = ["opencode", "{prompt}"]

[[prompt.runners]]
name = "amp"
argv = ["amp", "{prompt}"]

[[prompt.runners]]
name = "antigravity"
argv = ["antigravity", "{prompt}"]
```

(Seed argv shapes above are illustrative; each is pinned during implementation against the
CLI's actual interactive-with-initial-prompt invocation, and any that has none is seeded
with its closest equivalent and a comment in the seeded config.)

- **Argv, not a shell string.** Each element is one argv token; substitution happens
  *within* a token, so a multi-line prompt is one `execve` argument — no shell, no
  quoting, no cmd.exe. A custom runner that genuinely needs shell syntax (pipes) is out of
  scope for v1 (wrap it in a script and point the runner at that).
- **Seeded, not hard-coded.** When `runners_seeded` is absent, skit materializes the five
  presets *into the user's config* (and writes the marker) so they are immediately visible
  and editable — never a hidden built-in list. The marker distinguishes "never seeded"
  from "deliberately emptied": removing all five must not resurrect them.
- **`{prompt}` is the one reserved slot.** Validation (at `skit runner add` and on load):
  `argv` is a non-empty list of strings; `{prompt}` occurs exactly once across all tokens
  (it may be embedded, e.g. `"--message={prompt}"`, but not in `argv[0]`); no other
  `{holes}`; `{{`/`}}` escape literal braces.
- Config API mirrors the existing per-section loaders: `config.load_prompt_runners() ->
  list[PromptRunner]`, `config.save_prompt_runners(...)`; a frozen `PromptRunner(name:
  str, argv: tuple[str, ...])`. Corruption-tolerant like every other loader: a malformed
  row is skipped (doctor reports it); a missing section with no marker seeds the presets.
- **Last-picked state**: the most recently *picked* runner name (add-time picker or
  `--runner` on a run) is remembered under `state_dir()` (beside the existing `values/`
  last-used store — state, not config). It has exactly one job: prefill the picker for
  the *next* prompt added. It never silently decides a run.

### PromptLaunch, the render, and the runner-threading seam

**The render (stage 1 and stage 2) is a new, no-quote function** in `langs/prompt/`,
sharing only `_TEMPLATE_TOKEN_RE` with `TemplateLaunch`. It must NOT reuse
`TemplateLaunch._render`: that body wraps every substituted value in `quote_for_shell`
(`launch.py:216`) — correct for a `ShellLaunch` command string, corrupting for prompt text
(the agent would read literal `'…'` quotes) and fatal to risk-test #1's byte-identity
assertion. The new renderer substitutes raw values in one pass, honors `{{`/`}}`, and
raises `LaunchError` for a managed placeholder with no value (checked against the entry's
managed names, mirroring — not calling — the `_render` missing-check).

`PromptLaunch.build`:

1. Read the prompt body (the stored `prompt.md`, or the referenced original).
2. Stage 1: substitute the body's managed `{placeholder}` holes from `values`
   (`asm.command_values`, delivered per the flows amendment above). Unmanaged braces pass
   through verbatim.
3. Stage 2: substitute the rendered text into the runner argv's `{prompt}` token — plain
   string substitution inside one token, **no quoting of any kind**.
4. Append `extra` argv (anything after `--` on `skit run`) to the rendered runner argv —
   mirroring `TemplateLaunch`'s append (`launch.py:219-226`), so per-run agent flags
   (`skit run review -- --model opus`) pass through. `takes_argv=False` still keeps the
   reuse-last-args affordance off, exactly like command entries.
5. Return `ArgvLaunch(rendered_argv)`. The child inherits the terminal exactly as every
   interactive script already does; skit's after-run behavior applies unchanged.

**Threading the runner (the honest surface).** A prompt's runner can be overridden at run
time, so the selection must reach the strategy — and `values` must not smuggle it (it
would collide with a real param key). The protocol change:

- `LaunchStrategy.build` and `.describe` gain `*, runner: PromptRunner | None = None`.
  Under ty's strict protocol matching this means **all five existing strategies gain the
  keyword on both methods** (ten accept-and-ignore signatures) — a mechanical but real
  edit, stated here so nobody discovers it mid-implementation. `PromptRunner` lives in
  `config`; `base.py` (a LEAF module) and `launch.py` annotate it under `TYPE_CHECKING`
  only — `config` imports neither, so there is no runtime cycle, and the annotation must
  not be written as a runtime import.
- The runner threads through the call chain that exists today: CLI/TUI →
  `flows.execute(runner=…)` → `launcher.run_entry(runner=…)` → `_payload` →
  `build(runner=…)`; and on the transparency side `flows.transparency_lines(runner=…)` →
  `launcher.describe_command(runner=…)` → `describe(runner=…)`, so `--dry-run` prints the
  real resolved argv.
- **Resolution happens in the CLI/TUI layer, never in launcher/flows**: the interactive
  ask (below) is UI, and headless code must stay headless.

Runner resolution order (deterministic; the non-interactive contract):

1. explicit `--runner NAME` (CLI) / the run form's picker (TUI);
2. else the entry's pinned runner (`meta.runner`, set at add time, changeable in script
   settings or `skit params <prompt> --runner NAME`);
3. else, **interactive only**, ask (picker prefilled from last-picked state);
4. else — `--no-input`/pipe/CI with nothing resolvable — **clean error, exit 126, never
   guess.** An unknown `--runner NAME` (or a pinned name whose config row was removed) is
   also 126, listing the known names.

There is deliberately **no global default runner and no detection ranking**: unlike JS
runtimes, the runner choice changes the *result*, so skit never ranks agents. The
last-picked state only prefills pickers; it never resolves a non-interactive run.

`target()` returns the prompt path (so peek/edit resolve it). `preflight(entry)` keeps its
runner-less signature and **checks the pin only**: registry lookup of `meta.runner` (if
set) and `shutil.which(argv[0])` — a missing binary is the usual `NotExecutableError` →
exit 126 ("claude isn't installed…", naming `skit runner` as the fix) — plus the entry's
own `meta.needs`. A `--runner`/picker override is validated at build time (same errors,
same codes); preflight simply cannot see it, and that scope is deliberate rather than
accidental.

### Delivery & limits (the honest stance on secrets)

The rendered prompt travels as **one argv element**. Two consequences, stated plainly in
the docs rather than papered over:

- **Prompts are not a secrets channel, and skit does not pretend otherwise.** Whatever
  enters a prompt is written verbatim into the receiving agent's own session logs on
  disk — outside skit's control, unencrypted, often synced. No delivery mechanism on
  skit's side (argv, stdin, temp file) changes that, so skit implements none of them for
  secrecy. A prompt placeholder *may* still be marked secret — the generic promise (skit
  never persists the value in last-used/presets, masks it in transparency output) holds
  like any kind — but the user docs for the prompt kind state explicitly that the value's
  confidentiality ends at the runner boundary.
- **Length.** argv has platform limits (Windows ≈32k chars; POSIX ARG_MAX is generous).
  Rendering checks the assembled command line against a conservative platform bound and
  refuses with a clean `LaunchError` (exit 125) naming the size — an honest refusal today;
  stdin/file delivery can arrive later *per-runner* when a seed CLI actually supports it
  (deferred, see below).

## CLI surface

```
skit add notes/review.prompt.md            # add an existing prompt file (kind inferred)
skit add notes/review.md                   # bare .md: interactive ask; --no-input requires --prompt
skit add --prompt -n review                # author a new prompt: $EDITOR on a blank prompt.md
skit runner list                           # configured runners (seeds materialized on first need)
skit runner add mycli mycli run {prompt}   # variadic: the USER'S shell does the word splitting
skit runner remove mycli
skit run review --set target=src/foo.py            # runs with the entry's pinned runner
skit run review --runner codex --set target=...    # per-run override
skit run review --dry-run                          # print the exact resolved argv
skit params review --runner claude          # re-pin the entry's runner
skit show review --json                     # schema incl. placeholders + pinned runner
skit edit review                            # edit the prompt body ($EDITOR on prompt.md)
```

`skit runner add` takes the argv **variadically** — each shell word becomes one token, so
skit never parses a command string itself (no shlex ambiguity, no Windows-backslash
mangling). Because real runner argv contains flags (`claude --model sonnet {prompt}`),
the subcommand sets Click's `ignore_unknown_options` + `allow_extra_args` (the existing
`run` passthrough precedent) so flag-looking tokens land in the argv list instead of
erroring; `--` works as an explicit guard and is what SKILL.md teaches (the agent-skill
command-tree test rejects unknown-flag tokens outside a `--` tail, so the skill's examples
must use it).

`skit add --prompt` (no path) is the authoring entry point — it seeds a blank `prompt.md`
in the store and opens `$EDITOR`. Under `--no-input`/pipe there is no editor: it reads the
body from stdin (`skit add --prompt -n review < body.md`) or errors. Add-time runner
pinning: the interactive flows show the runner picker (prefilled from last-picked);
non-interactive adds pin only when `--runner` is given, else the entry stores no pin and
`run` resolves per the order above.

`run` passes the runner process's exit code through untouched (docker convention); skit
errors stay 125/126/127 — a missing/unknown/unresolvable runner is a skit error (126),
distinct from the agent itself failing.

## TUI surface

A `prompt` entry runs through the **same run form** as every other kind, with one
addition: a **runner picker** row at the top (chip-consistent, mouse- and
keyboard-operable per principle #2), prefilled with the entry's pin (or last-picked when
unpinned). Below it, the placeholders render as ordinary form fields (choices/bool/secret
all work — via the `placeholder_params` plan path). The add panel gains the same picker,
plus the bare-`.md` kind ask. Runner management (add/remove) lives in the settings screen
alongside the other config. Every new string is a static `gettext()` literal; the picker
wraps/degrades under the existing `tui_layout` size tiers — no per-screen width math.

## Non-interactive & JSON contracts (additive only)

- `show --json` gains, for prompt entries: `"runner": "claude"|null` (the pin) and
  `"runners_available": ["claude", …]`; placeholder params appear under the existing
  `fields` array (the payload has no `parameters` key), with `"param_source": "command"`
  — the delivery family, not the kind. Additive — no existing key changes.
- `runner list --json` → `[{"name": …, "argv": […]}]`.
- These `--json` keys and `SKILL.md` are the English-only machine contract (principle #1
  exemption); SKILL.md gains a "prompts & runners" section in the same commit, enforced by
  the agent-skill sync + command-tree resolution test.

## i18n

Every user-visible string (add/authoring prompts, the bare-`.md` ask, runner picker label,
settings copy, error messages) ships translated to zh_CN/zh_TW at 100% in the same change —
the coverage gate is blocking. Runner *names* and *argv* are data, not UI copy — never
translated.

## Correctness & security risks (each pinned by a test)

1. **No shell, no quoting, by construction** → a corpus payload `'; rm -rf ~; $(touch
   pwned)` in a placeholder value must appear **byte-identical** inside the child's argv
   (asserted via a recorder runner — a seeded test runner whose argv the test reads back)
   and cause no side effect. This is exactly why the render must not pass through
   `quote_for_shell`; the test keeps both properties pinned.
2. **The placeholder plan path** → a prompt entry's form shows its managed placeholders
   as fields, in body order; `--set` and `--no-input` deliver through `command_values` to
   the render; the `command` kind's behavior is byte-for-byte unaffected by the
   `placeholder_params` refactor at **all five** migrated sites (regression-pinned); and
   the read/run surfaces agree — `skit params <prompt>` lists exactly what the run form
   asks, `--deliver x=placeholder` is accepted, the TUI editor shows no flag input on
   placeholder rows. Mutation-tested.
3. **`{prompt}` slot validation** → a body placeholder named `prompt`, or a runner argv
   failing validation (zero/two `{prompt}`s, `{prompt}` in argv[0], stray `{holes}`), is
   rejected at add/params/runner-add time (corpus + unit).
4. **Runner unresolvable under `--no-input`** → exit 126, no guess; unknown `--runner`
   and a pinned-but-removed runner both list the known names. Pinned by a pipe/`--no-input`
   test.
5. **Multi-line / CJK / emoji prompt bodies** → byte-exact golden corpus under
   `tests/corpus/prompt/` (CRLF, no-trailing-newline, CJK, emoji), excluded from the
   pre-commit fixers like every other corpus; a multi-line body must arrive in the child's
   argv byte-identical on POSIX and Windows (the no-shell decision is what makes this
   testable at all).
6. **Over-long rendered argv** → clean LaunchError (125) at the platform bound, never a
   raw OS error; boundary test.
7. **Config `[[prompt.runners]]` corruption** → malformed rows are skipped and reported
   by doctor; a missing section with no `runners_seeded` marker seeds the five presets; a
   marker with zero rows stays empty (deliberate-emptying test); neither crashes list/run.
8. **Reference-mode prompt** → a referenced (not copied) `prompt.md` is read at run time;
   body edits to the original are picked up, drift against declared params reported
   (mirrors A7); `family="interpreted"` keeps `has_original_file` true so removal
   messaging stays honest; and the add path pins `workdir="invoke"` in reference mode too
   (the `add_script` explicit-override amendment, with a no-behavior-change test for
   existing callers).
9. **Compound-suffix inference** → `review.prompt.md`, `x.prompt`, and a bare `notes.md`
   each infer correctly (prompt / prompt / ask-or-refuse); existing single-suffix kinds
   unaffected.
10. **Last-picked state** → corrupt/absent state file degrades to "no prefill", never an
    error; the state value is a picker default only — a `--no-input` run must be provably
    unaffected by it (mutation-tested).

## Phases

- **P1 — the prompt kind, static (no runner yet).** `_prompt_spec` in the registry with a
  **stub launch strategy**: `build`/`preflight` raise `NotExecutableError` ("no runner
  configured yet" → the honest 126) until P2 replaces it, while `describe`/`target`
  answer benignly (a "no runner" line / the prompt path) so `--dry-run` and peek don't
  crash; the `registry.infer_kind` compound-suffix amendment; `langs/prompt/` regex
  placeholder analyzer + reconcile; the `LangSpec.placeholder_params` trait across **all
  five** `family == "template"` decision sites (`flows._declared_plan`, the two `cli.py`
  params sites, the two `tui_settings.py` sites — with the command-kind regression pin
  on each); `[[parameters]]` storage with the
  placeholder write-through cache; `prompt` reserved-name rejection; `skit add --prompt`
  authoring (editor + stdin paths, non-interactive contract); the bare-`.md` add-time ask
  (coordinated with #10's add-flow work); the `add_script` workdir override amendment +
  `workdir="invoke"` pinning; edit/peek/show; golden corpus. Full gate + review loop.
- **P2 — runners & execution.** `PromptRunner` model + `[[prompt.runners]]` loaders +
  seed materialization with the `runners_seeded` marker + `skit runner add/list/remove`
  (variadic argv, `ignore_unknown_options`); the `build`/`describe` `runner=` protocol
  extension across all five strategies + the threading through
  `execute`/`run_entry`/`_payload`/`describe_command`/`transparency_lines`; the no-quote
  renderer + `PromptLaunch` (two-stage render, extra-argv append, `ArgvLaunch`);
  resolution order + preflight-pin scope + last-picked state; the length bound;
  `--runner` CLI; TUI runner picker + settings management; JSON additions; SKILL.md sync;
  i18n to 100%.
- **P3 — docs, demo, positioning.** README×3 gain a "prompts" section and a runner example
  (ties into the AI-agent positioning: agents can *save prompts back* via the Skill, the
  same two-way story scripts already have); demo assets regenerated in the **same** Docker
  pass as any pending refresh; AGENTS.md amendment noting the prompt kind + the
  `build(runner=…)` protocol extension.

Each phase ends: full gate (`ruff format --check && ruff check && ty check && pytest --cov
&& mutmut run`) → commit → review loop (review → verify → fix → re-review to zero) → next
phase. P1 and P2 land in one PR (a kind that exists but cannot run should not sit alone on
main); P3 may trail.

## Deferred (explicitly out of v1)

- **Print/exec (non-interactive) runners** — `claude -p {prompt}`, `codex exec {prompt}`:
  just more argv templates, plus (maybe) a per-runner interactive/print pairing so one
  `--print` flag flips a run. Design when needed.
- **Per-runner stdin/file prompt delivery** — for CLIs that accept a prompt on stdin or
  from a file; also the escape hatch if the argv length bound ever bites in practice.
- **Import existing slash commands** from `~/.claude/commands/`, Codex prompts, opencode
  configs (`skit add --import-claude` etc.). Potentially the killer `skit add` experience,
  but a separate surface with per-tool format knowledge.
- **Per-runner parameters** (argv templates with holes beyond `{prompt}`, e.g.
  `--model {model}`). v1 runners consume only `{prompt}`.
- **An in-body `[tool.skit]` block** for prompt params (v1 uses `meta.toml
  [[parameters]]`).
- **Shell-syntax runners** (pipes/redirection in a runner definition) — wrap in a script
  instead.
