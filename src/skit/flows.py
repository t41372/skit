"""The unified form layer (headless): any script becomes one pokeable form.

This is the redesign's north star made code ("users should never have to memorize
commands or flags, even for their own scripts"). Three value *sources* feed the same
form model; only the delivery differs:

| source        | detected by                      | delivery                        |
|---------------|----------------------------------|---------------------------------|
| "inject"      | [tool.skit] managed params       | AST-inject a temp copy (A5)     |
| "argparse"    | static add_argument reading      | assemble real CLI flags         |
| "command"     | command-template placeholders    | fill the template               |

The layer is presentation-free: the TUI renders a FormPlan as widgets, the CLI as
line prompts or an inline mini-form — one logic, N renderings. Rules implemented
here:

- Prefill: definition default < last-used < preset (this run's input wins in the UI).
  A definition default is the SOURCE's current value for in-file params
  (Report.current_defaults), never the block's manage-time cache.
- Values persist as TYPED TEXT (token/glob originals) — intent, not expansion.
- Tokens expand and globs re-expand at assembly time, every run.
- Explicit-pass: a filled field is always passed (reproducibility); checkboxes pass
  their flag only when they differ from the script's default; degraded fields are
  omitted when empty so the script's own default applies. A delivers-empty field
  (FormField.delivers_empty: free-text with a known default) is WYSIWYG — cleared
  means an empty string is delivered, because the default is expressed by being IN
  the field, and '' is a value a user may genuinely mean.
- Secrets: never prefilled, never saved (argstate enforces C3 structurally); an
  env_source reads the value from the environment at assembly, and a missing
  variable is a hard, named error — never a silently empty value.
"""

from __future__ import annotations

import glob as _glob
import os
import shlex
from collections.abc import Mapping
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import TYPE_CHECKING

from . import analysis, argstate, config, launcher, params, tokens
from .i18n import gettext
from .langs.base import (
    InjectError,
    InjectGapError,
    InjectRequest,
    InjectSplitError,
    InjectSyntaxError,
    InjectValueError,
    LangSpec,
)
from .langs.launch import PromptLaunch
from .langs.launch import quote_for_shell as launch_quote
from .langs.python.shim import _coerce
from .langs.registry import spec_for
from .models import Entry
from .params import ParamDecl

if TYPE_CHECKING:
    from collections.abc import Callable

    from .config import PromptRunner
    from .langs.base import CliReader

_GLOB_CHARS = ("*", "?", "[")


class FormError(Exception):
    """A form-level failure with a user-ready, localized message."""


@dataclass
class FormField:
    key: str
    label: str
    kind: str = "str"  # str | int | float | bool | choice
    source: str = "inject"  # inject | flag | placeholder
    choices: list[str] = field(default_factory=list)
    default: str = ""  # string-rendered script default
    has_default: bool = False
    help: str = ""
    required: bool = False
    secret: bool = False
    env_source: str = ""
    degraded: bool = False  # free-text fallback; omit from delivery when empty
    multiple: bool = False  # shlex-split + glob-expand each piece
    repeat: bool = False  # multiple: emit the flag before EACH piece (click/parseArgs style)
    flag: str = ""  # "--output"; "" = positional (flag source only)
    action: str = ""  # store_true | store_false (bool flags)
    env_target: str = ""  # env source only: the variable to SET ("" = the field's key)
    input_binding: bool = False  # inject source: an intercepted input()/read prompt
    empty_uses_default: bool = False  # env source: empty still activates its source fallback

    @property
    def delivers_empty(self) -> bool:
        """Whether clearing this field sends an EMPTY STRING instead of "leave the
        script alone". WYSIWYG applies exactly where it is sound: a free-text (str/path)
        field whose default is known is prefilled with that default, so "use the
        default" is expressed by the default being IN the field — clearing it is a
        deliberate act that must deliver '' (an empty string is a legitimate value, and
        collapsing it into "unset" made it inexpressible). Everywhere else empty keeps
        meaning "unset": int/float ('' is never a value there, and empty is the only
        spelling of unset), bool/choice (the widget always holds a definite state),
        secrets (empty falls back to the env source), multi-value fields (empty = no
        pieces), degraded/defaultless fields (the script's own behavior is the only
        honest fallback), and input bindings (empty = let the script ask). Placeholder
        delivery is not listed because it has always substituted the field verbatim."""
        return (
            self.has_default
            and not self.secret
            and not self.degraded
            and not self.multiple
            and not self.input_binding
            and not self.empty_uses_default
            and self.kind in ("str", "path")
            and self.source in ("inject", "flag", "env")
        )

    @classmethod
    def from_decl(cls, d: ParamDecl) -> FormField:
        """Project one ParamDecl onto the render-only form model. The single converter for
        every value source, dispatched on delivery — the two hand-written converters this
        replaced (inject-managed params, static CLI flags) plus the command-template
        placeholder path, kept byte-for-byte to preserve the form's behaviour."""
        if d.delivery == "inject":
            # Managed const/input param: injected into a temp copy. source is left to
            # FormField's own default ("inject"). "str" is intentionally absent from the
            # type whitelist — an unknown type already falls back to "str".
            return cls(
                key=d.name,
                label=d.prompt or d.name,
                kind=d.type if d.type in ("int", "float", "bool", "path") else "str",
                default="" if d.default is None else _render_default(d.default),
                has_default=d.default is not None,
                secret=d.secret,
                env_source=d.env_source,
                input_binding=d.binding == "input",
            )
        if d.delivery == "flag":
            # Reflected from the script's own CLI parser: assembled into real argv. A degraded
            # field is a free-text field whatever its declared type said.
            action = d.action
            if not action and not d.degraded and d.type == "bool" and d.flag and not d.default:
                # A declared bool flag whose row names no action (hand-edited meta, or a
                # pre-hygiene `--type v=bool` edit) can only mean "pass the flag when on";
                # without this default the checkbox delivers nothing in EITHER state. Only
                # for a flag that is OFF by default, though: params.edit_declared refuses to
                # record the on-by-default shape, and inferring store_true for one that
                # reached meta.toml by hand would make the unticked box lie.
                action = "store_true"
            return cls(
                key=d.name,
                label=d.name,
                kind="str" if d.degraded else d.type,
                source="flag",
                choices=list(d.choices),
                default="" if d.default is None else _render_default(d.default),
                has_default=d.default is not None,
                help=d.help,
                required=d.required,
                secret=d.secret,
                degraded=d.degraded,
                multiple=d.multiple,
                repeat=d.repeat,
                flag=d.flag,
                action=action,
            )
        if d.delivery == "env":
            # Declared env parameter: the value becomes an environment variable on the
            # child process (zero rewriting — the transparency line shows the overlay).
            return cls(
                key=d.name,
                label=d.prompt or d.name,
                kind=d.type if d.type in ("int", "float", "bool", "choice", "path") else "str",
                source="env",
                choices=list(d.choices),
                default="" if d.default is None else _render_default(d.default),
                has_default=d.default is not None,
                help=d.help,
                required=d.required,
                secret=d.secret,
                env_source=d.env_source,
                env_target=d.env_target,
            )
        # Command-template placeholder (delivery="placeholder"): the value fills a {slot}
        # in the command string. A declared row supplies real schema (type/choices/default/
        # optional); an undeclared placeholder arrives as params.synthesized_placeholder,
        # whose defaults reproduce the historical required-free-text field exactly.
        return cls(
            key=d.name,
            label=d.prompt or d.name,
            kind=d.type if d.type in ("int", "float", "bool", "choice", "path") else "str",
            source="placeholder",
            choices=list(d.choices),
            default="" if d.default is None else _render_default(d.default),
            has_default=d.default is not None,
            help=d.help,
            required=d.required,
            secret=d.secret,
            env_source=d.env_source,
        )


@dataclass
class FormPlan:
    source: str  # "inject" | "argparse" | "command" | "none"
    fields: list[FormField] = field(default_factory=list)
    drift_lines: list[str] = field(default_factory=list)  # localized, shown as a banner
    degraded_reason: str = ""  # argparse whole-parser degradation: "subparsers" | "dynamic"
    specs: list[ParamDecl] = field(default_factory=list)  # inject source only
    text: str = ""  # script text (inject delivery needs it)

    @property
    def secret_names(self) -> set[str]:
        return {f.key for f in self.fields if f.secret}


@dataclass
class Assembly:
    """The delivery-ready result of a submitted form."""

    args: list[str] = field(default_factory=list)  # argv tail (flag source + extra args)
    inject_values: dict[str, str] = field(default_factory=dict)  # inject source, expanded
    command_values: dict[str, str] = field(default_factory=dict)  # placeholder source, expanded
    # command_values with secret VALUES replaced by ••• — the command source's counterpart
    # to masked_args, so a {api_key} placeholder is masked in the shown command line just
    # like a secret flag. The real command_values still runs the process.
    masked_command_values: dict[str, str] = field(default_factory=dict)
    display: list[tuple[str, str]] = field(default_factory=list)  # transparency (secrets masked)
    # args with secret VALUES replaced by ••• — what transparency/--dry-run print, so a
    # secret never lands in the scrollback (the process list is a documented boundary;
    # the terminal log needn't be).
    masked_args: list[str] = field(default_factory=list)
    # env source, expanded: variables overlaid onto the child process environment
    # (keyed by the TARGET variable name). Empty fields are absent — an unset optional
    # env param must leave the variable unset so the script's own default applies.
    env_values: dict[str, str] = field(default_factory=dict)
    masked_env: dict[str, str] = field(default_factory=dict)  # secrets → ••• for display


# --------------------------------------------------------------------------
# plan
# --------------------------------------------------------------------------


def _declared_plan(entry: Entry, lang: LangSpec) -> FormPlan | None:
    """The declared-schema plans (no source analysis involved): placeholder kinds
    (command templates and prompts) and program entries with [[parameters]] rows.
    None means "not this path — keep going"."""
    if lang.placeholder_params:
        # The placeholder list IS the field list; a declared [[parameters]] row supplies
        # a placeholder's schema (type/default/optional/secret override), an undeclared
        # one synthesizes the historical required-free-text field, and declared env
        # params ride along (see params.declared_for_template). The trait — not the
        # family — gates this: a prompt is family "interpreted" (it has an original
        # file) yet its form interface is placeholders, exactly like a command's.
        if lang.stored_name:
            return _placeholder_body_plan(entry)
        decls = params.declared_for_template(entry.meta.parameters, entry.meta.params or [])
        return FormPlan(source="command", fields=[FormField.from_decl(d) for d in decls])
    if lang.params_io is None and lang.cli_reader is None and entry.meta.parameters:
        # Declared parameters apply to every kind whose param home is meta AND has no other
        # detected surface — programs (their only possible form) and Tier-0 interpreted kinds
        # (env is the ${VAR:-default} idiom's zero-rewrite channel even before an analyzer
        # exists). Reader-bearing kinds (PowerShell) are DELIBERATELY excluded: their declared
        # rows must MERGE into the reader's param() plan as riders (plan_for_entry), never
        # short-circuit it — otherwise one declared env var would erase the whole param() form.
        # Flag rows assemble real argv, env rows overlay the child environment; other deliveries
        # mean nothing here and are dropped by the filter.
        decls = [
            d
            for d in params.declared_from_meta(entry.meta.parameters)
            if d.delivery in ("flag", "env")
        ]
        if decls:
            return FormPlan(source="declared", fields=[FormField.from_decl(d) for d in decls])
    return None


def _placeholder_body_plan(entry: Entry) -> FormPlan:
    """The prompt kind's plan: a command-template plan whose placeholder list is the
    entry's MANAGED names (`meta.params` — what the user kept at add time), with the
    body itself consulted fresh for drift. A managed hole the body no longer contains
    still renders as a field (the declared schema is the user's record), but the banner
    says its value would be ignored — the same honesty the in-file kinds get from
    reconcile. An unreadable/missing body degrades to the extra-args-only plan;
    preflight owns existence errors (the form layer never invents fields)."""
    if not entry.meta.interpolate:
        # Insertion is switched off for this prompt: no fields, no candidate scanning,
        # no drift — the body travels verbatim and the run form is just the runner
        # picker. (The managed list survives underneath for a later switch-on.)
        return FormPlan(source="command")
    managed = list(entry.meta.params or [])
    from .langs.prompt import text as prompt_text

    try:
        text = prompt_text.read(entry.script_path)
    except prompt_text.PromptEncodingError:
        # Never build fields or drift facts from replacement characters.  The launch
        # boundary's strict preflight/render returns the user-facing exit-125 refusal;
        # the form layer stays total and invents no schema from unreadable bytes.
        return FormPlan(source="none")
    except OSError:
        return FormPlan(source="none")
    from .langs.prompt import analyzer as prompt_analyzer

    fresh = prompt_analyzer.placeholder_names(text)
    gone = [name for name in managed if name not in fresh]
    drift = (
        [
            gettext(
                "No longer in the prompt (the value would be ignored): %(names)s — "
                "edit the body or update parameters with: skit params %(name)s"
            )
            % {"names": ", ".join(gone), "name": entry.meta.name}
        ]
        if gone
        else []
    )
    decls = params.declared_for_template(entry.meta.parameters, managed)
    return FormPlan(
        source="command",
        fields=[FormField.from_decl(d) for d in decls],
        drift_lines=drift,
        text=text,
    )


def _declared_riders(entry: Entry, taken: set[str]) -> list[FormField]:
    """Declared [[parameters]] flag/env rows to merge into an analyzable kind's plan, minus any
    name already fielded from the in-file block. Flag rows assemble real argv, env rows overlay the
    child environment; other deliveries mean nothing here and are dropped (mirrors _declared_plan)."""
    return [
        FormField.from_decl(d)
        for d in params.declared_from_meta(entry.meta.parameters)
        if d.delivery in ("flag", "env") and d.name not in taken
    ]


def reader_fields(lang: LangSpec | None, text: str) -> int:
    """How many form fields the kind's own CLI reader models from `text` — 0 when there
    is no modeled reader form (no reader, unreadable, dynamic/unmodelable parsing).

    THE predicate for the manage-a-constant trap, shared by every surface that decides
    whether to offer managing (add ticks, `params` advice, Entry settings, the flip
    note): managing REPLACES the run form only when a modeled reader form exists to be
    replaced (plan_for_entry prefers managed specs). A script that self-parses but
    couldn't be modeled (docopt/fire, a dynamic optstring) runs on the passthrough
    field either way — there, managed constants are additive and the offer is honest."""
    if lang is None or lang.cli_reader is None or not text:
        return 0
    spec = lang.cli_reader.read_cli(text)
    if spec is None or not spec.ok:
        return 0
    return len(spec.fields)


def _reader_plan(entry: Entry, reader: CliReader) -> FormPlan | None:
    """The form plan for a reader-only kind (PowerShell): read the script's own CLI surface
    statically and assemble real flags. None when the reader finds nothing readable (no
    surface, or no tool to run the read) — the caller then falls through to the "none" plan,
    exactly like the argparse/parseArgs fall-through does for analyzable kinds."""
    text = entry.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    spec = reader.read_cli(text)
    if spec is None:
        return None
    # A reader-only kind (PowerShell) never whole-spec-degrades: an unreadable surface comes
    # back as None (handled above), never as an ok=False ArgSpec, so there is no degraded-reason
    # branch here — the reader always yields a concrete, assemblable field list.
    return FormPlan(
        source="argparse", fields=[FormField.from_decl(a) for a in spec.fields], text=text
    )


def plan_for_entry(entry: Entry) -> FormPlan:  # noqa: PLR0911 — one return per plan source
    """Build the form plan for an entry. Total: unreadable/missing scripts yield the
    "none" plan (extra-args escape only) rather than raising — preflight owns existence
    errors, the form layer just refuses to invent fields it can't see."""
    lang = spec_for(entry.meta.kind)
    if lang is not None:
        declared = _declared_plan(entry, lang)
        if declared is not None:
            return declared
        # Reader-only kinds — a static CLI surface with no analyzer and no injector, i.e.
        # PowerShell's param() block read through PowerShell's own parser. They have no
        # in-file [tool.skit] analysis path, so they bypass the inject/reconcile machinery
        # below and consult the reader directly. Gated on `analyzer is None`, so every
        # analyzable kind (python/shell/js/ts/fish) keeps its exact existing path — and a
        # kind whose analyzer AND cli_reader were both degraded to None by a broken grammar
        # wheel (js/shell) never enters here either (its cli_reader is None too).
        if lang.analyzer is None and lang.cli_reader is not None and entry.script_path.exists():
            reader_plan = _reader_plan(entry, lang.cli_reader)
            # Declared [[parameters]] flag/env rows ride along after the reader's param()
            # fields, exactly like they merge into an analyzable kind's in-file plan — so
            # hand-declaring an env var augments the param() form instead of erasing it.
            if reader_plan is not None:
                taken = {f.key for f in reader_plan.fields}
                reader_plan.fields += _declared_riders(entry, taken)
                return reader_plan
            riders = _declared_riders(entry, set())
            if riders:
                # No readable param() surface, but declared rows still form a plan on their own.
                return FormPlan(source="declared", fields=riders)
    if (
        lang is None
        or lang.params_io is None
        or lang.analyzer is None
        or not entry.script_path.exists()
    ):
        return FormPlan(source="none")
    text = entry.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    specs = lang.params_io.read(text)
    if specs:
        report = lang.analyzer.reconcile(text, specs)
        drift = list(analysis.drift_lines(report, entry.meta.name)) if report.has_drift else []
        fields = [FormField.from_decl(s) for s in report.usable]
        _refresh_defaults(fields, report.current_defaults, report.empty_uses_default)
        # MERGE: declared [[parameters]] flag/env rows ride along after the analyzer's in-file
        # params (a shell/python entry may hand-declare an env or flag channel the analyzer can't
        # see). Names already fielded from the in-file block win — no duplicate rows. For the
        # overwhelming case (no meta.parameters) this appends nothing, so the plan is byte-identical.
        fields += _declared_riders(entry, {f.key for f in fields})
        return FormPlan(
            source="inject",
            fields=fields,
            drift_lines=drift,
            specs=report.usable,
            text=text,
        )
    # No in-file specs, but declared meta rows may still exist (env is the ${VAR:-default} channel
    # even before anything is managed in-file) — the declared path applies before CLI reflection.
    riders = _declared_riders(entry, set())
    if riders:
        return FormPlan(source="declared", fields=riders, text=text)
    if lang.cli_reader is not None:
        spec = lang.cli_reader.read_cli(text)
        if spec is not None:
            if not spec.ok:
                return FormPlan(source="argparse", degraded_reason=spec.reason, text=text)
            return FormPlan(
                source="argparse",
                fields=[FormField.from_decl(a) for a in spec.fields],
                text=text,
            )
    return FormPlan(source="none", text=text)


def _render_default(value: str | int | float | bool) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def _refresh_defaults(
    fields: list[FormField],
    current: Mapping[str, str | int | float | bool],
    empty_uses_default: set[str],
) -> None:
    """Refresh each field's default from the SOURCE (Report.current_defaults): the
    block's manage-time cache must never beat the script — a user who edits
    `X = "hello"` to `"bonjour"` must see bonjour prefilled, where the stale cache used
    to be silently re-injected over exactly that edit.

    A field with NO recorded default is left alone rather than given one. "The block
    claims a default" and "the script happens to assign a literal" are different facts:
    the first is a value skit promised to manage, the second is just the script's own
    text, which it already runs with. Inventing a default there would prefill a value
    that then gets injected into a temp copy on every run — spending a rewrite (and
    `__file__` / `$0`) to tell the script what it already says."""
    for f in fields:
        if f.has_default and f.key in current:
            f.default = _render_default(current[f.key])
        if f.key in empty_uses_default:
            f.empty_uses_default = True


# --------------------------------------------------------------------------
# prefill / validate
# --------------------------------------------------------------------------


def prefill(plan: FormPlan, slug: str, preset: str | None = None) -> dict[str, str]:
    """Definition default < last-used < preset. Secrets are never prefilled — their
    values are never on disk (C3), and echoing a remembered secret would defeat the
    point of masking."""
    state = argstate.load_state(slug)
    keys = {f.key for f in plan.fields}
    secret = plan.secret_names
    out: dict[str, str] = {}
    for f in plan.fields:
        if f.has_default and not f.secret:
            out[f.key] = f.default
    out.update({k: v for k, v in state["values"].items() if k in keys and k not in secret})
    if preset:
        chosen = state["presets"].get(preset, {})
        out.update({k: v for k, v in chosen.items() if k in keys and k not in secret})
    return out


def validate(plan: FormPlan, values: Mapping[str, str]) -> dict[str, str]:
    """Per-field, user-ready error messages; empty dict means the form may submit."""
    errors: dict[str, str] = {}
    for f in plan.fields:
        error = validate_value(f, values.get(f.key, ""))
        if error:
            errors[f.key] = error
    return errors


def validate_value(f: FormField, value: str) -> str | None:
    """One field's pre-submit check (renderers use this for inline re-prompting).
    Empty optional fields are fine (the script's own default applies); token-bearing
    values are deferred to assembly (they can't be type-checked before expansion)."""
    if not value.strip():
        if f.required:
            return gettext("%(name)s is required.") % {"name": f.label}
        return None
    if tokens.has_tokens(value):
        return None
    return _type_error(f, value)


def _type_error(f: FormField, value: str) -> str | None:
    if f.kind in ("int", "float", "bool"):
        try:
            # A multi-value field holds several values in one box (`--point 1 2`), so the
            # type applies to each PIECE — coercing the whole string rejected the only legal
            # input a typed nargs>1 option has ("1 2" is not a whole number, but 1 and 2 are).
            # Split the same way assembly will; an unsplittable string stays one piece and
            # fails as before.
            # f.key feeds only InjectValueError's param_name, which this function discards (it
            # rebuilds the message from f.label below); f.key -> None is thus equivalent. The
            # killable value/kind coercion stays covered by test_type_error_messages_exact.
            for piece in _shlex_pieces(value) if f.multiple else [value]:
                _coerce(piece, f.kind, f.key)  # pragma: no mutate
        except InjectValueError:
            type_names = {
                "int": gettext("a whole number"),
                "float": gettext("a number"),
                "bool": gettext("on or off"),
            }
            return gettext("%(name)s needs %(type)s — you typed %(value)r.") % {
                "name": f.label,
                "type": type_names[f.kind],
                "value": value,
            }
    if f.kind == "choice" and f.choices and value not in f.choices:
        return gettext("%(name)s must be one of: %(choices)s") % {
            "name": f.label,
            "choices": ", ".join(f.choices),
        }
    return None


def glob_feedback(value: str, cwd: Path) -> int | None:
    """Live match count for a multi-file field ("✓ matches N files"); None when the
    value has no glob characters (nothing to report)."""
    if not any(ch in value for ch in _GLOB_CHARS):
        return None
    try:
        pieces = shlex.split(value)
    except ValueError:
        return None
    count = 0
    for piece in pieces:
        if any(ch in piece for ch in _GLOB_CHARS):
            count += len(_glob.glob(piece, root_dir=str(cwd), recursive=True))
        else:
            count += 1
    return count


# --------------------------------------------------------------------------
# assemble
# --------------------------------------------------------------------------


def assemble(
    plan: FormPlan,
    values: Mapping[str, str],
    extra_args: list[str],
    *,
    cwd: Path,
    env: Mapping[str, str] | None = None,
    now: datetime | None = None,
    expand_extra: bool = True,
) -> Assembly:
    """Turn raw form values (token/glob originals) into delivery-ready material.
    Raises FormError with a user-ready message; never assembles around a hole."""
    if env is None:
        env = os.environ
    final: dict[str, str] = {}
    display: list[tuple[str, str]] = []
    for f in plan.fields:
        # A missing key means "unset": "" and None are falsy-equivalent everywhere downstream
        # (_final_value / _resolve_secret only read `raw` via truthiness), so the default is a
        # genuinely equivalent mutant. The killable "XXXX"-style mutant stays covered by
        # test_assemble_degraded_empty_omitted_filled_passed (a missing field must not inject).
        raw = values.get(f.key, "")  # pragma: no mutate
        value = _final_value(f, raw, cwd=cwd, env=env, now=now)
        if f.source == "inject" and (value or f.delivers_empty):
            # ONLY inject-delivered values belong under the "→ inject:" transparency
            # line (its caveat is the temporary-copy rewrite); env values already
            # render as a VAR=value prefix and flag/placeholder values appear in the
            # command line itself — listing them here claimed a delivery that never
            # happens. A delivered empty string shows as '' — a blank after the = would
            # read as nothing being delivered, which is the opposite of the truth.
            display.append((f.key, "•••" if f.secret else (value or "''")))
        final[f.key] = value
    # expand_extra=False: the CLI's `-- args` already went through the user's shell —
    # a second token/glob pass would rewrite what they deliberately quoted (and --raw
    # must be genuinely raw). The TUI's extra-args field has no shell, so it expands.
    if expand_extra:
        expanded_extra: list[str] = []
        for item in extra_args:
            try:
                expanded_extra.extend(
                    _expand_glob_piece(tokens.expand(item, cwd=cwd, env=env, now=now), cwd)
                )
            except tokens.TokenError as exc:
                raise FormError(str(exc)) from exc
    else:
        expanded_extra = list(extra_args)
    out = Assembly(display=display)
    # Routing is per FIELD on its source (the delivery axis), not per plan: one plan can
    # legitimately mix deliveries (declared flag+env params on a program entry;
    # placeholders + env params on a command template). final[f.key] is always present
    # (the loop above wrote every field's key).
    out.inject_values = {
        f.key: final[f.key]
        for f in plan.fields
        if f.source == "inject" and (final[f.key] or f.delivers_empty)
    }
    if any(f.source == "flag" for f in plan.fields):
        out.args = _assemble_flags(plan, final, cwd) + expanded_extra
        masked_final = {
            f.key: ("•••" if f.secret and final[f.key] else final[f.key]) for f in plan.fields
        }
        out.masked_args = _assemble_flags(plan, masked_final, cwd) + expanded_extra
    else:
        out.args = expanded_extra
        out.masked_args = list(expanded_extra)
    placeholder_fields = [f for f in plan.fields if f.source == "placeholder"]
    if placeholder_fields:
        out.command_values = {f.key: final[f.key] for f in placeholder_fields}
        # A {api_key} placeholder is a secret like any other: mask its value in the shown
        # command line so it never reaches the scrollback / --dry-run output, while the
        # real value still substitutes into the process that runs.
        out.masked_command_values = {
            f.key: ("•••" if f.secret and final[f.key] else final[f.key])
            for f in placeholder_fields
        }
    # Empty env fields are ABSENT (not set to ""): leaving the variable unset is what
    # lets the script's own default fire; an empty-string export would shadow it. The
    # exception is a delivers-empty field, where a cleared value IS the export — a
    # ${NAME:-default} script still falls back (shell's own ':-' semantics treat null
    # as unset), while a ${NAME-default} script genuinely receives the empty string.
    env_fields = [
        f for f in plan.fields if f.source == "env" and (final[f.key] or f.delivers_empty)
    ]
    out.env_values = {(f.env_target or f.key): final[f.key] for f in env_fields}
    out.masked_env = {
        (f.env_target or f.key): ("•••" if f.secret else final[f.key]) for f in env_fields
    }
    return out


def _final_value(
    f: FormField,
    raw: str,
    *,
    cwd: Path,
    env: Mapping[str, str],
    now: datetime | None,
) -> str:
    """One field's delivery value: secret resolution or token expansion, plus the
    post-expansion type check that pre-submit validation had to defer."""
    if f.secret:
        return _resolve_secret(f, raw, env)
    try:
        # Placeholder-delivery values keep `{{`/`}}` literal (prompt text is
        # brace-heavy; the body promise is "unmanaged text travels untouched") —
        # the escape pair belongs to inject/flag values only.
        value = (
            tokens.expand(raw, cwd=cwd, env=env, now=now, brace_escapes=f.source != "placeholder")
            if raw
            else ""
        )
    except tokens.TokenError as exc:
        raise FormError(str(exc)) from exc
    if raw and tokens.has_tokens(raw):
        error = _type_error(f, value)
        if error:
            raise FormError(error)
    return value


def _resolve_secret(f: FormField, raw: str, env: Mapping[str, str]) -> str:
    """This run's typed value wins; else the configured environment source; a configured
    source that is unset is a named error, not an empty string."""
    if raw:
        return raw
    if f.env_source:
        if f.env_source not in env:
            raise FormError(
                gettext("%(name)s reads from the environment variable %(env)s, but it isn't set.")
                % {"name": f.label, "env": f.env_source}
            )
        return env[f.env_source]
    return ""


def _assemble_flags(plan: FormPlan, final: Mapping[str, str], cwd: Path) -> list[str]:
    """Positional fields in declared order, then option flags in declared order.
    Explicit-pass rule: filled fields always travel; checkboxes only when they differ
    from the script default; degraded/empty fields are omitted."""
    positionals: list[str] = []
    flags: list[str] = []
    for f in plan.fields:
        if f.source != "flag":
            continue  # mixed-delivery plans: env/placeholder fields never enter argv
        value = final.get(f.key, "")
        if f.kind == "bool":
            fired = _coerce_bool_lenient(value)
            # A checkbox fires its flag only when it differs from the script default:
            # store_true fires on checked, store_false fires on unchecked. A flagless
            # bool (a hand-declared positional) has nothing to append — never argv "".
            if f.flag and (
                (f.action == "store_true" and fired) or (f.action == "store_false" and not fired)
            ):
                flags.append(f.flag)
            continue
        if not value and not f.delivers_empty:
            continue  # optional/degraded left empty: the script's own default applies
        pieces = _split_multi(value, cwd) if f.multiple else [value]
        if f.flag == "":
            positionals.extend(pieces)
        elif f.repeat:
            # Click multiple=True / parseArgs multiple: the option must be REPEATED per
            # value (`--tag a --tag b`); the one-flag-many-values shape below is argparse
            # nargs grammar and reads as one flag plus positionals to these parsers.
            for piece in pieces:
                flags.append(f.flag)
                flags.append(piece)
        else:
            flags.append(f.flag)
            flags.extend(pieces)
    return positionals + flags


def _shlex_pieces(value: str) -> list[str]:
    """The individual values a multi-value field's single box holds. An unbalanced quote is
    not a split — it is one literal value, the reading assembly has always used."""
    try:
        return shlex.split(value)
    except ValueError:
        return [value]


def _split_multi(value: str, cwd: Path) -> list[str]:
    """A multi-value field holds shell-ish text: split it like a shell would, then
    re-expand globs against the run's cwd (the TUI has no shell to do either)."""
    pieces = _shlex_pieces(value)
    out: list[str] = []
    for piece in pieces:
        out.extend(_expand_glob_piece(piece, cwd))
    return out


def _expand_glob_piece(piece: str, cwd: Path) -> list[str]:
    if not any(ch in piece for ch in _GLOB_CHARS):
        return [piece]
    matches = sorted(_glob.glob(piece, root_dir=str(cwd), recursive=True))
    return matches if matches else [piece]


def truthy(value: str) -> bool:
    """THE bool-spelling rule, public and single: every renderer that shows a checkbox
    state must accept exactly the spellings assembly accepts — "on"/"y" firing the
    flag at run time while rendering unchecked was two rules pretending to be one.
    Anything unrecognized counts as unchecked (validate() already rejected typed
    garbage for bool fields)."""
    return value.strip().lower() in ("true", "1", "yes", "y", "on")


_coerce_bool_lenient = truthy  # internal name kept for the assembly call sites


# --------------------------------------------------------------------------
# after the run
# --------------------------------------------------------------------------


def remembered_values(plan: FormPlan, values: Mapping[str, str]) -> dict[str, str]:
    """What LAST-USED stores: the values that differ from their definition default,
    empties dropped (an empty means "unset" — storing it would shadow a later
    definition default) except on a delivers-empty field, where a cleared value was
    genuinely delivered as '' and must replay as one.

    Accepting a default is not a choice: remembering it would freeze today's default
    and hide tomorrow's, which is the exact staleness this change set out to kill. A
    preset is the deliberate, named way to pin one — presets store the run's values
    verbatim, which is why only this last-used lane filters."""
    by_key = {f.key: f for f in plan.fields}
    out: dict[str, str] = {}
    for k, v in values.items():
        f = by_key.get(k)
        if f is not None and f.has_default and v == f.default:
            continue
        if v or (f is not None and f.delivers_empty):
            out[k] = v
    return out


def save_after_run(
    slug: str,
    plan: FormPlan,
    values: Mapping[str, str],
    extra_args: list[str],
    exit_code: int,
    *,
    at: str,
) -> None:
    """Persist intent (raw token/glob text), never expansion; secrets structurally
    stripped by argstate (C3); stamp the run for Library sorting and the r key."""
    # Retroactive C3 scrub: a placeholder/param that is secret NOW must not keep old
    # plaintext in values or presets from the days it wasn't (purge is idempotent).
    if plan.secret_names:
        argstate.purge_secret(slug, plan.secret_names)
    argstate.save_last(
        slug,
        values=remembered_values(plan, values),
        extra_args=list(extra_args),
        secret_names=plan.secret_names,
    )
    argstate.record_run(
        slug,
        exit_code,
        at=at,
        values=dict(values),
        secret_names=plan.secret_names,
    )


# --------------------------------------------------------------------------
# execute — the single delivery pipeline (one flow, two renderings)
# --------------------------------------------------------------------------

# Why a code did not launch, so each renderer can classify without re-catching the
# launcher/shim exception hierarchy (the CLI maps these to exit codes 125/126/127; the
# TUI maps them to a status line). "" means the script actually ran.
FAIL_BAD_VALUE = "bad_value"  # a value doesn't fit its declared type (shim rejected it)
FAIL_DRIFT = "drift"  # injection targets no longer match the definitions
FAIL_MISSING = "missing"  # the launch target is gone from disk
FAIL_NOT_EXECUTABLE = "not_executable"  # an exe exists but isn't +x
FAIL_LAUNCH = "launch"  # any other launch failure

# Docker-convention process exit codes when the launch itself failed (the script's own
# exit code passes through untouched whenever it did run): skit failures are 125, an
# existing-but-unexecutable target 126, a missing target 127. Shared by `skit run` and
# the TUI's exit-after-run path so the contract can't fork.
FAILURE_EXIT_CODES = {
    FAIL_BAD_VALUE: 125,
    FAIL_DRIFT: 125,
    FAIL_LAUNCH: 125,
    FAIL_NOT_EXECUTABLE: 126,
    FAIL_MISSING: 127,
}


@dataclass
class RunOutcome:
    """The result of execute(). code is the script's own exit code, or None when the
    script never launched (failure names why, message is user-ready and localized)."""

    code: int | None
    failure: str = ""
    message: str = ""

    @property
    def launched(self) -> bool:
        return self.code is not None


def transparency_lines(
    entry: Entry,
    asm: Assembly,
    injected: Path | None,
    *,
    runner: PromptRunner | None = None,
    exact_prompt: bool = False,
    validated_prompt_command: str | None = None,
) -> list[str]:
    """The plain what-actually-runs lines (trust through transparency; also how users
    passively learn their scripts' own flags). Secret values are already masked in
    asm.masked_args / asm.display. The renderer applies its own styling/escaping — these
    are semantic plain text, identical across CLI and TUI (the one place they used to
    drift: `k = v` vs `k = 'v'`). `runner` is the prompt kind's resolved per-run pick,
    threaded through to describe so --dry-run prints the real argv."""
    lines: list[str] = []
    if asm.inject_values:
        pairs = ", ".join(f"{k} = {v}" for k, v in asm.display)
        lines.append(gettext("→ inject: %(pairs)s") % {"pairs": pairs})
        lines.append(
            gettext(
                "  (written to a temporary copy, deleted after the run; your original file is untouched)"
            )
        )
    # Env-delivered values render as a copy-pasteable VAR=value prefix (masked) — the
    # honest picture of what actually happens: the child process env is overlaid, the
    # script itself is never rewritten for these.
    env_prefix = "".join(f"{k}={launch_quote(v)} " for k, v in asm.masked_env.items())
    spec = spec_for(entry.meta.kind)
    if validated_prompt_command is not None:
        # --dry-run must display the exact body snapshot that passed prompt argv
        # validation, never re-read a reference or concurrently edited copy here.
        described = validated_prompt_command
    elif not exact_prompt and spec is not None and isinstance(spec.launch, PromptLaunch):
        described = spec.launch.describe_compact(entry, asm.masked_args, runner=runner)
    else:
        described = launcher.describe_command(
            entry,
            asm.masked_args,
            asm.masked_command_values,
            script_override=injected,
            runner=runner,
        )
    lines.append("→ " + env_prefix + described)
    return lines


def validate_prompt_argv(
    entry: Entry,
    asm: Assembly,
    *,
    runner: PromptRunner | None = None,
) -> tuple[str, str] | None:
    """Validate prompt rendering/argv limits without looking up the runner on PATH.

    Non-prompt kinds are a no-op.  The CLI calls this before --dry-run output; execute
    calls it before normal transparency, so an impossible prompt is never dumped into
    terminal scrollback immediately before skit refuses it.
    """
    spec = spec_for(entry.meta.kind)
    if spec is None or not isinstance(spec.launch, PromptLaunch):
        return None
    return spec.launch.validate_argv_snapshot(
        entry,
        asm.args,
        asm.command_values,
        None,
        runner=runner,
        display_values=asm.masked_command_values,
    )


def _prompt_secret_warning(plan: FormPlan, asm: Assembly) -> str:
    """The delivery-boundary warning, only when plaintext will actually be sent."""
    sends_secret = any(
        f.source == "placeholder" and f.secret and bool(asm.command_values.get(f.key))
        for f in plan.fields
    )
    if not sends_secret:
        return ""
    return gettext(
        "Secret-marked values are never saved by skit, but this prompt sends them to the "
        "selected agent as plaintext; the agent may log or sync them."
    )


def _split_message(exc: InjectSplitError) -> str:
    """The user-facing wording for each way a `read` line would mangle a value. A closed dict of
    gettext literals (never gettext(reason)) so Babel can extract them — the same discipline
    analysis.render_warning uses."""
    return {
        "line-break": gettext(
            "%(name)s can't contain a line break: a shell `read` takes ONE line, so everything "
            "after the break would be thrown away."
        ),
        "field-split": gettext(
            "%(name)s is read on the same line as other values, so its value can't contain spaces "
            "or tabs — the shell would split it across the other fields. Only the LAST value on a "
            "`read` line may contain spaces."
        ),
        "edge-space": gettext(
            "%(name)s starts or ends with a space or tab, which a shell `read` strips off the "
            "line — the script would receive it trimmed. Remove the surrounding whitespace."
        ),
    }[exc.reason] % {"name": exc.name}


def execute(  # noqa: PLR0911, PLR0912 — one early return/branch per injection failure mode; a flat error dispatch
    entry: Entry,
    plan: FormPlan,
    asm: Assembly,
    *,
    emit: Callable[[str], None],
    warn: Callable[[str], None] | None = None,
    invoke_cwd: Path | None = None,
    runner: PromptRunner | None = None,
) -> RunOutcome:
    """Deliver an assembled run: inject (if the source calls for it) -> emit the
    transparency lines -> run straight through the terminal -> clean up the temp copy.

    The single delivery pipeline both `skit run` and the TUI go through, so the two can
    never drift again. The caller owns everything around it: prompting, the suspend
    boundary (the TUI must call this inside App.suspend()), the run banner, and mapping
    the outcome to an exit code or a status line. emit receives already-localized plain
    lines; the renderer styles them (dim console print / bare print).

    Injection is the kind's own `injector` capability (python rewrites a temp copy; shell
    picks per parameter between an environment overlay and a temp-copy rewrite), so this
    function knows nothing about any language. A kind with no injector simply doesn't
    inject — the same degradation as a missing analyzer, and unreachable in practice since
    an inject plan only exists where an analyzer does.
    """
    injected: Path | None = None
    prepared: launcher.PreparedLaunch | None = None
    emit_warning = warn or emit
    try:
        # The injector's env overlay rides ON TOP of the assembled env-delivered values:
        # both are "set this variable on the child", and a shell entry can legitimately
        # produce both at once (a declared env rider plus an envdefault param).
        env_overlay = dict(asm.env_values)
        spec = spec_for(entry.meta.kind)
        if spec is not None and isinstance(spec.launch, PromptLaunch):
            try:
                # Cross the delivery boundary only after the exact runner/body argv,
                # executable, needs and cwd have all succeeded. run_entry consumes
                # this same snapshot below; it never re-reads or rebuilds the prompt.
                prepared = launcher.prepare_entry(
                    entry,
                    asm.args,
                    values=asm.command_values,
                    invoke_cwd=invoke_cwd,
                    runner=runner,
                )
            except launcher.TargetMissingError as exc:
                return RunOutcome(None, FAIL_MISSING, str(exc))
            except launcher.NotExecutableError as exc:
                return RunOutcome(None, FAIL_NOT_EXECUTABLE, str(exc))
            except launcher.LaunchError as exc:
                return RunOutcome(None, FAIL_LAUNCH, str(exc))
            amp_seed = next(r for r in config.PROMPT_RUNNER_SEEDS if r.name == "amp")
            if prepared.prompt_runner == amp_seed:
                emit_warning(
                    gettext(
                        "The built-in amp runner is one-shot: amp -x runs this prompt once "
                        "and does not open an interactive session."
                    )
                )
            if prepared.warning:
                emit_warning(prepared.warning)
            secret_warning = _prompt_secret_warning(plan, asm)
            if secret_warning:
                emit_warning(secret_warning)
        if (
            plan.source == "inject"
            and asm.inject_values
            and spec is not None
            and spec.injector is not None
        ):
            try:
                result = spec.injector.inject(
                    InjectRequest(
                        text=plan.text,
                        specs=plan.specs,
                        values=asm.inject_values,
                        # entry.dir is write_injected's fallback directory (used only when
                        # the OS temp dir isn't writable);
                        # test_execute_inject_falls_back_to_entry_dir pins that this run
                        # passes it through.
                        entry_dir=entry.dir,
                        interpreter=entry.meta.interpreter or spec.default_interpreter,
                        # Deps-managed npm entries run their temp copy FROM entry_dir, so the
                        # runner's upward module resolution still finds entry_dir/node_modules.
                        # (A no-deps entry's temp copy stays in the OS temp dir — the secret-leftover
                        # default; a consequence, shared with the Python injector's __file__, is that
                        # `__dirname`/`import.meta.url` differ between an injected and a stored run.)
                        prefer_entry_dir=(
                            spec.deps_flavor == "npm"
                            and entry.meta.mode == "copy"
                            and bool(entry.meta.dependencies)
                        ),
                        # The original filename, so the JS/TS injector can give its temp copy an
                        # .mjs/.cjs extension when the origin pinned a module flavor (the store
                        # flattens the stored copy to script.js, losing that signal otherwise).
                        source=entry.meta.source,
                    )
                )
            except InjectValueError as exc:
                return RunOutcome(
                    None,
                    FAIL_BAD_VALUE,
                    gettext("%(value)s isn't a valid %(type)s for %(param)s.")
                    % {
                        "value": repr(exc.value),
                        "type": exc.type_name,
                        "param": exc.param_name,
                    },
                )
            except InjectGapError as exc:
                return RunOutcome(
                    None,
                    FAIL_BAD_VALUE,
                    gettext(
                        "%(empty)s is empty, but %(filled)s is filled and they are read on the "
                        "same line — a shell `read` would hand your value to %(empty)s. Fill "
                        "%(empty)s in, or clear %(filled)s."
                    )
                    % {"empty": exc.empty, "filled": exc.filled},
                )
            except InjectSplitError as exc:
                return RunOutcome(None, FAIL_BAD_VALUE, _split_message(exc))
            except InjectSyntaxError as exc:
                # skit corrupted the script (a quoting/escaping bug) — a resync fixes
                # nothing, so this must NOT carry the drift hint. Nothing was launched.
                return RunOutcome(
                    None,
                    FAIL_DRIFT,
                    gettext("skit refused to run its own injected copy: %(detail)s")
                    % {"detail": str(exc)},
                )
            except InjectError as exc:
                return RunOutcome(
                    None,
                    FAIL_DRIFT,
                    gettext(
                        "The script and its form definitions don't match anymore: %(detail)s. "
                        "Run `skit params %(name)s --resync` to fix it."
                    )
                    % {"name": entry.meta.name, "detail": str(exc)},
                )
            injected = result.path
            env_overlay.update(result.env)
            for line in result.warnings:
                emit(line)
        for line in transparency_lines(
            entry,
            asm,
            injected,
            runner=runner,
            validated_prompt_command=(prepared.safe_display if prepared is not None else None),
        ):
            emit(line)
        try:
            if prepared is None:
                # Keep the established non-prompt call seam unchanged; many callers
                # replace run_entry with a narrow adapter that knows no prepared kwarg.
                code = launcher.run_entry(
                    entry,
                    asm.args,
                    values=asm.command_values,
                    invoke_cwd=invoke_cwd,
                    script_override=injected,
                    env_overlay=env_overlay,
                    runner=runner,
                )
            else:
                code = launcher.run_entry(
                    entry,
                    asm.args,
                    values=asm.command_values,
                    invoke_cwd=invoke_cwd,
                    script_override=injected,
                    env_overlay=env_overlay,
                    runner=runner,
                    prepared=prepared,
                )
        except launcher.TargetMissingError as exc:
            return RunOutcome(None, FAIL_MISSING, str(exc))
        except launcher.NotExecutableError as exc:
            return RunOutcome(None, FAIL_NOT_EXECUTABLE, str(exc))
        except launcher.LaunchError as exc:
            return RunOutcome(None, FAIL_LAUNCH, str(exc))
        return RunOutcome(code)
    finally:
        if injected is not None and injected.exists():
            # missing_ok is redundant: exists() already gated this, and we created the
            # file ourselves, so True/False/None all unlink it identically here.
            injected.unlink(missing_ok=True)  # pragma: no mutate
