"""Shim: parameter injection at run time (AST location + precise source substitution).

Shares one candidate-decision set with analyzer (the reason A2 exists):
- const: literal assignments at module top level / main-guard top level, keyed by variable name,
  with the RHS span replaced in-source.
- input: every input() call in the file, keyed by order of appearance (B1), matched to a stored
  value by prompt text first and position only as a fallback (3a, `callmatch.match_calls` — shared
  with reconcile so both agree on the same call site). Each matched call site is rewritten
  in-source, exactly like a const's RHS, from `input(...)` to `_skit_i[K](...)`, where `_skit_i[K]`
  is a one-shot wrapper defined by a single-line preamble: it echoes "prompt + value" (masked to
  *** when secret) and hands back the queued value on its first invocation, then falls through to
  the real, unpatched `input` on every invocation after that — so input() inside loops, or called
  more times than there are form values, still behaves correctly. Binding the value to the call
  *site* rather than a global runtime call counter is what makes this correct even when a script's
  *runtime* execution order differs from its *static* source order (e.g. a function's input() is
  defined above a top-level input() but only invoked after it runs).

The substitution strategy is "locate via AST, replace as text": only the source span of a value node
(or, for input, the `input` callee name at a specific call site) changes; every other byte (PEP 723
/ [tool.skit] block, comments, layout) is left untouched. The preamble is a **single physical line**
inserted after the docstring / __future__ imports (preserving `__doc__` semantics and avoiding a
__future__ syntax error, B4), for a fixed line-number shift of 1. The injected result is written to
a temp file (the OS temp directory, not next to the script copy — a crash must never leave a
plaintext-secret file behind, 3b) and run from there; the copy itself is never modified (A5).
"""

from __future__ import annotations

import ast
import math
from dataclasses import dataclass

from ...callmatch import match_calls
from ...params import ParamDecl
from ...rewrite import ByteSpan, apply_byte_spans, line_start_table, linecol_to_byte, write_injected
from ..base import InjectError, InjectRequest, InjectResult, InjectValueError
from .analyzer import _is_main_guard, _literal_prompt, _literal_value

# The historical names, kept as aliases: the exception family moved to langs/base.py when shell
# grew a second injector (flows maps them for EVERY language, so they can't live in python's
# package), but `shim.ShimError` / `shim.ShimValueError` are a stable surface — tests and callers
# spell them that way, and `except ShimError` must keep catching a shell injector's drift error.
ShimError = InjectError
ShimValueError = InjectValueError


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


def _coerce(value: str, type_name: str, param_name: str) -> str | int | float | bool:
    """Convert the string from the form back to the defined type. If it can't convert, raise
    ShimValueError (explicit error; never silently inject a broken value) — a ShimError subclass so
    existing `except ShimError` callers keep working, but distinguishable from a missing-target
    ShimError by callers that want to give a value-specific message instead of a drift warning."""
    converters = {"int": int, "float": _coerce_float, "bool": _coerce_bool}
    conv = converters.get(type_name)
    if conv is None:
        return value
    try:
        return conv(value)
    except ValueError as exc:
        raise ShimValueError(value, type_name, param_name) from exc


def _const_targets(body: list[ast.stmt], name: str) -> list[ast.expr]:
    """In a block of top-level statements, the RHS nodes of literal assignments named `name`."""
    out: list[ast.expr] = []
    for stmt in body:
        target: ast.expr | None = None  # pragma: no mutate
        value: ast.expr | None = None  # pragma: no mutate
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


def _build_preamble(queue: dict[int, tuple[str, bool]]) -> str:
    """A single physical line defining one **per-call-site** one-shot override per queued input (3a).

    Each managed input() call is rewritten in-source (see `_node_replacement` on its `.func`) from
    `input(...)` to `_skit_i[K](...)`, where K is that call's resolved position in the CURRENT
    source. `_skit_i[K]` is a one-shot wrapper: on its first invocation it echoes "prompt + value"
    (masked to *** when secret, matching real terminal echo) and pops its queue entry; every
    invocation after that (loops, dynamic call counts) falls through to the real, unpatched `input`
    — same fallback contract the old global-counter design had. The difference is *what* the value
    is bound to: a specific call site instead of "the Nth call at runtime" — so a script whose
    *runtime* call order differs from its *static* source order (e.g. a function's input() is
    defined above a top-level input() but only invoked after it) can no longer swap values, because
    there is no shared counter left to race.
    """
    keys = sorted(queue)
    return (
        "import sys as _skit_s; _skit_o = input; _skit_q = "
        + repr(queue)
        + "; _skit_i = {k: (lambda p='', /, k=k: ("
        "(_skit_s.stdout.write(str(p) + ('***' if _skit_q[k][1] else _skit_q[k][0]) + chr(10)), "
        "_skit_q.pop(k)[0])[1] if k in _skit_q else _skit_o(p))) for k in "
        + repr(keys)
        + "}  # skit:shim\n"
    )


def _preamble_line_index(tree: ast.Module) -> int:
    """The 0-based line index where the preamble should be inserted.

    Skips the docstring and __future__ imports (B4: preserve `__doc__` semantics; __future__ must be
    the first non-docstring statement, so inserting before it would be a SyntaxError). Callers must
    only call this when the module has at least one top-level statement after that preamble.
    """
    body = tree.body
    i = 0  # pragma: no mutate — only used as a slice bound; body[None:] == body[0:]
    if (
        body
        and isinstance(body[0], ast.Expr)
        and isinstance(body[0].value, ast.Constant)
        and isinstance(body[0].value.value, str)
    ):
        i = 1
    for node in body[i:]:
        if not (isinstance(node, ast.ImportFrom) and node.module == "__future__"):
            break
    else:  # pragma: no cover — callers only invoke this when a stmt follows the preamble
        raise AssertionError("unreachable: caller guarantees a stmt follows the preamble")
    stmt = node
    # Decorators sit above the def/class lineno, so the insertion point must take the topmost one.
    decorators = getattr(stmt, "decorator_list", None) or []
    lineno = min([stmt.lineno, *(d.lineno for d in decorators)])
    return lineno - 1


def _insert_preamble(text: str, tree: ast.Module, preamble: str) -> str:
    idx = _preamble_line_index(tree)
    # A zero-width ByteSpan insert at the target line's start byte offset — byte-identical to
    # splicing the preamble line in via _physical_lines, but through the one shared rewrite core.
    offset = line_start_table(text)[idx]
    return apply_byte_spans(text, [ByteSpan(offset, offset, preamble)])


def _node_replacement(node: ast.expr, new_text: str) -> _Replacement:
    if node.end_lineno is None or node.end_col_offset is None:  # pragma: no cover
        raise ShimError("missing node span")
    return _Replacement(
        node.lineno, node.col_offset, node.end_lineno, node.end_col_offset, new_text
    )


def inject_entry(request: InjectRequest) -> InjectResult:
    """The Injector capability (registry-wired). Python delivers every managed value by
    rewriting a temp copy — no environment channel, no warnings — so the result is exactly the
    temp path `inject()` produced, and `flows.execute`'s behavior is byte-identical to the
    hardcoded call it replaced. `inject` is looked up through the module globals on every call
    (not captured at registry-build time), so monkeypatching `shim.inject` still works."""
    return InjectResult(
        path=write_injected(
            # entry_dir is write_injected's fallback directory (used only when the OS temp dir
            # isn't writable); suffix=".py" keeps the extension `uv run --script` expects.
            request.entry_dir,
            inject(request.text, request.specs, request.values),
            suffix=".py",
        )
    )


def inject(text: str, specs: list[ParamDecl], values: dict[str, str]) -> str:
    """Return the full injected script text. A parameter whose target can't be found means the
    definition has drifted, so raise ShimError (tell the user to re-add explicitly; never silently
    run stale values). A parameter whose target IS found but whose value doesn't fit the declared
    type raises the ShimValueError subclass instead (via `_coerce`) — that's a bad input, not
    drift, so callers should not react to it with re-add/drift wording."""
    try:
        tree = ast.parse(text)
    except SyntaxError as exc:
        raise ShimError(str(exc)) from exc

    replacements: list[_Replacement] = []
    missing: list[str] = []
    input_calls = _input_calls(tree)
    current_inputs = [(i, _literal_prompt(c)) for i, c in enumerate(input_calls)]
    stored_inputs = [
        (spec.order, spec.prompt)
        for spec in specs
        if spec.binding == "input" and spec.name in values
    ]
    input_bindings = match_calls(stored_inputs, current_inputs)
    queue: dict[int, tuple[str, bool]] = {}

    for spec in specs:
        if spec.name not in values:
            continue  # no value received, leave it alone (preserve the script's original behavior)
        raw = values[spec.name]
        if spec.binding == "input":
            # Resolve the call site the same way reconcile does (3a, shared via
            # callmatch.match_calls): prefer the stored prompt text over bare position, so a
            # source edit that inserts/removes an earlier input() can't silently rebind this value
            # onto the wrong question. No match at all (position gone too) = definition drift;
            # error explicitly rather than dropping the value into a black hole.
            binding = input_bindings.get(spec.order)
            if binding is None:
                missing.append(spec.name)
                continue
            resolved_order, _ambiguous = binding
            if resolved_order in queue:
                # Defense-in-depth: match_calls is meant to be strictly 1:1 (a claim-aware exact
                # pass, 3a-fix), but this loop keys off `spec.order`, not identity -- two ParamDecl
                # entries that happen to carry the same `order` (a hand-edited or otherwise
                # corrupted [tool.skit] block) look up the *same* binding and would otherwise both
                # queue a replacement over the identical `input` callee span. apply_byte_spans'
                # non-overlap contract can't survive that: two replacements at the same span corrupt
                # the injected copy into unparsable text (e.g. `_skit_i[0]_i[0](...)`). Never let a
                # second claimant reach apply_byte_spans; report it as drift instead, same as any
                # other target that can't be found.
                missing.append(spec.name)
                continue
            queue[resolved_order] = (raw, spec.secret)
            replacements.append(
                _node_replacement(input_calls[resolved_order].func, f"_skit_i[{resolved_order}]")
            )
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
        typed = _coerce(raw, spec.type, spec.name)
        replacements.extend(_node_replacement(n, repr(typed)) for n in nodes)

    if missing:
        raise ShimError(", ".join(missing))
    # Convert each ast-located (line/col) replacement into an absolute-byte ByteSpan and splice them
    # through the shared rewrite core. ast col_offsets are already UTF-8 byte offsets within their
    # line, so linecol_to_byte only adds the line's start offset (the multibyte-safe path).
    table = line_start_table(text)
    spans = [
        ByteSpan(
            linecol_to_byte(table, r.lineno, r.col),
            linecol_to_byte(table, r.end_lineno, r.end_col),
            r.new_text,
        )
        for r in replacements
    ]
    out = apply_byte_spans(text, spans)
    if queue:
        # Apply span replacements before inserting the line: every const/input replacement site is
        # after the insertion point (a top-level assignment can't precede docstring/__future__, and
        # any statement containing a queued input() call — or one of its ancestors — is itself a
        # top-level statement that also follows it), so lines above the insertion point are
        # unaffected and the index stays valid.
        preamble = _build_preamble(queue)
        # Invariant: queue is non-empty only if an input() call was queued, which means at least one
        # top-level statement (the one containing that call, or an ancestor of it) exists after the
        # docstring/__future__ preamble; _preamble_line_index always finds it.
        out = _insert_preamble(out, tree, preamble)
    return out
