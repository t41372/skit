"""Shim: parameter injection at run time (AST location + precise source substitution).

Shares one candidate-decision set with analyzer (the reason A2 exists):
- const: literal assignments at module top level / main-guard top level, keyed by variable name,
  with the RHS span replaced in-source.
- input: every input() call in the file, keyed by order of appearance (B1). From Phase 3 on this
  became an **interception queue**: inject a single-line preamble that overrides builtins.input and
  consumes form values in call order; when the queue is exhausted (or that position has no value) it
  restores/falls back to native stdin pass-through. This keeps input() inside loops, and a dynamic
  call count > the number of form values, behaving correctly.

The substitution strategy is "locate via AST, replace as text": only the source span of a value node
changes; every other byte (PEP 723 / [tool.skit] block, comments, layout) is left untouched. The
preamble is a **single physical line** inserted after the docstring / __future__ imports (preserving
`__doc__` semantics and avoiding a __future__ syntax error, B4), for a fixed line-number shift of 1.
The injected result is written to a temp file next to the script copy and run from there; the copy
itself is never modified (A5).
"""

from __future__ import annotations

import ast
import math
import os
import tempfile
from dataclasses import dataclass
from pathlib import Path

from .analyzer import _is_main_guard, _literal_value
from .metawriter import ParamSpec


class ShimError(Exception):
    pass


@dataclass
class _Replacement:
    lineno: int  # 1-based
    col: int
    end_lineno: int
    end_col: int
    new_text: str


def _coerce_float(value: str) -> float:
    f = float(value)
    # repr(inf/nan) is not a valid Python literal (X = inf -> NameError), so reject explicitly.
    if math.isnan(f) or math.isinf(f):
        raise ValueError(value)
    return f


def _coerce_bool(value: str) -> bool:
    low = value.strip().lower()
    if low in ("true", "1", "yes", "y", "on"):
        return True
    if low in ("false", "0", "no", "n", "off"):
        return False
    raise ValueError(value)


def _coerce(value: str, type_name: str) -> str | int | float | bool:
    """Convert the string from the form back to the defined type. If it can't convert, raise
    ShimError (explicit error; never silently inject a broken value)."""
    converters = {"int": int, "float": _coerce_float, "bool": _coerce_bool}
    conv = converters.get(type_name)
    if conv is None:
        return value
    try:
        return conv(value)
    except ValueError as exc:
        raise ShimError(f"{value!r} -> {type_name}") from exc


def _const_targets(body: list[ast.stmt], name: str) -> list[ast.expr]:
    """In a block of top-level statements, the RHS nodes of literal assignments named `name`."""
    out: list[ast.expr] = []
    for stmt in body:
        target: ast.expr | None = None
        value: ast.expr | None = None
        if isinstance(stmt, ast.Assign) and len(stmt.targets) == 1:
            target, value = stmt.targets[0], stmt.value
        elif isinstance(stmt, ast.AnnAssign) and stmt.value is not None:
            target, value = stmt.target, stmt.value
        if isinstance(target, ast.Name) and target.id == name and value is not None:
            ok, _ = _literal_value(value)
            if ok:
                out.append(value)
    return out


def _input_calls(tree: ast.Module) -> list[ast.Call]:
    calls = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Name)
        and node.func.id == "input"
    ]
    calls.sort(key=lambda c: (c.lineno, c.col_offset))
    return calls


# A single physical line. Behavior: on the i-th input() call, if the queue holds a value at
# position i, yield it and echo "prompt + value" (secret positions echo ***) to mimic terminal
# interaction; otherwise call native input (stdin pass-through). The queue is keyed by call
# position, not "first
# N", so input() in loops and dynamic call counts all behave correctly.
_PREAMBLE = (
    "import builtins as _skit_b, itertools as _skit_i, sys as _skit_s; "
    "_skit_q = {queue}; _skit_m = {masked}; _skit_c = _skit_i.count(); _skit_o = _skit_b.input; "
    "_skit_b.input = lambda p='', /: (lambda i: "
    "((_skit_s.stdout.write(str(p) + ('***' if i in _skit_m else _skit_q[i]) + chr(10)),"
    " _skit_q[i])[1] "
    "if i in _skit_q else _skit_o(p)))(next(_skit_c))  # skit:shim\n"
)


def _preamble_line_index(tree: ast.Module) -> int | None:
    """The 0-based line index where the preamble should be inserted; None means append at EOF.

    Skips the docstring and __future__ imports (B4: preserve `__doc__` semantics; __future__ must be
    the first non-docstring statement, so inserting before it would be a SyntaxError).
    """
    body = tree.body
    i = 0
    if (
        body
        and isinstance(body[0], ast.Expr)
        and isinstance(body[0].value, ast.Constant)
        and isinstance(body[0].value.value, str)
    ):
        i = 1
    while i < len(body):
        node = body[i]
        if not (isinstance(node, ast.ImportFrom) and node.module == "__future__"):
            break
        i += 1
    if i >= len(body):
        return None
    stmt = body[i]
    # Decorators sit above the def/class lineno, so the insertion point must take the topmost one.
    decorators = getattr(stmt, "decorator_list", None) or []
    lineno = min([stmt.lineno, *(d.lineno for d in decorators)])
    return lineno - 1


def _insert_preamble(text: str, tree: ast.Module, preamble: str) -> str:
    idx = _preamble_line_index(tree)
    lines = text.splitlines(keepends=True)
    if idx is None:
        if lines and not lines[-1].endswith("\n"):
            lines[-1] += "\n"
        return "".join(lines) + preamble
    return "".join([*lines[:idx], preamble, *lines[idx:]])


def _node_replacement(node: ast.expr, new_text: str) -> _Replacement:
    if node.end_lineno is None or node.end_col_offset is None:  # pragma: no cover
        raise ShimError("missing node span")
    return _Replacement(
        node.lineno, node.col_offset, node.end_lineno, node.end_col_offset, new_text
    )


def _apply(text: str, replacements: list[_Replacement]) -> str:
    """Apply replacements bottom-up to avoid span shifts. Spans are guaranteed non-overlapping: a
    const target's RHS must be a literal (same decision as analyzer), so it cannot contain an
    input() call.

    Note: ast's col_offset / end_col_offset are **UTF-8 byte** offsets, not character offsets.
    When a line contains multibyte characters (e.g. CJK), slicing the str directly misaligns; we
    must slice at the byte level and decode (a real bug caught by corpus 17_unicode_cjk).
    """
    lines = text.splitlines(keepends=True)
    for r in sorted(replacements, key=lambda r: (r.lineno, r.col), reverse=True):
        new_bytes = r.new_text.encode("utf-8")
        if r.lineno == r.end_lineno:
            line = lines[r.lineno - 1].encode("utf-8")
            lines[r.lineno - 1] = (line[: r.col] + new_bytes + line[r.end_col :]).decode("utf-8")
        else:
            first = lines[r.lineno - 1].encode("utf-8")
            last = lines[r.end_lineno - 1].encode("utf-8")
            merged = (first[: r.col] + new_bytes + last[r.end_col :]).decode("utf-8")
            lines[r.lineno - 1 : r.end_lineno] = [merged]
    return "".join(lines)


def write_injected(entry_dir: Path, content: str) -> Path:
    """Write the injected result to a unique temp file under entry_dir and return its path.

    - Unique filename (.injected-XXXX.py): concurrent runs of the same script don't clobber.
    - 0o600 permissions: the content may contain secret values (const substitutions / queue
      literals), so don't let other local users read it (an extension of C3; the caller must still
      delete the file in a finally).
    """
    fd, tmp = tempfile.mkstemp(dir=entry_dir, prefix=".injected-", suffix=".py")
    try:
        os.chmod(
            tmp, 0o600
        )  # mkstemp is already 0600; state the intent explicitly (no-op on Windows)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(content)
    except BaseException:
        os.unlink(tmp)
        raise
    return Path(tmp)


def inject(text: str, specs: list[ParamSpec], values: dict[str, str]) -> str:
    """Return the full injected script text. A parameter whose target can't be found means the
    definition has drifted, so raise ShimError (tell the user to re-add explicitly; never silently
    run stale values)."""
    try:
        tree = ast.parse(text)
    except SyntaxError as exc:
        raise ShimError(str(exc)) from exc

    replacements: list[_Replacement] = []
    missing: list[str] = []
    input_calls = _input_calls(tree)
    queue: dict[int, str] = {}
    masked: set[int] = set()

    for spec in specs:
        if spec.name not in values:
            continue  # no value received, leave it alone (preserve the script's original behavior)
        raw = values[spec.name]
        if spec.kind == "input":
            # Queue injection (don't rewrite the source input() calls). An order beyond the current
            # script's input() count = definition drift; error explicitly rather than dropping the
            # value into a black hole.
            if 0 <= spec.order < len(input_calls):
                queue[spec.order] = raw
                if spec.secret:
                    masked.add(spec.order)
            else:
                missing.append(spec.name)
            continue
        # const: module top level + main-guard top level; replace every same-name occurrence (both
        # the top-level definition and a guard-body override should take the new value).
        nodes = _const_targets(tree.body, spec.name)
        for stmt in tree.body:
            if _is_main_guard(stmt):
                nodes += _const_targets(stmt.body, spec.name)
        if not nodes:
            missing.append(spec.name)
            continue
        typed = _coerce(raw, spec.type)
        replacements.extend(_node_replacement(n, repr(typed)) for n in nodes)

    if missing:
        raise ShimError(", ".join(missing))
    out = _apply(text, replacements)
    if queue:
        # Apply span replacements before inserting the line: const replacements are all after the
        # insertion point (a top-level assignment can't precede docstring/__future__), so lines
        # above the insertion point are unaffected and the index stays valid.
        preamble = _PREAMBLE.format(queue=repr(queue), masked=repr(masked) if masked else "set()")
        out = _insert_preamble(out, tree, preamble)
    return out
