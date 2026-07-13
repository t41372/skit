"""Idiom normalization (the approved A5 amendment): `VAR=<literal>` → `VAR="${VAR:-<literal>}"`.

Opt-in, explicit, and permanent: it rewrites the user's STORED COPY (never a reference-mode
original) so a parameter that could only be delivered by rewriting a temp copy becomes one skit can
deliver through the environment — zero rewrite from then on, forever, and `$0` keeps pointing at the
real file. The script keeps running standalone exactly as before (the default is still the literal
it always had); it simply now honours an inherited value first, which is the canonical shell idiom.

The rewrite is byte-minimal: only that one assignment's VALUE span changes.

**Refusals** (reported, never silently skipped — the source is left untouched):

- ``unsafe-literal`` — the literal contains one of `` } " ` $ \\ `` or a newline. Inside the
  `"${VAR:-…}"` form those characters are no longer inert: `}` would close the expansion early, `"`
  would close the quote, and `` ` ``/`$`/`\\` reintroduce exactly the expansion the original literal
  did not have. There is no safe escaping of `}` inside `${…}` in POSIX sh, so the honest move is to
  refuse and keep injecting that one by temp copy.
- ``readonly`` — a `readonly` / `declare -r` const cannot be reassigned at all.
- ``multiple-assignments`` — the name is assigned more than once at top level. Normalizing them
  would invert last-write-wins (the first `${VAR:-…}` would win over every later assignment), i.e.
  silently change what the script computes.
- ``already-env`` — the assignment already reads its own name (it IS the idiom).
- ``not-a-const`` — no top-level literal assignment by that name.
- ``syntax-error`` — the script doesn't parse; nothing is rewritten.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from tree_sitter import Parser

from ...rewrite import ByteSpan, apply_byte_spans
from ..base import Normalization
from .analyzer import (
    _LANGUAGE,
    _assignment_operator,
    _literal_text,
    _references,
    _text,
    _toplevel_assignments,
)

if TYPE_CHECKING:
    from tree_sitter import Node

# Characters that stop being inert once the literal is re-homed inside `"${NAME:-…}"`. Two groups:
#   - expansion/quoting: } " ` $ \ and newline break out of the ${:-} or the surrounding "…".
#   - shell metacharacters: ; | & ( ) < > are harmless inside the single-quoted source literal (so
#     the analyzer offers the const), but tree-sitter-bash cannot parse them inside the expansion
#     body — and since analyze() degrades the WHOLE file to syntax_error on any parse error, a
#     normalize that slipped one of these through would silently drop EVERY parameter on the entry
#     while reporting success. Refuse them, so the const keeps being delivered by temp copy instead.
_UNSAFE = ("}", '"', "`", "$", "\\", "\n", ";", "|", "&", "(", ")", "<", ">")


def normalize_idiom(text: str, names: list[str]) -> Normalization:
    """Rewrite each named top-level const into the `${NAME:-default}` env-default idiom.

    Pure: returns the new text plus the names rewritten and the coded refusals; the caller decides
    whether to persist and how to word the refusals."""
    root = Parser(_LANGUAGE).parse(text.encode("utf-8")).root_node
    if root.has_error:
        return Normalization(text=text, refused=[f"syntax-error:{name}" for name in names])
    assignments = _by_name(root)
    spans: list[ByteSpan] = []
    normalized: list[str] = []
    refused: list[str] = []
    for name in names:
        span = _span_for(name, assignments.get(name, []), refused)
        if span is not None:
            spans.append(span)
            normalized.append(name)
    if not spans:
        return Normalization(text=text, refused=refused)
    return Normalization(text=apply_byte_spans(text, spans), normalized=normalized, refused=refused)


def _by_name(root: Node) -> dict[str, list[tuple[Node, bool]]]:
    """Every top-level `NAME=` assignment (plain `=` only — a `+=` is an accumulator, never a
    const), grouped by name, each with its readonly flag."""
    out: dict[str, list[tuple[Node, bool]]] = {}
    for node, readonly in _toplevel_assignments(root):
        name_node = node.child_by_field_name("name")
        if name_node is None or name_node.type != "variable_name":
            continue
        if _assignment_operator(node) != "=":
            continue
        out.setdefault(_text(name_node), []).append((node, readonly))
    return out


def _span_for(name: str, found: list[tuple[Node, bool]], refused: list[str]) -> ByteSpan | None:
    """The one value-span rewrite for `name`, or None with a coded refusal appended. A single
    refusal chain with one success exit: every branch below is a distinct, tested reason the
    `${NAME:-…}` form would not mean exactly what the plain assignment meant."""
    code = ""
    if not found:
        code = "not-a-const"
    elif len(found) > 1:
        code = "multiple-assignments"
    elif found[0][1]:
        code = "readonly"
    else:
        value = found[0][0].child_by_field_name("value")
        if value is None:
            code = "not-a-const"  # `VAR=` — no value at all
        elif _references(value, name):
            # Checked BEFORE the literal test: `VAR="${VAR:-x}"` has no literal RHS either, but
            # "it already IS the idiom" is the useful thing to say about it.
            code = "already-env"
        else:
            literal = _literal_text(value)
            if not literal:
                code = "not-a-const"  # an expansion / array / command substitution — no literal
            elif any(ch in literal for ch in _UNSAFE):
                code = "unsafe-literal"
            else:
                return ByteSpan(value.start_byte, value.end_byte, f'"${{{name}:-{literal}}}"')
    refused.append(f"{code}:{name}")
    return None
