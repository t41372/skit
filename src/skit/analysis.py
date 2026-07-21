"""Language-neutral analysis model + drift reconciliation (docs/design/multilang.md).

Two things live here, shared by every analyzable language (python today, shell next):

- **The candidate model** — `Candidate` (one detected parameter) and `Analysis` (the whole
  detection result). Every language's `analyze(text) -> Analysis` returns these; they are
  field-aligned with `params.ParamDecl` so the CLI, TUI, and reconcile can't drift on which
  fields carry over. Moved here verbatim from the Python analyzer so a second language
  (shell) does not import the Python one just to name its result type.

- **The reconcile machinery** — `Report`, `drift_lines`, `render_warning`, `EditResult`,
  `edit_specs`, `reconcile`. All of it is language-agnostic *except* the one call to
  `analyze(text)`, which each caller supplies as an explicit `analyze` parameter. The
  per-language module (`langs/python/reconcile.py`, `langs/shell`) is a thin wiring shim
  that binds its own analyzer's `analyze` and re-exports the rest.

This module only makes **decisions**; it does no I/O and produces no user copy beyond the
two shared render helpers (`drift_lines`/`render_warning`), which are the only copy exit
points — presentation/markup is left to the CLI/TUI. Headless, stdlib + skit-neutral only.
"""

from __future__ import annotations

from dataclasses import dataclass, field, replace
from typing import TYPE_CHECKING

from .callmatch import match_calls
from .params import ParamDecl

if TYPE_CHECKING:
    from collections.abc import Callable


@dataclass
class Candidate:
    """A candidate parameter. const/envdefault are keyed by variable name; input by call
    order (B1/A8)."""

    # "const" | "input" | "envdefault" — the source-anchor axis (field-aligned with
    # ParamDecl.binding). "envdefault" is shell's ${NAME:-default} idiom (env delivery).
    binding: str
    name: str  # const: variable name; input: display name (input-1, …); envdefault: variable name
    type: str = "str"  # one of INJECTABLE_TYPES
    default: str | int | float | bool | None = None  # const/envdefault: the source value/default
    prompt: str = ""  # input: the literal prompt of input()/read (if any)
    order: int = -1  # input: which input()/read call (0-based); -1 for const/envdefault
    lineno: int = 0
    secret: bool = False  # heuristic pre-check, editable during onboarding
    # Demotion signal (UX spec §0): a candidate that *parses* as a constant but whose usage
    # says "not a parameter" — currently only "accumulator" (literal init + AugAssign anywhere,
    # or reassigned inside a loop body). Demoted candidates default to unchecked at onboarding,
    # with the reason surfaced; clean candidates default to checked.
    demoted: bool = False
    demotion: str = ""  # symbolic reason id; the UI owns the human wording
    # envdefault only: the ${NAME} variable the value is read from at run time (== name; kept
    # explicit so an env-delivery caller never has to re-derive it). "" for const/input.
    env_name: str = ""


@dataclass
class Analysis:
    candidates: list[Candidate] = field(default_factory=list)
    frameworks: list[str] = field(default_factory=list)  # detected CLI frameworks
    syntax_error: bool = False
    uses_argv: bool = (
        False  # sys.argv / $1 / getopts appears -> the run form gets a passthrough hint
    )
    # Filename-looking string literals passed directly as call arguments (never bound to a
    # name): the "extract this into a named constant to manage it" hint. Capped, deduped,
    # source order. Only literals a cheap deterministic rule can vouch for — nothing else
    # (see the 'RGB' exclusion in the UX spec: no domain-knowledge guesses).
    filename_literals: list[str] = field(default_factory=list)
    # The script locates itself — shell's $0 / $BASH_SOURCE / dirname "$0", fish's
    # (status filename|dirname) — so a const rewrite that runs from a temp copy could change what
    # the script thinks its own path is. `skit params` turns this into a hint (pointing at
    # --normalize) for kinds that actually rewrite a copy; the Python analyzer never sets it.
    uses_self_location: bool = False

    @property
    def uses_cli_framework(self) -> bool:
        return bool(self.frameworks)


# ---------------------------------------------------------------- reconcile


@dataclass
class Report:
    ok: list[ParamDecl] = field(default_factory=list)
    missing: list[ParamDecl] = field(default_factory=list)
    changed: list[tuple[ParamDecl, Candidate]] = field(default_factory=list)
    rebind: list[tuple[ParamDecl, Candidate]] = field(default_factory=list)
    new: list[Candidate] = field(default_factory=list)
    syntax_error: bool = False

    @property
    def has_drift(self) -> bool:
        """Drift on the definition side (missing/changed/rebind). new is info, not drift (see
        module docstring)."""
        return bool(self.missing or self.changed or self.rebind)

    @property
    def usable(self) -> list[ParamDecl]:
        """Definitions still safe to inject (ok + changed + rebind; changed is only a type
        warning, rebind is only a positional-fallback warning -- both still inject, just flagged,
        per 3a: no silent drop, no silent wrong-value swap either)."""
        return self.ok + [spec for spec, _ in self.changed] + [spec for spec, _ in self.rebind]


def drift_lines(report: Report, name: str) -> list[str]:
    """The display lines for a drift report (shared by CLI/TUI). The only copy exit point in this
    module: plain-text old/new comparison; rich markup/color is wrapped by the caller."""
    from .i18n import gettext

    lines = [
        gettext("The parameter definitions for %(name)s have drifted from the script:")
        % {"name": name}
    ]
    # An envdefault whose ${NAME:-default} vanished or was shadowed by a plain assignment is a
    # correctness landmine (risk #1): the env value the user set would be silently ignored. It
    # gets a dedicated LOUD line, distinct from a const's ordinary "target gone".
    lines.extend(
        "  "
        + gettext(
            "%(name)s is no longer read from the environment (its ${...:-default} was removed or "
            "overridden by a plain assignment) — your value would be silently ignored. "
            "Re-add or resync."
        )
        % {"name": spec.name}
        for spec in report.missing
        if spec.binding == "envdefault"
    )
    lines.extend(
        "  "
        + gettext("%(name)s: injection target no longer exists (dropped from this run's form)")
        % {"name": spec.name}
        for spec in report.missing
        if spec.binding != "envdefault"
    )
    lines.extend(
        "  "
        + gettext(
            "%(name)s: type changed from %(old)s to %(new)s in the source (still injected — double-check the value)"
        )
        % {"name": spec.name, "old": spec.type, "new": cand.type}
        for spec, cand in report.changed
    )
    lines.extend(
        "  "
        + gettext(
            "%(name)s: its prompt no longer matches a unique input/read call; falling back to "
            "position (still injected — double-check this lands on the right question, "
            "especially if it's a secret)"
        )
        % {"name": spec.name}
        for spec, cand in report.rebind
    )
    lines.append(
        gettext("To refresh the definitions, run: skit params %(name)s --resync") % {"name": name}
    )
    return lines


def render_warning(warning: str) -> str:
    """Translate an EditResult warning ("code:name") into a user-facing line (shared by CLI/TUI).

    The codes are the closed set emitted by edit_specs; keeping the message lookup here (rather than
    a dynamic gettext(f"edit-warn-{code}")) lets Babel extract every string statically."""
    from .i18n import gettext

    code, _, name = warning.partition(":")
    return {
        "not-managed": gettext("%(name)s isn't a managed parameter; skipped."),
        "resync-dropped": gettext("Dropped %(name)s: it no longer exists in the script."),
        "already-managed": gettext("%(name)s is already managed; skipped."),
        "not-a-candidate": gettext(
            "%(name)s isn't a detectable parameter in the current script; skipped."
        ),
        "resync-skipped": gettext(
            "Could not parse the script (syntax error); resync skipped. "
            "Parameter definitions are unchanged."
        ),
        "resync-rebound": gettext(
            "%(name)s: re-anchored to its current position after its prompt stopped matching "
            "uniquely; double-check the prompt/secret assignment is still correct."
        ),
    }[code] % {"name": name}


@dataclass
class EditResult:
    specs: list[ParamDecl]
    warnings: list[str] = field(
        default_factory=list
    )  # unmatched names etc.; i18n key + value by CLI


def edit_specs(
    text: str,
    specs: list[ParamDecl],
    *,
    resync: bool = False,
    add: list[str] | tuple[str, ...] = (),
    remove: list[str] | tuple[str, ...] = (),
    secret: list[str] | tuple[str, ...] = (),
    no_secret: list[str] | tuple[str, ...] = (),
    prompts: dict[str, str] | None = None,
    analyze: Callable[[str], Analysis],
) -> EditResult:
    """Pure function: apply a set of edit operations to the existing `[tool.skit]` definitions and
    return the new definition list.

    Keys are always the **name** (const=variable name, input=input-N display name, matching
    `skit params`; an input's name is bound to its order, so it's unique for inputs too). The apply
    order is intentionally fixed: resync (prune/retype) -> remove -> add -> secret/no_secret/prompt
    (tweaks). No I/O; unmatched names are collected into warnings for the caller to render.

    `analyze` is the language's detector — the one language-specific dependency, threaded through so
    this stays neutral (the Python/shell reconcile shims bind their own).
    """
    prompts = prompts or {}
    warnings: list[str] = []
    # Shallow-copy each spec: this function claims to be pure and must never mutate the caller's
    # objects (resync changes type, tweaks change secret/prompt).
    by_name: dict[str, ParamDecl] = {s.name: replace(s) for s in specs}
    # Derive order from by_name's own (deduped) keys rather than re-deriving it from `specs`
    # directly: a corrupted/legacy definition set can contain duplicate names (analyzer used to be
    # able to emit two same-named const candidates; onboarding then wrote both), and order must
    # never contain a name absent from by_name, or `[by_name[n] for n in order]` below raises
    # KeyError once one of the two occurrences is removed (dict preserves first-occurrence
    # insertion order, so this only changes anything when specs already has duplicate names).
    order: list[str] = list(by_name)  # keep original order; new ones appended at the end

    # 1) resync: prune missing and update changed types per the current script (keeping custom
    #    secret/prompt/default).
    if resync:
        _apply_resync(text, specs, by_name, order, warnings, analyze=analyze)

    # 2) remove: explicit drop.
    for name in remove:
        if name in by_name:
            del by_name[name]
            order.remove(name)
        else:
            warnings.append(f"not-managed:{name}")

    # 3) add: bring a currently detected candidate under management (skip if already managed).
    if add:
        _apply_add(text, add, by_name, order, warnings, analyze=analyze)

    # 4) tweak secret / prompt (only for managed ones).
    _apply_tweaks(by_name, warnings, secret=secret, no_secret=no_secret, prompts=prompts)

    return EditResult(specs=[by_name[n] for n in order], warnings=warnings)


def _apply_resync(
    text: str,
    specs: list[ParamDecl],
    by_name: dict[str, ParamDecl],
    order: list[str],
    warnings: list[str],
    *,
    analyze: Callable[[str], Analysis],
) -> None:
    report = reconcile(text, specs, analyze=analyze)
    if report.syntax_error:
        # reconcile() can't tell "genuinely gone" from "the script doesn't parse right now" on its
        # own: with a syntax error, analyze() finds no candidates at all, so reconcile() marks
        # every spec missing (nothing can possibly match) even though nothing has actually changed.
        # Treating that as real drift here would prune the whole managed set, and write_params
        # drops the entire [tool.skit] block once params is empty -- a transient parse error (e.g.
        # mid-edit) would silently destroy the user's managed-parameter definitions. Leave
        # by_name/order untouched and tell the user resync didn't run instead.
        warnings.append("resync-skipped")
        return
    missing_names = {s.name for s in report.missing}
    changed_types = {spec.name: cand.type for spec, cand in report.changed}
    rebind_targets = {spec.name: cand for spec, cand in report.rebind}
    for name in list(order):
        if name in missing_names:
            warnings.append(f"resync-dropped:{name}")
            del by_name[name]
            order.remove(name)
        elif name in changed_types:
            # changed_types[name] is a Candidate.type (a plain str); ParamDecl.type is the
            # closed ParamType literal, so re-anchor the type through dataclasses.replace
            # (whose kwargs are untyped) rather than a direct attribute write. by_name[name]
            # is already a private shallow copy (see the replace() at the top of edit_specs),
            # so swapping in a fresh copy is the same visible effect as mutating in place.
            by_name[name] = replace(by_name[name], type=changed_types[name])
        elif name in rebind_targets:
            # The prompt no longer uniquely resolves; re-anchor to whichever call site position
            # currently supplied it, so the *next* run's plain reconcile() (no --resync) sees an
            # exact prompt match again instead of re-deriving the same fallback every time.
            cand = rebind_targets[name]
            warnings.append(f"resync-rebound:{name}")
            by_name[name].order = cand.order
            by_name[name].prompt = cand.prompt


def _apply_add(
    text: str,
    add: list[str] | tuple[str, ...],
    by_name: dict[str, ParamDecl],
    order: list[str],
    warnings: list[str],
    *,
    analyze: Callable[[str], Analysis],
) -> None:
    candidates = {c.name: c for c in analyze(text).candidates}
    for name in add:
        if name in by_name:
            warnings.append(f"already-managed:{name}")
        elif name in candidates:
            by_name[name] = ParamDecl.from_candidate(candidates[name])
            order.append(name)
        else:
            warnings.append(f"not-a-candidate:{name}")


def _apply_tweaks(
    by_name: dict[str, ParamDecl],
    warnings: list[str],
    *,
    secret: list[str] | tuple[str, ...],
    no_secret: list[str] | tuple[str, ...],
    prompts: dict[str, str],
) -> None:
    for name in secret:
        if name in by_name:
            by_name[name].secret = True
        else:
            warnings.append(f"not-managed:{name}")
    for name in no_secret:
        if name in by_name:
            by_name[name].secret = False
            by_name[name].env_source = ""  # an env source only means anything on a secret
        else:
            warnings.append(f"not-managed:{name}")
    for name, prompt in prompts.items():
        if name in by_name:
            by_name[name].prompt = prompt
        else:
            warnings.append(f"not-managed:{name}")


def _type_matches(spec: ParamDecl, cand: Candidate) -> bool:
    """Whether a stored const type still matches what the source derives. A declared
    ``path`` over a derived ``str`` is a user refinement — a string constant is exactly
    how a path lives in source — not drift; every other mismatch is real. Resync
    preservation falls out for free: a compatible pair never enters ``report.changed``,
    so ``_apply_resync`` never rewrites the declared type (docs/design/path.md)."""
    return cand.type == spec.type or (spec.type == "path" and cand.type == "str")


def reconcile(  # noqa: PLR0912 — one branch per binding and drift category; a flat dispatch
    text: str, specs: list[ParamDecl], *, analyze: Callable[[str], Analysis]
) -> Report:
    """Reconcile the definitions with the script's current content. On a syntax error, mark
    everything missing (nothing matches).

    Match keys by binding: const/envdefault by **variable name**, input by **prompt then order**
    (callmatch). An envdefault matches whenever its name is still read from the environment
    (default-text/type changes are fine — the value arrives by env either way); it goes missing
    only when the analyzer no longer reports it (the ${NAME:-...} was deleted, or the name became a
    plain top-level assignment that would shadow the env value — the analyzer's suppression rule)."""
    analysis = analyze(text)
    if analysis.syntax_error:
        return Report(missing=list(specs), syntax_error=True)

    consts = {c.name: c for c in analysis.candidates if c.binding == "const"}
    envdefaults = {c.name: c for c in analysis.candidates if c.binding == "envdefault"}
    inputs = {c.order: c for c in analysis.candidates if c.binding == "input"}
    stored_inputs = [(s.order, s.prompt) for s in specs if s.binding == "input"]
    current_inputs = [(c.order, c.prompt) for c in analysis.candidates if c.binding == "input"]
    input_bindings = match_calls(stored_inputs, current_inputs)

    report = Report()
    covered_consts: set[str] = set()
    covered_inputs: set[int] = set()
    covered_envs: set[str] = set()

    for spec in specs:
        if spec.binding == "input":
            binding = input_bindings.get(spec.order)
            if binding is None:
                report.missing.append(spec)
                continue
            resolved_order, ambiguous = binding
            cand = inputs[resolved_order]
            covered_inputs.add(resolved_order)
            if ambiguous:
                report.rebind.append((spec, cand))
            else:
                report.ok.append(spec)
            continue
        if spec.binding == "envdefault":
            cand = envdefaults.get(spec.name)
            if cand is None:
                report.missing.append(spec)
            else:
                covered_envs.add(spec.name)
                report.ok.append(spec)  # default-text/type changes stay ok (env delivery)
            continue
        cand = consts.get(spec.name)
        if cand is None:
            report.missing.append(spec)
        elif not _type_matches(spec, cand):
            covered_consts.add(spec.name)
            report.changed.append((spec, cand))
        else:
            covered_consts.add(spec.name)
            report.ok.append(spec)

    for cand in analysis.candidates:
        if (
            (cand.binding == "const" and cand.name not in covered_consts)
            or (cand.binding == "input" and cand.order not in covered_inputs)
            or (cand.binding == "envdefault" and cand.name not in covered_envs)
        ):
            report.new.append(cand)
    return report
