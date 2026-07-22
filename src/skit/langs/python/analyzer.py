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
import re
from typing import TypeGuard

from ...analysis import Analysis, Candidate
from ...params import is_secret_name

# Injectable type domain: the shim's AST substitution only supports these
# (JSON-representable, literal-reconstructable).
INJECTABLE_TYPES = ("str", "int", "float", "bool")


# Detecting these frameworks -> the script already has a proper CLI; suggest the L1 preset path
# rather than injection.
_CLI_FRAMEWORKS = ("argparse", "click", "typer", "docopt", "fire")


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
    """Scan top-level statements for literal constant assignments (single Name target).

    A name can be bound to more than one injectable literal in the same block (`X = 1` then later
    `X = 2`): a block runs sequentially, so the *last* assignment is the value the name actually
    holds once the block finishes running -- that's the "effective" definition a form default/type
    must agree with. It's also the only sound choice once a single ParamDecl is shared across every
    same-named occurrence: shim._const_targets replaces *every* occurrence of the name with the
    injected value (by design, so a guard-body override also gets the new value), so two
    same-named candidates would make the shim compute the replacement spans twice and corrupt the
    injected source (see the finding this fixes). Keep the *first* occurrence's position in the
    returned list (so candidates still read top-to-bottom like the source), but let a later
    occurrence's data replace it.
    """
    out: list[Candidate] = []
    index_by_name: dict[str, int] = {}
    for stmt in body:
        target: ast.expr | None = None  # pragma: no mutate
        value: ast.expr | None = None  # pragma: no mutate
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
        candidate = Candidate(
            binding="const",
            name=name,
            type=_type_name(v),
            default=v,
            lineno=stmt.lineno,
            secret=is_secret_name(name),
        )
        if name in index_by_name:
            out[index_by_name[name]] = candidate  # last occurrence's data wins; keep first slot
        else:
            index_by_name[name] = len(out)
            out.append(candidate)
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


def _bound_names(tree: ast.Module) -> dict[str, int]:
    """How many times each value name is bound anywhere in the module.

    Most targets are Store-context Names, but several binding forms keep their names
    as strings or AST fields instead (definitions, imports, exception handlers and
    pattern captures). Missing even one of those is unsound for constant folding: a
    local class named DEFAULT can shadow a top-level literal at the parser call site.
    This intentionally remains file-wide and conservative — any second binding makes
    the name ineligible rather than attempting partial scope execution. (Lives here,
    not in argspec, so input-candidate detection can share it without an import cycle;
    argspec's constant folding imports it back.)"""
    counts: dict[str, int] = {}

    def bump(name: str | None) -> None:
        if name:
            counts[name] = counts.get(name, 0) + 1

    for node in ast.walk(tree):
        if isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            bump(node.id)
        elif isinstance(node, ast.arg):
            bump(node.arg)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            bump(node.name)
        elif isinstance(node, ast.Import):
            for alias in node.names:
                # `import pkg.sub` binds pkg; `import pkg.sub as p` binds p.
                bump(alias.asname or alias.name.split(".")[0])
        elif isinstance(node, ast.ImportFrom):
            for alias in node.names:
                # Star imports can bind any public name, so no top-level literal is
                # provably unique in their presence. Mark every harvested name below
                # via the sentinel handled by _constant_env.
                bump("*" if alias.name == "*" else alias.asname or alias.name)
        elif isinstance(node, (ast.ExceptHandler, ast.MatchAs, ast.MatchStar)):
            bump(node.name)
        elif isinstance(node, ast.MatchMapping):
            bump(node.rest)
    return counts


# Every node type that opens a namespace of its own. Names bound inside one of these
# cannot change what `input` means outside it, so the scan below stops at each boundary.
_SCOPE_NODES = (
    ast.FunctionDef,
    ast.AsyncFunctionDef,
    ast.Lambda,
    ast.ClassDef,
    ast.ListComp,
    ast.SetComp,
    ast.DictComp,
    ast.GeneratorExp,
)


def _scope_body(scope: ast.AST) -> list[ast.AST]:
    """The nodes evaluated in `scope`'s OWN namespace.

    A function's parameters bind inside it, so `args` comes along; its decorators and
    defaults actually evaluate in the ENCLOSING scope, and counting them here is the one
    deliberate over-approximation (a decorator that binds `input` is not a thing)."""
    if isinstance(scope, (ast.FunctionDef, ast.AsyncFunctionDef)):
        return [scope.args, *scope.body]
    if isinstance(scope, ast.Lambda):
        return [scope.args, scope.body]
    if isinstance(scope, (ast.Module, ast.ClassDef)):
        return list(scope.body)
    return list(ast.iter_child_nodes(scope))  # comprehensions


def _scope_nodes(scope: ast.AST) -> tuple[list[ast.AST], list[ast.AST]]:
    """(nodes in this scope's own namespace, nested scopes to visit separately).

    A nested scope's own node stays in the first list — `def input(): ...` binds the name
    `input` out HERE — while its body goes to the second."""
    own: list[ast.AST] = []
    nested: list[ast.AST] = []
    stack = _scope_body(scope)
    while stack:
        node = stack.pop()
        own.append(node)
        if isinstance(node, _SCOPE_NODES):
            nested.append(node)
            continue
        stack.extend(ast.iter_child_nodes(node))
    return own, nested


def _binds_input(own: list[ast.AST]) -> bool:
    """Whether these same-scope nodes bind the name `input` — every binding form
    _bound_names knows, asked about one scope instead of the whole file."""
    for node in own:
        names: tuple[str | None, ...]
        if isinstance(node, ast.Name) and isinstance(node.ctx, (ast.Store, ast.Del)):
            names = (node.id,)
        elif isinstance(node, ast.arg):
            names = (node.arg,)
        elif isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            names = (node.name,)
        elif isinstance(node, ast.Import):
            # `import pkg.sub` binds pkg; `import pkg.sub as p` binds p.
            names = tuple(a.asname or a.name.split(".")[0] for a in node.names)
        elif isinstance(node, ast.ImportFrom):
            # A star import can bind any public name, `input` included.
            names = tuple("input" if a.name == "*" else a.asname or a.name for a in node.names)
        elif isinstance(node, (ast.ExceptHandler, ast.MatchAs, ast.MatchStar)):
            names = (node.name,)
        elif isinstance(node, ast.MatchMapping):
            names = (node.rest,)
        else:
            continue
        if "input" in names:
            return True
    return False


def _builtin_input_calls(tree: ast.Module) -> list[ast.Call]:
    """Every `input(...)` call whose `input` still resolves to the builtin, in source order.

    A script that binds `input` itself calls THAT, not the builtin prompt, and rewriting
    such a call would splice a stdin-fallback wrapper over the script's own function — so
    those call sites are dropped and any stored parameter for them surfaces as reconcile
    drift instead. The question is asked PER SCOPE, because it is a resolution question:
    `_bound_names` answers it file-wide, which is right where it came from (constant
    folding, where an extra binding merely skips a fold) and wrong here, where the cost is
    an entry that no longer runs. A parameter named `input` in one unrelated helper — a
    common name — must not strip the managed prompts off the rest of the file.

    Conservative in the two directions that stay cheap: a binding in a scope disables its
    nested scopes too (a closure sees it), and a binding in a CLASS body disables the
    methods below it even though Python's lookup skips class scope there — a miss costs a
    parameter, never a corrupted rewrite."""
    out: list[ast.Call] = []
    scopes: list[ast.AST] = [tree]
    while scopes:
        own, nested = _scope_nodes(scopes.pop())
        if _binds_input(own):
            continue
        out.extend(
            node
            for node in own
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Name)
            and node.func.id == "input"
        )
        scopes.extend(nested)
    out.sort(key=lambda c: (c.lineno, c.col_offset))
    return out


def _literal_prompt(call: ast.Call) -> str:
    """The literal first-argument string of an input() call, or "" if absent/non-literal.

    This doubles as the stable match key shim/reconcile prefer over bare call order (3a): a
    dynamic prompt (a variable, an f-string, no argument at all) has no literal text to key on and
    must fall back to "" -- callers then treat that the same as "no stable key available" and fall
    back to positional order.
    """
    if call.args and isinstance(call.args[0], ast.Constant) and isinstance(call.args[0].value, str):
        return call.args[0].value
    return ""


def _input_candidates(tree: ast.Module) -> list[Candidate]:
    """Every input() call in the file, numbered by order of appearance in the source (B1)."""
    calls = _builtin_input_calls(tree)
    out: list[Candidate] = []
    for i, call in enumerate(calls):
        prompt = _literal_prompt(call)
        candidate = Candidate(
            binding="input",
            name=f"input-{i + 1}",
            prompt=prompt,
            order=i,
            lineno=call.lineno,
            secret=is_secret_name(prompt),
        )
        candidate.type = "str"  # pragma: no mutate — matches Candidate's own field default
        out.append(candidate)
    return out


# Filename-shaped: no whitespace, a real extension (alpha-led, 2-4 chars — "3.14" is a
# version, not a file), and not a URL. Deliberately narrow; a missed hint is cheaper than
# a wrong one.
_FILENAME_RE = re.compile(r"[^\s]{1,120}\.[A-Za-z][A-Za-z0-9]{1,3}")
_FILENAME_HINT_CAP = 3


def _mutated_names(tree: ast.Module) -> set[str]:
    """Names that look like working variables, not parameters: augmented-assigned anywhere,
    or (re)assigned inside a for/while body."""
    out: set[str] = set()
    for node in ast.walk(tree):
        if isinstance(node, ast.AugAssign) and isinstance(node.target, ast.Name):
            out.add(node.target.id)
        elif isinstance(node, (ast.For, ast.While)):
            for sub in ast.walk(node):
                if isinstance(sub, ast.Assign):
                    out.update(t.id for t in sub.targets if isinstance(t, ast.Name))
                elif isinstance(sub, (ast.AnnAssign, ast.AugAssign)) and isinstance(
                    sub.target, ast.Name
                ):
                    out.add(sub.target.id)
    return out


def _uses_argv(tree: ast.Module) -> bool:
    """Any appearance of sys.argv (subscript, slice, len(...) — all imply CLI args)."""
    for node in ast.walk(tree):
        if (
            isinstance(node, ast.Attribute)
            and node.attr == "argv"
            and isinstance(node.value, ast.Name)
            and node.value.id == "sys"
        ):
            return True
    return False


def _filename_literals(tree: ast.Module) -> list[str]:
    """Filename-looking string literals passed directly as call arguments (source order,
    deduped, capped). A literal bound to a name first is already a candidate, not a hint."""
    out: list[str] = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        args: list[ast.expr] = list(node.args) + [kw.value for kw in node.keywords]
        for arg in args:
            if not (isinstance(arg, ast.Constant) and isinstance(arg.value, str)):
                continue
            s = arg.value
            if _FILENAME_RE.fullmatch(s) and "://" not in s and s not in out:
                out.append(s)
    return out[:_FILENAME_HINT_CAP]


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
    mutated = _mutated_names(tree)
    for c in candidates:
        if c.binding == "const" and c.name in mutated:
            c.demoted = True
            c.demotion = "accumulator"
    candidates.extend(_input_candidates(tree))
    return Analysis(
        candidates=candidates,
        frameworks=_detect_frameworks(tree),
        uses_argv=_uses_argv(tree),
        filename_literals=_filename_literals(tree),
    )
