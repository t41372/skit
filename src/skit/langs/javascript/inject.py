"""JS/TS injector: run-time value delivery for JavaScript & TypeScript const parameters.

JS candidates are const-only — there is no environment-default idiom and no interactive-read
equivalent in scope — so this injector has exactly one delivery: **rewrite the const's value node on
a temp copy**. It mirrors `langs/shell/inject.py`'s discipline (InjectError vs InjectValueError, a
mandatory offline re-parse gate, 0600 temp file) without shell's env/read machinery.

- **const → byte-span rewrite of every same-name top-level declaration's value node.** Quoting is
  normalized to the value's declared type: an int/float becomes a bare number, a bool becomes
  `true`/`false`, and a string becomes `json.dumps(value)` — a JS string literal is a strict superset
  of a JSON string, so json.dumps produces valid JS with quotes, backslashes, newlines and non-ASCII
  all correctly escaped. Every occurrence of the name is rewritten (mirrors python/shell), so the
  binding holds the injected value however the module ends up evaluating.

**Dual syntax gates** run before anything launches:

1. **Mandatory, offline** — a tree-sitter re-parse of the injected text (`has_error` ⇒
   InjectSyntaxError, temp copy removed). This is the guarantee: skit never launches text it
   corrupted, on any platform, with or without a runtime installed.
2. **Best-effort hardening** — `node --check <tmp>`, but ONLY when the resolvable runner is `node`
   and the temp copy is a `.js`/`.mjs` file. `node --check` does not accept a `.ts` file, and a
   deno/bun runner has no equivalently cheap parse-only check — so for those the offline gate stands
   alone (it is the mandatory guarantee, so nothing is lost). The gate does not consult
   `config.js.runner`; it is deliberately dependency-light, because gate 1 already vouched for the
   text.
"""

from __future__ import annotations

import json
import shutil
import subprocess
from typing import TYPE_CHECKING

from tree_sitter import Parser

from ...i18n import gettext
from ...params import coerce_default
from ...rewrite import ByteSpan, apply_byte_spans, write_injected
from ..base import (
    InjectError,
    InjectRequest,
    InjectResult,
    InjectSyntaxError,
    InjectValueError,
)
from .analyzer import _literal_value, _text, _toplevel_declarations, language_for

if TYPE_CHECKING:
    from pathlib import Path

    from tree_sitter import Node

# The temp-copy extension for each kind. It matches the stored copy's extension (js entries are
# stored as script.js, ts as script.ts), so the injected copy runs through the exact launch path the
# stored file would — the same runner, the same module resolution.
_SUFFIX = {"js": ".js", "ts": ".ts", "tsx": ".tsx"}

# Only these temp-copy extensions are eligible for gate 2 (`node --check` rejects .ts/.tsx).
_NODE_CHECK_SUFFIXES = (".js", ".mjs")

# The runner resolution order (deno > bun > node), mirroring launch.RunnerLaunch. Used only to
# decide whether gate 2 (`node --check`) applies — a non-node runner skips it.
_RUNNER_ORDER = ("deno", "bun", "node")

# The `--check` gate is parse-only, so it is fast; the timeout is a liveness guard, not a policy.
_GATE_TIMEOUT = 30.0


def inject(request: InjectRequest, *, lang: str = "js") -> InjectResult:
    """Deliver `request.values` for a JS/TS entry by rewriting a temp copy.

    Returns the injected temp copy (or None when no value was supplied — the original file runs),
    an always-empty environment overlay (JS has no env delivery in scope), and no warnings.

    Raises `InjectValueError` when a value doesn't fit its declared int/float/bool type,
    `InjectError` when an injection target no longer exists (drift), and `InjectSyntaxError` when a
    post-injection syntax gate rejects the result (nothing is launched; the temp copy is removed).
    """
    text = request.text
    root = _root(text, lang)
    spans: list[ByteSpan] = []
    missing: list[str] = []

    for spec in request.specs:
        if spec.name not in request.values:
            continue  # no value received: leave it alone (preserve the script's own behavior)
        targets = _const_targets(root, spec.name)
        if not targets:
            missing.append(spec.name)
            continue
        literal = _const_literal(request.values[spec.name], spec.type, spec.name)
        spans.extend(ByteSpan(node.start_byte, node.end_byte, literal) for node in targets)

    if missing:
        raise InjectError(", ".join(missing))
    if not spans:
        # Nothing to rewrite (no values, or none matched a supplied value): no temp copy is written
        # at all — the run launches the original file.
        return InjectResult()

    out = apply_byte_spans(text, spans)
    _gate_reparse(out, lang)  # gate 1 (mandatory, offline): never launch text we corrupted
    suffix = _SUFFIX.get(lang, ".js")
    path = write_injected(request.entry_dir, out, suffix=suffix)
    try:
        _gate_node(request.interpreter, path, suffix)  # gate 2 (hardening): `node --check`
    except BaseException:
        path.unlink(
            missing_ok=True
        )  # a refused copy must never be left behind (it may hold secrets)
        raise
    return InjectResult(path=path)


# ---------------------------------------------------------------- parsing


def _root(text: str, lang: str) -> Node:
    return Parser(language_for(lang)).parse(text.encode("utf-8")).root_node


# ---------------------------------------------------------------- const


def _const_targets(root: Node, name: str) -> list[Node]:
    """The value nodes of every top-level literal declaration of `name` — the exact set the analyzer
    counted as this const's candidate (same `_literal_value` predicate, same plain-identifier rule),
    so what a form offers is precisely what gets rewritten. `let`/`var` bindings are targets too:
    the analyzer offers them (demoted), and once a value is supplied they must actually be injected."""
    out: list[Node] = []
    for declarator, _keyword in _toplevel_declarations(root):
        name_node = declarator.child_by_field_name("name")
        value_node = declarator.child_by_field_name("value")
        if name_node is None or name_node.type != "identifier" or value_node is None:
            continue
        if _text(name_node) != name:
            continue
        if _literal_value(value_node) is not None:
            out.append(value_node)
    return out


def _const_literal(raw: str, type_name: str, param_name: str) -> str:
    """The source text to splice in place of a const's value, quoting NORMALIZED to the declared
    type: an int/float coerces to a bare JS number, a bool to `true`/`false`, and everything else to
    a `json.dumps` string literal (a JS string is a superset of a JSON string, so quotes/backslashes/
    newlines/non-ASCII are all handled). A value that doesn't fit its int/float/bool type raises
    InjectValueError — a bad input, never drift.

    `coerce_default` already refuses a non-finite float (`Infinity`/`NaN`) by raising ValueError, so
    a bare `repr(typed)` here is always a finite JS number literal — exactly like python's shim and
    shell's injector, which reject the same non-finite values."""
    if type_name in ("int", "float", "bool"):
        try:
            typed = coerce_default(raw, type_name)
        except ValueError as exc:
            raise InjectValueError(raw, type_name, param_name) from exc
        if isinstance(typed, bool):  # checked before int: bool is an int subclass
            return "true" if typed else "false"
        return repr(typed)
    return escape_string(raw)


def escape_string(value: str) -> str:
    """A JS string literal for a value. A JS string literal is a strict superset of a JSON string, so
    json.dumps produces valid, correctly-escaped JS — quotes, backslashes, newlines and non-ASCII all
    handled. The one escaper (the single place quoting can be right or wrong, and the seam a test
    monkeypatches to prove the syntax gate bites)."""
    return json.dumps(value)


# ---------------------------------------------------------------- gates


def _gate_reparse(out: str, lang: str) -> None:
    """Gate 1 (mandatory, offline): the injected text must still parse. This is what stands between a
    quoting bug and a corrupted script running with the user's real values."""
    if _root(out, lang).has_error:
        raise InjectSyntaxError(
            gettext(
                "the injected copy no longer parses as a JavaScript/TypeScript script (nothing was run)"
            )
        )


def _resolve_runner(interpreter: str) -> tuple[str | None, str | None]:
    """(runner name, absolute path) of the first installed runner — the recorded interpreter if the
    entry pins one, else deno > bun > node (mirrors launch.RunnerLaunch). (None, None) when nothing
    is installed. Used only to decide whether gate 2 applies."""
    candidates = (interpreter,) if interpreter else _RUNNER_ORDER
    for name in candidates:
        found = shutil.which(name)
        if found:
            return name.rsplit("/", 1)[-1].removesuffix(".exe"), found
    return None, None


def _gate_node(interpreter: str, path: Path, suffix: str) -> None:
    """Gate 2 (hardening): `node --check <file>` parses without executing. Applies ONLY when the temp
    copy is a .js/.mjs file AND the resolvable runner is node — node can't `--check` a .ts file, and
    deno/bun have no equivalently cheap parse-only check, so those rely on gate 1 (the mandatory
    guarantee). A missing/failed spawn never fails the run: gate 1 already vouched for the text."""
    if suffix not in _NODE_CHECK_SUFFIXES:
        return
    name, program = _resolve_runner(interpreter)
    if program is None or name != "node":
        return
    try:
        proc = subprocess.run(  # noqa: S603 — argv list, node resolved from PATH; `--check` never executes the script
            [program, "--check", str(path)],
            capture_output=True,
            check=False,
            timeout=_GATE_TIMEOUT,
        )
    except (OSError, subprocess.SubprocessError):
        return  # the gate itself couldn't run; gate 1 already vouched for the text
    if proc.returncode != 0:
        detail = proc.stderr.decode("utf-8", errors="replace").strip().splitlines()
        raise InjectSyntaxError(
            gettext("node rejected the injected copy: %(detail)s")
            % {"detail": detail[0] if detail else ""}
        )
