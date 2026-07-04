"""Analyzer: candidate-parameter detection at add time (the entry point to Layer 2, the core
differentiator).

The candidate-decision logic here (literal detection, the injectable type domain, main-guard
scanning) is the reason decision A2 exists: the Phase 3 shim rewrites the AST at run time and must
share exactly this logic, or a parameter selected at add time might be missed or misclassified at
run time. This module therefore uses **only the stdlib** and is fully headless, so the shim can
import or vendor it directly.

Detection scope (A4 + C4):
- Module top-level literal constant assignments (NAME = <str|int|float|bool>, negatives included).
- The same kind of assignment at the top of an `if __name__ == "__main__":` block (the second most
  common place to hard-code values).
- Every `input()` call in the file, keyed by **order of appearance** (B1); the prompt is taken from
  the first literal argument.
- argparse / click / typer import detection -> suggest the L1 args + preset path, no injection.
- Variable names / prompts containing KEY/TOKEN/SECRET/PASSWORD -> pre-check secret (C3).
"""

from __future__ import annotations

import ast
from dataclasses import dataclass, field
from typing import TypeGuard

# Injectable type domain: the shim's AST substitution only supports these
# (JSON-representable, literal-reconstructable).
INJECTABLE_TYPES = ("str", "int", "float", "bool")

# Secret pre-check heuristic (matched against the upper-cased variable name / prompt).
_SECRET_HINTS = ("KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD")

# Detecting these frameworks -> the script already has a proper CLI; suggest the L1 preset path
# rather than injection.
_CLI_FRAMEWORKS = ("argparse", "click", "typer", "docopt", "fire")


@dataclass
class Candidate:
    """A candidate parameter. const is keyed by variable name; input by call order (B1/A8)."""

    kind: str  # "const" | "input"
    name: str  # const: variable name; input: display name (input-1, input-2, …)
    type: str = "str"  # one of INJECTABLE_TYPES
    default: str | int | float | bool | None = None  # const: the original value in the source
    prompt: str = ""  # input: the literal prompt of input() (if any)
    order: int = -1  # input: which input() call (0-based); -1 for const
    lineno: int = 0
    secret: bool = False  # heuristic pre-check, editable during onboarding


@dataclass
class Analysis:
    candidates: list[Candidate] = field(default_factory=list)
    frameworks: list[str] = field(default_factory=list)  # detected CLI frameworks
    syntax_error: bool = False

    @property
    def uses_cli_framework(self) -> bool:
        return bool(self.frameworks)


def _is_secret_name(text: str) -> bool:
    up = text.upper()
    return any(h in up for h in _SECRET_HINTS)


def _literal_value(node: ast.expr) -> tuple[bool, str | int | float | bool | None]:
    """Whether the RHS is an injectable literal. Returns (ok, value).

    Handles unary +/- forms such as ``-3`` or ``+2.5``.
    """
    if isinstance(node, ast.Constant) and isinstance(node.value, (str, int, float, bool)):
        return True, node.value
    if (
        isinstance(node, ast.UnaryOp)
        and isinstance(node.op, (ast.USub, ast.UAdd))
        and isinstance(node.operand, ast.Constant)
        and isinstance(node.operand.value, (int, float))
        and not isinstance(node.operand.value, bool)
    ):
        v = node.operand.value
        return True, (-v if isinstance(node.op, ast.USub) else v)
    return False, None


def _type_name(value: object) -> str:
    # bool is a subclass of int, so it must be checked first.
    if isinstance(value, bool):
        return "bool"
    if isinstance(value, int):
        return "int"
    if isinstance(value, float):
        return "float"
    return "str"


def _const_candidates(body: list[ast.stmt]) -> list[Candidate]:
    """Scan top-level statements for literal constant assignments (single Name target)."""
    out: list[Candidate] = []
    for stmt in body:
        target: ast.expr | None = None
        value: ast.expr | None = None
        if isinstance(stmt, ast.Assign) and len(stmt.targets) == 1:
            target, value = stmt.targets[0], stmt.value
        elif isinstance(stmt, ast.AnnAssign) and stmt.value is not None:
            target, value = stmt.target, stmt.value
        if not isinstance(target, ast.Name) or value is None:
            continue
        name = target.id
        if name.startswith("_"):
            continue  # conventionally private/internal values; not treated as parameters
        ok, v = _literal_value(value)
        if not ok:
            continue
        out.append(
            Candidate(
                kind="const",
                name=name,
                type=_type_name(v),
                default=v,
                lineno=stmt.lineno,
                secret=_is_secret_name(name),
            )
        )
    return out


def _is_main_guard(stmt: ast.stmt) -> TypeGuard[ast.If]:
    """`if __name__ == "__main__":` (including the operands-reversed form)."""
    if not isinstance(stmt, ast.If):
        return False
    test = stmt.test
    if not (
        isinstance(test, ast.Compare) and len(test.ops) == 1 and isinstance(test.ops[0], ast.Eq)
    ):
        return False
    sides = [test.left, test.comparators[0]]
    has_name = any(isinstance(s, ast.Name) and s.id == "__name__" for s in sides)
    has_main = any(isinstance(s, ast.Constant) and s.value == "__main__" for s in sides)
    return has_name and has_main


def _input_candidates(tree: ast.Module) -> list[Candidate]:
    """Every input() call in the file, numbered by order of appearance in the source (B1)."""
    calls: list[ast.Call] = [
        node
        for node in ast.walk(tree)
        if isinstance(node, ast.Call)
        and isinstance(node.func, ast.Name)
        and node.func.id == "input"
    ]
    calls.sort(key=lambda c: (c.lineno, c.col_offset))
    out: list[Candidate] = []
    for i, call in enumerate(calls):
        prompt = ""
        if (
            call.args
            and isinstance(call.args[0], ast.Constant)
            and isinstance(call.args[0].value, str)
        ):
            prompt = call.args[0].value
        out.append(
            Candidate(
                kind="input",
                name=f"input-{i + 1}",
                type="str",
                prompt=prompt,
                order=i,
                lineno=call.lineno,
                secret=_is_secret_name(prompt),
            )
        )
    return out


def _detect_frameworks(tree: ast.Module) -> list[str]:
    found: list[str] = []
    for node in ast.walk(tree):
        mods: list[str] = []
        if isinstance(node, ast.Import):
            mods = [a.name.split(".")[0] for a in node.names]
        elif isinstance(node, ast.ImportFrom) and node.module and node.level == 0:
            mods = [node.module.split(".")[0]]
        for m in mods:
            if m in _CLI_FRAMEWORKS and m not in found:
                found.append(m)
    return found


def analyze(text: str) -> Analysis:
    """Detect candidate parameters in the script source. On a syntax error, return an empty result
    (no exception; add can still take the script into the store)."""
    try:
        tree = ast.parse(text)
    except SyntaxError:
        return Analysis(syntax_error=True)
    candidates = _const_candidates(tree.body)
    seen = {c.name for c in candidates}
    for stmt in tree.body:
        if _is_main_guard(stmt):
            for c in _const_candidates(stmt.body):
                if c.name not in seen:  # module top-level wins (a same-name main-guard assignment
                    candidates.append(c)  # is an override, not the definition)
                    seen.add(c.name)
    candidates.extend(_input_candidates(tree))
    return Analysis(candidates=candidates, frameworks=_detect_frameworks(tree))
