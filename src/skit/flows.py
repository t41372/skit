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
- Values persist as TYPED TEXT (token/glob originals) — intent, not expansion.
- Tokens expand and globs re-expand at assembly time, every run.
- Explicit-pass: a filled field is always passed (reproducibility); checkboxes pass
  their flag only when they differ from the script's default; degraded fields are
  omitted when empty so the script's own default applies.
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

from . import argspec, argstate, launcher, metawriter, reconcile, shim, tokens
from .analyzer import _is_secret_name
from .i18n import gettext
from .models import Entry
from .shim import ShimValueError, _coerce

if TYPE_CHECKING:
    from collections.abc import Callable

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
    flag: str = ""  # "--output"; "" = positional (flag source only)
    action: str = ""  # store_true | store_false (bool flags)


@dataclass
class FormPlan:
    source: str  # "inject" | "argparse" | "command" | "none"
    fields: list[FormField] = field(default_factory=list)
    drift_lines: list[str] = field(default_factory=list)  # localized, shown as a banner
    degraded_reason: str = ""  # argparse whole-parser degradation: "subparsers" | "dynamic"
    specs: list[metawriter.ParamSpec] = field(default_factory=list)  # inject source only
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


# --------------------------------------------------------------------------
# plan
# --------------------------------------------------------------------------


def plan_for_entry(entry: Entry) -> FormPlan:
    """Build the form plan for an entry. Total: unreadable/missing scripts yield the
    "none" plan (extra-args escape only) rather than raising — preflight owns existence
    errors, the form layer just refuses to invent fields it can't see."""
    if entry.meta.kind == "command":
        # required: an empty placeholder silently assembles a broken command (`convert '' ''`),
        # which the non-interactive contract forbids. secret: C3 applies to every source —
        # a {api_key} placeholder masks and never lands in a state file, like any other secret.
        fields = [
            FormField(
                key=p, label=p, source="placeholder", required=True, secret=_is_secret_name(p)
            )
            for p in (entry.meta.params or [])
        ]
        return FormPlan(source="command", fields=fields)
    if entry.meta.kind != "python" or not entry.script_path.exists():
        return FormPlan(source="none")
    text = entry.script_path.read_text(encoding="utf-8", errors="replace")  # pragma: no mutate
    specs = metawriter.read_params(text)
    if specs:
        report = reconcile.reconcile(text, specs)
        drift = list(reconcile.drift_lines(report, entry.meta.name)) if report.has_drift else []
        return FormPlan(
            source="inject",
            fields=[_field_from_spec(s) for s in report.usable],
            drift_lines=drift,
            specs=report.usable,
            text=text,
        )
    spec = argspec.read_cli(text)
    if spec is not None:
        if not spec.ok:
            return FormPlan(source="argparse", degraded_reason=spec.reason, text=text)
        return FormPlan(
            source="argparse", fields=[_field_from_arg(a) for a in spec.fields], text=text
        )
    return FormPlan(source="none", text=text)


def _field_from_spec(s: metawriter.ParamSpec) -> FormField:
    return FormField(
        key=s.name,
        label=s.prompt or s.name,
        # "str" is intentionally absent from the whitelist: an unknown type already falls back
        # to "str", so listing it would be redundant (and only breeds equivalent mutants).
        kind=s.type if s.type in ("int", "float", "bool") else "str",
        # source is left to FormField's own default ("inject") — inject IS the default source, so
        # spelling it out here would only be a redundant, unkillable-equivalent mutation site.
        # test_field_from_spec_maps_every_field pins that the result is "inject".
        default="" if s.default is None else _render_default(s.default),
        has_default=s.default is not None,
        secret=s.secret,
        env_source=s.env_source,
    )


def _field_from_arg(a: argspec.ArgField) -> FormField:
    return FormField(
        key=a.dest,
        label=a.dest,
        kind="str" if a.degraded else a.kind,
        source="flag",
        choices=list(a.choices),
        default="" if a.default is None else _render_default(a.default),
        has_default=a.default is not None,
        help=a.help,
        required=a.required,
        secret=a.secret,
        degraded=a.degraded,
        multiple=a.multiple,
        flag=a.flag,
        action=a.action,
    )


def _render_default(value: str | int | float | bool) -> str:
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


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
            # f.key feeds only ShimValueError's param_name, which this function discards (it
            # rebuilds the message from f.label below); f.key -> None is thus equivalent. The
            # killable value/kind coercion stays covered by test_type_error_messages_exact.
            _coerce(value, f.kind, f.key)  # pragma: no mutate
        except ShimValueError:
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
        if value:
            display.append((f.key, "•••" if f.secret else value))
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
    if plan.source == "inject":
        out.inject_values = {k: v for k, v in final.items() if v}
        out.args = expanded_extra
        out.masked_args = list(expanded_extra)
    elif plan.source == "argparse":
        out.args = _assemble_flags(plan, final, cwd) + expanded_extra
        # final[f.key] is always present (the loop above wrote every field's key).
        masked_final = {
            f.key: ("•••" if f.secret and final[f.key] else final[f.key]) for f in plan.fields
        }
        out.masked_args = _assemble_flags(plan, masked_final, cwd) + expanded_extra
    elif plan.source == "command":
        out.command_values = final
        # A {api_key} placeholder is a secret like any other: mask its value in the shown
        # command line so it never reaches the scrollback / --dry-run output, while the
        # real value still substitutes into the process that runs. final[f.key] is always
        # present (the loop above wrote every field's key).
        out.masked_command_values = {
            f.key: ("•••" if f.secret and final[f.key] else final[f.key]) for f in plan.fields
        }
        out.args = expanded_extra
        out.masked_args = list(expanded_extra)
    else:
        out.args = expanded_extra
        out.masked_args = list(expanded_extra)
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
        value = tokens.expand(raw, cwd=cwd, env=env, now=now) if raw else ""
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
        value = final.get(f.key, "")
        if f.kind == "bool":
            fired = _coerce_bool_lenient(value)
            # A checkbox fires its flag only when it differs from the script default:
            # store_true fires on checked, store_false fires on unchecked.
            if (f.action == "store_true" and fired) or (f.action == "store_false" and not fired):
                flags.append(f.flag)
            continue
        if not value:
            continue  # optional/degraded left empty: the script's own default applies
        pieces = _split_multi(value, cwd) if f.multiple else [value]
        if f.flag == "":
            positionals.extend(pieces)
        else:
            flags.append(f.flag)
            flags.extend(pieces)
    return positionals + flags


def _split_multi(value: str, cwd: Path) -> list[str]:
    """A multi-value field holds shell-ish text: split it like a shell would, then
    re-expand globs against the run's cwd (the TUI has no shell to do either)."""
    try:
        pieces = shlex.split(value)
    except ValueError:
        pieces = [value]
    out: list[str] = []
    for piece in pieces:
        out.extend(_expand_glob_piece(piece, cwd))
    return out


def _expand_glob_piece(piece: str, cwd: Path) -> list[str]:
    if not any(ch in piece for ch in _GLOB_CHARS):
        return [piece]
    matches = sorted(_glob.glob(piece, root_dir=str(cwd), recursive=True))
    return matches if matches else [piece]


def _coerce_bool_lenient(value: str) -> bool:
    """Checkbox state from its string form; anything unrecognized counts as unchecked
    (validate() already rejected typed garbage for bool fields)."""
    return value.strip().lower() in ("true", "1", "yes", "y", "on")


# --------------------------------------------------------------------------
# after the run
# --------------------------------------------------------------------------


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
        values={k: v for k, v in values.items() if v},
        extra_args=list(extra_args),
        secret_names=plan.secret_names,
    )
    argstate.record_run(slug, exit_code, at=at)


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


def transparency_lines(entry: Entry, asm: Assembly, injected: Path | None) -> list[str]:
    """The plain what-actually-runs lines (trust through transparency; also how users
    passively learn their scripts' own flags). Secret values are already masked in
    asm.masked_args / asm.display. The renderer applies its own styling/escaping — these
    are semantic plain text, identical across CLI and TUI (the one place they used to
    drift: `k = v` vs `k = 'v'`)."""
    lines: list[str] = []
    if asm.inject_values:
        pairs = ", ".join(f"{k} = {v}" for k, v in asm.display)
        lines.append(gettext("→ inject: %(pairs)s") % {"pairs": pairs})
        lines.append(
            gettext(
                "  (written to a temporary copy, deleted after the run; your original file is untouched)"
            )
        )
    lines.append(
        "→ "
        + launcher.describe_command(
            entry, asm.masked_args, asm.masked_command_values, script_override=injected
        )
    )
    return lines


def execute(
    entry: Entry,
    plan: FormPlan,
    asm: Assembly,
    *,
    emit: Callable[[str], None],
    invoke_cwd: Path | None = None,
) -> RunOutcome:
    """Deliver an assembled run: inject (if the source calls for it) -> emit the
    transparency lines -> run straight through the terminal -> clean up the temp copy.

    The single delivery pipeline both `skit run` and the TUI go through, so the two can
    never drift again. The caller owns everything around it: prompting, the suspend
    boundary (the TUI must call this inside App.suspend()), the run banner, and mapping
    the outcome to an exit code or a status line. emit receives already-localized plain
    lines; the renderer styles them (dim console print / bare print).
    """
    injected: Path | None = None
    try:
        if plan.source == "inject" and asm.inject_values:
            try:
                # entry.dir is write_injected's fallback directory (used only when the OS
                # temp dir isn't writable); test_execute_inject_falls_back_to_entry_dir
                # pins that this run passes it through.
                injected = shim.write_injected(
                    entry.dir, shim.inject(plan.text, plan.specs, asm.inject_values)
                )
            except ShimValueError as exc:
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
            except shim.ShimError as exc:
                return RunOutcome(
                    None,
                    FAIL_DRIFT,
                    gettext(
                        "The script and its form definitions don't match anymore: %(detail)s. "
                        "Run `skit params %(name)s --resync` to fix it."
                    )
                    % {"name": entry.meta.name, "detail": str(exc)},
                )
        for line in transparency_lines(entry, asm, injected):
            emit(line)
        try:
            code = launcher.run_entry(
                entry,
                asm.args,
                values=asm.command_values,
                invoke_cwd=invoke_cwd,
                script_override=injected,
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
