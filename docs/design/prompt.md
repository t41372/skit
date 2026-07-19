# Prompts as first-class entries — final design

Status: **approved** (v3, 2026-07-17). Resolves
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
- **v1 normally launches the agent's interactive session** (the prompt as opening
  message); Amp is the deliberate exception because its CLI has no
  interactive-with-initial-prompt form, so `amp -x` runs once. The v1 proposal had
  print/exec seeds (`claude -p`, `codex exec`) and deferred interactive as "session
  management". That framing was wrong: skit already hands the terminal to every
  interactive TUI script it launches — an interactive agent needs no more "management"
  than `htop` does.
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
- **Historical v2.1 decision (superseded by v3's independent grammar):** the two-stage
  render gets a new no-quote renderer because `TemplateLaunch._render` shell-quotes every
  substituted value (`quote_for_shell`, `launch.py:216`) — correct for `ShellLaunch`,
  corrupting for prompt text. v2.1 proposed sharing its token regex; v3 explicitly does
  not (see `langs.prompt.analyzer.TOKEN_RE` below).
- **The runner/preparation surface is stated honestly**: the optional kwarg touches all
  five existing strategies (`build` + `describe` + `preflight`). Normal prompt delivery
  additionally crosses `flows.execute` → `launcher.prepare_entry` →
  `PromptLaunch.build_snapshot` and hands the resulting `PreparedLaunch` to
  `launcher.run_entry`; dry-run crosses `flows.validate_prompt_argv` →
  `PromptLaunch.validate_argv`. The interactive ask lives only in CLI/TUI, while the
  launch strategy deterministically resolves a durable entry pin for headless/form-free
  callers; `preflight` checks the supplied override when present and otherwise the pin.
- **Reference-mode workdir**: `store.add_script` force-sets `workdir="origin"` for
  reference mode (`store.py:319`), which would land the agent in the prompt file's
  directory. An explicit-override amendment is specified.
- Smaller: internal naming de-collided from the JS "runner" (`PromptRunner`,
  `[[prompt.runners]]`, `load_prompt_runners`); a seed marker so an emptied runner list
  stays empty; `runner add` parses flag-bearing argv; body text gets **no** `{cwd}`/
  `{today}` expansion (value tokens work inside field values, as everywhere); extra argv
  stays on the option side of a runner's `--` delimiter (or appends when there is none);
  P1 ships a stub
  launch strategy.

Revision notes (v2.2 → v3, maintainer-decided after using the shipped v2.2):

- **Placeholders are DOUBLE-brace: `{{name}}`** (the prompt-template world's convention —
  Anthropic Console, Jinja2, Handlebars — chosen over the command-template `{name}`
  because prompts quote code: JSON, `${VAR}`, f-strings are full of single-brace
  identifiers that are not parameters). Runner argv slots follow: `{{prompt}}`. The
  command kind's shipped `{name}` grammar is untouched — two surfaces, two grammars,
  documented as such.
- **No escape sequences on the prompt surface.** Anything that isn't a MANAGED
  `{{name}}` — unmanaged holes, single braces, triple-stache — travels byte-identical.
  Nothing in a body ever needs escaping; residual false positives cost a candidate-list
  entry, never a text mutation. (The `{{`/`}}` escape rule of v2.2 is gone with the
  grammar that needed it.)
- **A per-prompt insertion master switch** (`meta.interpolate`, default on): off = no
  scanning, no fields, no drift, verbatim delivery. `skit add --no-interpolate`,
  `skit params NAME --interpolate/--no-interpolate`, an "off" answer in the interactive
  add, and a single checkbox in TUI Entry settings. The managed list survives an
  off/on round trip.
- **Flood guards** for prompts never written with insertion in mind: above
  AUTO_MANAGE_LIMIT (30) detections the auto path manages NOTHING (an explicit
  interactive/managed selection is always honored), and every candidate list surface
  (CLI add, params view, settings checkboxes) previews at most LIST_PREVIEW_LIMIT (20)
  names plus a "+N more" tail.

Cross-surface verification requirements (v2.1 → v2.2):

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

- a **prompt** is a stored text *document* with `{{placeholder}}` holes → the
  document-payload machinery (stored copy, edit, peek, modes) is reused as-is; the
  parameter-form machinery is reused through **one bounded, specified change**
  (`placeholder_params`, below);
- a **runner** is a named *argv template* with one reserved `{{prompt}}` slot
  (`["claude", "--", "{{prompt}}"]`) → uses the prompt surface's independent
  `langs.prompt.analyzer.TOKEN_RE` grammar for validation/fill, and `ArgvLaunch` for the
  spawn.

Running a prompt is a **two-stage render**: fill the prompt's own placeholders from the
form/`--set` values → substitute the rendered text into the chosen runner's `{{prompt}}`
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
- **v1 is interactive, with one explicit seed exception.** Claude, Codex, OpenCode and
  Antigravity open the agent's own session with the rendered prompt as the opening
  message — the issue's "quick command to launch the task". Amp has no such CLI form;
  its `amp -x` seed is deliberately one-shot, and both `runner list` and execution say so.
  Other non-interactive print/exec runners remain ordinary user-defined argv templates.
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
    return LangSpec(
        kind="prompt",
        family="interpreted",       # has an original file, copy/reference modes, editable body
        glyph="✎",                  # (final glyph TBD)
        launch=launch.PromptLaunch(),
        extensions=(".prompt.md", ".prompt"),   # compound suffix — see inference amendment
        stored_name="prompt.md",
        params_io=None,             # v1: declared params live in meta.toml [[parameters]]
        # NO analyzer capability, deliberately (command-kind parity — implementation
        # correction to the earlier draft, which registered one): every spec.analyzer
        # consumer is shaped for params_io kinds, and registering one would flip
        # `run --raw` into the skip-the-form branch, which then fails build with
        # "Missing parameter values" for every managed hole. Detection lives in
        # langs/prompt/analyzer.py as plain functions consumed directly by the
        # add/params/plan surfaces.
        supports_modes=True,
        takes_argv=False,           # reuse-last-args stays off; extra argv follows rule below
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
entries: `FormField.from_decl` → `asm.command_values` → `flows.execute` →
`launcher.prepare_entry` → `PromptLaunch.build_snapshot` → stage-1 render. The plan's
`source` tag stays `"command"` — it names the delivery family, not the kind. This is the
design's load-bearing integration point and gets its own tests (form fields appear, in
body order; `--set`/`--no-input` reach the render; mutation-tested).

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
`{{names}}` found in the body) keeps the form populated and ordered, and rich per-param data
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

The prompt body is scanned for `{{name}}` tokens (double braces, Unicode identifier body, not
brace-adjacent — a Handlebars `{{{raw}}}` is someone else's syntax and stays quiet). The
grammar is deliberately INDEPENDENT of `TemplateLaunch`'s single-brace pattern (v3): code
snippets full of `{x}`/`${VAR}`/JSON braces are never candidates. Each distinct `{{name}}`
becomes a declared-param candidate offered in the existing tick-to-manage add panel;
unticked candidates are left verbatim at render time, and there are NO escape sequences —
what isn't managed is never touched. No tree-sitter, no import guard: a pure stdlib regex
scan living in `langs/prompt/`, never degrading to `None`.

Reserved name: **`prompt` is never a placeholder in a prompt body.** Not a mechanical
collision (body holes are stage 1, the runner slot is stage 2) but an ergonomic guard: a
form field named "prompt" on a prompt entry, and a future per-runner `{{prompt}}` hole,
would be endless confusion. The analyzer excludes it from detection outright — a literal
`{{prompt}}` in a body (a code sample, say) passes through to the agent verbatim rather
than erroring, which is the right call for text that quotes runner templates. It can
therefore never be managed, offered, or --add'ed (the fresh-scan placeholder truth never
contains it).

Value tokens: `{cwd}`/`{today}` expand **inside field values** (the existing
`flows._final_value` pipeline), same as every kind. The body itself gets **no** token
expansion — a literal `{today}` in a prompt body is literal text, not a double-brace
placeholder candidate, and passes through to the agent verbatim.

Reconcile mirrors the command path: if the user removes a `{{hole}}` from the body, the
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
argv = ["claude", "--", "{{prompt}}"]

[[prompt.runners]]
name = "codex"
argv = ["codex", "--", "{{prompt}}"]

[[prompt.runners]]
name = "opencode"
argv = ["opencode", "--prompt={{prompt}}"]

[[prompt.runners]]
name = "amp"
argv = ["amp", "-x", "{{prompt}}"]

[[prompt.runners]]
name = "antigravity"
argv = ["agy", "--prompt-interactive", "{{prompt}}"]
```

(These are the shipped seeds. Amp's CLI has no interactive-with-initial-prompt invocation,
so `amp -x` is the deliberate one-shot exception; the human `runner list` view and the
run path make that behavior explicit. OpenCode binds the value in the `--prompt=…` token
because its yargs parser would treat a separate option-looking value such as `--help` or
`--version` as a CLI option instead of prompt text. Config rows remain plain editable data.)

- **Argv, not a shell string.** Each element is one argv token; substitution happens
  *within* a token, so a multi-line prompt is one `execve` argument — no shell, no
  quoting, no cmd.exe. A custom runner that genuinely needs shell syntax (pipes) is out of
  scope for v1 (wrap it in a script and point the runner at that).
- **Seeded, not hard-coded.** Loading is read-only (before seeding, the effective list
  IS the five presets — a `skit run`/`show` never writes config as a side effect); the
  presets materialize *into the user's config* (with the `runners_seeded` marker) on the
  first `skit runner` management action — the moment the user goes looking for the data,
  it is visible and editable, never a hidden built-in list. The marker distinguishes
  "never seeded" from "deliberately emptied": removing all five must not resurrect them.
- **`{{prompt}}` is the one reserved slot.** Validation (at `skit runner add` and on load):
  `argv` is a non-empty list of strings; `{{prompt}}` occurs exactly once across all tokens
  (it may be embedded, e.g. `"--message={{prompt}}"`, but not in `argv[0]`); no other
  `{{holes}}`; single-brace text is literal (a tool's own `{x}` syntax passes untouched).
- Config API mirrors the existing per-section loaders: `config.load_prompt_runners() ->
  list[PromptRunner]`, `config.save_prompt_runners(...)`; a frozen `PromptRunner(name:
  str, argv: tuple[str, ...])`. Corruption-tolerant like every other loader: a malformed
  row is skipped (doctor reports it); a missing section with no marker seeds the presets.
- **Last-picked state**: the most recently *picked* runner name (add-time picker or
  `--runner` on a run) is remembered under `state_dir()` (beside the existing `values/`
  last-used store — state, not config). It has exactly one job: prefill the *next runner
  picker* (add review or an unpinned interactive run). It never silently decides a run.

### PromptLaunch, the render, and the prepared-launch seam

**The render (stage 1 and stage 2) is a new, no-quote function** in `langs/prompt/` and
uses `langs.prompt.analyzer.TOKEN_RE`, deliberately independent of `TemplateLaunch`'s
single-brace grammar. It must NOT reuse `TemplateLaunch._render`: that body wraps every
substituted value in `quote_for_shell` (`launch.py:216`) — correct for a `ShellLaunch`
command string, corrupting for prompt text (the agent would read literal `'…'` quotes)
and fatal to risk-test #1's byte-identity assertion. The new renderer substitutes raw
values in one pass and implements **no escape sequences**: unmanaged double-brace holes,
single braces, and triple-stache text pass through unchanged. It raises `LaunchError`
for a managed placeholder with no value (checked against the entry's managed names,
mirroring — not calling — the `_render` missing-check).

`PromptLaunch.build_snapshot` is the canonical preparation path for a real prompt run:

1. Resolve the supplied runner object, or the entry's durable `meta.runner` pin, to one
   configured `PromptRunner` row. No runner ranking or interactive ask occurs here.
2. Read the prompt body once (the stored `prompt.md`, or the referenced original).
3. Stage 1: substitute the body's managed `{{placeholder}}` holes from `values`
   (`asm.command_values`, delivered per the flows amendment above). Unmanaged braces pass
   through verbatim.
4. Stage 2: substitute the rendered text into the runner argv's `{{prompt}}` token — plain
   string substitution inside one token, **no quoting of any kind**.
5. Place `extra` argv (anything after `--` on `skit run`) on the option side of a
   runner template's end-of-options delimiter, when it has one; otherwise append it.
   Thus per-run agent flags (`skit run review -- --model opus`) keep working while a
   positional prompt beginning with `--help` cannot turn into an agent option.
   `takes_argv=False` still keeps the reuse-last-args affordance off.
6. Validate the configured argv, resolve its executable, replace only `argv[0]` with that
   resolved binary, and validate the real argv again. Body/render/length failures therefore
   still precede a missing-binary refusal.
7. Return the `ArgvLaunch`, a compact safe-display string made from that **same runner
   row** with an omitted-prompt sentinel, and the chosen `PromptRunner`. The child inherits
   the terminal exactly as every interactive script already does; skit's after-run behavior
   applies unchanged.

`PromptLaunch.build` delegates to `build_snapshot` and returns only its payload for the
generic `LaunchStrategy` protocol. `launcher.prepare_entry` adds the validated working
directory to the snapshot, checks `meta.needs`, and freezes all of it in an immutable
`PreparedLaunch`. For prompts, `flows.execute` must complete that preparation **before**
emitting the Amp one-shot note, the plaintext-secret warning, or transparency. It then
passes the same object to `launcher.run_entry(prepared=…)`, which spawns it without
re-reading the body, reloading a runner row, or rebuilding argv. The transparency line
uses the prepared object's compact display, and the Amp note keys off its chosen runner;
a concurrent body/config edit therefore cannot make the UI describe different delivery
semantics from the argv already prepared for spawn. If preparation fails, none of those
delivery-semantic lines are emitted.

Dry-run deliberately has a different, PATH-independent boundary: the CLI calls
`flows.validate_prompt_argv`, which calls `PromptLaunch.validate_argv`. That method
resolves one configured row, reads the body once, renders and length-checks the real argv,
and derives any secret-masked display from that same body/runner snapshot. The returned
validated display is passed directly to `flows.transparency_lines`; transparency never
calls `describe` to re-read a prompt that already passed validation. `describe` remains a
side-effect-free fallback/general descriptive surface, not the second half of the prompt
dry-run path.

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
- For a normal run, a supplied CLI/TUI choice threads through `flows.execute(runner=…)` →
  `launcher.prepare_entry(runner=…)` → `PromptLaunch.build_snapshot(runner=…)` →
  `PreparedLaunch` → `launcher.run_entry(prepared=…)`. Normal transparency uses the
  snapshot's omitted-payload display instead of dumping an entire document into
  scrollback. For dry-run, the chosen override follows `flows.validate_prompt_argv` →
  `PromptLaunch.validate_argv`, whose returned display goes straight to
  `transparency_lines`.
- **Interactive resolution happens only in CLI/TUI.** Those layers resolve an explicit
  option/picker and may resolve the pin before calling flows. Form-free and other headless
  call paths may pass `runner=None`; `PromptLaunch._resolve_runner` then performs the same
  deterministic lookup of `entry.meta.runner`, or raises exit 126. It never prompts,
  ranks, or consults last-picked state. This keeps launch code headless without making a
  durable pin unusable outside the run form.

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

`target()` returns the prompt path (so peek/edit resolve it). `PromptLaunch.preflight`
reads and validates the body, then checks the runner the launch will actually use: an
explicit picker choice when supplied, otherwise the entry's pin. Its runner check resolves
the configured row and calls `shutil.which(argv[0])`; `launcher.preflight` then checks the
entry's `meta.needs` and workdir. A missing binary is the usual `NotExecutableError` → exit
126. The TUI calls preflight after the form resolves its picker but before suspending the
terminal, so a stale or broken pin cannot block a valid per-run override; health and
form-free rerun omit the override and continue to validate the pin. Preflight is an early
readiness check only: `prepare_entry` creates the authoritative, single-use delivery
snapshot for a real run, in body/render → binary → needs → workdir order after the runner
row has resolved.

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
  confidentiality ends at the runner boundary. Add summary states both halves, and a real
  run with a non-empty secret-marked value warns immediately before delivery; dry-run does
  not warn because it sends nothing.
- **Length.** argv has platform limits, but their serialization differs. On Windows,
  `CreateProcess` caps the command line at roughly 32k UTF-16 code units, so skit measures
  Python's exact `subprocess.list2cmdline(argv)` result encoded as UTF-16LE plus its NUL
  terminator, with a conservative 60,000-byte ceiling. On POSIX, skit sums each token's
  `os.fsencode` byte length plus its terminating NUL and uses a conservative 100,000-byte
  ceiling (below Linux's 128 KiB single-argument limit). A NUL or unencodable token is
  refused too. Every refusal is a clean `LaunchError` (exit 125) naming the measured size;
  stdin/file delivery can arrive later *per-runner* when a seed CLI actually supports it
  (deferred, see below).

## CLI surface

```
skit add notes/review.prompt.md            # add an existing prompt file (kind inferred)
skit add notes/review.md                   # bare .md: interactive ask; --no-input requires --prompt
skit add --prompt -n review                # author a new prompt: $EDITOR on a blank prompt.md
skit runner list                           # configured runners (seeds materialized on first need)
skit runner add mycli -- mycli run {{prompt}} # variadic: the USER'S shell does the word splitting
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
mangling). Because real runner argv contains flags (`claude --model sonnet {{prompt}}`),
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

A path-based `skit add x.prompt.md` in an interactive terminal with `form = "tui"` hosts
the **prompt review panel** (see the TUI surface below) — the exact gate the python lane
uses; an unknown `--runner` is refused before the panel opens (never silently dropped).
The plain-mode runner asks (add-time and run-time) print a one-line custom-agent teaching
(`skit runner add NAME COMMAND…`) so the capability is discoverable without the TUI.

`run` passes the runner process's exit code through untouched (docker convention); skit
errors stay 125/126/127 — a missing/unknown/unresolvable runner is a skit error (126),
distinct from the agent itself failing.

## TUI surface

A `prompt` entry runs through the **same run form** as every other kind, with one
addition: a **runner picker** row at the top — a value-keyed `Select` dropdown (the
runner is a SECONDARY control: collapsed it costs one row instead of pushing the
parameter fields down, and its overlay scales to any number of agents), prefilled with
the entry's pin (or last-picked when unpinned). The form's priority Enter binding
("Enter runs from any field") coexists with dropdowns via a shim in `action_submit`:
Enter on a focused Select toggles its overlay, Enter inside the open overlay chooses
the highlighted option, everything else submits. The preset row and every other runner
picker (Entry settings pin, the prompt review panel) use the same dropdown; reads are
value-keyed, so a runner list that changes mid-session can never shift an index
mapping. Below it, the placeholders render as ordinary form fields (choices/bool/secret
all work — via the `placeholder_params` plan path). Every new string is a static
`gettext()` literal.

**The prompt review panel (`tui_add.PromptReviewScreen`).** Adding a prompt is never a
blind direct add: the panel is the prompt twin of the python `AddReviewScreen`, with the
same two faces (pushed from the Library's `a`, and hosted alone when an interactive
terminal runs `skit add x.prompt.md` with `form = "tui"` — the exact gate the python lane
uses, so the two kinds cannot drift). Sections: name/description/storage, the **insertion
master switch** (off folds the tick list away and stores `interpolate = false`), the
**placeholder tick list** (all pre-ticked under `AUTO_MANAGE_LIMIT`; a flooded prompt
shows only the `LIST_PREVIEW_LIMIT` preview, pre-ticks nothing, and says so), and the
**runner picker** ("ask on the run form" + the configured names, prefilled `--runner` >
last-picked > ask). Ctrl+E opens the user's original in `$EDITOR` and rescans on return,
preserving everything already set on the panel. Pipes/CI/`--no-input`/`form = "plain"`/
`TERM=dumb` keep the line-prompt path — the non-interactive contract is untouched.

**Custom agents are first-class in the TUI (`tui_runner.RunnerAddModal`).** Every
runner-picking surface — the run form's picker row, the prompt review panel, and Script
settings — carries the same "Ctrl+N New agent…" chip (footer grammar: the visible key
hint IS the click target). The modal takes a name and ONE command line; the command is
split into argv tokens with `shlex` **once, here** (quotes group words; native Windows
rules preserve path backslashes), validated by the same `validate_prompt_runner_argv`
rule the CLI enforces, and saved through `ensure_prompt_runners_seeded()` so the seeds
materialize beside it. The run form's extra-argument field uses the same platform-aware
argv text codec, so agent flags and ordinary script arguments have identical Windows
path behavior. Names are stable config keys because prompt pins persist them:
editing a runner changes its argv in place, not its name. The new runner joins the open
picker selected — no restart, no CLI detour. A prompt run with an **emptied** runner list
opens this modal instead of dead-ending on a `skit runner add` incantation; in Script
settings the value-keyed `Select` is rebuilt after the modal and save reads its selected
value directly, so there is no index mapping that can shift under a config change. The
pin save runs for every prompt — including insertion-off ones, whose declared-params
branch is skipped. Removing a runner deliberately retains prompt pins (re-adding the row restores
them), but both management surfaces count and warn about the prompts that will need a new
choice before confirming the removal; Health reports the resulting blocked pins too.

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
3. **`{{prompt}}` slot validation** → a body `{{prompt}}` is excluded from detection and
   passes through verbatim (corpus-pinned); a runner argv failing validation (zero/two
   `{{prompt}}`s, `{{prompt}}` in argv[0], stray `{{holes}}`) is rejected at runner-add time.
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
11. **Single delivery snapshot** → a normal run resolves its runner row and body once,
    prepares the executable argv before any delivery warning/transparency, and spawns that
    exact `PreparedLaunch`; a dry-run validates and displays one body/runner snapshot.
    Concurrent reference-body or runner-config edits cannot produce validate-A/display-B
    or display-A/spawn-B behavior, and a preparation failure emits no delivery-semantic
    lines.

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
  extension across all five strategies + `PromptLaunch.build_snapshot`/`validate_argv` +
  the `flows.execute`/`validate_prompt_argv` → `launcher.prepare_entry`/
  `PreparedLaunch` delivery boundaries; the no-quote renderer + `PromptLaunch`
  (two-stage render, delimiter-aware extra-argv placement, `ArgvLaunch`);
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

- **Print/exec (non-interactive) runners** — `claude -p {{prompt}}`, `codex exec {{prompt}}`:
  just more argv templates, plus (maybe) a per-runner interactive/print pairing so one
  `--print` flag flips a run. Design when needed.
- **Per-runner stdin/file prompt delivery** — for CLIs that accept a prompt on stdin or
  from a file; also the escape hatch if the argv length bound ever bites in practice.
- **Import existing slash commands** from `~/.claude/commands/`, Codex prompts, opencode
  configs (`skit add --import-claude` etc.). Potentially the killer `skit add` experience,
  but a separate surface with per-tool format knowledge.
- **Per-runner parameters** (argv templates with holes beyond `{{prompt}}`, e.g.
  `--model {{model}}`). v1 runners consume only `{{prompt}}`.
- **An in-body `[tool.skit]` block** for prompt params (v1 uses `meta.toml
  [[parameters]]`).
- **Shell-syntax runners** (pipes/redirection in a runner definition) — wrap in a script
  instead.
